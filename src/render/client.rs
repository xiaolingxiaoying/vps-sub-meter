//! 4-format client subscription compilation (direct & Cloudflare Argo preferred-IP).

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use crate::config::{Config, WsProtocol, format_url_host, pct};
use crate::secrets::Secrets;
use crate::store;

use super::{ClientBundle, CompileError};

const BUNDLED_MIHOMO_TEMPLATE: &str = include_str!("../../templates/mihomo.yaml");
const BUNDLED_SINGBOX_TEMPLATE: &str = include_str!("../../templates/sing-box.json");

pub fn load_mihomo_template(root: &Path) -> Result<serde_yaml::Value, CompileError> {
    let custom = root.join("etc/vps-sub-meter/templates/mihomo.yaml");
    let text = match store::read_to_string_no_follow(&custom) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BUNDLED_MIHOMO_TEMPLATE.to_owned(),
        Err(e) => return Err(e.into()),
    };
    serde_yaml::from_str(&text).map_err(|e| CompileError::Format(format!("mihomo.yaml: {e}")))
}

pub fn load_singbox_template(root: &Path) -> Result<Value, CompileError> {
    let custom = root.join("etc/vps-sub-meter/templates/sing-box.json");
    let text = match store::read_to_string_no_follow(&custom) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BUNDLED_SINGBOX_TEMPLATE.to_owned(),
        Err(e) => return Err(e.into()),
    };
    serde_json::from_str(&text).map_err(|e| CompileError::Format(format!("sing-box.json: {e}")))
}

pub fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_private()
        && !ip.is_loopback()
        && !ip.is_link_local()
        && !ip.is_broadcast()
        && !ip.is_documentation()
        && !ip.is_multicast()
        && !ip.is_unspecified()
        && ip.octets()[0] != 0
        && !(ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        && ip.octets()[0] < 240
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
    let Ok(text) = store::read_to_string_no_follow(&path) else {
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

    let clash_yaml =
        render_clash_from_template(root, config, &clash_proxies, &all_names, &tcp_names, None)?;
    let sing_box_json = (!singbox_outbounds.is_empty())
        .then(|| {
            render_singbox_from_template(root, config, &singbox_outbounds, &singbox_names, None)
        })
        .transpose()?;
    finalize_bundle(uris, clash_yaml, sing_box_json, all_names, singbox_names)
}

pub fn compile_argo_clients(
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
            let (clash, sb, uri) = render_argo_ws_node(
                ws.protocol,
                &label,
                server_addr,
                tunnel,
                &sec.uuid,
                &sec.path,
            );
            clash_proxies.push(clash);
            singbox_outbounds.push(sb);
            uris.push(uri);
        }
    }

    let argo_domains = Some((tunnel, sub_domain));
    let clash_yaml =
        render_clash_from_template(root, config, &clash_proxies, &all_names, &[], argo_domains)?;
    let sing_box_json = (!singbox_outbounds.is_empty())
        .then(|| {
            render_singbox_from_template(root, config, &singbox_outbounds, &all_names, argo_domains)
        })
        .transpose()?;
    finalize_bundle(
        uris,
        clash_yaml,
        sing_box_json,
        all_names.clone(),
        all_names,
    )
}

fn render_argo_ws_node(
    protocol: WsProtocol,
    label: &str,
    server: &str,
    tunnel: &str,
    uuid: &str,
    path: &str,
) -> (Value, Value, String) {
    let proto = match protocol {
        WsProtocol::Vless => "vless",
        WsProtocol::Vmess => "vmess",
    };
    let mut clash = json!({
        "name": label,
        "type": proto,
        "server": server,
        "port": 443,
        "uuid": uuid,
        "network": "ws",
        "udp": false,
        "tls": true,
        "servername": tunnel,
        "client-fingerprint": "chrome",
        "skip-cert-verify": false,
        "ws-opts": {
            "path": path,
            "headers": { "Host": tunnel }
        }
    });
    let mut sb = json!({
        "type": proto,
        "tag": label,
        "server": server,
        "server_port": 443,
        "uuid": uuid,
        "tls": {
            "enabled": true,
            "server_name": tunnel,
            "utls": { "enabled": true, "fingerprint": "chrome" }
        },
        "transport": {
            "type": "ws",
            "path": path,
            "headers": { "Host": tunnel }
        }
    });
    let uri = match protocol {
        WsProtocol::Vless => format!(
            "vless://{uuid}@{server}:443?encryption=none&security=tls&sni={}&fp=chrome&type=ws&host={}&path={}#{}",
            pct(tunnel),
            pct(tunnel),
            pct(path),
            pct(label)
        ),
        WsProtocol::Vmess => {
            clash["alterId"] = json!(0);
            clash["cipher"] = json!("auto");
            sb["security"] = json!("auto");
            sb["alter_id"] = json!(0);
            let payload = json!({
                "v": "2",
                "ps": label,
                "add": server,
                "port": "443",
                "id": uuid,
                "aid": "0",
                "scy": "auto",
                "net": "ws",
                "type": "none",
                "host": tunnel,
                "path": path,
                "tls": "tls",
                "sni": tunnel
            });
            format!(
                "vmess://{}",
                STANDARD.encode(serde_json::to_vec(&payload).unwrap())
            )
        }
    };
    (clash, sb, uri)
}

fn finalize_bundle(
    uris: Vec<String>,
    clash_yaml: String,
    sing_box_json: Option<String>,
    direct_node_names: Vec<String>,
    singbox_node_names: Vec<String>,
) -> Result<ClientBundle, CompileError> {
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
        direct_node_names,
        singbox_node_names,
    })
}

fn render_clash_from_template(
    root: &Path,
    config: &Config,
    proxies: &[Value],
    all_names: &[String],
    tcp_names: &[String],
    argo_domains: Option<(&str, &str)>,
) -> Result<String, CompileError> {
    let mut doc = load_mihomo_template(root)?;
    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| CompileError::Format("mihomo.yaml 必须是 mapping".into()))?;

    let probe = &config.latency_probe_url;
    let urltest = |name: &str, tolerance: u32, members: &[String]| {
        json!({
            "name": name,
            "type": "url-test",
            "url": probe,
            "interval": 300,
            "tolerance": tolerance,
            "proxies": members
        })
    };

    let mut groups = Vec::new();
    let mut selector_members = Vec::new();
    if argo_domains.is_some() {
        selector_members.push("Argo优选".to_owned());
        selector_members.extend(all_names.iter().cloned());
    } else {
        selector_members.extend(all_names.iter().cloned());
        if !tcp_names.is_empty() {
            selector_members.push("TCP优选".into());
        }
        selector_members.push("自动选择".into());
    }
    selector_members.push("DIRECT".into());
    groups.push(json!({
        "name": "代理",
        "type": "select",
        "url": probe,
        "interval": 300,
        "proxies": selector_members
    }));
    if argo_domains.is_some() {
        groups.push(urltest("Argo优选", 100, all_names));
    } else {
        if !tcp_names.is_empty() {
            groups.push(urltest("TCP优选", 50, tcp_names));
        }
        groups.push(urltest("自动选择", 50, all_names));
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

    if let Some((tunnel, sub)) = argo_domains {
        push_rule("NETWORK,udp,REJECT");
        push_rule(&format!("DOMAIN,{sub},DIRECT"));
        push_rule(&format!("DOMAIN,{tunnel},DIRECT"));
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

    if let Some((tunnel, sub)) = argo_domains
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
    argo_domains: Option<(&str, &str)>,
) -> Result<String, CompileError> {
    let mut doc = load_singbox_template(root)?;
    let (auto_tag, tolerance) = if argo_domains.is_some() {
        ("Argo优选", 100)
    } else {
        ("自动选择", 50)
    };
    let mut sel = if argo_domains.is_some() {
        let mut s = vec![auto_tag.to_owned()];
        s.extend(node_tags.iter().cloned());
        s
    } else {
        let mut s = node_tags.to_vec();
        s.push(auto_tag.to_owned());
        s
    };
    sel.push("DIRECT".into());

    let mut outbounds = vec![
        json!({
            "type": "selector",
            "tag": "代理",
            "outbounds": sel,
            "default": auto_tag
        }),
        json!({
            "type": "urltest",
            "tag": auto_tag,
            "outbounds": node_tags,
            "url": &config.latency_probe_url,
            "interval": "5m",
            "tolerance": tolerance
        }),
    ];
    outbounds.extend(node_outbounds.iter().cloned());
    outbounds.push(json!({ "type": "direct", "tag": "DIRECT" }));
    doc["outbounds"] = Value::Array(outbounds);

    if let Some((tunnel, sub)) = argo_domains {
        let mut prefix_rules = vec![
            json!({ "protocol": "dns", "action": "hijack-dns" }),
            json!({ "domain": [sub, tunnel], "outbound": "DIRECT" }),
            json!({ "network": "udp", "action": "reject" }),
        ];
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

#[cfg(test)]
mod tests {
    use super::super::compile_all;
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
        assert!(compiled.argo_clients.is_some());
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

        let argo = compile_argo_clients(dir.path(), &cfg, &sec).unwrap();
        let clash: serde_yaml::Value = serde_yaml::from_str(&argo.clash_yaml).unwrap();
        // 2 preferred IPs + 1 fallback domain = 3 nodes
        assert_eq!(clash["proxies"].as_sequence().unwrap().len(), 3);
        assert_eq!(
            clash["rules"].as_sequence().unwrap()[0].as_str(),
            Some("NETWORK,udp,REJECT")
        );
    }
}
