"""Reproduce a partial certificate/key update in an isolated temporary root.

python3 reproduce_partial_pin.py /absolute/path/to/sbctl
Only the CLI subprocess receives a file-size limit; no services are touched.
"""
from pathlib import Path
import json
import resource
import signal
import subprocess
import sys
import tempfile

BINARY = str(Path(sys.argv[1]).resolve())


with tempfile.TemporaryDirectory(prefix='sbctl-pair-review-') as tmp:
    root = Path(tmp)

    def cli(*args, before=None):
        return subprocess.run([BINARY, '--root', str(root), *args], capture_output=True,
                              preexec_fn=before, timeout=30)

    result = cli('config', 'init', '--mode', 'direct', '--subscription-host',
                 'sub.example.test', '--interface', 'ens3', '--protocol',
                 'vless-reality', '--reality-decoy-sni', 'www.cloudflare.com')
    assert result.returncode == 0
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
    before = {name: (pinned / name).read_bytes() for name in ['fullchain.pem', 'privkey.pem']}
    generate()
    limit = (live / 'fullchain.pem').stat().st_size + 1
    assert (live / 'privkey.pem').stat().st_size > limit

    def fail_second_file():
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))

    result = cli('certificate', 'verify', before=fail_second_file)
    assert result.returncode == 2 and b'File too large' in result.stderr
    certificate_changed = (pinned / 'fullchain.pem').read_bytes() != before['fullchain.pem']
    key_preserved = (pinned / 'privkey.pem').read_bytes() == before['privkey.pem']
    assert certificate_changed and key_preserved, 'partial pair defect was not reproduced'
    print(json.dumps({'write_error_returned': True, 'certificate_changed': certificate_changed,
                      'old_key_preserved': key_preserved, 'mixed_pair_published': True}))
