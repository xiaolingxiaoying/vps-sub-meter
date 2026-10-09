use base64::Engine;

use super::client_skip_cert_verify;
use crate::canonical::CanonicalNode;
use crate::config::DeploymentConfig;
use crate::subscription::artifacts::SubscriptionError;
use crate::subscription::base64_uri;

/// The VMess config is base64-encoded into a byte-frozen artifact (ADR-0021),
/// so its key order must not depend on how the binary was built: `serde_json`
/// sorts object keys in a root-package build but preserves insertion order as
/// soon as another workspace member unifies `preserve_order` on. Emitting the
/// pairs sorted by hand pins the bytes for both build shapes.
fn vmess_payload(
    tag: &str,
    host: &str,
    port: u16,
    uuid: &str,
    tls_server_name: &str,
    path: &str,
    insecure: u8,
) -> String {
    let mut fields: Vec<(&str, String)> = vec![
        ("v", "2".to_owned()),
        ("ps", tag.to_owned()),
        ("add", host.to_owned()),
        ("port", port.to_string()),
        ("id", uuid.to_owned()),
        ("aid", "0".to_owned()),
        ("scy", "auto".to_owned()),
        ("net", "ws".to_owned()),
        ("type", "none".to_owned()),
        ("host", tls_server_name.to_owned()),
        ("path", path.to_owned()),
        ("tls", "tls".to_owned()),
        ("sni", tls_server_name.to_owned()),
    ];
    // v2rayN reads `insecure`; Shadowrocket-compatible importers also use
    // `allowInsecure`. Domain-mode links keep their historical verified form.
    if insecure != 0 {
        fields.push(("insecure", "1".to_owned()));
        fields.push(("allowInsecure", "1".to_owned()));
    }
    fields.sort_unstable_by_key(|(key, _)| *key);
    let body = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}:{}",
                serde_json::Value::String((*key).to_owned()),
                serde_json::Value::String(value.clone())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// The `vmess://` line for one node, shared by the plain and Shadowrocket URI
/// forms so the frozen payload can never drift between them.
fn vmess_uri(
    tag: &str,
    host: &str,
    port: u16,
    uuid: &str,
    tls_server_name: &str,
    path: &str,
    insecure: u8,
) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(vmess_payload(
        tag,
        host,
        port,
        uuid,
        tls_server_name,
        path,
        insecure,
    ));
    format!("vmess://{encoded}\n")
}

/// The Shadowrocket-adapted Base64 URI list (research:
/// `docs/research/sing-box-client-version-differences.md` §6). Differences
/// from the plain `uri` rendering: passwords and SNI values are always
/// percent-encoded (Shadowrocket 2.2.44 fixed URI password decoding, which
/// implies special characters must arrive encoded), TUIC carries
/// `udp_relay_mode`, and AnyTLS follows the official anytls-go scheme with
/// the path slash and without the non-standard `security` parameter.
pub(crate) fn shadowrocket(
    config: &DeploymentConfig,
    nodes: &[CanonicalNode],
) -> Result<String, SubscriptionError> {
    let insecure = if client_skip_cert_verify(config) {
        1
    } else {
        0
    };
    let aliases = cert_verify_aliases(insecure, false);
    let tuic_aliases = cert_verify_aliases(insecure, true);
    let mut uris = String::new();
    // A URI authority needs IPv6 hosts bracketed; the server and Clash
    // renderers deliberately keep the bare address.
    let nodes = nodes
        .iter()
        .map(CanonicalNode::with_bracketed_host)
        .collect::<Vec<_>>();
    for node in &nodes {
        match &node {
            CanonicalNode::VlessReality {
                host,
                port,
                uuid,
                public_key,
                short_id,
                decoy_sni,
                ..
            } => uris.push_str(&format!("vless://{uuid}@{host}:{port}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={}&fp=chrome&pbk={public_key}&sid={short_id}&type=tcp#{}\n", percent_encode(decoy_sni), node.tag())),
            CanonicalNode::VmessWebsocket {
                host,
                port,
                tls_server_name,
                uuid,
                path,
            } => {
                uris.push_str(&vmess_uri(
                    node.tag(),
                    host,
                    *port,
                    uuid,
                    tls_server_name,
                    path,
                    insecure,
                ));
            }
            CanonicalNode::Hysteria2 {
                host,
                port,
                tls_server_name,
                password,
            } => uris.push_str(&format!(
                "hysteria2://{}@{}:{}?insecure={}{aliases}&sni={}#{}\n",
                percent_encode(password),
                host,
                port,
                insecure,
                percent_encode(tls_server_name),
                node.tag()
            )),
            CanonicalNode::Tuic {
                host,
                port,
                tls_server_name,
                uuid,
                password,
            } => uris.push_str(&format!(
                "tuic://{}:{}@{}:{}?congestion_control=bbr&udp_relay_mode=native&alpn=h3&insecure={}{tuic_aliases}&sni={}#{}\n",
                percent_encode(uuid),
                percent_encode(password),
                host,
                port,
                insecure,
                percent_encode(tls_server_name),
                node.tag()
            )),
            CanonicalNode::Anytls {
                host,
                port,
                tls_server_name,
                password,
            } => uris.push_str(&format!(
                "anytls://{}@{}:{}/?insecure={}{aliases}&sni={}#{}\n",
                percent_encode(password),
                host,
                port,
                insecure,
                percent_encode(tls_server_name),
                node.tag()
            )),
        }
    }
    Ok(base64_uri(&uris))
}

/// Percent-encodes everything outside the RFC 3986 unreserved set so secrets
/// with special characters survive URI parsing in every client.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// One node's share link, without the trailing newline.
///
/// Split out of [`uri`] so the index page and `sbctl status nodes --uri` can
/// show the same string a subscriber downloads, rather than making a user fetch
/// the credential'd artifact to see their own node parameters. `node` must
/// already carry a bracketed host if it is IPv6: see [`uri`].
pub(crate) fn node_uri(insecure: u8, node: &CanonicalNode) -> String {
    let aliases = cert_verify_aliases(insecure, false);
    let tuic_aliases = cert_verify_aliases(insecure, true);
    match node {
        CanonicalNode::VlessReality {
            host,
            port,
            uuid,
            public_key,
            short_id,
            decoy_sni,
            ..
        } => format!(
            "vless://{uuid}@{host}:{port}?encryption=none&flow=xtls-rprx-vision&security=reality&sni={decoy_sni}&fp=chrome&pbk={public_key}&sid={short_id}&type=tcp#{}\n",
            node.tag()
        ),
        CanonicalNode::VmessWebsocket {
            host,
            port,
            tls_server_name,
            uuid,
            path,
        } => vmess_uri(
            node.tag(),
            host,
            *port,
            uuid,
            tls_server_name,
            path,
            insecure,
        ),
        CanonicalNode::Hysteria2 {
            host,
            port,
            tls_server_name,
            password,
        } => format!(
            "hysteria2://{password}@{host}:{port}?insecure={insecure}{aliases}&sni={tls_server_name}#{}\n",
            node.tag()
        ),
        CanonicalNode::Tuic {
            host,
            port,
            tls_server_name,
            uuid,
            password,
        } => format!(
            "tuic://{uuid}:{password}@{host}:{port}?congestion_control=bbr&alpn=h3&insecure={insecure}{tuic_aliases}&sni={tls_server_name}#{}\n",
            node.tag()
        ),
        CanonicalNode::Anytls {
            host,
            port,
            tls_server_name,
            password,
        } => format!(
            "anytls://{password}@{host}:{port}?security=tls&insecure={insecure}{aliases}&sni={tls_server_name}#{}\n",
            node.tag()
        ),
    }
}

/// Importers disagree on the query key (sing-box-yg emits these aliases too).
/// Only self-signed mode opts out; domain-mode links keep verification enabled.
fn cert_verify_aliases(insecure: u8, tuic: bool) -> &'static str {
    match (insecure != 0, tuic) {
        (true, true) => "&allowInsecure=1&allow_insecure=1",
        (true, false) => "&allowInsecure=1",
        (false, _) => "",
    }
}

/// Whether the generated client artifacts skip certificate verification.
pub(crate) fn insecure_flag(config: &DeploymentConfig) -> u8 {
    if client_skip_cert_verify(config) {
        1
    } else {
        0
    }
}

pub(crate) fn uri(
    config: &DeploymentConfig,
    nodes: &[CanonicalNode],
) -> Result<String, SubscriptionError> {
    let insecure = insecure_flag(config);
    let mut uris = String::new();
    // A URI authority needs IPv6 hosts bracketed; the server and Clash
    // renderers deliberately keep the bare address.
    let nodes = nodes
        .iter()
        .map(CanonicalNode::with_bracketed_host)
        .collect::<Vec<_>>();
    for node in &nodes {
        uris.push_str(&node_uri(insecure, node));
    }
    Ok(uris)
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{shadowrocket, uri};
    use crate::config::{
        CertificateMode, DeploymentConfig, ManagedProtocol, ProtocolPorts, SubscriptionMode,
    };
    use crate::subscription::render::{clash, sing_box};
    use crate::subscription::{SubscriptionFormat, SubscriptionRoute, route_url};

    #[test]
    fn every_uri_import_preserves_the_certificate_policy_for_each_tls_protocol() {
        for (mode, host) in [
            (SubscriptionMode::IpFallback, "203.0.113.7"),
            (SubscriptionMode::Direct, "sub.example.test"),
            (SubscriptionMode::ExternalProxy, "sub.example.test"),
        ] {
            let mut config = DeploymentConfig::new_with_ports(
                mode.clone(),
                host.into(),
                None,
                if mode == SubscriptionMode::IpFallback {
                    Some(2080)
                } else {
                    None
                },
                "ens3".into(),
                vec![
                    ManagedProtocol::VlessReality,
                    ManagedProtocol::VmessWebsocket,
                    ManagedProtocol::Hysteria2,
                    ManagedProtocol::Tuic,
                    ManagedProtocol::Anytls,
                ],
                Some("www.cloudflare.com".into()),
                ProtocolPorts::default(),
            )
            .expect("deployment is valid");
            for certificate_mode in [CertificateMode::SelfSigned, CertificateMode::Domain] {
                if mode == SubscriptionMode::IpFallback
                    && certificate_mode == CertificateMode::Domain
                {
                    continue;
                }
                config.certificate_mode = certificate_mode;
                config.validate().expect("certificate mode is valid");
                let skip = config.certificate_mode == CertificateMode::SelfSigned;
                let nodes = crate::canonical::nodes(&config);
                let plain = uri(&config, &nodes).expect("URI renders");
                let rocket = String::from_utf8(
                    base64::engine::general_purpose::STANDARD
                        .decode(shadowrocket(&config, &nodes).expect("Shadowrocket renders"))
                        .expect("subscription decodes"),
                )
                .expect("UTF-8");
                for artifact in [&plain, &rocket] {
                    let lines: Vec<_> = artifact.lines().collect();
                    assert_eq!(lines.len(), 5);
                    assert!(
                        !lines[0].contains("insecure"),
                        "Reality retains key authentication"
                    );
                    let payload: serde_json::Value = serde_json::from_slice(
                        &base64::engine::general_purpose::STANDARD
                            .decode(lines[1].strip_prefix("vmess://").expect("VMess URI"))
                            .expect("VMess decodes"),
                    )
                    .expect("VMess JSON");
                    assert_eq!(payload["sni"], config.protocol_server_name());
                    assert_eq!(payload["tls"], "tls");
                    for key in ["insecure", "allowInsecure"] {
                        assert_eq!(payload[key].as_str(), if skip { Some("1") } else { None });
                    }
                    for line in &lines[2..] {
                        let url = url::Url::parse(line).expect("TLS URI parses");
                        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
                        assert_eq!(
                            query.get("sni").map(|v| v.as_ref()),
                            Some(config.protocol_server_name())
                        );
                        assert_eq!(
                            query.get("insecure").map(|v| v.as_ref()),
                            Some(if skip { "1" } else { "0" })
                        );
                        assert_eq!(
                            query.get("allowInsecure").map(|v| v.as_ref()),
                            if skip { Some("1") } else { None }
                        );
                        if url.scheme() == "tuic" {
                            assert_eq!(
                                query.get("allow_insecure").map(|v| v.as_ref()),
                                if skip { Some("1") } else { None }
                            );
                        }
                    }
                }
                let json: serde_json::Value =
                    serde_json::from_str(&sing_box(&config, &nodes).expect("sing-box renders"))
                        .expect("JSON");
                for node in &json["outbounds"].as_array().expect("outbounds")[1..] {
                    assert_eq!(node["tls"]["insecure"], skip);
                }
                let yaml: serde_yaml::Value =
                    serde_yaml::from_str(&clash(&config, &nodes).expect("Clash renders"))
                        .expect("YAML");
                for node in &yaml["proxies"].as_sequence().expect("proxies")[1..] {
                    assert_eq!(node["skip-cert-verify"].as_bool(), Some(skip));
                }
            }
        }
    }

    #[test]
    fn an_ipv6_host_is_bracketed_only_where_a_uri_authority_requires_it() {
        let config = DeploymentConfig::new_with_ports(
            SubscriptionMode::IpFallback,
            "2001:db8::1".into(),
            None,
            Some(2080),
            "ens3".into(),
            vec![ManagedProtocol::Hysteria2],
            Some("www.cloudflare.com".into()),
            ProtocolPorts::default(),
        )
        .expect("an IPv6 no-domain deployment is valid");
        let nodes = crate::canonical::nodes(&config);

        let uri = uri(&config, &nodes).expect("uri artifacts generate");
        assert!(
            uri.contains("@[2001:db8::1]:"),
            "an unbracketed IPv6 authority is not a parseable URI: {uri}"
        );
        let url = route_url(&config, SubscriptionRoute::Format(SubscriptionFormat::Uri))
            .expect("the subscription URL builds");
        assert!(
            url.starts_with("http://[2001:db8::1]:2080/sub/"),
            "the share link must bracket the IPv6 host: {url}"
        );

        let clash = clash(&config, &nodes).expect("clash artifacts generate");
        assert!(
            !clash.contains("[2001:db8::1]"),
            "configuration fields take a bare address; brackets leak into server fields"
        );
        let sing_box = sing_box(&config, &nodes).expect("sing-box artifacts generate");
        assert!(
            !sing_box.contains("[2001:db8::1]"),
            "sing-box `server` must carry the bare address"
        );
    }
}
