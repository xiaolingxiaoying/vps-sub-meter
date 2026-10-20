import json
import os
import pathlib
import subprocess
import tempfile
import select
import time

SBCTL = pathlib.Path.home() / 'ws/target-vps/debug/sbctl'
CORE = pathlib.Path.home() / 'bin/sing-box-1.14.1'


def cli(root, *args):
    return subprocess.run([str(SBCTL), '--root', str(root), *args], capture_output=True, text=True, timeout=40)


def seed(root):
    result = cli(root, 'config', 'init', '--mode', 'ip-fallback', '--subscription-host', '203.0.113.7', '--http-port', '2080', '--interface', 'ens3', '--protocol', 'vless-reality', '--reality-decoy-sni', 'www.cloudflare.com')
    assert result.returncode == 0, result.stderr


with tempfile.TemporaryDirectory(prefix='sbctl-review-') as directory:
    root = pathlib.Path(directory)
    seed(root)
    result = cli(root, 'rule', 'add', 'fakeip-filter', 'DOMAIN,exact-review.example', 'DOMAIN-KEYWORD,keyword-review', 'DOMAIN-SUFFIX,suffix-review.example', '--sing-box-bin', str(CORE))
    print('fakeip add exit:', result.returncode)
    assert result.returncode == 0, result.stderr
    artifacts = root / 'var/lib/sbctl/artifacts'
    sb = json.loads((artifacts / 'subscription-sing-box-full.json').read_text())
    sb_dns = json.dumps(sb['dns'], ensure_ascii=False)
    clash = (artifacts / 'subscription-clash.yaml').read_text()
    for marker in ('exact-review.example', 'keyword-review', 'suffix-review.example'):
        print('fakeip marker:', marker, 'sing-box dns:', marker in sb_dns, 'mihomo:', marker in clash)
        assert marker in sb_dns and marker in clash

    result = cli(root, 'rule-set', 'add', 'review-format', '--url', 'https://cdn.example/review.srs', '--outbound', 'reject', '--sing-box-bin', '/bin/true')
    print('srs register exit:', result.returncode)
    assert result.returncode == 0, result.stderr
    clash = (artifacts / 'subscription-clash.yaml').read_text()
    assert 'review.srs' not in clash and 'RULE-SET,review-format,' not in clash
    print('SRS excluded from mihomo: True')
    result = cli(root, 'rule-set', 'add', 'review-format', '--url', 'https://cdn.example/review.mrs', '--outbound', 'reject', '--sing-box-bin', '/bin/true')
    print('mrs register exit:', result.returncode)
    assert result.returncode == 0, result.stderr
    sb = json.loads((artifacts / 'subscription-sing-box-full.json').read_text())
    assert 'review.mrs' not in json.dumps(sb)
    assert all(s['tag'] != 'review-format' for s in sb['route']['rule_set'])
    print('MRS excluded from sing-box: True')

    result = cli(root, 'rule-set', 'interval', '18446744073709551615d', '--sing-box-bin', '/bin/true')
    print('overflow interval exit:', result.returncode)
    print('overflow diagnostic:', result.stderr.strip().splitlines()[:3])
    assert result.returncode == 2 and 'panicked' not in result.stderr

with tempfile.TemporaryDirectory(prefix='sbctl-wizard-review-') as directory:
    root = pathlib.Path(directory)
    seed(root)
    fake_systemctl = root / 'usr/bin/systemctl'
    fake_systemctl.parent.mkdir(parents=True)
    fake_systemctl.write_text('#!/bin/sh\nexit 0\n')
    fake_systemctl.chmod(0o700)
    (root / 'sys/class/net/ens3').mkdir(parents=True)
    config_path = root / 'etc/sbctl/config.toml'
    import re
    original = re.search(r'^subscription_credential = "([^"]+)"', config_path.read_text(), re.M).group(1)
    wizard = subprocess.Popen([str(SBCTL), '--root', str(root), 'config', 'wizard', '--sing-box-bin', str(CORE)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # Wait for the first flushed prompt; this proves the wizard has read the config.
    seen = b''
    deadline = time.monotonic() + 10
    while b': ' not in seen:
        assert time.monotonic() < deadline, seen
        if select.select([wizard.stdout], [], [], 0.5)[0]:
            seen += os.read(wizard.stdout.fileno(), 4096)
    rotated = cli(root, 'credential', 'rotate')
    assert rotated.returncode == 0, rotated.stderr
    current = re.search(r'^subscription_credential = "([^"]+)"', config_path.read_text(), re.M).group(1)
    print('credential changed during wizard:', current != original)
    answers = [''] * 17
    answers[14] = 'Asia/Tokyo'
    stdout, stderr = wizard.communicate(('\n'.join(answers + ['y']) + '\n').encode(), timeout=30)
    print('wizard exit:', wizard.returncode)
    if wizard.returncode:
        print('wizard stderr:', stderr.decode())
    after = re.search(r'^subscription_credential = "([^"]+)"', config_path.read_text(), re.M).group(1)
    print('wizard restored invalidated subscription credential:', after == original)
    print('wizard retained rotated subscription credential:', after == current)
    assert wizard.returncode == 2 and after == current and after != original

with tempfile.TemporaryDirectory(prefix='sbctl-rule-binary-review-') as directory:
    root = pathlib.Path(directory)
    source = root / 'rule.json'
    source.write_text(json.dumps({'version': 3, 'rules': [{'domain_suffix': ['review.example']}]}))
    binary = root / 'rule.srs'
    compiled = subprocess.run([str(CORE), 'rule-set', 'compile', str(source), '-o', str(binary)], capture_output=True, text=True)
    assert compiled.returncode == 0, compiled.stderr
    # Check the exact SRS payload with each actual parser, without network requests.
    sing_config = root / 'sing-box.json'
    sing_config.write_text(json.dumps({'route': {'rule_set': [{'type': 'local', 'tag': 'review', 'format': 'binary', 'path': str(binary)}]}}))
    valid = subprocess.run([str(CORE), 'check', '-c', str(sing_config)], capture_output=True, text=True)
    print('real sing-box accepts compiled SRS:', valid.returncode == 0)
    mihomo_config = root / 'mihomo.yaml'
    mihomo_config.write_text('rule-providers:\n  review:\n    type: file\n    behavior: domain\n    format: mrs\n    path: ' + str(binary) + '\nrules:\n  - RULE-SET,review,REJECT\n  - MATCH,DIRECT\n')
    invalid = subprocess.run([str(pathlib.Path.home() / 'bin/mihomo'), '-t', '-d', str(root), '-f', str(mihomo_config)], capture_output=True, text=True, timeout=20)
    print('real mihomo checking SRS declared as MRS exit:', invalid.returncode)
    print('real mihomo diagnostic:', (invalid.stdout + invalid.stderr).strip()[-1400:])
    runtime = subprocess.Popen([str(pathlib.Path.home() / 'bin/mihomo'), '-d', str(root), '-f', str(mihomo_config)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    try:
        logs, _ = runtime.communicate(timeout=3)
    except subprocess.TimeoutExpired:
        runtime.terminate()
        logs, _ = runtime.communicate(timeout=5)
    print('real mihomo provider runtime diagnostic:', logs.decode().strip()[-1800:])
