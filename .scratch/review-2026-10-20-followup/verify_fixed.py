"""Verify a supplied candidate in isolated roots, without touching host services."""
import json
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path

SBCTL = Path(os.environ['SBCTL_BIN']).resolve()


def run(root, *args, env=None, ok=0):
    result = subprocess.run([str(SBCTL), '--root', str(root), *args],
                            capture_output=True, text=True, env=env, timeout=30)
    assert result.returncode == ok, (args, result.returncode, result.stderr)
    return result


def write(root, relative, body, executable=False):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    if executable:
        path.chmod(0o700)
    return path


def seed(root):
    run(root, 'config', 'init', '--mode', 'ip-fallback',
        '--subscription-host', '127.0.0.1', '--http-port', '2080',
        '--interface', 'ens3', '--protocol', 'vless-reality',
        '--reality-decoy-sni', 'www.cloudflare.com')
    checker = os.environ.get('CHECKER_BIN')
    write(root, 'usr/local/bin/sing-box',
          '#!/bin/sh\n' + (f'exec "{checker}" "$@"\n' if checker else 'exit 0\n'), True)
    write(root, 'usr/bin/systemctl', '#!/bin/sh\nexit 0\n', True)


def wait_for(path):
    end = time.monotonic() + 15
    while not path.exists():
        assert time.monotonic() < end, f'timeout waiting for {path.name}'
        time.sleep(0.02)


def secret(root):
    text = (root / 'etc/sbctl/config.toml').read_text()
    return re.search(r'^subscription_credential = "([^"]+)"', text, re.M)[1]


def api_rollback():
    with tempfile.TemporaryDirectory(prefix='sbctl-review-api-') as tmp:
        root = Path(tmp)
        seed(root)
        old = secret(root)
        write(root, 'usr/bin/systemctl', '''#!/bin/sh
if [ "$AUDIT_BLOCK" = 1 ] && [ "$1" = restart ] && [ ! -e "''' + str(root / 'failed') + '''" ]; then
  touch "''' + str(root / 'ready') + '''"
  while [ ! -e "''' + str(root / 'release') + '''" ]; do sleep 0.02; done
  touch "''' + str(root / 'failed') + '''"
  exit 1
fi
exit 0
''', True)
        env = dict(os.environ, AUDIT_BLOCK='1')
        process = subprocess.Popen([str(SBCTL), '--root', str(root), 'sing-box',
                                    'api', 'enable', '--port', '19090'],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, env=env)
        try:
            wait_for(root / 'ready')
            rotation = subprocess.Popen([str(SBCTL), '--root', str(root), 'credential', 'rotate'],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            time.sleep(0.3)
            assert rotation.poll() is None, 'rotation must wait for API recovery'
            (root / 'release').touch()
            _, err = process.communicate(timeout=20)
            assert process.returncode == 2, err
            _, err = rotation.communicate(timeout=20)
            assert rotation.returncode == 0, err
            assert secret(root) != old
            print('PASSED API recovery serializes concurrent credential rotation')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


def clear_server():
    with tempfile.TemporaryDirectory(prefix='sbctl-review-clear-') as tmp:
        root = Path(tmp)
        seed(root)
        write(root, 'var/lib/sbctl/ownership', 'managed by sbctl\n')
        override = write(root, 'etc/sbctl/overrides/sing-box-server.json',
                         '{"log":{"level":"debug"}}\n')
        run(root, 'regenerate')
        active = root / 'etc/sing-box/config.json'
        assert json.loads(active.read_text())['log']['level'] == 'debug'
        log = root / 'systemctl.log'
        write(root, 'usr/bin/systemctl', '#!/bin/sh\nprintf "%s\\n" "$*" >> "' + str(log) + '"\nexit 0\n', True)
        run(root, 'config', 'override', 'clear', 'server')
        assert not override.exists()
        assert json.loads(active.read_text())['log']['level'] != 'debug'
        assert 'restart' in log.read_text()
        print('PASSED server override clear activates the generated configuration')


def rules_mode():
    for mode in ['append', 'replace']:
        with tempfile.TemporaryDirectory(prefix='sbctl-review-rules-') as tmp:
            root = Path(tmp)
            seed(root)
            rule = {'domain': ['review.example'], 'outbound': 'DIRECT'}
            write(root, 'etc/sbctl/overrides/sing-box-override.json', json.dumps({
                'rules_mode': mode, 'route': {'rules': [rule]}}))
            write(root, 'etc/sbctl/overrides/clash-override.yaml',
                  f'rules_mode: {mode}\nrules:\n  - DOMAIN,review.example,DIRECT\n')
            run(root, 'regenerate')
            artifacts = root / 'var/lib/sbctl/artifacts'
            profile = json.loads((artifacts / 'subscription-sing-box-full.json').read_text())
            assert profile['route']['rules'][-1] == rule
            assert len(profile['route']['rules']) == 1 if mode == 'replace' else len(profile['route']['rules']) > 1
            clash = (artifacts / 'subscription-clash.yaml').read_text()
            assert ('MATCH,' not in clash) if mode == 'replace' else ('MATCH,' in clash)
            assert 'rules_mode' not in clash
            print(f'PASSED rules_mode={mode} in generated sing-box and Clash artifacts')


def open_editor():
    with tempfile.TemporaryDirectory(prefix='sbctl-review-editor-') as tmp:
        root = Path(tmp)
        seed(root)
        source = write(root, 'etc/sbctl/overrides/clash-override.yaml', 'mixed-port: 17890\n')
        run(root, 'regenerate')
        editor = write(root, 'editor.sh', '''#!/bin/sh
printf 'mixed-port: 17891\\n' > "$1"
touch "''' + str(root / 'ready') + '''"
while [ ! -e "''' + str(root / 'release') + '''" ]; do sleep 0.02; done
exit 1
''', True)
        process = subprocess.Popen([str(SBCTL), '--root', str(root), 'config',
                                    'override', 'edit', 'clash'],
                                   env=dict(os.environ, VISUAL=str(editor)),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            wait_for(root / 'ready')
            run(root, 'rule', 'add', 'direct', 'review.example')
            artifact = root / 'var/lib/sbctl/artifacts/subscription-clash.yaml'
            assert re.search(r'^mixed-port: 17890$', artifact.read_text(), re.M)
            (root / 'release').touch()
            process.communicate(timeout=10)
            assert process.returncode == 2
            assert source.read_text() == 'mixed-port: 17890\n'
            assert re.search(r'^mixed-port: 17890$', artifact.read_text(), re.M)
            print('PASSED failed override draft stays private during concurrent regeneration')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


if __name__ == '__main__':
    api_rollback()
    rules_mode()
    clear_server()
    open_editor()
