"""Run as root in a disposable network namespace; no live deployment writes.

unshare --net python3 verify_isolated.py /absolute/path/to/sbctl [--old]
"""
import json
import os
from pathlib import Path
import re
import select
import socket
import ssl
import subprocess
import sys
import tempfile
import time

BINARY = str(Path(sys.argv[1]).resolve())
OLD = '--old' in sys.argv
subprocess.run(['ip', 'link', 'set', 'lo', 'up'], check=True)


def run(root, *args, success=True):
    result = subprocess.run([BINARY, '--root', str(root), *args],
                            capture_output=True, timeout=30)
    if success:
        assert result.returncode == 0, 'isolated command failed: ' + args[0]
    return result


def connect(port, source=3):
    return socket.create_connection(('127.0.0.1', port), timeout=2,
                                    source_address=(f'127.0.0.{source}', 0))


def request(port, path, context=None):
    with connect(port) as raw:
        stream = context.wrap_socket(raw, server_hostname='audit.example.test') if context else raw
        stream.sendall(f'GET {path} HTTP/1.1\r\nHost: audit.example.test\r\nConnection: close\r\n\r\n'.encode())
        result = b''
        while True:
            part = stream.recv(65536)
            if not part:
                break
            result += part
        stream.close()
        return result


def flood_check(port, path, expected_status, context=None, exempt=False):
    stalled = []
    try:
        for _ in range(32):
            stalled.append(connect(port, 2))
        time.sleep(0.15)
        active = 0
        for stream in stalled:
            if not select.select([stream], [], [], 0)[0]:
                active += 1
        assert active == (32 if OLD or exempt else 8), f'unexpected held connection count: {active}'
        try:
            result = request(port, path, context)
            available = result.startswith(f'HTTP/1.1 {expected_status}'.encode())
        except (OSError, ssl.SSLError):
            available = False
        assert available == (not OLD and not exempt), 'unexpected neighbour availability'
        print(json.dumps({'port': port, 'held_connections': active,
                          'neighbour_available': available, 'proxy_exempt': exempt}), flush=True)
    finally:
        for stream in stalled:
            stream.close()
    time.sleep(0.15)
    assert request(port, path, context).startswith(f'HTTP/1.1 {expected_status}'.encode()), 'capacity did not recover'


def seed(root, mode):
    args = ['config', 'init', '--mode', mode, '--subscription-host',
            '127.0.0.1' if mode == 'ip-fallback' else 'audit.example.test',
            '--interface', 'ens3', '--protocol', 'vless-reality',
            '--reality-decoy-sni', 'www.cloudflare.com']
    if mode == 'ip-fallback':
        args += ['--http-port', '2080']
    if mode == 'external-proxy':
        args += ['--listen-port', '2080']
    run(root, *args)
    config = (root / 'etc/sbctl/config.toml').read_text()
    return re.search(r'^subscription_credential = "([^"]+)"', config, re.M)[1]


for mode in ['ip-fallback', 'external-proxy', 'direct']:
    with tempfile.TemporaryDirectory(prefix='sbctl-connection-audit-') as tmp:
        root = Path(tmp)
        credential = seed(root, mode)
        listeners = []
        if mode == 'direct':
            certdir = root / 'var/lib/sbctl/certificates/audit.example.test'
            certdir.mkdir(parents=True, exist_ok=True)
            subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
                            '-keyout', str(certdir / 'privkey.pem'), '-out', str(certdir / 'fullchain.pem'),
                            '-days', '1', '-subj', '/CN=audit.example.test',
                            '-addext', 'subjectAltName=DNS:audit.example.test'],
                           capture_output=True, check=True)
            context = ssl.create_default_context(cafile=str(certdir / 'fullchain.pem'))
            for port in [80, 443]:
                listener = socket.socket()
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                listener.bind(('127.0.0.1', port))
                listener.listen(128)
                listeners.append(listener)
            activation = '''import os,sys
fds = [os.dup(int(fd)) for fd in sys.argv[1:3]]
for target, source in zip([3,4],fds): os.dup2(source,target,inheritable=True)
for source in fds: os.close(source)
os.environ.update(LISTEN_PID=str(os.getpid()),LISTEN_FDS='2')
os.execv(sys.argv[3], sys.argv[3:])
'''
            args = [sys.executable, '-c', activation, *[str(s.fileno()) for s in listeners],
                    BINARY, '--root', str(root), 'serve']
        else:
            args = [BINARY, '--root', str(root), 'serve', '--bind', '127.0.0.1:2080']
        with (root / 'daemon.log').open('wb') as log:
            daemon = subprocess.Popen(args, stdout=log, stderr=log,
                                      pass_fds=tuple(s.fileno() for s in listeners))
            try:
                for listener in listeners:
                    listener.close()
                for _ in range(30):
                    try:
                        result = request(443 if mode == 'direct' else 2080,
                                         f'/sub/{credential}/uri', context if mode == 'direct' else None)
                        if result.startswith(b'HTTP/1.1 200'):
                            break
                    except OSError:
                        pass
                    time.sleep(0.1)
                else:
                    raise AssertionError('isolated listener did not start')
                if mode == 'direct':
                    flood_check(443, f'/sub/{credential}/uri', 200, context)
                    flood_check(80, '/.well-known/acme-challenge/missing', 404)
                else:
                    flood_check(2080, f'/sub/{credential}/uri', 200, exempt=mode == 'external-proxy')
            finally:
                daemon.terminate()
                daemon.wait(timeout=5)

print('PASSED isolated connection admission, neighbour availability and release after disconnect', flush=True)

with tempfile.TemporaryDirectory(prefix='sbctl-rules-mode-audit-') as tmp:
    root = Path(tmp)
    seed(root, 'ip-fallback')
    kernel = root / 'usr/local/bin/sing-box'
    kernel.parent.mkdir(parents=True)
    kernel.symlink_to('/usr/local/bin/sing-box')
    paths = list((root / 'var/lib/sbctl/artifacts').glob('*')) + [root / 'etc/sbctl/config.toml']
    before = {str(path): path.read_bytes() for path in paths}
    for filename, value in [('sing-box-override.json', '{"rules_mode":false}'),
                            ('clash-override.yaml', 'rules_mode: false\n'),
                            ('sing-box-server.json', '{"rules_mode":null}')]:
        target = root / 'etc/sbctl/overrides' / filename
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)
        result = run(root, 'regenerate', success=False)
        assert result.returncode == (0 if OLD else 2), 'unexpected rules_mode validation outcome'
        if not OLD:
            assert b'rules_mode' in result.stderr
            assert all(Path(name).read_bytes() == data for name, data in before.items()), 'failed validation changed persisted data'
        target.unlink()
    print('CONFIRMED old rules_mode acceptance' if OLD else 'PASSED invalid rules_mode rejected without changing configuration or artifacts', flush=True)
