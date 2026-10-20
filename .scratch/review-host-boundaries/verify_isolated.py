"""Compare old/new Actions binaries in a disposable network namespace.

unshare --net python3 verify_isolated.py /absolute/path/to/sbctl [--old]
PATH commands are harmless markers; only temporary fixture files are changed.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

BINARY = str(Path(sys.argv[1]).resolve())
OLD = '--old' in sys.argv
subprocess.run(['ip', 'link', 'set', 'lo', 'up'], check=True)


def run(root, args, environment=None, before_exec=None):
    return subprocess.run([BINARY, '--root', str(root), *args], capture_output=True,
                          env=environment, preexec_fn=before_exec, timeout=35)


def write(root, relative, text, executable=False):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    if executable:
        path.chmod(0o700)
    return path


def seed(root, direct=False):
    args = ['config', 'init', '--mode', 'direct' if direct else 'ip-fallback',
            '--subscription-host', 'audit.example.test' if direct else '127.0.0.1',
            '--interface', 'ens3', '--protocol', 'vless-reality',
            '--reality-decoy-sni', 'www.cloudflare.com']
    if not direct:
        args += ['--http-port', '2080']
    assert run(root, args).returncode == 0, 'could not seed fixture'


for operation, program in [('restart', 'systemctl'), ('bbr', 'sysctl'), ('renew', 'certbot')]:
    with tempfile.TemporaryDirectory(prefix='sbctl-boundary-audit-') as tmp:
        root = Path(tmp)
        seed(root, direct=operation == 'renew')
        write(root, 'proc/sys/net/ipv4/tcp_congestion_control', 'cubic\n')
        write(root, 'proc/sys/net/core/default_qdisc', 'pfifo_fast\n')
        write(root, 'usr/local/bin/sing-box', '#!/bin/sh\nexit 0\n', True)
        host_marker = root / 'unexpected-host-command'
        fixture_marker = root / 'expected-fixture-command'
        commands = root / 'host-path'
        write(commands, program, '#!/bin/sh\nprintf called > "$AUDIT_HOST_MARKER"\nexit 0\n', True)
        environment = dict(os.environ, PATH=str(commands), AUDIT_HOST_MARKER=str(host_marker),
                           AUDIT_FIXTURE_MARKER=str(fixture_marker))
        args = {'restart': ['restart'], 'bbr': ['system', 'bbr'],
                'renew': ['certificate', 'renew']}[operation]
        result = run(root, args, environment)
        assert host_marker.exists() == OLD, 'unexpected host command resolution: ' + operation
        if not OLD:
            assert result.returncode == 2 and b'fixture command is missing' in result.stderr
        host_marker.unlink(missing_ok=True)
        write(root, 'usr/bin/' + program,
              '#!/bin/sh\nprintf called > "$AUDIT_FIXTURE_MARKER"\nexit 0\n', True)
        result = run(root, args, environment)
        assert fixture_marker.exists() and not host_marker.exists(), 'rooted fixture not preferred'
        if operation != 'renew':
            assert result.returncode == 0, 'rooted fixture failed: ' + operation
        print(json.dumps({'operation': operation, 'missing_fixture_reached_host_marker': OLD,
                          'provided_fixture_preferred': True}), flush=True)

with tempfile.TemporaryDirectory(prefix='sbctl-rollback-audit-') as tmp:
    root = Path(tmp)
    seed(root)
    active = write(root, 'etc/sing-box/config.json', 'active-file-must-survive\n')
    inode = active.stat().st_ino
    artifacts = {path: path.read_bytes() for path in (root / 'var/lib/sbctl/artifacts').glob('*')}
    checked = root / 'real-server-check-passed'
    write(root, 'usr/local/bin/sing-box', '''#!/bin/sh
/usr/local/bin/sing-box "$@" || exit $?
if [ "$1" = check ] && /bin/grep -q '"users"' "$3"; then
  printf checked > "$AUDIT_CHECK_MARKER"
  exec /usr/bin/prlimit --pid "$PPID" --fsize=0:0
fi
''', True)

    def ignore_file_limit_signal():
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)

    result = run(root, ['regenerate'], dict(os.environ, AUDIT_CHECK_MARKER=str(checked)),
                 ignore_file_limit_signal)
    assert checked.exists() and result.returncode == 2 and b'File too large' in result.stderr
    assert active.exists() == (not OLD), 'unexpected active configuration recovery'
    assert all(path.read_bytes() == data for path, data in artifacts.items()), 'old artifact bytes changed'
    if not OLD:
        assert active.read_text() == 'active-file-must-survive\n'
        assert active.stat().st_ino == inode, 'untouched active configuration was rewritten'
    print(json.dumps({'real_kernel_check_passed': True, 'actual_write_error_injected': True,
                      'active_file_retained_without_rewrite': not OLD,
                      'old_artifacts_unchanged': True}), flush=True)

print('CONFIRMED old defects' if OLD else 'PASSED command and rollback boundaries', flush=True)
