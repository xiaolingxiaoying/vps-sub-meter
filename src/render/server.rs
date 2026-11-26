//! Xray & sing-box server configuration rendering.

use serde_json::{Value, json};

use crate::config::{ArgoWsInbound, Config, EngineKind, WsProtocol};
use crate::secrets::Secrets;

use super::{CertPaths, CompileError};

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

pub fn compile_xray_server(
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

    let doc = json!({
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

    let doc = json!({
        "log": { "level": "warn" },
        "dns": { "strategy": if config.ipv4_only { "ipv4_only" } else { "prefer_ipv4" } },
        "inbounds": inbounds,
        "outbounds": [{ "type": "direct", "tag": "direct" }],
        "route": { "rules": rules }
    });
    Ok(Some(serde_json::to_string_pretty(&doc).unwrap()))
}

#[cfg(test)]
mod tests {
    use super::super::compile_all;
    use crate::config::{ArgoWsInbound, Config, EngineKind, WsProtocol};
    use serde_json::Value;

    #[test]
    fn compiles_dual_core_servers_with_ssrf_guard_and_sni_demux() {
        let dir = tempfile::tempdir().unwrap();
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
}
