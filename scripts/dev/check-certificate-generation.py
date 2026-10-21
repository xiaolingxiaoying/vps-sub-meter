"""Check whole-pair publication after an actual CLI write failure.

python3 check-certificate-generation.py /absolute/sbctl [--old]
sudo unshare --net python3 check-certificate-generation.py /absolute/sbctl --serve
Only temporary trees and the faulted CLI's file-size limit are changed.
"""
import http.client
import json
import os
from pathlib import Path
import re
import resource
import signal
import socket
import ssl
import stat
import subprocess
import sys
import tempfile
import time

BINARY = str(Path(sys.argv[1]).resolve())
OLD = '--old' in sys.argv
SERVE = '--serve' in sys.argv
if SERVE:
    assert os.geteuid() == 0, 'private namespace listener check requires root'
    assert os.stat('/proc/self/ns/net').st_ino != os.stat('/proc/1/ns/net').st_ino, 'use unshare --net'
    subprocess.run(['/usr/sbin/ip', 'link', 'set', 'lo', 'up'], check=True)

with tempfile.TemporaryDirectory(prefix='sbctl-generation-') as tmp:
    root = Path(tmp)

    def cli(*args, before=None):
        return subprocess.run([BINARY, '--root', str(root), *args], capture_output=True,
                              preexec_fn=before, timeout=30)

    result = cli('config', 'init', '--mode', 'direct', '--subscription-host',
                 'sub.example.test', '--interface', 'ens3', '--protocol',
                 'vless-reality', '--reality-decoy-sni', 'www.cloudflare.com')
    assert result.returncode == 0, 'fixture initialization failed'
    live = root / 'etc/letsencrypt/live/sub.example.test'
    live.mkdir(parents=True)

    def generate():
        subprocess.run(['/usr/bin/openssl', 'req', '-x509', '-newkey', 'rsa:2048',
                        '-nodes', '-days', '1', '-subj', '/CN=sub.example.test',
                        '-addext', 'subjectAltName=DNS:sub.example.test',
                        '-keyout', str(live / 'privkey.pem'),
                        '-out', str(live / 'fullchain.pem')], check=True,
                       capture_output=True, timeout=30)

    generate()
    assert cli('certificate', 'verify').returncode == 0
    pinned = root / 'var/lib/sbctl/certificates/sub.example.test'
    names = ['fullchain.pem', 'privkey.pem']
    before = {name: (pinned / name).read_bytes() for name in names}
    inode = pinned.stat().st_ino
    # Preserve a separate self-signed pair and an administrator's flat file.
    extras = {'cert.pem': before['fullchain.pem'], 'key.pem': before['privkey.pem'],
              'operator-note': b'keep this auxiliary file\n'}
    for name, body in extras.items():
        (pinned / name).write_bytes(body)
        (pinned / name).chmod(0o600)
    extra_metadata = {name: (pinned / name).stat() for name in extras}
    generate()
    limit = (live / 'fullchain.pem').stat().st_size + 1
    assert (live / 'privkey.pem').stat().st_size > limit

    def fail_key_write():
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))

    result = cli('certificate', 'verify', before=fail_key_write)
    assert result.returncode == 2 and b'File too large' in result.stderr
    certificate_changed = (pinned / 'fullchain.pem').read_bytes() != before['fullchain.pem']
    assert certificate_changed == OLD
    assert (pinned / 'privkey.pem').read_bytes() == before['privkey.pem']
    if not OLD:
        assert pinned.stat().st_ino == inode
        assert not list(pinned.parent.glob('.sbctl-generation.*'))
        status = cli('certificate', 'status')
        assert status.returncode == 0 and b'the service is still serving' in status.stdout, 'old pinned pair must still validate'
    print(json.dumps({'write_failure_injected': True, 'certificate_changed': certificate_changed,
                      'old_key_preserved': True, 'old_pair_preserved': not OLD}), flush=True)

    # Prepare legal traffic data for a real subscription response after restart.
    for relative, body in [('sys/class/net/ens3/statistics/rx_bytes', '100\n'),
                           ('sys/class/net/ens3/statistics/tx_bytes', '200\n'),
                           ('proc/sys/kernel/random/boot_id', 'test-boot\n')]:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
    configuration = (root / 'etc/sbctl/config.toml').read_text()
    token = re.search(r'^subscription_credential = "([^"]+)"', configuration, re.M)[1]

    def restart_and_probe(trusted_pem):
        # Model systemd's socket handoff inside this private namespace.
        listeners = []
        for port in [80, 443]:
            listener = socket.socket()
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(('127.0.0.1', port))
            listener.listen(16)
            listeners.append(listener)
        launcher = '''import fcntl,os,sys
sources=[fcntl.fcntl(int(fd),fcntl.F_DUPFD_CLOEXEC,10) for fd in sys.argv[3:5]]
for source,target in zip(sources,[3,4]):
    os.dup2(source,target,inheritable=True)
os.environ['LISTEN_PID']=str(os.getpid())
os.environ['LISTEN_FDS']='2'
os.execv(sys.argv[1],[sys.argv[1],'--root',sys.argv[2],'serve'])
'''
        diagnostics = root / 'serve-stderr.log'
        try:
            with diagnostics.open('wb') as errors:
                process = subprocess.Popen([sys.executable, '-c', launcher, BINARY, str(root),
                    *[str(listener.fileno()) for listener in listeners]],
                    pass_fds=tuple(listener.fileno() for listener in listeners),
                    stdout=subprocess.DEVNULL, stderr=errors)
        finally:
            for listener in listeners:
                listener.close()
        try:
            context = ssl.create_default_context(cadata=trusted_pem.decode())
            deadline = time.monotonic() + 8
            while True:
                connection = http.client.HTTPSConnection('sub.example.test', timeout=1, context=context)
                connection._create_connection = lambda *args, **kwargs: socket.create_connection(('127.0.0.1', 443), timeout=1)
                try:
                    connection.request('GET', f'/sub/{token}/uri')
                    response = connection.getresponse()
                    assert response.status == 200
                    assert response.read() == (root / 'var/lib/sbctl/artifacts/subscription-uri.txt').read_bytes()
                    break
                except (OSError, ssl.SSLError, http.client.HTTPException):
                    assert process.poll() is None and time.monotonic() < deadline, (
                        'restarted TLS could not load the preserved pair: '
                        + diagnostics.read_text().replace(token, '[redacted]')[:2000])
                    time.sleep(0.05)
                finally:
                    connection.close()
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    if SERVE and not OLD:
        restart_and_probe(before['fullchain.pem'])
        print('PASSED restart TLS and exact subscription after failed pin', flush=True)
    # After the fault is removed a normal retry must publish both new files.
    assert cli('certificate', 'verify').returncode == 0
    for name in names:
        assert (pinned / name).read_bytes() == (live / name).read_bytes()
    for name, body in extras.items():
        path = pinned / name
        assert path.read_bytes() == body
        assert stat.S_IMODE(path.stat().st_mode) == stat.S_IMODE(extra_metadata[name].st_mode)
        assert path.stat().st_ino == extra_metadata[name].st_ino
    if SERVE and not OLD:
        restart_and_probe((live / 'fullchain.pem').read_bytes())
        print('PASSED restart TLS and exact subscription after successful retry', flush=True)
    print('CONFIRMED old partial-pair defect' if OLD else
          'PASSED whole-pair write failure, recovery and auxiliary file preservation', flush=True)
