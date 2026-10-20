"""Check privileged writes against links planted by a real unprivileged UID.

sudo python3 check-managed-fs-boundaries.py /absolute/path/to/sbctl [--old]
Only disposable temporary trees are modified; no accounts or services change.
--old confirms the defects in a previous release instead of expecting a fix.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

BINARY = str(Path(sys.argv[1]).resolve())
OLD = '--old' in sys.argv
UID = 65000
GID = 65000
assert os.geteuid() == 0, 'run this isolated permission check as root'


def unprivileged():
    os.setgroups([GID])
    os.setgid(GID)
    os.setuid(UID)


def initialize(root):
    return subprocess.run([BINARY, '--root', str(root), 'config', 'init',
        '--mode', 'ip-fallback', '--subscription-host', '127.0.0.1',
        '--http-port', '2080', '--interface', 'ens3', '--protocol', 'vless-reality',
        '--reality-decoy-sni', 'www.cloudflare.com'], capture_output=True, timeout=30)


for case in ['artifact-parent', 'self-signed-key']:
    with tempfile.TemporaryDirectory(prefix='sbctl-managed-uid-') as tmp:
        base = Path(tmp)
        base.chmod(0o755)
        root = base / 'deployment'
        parent = root / 'var/lib/sbctl'
        parent.mkdir(parents=True)
        os.chown(parent, 0, GID)
        parent.chmod(0o1771)
        protected = base / 'protected'
        protected.mkdir(mode=0o700)
        victim = protected / ('subscription-uri.txt' if case == 'artifact-parent' else 'private-state')
        victim.write_bytes(b'root-only sentinel\n')
        victim.chmod(0o600)
        if case == 'artifact-parent':
            controlled = parent / 'artifacts'
        else:
            controlled = parent / 'certificates/www.bing.com'
        controlled.mkdir(parents=True)
        os.chown(controlled, UID, GID)
        controlled.chmod(0o700)
        actor = '''import os,sys
from pathlib import Path
controlled,protected,victim = map(Path,sys.argv[1:4])
assert not os.access(protected,os.W_OK) and not os.access(victim,os.R_OK)
if sys.argv[4] == 'artifact-parent':
    controlled.rename(controlled.with_name('artifacts-saved'))
    controlled.symlink_to(protected,target_is_directory=True)
else:
    (controlled/'key.pem').symlink_to(victim)
'''
        subprocess.run(['/usr/bin/python3', '-c', actor, str(controlled), str(protected),
                        str(victim), case], preexec_fn=unprivileged, check=True, timeout=15)
        result = initialize(root)
        changed = victim.read_bytes() != b'root-only sentinel\n'
        assert changed == OLD, 'unexpected privileged write outcome: ' + case
        assert result.returncode == (0 if OLD else 2), 'unexpected CLI result: ' + case
        assert stat.S_IMODE(parent.stat().st_mode) == 0o1771
        if not OLD:
            assert len(list(protected.iterdir())) == 1
            assert stat.S_IMODE(victim.stat().st_mode) == 0o600
            assert victim.stat().st_uid == 0
        print(json.dumps({'case': case, 'unprivileged_actor_cannot_access_target': True,
                          'root_owned_target_changed': changed}), flush=True)

with tempfile.TemporaryDirectory(prefix='sbctl-managed-normal-') as tmp:
    root = Path(tmp)
    parent = root / 'var/lib/sbctl'
    parent.mkdir(parents=True)
    parent.chmod(0o1771)
    result = initialize(root)
    assert result.returncode == 0, 'normal initialization failed'
    assert stat.S_IMODE(parent.stat().st_mode) == 0o1771
    assert (parent / 'artifacts/subscription-uri.txt').is_file()
    assert stat.S_IMODE((parent / 'certificates/www.bing.com/key.pem').stat().st_mode) == 0o640
print('CONFIRMED old privileged-write defects' if OLD else
      'PASSED real-UID link rejection, protected target integrity, normal initialization and modes', flush=True)
