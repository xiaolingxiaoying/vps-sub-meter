"""Read private outbound data on stdin; print only protocol names and HTTP status."""
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

nodes = json.load(sys.stdin)['outbounds']
for outbound in nodes:
    protocol = outbound['type']
    with tempfile.TemporaryDirectory(prefix='sbctl-v015-forward-') as directory:
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        outbound['tag'] = 'test-proxy'
        config = {'log': {'level': 'error'}, 'inbounds': [
            {'type': 'socks', 'tag': 'socks-in', 'listen': '127.0.0.1', 'listen_port': port}],
            'outbounds': [outbound], 'route': {'final': 'test-proxy'}}
        path = Path(directory) / 'client.json'
        path.write_text(json.dumps(config))
        path.chmod(0o600)
        with open(Path(directory) / 'client.log', 'w') as log:
            child = subprocess.Popen(['/home/ly/bin/sing-box-1.14.1', 'run', '-c', str(path)],
                                     stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 10
                while True:
                    try:
                        with socket.create_connection(('127.0.0.1', port), timeout=0.2):
                            break
                    except OSError:
                        assert time.monotonic() < deadline and child.poll() is None, protocol + ' client startup failed'
                        time.sleep(0.05)
                result = subprocess.run(['curl', '--silent', '--show-error', '--socks5-hostname',
                    f'127.0.0.1:{port}', '--connect-timeout', '15', '--max-time', '40',
                    '--output', '/dev/null', '--write-out', '%{http_code}',
                    'https://www.gstatic.com/generate_204'], capture_output=True, text=True, timeout=45)
                assert result.returncode == 0 and result.stdout == '204', protocol + ' forwarding failed'
                print(protocol + ': HTTPS 204', flush=True)
            finally:
                child.terminate()
                child.wait(timeout=5)
