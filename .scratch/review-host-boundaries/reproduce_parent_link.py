from pathlib import Path
import subprocess
import sys
import tempfile

binary = sys.argv[1] if len(sys.argv) > 1 else '/home/ly/ws/target-vps/debug/sbctl'

with tempfile.TemporaryDirectory(prefix='sbctl-parent-link-research-') as tmp:
    base = Path(tmp)
    root = base / 'deployment'
    outside = base / 'outside'
    outside.mkdir()
    parent = root / 'var/lib/sbctl'
    parent.mkdir(parents=True)
    (parent / 'artifacts').symlink_to(outside, target_is_directory=True)
    result = subprocess.run([binary, '--root', str(root),
        'config', 'init', '--mode', 'ip-fallback', '--subscription-host', '127.0.0.1',
        '--http-port', '2080', '--interface', 'ens3', '--protocol', 'vless-reality',
        '--reality-decoy-sni', 'www.cloudflare.com'], capture_output=True, text=True)
    print('command_exit=', result.returncode)
    print('artifact_files_created_outside_deployment_root=', len(list(outside.iterdir())))
    assert result.returncode == 0 and (outside / 'subscription-uri.txt').is_file()
    print('CONFIRMED path-based artifact initialization follows parent link; live race reachability remains unproven')
