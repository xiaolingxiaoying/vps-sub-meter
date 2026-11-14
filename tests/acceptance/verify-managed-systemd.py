#!/usr/bin/env python3
"""Destructive systemd test, only inside the disposable acceptance container."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

assert os.environ.get('SBCTL_MANAGED_SYSTEMD_TEST') == '1', 'disposable acceptance environment required'
binary = sys.argv[1]
source = Path('/etc/sbctl/nodes.json')
pointer = Path('/var/lib/sbctl/current-generation.json')
journal = Path('/var/lib/sbctl/deployment-transaction.json')
xray = Path('/usr/local/bin/sbctl-xray')
xray.write_text('''#!/bin/sh
case "$1:$2" in
 version:*) echo 'Xray 26.10.31' ;;
 run:-test) exit 0 ;;
 run:*) while :; do sleep 3600; done ;;
 *) exit 2 ;;
esac
''')
xray.chmod(0o755)

def run(*args, success=True):
    result = subprocess.run([binary, *args], capture_output=True, text=True, timeout=45)
    assert (result.returncode == 0) == success, 'managed systemd command did not have the expected outcome'
    return result

def mode(expected):
    for name, enabled in [('sing-box.service', expected != 'xray'), ('sbctl-xray.service', expected != 'sing-box')]:
        assert (subprocess.run(['systemctl', 'is-active', '--quiet', name]).returncode == 0) == enabled
    assert subprocess.run(['systemctl', 'is-active', '--quiet', 'sbctl.service']).returncode == 0

run('node', 'migrate', '--apply')
registry = json.loads(source.read_text())
unsupported = [node['id'] for node in registry['nodes']
               if node['data']['settings']['protocol'] in ('hysteria2', 'tuic', 'anytls')]
run('core', 'mode', 'dual', '--apply')
mode('dual')
run('core', 'mode', 'xray', '--disable', ','.join(unsupported), '--apply')
mode('xray')
run('core', 'mode', 'sing-box', '--apply')
mode('sing-box')
run('core', 'mode', 'dual', '--apply')
mode('dual')
assert 'PartOf=sing-box.service' not in Path('/etc/systemd/system/sbctl-xray.service').read_text()
assert subprocess.run(['runuser', '-u', 'sbctl', '--', 'test', '-r', source]).returncode != 0

node = json.loads(source.read_text())['nodes'][0]['id']
before = source.read_bytes(), pointer.read_bytes()
run('node', 'edit', node, '--listen', '192.0.2.200', '--apply', success=False)
assert (source.read_bytes(), pointer.read_bytes()) == before
mode('dual')

with open(os.devnull, 'wb') as sink:
    process = subprocess.Popen([binary, 'node', 'edit', node, '--name', 'interrupted', '--apply'], stdout=sink, stderr=sink)
    deadline = time.monotonic() + 15
    while not journal.exists() and process.poll() is None and time.monotonic() < deadline:
        time.sleep(0.005)
    assert journal.exists(), 'transaction journal was not persisted before activation'
    process.kill()
    process.wait()
run('core', 'recover')
assert (source.read_bytes(), pointer.read_bytes()) == before
assert not journal.exists()
mode('dual')
print('PASS managed systemd modes, source permissions, startup failure rollback and interrupted recovery', flush=True)
