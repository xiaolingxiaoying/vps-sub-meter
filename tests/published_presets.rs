use base64::Engine;
use sbctl::config::{DeploymentConfig, ManagedProtocol, ProtocolPorts, SubscriptionMode};
use sbctl::core::presets::{Node, Preset, Reality};
use sbctl::subscription::{SubscriptionRoute, generated_artifacts, route_url};

#[test]
fn published_presets_join_existing_routes_and_groups_without_changing_credentials() {
    let root = tempfile::tempdir().unwrap();
    let config = DeploymentConfig::new_with_ports(
        SubscriptionMode::IpFallback,
        "127.0.0.1".into(),
        None,
        Some(29000),
        "eth0".into(),
        vec![ManagedProtocol::VlessReality],
        Some("example.com".into()),
        ProtocolPorts {
            vless_reality: Some(30000),
            ..Default::default()
        },
    )
    .unwrap();
    let before = route_url(&config, SubscriptionRoute::Index).unwrap();
    let encode = |key: [u8; 32]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);
    let presets = [
        Preset::VlessXhttpReality,
        Preset::VlessXhttpTls,
        Preset::VlessWsTls,
        Preset::TrojanTcpTls,
    ];
    let nodes: Vec<Node> = presets
        .into_iter()
        .enumerate()
        .map(|(i, preset)| Node {
            name: format!("extra-{i}"),
            preset,
            server: "example.com".into(),
            port: 31000 + i as u16,
            server_name: "example.com".into(),
            credential: if preset.is_trojan() {
                "private-fixture-password".into()
            } else {
                "b831381d-6324-4d53-ad4f-8cda48b30811".into()
            },
            path: (!preset.is_trojan()).then(|| "/test".into()),
            reality: preset.is_reality().then(|| Reality {
                private_key: encode([1; 32]),
                public_key: encode(x25519_dalek::x25519(
                    [1; 32],
                    x25519_dalek::X25519_BASEPOINT_BYTES,
                )),
                short_id: "0123456789abcdef".into(),
                target: "example.com:443".into(),
            }),
        })
        .collect();
    let source = root.path().join("etc/sbctl/protocol-presets.json");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, serde_json::to_vec(&nodes).unwrap()).unwrap();
    let artifacts = generated_artifacts(&config, root.path()).unwrap();
    let get = |name: &str| {
        artifacts
            .iter()
            .find(|(n, _)| n == name)
            .unwrap()
            .1
            .as_str()
    };
    assert_eq!(get("subscription-uri.txt").lines().count(), 5);
    let sing: serde_json::Value =
        serde_json::from_str(get("subscription-sing-box-full.json")).unwrap();
    let outbounds = sing["outbounds"].as_array().unwrap();
    assert!(outbounds.iter().any(|n| n["tag"] == "extra-2"));
    assert!(
        !outbounds
            .iter()
            .any(|n| n["tag"] == "extra-0" || n["tag"] == "extra-1")
    );
    assert!(
        outbounds
            .iter()
            .filter_map(|n| n["outbounds"].as_array())
            .any(|g| g.iter().any(|n| n == "extra-2"))
    );
    let clash: serde_json::Value = serde_yaml::from_str(get("subscription-clash.yaml")).unwrap();
    assert_eq!(clash["proxies"].as_array().unwrap().len(), 4);
    assert!(
        clash["proxies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["network"] == "xhttp")
    );
    let legacy: serde_json::Value =
        serde_yaml::from_str(get("subscription-clash-1.18.yaml")).unwrap();
    assert_eq!(legacy["proxies"].as_array().unwrap().len(), 3);
    let shadow = base64::engine::general_purpose::STANDARD
        .decode(get("subscription-shadowrocket.txt"))
        .unwrap();
    assert_eq!(String::from_utf8(shadow).unwrap().lines().count(), 3);
    let b64 = base64::engine::general_purpose::STANDARD
        .decode(get("subscription-base64-uri.txt"))
        .unwrap();
    assert_eq!(String::from_utf8(b64).unwrap(), get("subscription-uri.txt"));
    assert!(!get("subscription-preset-nodes.json").contains("private_key"));
    assert_eq!(
        route_url(&config, SubscriptionRoute::Index).unwrap(),
        before
    );
    std::fs::write(source, b"invalid-private-fixture").unwrap();
    let error = generated_artifacts(&config, root.path()).unwrap_err();
    assert!(!error.to_string().contains("invalid-private-fixture"));
}
