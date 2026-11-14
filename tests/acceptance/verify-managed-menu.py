#!/usr/bin/env python3
"""PTY coverage of typed node drafts, validation, cancellation and groups."""
import json
from pathlib import Path
import runpy
import tempfile

helpers = runpy.run_path(str(Path(__file__).with_name('verify-menu.py')))
Menu, invoke = helpers['Menu'], helpers['invoke']

with tempfile.TemporaryDirectory(prefix='sbctl-managed-menu-') as temp:
    root = Path(temp)
    helpers['initialize'](root)
    helpers['install_checker'](root)
    invoke(root, 'node', 'migrate', '--apply')
    source = root / 'etc/sbctl/nodes.json'
    pointer = root / 'var/lib/sbctl/current-generation.json'
    original = source.read_bytes(), pointer.read_bytes()

    menu = Menu(root)
    menu.choose(2, '节点与组合')
    menu.choose(2, '1. 筛选 / 分页查看节点')
    menu.send('2')
    menu.wait('预设编号')
    menu.send('bad')
    menu.wait('无效预设，请重试')
    menu.send('8')
    menu.wait('节点名称')
    menu.send('draft only')
    menu.wait('端口（回车自动分配）')
    menu.send('bad-port')
    menu.wait('请输入 10000–65535')
    menu.send('')
    menu.wait('应用以上变更')
    menu.send('2')
    menu.wait('已取消，草稿未保存')
    menu.pause()
    assert (source.read_bytes(), pointer.read_bytes()) == original

    menu.send('2')
    menu.wait('预设编号')
    menu.send('8')
    menu.wait('节点名称')
    menu.send('WS TLS')
    menu.wait('端口（回车自动分配）')
    menu.send('')
    menu.wait('应用以上变更')
    menu.send('1')
    menu.wait('已应用并发布订阅')
    menu.pause()
    registry = json.loads(source.read_text())
    assert len(registry['nodes']) == 2
    assert registry['nodes'][1]['name'] == 'WS TLS'
    menu.send('0')
    menu.wait('=== 节点与组合 ===')
    menu.send('0')
    menu.wait('=== 主菜单 ===')
    menu.close()

    menu = Menu(root)
    menu.choose(3, '节点组与订阅')
    menu.choose(2, '1. 查看节点组')
    menu.send('2')
    menu.wait('组名称')
    menu.send('常用')
    menu.wait('节点编号或 ID，逗号分隔')
    menu.send('2')
    menu.wait('策略：1 手动选择')
    menu.send('2')
    menu.wait('应用以上变更')
    menu.send('1')
    menu.wait('已应用并发布订阅')
    menu.pause()
    registry = json.loads(source.read_text())
    assert registry['groups'][0]['strategy'] == 'urltest'
    generation = json.loads(pointer.read_text())
    assert any(name.startswith('group-') for name in generation['files'])
    menu.send('0')
    menu.wait('=== 节点组与订阅 ===')
    menu.send('0')
    menu.wait('=== 主菜单 ===')
    menu.close()

print('PASS managed menu drafts, local validation, cancellation, node creation and groups', flush=True)
