"""Exercise the real sing-box /memory stream using an isolated CLI configuration."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import time

from verify_fixed import seed, run

with tempfile.TemporaryDirectory(prefix='sbctl-memory-stream-') as directory:
    root = Path(directory)
    seed(root)
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    secret = secrets.token_hex(32)
    config_path = root / 'etc/sbctl/config.toml'
    with config_path.open('a') as config:
        config.write(f'\n[server_clash_api]\nport = {port}\nsecret = "{secret}"\n')
    core_path = root / 'core.json'
    core_path.write_text(json.dumps({'log': {'level': 'error'}, 'outbounds': [
        {'type': 'direct', 'tag': 'direct'}], 'experimental': {'clash_api': {
            'external_controller': f'127.0.0.1:{port}', 'secret': secret}}}))
    core_path.chmod(0o600)
    with open(root / 'core.log', 'w') as log:
        process = subprocess.Popen([os.environ['CORE_BIN'], 'run', '-c', str(core_path)], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while True:
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=0.2):
                        break
                except OSError:
                    assert process.poll() is None and time.monotonic() < deadline
                    time.sleep(0.05)
            start = time.monotonic()
            result = run(root, 'sing-box', 'api', 'status')
            assert '接口可达' in result.stdout
            assert time.monotonic() - start < 5
            print('PASSED real sing-box memory stream: API status returned the first snapshot', flush=True)
        finally:
            process.terminate()
            process.wait(timeout=5)
