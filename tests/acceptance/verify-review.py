#!/usr/bin/env python3
"""Read-only-host smoke tests for a candidate sbctl binary (Python stdlib only).

Usage: python3 tests/acceptance/verify-review.py /path/to/sbctl \
           [--sing-box /path/to/sing-box-1.14.2]

Run inside a private network namespace with loopback enabled when testing a
shared VPS. This script does not create namespaces or change network settings.
All configuration/counters/shims live in a mode-0700 temporary --root; the sole
listener binds 127.0.0.1 on an ephemeral port. No install, service mutation,
sysctl, firewall, or host /etc operation is performed. Optional sing-box executes
only `version` and `check`, never `run`. Captured secrets are never printed.
"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit


class SmokeFailure(Exception):
    pass


def require(condition, label):
    if not condition:
        raise SmokeFailure(label)


def json_document(contents, label):
    try:
        return json.loads(contents)
    except json.JSONDecodeError:
        raise SmokeFailure("invalid JSON: " + label) from None


def write(path, contents, executable=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o700 if executable else 0o600)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--sing-box", type=Path, help="optional real core; version/check only")
    args = parser.parse_args()
    require(os.name == "posix", "review smoke requires Linux/WSL")
    binary = args.binary.resolve()
    require(binary.is_file(), "candidate binary is missing")
    real_core = args.sing_box.resolve() if args.sing_box else None
    require(real_core is None or real_core.is_file(), "real core binary is missing")

    with tempfile.TemporaryDirectory(prefix="sbctl-review-") as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        denied_log = root / "unexpected-command.log"
        environment = dict(os.environ)
        environment["PATH"] = str(root / "usr/bin") + os.pathsep + environment.get("PATH", "")
        environment["SBCTL_REVIEW_BLOCK_LOG"] = str(denied_log)
        # Fail closed if a reviewed path ever starts invoking host-management
        # commands. Read-only systemctl requests use a local inert fixture.
        for name in ("sysctl", "ufw", "iptables", "ip6tables", "nft", "service",
                     "certbot", "useradd", "usermod", "groupadd", "curl", "wget"):
            write(root / "usr/bin" / name,
                  '#!/bin/sh\nprintf "blocked\\n" >> "$SBCTL_REVIEW_BLOCK_LOG"\nexit 97\n', True)
        write(root / "usr/bin/systemctl", '''#!/bin/sh
case "$1" in
  show) printf 'ActiveState=inactive\nSubState=dead\nNRestarts=0\nMainPID=0\n'; exit 0 ;;
  is-active) exit 3 ;;
  *) printf 'blocked\n' >> "$SBCTL_REVIEW_BLOCK_LOG"; exit 97 ;;
esac
''', True)
        checker = root / "usr/local/bin/sing-box"
        write(checker, '''#!/bin/sh
case "$1" in
  version) printf 'sing-box version 1.14.2\n'; exit 0 ;;
  check) exit 0 ;;
  *) printf 'blocked\n' >> "$SBCTL_REVIEW_BLOCK_LOG"; exit 97 ;;
esac
''', True)
        if real_core:
            write(checker, '#!/bin/sh\ncase "$1" in\n'
                  '  version|check) exec ' + shlex.quote(str(real_core)) + ' "$@" ;;\n'
                  '  *) printf "blocked\\n" >> "$SBCTL_REVIEW_BLOCK_LOG"; exit 97 ;;\nesac\n', True)
        core = checker

        def invoke(*command, expected=0):
            result = subprocess.run([str(binary), "--root", str(root), *command],
                                    capture_output=True, env=environment, timeout=30)
            # Deliberately do not include captured output or argv in failures:
            # subscription URLs, terminal QR output and parse errors may be private.
            require(result.returncode == expected, "unexpected CLI exit status for " + command[0])
            return result

        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        write(root / "sys/class/net/ens3/statistics/rx_bytes", "100\n")
        write(root / "sys/class/net/ens3/statistics/tx_bytes", "200\n")
        write(root / "proc/sys/kernel/random/boot_id", "isolated-review-boot\n")
        init = ["config", "init", "--mode", "ip-fallback", "--subscription-host", "127.0.0.1",
                "--http-port", str(port), "--interface", "ens3", "--monthly-traffic-limit", "1073741824",
                "--reality-decoy-sni", "www.cloudflare.com", "--sing-box-bin", str(core)]
        for protocol in ("vless-reality", "vmess-websocket", "hysteria2", "tuic", "anytls"):
            init.extend(["--protocol", protocol])
        invoke(*init)
        invoke("config", "validate")
        config_path = root / "etc/sbctl/config.toml"
        original_config = config_path.read_text(encoding="utf-8")
        match = re.search(r'^subscription_credential = "([^"\r\n]+)"$', original_config, re.MULTILINE)
        require(match is not None, "fixture subscription credential is missing")
        credential = match.group(1)
        invoke("accounting-reset")
        write(root / "sys/class/net/ens3/statistics/rx_bytes", "130\n")
        write(root / "sys/class/net/ens3/statistics/tx_bytes", "260\n")
        status_result = invoke("status", "--json")
        status = json_document(status_result.stdout, "status stdout")
        require(status["traffic"]["total"] == 90, "fixture traffic total mismatch")
        require(credential.encode() not in status_result.stdout + status_result.stderr,
                "status disclosed the subscription credential")
        state_path = root / "var/lib/sbctl/state.json"
        prior_state = state_path.read_bytes()
        prior_state_modified = state_path.stat().st_mtime_ns
        prior_artifacts = {path.name: hashlib.sha256(path.read_bytes()).digest()
                           for path in (root / "var/lib/sbctl/artifacts").iterdir() if path.is_file()}
        if real_core:
            for artifact in ("sing-box-server.json", "subscription-sing-box.json", "subscription-sing-box-1.14.json"):
                path = root / "var/lib/sbctl/artifacts" / artifact
                checked = subprocess.run([str(core), "check", "-c", str(path)], capture_output=True,
                                         env=environment, timeout=30)
                require(checked.returncode == 0, "real sing-box rejected fixture artifact: " + artifact)

        links_output = invoke("sub").stdout.decode("utf-8")
        prefix = "/sub/" + credential + "/"
        suffixes = set()
        for url in re.findall(r"https?://[^\s<>]+", links_output):
            parsed = urlsplit(url)
            if parsed.hostname == "127.0.0.1" and parsed.path.startswith(prefix):
                suffixes.add(parsed.path[len(prefix):])
        formats = sorted(suffix for suffix in suffixes if suffix != "index" and not suffix.startswith("qr/"))
        require(len(formats) >= 13, "subscription matrix unexpectedly incomplete")
        require(all("qr/" + suffix in suffixes for suffix in formats), "matrix QR link is missing")

        log_path = root / "serve.stderr"
        with log_path.open("wb") as service_log:
            server = subprocess.Popen([str(binary), "--root", str(root), "serve", "--bind", f"127.0.0.1:{port}"],
                                      stdout=subprocess.DEVNULL, stderr=service_log, env=environment)
            try:
                deadline = time.monotonic() + 8
                ready = False
                while time.monotonic() < deadline and server.poll() is None:
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            ready = True
                        break
                    except OSError:
                        time.sleep(0.05)
                require(ready, "isolated subscription listener did not start")

                def request(path, expected=200, method="GET"):
                    # Separate loopback peers keep the enlarged matrix below
                    # each source's request budget without changing networking.
                    # Read the raw wire: HTTP clients discard HEAD bodies on
                    # their own and would hide a server sending forbidden bytes.
                    peer = "127.0.0.2" if method == "HEAD" else "127.0.0.1"
                    with socket.create_connection(("127.0.0.1", port), timeout=5,
                                                  source_address=(peer, 0)) as connection:
                        connection.sendall((method + " " + path + " HTTP/1.1\r\n"
                                            "Host: localhost\r\nConnection: close\r\n\r\n").encode("ascii"))
                        chunks = []
                        while chunk := connection.recv(65536):
                            chunks.append(chunk)
                    header_block, separator, body = b"".join(chunks).partition(b"\r\n\r\n")
                    require(bool(separator), "HTTP response has no header terminator")
                    lines = header_block.decode("iso-8859-1").split("\r\n")
                    require(lines[0].split(" ")[1] == str(expected),
                            "unexpected HTTP status in subscription smoke")
                    headers = {key.lower(): value.strip() for key, value in
                               (line.split(":", 1) for line in lines[1:])}
                    require(headers.get("cache-control") == "no-store", "HTTP response permitted caching")
                    return headers, body

                def check_head(path, get_headers, get_body, expected=200):
                    headers, body = request(path, expected, method="HEAD")
                    require(not body, "HEAD sent representation bytes")
                    require({key: value for key, value in headers.items() if key != "date"}
                            == {key: value for key, value in get_headers.items() if key != "date"},
                            "HEAD metadata differs from GET")
                    length = headers.get("content-length", "")
                    require(length.isdecimal() and int(length) == len(get_body),
                            "HEAD content length differs from GET representation")

                for suffix in formats:
                    headers, body = request(prefix + suffix)
                    check_head(prefix + suffix, headers, body)
                    require(bool(body), "subscription artifact is empty: " + suffix)
                    require("upload=30; download=60; total=1073741824; expire=" in headers.get("subscription-userinfo", ""),
                            "subscription traffic metadata mismatch: " + suffix)
                    if suffix.endswith(".json"):
                        require(headers.get("content-type", "").startswith("application/json"), "wrong JSON content type")
                        require(isinstance(json_document(body, suffix), dict), "invalid JSON artifact")
                    elif suffix.endswith(".yaml"):
                        require(headers.get("content-type", "").startswith("application/yaml"), "wrong YAML content type")
                        require(b"proxies:" in body, "YAML artifact has no proxy list")
                    elif suffix in ("uri.txt", "shadowrocket.txt"):
                        require(b"vless://" in base64.b64decode(body.strip(), validate=True), "invalid base64 URI artifact")
                    else:
                        require(b"vless://" in body, "URI artifact has no VLESS link")
                    qr_headers, qr_body = request(prefix + "qr/" + suffix)
                    check_head(prefix + "qr/" + suffix, qr_headers, qr_body)
                    require(qr_headers.get("content-type", "").startswith("image/svg+xml") and b"<svg" in qr_body,
                            "invalid SVG QR artifact: " + suffix)
                headers, body = request(prefix + "index")
                check_head(prefix + "index", headers, body)
                require(headers.get("content-type", "").startswith("text/html") and "sbctl 订阅中心".encode() in body,
                        "invalid subscription index")
                for invalid in ("bogus", "sing-box-1.09.json", "clash-1.17.yaml", "uri/extra", "qr/bogus",
                                "uri?x=1", "uri?", "index?x=1", "qr/uri?x=1"):
                    headers, body = request(prefix + invalid, 404)
                    check_head(prefix + invalid, headers, body, 404)
                    require(not body and "subscription-userinfo" not in headers,
                            "invalid route disclosed subscription information")
                for invalid in ("/sub/invalid-credential/uri", "/"):
                    headers, body = request(invalid, 404)
                    check_head(invalid, headers, body, 404)
                for method in ("POST", "PUT", "DELETE", "OPTIONS"):
                    headers, body = request(prefix + "uri", 405, method=method)
                    require(headers.get("allow") == "GET, HEAD" and not body,
                            "unsupported method did not advertise GET and HEAD")
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=5)
        require(credential.encode() not in log_path.read_bytes(), "subscription log disclosed the credential")
        require(state_path.read_bytes() == prior_state, "read-only HTTP requests changed accounting state")
        require(state_path.stat().st_mtime_ns == prior_state_modified,
                "read-only HTTP requests rewrote accounting state")
        require(prior_artifacts == {path.name: hashlib.sha256(path.read_bytes()).digest()
                                   for path in (root / "var/lib/sbctl/artifacts").iterdir() if path.is_file()},
                "read-only HTTP requests changed subscription artifacts")
        print(f"PASS isolated subscription matrix: {len(formats)} formats and matching QR routes", flush=True)
        print("PASS HEAD metadata/content length matches GET for all formats, QR routes and index; no wire body", flush=True)
        print("PASS index, invalid credentials, unknown routes, query rejection, method boundary and read-only state", flush=True)

        malformed = original_config.replace(match.group(0), 'subscription_credential = ["' + credential + '"]', 1)
        config_path.write_text(malformed, encoding="utf-8")
        try:
            for command in (("config", "show"), ("config", "validate"), ("status",), ("status", "--json"),
                            ("sub",), ("credential", "list"), ("node",), ("traffic",),
                            ("serve", "--bind", f"127.0.0.1:{port}")):
                result = invoke(*command, expected=2)
                require(credential.encode() not in result.stdout + result.stderr,
                        "configuration error disclosed the credential")
        finally:
            config_path.write_text(original_config, encoding="utf-8")
        require(not denied_log.exists(), "an unexpected host-management or download command was attempted")
        print("PASS malformed configuration diagnostics are redacted across nine CLI entry points", flush=True)
        print("PASS real sing-box version/check only" if real_core else "PASS fixture checker only (no real-core claim)", flush=True)
    print("PASS review smoke completed; temporary deployment and listener removed", flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Keep random credentials and generated configurations out of logs,
        # including on unexpected parsing/transport failures.
        message = str(error) if isinstance(error, SmokeFailure) else type(error).__name__
        print("FAIL review smoke: " + message, file=sys.stderr)
        sys.exit(1)
