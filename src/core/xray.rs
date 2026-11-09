//! Xray server renderer. Does not write files, rotate credentials or activate
//! services; publication belongs to the deployment transaction.
use crate::canonical::CanonicalNode;
use serde_json::{Value, json};

/// Fixed research baseline, not an assertion of production interoperability.
pub const BASELINE_VERSION: &str = "26.10.31";

pub struct CertificatePaths<'a> {
    pub certificate: &'a str,
    pub private_key: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("Xray server requires at least one enabled node")]
    Empty,
    #[error("protocol {0} has no validated Xray server adapter")]
    Unsupported(&'static str),
    #[error("VMess WebSocket TLS requires certificate and private-key paths")]
    CertificateRequired,
}

pub fn render_server(
    nodes: &[CanonicalNode],
    certificate: Option<CertificatePaths<'_>>,
    ipv4_only: bool,
) -> Result<Value, RenderError> {
    if nodes.is_empty() {
        return Err(RenderError::Empty);
    }
    let mut inbounds = Vec::new();
    for node in nodes {
        let (port, protocol, settings, mut stream) = match node {
            CanonicalNode::VlessReality {
                port,
                uuid,
                private_key,
                short_id,
                decoy_sni,
                ..
            } => (
                *port,
                "vless",
                json!({"clients": [{"id": uuid, "flow": "xtls-rprx-vision"}], "decryption": "none"}),
                json!({"network": "raw", "security": "reality", "realitySettings": {
                    "target": format!("{decoy_sni}:443"), "serverNames": [decoy_sni],
                    "privateKey": private_key, "shortIds": [short_id]
                }}),
            ),
            CanonicalNode::VmessWebsocket {
                port, uuid, path, ..
            } => {
                let paths = certificate
                    .as_ref()
                    .filter(|paths| !paths.certificate.is_empty() && !paths.private_key.is_empty())
                    .ok_or(RenderError::CertificateRequired)?;
                (
                    *port,
                    "vmess",
                    json!({"clients": [{"id": uuid}]}),
                    json!({"network": "ws", "security": "tls", "wsSettings": {"path": path},
                        "tlsSettings": {"certificates": [{"certificateFile": paths.certificate, "keyFile": paths.private_key}]}}),
                )
            }
            CanonicalNode::Hysteria2 { .. } => return Err(RenderError::Unsupported("hysteria2")),
            CanonicalNode::Tuic { .. } => return Err(RenderError::Unsupported("tuic")),
            CanonicalNode::Anytls { .. } => return Err(RenderError::Unsupported("anytls")),
        };
        if ipv4_only {
            stream["sockopt"] = json!({"domainStrategy": "UseIPv4"});
        }
        inbounds.push(json!({"tag": node.tag(), "listen": "::", "port": port,
            "protocol": protocol, "settings": settings, "streamSettings": stream}));
    }
    let mut direct = json!({"tag": "direct", "protocol": "freedom", "settings": {}});
    if ipv4_only {
        direct["settings"]["domainStrategy"] = json!("ForceIPv4");
    }
    Ok(json!({"log": {"loglevel": "warning"}, "inbounds": inbounds, "outbounds": [direct]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vmess() -> CanonicalNode {
        CanonicalNode::VmessWebsocket {
            host: "example.com".into(),
            port: 23456,
            tls_server_name: "example.com".into(),
            uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
            path: "/test".into(),
        }
    }

    #[test]
    fn refuses_missing_tls_and_unvalidated_protocols_without_returning_secrets() {
        assert!(matches!(
            render_server(&[vmess()], None, false),
            Err(RenderError::CertificateRequired)
        ));
        let node = CanonicalNode::Anytls {
            host: "example.com".into(),
            port: 12345,
            tls_server_name: "example.com".into(),
            password: "PRIVATE-PASSWORD".into(),
        };
        let error = render_server(&[node], None, false).unwrap_err();
        assert!(matches!(error, RenderError::Unsupported("anytls")));
        assert!(!error.to_string().contains("PRIVATE-PASSWORD"));
    }

    #[test]
    fn vmess_preserves_credentials_path_and_port() {
        let node = vmess();
        let original = node.clone();
        let document = render_server(
            &[node],
            Some(CertificatePaths {
                certificate: "/cert.pem",
                private_key: "/key.pem",
            }),
            true,
        )
        .unwrap();
        let inbound = &document["inbounds"][0];
        assert_eq!(inbound["port"], 23456);
        assert_eq!(
            inbound["settings"]["clients"][0]["id"],
            "b831381d-6324-4d53-ad4f-8cda48b30811"
        );
        assert_eq!(inbound["streamSettings"]["wsSettings"]["path"], "/test");
        assert_eq!(
            document["outbounds"][0]["settings"]["domainStrategy"],
            "ForceIPv4"
        );
        assert_eq!(original, vmess());
    }
}
