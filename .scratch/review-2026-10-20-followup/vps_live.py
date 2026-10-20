"""Temporary live VPS acceptance, with root-only backup and timed restoration."""
import fcntl
import grp
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import socket
import ssl
import stat
import subprocess
import sys
import time

BASE = Path('/root/sbctl-v015-acceptance')
TIMERS = ['sbctl-accounting-reset.timer', 'sbctl-credential-rotate.timer', 'sbctl-email.timer']
SERVICES = ['sbctl.service', 'sing-box.service', 'sbctl-http.socket']
FORMATS = {
    'sing-box.json': 'subscription-sing-box.json',
    'sing-box-full.json': 'subscription-sing-box-full.json',
    **{f'sing-box-1.{minor}.json': f'subscription-sing-box-1.{minor}.json' for minor in range(10, 15)},
    'clash.yaml': 'subscription-clash.yaml', 'clash-1.18.yaml': 'subscription-clash-1.18.yaml',
    'uri': 'subscription-uri.txt', 'uri.txt': 'subscription-base64-uri.txt',
    'shadowrocket.txt': 'subscription-shadowrocket.txt',
}


def run(args, ok=True, env=None):
    result = subprocess.run([str(x) for x in args], text=True, capture_output=True,
                            timeout=100, env=env)
    if ok and result.returncode:
        raise RuntimeError(f'command failed: {args[0]} {args[1] if len(args) > 1 else ""}')
    return result


def digest(path):
    with open(path, 'rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest() if hasattr(hashlib, 'file_digest') else hashlib.sha256(source.read()).hexdigest()


def lock():
    fd = os.open('/var/lib/sbctl/.operation.lock', os.O_RDWR | os.O_NOFOLLOW)
    fcntl.flock(fd, fcntl.LOCK_EX)
    return fd


def backup():
    BASE.mkdir(mode=0o700, exist_ok=True)
    folder = BASE / ('backup-' + time.strftime('%Y%m%d-%H%M%S', time.gmtime()))
    folder.mkdir(mode=0o700)
    timer_states = {unit: run(['systemctl', 'is-active', unit], False).stdout.strip() for unit in TIMERS}
    for unit in TIMERS:
        run(['systemctl', 'stop', unit], False)
        run(['systemctl', 'stop', unit.replace('.timer', '.service')], False)
    fd = lock()
    try:
        paths = ['usr/local/bin/sbctl', 'usr/local/bin/sing-box', 'etc/sbctl',
                 'var/lib/sbctl', 'etc/sing-box', 'etc/letsencrypt']
        for unit in SERVICES + TIMERS + [x.replace('.timer', '.service') for x in TIMERS]:
            for suffix in ['', '.d']:
                relative = 'etc/systemd/system/' + unit + suffix
                if os.path.lexists('/' + relative):
                    paths.append(relative)
        paths = [x for x in paths if os.path.lexists('/' + x)]
        run(['tar', '--acls', '--xattrs', '--exclude=var/lib/sbctl/.operation.lock',
             '-czf', folder / 'deployment.tgz', '-C', '/', *paths])
        selected = [Path('/usr/local/bin/sbctl'), Path('/usr/local/bin/sing-box'),
                    Path('/etc/sbctl/config.toml'), Path('/var/lib/sbctl/state.json'),
                    Path('/etc/sing-box/config.json')]
        selected += list(Path('/var/lib/sbctl/artifacts').glob('*'))
        selected += [x for x in Path('/var/lib/sbctl/certificates').rglob('*') if x.is_file()]
        files = {}
        for path in selected:
            if path.is_file():
                meta = path.stat()
                files[str(path)] = {'sha256': digest(path), 'uid': meta.st_uid,
                                    'gid': meta.st_gid, 'mode': stat.S_IMODE(meta.st_mode)}
        (folder / 'baseline.json').write_text(json.dumps({'files': files, 'timers': timer_states}, indent=2))
        print(json.dumps({'backup': str(folder), 'archive_sha256': digest(folder / 'deployment.tgz')}), flush=True)
        return folder
    finally:
        os.close(fd)


def restore(folder):
    folder = Path(folder).resolve()
    assert folder.parent == BASE and folder.name.startswith('backup-')
    baseline = json.loads((folder / 'baseline.json').read_text())
    for unit in SERVICES + TIMERS:
        run(['systemctl', 'stop', unit], False)
    fd = lock()
    try:
        for name in ['99-v015-acceptance.json', '99-v015-invalid.json']:
            path = Path('/etc/sbctl/overrides/sing-box-server.d') / name
            path.unlink(missing_ok=True)
        run(['tar', '--acls', '--xattrs', '-xzf', folder / 'deployment.tgz', '-C', '/'])
        run(['systemctl', 'daemon-reload'])
        run(['/usr/local/bin/sbctl', 'restart'])
        for path, expected in baseline['files'].items():
            meta = Path(path).stat()
            assert digest(path) == expected['sha256'], 'restore bytes differ: ' + path
            assert (meta.st_uid, meta.st_gid, stat.S_IMODE(meta.st_mode)) == (expected['uid'], expected['gid'], expected['mode']), 'restore metadata differ: ' + path
    finally:
        os.close(fd)
        for unit, state in baseline['timers'].items():
            if state == 'active':
                run(['systemctl', 'start', unit])
    run(['systemctl', 'is-active', '--quiet', *SERVICES])
    check_public()
    (folder / 'restored.json').write_text(json.dumps({'restored': True, 'original_files_and_metadata_match': True}))
    run(['systemctl', 'stop', baseline.get('rollback_unit', 'sbctl-v015-acceptance-rollback') + '.timer'], False)
    print('RESTORED original deployment bytes, metadata, timers and public subscription', flush=True)


def request(host, token, route):
    connection = http.client.HTTPSConnection(host, timeout=15, context=ssl.create_default_context())
    connection._create_connection = lambda address, *args, **kwargs: socket.create_connection(('127.0.0.1', 443), timeout=15)
    connection.request('GET', f'/sub/{token}/{route}')
    response = connection.getresponse()
    result = (response.status, response.read(), dict(response.getheaders()))
    connection.close()
    return result


def check_public():
    config = Path('/etc/sbctl/config.toml').read_text()
    host = re.search(r'^subscription_host = "([^"]+)"', config, re.M)[1]
    token = re.search(r'^subscription_credential = "([^"]+)"', config, re.M)[1]
    for route, artifact in FORMATS.items():
        status, body, headers = request(host, token, route)
        assert status == 200 and body == (Path('/var/lib/sbctl/artifacts') / artifact).read_bytes(), 'subscription mismatch: ' + route
        assert any(k.lower() == 'subscription-userinfo' for k in headers), 'missing traffic header'
        status, body, _ = request(host, token, 'qr/' + route)
        assert status == 200 and b'<svg' in body, 'QR mismatch: ' + route
    assert request(host, token, 'index')[0] == 200
    assert request(host, 'invalid-review-credential', 'uri')[0] == 404
    assert request(host, token, 'uri?unexpected=1')[0] == 404


def pid():
    return run(['systemctl', 'show', '-p', 'MainPID', '--value', 'sing-box.service']).stdout.strip()


def editor(folder, document):
    path = folder / 'editor.sh'
    path.write_text('#!/bin/sh\nprintf \'%s\\n\' \'' + json.dumps(document) + '\' > "$1"\n')
    path.chmod(0o700)
    return dict(os.environ, VISUAL=str(path))


def live(candidate, expected_digest):
    assert os.geteuid() == 0
    candidate = Path(candidate).resolve()
    assert digest(candidate) == expected_digest
    before = Path('/etc/sbctl/config.toml').read_text()
    assert 'subscription_mode = "direct"' in before
    assert '[server_clash_api]' not in before
    assert not list(Path('/etc/sbctl/overrides/sing-box-server.d').glob('*.json'))
    assert not Path('/etc/sbctl/overrides/sing-box-server.json').exists()
    folder = backup()
    guard = 'sbctl-v015-acceptance-rollback-' + folder.name
    baseline_path = folder / 'baseline.json'
    baseline = json.loads(baseline_path.read_text())
    baseline['rollback_unit'] = guard
    baseline_path.write_text(json.dumps(baseline, indent=2))
    try:
        run(['systemd-run', '--unit=' + guard, '--on-active=15m',
             '--timer-property=AccuracySec=1s', '/usr/bin/python3', str(Path(__file__).resolve()), 'restore', folder])
        fd = lock()
        try:
            target = Path('/usr/local/bin/sbctl.v015-next')
            shutil.copyfile(candidate, target)
            target.chmod(0o755)
            os.replace(target, '/usr/local/bin/sbctl')
        finally:
            os.close(fd)
        assert digest('/usr/local/bin/sbctl') == expected_digest
        run(['/usr/local/bin/sbctl', 'regenerate'])
        run(['/usr/local/bin/sbctl', 'restart'])
        check_public()
        report = {'candidate_sha256': expected_digest, 'public_12_formats_and_qrs': True}
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = str(sock.getsockname()[1])
        run(['/usr/local/bin/sbctl', 'sing-box', 'api', 'enable', '--port', port])
        config = Path('/etc/sbctl/config.toml').read_text()
        host = re.search(r'^subscription_host = "([^"]+)"', config, re.M)[1]
        key = Path('/var/lib/sbctl/certificates') / host / 'privkey.pem'
        assert key.stat().st_gid == grp.getgrnam('sbctl-cert').gr_gid
        run(['runuser', '-u', 'sbctl', '--', 'test', '-r', key])
        check_public()
        report['api_enable_preserves_direct_tls_access'] = True
        before_pid = pid()
        env = editor(folder, {'log': {'level': 'debug'}})
        run(['/usr/local/bin/sbctl', 'config', 'override', 'edit', 'server', '--layer', '99-v015-acceptance.json'], env=env)
        assert json.loads(Path('/etc/sing-box/config.json').read_text())['log']['level'] == 'debug'
        assert pid() != before_pid
        before_pid = pid()
        run(['/usr/local/bin/sbctl', 'config', 'override', 'clear', 'server'])
        assert not Path('/etc/sbctl/overrides/sing-box-server.d/99-v015-acceptance.json').exists()
        assert pid() != before_pid
        assert json.loads(Path('/etc/sing-box/config.json').read_text())['log']['level'] != 'debug'
        check_public()
        report['server_override_edit_and_clear_restart_real_service'] = True
        source_before = Path('/etc/sbctl/config.toml').read_bytes()
        active_before = Path('/etc/sing-box/config.json').read_bytes()
        env = editor(folder, {'route': {'final': 'nonexistent-review-outbound'}})
        result = run(['/usr/local/bin/sbctl', 'config', 'override', 'edit', 'server', '--layer', '99-v015-invalid.json'], ok=False, env=env)
        assert result.returncode == 2
        assert not Path('/etc/sbctl/overrides/sing-box-server.d/99-v015-invalid.json').exists()
        assert Path('/etc/sbctl/config.toml').read_bytes() == source_before
        assert Path('/etc/sing-box/config.json').read_bytes() == active_before
        report['invalid_override_preserves_live_source_and_config'] = True
        run(['/usr/local/bin/sbctl', 'sing-box', 'api', 'disable'])
        run(['runuser', '-u', 'sbctl', '--', 'test', '-r', key])
        check_public()
        after = Path('/etc/sbctl/config.toml').read_text()
        assert re.search(r'^subscription_credential = .*', after, re.M)[0] == re.search(r'^subscription_credential = .*', before, re.M)[0]
        assert '[server_clash_api]' not in after
        assert digest('/var/lib/sbctl/state.json') == json.loads((folder / 'baseline.json').read_text())['files']['/var/lib/sbctl/state.json']['sha256']
        report['api_disable_preserves_tls_credentials_and_accounting'] = True
        (folder / 'candidate-report.json').write_text(json.dumps(report, indent=2))
        (BASE / 'active-backup').write_text(str(folder))
        print(json.dumps(report), flush=True)
        print('LIVE_CANDIDATE_READY_FOR_EXTERNAL_PROBE', flush=True)
    except BaseException:
        restore(folder)
        raise


if __name__ == '__main__':
    os.umask(0o077)
    if sys.argv[1] == 'restore':
        restore(sys.argv[2])
    else:
        live(sys.argv[2], sys.argv[3])
