"""Re-pin the current valid certificate inside the backed-up live acceptance.
Called by the temporary live helper while its timed recovery guard is active.
No credentials, PEM material or private configuration are printed.
"""
import grp
import hashlib
import json
from pathlib import Path
import re
import stat
import subprocess


def run(*args, ok=True):
    result = subprocess.run(args, capture_output=True, timeout=90)
    if ok:
        assert result.returncode == 0, 'live generation operation failed: ' + args[0]
    return result


configuration = Path('/etc/sbctl/config.toml').read_text()
host = re.search(r'^subscription_host = "([^"]+)"', configuration, re.M)[1]
directory = Path('/var/lib/sbctl/certificates') / host
old_inode = directory.stat().st_ino
old_core_pid = run('/usr/bin/systemctl', 'show', 'sing-box.service', '-p', 'MainPID', '--value').stdout
before = {}
for path in directory.iterdir():
    metadata = path.lstat()
    assert stat.S_ISREG(metadata.st_mode), 'live acceptance requires a flat regular certificate directory'
    before[path.name] = (hashlib.sha256(path.read_bytes()).digest(), metadata.st_uid,
                         metadata.st_gid, stat.S_IMODE(metadata.st_mode), metadata.st_ino)
run('/usr/local/bin/sbctl', 'certificate', 'verify')
assert directory.stat().st_ino == old_inode, 'unchanged live pin replaced the watched directory'
assert run('/usr/bin/systemctl', 'show', 'sing-box.service', '-p', 'MainPID', '--value').stdout == old_core_pid
assert {path.name for path in directory.iterdir()} == set(before)
for name, expected in before.items():
    path = directory / name
    metadata = path.stat()
    actual = (hashlib.sha256(path.read_bytes()).digest(), metadata.st_uid,
              metadata.st_gid, stat.S_IMODE(metadata.st_mode), metadata.st_ino)
    assert actual[:4] == expected[:4], 'live certificate bytes or access policy changed'
    if name not in ['fullchain.pem', 'privkey.pem']:
        assert actual[4] == expected[4], 'auxiliary certificate inode changed'
run('/usr/local/bin/sbctl', 'restart')
run('/usr/bin/systemctl', 'is-active', '--quiet', 'sbctl.service', 'sing-box.service', 'sbctl-http.socket')
key = directory / 'privkey.pem'
assert key.stat().st_uid == 0 and key.stat().st_gid == grp.getgrnam('sbctl-cert').gr_gid
assert stat.S_IMODE(key.stat().st_mode) == 0o640
lock = directory.parent / '.operation.lock'
assert lock.stat().st_uid == 0 and stat.S_IMODE(lock.stat().st_mode) == 0o600
for account in ['sbctl', 'sing-box']:
    run('/usr/sbin/runuser', '-u', account, '--', '/usr/bin/test', '-r', str(key))
    assert run('/usr/sbin/runuser', '-u', account, '--', '/usr/bin/test', '-r', str(lock), ok=False).returncode == 1
print(json.dumps({'unchanged_pin_keeps_directory_and_core': True, 'material_and_auxiliary_files_preserved': True,
                  'real_service_restart': True, 'both_accounts_read_keys_not_publication_lock': True}), flush=True)
