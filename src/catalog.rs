//! One plain-language protocol catalog for setup, management and subscriptions.
#[derive(Clone, Copy)]
pub struct Protocol {
    pub key: &'static str,
    pub label: &'static str,
    pub engine: &'static str,
    pub note: &'static str,
    pub udp: bool,
}
pub const PROTOCOLS: &[Protocol] = &[
    Protocol {
        key: "vless-reality",
        label: "VLESS + Reality",
        engine: "sing-box / Xray",
        note: "无需域名；Xray 下仅向已验证客户端提供",
        udp: false,
    },
    Protocol {
        key: "vmess-websocket",
        label: "VMess + WebSocket + TLS",
        engine: "sing-box / Xray",
        note: "通用客户端；自动准备证书",
        udp: false,
    },
    Protocol {
        key: "hysteria2",
        label: "Hysteria2",
        engine: "sing-box",
        note: "UDP；支持端口跳跃",
        udp: true,
    },
    Protocol {
        key: "tuic",
        label: "TUIC",
        engine: "sing-box",
        note: "UDP；需要客户端支持 TUIC v5",
        udp: true,
    },
    Protocol {
        key: "anytls",
        label: "AnyTLS",
        engine: "sing-box",
        note: "需要 sing-box 1.12 或更新客户端",
        udp: false,
    },
    Protocol {
        key: "vless-xhttp-reality",
        label: "VLESS + XHTTP + Reality",
        engine: "Xray",
        note: "无需域名；按客户端兼容性提供",
        udp: false,
    },
    Protocol {
        key: "vless-xhttp-tls",
        label: "VLESS + XHTTP + TLS",
        engine: "Xray",
        note: "适合支持 XHTTP 的客户端",
        udp: false,
    },
    Protocol {
        key: "vless-ws-tls",
        label: "VLESS + WebSocket + TLS",
        engine: "sing-box / Xray",
        note: "通用客户端；自动准备证书",
        udp: false,
    },
    Protocol {
        key: "trojan-tcp-tls",
        label: "Trojan + TLS",
        engine: "sing-box / Xray",
        note: "通用客户端；自动准备证书",
        udp: false,
    },
];
pub fn resolve(value: &str) -> Option<&'static Protocol> {
    PROTOCOLS.iter().find(|p| {
        p.key.eq_ignore_ascii_case(value.trim()) || p.label.eq_ignore_ascii_case(value.trim())
    })
}
pub fn label(key: &str) -> &str {
    resolve(key).map_or(key, |p| p.label)
}
pub fn udp(key: &str) -> bool {
    resolve(key).is_some_and(|p| p.udp)
}
pub fn node_port(port: u16) -> bool {
    port == 443 || port >= 10000
}

pub fn unique_label(key: &str, existing: impl IntoIterator<Item = String>) -> String {
    let existing: std::collections::BTreeSet<_> = existing.into_iter().collect();
    let base = label(key);
    if !existing.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|s| !existing.contains(s))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readable_names_keep_protocol_identity_and_duplicate_numbering() {
        assert_eq!(
            resolve("VLESS + WebSocket + TLS").unwrap().key,
            "vless-ws-tls"
        );
        assert_eq!(
            unique_label("hysteria2", vec!["Hysteria2".into()]),
            "Hysteria2 2"
        );
        assert!(node_port(443));
        assert!(!node_port(80));
    }
}
