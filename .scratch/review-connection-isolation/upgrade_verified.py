"""Apply the signed Actions build with a timed recovery guard, then commit
only after the workstation's external acceptance has completed.

Requires the v0.0.15 live acceptance helper staged as /root/sbctl-v016-live.py
with its v015 path/unit prefixes changed to v016. No credentials are printed.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import sys

os.umask(0o077)
spec = importlib.util.spec_from_file_location('live', '/root/sbctl-v016-live.py')
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)
POINTER = live.BASE / 'upgrade-backup'


def current_process_digest():
    pid = live.run(['systemctl', 'show', '-p', 'MainPID', '--value', 'sbctl.service']).stdout.strip()
    assert pid.isdigit() and int(pid) > 0
    return live.digest(f'/proc/{pid}/exe')


def begin(candidate, manifest, expected):
    assert os.geteuid() == 0 and live.digest(candidate) == expected
    live.run(['/usr/local/bin/sbctl', 'update', '--check', '--manifest', manifest])
    folder = live.backup()
    baseline_path = folder / 'baseline.json'
    baseline = json.loads(baseline_path.read_text())
    guard = 'sbctl-v016-upgrade-rollback-' + folder.name
    baseline.update(rollback_unit=guard, candidate_sha256=expected)
    baseline_path.write_text(json.dumps(baseline, indent=2))
    POINTER.write_text(str(folder))
    try:
        live.run(['systemd-run', '--unit=' + guard, '--on-active=15m',
                  '--timer-property=AccuracySec=1s', '/usr/bin/python3',
                  '/root/sbctl-v016-live.py', 'restore', folder])
        live.run(['/usr/local/bin/sbctl', 'update', '--manifest', manifest,
                  '--sbctl-artifact', candidate,
                  '--sing-box-artifact', '/usr/local/bin/sing-box'])
        assert live.digest('/usr/local/bin/sbctl') == expected
        assert current_process_digest() == expected
        for path in ['/etc/sbctl/config.toml', '/var/lib/sbctl/state.json', '/usr/local/bin/sing-box']:
            assert live.digest(path) == baseline['files'][path]['sha256'], 'upgrade changed preserved file: ' + path
        live.check_public()
        assert live.run(['/usr/local/bin/sbctl', '--version']).stdout.strip() == 'sbctl 0.0.16'
        report = {'signed_update_passed': True, 'running_sha256': expected,
                  'credentials_configuration_accounting_and_kernel_preserved': True,
                  'public_subscription_passed': True}
        (folder / 'upgrade-report.json').write_text(json.dumps(report, indent=2))
        print(json.dumps(report), flush=True)
        print('READY_FOR_EXTERNAL_ACCEPTANCE', flush=True)
    except BaseException:
        live.restore(folder)
        raise


def commit():
    folder = Path(POINTER.read_text()).resolve()
    assert folder.parent == live.BASE and re.fullmatch(r'backup-\d{8}-\d{6}', folder.name)
    baseline = json.loads((folder / 'baseline.json').read_text())
    assert live.digest('/usr/local/bin/sbctl') == baseline['candidate_sha256']
    assert current_process_digest() == baseline['candidate_sha256']
    live.check_public()
    for unit, state in baseline['timers'].items():
        if state == 'active':
            live.run(['systemctl', 'start', unit])
    live.run(['systemctl', 'stop', baseline['rollback_unit'] + '.timer'])
    assert live.run(['systemctl', 'is-active', baseline['rollback_unit'] + '.timer'], False).stdout.strip() == 'inactive'
    (folder / 'upgrade-committed.json').write_text(json.dumps({'committed': True}))
    print('COMMITTED signed v0.0.16 upgrade; timers restored; rollback guard inactive; private backup retained', flush=True)


if sys.argv[1] == 'begin':
    begin(*sys.argv[2:5])
elif sys.argv[1] == 'commit':
    commit()
else:
    raise SystemExit('expected begin or commit')
