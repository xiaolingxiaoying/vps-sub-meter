"""Check API rollback certificate access with real Linux UIDs in a disposable chroot.

Usage: sudo python3 check-api-certificate-permissions.py /absolute/path/to/sbctl
No host accounts, services, certificate material or network listeners are changed.
"""
import os
import re
import shutil
import stat
import subprocess
import tempfile
import sys
from pathlib import Path

SBCTL = Path(sys.argv[1]).resolve()


def write(root, relative, body, executable=False):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    if executable:
        path.chmod(0o700)
    return path


def copy_program(root, source, relative=None):
    source = Path(source)
    destination = root / (relative or source.relative_to('/'))
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    dependencies = subprocess.run(['ldd', str(source)], capture_output=True, text=True)
    for dependency in re.findall(r'(/[^\s()]+)', dependencies.stdout):
        origin = Path(dependency)
        target = root / origin.relative_to('/')
        target.parent.mkdir(parents=True, exist_ok=True)
        if not target.exists():
            shutil.copy2(origin, target)


def main():
    assert os.geteuid() == 0
    with tempfile.TemporaryDirectory(prefix='sbctl-review-tls-') as tmp:
        root = Path(tmp)
        root.chmod(0o755)
        for program in ['/bin/sh', '/usr/bin/chown', '/usr/bin/chmod', '/usr/bin/find',
                        '/usr/bin/test']:
            copy_program(root, program)
        copy_program(root, SBCTL, 'usr/local/bin/sbctl')
        write(root, 'etc/passwd', 'root:x:0:0:root:/root:/bin/sh\n'
              'sbctl:x:65000:65000:sbctl:/:/bin/false\n'
              'sing-box:x:65001:65001:sing-box:/:/bin/false\n')
        write(root, 'etc/group', 'root:x:0:\nsbctl:x:65000:\nsing-box:x:65001:\n'
              'sbctl-cert:x:65002:sbctl,sing-box\n')
        write(root, 'usr/local/bin/sing-box', '#!/bin/sh\nexit 0\n', True)
        write(root, 'usr/bin/systemctl', '#!/bin/sh\nexit 1\n', True)
        write(root, 'etc/systemd/system/sbctl.service',
              '[Unit]\nDescription=sbctl private subscription service\n')
        (root / 'tmp').mkdir(mode=0o1777)
        (root / 'dev').mkdir()
        os.mknod(root / 'dev/null', stat.S_IFCHR | 0o666, os.makedev(1, 3))
        environment = dict(os.environ, PATH='/usr/local/bin:/usr/bin:/bin')

        def chroot(*args):
            return subprocess.run(['/usr/sbin/chroot', str(root), *args],
                                  capture_output=True, text=True, env=environment, timeout=30)

        result = chroot('/usr/local/bin/sbctl', 'config', 'init', '--mode', 'direct',
                        '--subscription-host', 'sub.example.test', '--interface', 'ens3',
                        '--protocol', 'vless-reality', '--reality-decoy-sni', 'www.cloudflare.com')
        assert result.returncode == 0, result.stderr
        write(root, 'etc/sing-box/config.json',
              (root / 'var/lib/sbctl/artifacts/sing-box-server.json').read_text())
        # Exercise a first real pin. Pre-seeding the pinned directory's modes
        # hides failures to grant traversal of newly created storage parents.
        live_dir = root / 'etc/letsencrypt/live/sub.example.test'
        live_dir.mkdir(parents=True)
        subprocess.run(['/usr/bin/openssl', 'req', '-x509', '-newkey', 'rsa:2048',
                        '-nodes', '-days', '1', '-subj', '/CN=sub.example.test',
                        '-addext', 'subjectAltName=DNS:sub.example.test',
                        '-keyout', str(live_dir / 'privkey.pem'),
                        '-out', str(live_dir / 'fullchain.pem')],
                       check=True, capture_output=True, timeout=30)
        result = chroot('/usr/local/bin/sbctl', 'certificate', 'verify')
        assert result.returncode == 0, result.stderr
        cert_dir = root / 'var/lib/sbctl/certificates/sub.example.test'
        for directory in [cert_dir.parent, cert_dir]:
            assert directory.stat().st_gid == 65002, 'fresh pin must grant the shared certificate group'
            assert stat.S_IMODE(directory.stat().st_mode) == 0o750, 'fresh pin must grant parent traversal'
        result = chroot('/usr/local/bin/sbctl', 'regenerate')
        assert result.returncode == 0, result.stderr
        for directory in [root / 'var', root / 'var/lib']:
            directory.chmod(0o755)
        key = '/var/lib/sbctl/certificates/sub.example.test/privkey.pem'

        def readable(uid=65000):
            def enter_daemon():
                os.chroot(root)
                os.chdir('/')
                os.setgroups([65002])
                os.setgid(uid)
                os.setuid(uid)
            probe = subprocess.run(['/usr/bin/test', '-r', key], preexec_fn=enter_daemon,
                                   capture_output=True, text=True, timeout=5)
            assert not probe.stderr, probe.stderr
            return probe.returncode == 0

        assert readable(), 'initial Direct permission policy must permit the subscription daemon'
        assert readable(65001), 'first pin must permit the sing-box data plane'
        before = (root / key.lstrip('/')).stat()
        result = chroot('/usr/local/bin/sbctl', 'sing-box', 'api', 'enable', '--port', '19090')
        assert result.returncode == 2, result.stderr
        after = (root / key.lstrip('/')).stat()
        assert readable(), 'API recovery must preserve certificate access for the subscription daemon'
        assert before.st_gid == after.st_gid == 65002
        assert stat.S_IMODE(after.st_mode) == 0o640
        print('Fresh Direct pin grants both service UIDs access; API rollback preserves root:sbctl-cert 0640')


if __name__ == '__main__':
    main()
