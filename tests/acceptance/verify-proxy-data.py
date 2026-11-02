#!/usr/bin/env python3
"""Offline five-protocol data-plane check inside a PRIVATE Linux network namespace.

Usage (namespace creation and loopback setup are the caller's responsibility):
  unshare --net -- sh -ec 'ip link set lo up; python3 verify-proxy-data.py \
      /path/to/sbctl --sing-box /path/to/sing-box'

The script refuses the initial network namespace and any namespace containing a
non-loopback interface. The optional --port-hopping mode then creates a private
veth pair and a second namespace to exercise real ingress nftables redirects.
The default mode uses loopback only. No host TUN, system proxy, DNS, firewall,
sysctl or systemd state is changed. sbctl writes only a private
temporary --root. It generates five original nodes and their bare outbounds.
Server adaptation is limited to tightening inbound listen addresses to loopback
and pointing Reality's decoy dial at a local TLS 1.3 server, using sbctl's generated
certificate. SNI, credentials, keys, short IDs and all five client outbounds stay
unchanged. Each standalone client routes a manual SOCKS5 GET through exactly one
node to a local HTTP origin. This is an offline protocol check, not public-network
reachability, real-domain camouflage, certificate trust or systemd acceptance.
"""

import argparse
from contextlib import ExitStack
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import signal
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time


BODY = b"sbctl-private-netns-five-protocol-data-plane-ok\n"
PROTOCOLS = ("vless-reality", "vmess-websocket", "hysteria2", "tuic", "anytls")
TYPES = {"vless", "vmess", "hysteria2", "tuic", "anytls"}


class CheckFailure(Exception):
    pass


def require(condition, label):
    if not condition:
        raise CheckFailure(label)


def interrupt(_signal, _frame):
    raise CheckFailure("interrupted")


def private_namespace():
    require(sys.platform.startswith("linux"), "Linux network namespaces are required")
    try:
        current = os.readlink("/proc/self/ns/net")
        initial = os.readlink("/proc/1/ns/net")
    except OSError:
        raise CheckFailure("cannot inspect network namespaces; run as root inside unshare --net") from None
    require(current != initial,
            "refusing the initial network namespace; run via unshare --net")
    interfaces = {name for _, name in socket.if_nameindex()}
    require(interfaces == {"lo"}, "refusing a namespace with non-loopback interfaces")


def write_json(path, value):
    path.write_text(json.dumps(value), encoding="utf-8")
    path.chmod(0o600)


def stop_process(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=4)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=4)


def start_core(stack, core, config, root, label, prefix=()):
    checked = subprocess.run([str(core), "check", "-c", str(config)],
                             capture_output=True, cwd=root, timeout=20)
    require(checked.returncode == 0, label + " configuration check failed")
    log = stack.enter_context((root / (label + ".log")).open("wb"))
    process = subprocess.Popen([*prefix, str(core), "run", "-c", str(config)],
                               cwd=root, stdout=log, stderr=log, start_new_session=True)
    stack.callback(stop_process, process)
    return process


def wait_tcp(port, process, label, host="127.0.0.1"):
    deadline = time.monotonic() + 6
    while time.monotonic() < deadline:
        require(process.poll() is None, label + " process exited during startup")
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.05)
    raise CheckFailure(label + " loopback listener did not start")


def free_port():
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        return reservation.getsockname()[1]


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(BODY)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(BODY)

    def log_message(self, *_arguments):
        pass


class QuietServer(ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, _request, _client_address):
        pass


class DecoyServer(QuietServer):
    def get_request(self):
        connection, address = self.socket.accept()
        connection.settimeout(3)
        try:
            return self.tls_context.wrap_socket(connection, server_side=True), address
        except BaseException:
            connection.close()
            raise


def serve_http(stack, tls_context=None):
    server_class = DecoyServer if tls_context else QuietServer
    server = server_class(("127.0.0.1", 0), Handler)
    if tls_context:
        server.tls_context = tls_context
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
    thread.start()

    def close():
        server.shutdown()
        server.server_close()
        thread.join(timeout=4)

    stack.callback(close)
    return server.server_address[1]


def receive_exact(connection, length):
    data = bytearray()
    while len(data) < length:
        chunk = connection.recv(length - len(data))
        require(bool(chunk), "SOCKS response ended early")
        data.extend(chunk)
    return bytes(data)


def proxy_get(socks_port, origin_port, socks_host="127.0.0.1"):
    with socket.create_connection((socks_host, socks_port), timeout=10) as connection:
        connection.sendall(b"\x05\x01\x00")
        require(receive_exact(connection, 2) == b"\x05\x00", "SOCKS authentication negotiation failed")
        connection.sendall(b"\x05\x01\x00\x01" + socket.inet_aton("127.0.0.1") + struct.pack("!H", origin_port))
        version, reply, _reserved, address_type = receive_exact(connection, 4)
        require(version == 5 and reply == 0, "SOCKS proxy could not establish the protocol connection")
        if address_type == 1:
            receive_exact(connection, 6)
        elif address_type == 4:
            receive_exact(connection, 18)
        elif address_type == 3:
            receive_exact(connection, receive_exact(connection, 1)[0] + 2)
        else:
            raise CheckFailure("SOCKS response has an invalid address type")
        connection.sendall(b"GET /offline-data-plane HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        response = bytearray()
        while True:
            chunk = connection.recv(4096)
            if not chunk:
                break
            response.extend(chunk)
            require(len(response) < 1024 * 1024, "origin response exceeded the fixed test boundary")
        headers, separator, body = response.partition(b"\r\n\r\n")
        require(bool(separator) and headers.split(b"\r\n", 1)[0].endswith(b" 200 OK"),
                "origin did not return HTTP 200 through the protocol")
        require(body == BODY, "origin body mismatch through the protocol")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--sing-box", type=Path, required=True)
    parser.add_argument("--port-hopping", action="store_true", help="verify Hy2 redirects and a live hop in this private namespace")
    args = parser.parse_args()
    private_namespace()
    signal.signal(signal.SIGTERM, interrupt)
    sbctl = args.binary.resolve()
    core = args.sing_box.resolve()
    require(sbctl.is_file() and core.is_file(), "candidate or real core binary is missing")
    phase = "initialization"
    try:
        with ExitStack() as stack:
            temporary = stack.enter_context(tempfile.TemporaryDirectory(prefix="sbctl-proxy-data-"))
            root = Path(temporary)
            root.chmod(0o700)
            client_prefix = []
            socks_host = "127.0.0.1"
            if args.port_hopping:
                client_ns = subprocess.Popen(["unshare", "--net", "sleep", "120"])
                stack.callback(stop_process, client_ns)
                for _ in range(100):
                    if os.readlink(f"/proc/{client_ns.pid}/ns/net") != os.readlink("/proc/self/ns/net"):
                        break
                    time.sleep(0.01)
                client_prefix = ["nsenter", "-t", str(client_ns.pid), "-n"]
                def net(command):
                    subprocess.run(command, check=True, capture_output=True, timeout=10)
                net(["ip", "link", "add", "hy2-server", "type", "veth", "peer", "name", "hy2-client"])
                net(["ip", "link", "set", "hy2-client", "netns", str(client_ns.pid)])
                net(["ip", "address", "add", "192.0.2.1/30", "dev", "hy2-server"])
                net(["ip", "link", "set", "hy2-server", "up"])
                net(client_prefix + ["ip", "link", "set", "lo", "up"])
                net(client_prefix + ["ip", "address", "add", "192.0.2.2/30", "dev", "hy2-client"])
                net(client_prefix + ["ip", "link", "set", "hy2-client", "up"])
                socks_host = "192.0.2.2"

            init = [str(sbctl), "--root", str(root), "config", "init", "--mode", "ip-fallback",
                    "--subscription-host", "192.0.2.1" if args.port_hopping else "127.0.0.1",
                    "--http-port", str(free_port()), "--interface", "hy2-server" if args.port_hopping else "lo",
                    "--reality-decoy-sni", "www.cloudflare.com", "--protocol-sni", "www.cloudflare.com",
                    "--sing-box-bin", str(core)]
            for protocol in PROTOCOLS:
                init.extend(["--protocol", protocol])
            initialized = subprocess.run(init, capture_output=True, cwd=root, timeout=30)
            require(initialized.returncode == 0, "isolated five-protocol configuration generation failed")
            artifacts = root / "var/lib/sbctl/artifacts"
            if args.port_hopping:
                existing_server = json.loads((artifacts / "sing-box-server.json").read_bytes())
                occupied = {n["listen_port"] for n in existing_server["inbounds"]}
                start = next(p for p in range(40000, 60000, 100) if not occupied.intersection(range(p, p + 11)))
                config_path = root / "etc/sbctl/config.toml"
                config_path.write_text(f"hysteria2_port_hopping = {{ start = {start}, end = {start + 10}, interval_seconds = 5 }}\n" + config_path.read_text())
                regenerated = subprocess.run([str(sbctl), "--root", str(root), "regenerate", "--sing-box-bin", str(core)], capture_output=True, timeout=30)
                require(regenerated.returncode == 0, "hopping artifacts failed regeneration")
                # Alternate roots cannot invoke a host command: explicitly provide
                # this namespace's nft binary inside the disposable root.
                import shutil
                nft = shutil.which("nft")
                require(nft is not None, "install nftables before the hopping check")
                (root / "usr/bin").mkdir(parents=True, exist_ok=True)
                (root / "usr/bin/nft").symlink_to(nft)
                hook = [str(sbctl), "--root", str(root), "sing-box", "port-hopping-firewall"]
                applied = subprocess.run(hook, capture_output=True, timeout=10)
                require(applied.returncode == 0, "hopping firewall hook failed")
                stack.callback(lambda: subprocess.run(hook + ["--cleanup"], capture_output=True, timeout=10))
                table = subprocess.run([nft, "list", "table", "inet", "sbctl_hy2"], capture_output=True)
                require(table.returncode == 0, "hopping table missing")

            server = json.loads((artifacts / "sing-box-server.json").read_bytes())
            original_server = copy.deepcopy(server)
            bare = json.loads((artifacts / "subscription-sing-box.json").read_bytes())
            original_outbounds = copy.deepcopy(bare["outbounds"])
            require({node["type"] for node in server["inbounds"]} == TYPES, "server protocol matrix is incomplete")
            require({node["type"] for node in bare["outbounds"]} == TYPES, "bare outbound matrix is incomplete")
            require(all(node["server"] == ("192.0.2.1" if args.port_hopping else "127.0.0.1") for node in bare["outbounds"]),
                    "a generated client node escapes the private test network")

            certificate_tls = next(node["tls"] for node in server["inbounds"] if node["type"] == "vmess")
            certificate = Path(certificate_tls["certificate_path"]).resolve()
            private_key = Path(certificate_tls["key_path"]).resolve()
            require(certificate.is_relative_to(root) and private_key.is_relative_to(root),
                    "generated certificate path escapes the temporary root")
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = ssl.TLSVersion.TLSv1_3
            context.maximum_version = ssl.TLSVersion.TLSv1_3
            context.set_alpn_protocols(["h2", "http/1.1"])
            context.load_cert_chain(certificate, private_key)
            origin_port = serve_http(stack)
            decoy_port = serve_http(stack, context)
            for node in server["inbounds"]:
                node["listen"] = "0.0.0.0" if args.port_hopping else "127.0.0.1"
                if node["type"] == "vless":
                    node["tls"]["reality"]["handshake"] = {"server": "127.0.0.1", "server_port": decoy_port}
            # Assert the complete adaptation boundary before starting any core.
            expected = copy.deepcopy(original_server)
            for node in expected["inbounds"]:
                node["listen"] = "0.0.0.0" if args.port_hopping else "127.0.0.1"
                if node["type"] == "vless":
                    node["tls"]["reality"]["handshake"] = {"server": "127.0.0.1", "server_port": decoy_port}
            require(server == expected, "server adaptation changed an unexpected protocol field")
            server_path = root / "offline-server.json"
            write_json(server_path, server)
            phase = "server startup"
            server_process = start_core(stack, core, server_path, root, "server")
            vmess_port = next(node["listen_port"] for node in server["inbounds"] if node["type"] == "vmess")
            wait_tcp(vmess_port, server_process, "server")

            for node in bare["outbounds"]:
                phase = node["type"]
                with ExitStack() as client_stack:
                    socks_port = free_port()
                    client = {
                        "log": {"level": "error"},
                        "inbounds": [{"type": "socks", "tag": "offline-socks", "listen": "0.0.0.0" if args.port_hopping else "127.0.0.1", "listen_port": socks_port}],
                        "outbounds": [copy.deepcopy(node)],
                        "route": {"final": node["tag"]},
                    }
                    require(client["outbounds"][0] == node, "client outbound was modified")
                    client_path = root / (node["type"] + "-client.json")
                    write_json(client_path, client)
                    client_process = start_core(client_stack, core, client_path, root, node["type"], client_prefix)
                    wait_tcp(socks_port, client_process, node["type"], socks_host)
                    proxy_get(socks_port, origin_port, socks_host)
                    if args.port_hopping and node["type"] == "hysteria2":
                        time.sleep(6)
                        proxy_get(socks_port, origin_port, socks_host)
                        counters = subprocess.run([nft, "list", "table", "inet", "sbctl_hy2"], capture_output=True, text=True)
                        require(counters.returncode == 0 and "packets 0 bytes 0" not in counters.stdout,
                                "Hy2 did not traverse the hopping redirect")
                        print("PASS Hy2 transfer before and after a live port hop", flush=True)

                    require(server_process.poll() is None and client_process.poll() is None,
                            "a protocol core exited during the data-plane request")
                    print("PASS isolated protocol HTTP body: " + node["type"], flush=True)
            require(bare["outbounds"] == original_outbounds, "bare outbounds changed during testing")
            if args.port_hopping:
                cleaned = subprocess.run(hook + ["--cleanup"], capture_output=True, timeout=10)
                require(cleaned.returncode == 0, "hopping firewall cleanup failed")
                require(subprocess.run([nft, "list", "table", "inet", "sbctl_hy2"], capture_output=True).returncode != 0,
                        "hopping table survived cleanup")
                print("PASS Hy2 nftables cleanup", flush=True)

        print("PASS five offline data planes; all processes, listeners and temporary credentials removed", flush=True)
    except Exception as error:
        # Never print captured core logs, credential-bearing JSON or exception
        # payloads from third-party parsers/transports.
        reason = str(error) if isinstance(error, CheckFailure) else type(error).__name__
        raise CheckFailure(phase + ": " + reason) from None


if __name__ == "__main__":
    try:
        main()
    except (Exception, KeyboardInterrupt) as error:
        reason = str(error) if isinstance(error, CheckFailure) else type(error).__name__
        print("FAIL isolated protocol check: " + reason, file=sys.stderr)
        sys.exit(1)
