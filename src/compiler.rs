//! Dual-core server (Xray + sing-box) and 4-format client subscription compiler,
//! plus dynamic Cloudflare Argo preferred-IP subscription rendering.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use crate::config::{
    ArgoWsInbound, Config, EngineKind, WsProtocol, format_url_host, is_public_ipv4,
};
use crate::fs_util;
use crate::merge;
use crate::secrets::Secrets;

const BUNDLED_MIHOMO_TEMPLATE: &str = include_str!("../templates/mihomo.yaml");
const BUNDLED_SINGBOX_TEMPLATE: &str = include_str!("../templates/sing-box.json");

pub const ANYTLS_PADDING_SCHEME: &[&str] = &[
    "stop=8",
    "0=30-30",
    "1=100-400",
    "2=400-500,c,500-1000,c,500-1000,c,500-1000,c,500-1000",
    "3=9-9,500-1000",
    "4=500-1000",
    "5=500-1000",
    "6=500-1000",
    "7=500-1000",
];

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("配置校验失败: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("读取模板或覆写文件失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("模板或覆写格式错误: {0}")]
    Format(String),
}

#[derive(Clone, Debug)]
pub struct ClientBundle {
    pub clash_yaml: String,
    pub sing_box_json: Option<String>,
    pub shadowrocket_txt: String,
    pub uri_txt: String,
    pub direct_node_names: Vec<String>,
    pub singbox_node_names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CompiledDeployment {
    pub xray_server: Option<String>,
    pub singbox_server: Option<String>,
    pub clients: ClientBundle,
}

#[derive(Clone, Debug)]
pub struct CertPaths {
    pub fullchain: String,
    pub privkey: String,
}

pub fn cert_paths(root: &Path, config: &Config) -> CertPaths {
    let base = if root == Path::new("/") {
        PathBuf::from("/var/lib/vps-sub-meter/certificates")
    } else {
        root.join("var/lib/vps-sub-meter/certificates")
    };
    let dir = base.join(config.tls_sni());
    CertPaths {
        fullchain: dir.join("fullchain.pem").to_string_lossy().into_owned(),
        privkey: dir.join("privkey.pem").to_string_lossy().into_owned(),
    }
}

pub fn load_mihomo_template(root: &Path) -> Result<serde_yaml::Value, CompileError> {
    let custom = root.join("etc/vps-sub-meter/templates/mihomo.yaml");
    let text = match fs_util::read_to_string_no_follow(&custom) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BUNDLED_MIHOMO_TEMPLATE.to_owned(),
        Err(e) => return Err(e.into()),
    };
    serde_yaml::from_str(&text).map_err(|e| CompileError::Format(format!("mihomo.yaml: {e}")))
}

pub fn load_singbox_template(root: &Path) -> Result<Value, CompileError> {
    let custom = root.join("etc/vps-sub-meter/templates/sing-box.json");
    let text = match fs_util::read_to_string_no_follow(&custom) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BUNDLED_SINGBOX_TEMPLATE.to_owned(),
        Err(e) => return Err(e.into()),
    };
    serde_json::from_str(&text).map_err(|e| CompileError::Format(format!("sing-box.json: {e}")))
}

fn load_optional_json_override(root: &Path, rel: &str) -> Result<Option<Value>, CompileError> {
    match fs_util::read_to_string_no_follow(&root.join(rel)) {
        Ok(s) => {
            let v: Value = serde_json::from_str(&s)
                .map_err(|e| CompileError::Format(format!("{rel}: {e}")))?;
            if !v.is_object() {
                return Err(CompileError::Format(format!("{rel} 顶层必须是 JSON 对象")));
            }
            Ok(Some(v))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn compile_all(
    root: &Path,
    config: &Config,
    secrets: &Secrets,
) -> Result<CompiledDeployment, CompileError> {
    config.validate()?;
    let certs = cert_paths(root, config);
    let xray_server = compile_xray_server(root, config, secrets, &certs)?;
    let singbox_server = compile_singbox_server(root, config, secrets, &certs)?;
    let clients = compile_direct_clients(root, config, secrets)?;
    Ok(CompiledDeployment {
        xray_server,
        singbox_server,
        clients,
    })
}

pub fn compile_xray_server(
    root: &Path,
    config: &Config,
    secrets: &Secrets,
    certs: &CertPaths,
) -> Result<Option<String>, CompileError> {
    if !config.any_xray_enabled() {
        return Ok(None);
    }

    let mut inbounds = Vec::new();
    let mut rules = Vec::new();
    let mut outbounds = vec![
        json!({
            "tag": "direct",
            "protocol": "freedom",
            "settings": { "domainStrategy": if config.ipv4_only { "ForceIPv4" } else { "AsIs" } }
        }),
        json!({ "tag": "block", "protocol": "blackhole" }),
    ];

    if config.xray_fronts_443() {
        let lb_port = config.nodes.vless_reality_pqc.loopback_port;
        inbounds.push(json!({
            "tag": "sni-demux-443",
            "listen": "0.0.0.0",
            "port": config.server.https_port,
            "protocol": "dokodemo-door",
            "settings": {
                "address": "127.0.0.1",
                "port": lb_port,
                "network": "tcp"
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["tls"],
                "routeOnly": true
            }
        }));
        outbounds.push(json!({
            "tag": "to-vsm-https",
            "protocol": "freedom",
            "settings": {
                "redirect": format!("127.0.0.1:{}", config.server.loopback_https_port)
            }
        }));
        outbounds.push(json!({
            "tag": "to-reality-local",
            "protocol": "freedom",
            "settings": {
                "redirect": format!("127.0.0.1:{lb_port}")
            }
        }));
        rules.push(json!({
            "type": "field",
            "inboundTag": ["sni-demux-443"],
            "domain": [format!("full:{}", config.domain.trim())],
            "outboundTag": "to-vsm-https"
        }));
        rules.push(json!({
            "type": "field",
            "inboundTag": ["sni-demux-443"],
            "outboundTag": "to-reality-local"
        }));
    }

    if config.nodes.vless_reality_pqc.enabled {
        let (listen, port) = if config.xray_fronts_443() {
            ("127.0.0.1", config.nodes.vless_reality_pqc.loopback_port)
        } else {
            ("0.0.0.0", config.nodes.vless_reality_pqc.port)
        };
        let decoy = config.nodes.vless_reality_pqc.decoy_sni.trim();
        let s = &secrets.vless_reality_pqc;
        inbounds.push(json!({
            "tag": "vless-reality-pqc",
            "listen": listen,
            "port": port,
            "protocol": "vless",
            "settings": {
                "clients": [{
                    "id": s.uuid,
                    "flow": "xtls-rprx-vision"
                }],
                "decryption": s.vless_decryption
            },
            "streamSettings": {
                "network": "raw",
                "security": "reality",
                "realitySettings": {
                    "show": false,
                    "target": format!("{decoy}:443"),
                    "serverNames": [decoy],
                    "privateKey": s.x25519_private_key,
                    "shortIds": [s.short_id],
                    "mldsa65Seed": s.mldsa65_seed
                }
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "routeOnly": true
            }
        }));
    }

    if config.nodes.vless_xhttp_pqc.enabled {
        let s = &secrets.vless_xhttp_pqc;
        let sni = config.tls_sni();
        inbounds.push(json!({
            "tag": "vless-xhttp-pqc",
            "listen": "0.0.0.0",
            "port": config.nodes.vless_xhttp_pqc.port,
            "protocol": "vless",
            "settings": {
                "clients": [{ "id": s.uuid }],
                "decryption": s.vless_decryption
            },
            "streamSettings": {
                "network": "xhttp",
                "security": "tls",
                "tlsSettings": {
                    "serverName": sni,
                    "minVersion": "1.3",
                    "alpn": ["h2", "http/1.1"],
                    "certificates": [{
                        "certificateFile": certs.fullchain,
                        "keyFile": certs.privkey
                    }]
                },
                "xhttpSettings": {
                    "path": s.path,
                    "host": sni,
                    "mode": "auto"
                }
            },
            "sniffing": {
                "enabled": true,
                "destOverride": ["http", "tls", "quic"],
                "routeOnly": true
            }
        }));
    }

    if config.argo.enabled {
        for ws in config
            .argo
            .ws_inbounds
            .iter()
            .filter(|w| w.engine == EngineKind::Xray)
        {
            if let Some(sec) = secrets.argo_ws.get(&ws.id) {
                inbounds.push(render_xray_ws_inbound(ws, &sec.uuid, &sec.path, certs));
            }
        }
    }

    // Anti-SSRF block rule MUST come AFTER sni-demux-443 rules!
    rules.push(json!({
        "type": "field",
        "ip": [
            "127.0.0.0/8",
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "169.254.0.0/16",
            "100.64.0.0/10",
            "198.18.0.0/15",
            "::1/128",
            "fc00::/7",
            "fe80::/10"
        ],
        "outboundTag": "block"
    }));

    let mut doc = json!({
        "log": {
            "loglevel": "warning",
            "access": "none"
        },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "routing": {
            "domainStrategy": "AsIs",
            "rules": rules
        }
    });
    if let Some(overlay) =
        load_optional_json_override(root, "etc/vps-sub-meter/overrides/xray.json")?
    {
        merge::deep_merge(&mut doc, &overlay);
    }
    Ok(Some(serde_json::to_string_pretty(&doc).unwrap()))
}

fn render_xray_ws_inbound(ws: &ArgoWsInbound, uuid: &str, path: &str, certs: &CertPaths) -> Value {
    let mut stream = json!({
        "network": "ws",
        "security": if ws.tls { "tls" } else { "none" },
        "wsSettings": { "path": path }
    });
    if ws.tls {
        stream["tlsSettings"] = json!({
            "alpn": ["http/1.1"],
            "certificates": [{
                "certificateFile": certs.fullchain,
                "keyFile": certs.privkey
            }]
        });
    }
    let settings = match ws.protocol {
        WsProtocol::Vless => json!({ "clients": [{ "id": uuid }], "decryption": "none" }),
        WsProtocol::Vmess => json!({ "clients": [{ "id": uuid }] }),
    };
    json!({
        "tag": format!("argo-{}", ws.id),
        "listen": ws.listen,
        "port": ws.port,
        "protocol": match ws.protocol {
            WsProtocol::Vless => "vless",
            WsProtocol::Vmess => "vmess",
        },
        "settings": settings,
        "streamSettings": stream
    })
}

pub fn compile_singbox_server(
    root: &Path,
    config: &Config,
    secrets: &Secrets,
    certs: &CertPaths,
) -> Result<Option<String>, CompileError> {
    if !config.any_singbox_enabled() {
        return Ok(None);
    }
    let sni = config.tls_sni();
    let mut inbounds = Vec::new();

    if config.nodes.anytls_tcp.enabled {
        inbounds.push(json!({
            "type": "anytls",
            "tag": "anytls-tcp",
            "listen": "0.0.0.0",
            "listen_port": config.nodes.anytls_tcp.port,
            "users": [{
                "name": "default",
                "password": secrets.anytls_tcp.password
            }],
            "padding_scheme": ANYTLS_PADDING_SCHEME,
            "tls": {
                "enabled": true,
                "server_name": sni,
                "certificate_path": certs.fullchain,
                "key_path": certs.privkey
            }
        }));
    }

    if config.nodes.hysteria2_hop.enabled {
        let hy = &config.nodes.hysteria2_hop;
        let mut obj = json!({
            "type": "hysteria2",
            "tag": "hysteria2-hop",
            "listen": "0.0.0.0",
            "listen_port": hy.port,
            "up_mbps": hy.server_up_mbps,
            "down_mbps": hy.server_down_mbps,
            "users": [{
                "name": "default",
                "password": secrets.hysteria2_hop.password
            }],
            "tls": {
                "enabled": true,
                "server_name": sni,
                "alpn": ["h3"],
                "certificate_path": certs.fullchain,
                "key_path": certs.privkey
            }
        });
        if hy.obfs_salamander {
            obj["obfs"] = json!({
                "type": "salamander",
                "password": secrets.hysteria2_hop.obfs_password
            });
        }
        inbounds.push(obj);
    }

    if config.nodes.tuic_bbr.enabled {
        inbounds.push(json!({
            "type": "tuic",
            "tag": "tuic-bbr",
            "listen": "0.0.0.0",
            "listen_port": config.nodes.tuic_bbr.port,
            "users": [{
                "name": "default",
                "uuid": secrets.tuic_bbr.uuid,
                "password": secrets.tuic_bbr.password
            }],
            "congestion_control": "bbr",
            "zero_rtt_handshake": true,
            "auth_timeout": "3s",
            "heartbeat": "10s",
            "tls": {
                "enabled": true,
                "server_name": sni,
                "alpn": ["h3"],
                "certificate_path": certs.fullchain,
                "key_path": certs.privkey
            }
        }));
    }

    if config.argo.enabled {
        for ws in config
            .argo
            .ws_inbounds
            .iter()
            .filter(|w| w.engine == EngineKind::SingBox)
        {
            if let Some(sec) = secrets.argo_ws.get(&ws.id) {
                let mut inbound = json!({
                    "type": match ws.protocol {
                        WsProtocol::Vless => "vless",
                        WsProtocol::Vmess => "vmess",
                    },
                    "tag": format!("argo-{}", ws.id),
                    "listen": ws.listen,
                    "listen_port": ws.port,
                    "users": [if ws.protocol == WsProtocol::Vless {
                        json!({ "uuid": sec.uuid })
                    } else {
                        json!({ "uuid": sec.uuid, "alterId": 0 })
                    }],
                    "transport": {
                        "type": "ws",
                        "path": sec.path
                    }
                });
                if ws.tls {
                    inbound["tls"] = json!({
                        "enabled": true,
                        "server_name": sni,
                        "alpn": ["http/1.1"],
                        "certificate_path": certs.fullchain,
                        "key_path": certs.privkey
                    });
                }
                inbounds.push(inbound);
            }
        }
    }

    let mut rules = Vec::new();
    if config.ipv4_only {
        rules.push(json!({ "action": "resolve", "strategy": "ipv4_only" }));
    }
    rules.push(json!({ "action": "reject", "ip_cidr": ["198.18.0.0/15"] }));
    rules.push(json!({ "action": "reject", "ip_is_private": true }));
    rules.push(json!({
        "action": "reject",
        "ip_cidr": [
            "0.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "224.0.0.0/4",
            "::1/128",
            "fe80::/10"
        ]
    }));

    let mut doc = json!({
        "log": { "level": "warn" },
        "dns": { "strategy": if config.ipv4_only { "ipv4_only" } else { "prefer_ipv4" } },
        "inbounds": inbounds,
        "outbounds": [{ "type": "direct", "tag": "direct" }],
        "route": { "rules": rules }
    });
    if let Some(overlay) =
        load_optional_json_override(root, "etc/vps-sub-meter/overrides/sing-box.json")?
    {
        merge::deep_merge(&mut doc, &overlay);
    }
    Ok(Some(serde_json::to_string_pretty(&doc).unwrap()))
}

pub fn compile_direct_clients(
    root: &Path,
    config: &Config,
    secrets: &Secrets,
) -> Result<ClientBundle, CompileError> {
    let insecure = config.is_ip_mode();
    let host = config.primary_host();
    let sni = config.tls_sni();

    let mut clash_proxies = Vec::new();
    let mut tcp_names = Vec::new();
    let mut all_names = Vec::new();
    let mut uris = Vec::new();
    let mut singbox_outbounds = Vec::new();
    let mut singbox_names = Vec::new();

    if config.nodes.vless_reality_pqc.enabled {
        let n = &config.nodes.vless_reality_pqc;
        let s = &secrets.vless_reality_pqc;
        let connect_host = n
            .connect_host
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| {
                if !config.public_ip.trim().is_empty() {
                    config.public_ip.trim()
                } else {
                    host
                }
            });
        all_names.push(n.name.clone());
        tcp_names.push(n.name.clone());
        clash_proxies.push(json!({
            "name": n.name,
            "type": "vless",
            "server": connect_host,
            "port": n.port,
            "uuid": s.uuid,
            "network": "tcp",
            "udp": true,
            "tls": true,
            "servername": n.decoy_sni,
            "client-fingerprint": "chrome",
            "flow": "xtls-rprx-vision",
            "encryption": s.vless_encryption,
            "reality-opts": {
                "public-key": s.x25519_public_key,
                "short-id": s.short_id,
                "support-x25519mlkem768": true
            }
        }));
        uris.push(format!(
            "vless://{}@{}:{}?encryption={}&flow=xtls-rprx-vision&security=reality&sni={}&fp=chrome&pbk={}&sid={}&pqv={}&type=tcp#{}",
            s.uuid,
            format_url_host(connect_host),
            n.port,
            pct(&s.vless_encryption),
            pct(&n.decoy_sni),
            pct(&s.x25519_public_key),
            pct(&s.short_id),
            pct(&s.mldsa65_verify),
            pct(&n.name)
        ));
    }

    if config.nodes.vless_xhttp_pqc.enabled {
        let n = &config.nodes.vless_xhttp_pqc;
        let s = &secrets.vless_xhttp_pqc;
        all_names.push(n.name.clone());
        tcp_names.push(n.name.clone());
        clash_proxies.push(json!({
            "name": n.name,
            "type": "vless",
            "server": host,
            "port": n.port,
            "uuid": s.uuid,
            "network": "xhttp",
            "udp": true,
            "tls": true,
            "servername": sni,
            "alpn": ["h2"],
            "client-fingerprint": "chrome",
            "skip-cert-verify": insecure,
            "encryption": s.vless_encryption,
            "xhttp-opts": {
                "path": s.path,
                "host": sni,
                "mode": "auto"
            }
        }));
        let extra_insecure = if insecure {
            "&allowInsecure=1&insecure=1"
        } else {
            ""
        };
        uris.push(format!(
            "vless://{}@{}:{}?encryption={}&security=tls&sni={}&alpn=h2&fp=chrome&type=xhttp&host={}&path={}&mode=auto{}#{}",
            s.uuid,
            format_url_host(host),
            n.port,
            pct(&s.vless_encryption),
            pct(sni),
            pct(sni),
            pct(&s.path),
            extra_insecure,
            pct(&n.name)
        ));
    }

    if config.nodes.anytls_tcp.enabled {
        let n = &config.nodes.anytls_tcp;
        let s = &secrets.anytls_tcp;
        all_names.push(n.name.clone());
        tcp_names.push(n.name.clone());
        singbox_names.push(n.name.clone());
        clash_proxies.push(json!({
            "name": n.name,
            "type": "anytls",
            "server": host,
            "port": n.port,
            "password": s.password,
            "sni": sni,
            "client-fingerprint": "chrome",
            "skip-cert-verify": insecure,
            "udp": true,
            "idle-session-check-interval": 30,
            "idle-session-timeout": 30,
            "min-idle-session": 0
        }));
        singbox_outbounds.push(json!({
            "type": "anytls",
            "tag": n.name,
            "server": host,
            "server_port": n.port,
            "password": s.password,
            "idle_session_check_interval": "30s",
            "idle_session_timeout": "30s",
            "min_idle_session": 5,
            "tls": {
                "enabled": true,
                "server_name": sni,
                "insecure": insecure
            }
        }));
        uris.push(format!(
            "anytls://{}@{}:{}/?insecure={}&sni={}&fp=chrome#{}",
            pct(&s.password),
            format_url_host(host),
            n.port,
            u8::from(insecure),
            pct(sni),
            pct(&n.name)
        ));
    }

    if config.nodes.hysteria2_hop.enabled {
        let n = &config.nodes.hysteria2_hop;
        let s = &secrets.hysteria2_hop;
        let hop = config.parse_hop_range()?;
        all_names.push(n.name.clone());
        singbox_names.push(n.name.clone());

        let mut clash_hy = json!({
            "name": n.name,
            "type": "hysteria2",
            "server": host,
            "port": n.port,
            "password": s.password,
            "up": format!("{} Mbps", n.up_mbps),
            "down": format!("{} Mbps", n.down_mbps),
            "sni": sni,
            "alpn": ["h3"],
            "skip-cert-verify": insecure
        });
        if let Some((start, end)) = hop {
            clash_hy["ports"] = json!(format!("{start}-{end}"));
            clash_hy["hop-interval"] = json!(n.hop_interval_sec);
        }
        if n.obfs_salamander {
            clash_hy["obfs"] = json!("salamander");
            clash_hy["obfs-password"] = json!(s.obfs_password);
        }
        clash_proxies.push(clash_hy);

        let mut sb_hy = json!({
            "type": "hysteria2",
            "tag": n.name,
            "server": host,
            "password": s.password,
            "up_mbps": n.up_mbps,
            "down_mbps": n.down_mbps,
            "tls": {
                "enabled": true,
                "server_name": sni,
                "alpn": ["h3"],
                "insecure": insecure
            }
        });
        if let Some((start, end)) = hop {
            sb_hy["server_ports"] = json!([format!("{start}:{end}")]);
            sb_hy["hop_interval"] = json!(format!("{}s", n.hop_interval_sec));
        } else {
            sb_hy["server_port"] = json!(n.port);
        }
        if n.obfs_salamander {
            sb_hy["obfs"] = json!({
                "type": "salamander",
                "password": s.obfs_password
            });
        }
        singbox_outbounds.push(sb_hy);

        let mut query = Vec::new();
        if let Some((start, end)) = hop {
            query.push(format!("mport={start}-{end}"));
        }
        if n.obfs_salamander {
            query.push("obfs=salamander".into());
            query.push(format!("obfs-password={}", pct(&s.obfs_password)));
        }
        query.push(format!("sni={}", pct(sni)));
        query.push(format!("insecure={}", u8::from(insecure)));
        uris.push(format!(
            "hysteria2://{}@{}:{}/?{}#{}",
            pct(&s.password),
            format_url_host(host),
            n.port,
            query.join("&"),
            pct(&n.name)
        ));
    }

    if config.nodes.tuic_bbr.enabled {
        let n = &config.nodes.tuic_bbr;
        let s = &secrets.tuic_bbr;
        all_names.push(n.name.clone());
        singbox_names.push(n.name.clone());
        clash_proxies.push(json!({
            "name": n.name,
            "type": "tuic",
            "server": host,
            "port": n.port,
            "uuid": s.uuid,
            "password": s.password,
            "sni": sni,
            "alpn": ["h3"],
            "congestion-controller": "bbr",
            "udp-relay-mode": "native",
            "reduce-rtt": true,
            "heartbeat-interval": 10000,
            "skip-cert-verify": insecure,
            "disable-sni": false
        }));
        singbox_outbounds.push(json!({
            "type": "tuic",
            "tag": n.name,
            "server": host,
            "server_port": n.port,
            "uuid": s.uuid,
            "password": s.password,
            "congestion_control": "bbr",
            "udp_relay_mode": "native",
            "zero_rtt_handshake": true,
            "heartbeat": "10s",
            "tls": {
                "enabled": true,
                "server_name": sni,
                "alpn": ["h3"],
                "insecure": insecure
            }
        }));
        uris.push(format!(
            "tuic://{}:{}@{}:{}?congestion_control=bbr&udp_relay_mode=native&alpn=h3&insecure={}&sni={}#{}",
            pct(&s.uuid),
            pct(&s.password),
            format_url_host(host),
            n.port,
            u8::from(insecure),
            pct(sni),
            pct(&n.name)
        ));
    }

    let clash_yaml = render_clash_from_template(
        root,
        config,
        &clash_proxies,
        &all_names,
        &tcp_names,
        false,
        None,
    )?;

    let sing_box_json = if singbox_outbounds.is_empty() {
        None
    } else {
        Some(render_singbox_from_template(
            root,
            config,
            &singbox_outbounds,
            &singbox_names,
            false,
            None,
        )?)
    };

    let uri_txt = format!("{}\n", uris.join("\n"));
    let shadowrocket_txt = STANDARD.encode(uri_txt.as_bytes());

    Ok(ClientBundle {
        clash_yaml,
        sing_box_json,
        shadowrocket_txt,
        uri_txt,
        direct_node_names: all_names,
        singbox_node_names: singbox_names,
    })
}

pub fn load_argo_preferred_ips(root: &Path, config: &Config) -> Vec<String> {
    let raw = config.argo.preferred_ips_file.trim();
    let path = if root == Path::new("/") {
        PathBuf::from(raw)
    } else if let Some(rel) = raw.strip_prefix('/') {
        root.join(rel)
    } else {
        root.join(raw)
    };
    let Ok(text) = fs_util::read_to_string_no_follow(&path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let token = line.split('#').next().unwrap_or("").trim();
        if token.is_empty() {
            continue;
        }
        if let Ok(ip) = token.parse::<Ipv4Addr>()
            && is_public_ipv4(ip)
            && !out.contains(&ip.to_string())
        {
            out.push(ip.to_string());
            if out.len() >= 32 {
                break;
            }
        }
    }
    out
}

pub fn compile_argo_suite(
    root: &Path,
    config: &Config,
    secrets: &Secrets,
) -> Result<ClientBundle, CompileError> {
    let preferred = load_argo_preferred_ips(root, config);
    let tunnel = config.argo.tunnel_domain.trim();
    let sub_domain = config.argo.sub_domain.trim();

    let mut clash_proxies = Vec::new();
    let mut singbox_outbounds = Vec::new();
    let mut uris = Vec::new();
    let mut all_names = Vec::new();

    // Preferred-IP expanded nodes first, then fallback domain nodes.
    let mut targets: Vec<(String, String)> = preferred
        .iter()
        .enumerate()
        .map(|(i, ip)| (ip.clone(), format!("CF-{:02} {}", i + 1, ip)))
        .collect();
    targets.push((tunnel.to_owned(), "回退域名".to_owned()));

    for (server_addr, suffix) in &targets {
        for ws in &config.argo.ws_inbounds {
            let Some(sec) = secrets.argo_ws.get(&ws.id) else {
                continue;
            };
            let label = format!("{} [{suffix}]", ws.name);
            all_names.push(label.clone());
            match ws.protocol {
                WsProtocol::Vless => {
                    clash_proxies.push(json!({
                        "name": label,
                        "type": "vless",
                        "server": server_addr,
                        "port": 443,
                        "uuid": sec.uuid,
                        "network": "ws",
                        "udp": false,
                        "tls": true,
                        "servername": tunnel,
                        "client-fingerprint": "chrome",
                        "skip-cert-verify": false,
                        "ws-opts": {
                            "path": sec.path,
                            "headers": { "Host": tunnel }
                        }
                    }));
                    singbox_outbounds.push(json!({
                        "type": "vless",
                        "tag": label,
                        "server": server_addr,
                        "server_port": 443,
                        "uuid": sec.uuid,
                        "tls": {
                            "enabled": true,
                            "server_name": tunnel,
                            "utls": { "enabled": true, "fingerprint": "chrome" }
                        },
                        "transport": {
                            "type": "ws",
                            "path": sec.path,
                            "headers": { "Host": tunnel }
                        }
                    }));
                    uris.push(format!(
                        "vless://{}@{}:443?encryption=none&security=tls&sni={}&fp=chrome&type=ws&host={}&path={}#{}",
                        sec.uuid,
                        server_addr,
                        pct(tunnel),
                        pct(tunnel),
                        pct(&sec.path),
                        pct(&label)
                    ));
                }
                WsProtocol::Vmess => {
                    clash_proxies.push(json!({
                        "name": label,
                        "type": "vmess",
                        "server": server_addr,
                        "port": 443,
                        "uuid": sec.uuid,
                        "alterId": 0,
                        "cipher": "auto",
                        "network": "ws",
                        "udp": false,
                        "tls": true,
                        "servername": tunnel,
                        "client-fingerprint": "chrome",
                        "skip-cert-verify": false,
                        "ws-opts": {
                            "path": sec.path,
                            "headers": { "Host": tunnel }
                        }
                    }));
                    singbox_outbounds.push(json!({
                        "type": "vmess",
                        "tag": label,
                        "server": server_addr,
                        "server_port": 443,
                        "uuid": sec.uuid,
                        "security": "auto",
                        "alter_id": 0,
                        "tls": {
                            "enabled": true,
                            "server_name": tunnel,
                            "utls": { "enabled": true, "fingerprint": "chrome" }
                        },
                        "transport": {
                            "type": "ws",
                            "path": sec.path,
                            "headers": { "Host": tunnel }
                        }
                    }));
                    let payload = json!({
                        "v": "2",
                        "ps": label,
                        "add": server_addr,
                        "port": "443",
                        "id": sec.uuid,
                        "aid": "0",
                        "scy": "auto",
                        "net": "ws",
                        "type": "none",
                        "host": tunnel,
                        "path": sec.path,
                        "tls": "tls",
                        "sni": tunnel
                    });
                    uris.push(format!(
                        "vmess://{}",
                        STANDARD.encode(serde_json::to_vec(&payload).unwrap())
                    ));
                }
            }
        }
    }

    let clash_yaml = render_clash_from_template(
        root,
        config,
        &clash_proxies,
        &all_names,
        &[],
        true,
        Some((tunnel, sub_domain)),
    )?;
    let sing_box_json = if singbox_outbounds.is_empty() {
        None
    } else {
        Some(render_singbox_from_template(
            root,
            config,
            &singbox_outbounds,
            &all_names,
            true,
            Some((tunnel, sub_domain)),
        )?)
    };
    let uri_txt = if uris.is_empty() {
        String::new()
    } else {
        format!("{}\n", uris.join("\n"))
    };
    let shadowrocket_txt = STANDARD.encode(uri_txt.as_bytes());
    Ok(ClientBundle {
        clash_yaml,
        sing_box_json,
        shadowrocket_txt,
        uri_txt,
        direct_node_names: all_names.clone(),
        singbox_node_names: all_names,
    })
}

fn render_clash_from_template(
    root: &Path,
    config: &Config,
    proxies: &[Value],
    all_names: &[String],
    tcp_names: &[String],
    argo_mode: bool,
    argo_domains: Option<(&str, &str)>,
) -> Result<String, CompileError> {
    let mut doc = load_mihomo_template(root)?;
    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| CompileError::Format("mihomo.yaml 必须是 mapping".into()))?;

    let mut groups = Vec::new();
    let probe = &config.latency_probe_url;

    if argo_mode {
        let mut selector_members = vec!["Argo优选".to_owned()];
        selector_members.extend(all_names.iter().cloned());
        selector_members.push("DIRECT".into());
        groups.push(json!({
            "name": "代理",
            "type": "select",
            "url": probe,
            "interval": 300,
            "proxies": selector_members
        }));
        groups.push(json!({
            "name": "Argo优选",
            "type": "url-test",
            "url": probe,
            "interval": 300,
            "tolerance": 100,
            "proxies": all_names
        }));
    } else {
        let mut selector_members = all_names.to_vec();
        if !tcp_names.is_empty() {
            selector_members.push("TCP优选".into());
        }
        selector_members.push("自动选择".into());
        selector_members.push("DIRECT".into());
        groups.push(json!({
            "name": "代理",
            "type": "select",
            "url": probe,
            "interval": 300,
            "proxies": selector_members
        }));
        if !tcp_names.is_empty() {
            groups.push(json!({
                "name": "TCP优选",
                "type": "url-test",
                "url": probe,
                "interval": 300,
                "tolerance": 50,
                "proxies": tcp_names
            }));
        }
        groups.push(json!({
            "name": "自动选择",
            "type": "url-test",
            "url": probe,
            "interval": 300,
            "tolerance": 50,
            "proxies": all_names
        }));
    }

    map.insert(
        serde_yaml::Value::String("proxies".into()),
        serde_yaml::to_value(proxies).unwrap(),
    );
    map.insert(
        serde_yaml::Value::String("proxy-groups".into()),
        serde_yaml::to_value(&groups).unwrap(),
    );

    // Ensure Fake-IP reject guard is always at the top of rules, and deduplicate.
    let existing_rules = map
        .get("rules")
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    let mut final_rules = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push_rule = |r: &str| {
        let norm = r.replace(' ', "").to_ascii_lowercase();
        if seen.insert(norm) {
            final_rules.push(serde_yaml::Value::String(r.to_owned()));
        }
    };

    if argo_mode {
        push_rule("NETWORK,udp,REJECT");
        if let Some((tunnel, sub)) = argo_domains {
            push_rule(&format!("DOMAIN,{sub},DIRECT"));
            push_rule(&format!("DOMAIN,{tunnel},DIRECT"));
        }
    }
    push_rule("IP-CIDR,198.18.0.0/15,REJECT,no-resolve");
    for item in existing_rules {
        if let Some(s) = item.as_str() {
            push_rule(s);
        }
    }
    map.insert(
        serde_yaml::Value::String("rules".into()),
        serde_yaml::Value::Sequence(final_rules),
    );

    if argo_mode
        && let Some((tunnel, sub)) = argo_domains
        && let Some(dns) = map.get_mut("dns").and_then(|v| v.as_mapping_mut())
        && let Some(policy) = dns
            .get_mut("nameserver-policy")
            .and_then(|v| v.as_mapping_mut())
    {
        policy.insert(
            serde_yaml::Value::String(format!("{sub},{tunnel}")),
            serde_yaml::to_value(vec!["https://1.12.12.12/dns-query"]).unwrap(),
        );
    }

    serde_yaml::to_string(&doc).map_err(|e| CompileError::Format(e.to_string()))
}

fn render_singbox_from_template(
    root: &Path,
    config: &Config,
    node_outbounds: &[Value],
    node_tags: &[String],
    argo_mode: bool,
    argo_domains: Option<(&str, &str)>,
) -> Result<String, CompileError> {
    let mut doc = load_singbox_template(root)?;
    let probe = &config.latency_probe_url;

    let mut outbounds = Vec::new();
    if argo_mode {
        let mut sel = vec!["Argo优选".to_owned()];
        sel.extend(node_tags.iter().cloned());
        sel.push("DIRECT".into());
        outbounds.push(json!({
            "type": "selector",
            "tag": "代理",
            "outbounds": sel,
            "default": "Argo优选"
        }));
        outbounds.push(json!({
            "type": "urltest",
            "tag": "Argo优选",
            "outbounds": node_tags,
            "url": probe,
            "interval": "5m",
            "tolerance": 100
        }));
    } else {
        let mut sel = node_tags.to_vec();
        sel.push("自动选择".into());
        sel.push("DIRECT".into());
        outbounds.push(json!({
            "type": "selector",
            "tag": "代理",
            "outbounds": sel,
            "default": "自动选择"
        }));
        outbounds.push(json!({
            "type": "urltest",
            "tag": "自动选择",
            "outbounds": node_tags,
            "url": probe,
            "interval": "5m",
            "tolerance": 50
        }));
    }
    outbounds.extend(node_outbounds.iter().cloned());
    outbounds.push(json!({ "type": "direct", "tag": "DIRECT" }));
    doc["outbounds"] = Value::Array(outbounds);

    if argo_mode {
        let mut prefix_rules = vec![json!({ "protocol": "dns", "action": "hijack-dns" })];
        if let Some((tunnel, sub)) = argo_domains {
            prefix_rules.push(json!({ "domain": [sub, tunnel], "outbound": "DIRECT" }));
        }
        prefix_rules.push(json!({ "network": "udp", "action": "reject" }));
        if let Some(rules) = doc["route"]["rules"].as_array() {
            for r in rules {
                if r.get("protocol").and_then(Value::as_str) == Some("dns") {
                    continue;
                }
                prefix_rules.push(r.clone());
            }
        }
        doc["route"]["rules"] = Value::Array(prefix_rules);
    }

    serde_json::to_string_pretty(&doc).map_err(|e| CompileError::Format(e.to_string()))
}

pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ArgoWsInbound, EngineKind, WsProtocol};

    fn sample_config() -> Config {
        let mut cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        cfg.argo.enabled = true;
        cfg.argo.tunnel_domain = "argo.example.com".into();
        cfg.argo.sub_domain = "sub.example.com".into();
        cfg.argo.ws_inbounds = vec![ArgoWsInbound {
            id: "ws-vless".into(),
            name: "Argo VLESS".into(),
            protocol: WsProtocol::Vless,
            engine: EngineKind::Xray,
            listen: "127.0.0.1".into(),
            port: 23382,
            tls: false,
        }];
        cfg
    }

    #[test]
    fn compiles_dual_core_servers_with_ssrf_guard_and_sni_demux() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = sample_config();
        let sec = crate::test_secrets::fixture(&cfg);
        let compiled = compile_all(dir.path(), &cfg, &sec).unwrap();

        let xray: Value = serde_json::from_str(compiled.xray_server.as_deref().unwrap()).unwrap();
        assert_eq!(xray["inbounds"][0]["tag"], "sni-demux-443");
        assert_eq!(xray["routing"]["rules"][0]["outboundTag"], "to-vsm-https");
        assert_eq!(
            xray["routing"]["rules"][1]["outboundTag"],
            "to-reality-local"
        );
        assert_eq!(xray["routing"]["rules"][2]["outboundTag"], "block");

        let sb: Value = serde_json::from_str(compiled.singbox_server.as_deref().unwrap()).unwrap();
        assert_eq!(sb["inbounds"].as_array().unwrap().len(), 3);
        assert_eq!(
            sb["inbounds"][0]["padding_scheme"]
                .as_array()
                .unwrap()
                .len(),
            9
        );
        assert_eq!(sb["inbounds"][1]["obfs"]["type"], "salamander");
        assert_eq!(sb["inbounds"][2]["congestion_control"], "bbr");
    }

    #[test]
    fn compiles_four_clean_subscriptions_with_fakeip_guard_and_singbox_filtering() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = sample_config();
        let sec = crate::test_secrets::fixture(&cfg);
        let compiled = compile_all(dir.path(), &cfg, &sec).unwrap();

        let clash: serde_yaml::Value = serde_yaml::from_str(&compiled.clients.clash_yaml).unwrap();
        assert_eq!(clash["proxies"].as_sequence().unwrap().len(), 5);
        assert_eq!(
            clash["rules"].as_sequence().unwrap()[0].as_str(),
            Some("IP-CIDR,198.18.0.0/15,REJECT,no-resolve")
        );
        assert_eq!(clash["tun"]["auto-detect-interface"].as_bool(), Some(false));

        let sb: Value =
            serde_json::from_str(compiled.clients.sing_box_json.as_deref().unwrap()).unwrap();
        let outbounds = sb["outbounds"].as_array().unwrap();
        // selector + urltest + 3 compatible nodes (AnyTLS, Hy2, TUIC) + DIRECT
        assert_eq!(outbounds.len(), 6);
        assert_eq!(compiled.clients.singbox_node_names.len(), 3);
        assert_eq!(compiled.clients.uri_txt.lines().count(), 5);
    }

    #[test]
    fn argo_preferred_ips_expand_dynamically_with_udp_reject() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = sample_config();
        let sec = crate::test_secrets::fixture(&cfg);
        std::fs::create_dir_all(dir.path().join("etc/vps-sub-meter")).unwrap();
        std::fs::write(
            dir.path().join("etc/vps-sub-meter/argo-ips.txt"),
            "# comment\n1.0.0.1\n127.0.0.1 # ignored private\n1.1.1.1\n",
        )
        .unwrap();

        let argo = compile_argo_suite(dir.path(), &cfg, &sec).unwrap();
        let clash: serde_yaml::Value = serde_yaml::from_str(&argo.clash_yaml).unwrap();
        // 2 preferred IPs + 1 fallback domain = 3 nodes
        assert_eq!(clash["proxies"].as_sequence().unwrap().len(), 3);
        assert_eq!(
            clash["rules"].as_sequence().unwrap()[0].as_str(),
            Some("NETWORK,udp,REJECT")
        );
    }
}
