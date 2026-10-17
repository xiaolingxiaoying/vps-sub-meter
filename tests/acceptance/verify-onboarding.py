#!/usr/bin/env python3
"""PTY smoke for guided and quick onboarding; cancellation must leave no deployment."""

import argparse
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import subprocess
import tempfile
import termios
import time


parser = argparse.ArgumentParser()
parser.add_argument("--cli", required=True, type=Path)
args = parser.parse_args()
cli = args.cli.resolve()


def make_fixture(base: Path, name: str) -> tuple[Path, Path]:
    root = base / name
    for relative in (
        "etc",
        "run/systemd/system",
        "proc/net",
        "sys/class/net/ens3",
    ):
        (root / relative).mkdir(parents=True, exist_ok=True)
    (root / "etc/os-release").write_text(
        'ID=debian\nVERSION_ID="12"\n', encoding="utf-8"
    )
    (root / "proc/net/route").write_text(
        "Iface\tDestination\tGateway\tFlags\n"
        "ens3\t00000000\t00000000\t0003\n",
        encoding="utf-8",
    )
    checker = root / "fixture-sing-box"
    checker.write_text(
        "#!/bin/sh\n"
        "if [ \"${1:-}\" = version ]; then echo 'sing-box version 1.14.1'; fi\n"
        "exit 0\n",
        encoding="utf-8",
    )
    checker.chmod(0o700)
    return root, checker


def run_tty(command: list[str], answers: list[str]) -> str:
    master, slave = pty.openpty()

    def attach_controlling_terminal() -> None:
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    process = subprocess.Popen(
        command,
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        preexec_fn=attach_controlling_terminal,
    )
    os.close(slave)
    os.write(master, ("\n".join(answers) + "\n").encode())
    output = bytearray()
    deadline = time.monotonic() + 40
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.2)
            if ready:
                try:
                    chunk = os.read(master, 8192)
                except OSError:
                    chunk = b""
                if chunk:
                    output.extend(chunk)
            if process.poll() is not None:
                # Drain text already buffered by the terminal before returning.
                while select.select([master], [], [], 0)[0]:
                    try:
                        chunk = os.read(master, 8192)
                    except OSError:
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
                break
        else:
            process.send_signal(signal.SIGKILL)
            raise TimeoutError(f"onboarding timed out: {command}")
        if process.returncode != 0:
            raise AssertionError(
                f"CLI exited {process.returncode}: {output.decode(errors='replace')}"
            )
        return output.decode(errors="replace")
    finally:
        os.close(master)
        if process.poll() is None:
            process.kill()
            process.wait()


def assert_cancelled(root: Path) -> None:
    for relative in (
        "etc/sbctl/config.toml",
        "etc/sing-box/config.json",
        "var/lib/sbctl/ownership",
        "usr/local/bin/sing-box",
        "etc/systemd/system/sbctl.service",
    ):
        assert not (root / relative).exists(), f"cancel wrote {relative}"


with tempfile.TemporaryDirectory(prefix="sbctl-onboarding-") as temp:
    base = Path(temp)
    guided_root, guided_checker = make_fixture(base, "guided")
    guided_answers = [
        "",  # subscription mode: direct
        "sub.example.test",
        "",  # proxy host
        "",  # Certbot email
        "y",  # confirm skipping email registration
        "",  # detected interface
        "",  # certificate mode
        *("" for _ in range(5)),  # keep all five managed protocols enabled
        *("" for _ in range(5)),  # allocate protocol ports
        "www.cloudflare.com",  # Reality decoy SNI
        "",  # protocol SNI
        "n",  # skip template customization
        "n",  # skip accounting customization
        "n",  # cancel final summary
    ]
    guided_output = run_tty(
        [
            str(cli), "--root", str(guided_root), "install", "--guided",
            "--sing-box-bin", str(guided_checker), "--no-start",
        ],
        guided_answers,
    )
    for expected in (
        "client content template: standard",
        "monthly traffic limit: 0 bytes",
        "accounting policy: natural-month",
        "VPS refresh timezone: America/Los_Angeles",
        "client display timezone: Asia/Shanghai",
        "installation cancelled; the host was not changed",
    ):
        assert expected in guided_output, f"guided summary missed {expected!r}"
    assert_cancelled(guided_root)
    print("PASS guided defaults and cancellation")

    quick_root, quick_checker = make_fixture(base, "quick")
    quick_answers = [
        "y",  # customize client template
        "compact",
        "y",  # customize accounting independently
        "128",  # GiB limit
        "1",  # Los Angeles accounting timezone
        "",  # Shanghai display timezone default
        "anchored-month",
        "2026-10-20T12:30",
        "n",  # cancel final summary
    ]
    quick_output = run_tty(
        [
            str(cli), "--root", str(quick_root), "install",
            "--mode", "external-proxy",
            "--subscription-host", "sub.example.test",
            "--interface", "ens3",
            "--reality-decoy-sni", "www.cloudflare.com",
            "--sing-box-bin", str(quick_checker), "--no-start",
        ],
        quick_answers,
    )
    for expected in (
        "client content template: compact",
        "monthly traffic limit: 137438953472 bytes",
        "accounting policy: anchored-month",
        "VPS refresh timezone: America/Los_Angeles",
        "client display timezone: Asia/Shanghai",
        "anchored reset: 2026-10-20T12:30",
        "installation cancelled; the host was not changed",
    ):
        assert expected in quick_output, f"quick summary missed {expected!r}"
    assert_cancelled(quick_root)
    print("PASS quick custom template/accounting and cancellation")

    # The two opt-in prompts must be independently usable, not one combined gate.
    for name, answers, expected in [
        ("template-only", ["y", "compact", "n", "n"], ["client content template: compact", "monthly traffic limit: 0 bytes"]),
        ("accounting-only", ["n", "y", "64", "", "", "natural-month", "n"], ["client content template: standard", "monthly traffic limit: 68719476736 bytes"]),
    ]:
        root, checker = make_fixture(base, name)
        output = run_tty([
            str(cli), "--root", str(root), "install", "--mode", "external-proxy",
            "--subscription-host", "sub.example.test", "--interface", "ens3",
            "--reality-decoy-sni", "www.cloudflare.com", "--sing-box-bin", str(checker), "--no-start",
        ], answers)
        for text in expected:
            assert text in output, f"{name} summary missed {text!r}"
        assert_cancelled(root)
        print(f"PASS {name} independent optional customization")
