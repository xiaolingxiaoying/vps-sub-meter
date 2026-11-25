#!/usr/bin/env python3
"""Universal idempotent state & configuration migrator for vps-sub-meter.

Supports upgrading from ALL historical deployments:
  1. Gen 1 (`sub-srv` + `sing-box-yg` / Caddy on `main` & `v2`):
     /etc/sub-srv/config.conf + /var/lib/subsrv/tx_state.json
  2. Gen 2 (`sbctl` v0.0.1..v0.0.28 on `dev`, plus optional hybrid PQC/Argo):
     /etc/sbctl/config.toml + /var/lib/sbctl/state.json
  3. Gen 3 (`vps-sub-meter` v2.0.0+):
     /etc/vps-sub-meter/config.toml + /etc/vps-sub-meter/secrets.json

Security: never prints tokens, UUIDs, passwords, keys, or secret paths.
Compatible with Python 3.10+ standard library (does not require `tomllib`).
"""

from __future__ import annotations

import argparse
import ast
import calendar
from datetime import datetime, timezone
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from zoneinfo import ZoneInfo

try:
    import grp
    import pwd
except ImportError:  # Windows unit tests
    grp = None  # type: ignore[assignment]
    pwd = None  # type: ignore[assignment]


def toml_escape(value: str) -> str:
    return (
        value.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\r", "")
    )


def strip_toml_comment(line: str) -> str:
    out: list[str] = []
    in_sq = False
    in_dq = False
    escaped = False
    for ch in line:
        if escaped:
            out.append(ch)
            escaped = False
            continue
        if in_dq and ch == "\\":
            out.append(ch)
            escaped = True
            continue
        if ch == '"' and not in_sq:
            in_dq = not in_dq
        elif ch == "'" and not in_dq:
            in_sq = not in_sq
        elif ch == "#" and not in_sq and not in_dq:
            break
        out.append(ch)
    return "".join(out).strip()


def parse_toml_value(raw: str):
    val = raw.strip()
    if val == "true":
        return True
    if val == "false":
        return False
    if val.startswith('"') or val.startswith("'") or val.startswith("["):
        return ast.literal_eval(val)
    if re.fullmatch(r"-?\d+", val):
        return int(val)
    if re.fullmatch(r"-?\d+\.\d+", val):
        return float(val)
    return val


def parse_simple_toml(text: str) -> dict:
    """Minimal TOML subset parser for /etc/sbctl/config.toml (Python 3.10 safe)."""
    root: dict = {}
    current = root
    lines = text.splitlines()
    idx = 0
    while idx < len(lines):
        line = strip_toml_comment(lines[idx])
        idx += 1
        if not line:
            continue
        # Handle multi-line array values
        while (
            "=" in line
            and line.split("=", 1)[1].lstrip().startswith("[")
            and line.count("[") > line.count("]")
            and idx < len(lines)
        ):
            next_line = strip_toml_comment(lines[idx])
            idx += 1
            line = f"{line} {next_line}"

        if line.startswith("[[") and line.endswith("]]"):
            sec_path = [p.strip() for p in line[2:-2].split(".") if p.strip()]
            target = root
            for part in sec_path[:-1]:
                target = target.setdefault(part, {})
            arr = target.setdefault(sec_path[-1], [])
            item: dict = {}
            arr.append(item)
            current = item
            continue
        if line.startswith("[") and line.endswith("]"):
            sec_path = [p.strip() for p in line[1:-1].split(".") if p.strip()]
            target = root
            for part in sec_path:
                target = target.setdefault(part, {})
            current = target
            continue
        if "=" in line:
            k, v = line.split("=", 1)
            current[k.strip()] = parse_toml_value(v)
    return root


def parse_subsrv_conf(text: str) -> dict[str, str]:
    """Parse key=value lines written by legacy Gen 1 setup.sh / Python repr."""
    data: dict[str, str] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        key = k.strip()
        val_raw = v.strip()
        if not val_raw:
            data[key] = ""
            continue
        if (val_raw.startswith("'") and val_raw.endswith("'")) or (
            val_raw.startswith('"') and val_raw.endswith('"')
        ):
            try:
                data[key] = str(ast.literal_eval(val_raw))
                continue
            except Exception:
                pass
        try:
            parts = shlex.split(val_raw)
            data[key] = parts[0] if parts else ""
        except Exception:
            data[key] = val_raw.strip("'\"")
    return data


def is_ip_literal(host: str) -> bool:
    try:
        ipaddress.ip_address(host.strip())
        return True
    except ValueError:
        return False


def valid_hostname(host: str) -> bool:
    h = host.strip()
    if not h or len(h) > 253 or is_ip_literal(h) or "." not in h:
        return False
    return bool(re.fullmatch(r"[A-Za-z0-9.-]+", h))


def detect_default_interface(root: Path, preferred: str = "") -> str:
    cand = preferred.strip()
    if cand and (root / "sys/class/net" / cand).exists():
        return cand
    net_dir = root / "sys/class/net"
    if net_dir.is_dir():
        for entry in sorted(net_dir.iterdir()):
            if entry.name != "lo":
                return entry.name
    return cand or "eth0"


def detect_public_ip(root: Path, candidate: str = "", domain: str = "") -> str:
    c = candidate.strip()
    if c and is_ip_literal(c):
        try:
            if not ipaddress.ip_address(c).is_private:
                return c
        except ValueError:
            pass
    if root == Path("/"):
        try:
            out = subprocess.check_output(
                ["ip", "-4", "-o", "route", "get", "1.1.1.1"],
                text=True,
                stderr=subprocess.DEVNULL,
                timeout=3,
            )
            m = re.search(r"\bsrc\s+(\d+\.\d+\.\d+\.\d+)\b", out)
            if m and not ipaddress.ip_address(m.group(1)).is_private:
                return m.group(1)
        except Exception:
            pass
        for url in ("https://api.ipify.org", "https://checkip.amazonaws.com"):
            try:
                ip_txt = urllib.request.urlopen(url, timeout=3).read().decode("utf-8").strip()
                if is_ip_literal(ip_txt) and not ipaddress.ip_address(ip_txt).is_private:
                    return ip_txt
            except Exception:
                pass
        if domain:
            try:
                resolved = socket.gethostbyname(domain.strip())
                if is_ip_literal(resolved) and not ipaddress.ip_address(resolved).is_private:
                    return resolved
            except Exception:
                pass
    return c if (c and is_ip_literal(c)) else "203.0.113.10"


def read_sysfs_counter(root: Path, iface: str, kind: str) -> int:
    p = root / "sys/class/net" / iface / "statistics" / f"{kind}_bytes"
    try:
        return int(p.read_text(encoding="utf-8").strip())
    except Exception:
        return 0


def read_boot_id(root: Path) -> str:
    p = root / "proc/sys/kernel/random/boot_id"
    try:
        val = p.read_text(encoding="utf-8").strip()
        if val:
            return val
    except Exception:
        pass
    return "unknown-boot-id"


def compute_next_reset_ts(
    reset_day: int, reset_hour: int, reset_minute: int, tz_name: str
) -> int:
    try:
        tz = ZoneInfo(tz_name)
    except Exception:
        tz = timezone.utc
    now_local = datetime.now(tz)
    year, month = now_local.year, now_local.month
    last_day = calendar.monthrange(year, month)[1]
    day = min(max(1, reset_day), last_day)
    cand = datetime(year, month, day, reset_hour, reset_minute, 0, tzinfo=tz)
    if cand <= now_local:
        if month == 12:
            year += 1
            month = 1
        else:
            month += 1
        last_day = calendar.monthrange(year, month)[1]
        day = min(max(1, reset_day), last_day)
        cand = datetime(year, month, day, reset_hour, reset_minute, 0, tzinfo=tz)
    return int(cand.timestamp())


def is_root() -> bool:
    return hasattr(os, "geteuid") and os.geteuid() == 0


def write_atomic(
    path: Path, data: bytes, mode: int, uid: int | None = None, gid: int | None = None
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f".{path.name}.tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, mode)
    try:
        os.write(fd, data)
        os.fsync(fd)
    finally:
        os.close(fd)
    if uid is not None and gid is not None and is_root() and hasattr(os, "chown"):
        os.chown(tmp, uid, gid)
    os.chmod(tmp, mode)
    os.replace(tmp, path)


def live_ids(root: Path) -> tuple[tuple[int, int] | None, int | None]:
    if root != Path("/") or not is_root() or grp is None or pwd is None:
        return None, None
    try:
        uid = pwd.getpwnam("vps-sub-meter").pw_uid
        gid = grp.getgrnam("vps-sub-meter").gr_gid
        return (uid, gid), gid
    except KeyError:
        return None, None


def render_config_toml(
    *,
    profile_name: str,
    domain: str,
    public_ip: str,
    interface: str,
    ipv4_only: bool,
    extra_https_port: int | None,
    http_fallback_port: int,
    monthly_gb: float,
    reset_day: int,
    reset_time: str,
    reset_tz: str,
    display_tz: str,
    decoy_sni: str,
    node_names: list[str],
    reality_loopback: int,
    xhttp_port: int,
    anytls_port: int,
    hy2_port: int,
    hy2_range: str | None,
    hy2_up: int,
    hy2_down: int,
    tuic_port: int,
    argo_cfg: dict | None,
) -> str:
    extra_line = (
        f"extra_https_port = {extra_https_port}\n"
        if extra_https_port is not None
        else ""
    )
    hop_line = f'hop_range = "{toml_escape(hy2_range)}"\n' if hy2_range else ""
    base = (
        f'profile_name = "{toml_escape(profile_name)}"\n'
        f'domain = "{toml_escape(domain)}"\n'
        f'public_ip = "{toml_escape(public_ip)}"\n'
        f'interface = "{toml_escape(interface)}"\n'
        f"ipv4_only = {'true' if ipv4_only else 'false'}\n"
        f'latency_probe_url = "https://www.gstatic.com/generate_204"\n\n'
        f"[server]\n"
        f"acme_port = 80\n"
        f"https_port = 443\n"
        f"loopback_https_port = 8444\n"
        f"{extra_line}"
        f"http_fallback_port = {http_fallback_port}\n\n"
        f"[traffic]\n"
        f"monthly_gb = {monthly_gb:.2f}\n"
        f"reset_day = {reset_day}\n"
        f'reset_time = "{toml_escape(reset_time)}"\n'
        f'reset_tz = "{toml_escape(reset_tz)}"\n'
        f'display_tz = "{toml_escape(display_tz)}"\n\n'
        f"[nodes.vless_reality_pqc]\n"
        f"enabled = true\n"
        f'name = "{toml_escape(node_names[0])}"\n'
        f"port = 443\n"
        f"loopback_port = {reality_loopback}\n"
        f'decoy_sni = "{toml_escape(decoy_sni)}"\n\n'
        f"[nodes.vless_xhttp_pqc]\n"
        f"enabled = true\n"
        f'name = "{toml_escape(node_names[1])}"\n'
        f"port = {xhttp_port}\n\n"
        f"[nodes.anytls_tcp]\n"
        f"enabled = true\n"
        f'name = "{toml_escape(node_names[2])}"\n'
        f"port = {anytls_port}\n\n"
        f"[nodes.hysteria2_hop]\n"
        f"enabled = true\n"
        f'name = "{toml_escape(node_names[3])}"\n'
        f"port = {hy2_port}\n"
        f"{hop_line}"
        f"hop_interval_sec = 30\n"
        f"obfs_salamander = true\n"
        f"up_mbps = 50\n"
        f"down_mbps = 200\n"
        f"server_up_mbps = {hy2_up}\n"
        f"server_down_mbps = {hy2_down}\n\n"
        f"[nodes.tuic_bbr]\n"
        f"enabled = true\n"
        f'name = "{toml_escape(node_names[4])}"\n'
        f"port = {tuic_port}\n"
    )
    if argo_cfg:
        base += (
            "\n[argo]\n"
            "enabled = true\n"
            'profile_name = "Argo CDN Backup"\n'
            f'tunnel_domain = "{toml_escape(argo_cfg["tunnel_domain"])}"\n'
            f'sub_domain = "{toml_escape(argo_cfg["sub_domain"])}"\n'
            f'loopback_sub_port = {argo_cfg["loopback_sub_port"]}\n'
            'preferred_ips_file = "/etc/vps-sub-meter/argo-ips.txt"\n'
        )
        for ws in argo_cfg["ws_inbounds"]:
            base += (
                "\n[[argo.ws_inbounds]]\n"
                f'id = "{toml_escape(ws["id"])}"\n'
                f'name = "{toml_escape(ws["name"])}"\n'
                f'protocol = "{toml_escape(ws["protocol"])}"\n'
                f'engine = "{toml_escape(ws["engine"])}"\n'
                f'listen = "{toml_escape(ws["listen"])}"\n'
                f'port = {ws["port"]}\n'
                f"tls = {'true' if ws['tls'] else 'false'}\n"
            )
    else:
        base += (
            "\n[argo]\n"
            "enabled = false\n"
            'profile_name = "Argo CDN Backup"\n'
            'tunnel_domain = ""\n'
            'sub_domain = ""\n'
            "loopback_sub_port = 18081\n"
            'preferred_ips_file = "/etc/vps-sub-meter/argo-ips.txt"\n'
        )
    return base


def detect_generation(root: Path) -> str | None:
    if (root / "etc/vps-sub-meter/config.toml").is_file():
        return "v2"
    if (root / "etc/sbctl/config.toml").is_file():
        return "sbctl"
    if (root / "etc/sub-srv/config.conf").is_file():
        return "sub-srv-v1"
    return None


def migrate_sbctl(root: Path, check_only: bool) -> dict:
    raw = parse_simple_toml((root / "etc/sbctl/config.toml").read_text(encoding="utf-8"))
    sub_host = str(raw.get("subscription_host", "")).strip()
    proxy_host = str(raw.get("proxy_host", "")).strip() or sub_host

    if not sub_host or is_ip_literal(sub_host):
        domain = ""
        public_ip = sub_host if is_ip_literal(sub_host) else detect_public_ip(root, proxy_host)
    else:
        domain = sub_host
        public_ip = detect_public_ip(root, proxy_host, domain)

    interface = detect_default_interface(root, str(raw.get("interface", "eth0")))
    ipv4_only = bool(raw.get("ipv4_only", False))
    http_fallback = int(raw.get("http_port", 2080) or 2080)
    if http_fallback < 1024 or http_fallback == 8444:
        http_fallback = 2080

    decoy = str(raw.get("reality_decoy_sni", "")).strip()
    if not valid_hostname(decoy):
        decoy = "aws.amazon.com"

    # Tokens: keep active credentials ONLY (strictly exclude revoked entries).
    now_ts = int(time.time())
    tokens: list[dict] = []
    seen_tok: set[str] = set()

    def push_token(name: str, tok: str) -> None:
        t = tok.strip()
        if len(t) >= 16 and t not in seen_tok:
            seen_tok.add(t)
            tokens.append({"name": name, "token": t, "created_at": now_ts})

    xray_tok_file = root / "usr/local/etc/xray-sub/token.txt"
    if xray_tok_file.is_file():
        push_token("primary-8443", xray_tok_file.read_text(encoding="utf-8").strip())

    primary_cred = str(raw.get("subscription_credential", "")).strip()
    if primary_cred:
        push_token("sbctl-primary", primary_cred)

    for idx, entry in enumerate(raw.get("subscription_credentials", [])):
        if not isinstance(entry, dict):
            continue
        if entry.get("revoked_at") is not None:
            continue
        push_token(
            str(entry.get("name") or f"device-{idx + 1}"),
            str(entry.get("credential", "")),
        )

    # Defaults for nodes & secrets
    node_names = [
        "vless-reality-pqc",
        "vless-xhttp-pqc",
        "anytls-tcp",
        "hysteria2-hop",
        "tuic-bbr",
    ]
    reality_loopback = 1443
    xhttp_port = 2053
    anytls_port = 2083
    hy2_port = 34443
    hy2_range: str | None = "30000-35000"
    hy2_up = 100
    hy2_down = 200
    tuic_port = 2096
    extra_https_port: int | None = None

    sub_listen_port = raw.get("subscription_listen_port")
    if isinstance(sub_listen_port, int) and 1024 <= sub_listen_port <= 65535 and sub_listen_port not in (8444, http_fallback):
        extra_https_port = sub_listen_port
    if xray_tok_file.is_file() and extra_https_port is None:
        extra_https_port = 8443

    vr_raw = raw.get("vless_reality", {}) if isinstance(raw.get("vless_reality"), dict) else {}
    vless_reality_sec = {
        "uuid": str(vr_raw.get("uuid", "")),
        "x25519_private_key": str(vr_raw.get("private_key", "")),
        "x25519_public_key": str(vr_raw.get("public_key", "")),
        "short_id": str(vr_raw.get("short_id", "")),
        "mldsa65_seed": "",
        "mldsa65_verify": "",
        "vless_decryption": "",
        "vless_encryption": "",
    }
    vless_xhttp_sec = {
        "uuid": "",
        "path": "",
        "vless_decryption": "",
        "vless_encryption": "",
    }
    anytls_sec = {"password": str((raw.get("anytls") or {}).get("password", ""))}
    hy2_sec = {
        "password": str((raw.get("hysteria2") or {}).get("password", "")),
        "obfs_password": "",
    }
    tuic_raw = raw.get("tuic", {}) if isinstance(raw.get("tuic"), dict) else {}
    tuic_sec = {
        "uuid": str(tuic_raw.get("uuid", "")),
        "password": str(tuic_raw.get("password", "")),
    }

    # Check if hybrid xray + sing-box-pqc setup is present
    xray_p = root / "usr/local/etc/xray/config.json"
    sb_pqc_p = root / "usr/local/etc/sing-box-pqc/config.json"
    uri_p = root / "usr/local/etc/xray-sub/uri.txt"
    if xray_p.is_file() and sb_pqc_p.is_file():
        xray_cfg = json.loads(xray_p.read_text(encoding="utf-8"))
        sb_pqc = json.loads(sb_pqc_p.read_text(encoding="utf-8"))
        uris = [l.strip() for l in uri_p.read_text(encoding="utf-8").splitlines() if l.strip()] if uri_p.is_file() else []

        def uri_query(substr: str) -> dict[str, str]:
            for uline in uris:
                if uline.startswith("vless://") and substr in uline:
                    parsed = urllib.parse.urlsplit(uline)
                    return {k: v[0] for k, v in urllib.parse.parse_qs(parsed.query, keep_blank_values=True).items()}
            return {}

        ib_r = next((i for i in xray_cfg.get("inbounds", []) if i.get("tag") == "vless-reality-pqc"), None)
        if ib_r:
            qr = uri_query("vless-pqc") or uri_query("security=reality")
            rset = ib_r["streamSettings"]["realitySettings"]
            names = rset.get("serverNames") or []
            if names and valid_hostname(names[0]):
                decoy = names[0]
            reality_loopback = int(ib_r.get("port", 1443))
            vless_reality_sec = {
                "uuid": ib_r["settings"]["clients"][0]["id"],
                "x25519_private_key": rset["privateKey"],
                "x25519_public_key": qr.get("pbk", ""),
                "short_id": (rset.get("shortIds") or [""])[0],
                "mldsa65_seed": rset.get("mldsa65Seed", ""),
                "mldsa65_verify": qr.get("pqv", ""),
                "vless_decryption": ib_r["settings"].get("decryption", ""),
                "vless_encryption": qr.get("encryption", ""),
            }

        ib_x = next((i for i in xray_cfg.get("inbounds", []) if i.get("tag") == "vless-xhttp-pqc"), None)
        if ib_x:
            qx = uri_query("xhttp")
            xhttp_port = int(ib_x.get("port", 2053))
            vless_xhttp_sec = {
                "uuid": ib_x["settings"]["clients"][0]["id"],
                "path": ib_x["streamSettings"]["xhttpSettings"]["path"],
                "vless_decryption": ib_x["settings"].get("decryption", ""),
                "vless_encryption": qx.get("encryption", ""),
            }

        ib_a = next((i for i in sb_pqc.get("inbounds", []) if i.get("type") == "anytls"), None)
        if ib_a:
            anytls_port = int(ib_a["listen_port"])
            anytls_sec["password"] = ib_a["users"][0]["password"]
        ib_h = next((i for i in sb_pqc.get("inbounds", []) if i.get("type") == "hysteria2"), None)
        if ib_h:
            hy2_port = int(ib_h["listen_port"])
            hy2_up = int(ib_h.get("up_mbps", 100))
            hy2_down = int(ib_h.get("down_mbps", 200))
            hy2_sec = {
                "password": ib_h["users"][0]["password"],
                "obfs_password": (ib_h.get("obfs") or {}).get("password", ""),
            }
        ib_t = next((i for i in sb_pqc.get("inbounds", []) if i.get("type") == "tuic"), None)
        if ib_t:
            tuic_port = int(ib_t["listen_port"])
            tuic_sec = {
                "uuid": ib_t["users"][0]["uuid"],
                "password": ib_t["users"][0]["password"],
            }

        clash_p = root / "usr/local/etc/xray-sub/clash.yaml"
        if clash_p.is_file():
            extracted = []
            for line in clash_p.read_text(encoding="utf-8").splitlines():
                m = re.match(r"^\s*-\s*name:\s*(.+)$", line)
                if m and m.group(1).strip() not in ("代理", "TCP优选", "自动选择"):
                    extracted.append(m.group(1).strip())
            if len(extracted) >= 5:
                node_names = extracted[:5]

    # Argo migration if configured
    argo_cfg = None
    argo_ws_sec: dict = {}
    argo_ips: list[str] = []
    argo_candidates = sorted((root / "etc").glob("*argo-sub/config.json"))
    argo_json_p = argo_candidates[0] if argo_candidates else None
    sb_main_p = root / "etc/sing-box/config.json"
    if argo_json_p and argo_json_p.is_file() and sb_main_p.is_file():
        ad = json.loads(argo_json_p.read_text(encoding="utf-8"))
        sbm = json.loads(sb_main_p.read_text(encoding="utf-8"))
        argo_ips = [ip.strip() for ip in ad.get("preferred_ips", []) if str(ip).strip()]
        ws_list = []
        for idx, ib in enumerate(sbm.get("inbounds", [])):
            if (ib.get("transport") or {}).get("type") != "ws":
                continue
            proto = ib.get("type")
            if proto not in ("vmess", "vless"):
                continue
            users = ib.get("users") or []
            if not users or not users[0].get("uuid"):
                continue
            raw_tag = str(ib.get("tag") or f"ws-{idx + 1}")
            ws_id = re.sub(r"[^A-Za-z0-9_-]+", "-", raw_tag.lower()).strip("-") or f"ws-{idx + 1}"
            ws_list.append(
                {
                    "id": ws_id,
                    "name": raw_tag,
                    "protocol": proto,
                    "engine": "sing-box",
                    "listen": "127.0.0.1",
                    "port": int(ib["listen_port"]),
                    "tls": bool((ib.get("tls") or {}).get("enabled", False)),
                }
            )
            argo_ws_sec[ws_id] = {
                "uuid": users[0]["uuid"],
                "path": ib["transport"]["path"],
            }
        if ws_list and valid_hostname(ad.get("proxy_host", "")) and valid_hostname(ad.get("subscription_host", "")):
            argo_cfg = {
                "tunnel_domain": ad["proxy_host"].strip(),
                "sub_domain": ad["subscription_host"].strip(),
                "loopback_sub_port": int(ad.get("listen_port", 18081)),
                "ws_inbounds": ws_list,
            }

    # Traffic config & state
    limit_bytes = int(raw.get("monthly_traffic_limit", 0) or 0)
    monthly_gb = round(limit_bytes / float(1024**3), 2) if limit_bytes > 0 else 0.0
    reset_tz = str(raw.get("accounting_timezone", "UTC") or "UTC")
    display_tz = str(raw.get("client_display_timezone", "Asia/Shanghai") or "Asia/Shanghai")
    reset_day, reset_hour, reset_min = 1, 0, 0
    anchored = str(raw.get("anchored_reset_at", "") or "")
    m_rst = re.match(r"^\d{4}-\d{2}-(\d{2})T(\d{2}):(\d{2})", anchored)
    if str(raw.get("accounting_policy", "")) == "anchored-month" and m_rst:
        reset_day = max(1, min(31, int(m_rst.group(1))))
        reset_hour = min(23, int(m_rst.group(2)))
        reset_min = min(59, int(m_rst.group(3)))
    reset_time = f"{reset_hour:02d}:{reset_min:02d}"

    cur_rx = read_sysfs_counter(root, interface, "rx")
    cur_tx = read_sysfs_counter(root, interface, "tx")
    boot_id = read_boot_id(root)
    acc_rx, acc_tx = 0, 0
    sb_state_p = root / "var/lib/sbctl/state.json"
    if sb_state_p.is_file():
        st = json.loads(sb_state_p.read_text(encoding="utf-8"))
        acc_rx = int(st.get("accumulated_rx", 0))
        acc_tx = int(st.get("accumulated_tx", 0))
        if st.get("boot_id") == boot_id:
            brx = int(st.get("baseline_rx", cur_rx))
            btx = int(st.get("baseline_tx", cur_tx))
            if cur_rx >= brx:
                acc_rx += cur_rx - brx
            if cur_tx >= btx:
                acc_tx += cur_tx - btx

    next_reset_ts = compute_next_reset_ts(reset_day, reset_hour, reset_min, reset_tz)
    xs_state_p = root / "var/lib/xray-sub/traffic-state.json"
    if xs_state_p.is_file():
        xs_st = json.loads(xs_state_p.read_text(encoding="utf-8"))
        if isinstance(xs_st.get("next_reset_ts"), int) and xs_st["next_reset_ts"] > int(time.time()):
            next_reset_ts = int(xs_st["next_reset_ts"])

    summary = {
        "generation": "sbctl",
        "domain": domain,
        "public_ip": public_ip,
        "interface": interface,
        "tokens_preserved": len(tokens),
        "argo_enabled": bool(argo_cfg),
        "extra_https_port": extra_https_port,
        "traffic_acc_rx": acc_rx,
        "traffic_acc_tx": acc_tx,
    }
    if check_only:
        return summary

    vsm_ugid, vsm_gid = live_ids(root)
    toml_text = render_config_toml(
        profile_name="VPS 5Proto-PQC",
        domain=domain,
        public_ip=public_ip,
        interface=interface,
        ipv4_only=ipv4_only,
        extra_https_port=extra_https_port,
        http_fallback_port=http_fallback,
        monthly_gb=monthly_gb,
        reset_day=reset_day,
        reset_time=reset_time,
        reset_tz=reset_tz,
        display_tz=display_tz,
        decoy_sni=decoy,
        node_names=node_names,
        reality_loopback=reality_loopback,
        xhttp_port=xhttp_port,
        anytls_port=anytls_port,
        hy2_port=hy2_port,
        hy2_range=hy2_range,
        hy2_up=hy2_up,
        hy2_down=hy2_down,
        tuic_port=tuic_port,
        argo_cfg=argo_cfg,
    )
    write_atomic(
        root / "etc/vps-sub-meter/config.toml",
        toml_text.encode("utf-8"),
        0o640,
        uid=0 if is_root() else None,
        gid=vsm_gid,
    )

    secrets_obj = {
        "tokens": tokens,
        "vless_reality_pqc": vless_reality_sec,
        "vless_xhttp_pqc": vless_xhttp_sec,
        "anytls_tcp": anytls_sec,
        "hysteria2_hop": hy2_sec,
        "tuic_bbr": tuic_sec,
        "argo_ws": argo_ws_sec,
    }
    write_atomic(
        root / "etc/vps-sub-meter/secrets.json",
        (json.dumps(secrets_obj, indent=2) + "\n").encode("utf-8"),
        0o600,
        uid=0 if is_root() else None,
        gid=0 if is_root() else None,
    )

    if argo_ips:
        write_atomic(
            root / "etc/vps-sub-meter/argo-ips.txt",
            ("# Argo Cloudflare 优选 IP（支持自定义编辑，保存后即时生效）\n" + "\n".join(argo_ips) + "\n").encode("utf-8"),
            0o640,
            uid=0 if is_root() else None,
            gid=vsm_gid,
        )

    traffic_obj = {
        "schema": 1,
        "interface": interface,
        "boot_id": boot_id,
        "baseline_rx": cur_rx,
        "baseline_tx": cur_tx,
        "accumulated_rx": acc_rx,
        "accumulated_tx": acc_tx,
        "cycle_next_reset_ts": next_reset_ts,
    }
    write_atomic(
        root / "var/lib/vps-sub-meter/traffic-state.json",
        (json.dumps(traffic_obj, indent=2) + "\n").encode("utf-8"),
        0o640,
        uid=vsm_ugid[0] if vsm_ugid else None,
        gid=vsm_ugid[1] if vsm_ugid else None,
    )
    return summary


def migrate_sub_srv_v1(root: Path, check_only: bool) -> dict:
    conf = parse_subsrv_conf((root / "etc/sub-srv/config.conf").read_text(encoding="utf-8"))
    raw_host = conf.get("DOMAIN", "").strip()
    if not raw_host or is_ip_literal(raw_host):
        domain = ""
        public_ip = raw_host if is_ip_literal(raw_host) else detect_public_ip(root)
    else:
        domain = raw_host
        public_ip = detect_public_ip(root, "", domain)

    interface = detect_default_interface(root, conf.get("IFACE", "eth0"))
    try:
        monthly_gb = round(float(conf.get("TRAFFIC_LIMIT_GIB", "0") or 0), 2)
    except ValueError:
        monthly_gb = 0.0
    if monthly_gb < 0:
        monthly_gb = 0.0

    reset_tz = conf.get("TZ_NAME", "UTC").strip() or "UTC"
    reset_hour = min(23, max(0, int(conf.get("RESET_HOUR", "0") or 0)))
    reset_min = min(59, max(0, int(conf.get("RESET_MINUTE", "0") or 0)))
    reset_day = 1
    anchor_date = conf.get("RESET_ANCHOR_DATE", "").strip()
    m_date = re.match(r"^\d{4}-\d{2}-(\d{2})$", anchor_date)
    if m_date:
        reset_day = max(1, min(31, int(m_date.group(1))))
    elif re.fullmatch(r"\d+", conf.get("RESET_DAY", "").strip()):
        reset_day = max(1, min(31, int(conf["RESET_DAY"])))
    reset_time = f"{reset_hour:02d}:{reset_min:02d}"

    tok = conf.get("TOKEN", "").strip()
    tokens = (
        [{"name": "v1-migrated", "token": tok, "created_at": int(time.time())}]
        if len(tok) >= 16
        else []
    )

    cur_rx = read_sysfs_counter(root, interface, "rx")
    cur_tx = read_sysfs_counter(root, interface, "tx")
    boot_id = read_boot_id(root)
    acc_rx, acc_tx = 0, 0
    tx_state_p = root / "var/lib/subsrv/tx_state.json"
    if tx_state_p.is_file():
        st = json.loads(tx_state_p.read_text(encoding="utf-8"))
        base_rx = int(st.get("base_rx", cur_rx))
        base_tx = int(st.get("base_tx", cur_tx))
        acc_rx = max(0, cur_rx - base_rx)
        acc_tx = max(0, cur_tx - base_tx)
    elif conf.get("USED_TRAFFIC_GIB"):
        try:
            used_half = int(float(conf["USED_TRAFFIC_GIB"]) * (1024**3) / 2.0)
            acc_rx = max(0, used_half)
            acc_tx = max(0, used_half)
        except ValueError:
            pass

    next_reset_ts = compute_next_reset_ts(reset_day, reset_hour, reset_min, reset_tz)
    summary = {
        "generation": "sub-srv-v1",
        "domain": domain,
        "public_ip": public_ip,
        "interface": interface,
        "tokens_preserved": len(tokens),
        "argo_enabled": False,
        "extra_https_port": None,
        "traffic_acc_rx": acc_rx,
        "traffic_acc_tx": acc_tx,
    }
    if check_only:
        return summary

    vsm_ugid, vsm_gid = live_ids(root)
    toml_text = render_config_toml(
        profile_name="VPS 5Proto-PQC",
        domain=domain,
        public_ip=public_ip,
        interface=interface,
        ipv4_only=False,
        extra_https_port=None,
        http_fallback_port=2080,
        monthly_gb=monthly_gb,
        reset_day=reset_day,
        reset_time=reset_time,
        reset_tz=reset_tz,
        display_tz="Asia/Shanghai",
        decoy_sni="aws.amazon.com",
        node_names=[
            "vless-reality-pqc",
            "vless-xhttp-pqc",
            "anytls-tcp",
            "hysteria2-hop",
            "tuic-bbr",
        ],
        reality_loopback=1443,
        xhttp_port=2053,
        anytls_port=2083,
        hy2_port=34443,
        hy2_range="30000-35000",
        hy2_up=100,
        hy2_down=200,
        tuic_port=2096,
        argo_cfg=None,
    )
    write_atomic(
        root / "etc/vps-sub-meter/config.toml",
        toml_text.encode("utf-8"),
        0o640,
        uid=0 if is_root() else None,
        gid=vsm_gid,
    )

    secrets_obj = {
        "tokens": tokens,
        "vless_reality_pqc": {},
        "vless_xhttp_pqc": {},
        "anytls_tcp": {},
        "hysteria2_hop": {},
        "tuic_bbr": {},
        "argo_ws": {},
    }
    write_atomic(
        root / "etc/vps-sub-meter/secrets.json",
        (json.dumps(secrets_obj, indent=2) + "\n").encode("utf-8"),
        0o600,
        uid=0 if is_root() else None,
        gid=0 if is_root() else None,
    )

    traffic_obj = {
        "schema": 1,
        "interface": interface,
        "boot_id": boot_id,
        "baseline_rx": cur_rx,
        "baseline_tx": cur_tx,
        "accumulated_rx": acc_rx,
        "accumulated_tx": acc_tx,
        "cycle_next_reset_ts": next_reset_ts,
    }
    write_atomic(
        root / "var/lib/vps-sub-meter/traffic-state.json",
        (json.dumps(traffic_obj, indent=2) + "\n").encode("utf-8"),
        0o640,
        uid=vsm_ugid[0] if vsm_ugid else None,
        gid=vsm_ugid[1] if vsm_ugid else None,
    )
    return summary


def run_migration(root: Path, check_only: bool = False) -> dict:
    gen = detect_generation(root)
    if gen is None:
        raise FileNotFoundError(
            "未检测到历史部署（/etc/vps-sub-meter、/etc/sbctl 或 /etc/sub-srv），请使用 install.sh 全新安装"
        )
    if gen == "v2":
        return {"generation": "v2", "migrated": False}
    if gen == "sbctl":
        res = migrate_sbctl(root, check_only=check_only)
        res["migrated"] = not check_only
        return res
    res = migrate_sub_srv_v1(root, check_only=check_only)
    res["migrated"] = not check_only
    return res


def main() -> int:
    ap = argparse.ArgumentParser(description="vps-sub-meter universal legacy migrator")
    ap.add_argument("--root", default="/", help="Target filesystem root")
    ap.add_argument("--check", action="store_true", help="Inspect only without writing")
    args = ap.parse_args()
    root = Path(args.root)
    try:
        res = run_migration(root, check_only=args.check)
    except FileNotFoundError as exc:
        print(str(exc), file=sys.stderr)
        return 3
    print(json.dumps(res, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
