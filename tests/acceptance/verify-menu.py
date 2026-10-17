#!/usr/bin/env python3
"""Isolated PTY acceptance checks for the interactive sbctl menu.

Usage: python3 tests/acceptance/verify-menu.py /path/to/sbctl
All deployment state is created under temporary directories. No network or
service operation is performed; the only sing-box executable is a local stub.
"""

import argparse
import os
from pathlib import Path
import pty
import select
import signal
import subprocess
import tempfile
import time


parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path, help="path to the sbctl executable")
args = parser.parse_args()
binary = args.binary.resolve()
if not binary.is_file():
    raise SystemExit(f"sbctl binary not found: {binary}")


class Menu:
    def __init__(self, root):
        self.root = Path(root)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execv(str(binary), [str(binary), "--root", str(self.root), "menu"])
        self.output = bytearray()
        self.cursor = 0
        self.wait("=== 主菜单 ===")

    def wait(self, needle, timeout=8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            encoded = needle.encode("utf-8")
            index = self.output.find(encoded, self.cursor)
            if index >= 0:
                self.cursor = index + len(encoded)
                return self.output.decode("utf-8", errors="replace")
            ready, _, _ = select.select([self.fd], [], [], min(0.2, deadline - time.monotonic()))
            if ready:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    break
                self.output.extend(chunk)
        raise AssertionError(f"timed out waiting for {needle!r}; output:\n{self.output.decode('utf-8', 'replace')}")

    def send(self, text=""):
        os.write(self.fd, (text + "\n").encode())

    def choose(self, number, expected):
        self.send(str(number))
        if expected in {
            "概览与诊断", "部署与协议", "订阅与客户端", "流量与通知",
            "服务与更新", "高级与维护", "主菜单",
        }:
            expected = f"=== {expected} ==="
        text = self.wait(expected)
        # A PTY read may stop after the title, before any entries are printed.
        # Inspect a page only once its input prompt proves rendering finished.
        if expected.startswith("=== ") and expected.endswith(" ==="):
            text = self.wait("请选择 [0]: ")
        return text

    def pause(self):
        self.wait("按回车返回菜单")
        self.send()

    def close(self):
        for _ in range(2):
            try:
                self.send("0")
            except OSError:
                break
            deadline = time.monotonic() + 0.6
            while time.monotonic() < deadline:
                try:
                    waited, _ = os.waitpid(self.pid, os.WNOHANG)
                except ChildProcessError:
                    os.close(self.fd)
                    return
                if waited:
                    os.close(self.fd)
                    return
                time.sleep(0.05)
        try:
            os.kill(self.pid, signal.SIGTERM)
            os.waitpid(self.pid, 0)
        finally:
            os.close(self.fd)
        raise AssertionError("menu did not exit after selecting 0")


def invoke(root, *command):
    result = subprocess.run(
        [str(binary), "--root", str(root), *command],
        capture_output=True,
        text=True,
        timeout=15,
    )
    if result.returncode:
        raise AssertionError(f"command failed: {command}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def initialize(root):
    # Even if a confirmation regresses, every service command remains inside
    # this fixture and fails closed instead of reaching the host systemd.
    systemctl = Path(root) / "usr/bin/systemctl"
    systemctl.parent.mkdir(parents=True, exist_ok=True)
    systemctl.write_text("#!/bin/sh\necho unexpected systemctl invocation >&2\nexit 97\n", encoding="utf-8")
    systemctl.chmod(0o700)
    invoke(
        root,
        "config", "init", "--mode", "ip-fallback", "--subscription-host",
        "203.0.113.7", "--http-port", "2080", "--interface", "ens3",
        "--protocol", "vless-reality", "--reality-decoy-sni", "www.cloudflare.com",
    )


def install_checker(root, accepts=True):
    path = Path(root) / "usr/local/bin/sing-box"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\n" + ("exit 0\n" if accepts else "exit 1\n"), encoding="utf-8")
    path.chmod(0o700)
    return path


def snapshot(root):
    root = Path(root)
    files = [root / "etc/sbctl/config.toml", root / "etc/sing-box/config.json"]
    artifact_dir = root / "var/lib/sbctl/artifacts"
    if artifact_dir.exists():
        files.extend(path for path in artifact_dir.iterdir() if path.is_file())
    return {path.relative_to(root).as_posix(): path.read_bytes() for path in files if path.is_file()}


def navigation_and_deployment_visibility():
    with tempfile.TemporaryDirectory(prefix="sbctl-menu-empty-") as temp:
        root = Path(temp)
        menu = Menu(root)
        for number, title in enumerate(
            ["概览与诊断", "部署与协议", "订阅与客户端", "流量与通知", "服务与更新", "高级与维护"], 1
        ):
            text = menu.choose(number, title)
            if number == 2:
                assert "快速安装（默认五协议" in text, "fresh root must offer installation"
            menu.send("0")
            menu.wait("=== 主菜单 ===")
        menu.send("abc")
        menu.wait("无效选择")
        menu.send()
        menu.wait("=== 主菜单 ===")
        menu.close()

        initialize(root)
        menu = Menu(root)
        text = menu.choose(2, "部署与协议")
        assert "快速安装（默认五协议" not in text and "引导安装" not in text, "deployed root must hide fresh-install choices"
        menu.close()
    print("PASS menu navigation, invalid input, and deployment-aware choices", flush=True)


def template_paths():
    # Cancel keeps every published configuration byte; success changes client
    # artifacts while leaving the server configuration untouched.
    with tempfile.TemporaryDirectory(prefix="sbctl-menu-template-") as temp:
        root = Path(temp)
        initialize(root)
        install_checker(root)
        original = snapshot(root)
        menu = Menu(root)
        menu.choose(3, "订阅与客户端")
        menu.choose(1, "客户端内容模板")
        menu.send("1")
        menu.wait("确认应用以上配置")
        menu.send("n")
        menu.wait("模板切换已取消")
        menu.pause()
        after_cancel = snapshot(root)
        assert after_cancel == original, "cancelled template choice must leave config and artifacts byte-identical"
        menu.close()

        menu = Menu(root)
        menu.choose(3, "订阅与客户端")
        menu.choose(1, "客户端内容模板")
        menu.send("1")
        menu.wait("确认应用以上配置")
        menu.send("y")
        menu.wait("客户端模板已切换为 compact")
        menu.pause()
        changed = snapshot(root)
        assert changed["etc/sbctl/config.toml"] != original["etc/sbctl/config.toml"]
        assert changed["var/lib/sbctl/artifacts/sing-box-server.json"] == original["var/lib/sbctl/artifacts/sing-box-server.json"]
        assert changed["var/lib/sbctl/artifacts/subscription-sing-box-full.json"] != original["var/lib/sbctl/artifacts/subscription-sing-box-full.json"]
        menu.close()

    # A dangling client override is refused before configuration/artifact write.
    with tempfile.TemporaryDirectory(prefix="sbctl-menu-invalid-override-") as temp:
        root = Path(temp)
        initialize(root)
        install_checker(root)
        override = root / "etc/sbctl/overrides/sing-box-override.json"
        override.parent.mkdir(parents=True, exist_ok=True)
        override.write_text(
            '{"route":{"rules":[{"domain_suffix":["bad.test"],"outbound":"missing-group"}]}}',
            encoding="utf-8",
        )
        before = snapshot(root)
        menu = Menu(root)
        menu.choose(3, "订阅与客户端")
        menu.choose(1, "客户端内容模板")
        menu.send("1")
        menu.wait("确认应用以上配置")
        menu.send("y")
        menu.wait("模板切换失败")
        menu.pause()
        assert snapshot(root) == before, "invalid override must preserve every published artifact and config"
        menu.close()
    print("PASS template success, cancellation, and dangling-override rollback", flush=True)


def refused_confirmations():
    with tempfile.TemporaryDirectory(prefix="sbctl-menu-refusal-") as temp:
        root = Path(temp)
        initialize(root)
        before = snapshot(root)
        secret = None
        config_text = (root / "etc/sbctl/config.toml").read_text(encoding="utf-8")
        for line in config_text.splitlines():
            if line.startswith("subscription_credential ="):
                secret = line.split('"', 2)[1]
                break
        menu = Menu(root)

        # Default credential rotation.
        menu.choose(3, "订阅与客户端")
        menu.choose(5, "确认轮换默认订阅凭据")
        menu.send("n")
        menu.pause()
        menu.send("0")
        menu.wait("=== 主菜单 ===")

        # Refusing the credential-bearing preview must reveal no secret.
        menu.choose(6, "高级与维护")
        menu.choose(1, "=== 高级与维护 / 配置与覆写 ===")
        menu.choose(6, "确认在终端显示完整配置")
        menu.send("n")
        menu.pause()
        menu.send("0")
        menu.wait("=== 高级与维护 ===")
        menu.send("0")
        menu.wait("=== 主菜单 ===")

        # Restart and reinstall are refused before any service operation.
        menu.choose(5, "服务与更新")
        menu.choose(1, "确认校验配置并重启服务")
        menu.send("n")
        menu.pause()
        menu.send("0")
        menu.wait("=== 主菜单 ===")
        menu.choose(6, "高级与维护")
        menu.choose(5, "备份旧部署，停止并禁用冲突服务")
        menu.send("n")
        menu.pause()
        menu.send("0")
        menu.wait("=== 主菜单 ===")
        menu.choose(6, "高级与维护")
        menu.choose(6, "确认卸载服务和二进制并保留备份")
        menu.send("n")
        menu.pause()
        menu.send("0")
        menu.wait("=== 主菜单 ===")

        # Purge's second confirmation must also be affirmative.
        menu.choose(6, "高级与维护")
        menu.choose(7, "确认彻底清理 sbctl 数据")
        menu.send("y")
        menu.wait("再次确认")
        menu.send("n")
        menu.pause()
        transcript = menu.output.decode("utf-8", errors="ignore")
        if secret:
            assert secret not in transcript, "refused actions must not print the subscription credential"
        menu.close()
        assert snapshot(root) == before, "refused confirmations must leave configuration and artifacts unchanged"
    print("PASS refused rotation, restart, reinstall, uninstall, and double-confirm purge", flush=True)


navigation_and_deployment_visibility()
template_paths()
refused_confirmations()
print("PASS all menu acceptance checks", flush=True)
