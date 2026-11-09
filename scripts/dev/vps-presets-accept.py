"""Run WSL-built protocol tests on an authorized Linux VPS. Never compile here.

Python is only the acceptance driver; the deployed program and tests are native
executables. Logs/backups stay private. Run --activate only on a test deployment.
"""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import socket
import ssl
import subprocess
import time

os.umask(0o077)
parser = argparse.ArgumentParser()
parser.add_argument('--artifacts', type=Path, required=True)
parser.add_argument('--evidence', type=Path, required=True)
parser.add_argument('--activate', action='store_true')
parser.add_argument('--inject-startup-failure', action='store_true', help='With --activate, expect exit 23 on serve and verify restoration')
parser.add_argument('--reality-target', help='Explicit external Reality target, host:port')
parser.add_argument('--client-matrix', action='store_true', help='Also exercise installed sing-box and staged Mihomo clients')
args = parser.parse_args()
if args.inject_startup_failure and not args.activate:
    parser.error('--inject-startup-failure requires --activate')
artifacts, evidence = args.artifacts.resolve(), args.evidence.resolve()
assert artifacts.parent == Path('/opt') and artifacts.name.startswith('sbctl-protocol-test-')
assert evidence.parent == Path('/root') and evidence.name.startswith('sbctl-protocol-test-')
evidence.mkdir(mode=0o700, exist_ok=True)
live = Path('/usr/local/bin/sbctl')
config = Path('/etc/sbctl/config.toml')
results = {}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(name, command, expected=0, timeout=90):
    completed = subprocess.run([str(x) for x in command], capture_output=True,
                               text=True, timeout=timeout)
    (evidence / (name + '.log')).write_text(completed.stdout + completed.stderr)
    if expected is not None and completed.returncode != expected:
        raise RuntimeError(name + ' failed; see private log')
    return completed


def active(unit):
    return subprocess.run(['systemctl', 'is-active', '--quiet', unit]).returncode == 0


def fetch(route):
    text = config.read_text()
    host = re.search(r'^subscription_host = "([^"]+)"', text, re.M).group(1)
    credential = re.search(r'^subscription_credential = "([^"]+)"', text, re.M).group(1)
    conn = http.client.HTTPSConnection(host, timeout=15, context=ssl.create_default_context())
    conn._create_connection = lambda *a, **kw: socket.create_connection(('127.0.0.1', 443), 15)
    conn.request('GET', '/sub/' + credential + '/' + route)
    response = conn.getresponse()
    status, body = response.status, response.read()
    conn.close()
    return status, body


def health():
    run('live-config', [live, 'config', 'validate'])
    run('live-core', ['/usr/local/bin/sing-box', 'check', '-c', '/etc/sing-box/config.json'])
    assert active('sbctl.service') and active('sing-box.service'), 'managed service inactive'
    routes = ['sing-box.json', 'sing-box-full.json', 'sing-box-1.10.json',
              'sing-box-1.11.json', 'sing-box-1.12.json', 'sing-box-1.13.json',
              'sing-box-1.14.json', 'clash.yaml', 'clash-1.18.yaml', 'uri',
              'uri.txt', 'shadowrocket.txt', 'index']
    report = {}
    for route in routes:
        status, body = fetch(route)
        assert status == 200 and body, 'subscription health check failed'
        report[route] = status
    assert fetch('uri?test=1')[0] == 404
    return report


try:
    assert os.uname().machine == 'x86_64'
    before_config = sha(config)
    before_core = sha(Path('/usr/local/bin/sing-box'))
    run('candidate-version', [artifacts / 'sbctl-linux-amd64', '--version'])
    run('candidate-presets', [artifacts / 'sbctl-linux-amd64', 'protocol', 'list'])
    for name in ['xray_core', 'preset_connections']:
        unit = 'sbctl-presets-' + name.replace('_', '-') + '-' + str(os.getpid())
        command = ['systemd-run', '--quiet', '--wait', '--pipe', '--collect',
                   '--unit=' + unit, '-p', 'DynamicUser=yes', '-p', 'PrivateTmp=yes',
                   '-p', 'ProtectSystem=strict', '-p', 'ProtectHome=yes',
                   '-p', 'NoNewPrivileges=yes',
                   '--setenv=SBCTL_XRAY_BIN=' + str(artifacts / 'xray-linux-amd64'),
                   artifacts / (name + '-linux-amd64'), '--ignored', '--nocapture']
        if name == 'preset_connections':
            command += ['--exact', 'tls_presets_carry_http_through_a_real_socks_client']
        run(name, command)
        results[name] = 'passed under systemd DynamicUser / NoNewPrivileges'
    if args.reality_target:
        run('external-reality', ['systemd-run', '--quiet', '--wait', '--pipe', '--collect',
            '--unit=sbctl-reality-' + str(os.getpid()), '-p', 'DynamicUser=yes', '-p', 'PrivateTmp=yes',
            '-p', 'NoNewPrivileges=yes', '--setenv=SBCTL_XRAY_BIN=' + str(artifacts / 'xray-linux-amd64'),
            '--setenv=SBCTL_REALITY_TARGET=' + args.reality_target,
            artifacts / 'preset_connections-linux-amd64', '--ignored', '--exact', 'xhttp_reality_uses_external_decoy'])
        results['external_reality'] = 'passed'
    if args.client_matrix:
        for kind, binary in [('sing_box', Path('/usr/local/bin/sing-box')), ('mihomo', artifacts / 'mihomo')]:
            run('client-' + kind, ['systemd-run', '--quiet', '--wait', '--pipe', '--collect',
                '--unit=sbctl-client-' + kind.replace('_', '-') + '-' + str(os.getpid()),
                '-p', 'DynamicUser=yes', '-p', 'PrivateTmp=yes', '-p', 'NoNewPrivileges=yes',
                '--setenv=SBCTL_XRAY_BIN=' + str(artifacts / 'xray-linux-amd64'),
                '--setenv=SBCTL_CLIENT_BIN=' + str(binary),
                artifacts / 'preset_connections-linux-amd64', '--ignored', '--exact', kind + '_compatible_presets_carry_http'])
            results[kind + '_client'] = 'passed'
    results['before_activation'] = health()
    if args.activate:
        before_program = sha(live)
        backup = evidence / 'sbctl-before-activation'
        assert not backup.exists(), 'activation backup already exists; use a fresh evidence directory'
        shutil.copy2(live, backup)
        candidate = live.with_name('sbctl.protocol-candidate')
        if args.inject_startup_failure:
            candidate.write_text('#!/bin/sh\nif [ "$1" = "serve" ]; then exit 23; fi\nexec "' + str(artifacts / 'sbctl-linux-amd64') + '" "$@"\n')
        else:
            shutil.copyfile(artifacts / 'sbctl-linux-amd64', candidate)
        os.chmod(candidate, 0o755)
        os.chown(candidate, 0, 0)
        os.replace(candidate, live)
        try:
            run('restart-candidate', ['systemctl', 'restart', 'sbctl.service'])
            time.sleep(2)
            results['after_activation'] = health()
            assert sha(config) == before_config, 'deployment configuration changed'
            assert sha(Path('/usr/local/bin/sing-box')) == before_core, 'data plane changed'
            run('installed-presets', [live, 'protocol', 'list'])
            results['installed_sha256'] = sha(live)
            assert not args.inject_startup_failure, 'injected serve failure was not detected'
        except Exception:
            failure_status = run('failed-service-status', ['systemctl', 'show', 'sbctl.service', '-p', 'ExecMainStatus', '--value'], expected=None).stdout.strip()
            shutil.copyfile(backup, candidate)
            os.chmod(candidate, 0o755)
            os.replace(candidate, live)
            run('reset-failed', ['systemctl', 'reset-failed', 'sbctl.service'], expected=None)
            run('restore-program', ['systemctl', 'restart', 'sbctl.service'])
            results['rollback_health'] = health()
            assert sha(live) == before_program, 'program rollback digest mismatch'
            if args.inject_startup_failure and failure_status == '23':
                results['startup_failure_recovered'] = True
                results['restored_sha256'] = sha(live)
            else:
                raise
    results['passed'] = True
except Exception as error:
    results['passed'] = False
    # Avoid emitting subprocess diagnostics or secret-bearing config content.
    results['error'] = type(error).__name__ + ': ' + str(error)
finally:
    (evidence / 'acceptance-result.json').write_text(json.dumps(results, indent=2))
    print(json.dumps(results), flush=True)
raise SystemExit(0 if results['passed'] else 1)
