"""Fetch candidate subscriptions from the workstation; keep credentials in memory."""
import getpass
import http.client
import json
from pathlib import Path
import re
import socket
import ssl
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET

import paramiko
from PIL import Image, ImageDraw
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'vps-acceptance/python-libs'))
import zxingcpp

client = paramiko.SSHClient()
client.load_host_keys('.scratch/vps-acceptance/known_hosts')
client.connect('38.207.187.237', username='root', password=getpass.getpass('VPS password: '),
               look_for_keys=False, allow_agent=False, timeout=20)

try:
    with client.open_sftp() as sftp:
        with sftp.open('/etc/sbctl/config.toml') as source:
            config = tomllib.loads(source.read().decode())
        expected = {}
        for name in sftp.listdir('/var/lib/sbctl/artifacts'):
            if name.startswith('subscription-'):
                with sftp.open('/var/lib/sbctl/artifacts/' + name) as source:
                    expected[name] = source.read()
    host = config['subscription_host']
    token = config['subscription_credential']

    def get(route, credential=token):
        connection = http.client.HTTPSConnection(host, timeout=20, context=ssl.create_default_context())
        connection._create_connection = lambda address, *args, **kwargs: socket.create_connection(('38.207.187.237', 443), timeout=20)
        connection.request('GET', f'/sub/{credential}/{route}')
        response = connection.getresponse()
        answer = response.status, response.read(), {k.lower(): v for k, v in response.getheaders()}
        connection.close()
        return answer

    routes = {
        'sing-box.json': 'subscription-sing-box.json',
        'sing-box-full.json': 'subscription-sing-box-full.json',
        **{f'sing-box-1.{minor}.json': f'subscription-sing-box-1.{minor}.json' for minor in range(10, 15)},
        'clash.yaml': 'subscription-clash.yaml', 'clash-1.18.yaml': 'subscription-clash-1.18.yaml',
        'uri': 'subscription-uri.txt', 'uri.txt': 'subscription-base64-uri.txt',
        'shadowrocket.txt': 'subscription-shadowrocket.txt',
    }
    for route, name in routes.items():
        status, body, headers = get(route)
        assert status == 200 and body == expected[name], 'external artifact mismatch: ' + route
        assert 'subscription-userinfo' in headers
        status, body, _ = get('qr/' + route)
        assert status == 200
        svg = ET.fromstring(body)
        image = Image.new('L', (int(svg.attrib['width']), int(svg.attrib['height'])), 255)
        draw = ImageDraw.Draw(image)
        for path in svg.findall('{http://www.w3.org/2000/svg}path'):
            for x, y, w, h in re.findall(r'M(\d+) (\d+)h(\d+)v(\d+)H\d+V\d+', path.attrib['d']):
                x, y, w, h = map(int, (x, y, w, h))
                draw.rectangle((x, y, x + w - 1, y + h - 1), fill=0)
        results = zxingcpp.read_barcodes(image.resize((image.width * 6, image.height * 6), Image.Resampling.NEAREST))
        assert len(results) == 1 and results[0].text == f'https://{host}/sub/{token}/{route}', 'QR decode mismatch: ' + route
    assert get('index')[0] == 200
    assert get('uri', 'invalid-review-credential')[0] == 404
    assert get('uri?invalid=1')[0] == 404
    print('PASSED external TLS: 12 exact subscription bodies, traffic headers, 12 decoded QR codes, index and invalid routes', flush=True)
    if '--forward' in sys.argv:
        result = subprocess.run(['wsl', '-d', 'Ubuntu-22.04', '--', 'python3',
            '/mnt/c/Users/ranly/Documents/ChatGPT/vps-sub-meter/.scratch/review-2026-10-20-followup/forward_probe.py'],
            input=expected['subscription-sing-box.json'].decode(), text=True, capture_output=True, timeout=260)
        assert result.returncode == 0, 'external five-protocol forwarding failed: ' + result.stdout
        print(result.stdout, end='', flush=True)
finally:
    client.close()
