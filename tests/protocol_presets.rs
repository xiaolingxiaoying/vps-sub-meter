use sbctl::core::presets::{Node, Preset, validate_nodes};

fn ws() -> Node {
    Node {
        name: "测试节点".into(),
        preset: Preset::VlessWsTls,
        server: "2001:db8::1".into(),
        port: 23456,
        server_name: "example.com".into(),
        credential: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
        path: Some("/proxy?test=1".into()),
        reality: None,
    }
}

#[test]
fn vless_ws_has_matching_client_fields_and_encoded_uri() {
    let node = ws();
    let sing = node.sing_box_outbound().unwrap();
    let mihomo = node.mihomo_proxy(false).unwrap();
    let xray = node.xray_outbound().unwrap();
    assert_eq!(sing["uuid"], mihomo["uuid"]);
    assert_eq!(sing["uuid"], xray["settings"]["vnext"][0]["users"][0]["id"]);
    assert_eq!(sing["transport"]["path"], mihomo["ws-opts"]["path"]);
    let uri = url::Url::parse(&node.share_uri().unwrap()).unwrap();
    assert_eq!(uri.host_str(), Some("[2001:db8::1]"));
    assert!(
        uri.query_pairs()
            .any(|(key, value)| key == "path" && value == "/proxy?test=1")
    );
    assert!(sing["tls"]["insecure"].is_null());
}

#[test]
fn incompatible_profiles_and_duplicate_listener_sets_fail() {
    let mut node = ws();
    node.preset = Preset::VlessXhttpTls;
    assert!(node.sing_box_outbound().is_err());
    assert!(node.mihomo_proxy(false).is_err());
    assert_eq!(node.mihomo_proxy(true).unwrap()["network"], "xhttp");
    assert!(validate_nodes(&[ws(), ws()]).is_err());
    node.reality = None;
    node.preset = Preset::VlessXhttpReality;
    assert!(node.validate().is_err());
}

#[test]
fn arbitrary_transport_fields_and_secret_leaks_are_rejected() {
    let mut value = serde_json::to_value(ws()).unwrap();
    value["flow"] = serde_json::json!("xtls-rprx-vision");
    assert!(serde_json::from_value::<Node>(value).is_err());
    let mut node = ws();
    node.credential = "PRIVATE-CREDENTIAL".into();
    assert!(
        !node
            .validate()
            .unwrap_err()
            .to_string()
            .contains("PRIVATE-CREDENTIAL")
    );
}

#[test]
fn current_xray_reality_baseline_is_not_exported_as_mihomo_compatible() {
    use base64::Engine;
    use sbctl::core::presets::Reality;
    let encode = |bytes: [u8; 32]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let mut node = ws();
    node.preset = Preset::VlessXhttpReality;
    node.reality = Some(Reality {
        private_key: encode([1; 32]),
        public_key: encode(x25519_dalek::x25519(
            [1; 32],
            x25519_dalek::X25519_BASEPOINT_BYTES,
        )),
        short_id: "0123456789abcdef".into(),
        target: "example.com:443".into(),
    });
    assert!(node.xray_outbound().is_ok());
    assert!(node.mihomo_proxy(true).is_err());
    assert!(node.sing_box_outbound().is_err());
}
