#!/usr/bin/env python3
"""Portable, isolated 1.14.2 smoke; never enables system proxy or TUN."""
import argparse
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument("--cli", required=True, type=Path)
parser.add_argument("--core", required=True, type=Path)
args = parser.parse_args()
env = {key: value for key, value in os.environ.items() if key not in ("BOX_API_URL", "BOX_API_SECRET")}
body = {}
slow = threading.Event()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.endswith("sing-box-full.json"):
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        elif self.path == "/slow":
            slow.wait(15)
            self.send_response(204)
            self.end_headers()
        else:
            self.send_response(204)
            self.end_headers()

    def log_message(self, *_):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
work = tempfile.TemporaryDirectory(prefix="sbcli-real-")
data = Path(work.name) / "data"
(data / "core").mkdir(parents=True)
shutil.copy2(args.core.resolve(), data / "core" / ("sing-box.exe" if os.name == "nt" else "sing-box"))
cli = args.cli.resolve()


def call(*command, expected=0):
    result = subprocess.run([str(cli), "--data-dir", str(data), "--json", *command],
                            capture_output=True, text=True, encoding="utf-8", env=env, timeout=80)
    if result.returncode != expected:
        raise AssertionError(f"{command}: exit={result.returncode}: {result.stderr}")
    print("PASS", " ".join(command), "exit", expected, flush=True)
    return json.loads(result.stdout) if result.stdout.strip() else None


try:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        mixed_port = listener.getsockname()[1]
    call("inbound", "set", "mixed", "--port", str(mixed_port))
    body.update({"log": {"level": "debug"}, "outbounds": [
        {"type": "direct", "tag": "direct"}, {"type": "direct", "tag": "direct2"},
        {"type": "urltest", "tag": "auto", "outbounds": ["direct", "direct2"],
         "url": f"http://127.0.0.1:{server.server_port}/", "interval": "10m"},
        {"type": "selector", "tag": "proxy", "outbounds": ["auto", "direct"], "default": "direct"}],
        "route": {"final": "proxy"}, "dns": {"servers": [{"type": "local", "tag": "local"}]}})
    source = Path(work.name) / "source.json"
    source.write_text(json.dumps(body), encoding="utf-8")
    call("profile", "import", str(source), "--name", "fixture")
    call("config", "check")
    call("start")
    status = call("status")
    assert status["core_running"] and status["core_runtime_version"] == "1.14.2"
    assert not status["settings"]["ingress"]["system_proxy"]
    call("run", expected=5)
    assert call("traffic")["available"]
    assert len(call("outbound", "list")) == 4
    call("group", "select", "proxy", "direct")
    call("group", "urltest", "auto")
    # Unknown mode names must not turn an ignored official setter into success.
    call("mode", "set", "global", expected=3)
    call("config", "mode-enable", "--proxy", "proxy", "--direct", "direct",
         "--proxy-dns", "local", "--direct-dns", "local")
    assert call("status")["pending_configuration"]
    call("config", "apply")
    call("mode", "set", "global")
    assert call("mode", "show")["mode"] == "Global"
    assert "Global" in call("mode", "list")["modes"]

    # Bad staged input is refused before disrupting the healthy instance.
    cache = data / "cache" / "profiles" / (hashlib.sha256(b"fixture").hexdigest() + ".json")
    pending = cache.with_suffix(".pending.json")
    pending.write_text('{"outbounds":[{"type":"not-a-protocol"}]}', encoding="utf-8")
    result = subprocess.run([str(cli), "--data-dir", str(data), "--json", "config", "apply"],
                            env=env, capture_output=True, timeout=80)
    assert result.returncode != 0 and not result.stdout
    assert call("status")["core_running"]
    pending.unlink()

    # A valid configuration that cannot bind at startup must recover the prior
    # listener/settings and acknowledge failure, rather than leave a dead core.
    old_port = call("status")["settings"]["mixed_port"]
    with socket.socket() as occupied:
        occupied.bind(("127.0.0.1", 0))
        occupied.listen()
        result = subprocess.run([str(cli), "--data-dir", str(data), "--json", "inbound", "set",
                                 "mixed", "--port", str(occupied.getsockname()[1])],
                                env=env, capture_output=True, timeout=80)
        assert result.returncode != 0
        recovered = call("status")
        assert recovered["core_running"] and recovered["settings"]["mixed_port"] == old_port
    print("PASS rejected candidate and recovered failed listener", flush=True)

    # Produce a real open connection through Mixed and close it over gRPC.
    def open_slow():
        connection = http.client.HTTPConnection("127.0.0.1", old_port, timeout=20)
        try:
            connection.request("GET", f"http://127.0.0.1:{server.server_port}/slow")
            connection.getresponse()
        except (OSError, http.client.HTTPException):
            pass
        finally:
            connection.close()
    request_thread = threading.Thread(target=open_slow, daemon=True)
    request_thread.start()
    deadline = time.monotonic() + 8
    while True:
        rows = call("connection", "list")["connections"]
        if rows:
            break
        assert time.monotonic() < deadline, "no real connection observed"
        time.sleep(.25)
    call("connection", "show", rows[0]["id"])
    call("connection", "close", rows[0]["id"])
    call("connection", "close", "--all")
    call("profile", "import", f"http://127.0.0.1:{server.server_port}/sub/fixture/sing-box-full.json", "--name", "remote")
    body["route"]["final"] = "direct"
    call("profile", "refresh")
    assert call("status")["pending_configuration"]
    assert call("config", "show", "--raw")["route"]["final"] == "proxy"
    call("config", "apply")
    assert call("config", "show", "--raw")["route"]["final"] == "direct"
    print("PASS refresh staged until explicit apply", flush=True)
    call("restart")
    call("reload")
    call("stop")
    assert call("daemon", "status")["daemon_running"]
    print("REAL CORE SMOKE PASSED", flush=True)
finally:
    slow.set()
    call("daemon", "stop")
    server.shutdown()
    work.cleanup()
