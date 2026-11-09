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
