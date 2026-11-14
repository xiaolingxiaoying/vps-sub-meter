use sbctl::{
    config::{DeploymentConfig, DeploymentStore, ManagedProtocol, SubscriptionMode},
    core::CoreKind,
    nodes::{self, Mode, Strategy},
    subscription::{self, SubscriptionFormat as Format},
};
use serde_json::Value;

fn fixture() -> (tempfile::TempDir, DeploymentConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = DeploymentConfig::new(
        SubscriptionMode::IpFallback,
        "203.0.113.8".into(),
        None,
        Some(2080),
        "ens3".into(),
        vec![
            ManagedProtocol::VlessReality,
            ManagedProtocol::VmessWebsocket,
        ],
        Some("www.cloudflare.com".into()),
    )
    .unwrap();
    let store = DeploymentStore::new(root.path());
    store.initialize(&config).unwrap();
    subscription::regenerate(&store, &config, None, false).unwrap();
    (root, config)
}
#[test]
fn migration_preserves_share_bytes_and_credentials_and_is_idempotent() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let old = subscription::read_authorized(
        &store,
        &config,
        &config.subscription_credential,
        Format::Uri,
    )
    .unwrap();
    let registry = nodes::load_or_import(root.path(), &config).unwrap();
    sbctl::deployment::apply(&store, &config, &registry, registry.clone()).unwrap();
    let first = nodes::read_optional(root.path()).unwrap().unwrap();
    assert_eq!(
        subscription::read_authorized(
            &store,
            &config,
            &config.subscription_credential,
            Format::Uri
        )
        .unwrap(),
        old
    );
    assert!(store.load().unwrap() == config);
    sbctl::deployment::apply(&store, &config, &first, first.clone()).unwrap();
    let next = nodes::read_optional(root.path()).unwrap().unwrap();
    assert!(first.nodes == next.nodes);
    assert_eq!(next.revision, 2);
}
#[test]
fn duplicate_protocols_groups_and_stable_tags_share_one_source() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let before = nodes::load_or_import(root.path(), &config).unwrap();
    let mut after = before.clone();
    let mut copied = after.nodes[0].clone();
    copied.id = nodes::new_id();
    copied.tag = format!("node-{}", copied.id);
    copied.name = "备用 Reality".into();
    copied.renew_secrets().unwrap();
    copied
        .patch(
            "port",
            serde_json::json!(nodes::free_port(&after, &config).unwrap()),
        )
        .unwrap();
    let tag = copied.tag.clone();
    let id = copied.id.clone();
    assert!(copied.data != after.nodes[0].data);
    after.nodes.push(copied);
    after.groups.push(nodes::Group {
        id: "backup".into(),
        name: "备用".into(),
        members: vec![id.clone()],
        strategy: Strategy::Urltest,
    });
    sbctl::deployment::apply(&store, &config, &before, after).unwrap();
    let generation = sbctl::deployment::current(root.path()).unwrap().unwrap();
    let text = sbctl::deployment::read_generation(
        root.path(),
        &generation,
        "group-backup--subscription-sing-box-full.json",
    )
    .unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        doc["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["type"] == "vless")
            .count(),
        1
    );
    assert_eq!(doc["outbounds"][0]["tag"], tag);
    let registry = nodes::read_optional(root.path()).unwrap().unwrap();
    let mut rename = registry.clone();
    rename.node_mut(&id).unwrap().name = "改名".into();
    let plan = sbctl::deployment::plan(root.path(), &config, &registry, &rename).unwrap();
    assert!(plan.restart.is_empty());
    sbctl::deployment::apply(&store, &config, &registry, rename).unwrap();
    let registry = nodes::read_optional(root.path()).unwrap().unwrap();
    assert_eq!(registry.nodes.last().unwrap().tag, tag);
}
#[test]
fn xray_only_subscription_works_without_server_sing_box_and_filters_xhttp() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let before = nodes::load_or_import(root.path(), &config).unwrap();
    let mut after = before.clone();
    for n in &mut after.nodes {
        n.enabled = false;
    }
    after.mode = Mode::Xray;
    let node = nodes::create(
        &after,
        &config,
        "vless-xhttp-tls",
        "XHTTP",
        None,
        Some(CoreKind::Xray),
    )
    .unwrap();
    let id = node.id.clone();
    after.nodes.push(node);
    sbctl::deployment::apply(&store, &config, &before, after).unwrap();
    let generation = sbctl::deployment::current(root.path()).unwrap().unwrap();
    assert!(!generation.files.iter().any(|n| n == "sing-box-server.json"));
    assert!(
        !generation
            .files
            .iter()
            .any(|n| n == "subscription-sing-box-full.json")
    );
    assert!(
        generation
            .files
            .iter()
            .any(|n| n == "subscription-clash.yaml")
    );
    assert!(
        subscription::read_authorized(
            &store,
            &config,
            &config.subscription_credential,
            Format::Uri
        )
        .unwrap()
        .contains("type=xhttp")
    );
    let registry = nodes::read_optional(root.path()).unwrap().unwrap();
    let mut next = registry.clone();
    assert!(next.switch(Mode::SingBox, &[]).is_err());
    next = registry;
    next.switch(Mode::SingBox, &[id]).unwrap();
}
#[test]
fn invalid_template_and_stale_draft_preserve_published_generation() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let registry = nodes::load_or_import(root.path(), &config).unwrap();
    sbctl::deployment::apply(&store, &config, &registry, registry.clone()).unwrap();
    let before = sbctl::deployment::current(root.path()).unwrap().unwrap().id;
    assert!(sbctl::deployment::apply(&store, &config, &registry, registry.clone()).is_err());
    let current = nodes::read_optional(root.path()).unwrap().unwrap();
    store
        .write_root_only_locked("etc/sbctl/templates/client/sing-box.json", b"{bad")
        .unwrap();
    assert!(sbctl::deployment::apply(&store, &config, &current, current.clone()).is_err());
    assert_eq!(
        sbctl::deployment::current(root.path()).unwrap().unwrap().id,
        before
    );
    assert!(nodes::read_optional(root.path()).unwrap().unwrap() == current);
    store
        .write_root_only_locked("etc/sbctl/templates/client/sing-box.json", br#"{"dns":23}"#)
        .unwrap();
    assert!(sbctl::deployment::apply(&store, &config, &current, current.clone()).is_err());
    assert_eq!(
        sbctl::deployment::current(root.path()).unwrap().unwrap().id,
        before
    );
}

#[test]
fn xray_legacy_nodes_export_explicit_vmess_aead_and_filter_unvalidated_reality() {
    let (root, config) = fixture();
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    registry.switch(Mode::Xray, &[]).unwrap();
    let artifacts = subscription::managed::generate(&config, root.path(), &registry).unwrap();
    let full: Value = serde_json::from_str(
        &artifacts
            .iter()
            .find(|(n, _)| n == "subscription-sing-box-full.json")
            .unwrap()
            .1,
    )
    .unwrap();
    assert!(
        !full["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["type"] == "vless")
    );
    assert_eq!(
        full["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["type"] == "vmess")
            .unwrap()["security"],
        "aes-128-gcm"
    );
    let uri = &artifacts
        .iter()
        .find(|(n, _)| n == "subscription-uri.txt")
        .unwrap()
        .1;
    assert!(uri.contains("vless://"));
    use base64::Engine;
    let payload = uri
        .lines()
        .find_map(|s| s.strip_prefix("vmess://"))
        .unwrap();
    let vmess: Value = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(vmess["scy"], "aes-128-gcm");
}

#[test]
fn server_overrides_cannot_desynchronize_managed_nodes_or_disappear_on_mode_change() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    let overlay = format!(
        r#"{{"inbounds":[{{"tag":"{}","listen_port":23456}}]}}"#,
        registry.nodes[0].tag
    );
    store
        .write_root_only_locked(
            "etc/sbctl/overrides/sing-box-server.json",
            overlay.as_bytes(),
        )
        .unwrap();
    assert!(subscription::managed::generate(&config, root.path(), &registry).is_err());
    store.write_root_only_locked("etc/sbctl/overrides/sing-box-server.json", br#"{"inbounds":[{"type":"socks","tag":"custom-local","listen":"127.0.0.1","listen_port":23456}]}"#).unwrap();
    assert!(subscription::managed::generate(&config, root.path(), &registry).is_ok());
    registry.switch(Mode::Xray, &[]).unwrap();
    assert!(subscription::managed::generate(&config, root.path(), &registry).is_err());
    store
        .write_root_only_locked("etc/sbctl/overrides/sing-box-server.json", b"{}")
        .unwrap();
    assert!(subscription::managed::generate(&config, root.path(), &registry).is_ok());
}

#[test]
#[ignore = "requires SBCTL_XRAY_BIN and SING_BOX_BIN_1_14"]
fn xray_vmess_tls_carries_a_real_request_from_sing_box() {
    use std::{
        io::{Read, Write},
        process::{Command, Stdio},
    };
    let (root, config) = fixture();
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    registry.mode = Mode::Xray;
    registry.nodes[0].enabled = false;
    let node = &mut registry.nodes[1];
    node.core = CoreKind::Xray;
    node.patch("server", serde_json::json!("127.0.0.1"))
        .unwrap();
    let mut server = subscription::managed::servers(&config, root.path(), &registry)
        .unwrap()
        .remove(0)
        .1;
    server["outbounds"][0]["settings"]["finalRules"] =
        serde_json::json!([{"action":"allow","ip":["127.0.0.1/32"]}]);
    let path = root.path().join("xray.json");
    std::fs::write(&path, server.to_string()).unwrap();
    let mut child = Command::new(std::env::var_os("SBCTL_XRAY_BIN").unwrap())
        .args(["run", "-c"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = origin.local_addr().unwrap().port();
    origin.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            if let Ok((mut stream, _)) = origin.accept() {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let _ = stream.read(&mut [0u8; 4096]);
                let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    let binary = std::path::PathBuf::from(std::env::var_os("SING_BOX_BIN_1_14").unwrap());
    let result = sbctl::probe::node(
        root.path(),
        &config,
        &registry.nodes[1],
        &format!("http://127.0.0.1:{port}"),
        Some(&binary),
    );
    let _ = child.kill();
    let _ = child.wait();
    thread.join().unwrap();
    assert_eq!(result.unwrap().http_status, 204);
}
#[test]
fn historical_rollback_never_restores_subscription_credentials_or_accounting() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let before = nodes::load_or_import(root.path(), &config).unwrap();
    sbctl::deployment::apply(&store, &config, &before, before.clone()).unwrap();
    let before = nodes::read_optional(root.path()).unwrap().unwrap();
    let mut after = before.clone();
    after.nodes[0].name = "changed".into();
    sbctl::deployment::apply(&store, &config, &before, after).unwrap();
    let mut rotated = config.clone();
    rotated.subscription_credential = sbctl::config::generate_subscription_credential().unwrap();
    store.replace(&rotated).unwrap();
    store.write_state(b"unique-accounting-marker").unwrap();
    sbctl::deployment::rollback(&store).unwrap();
    assert!(store.load().unwrap() == rotated);
    assert_eq!(
        store.read_state().unwrap().unwrap(),
        b"unique-accounting-marker"
    );
    assert!(nodes::read_optional(root.path()).unwrap().unwrap().nodes == before.nodes);
    assert!(
        subscription::read_authorized(
            &store,
            &rotated,
            &config.subscription_credential,
            Format::Uri
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires pinned official cores via SBCTL_XRAY_BIN, SING_BOX_BIN_1_10..1_14 and MIHOMO_BIN"]
fn managed_generations_pass_real_servers_and_every_client_profile() {
    let (root, config) = fixture();
    let before = nodes::load_or_import(root.path(), &config).unwrap();
    let mut registry = before.clone();
    registry.mode = Mode::Dual;
    for preset in [
        "hysteria2",
        "tuic",
        "anytls",
        "vless-xhttp-reality",
        "vless-xhttp-tls",
        "vless-ws-tls",
        "trojan-tcp-tls",
    ] {
        let core = if preset.starts_with("vless-xhttp") {
            Some(CoreKind::Xray)
        } else {
            None
        };
        let node = nodes::create(&registry, &config, preset, preset, None, core).unwrap();
        registry.nodes.push(node);
    }
    registry.groups.push(nodes::Group {
        id: "all".into(),
        name: "全部测试节点".into(),
        members: registry.nodes.iter().map(|n| n.id.clone()).collect(),
        strategy: Strategy::Urltest,
    });
    for (core, doc) in subscription::managed::servers(&config, root.path(), &registry).unwrap() {
        let var = if core == CoreKind::Xray {
            "SBCTL_XRAY_BIN"
        } else {
            "SING_BOX_BIN_1_14"
        };
        let binary = std::path::PathBuf::from(std::env::var_os(var).expect(var));
        core.check_config(&binary, &doc.to_string())
            .unwrap_or_else(|e| panic!("{core}: {e}"));
    }
    let artifacts = subscription::managed::generate(&config, root.path(), &registry).unwrap();
    for row in subscription::subscription_matrix() {
        let binary = match row.format {
            Format::SingBoxVersion(v) => Some(std::path::PathBuf::from(
                std::env::var_os(format!("SING_BOX_BIN_{}_{}", v.major, v.minor))
                    .expect("version core"),
            )),
            Format::SingBoxFull => Some(std::path::PathBuf::from(
                std::env::var_os("SING_BOX_BIN_1_14").unwrap(),
            )),
            _ => None,
        };
        for name in [
            row.format.artifact_name().into_owned(),
            format!("group-all--{}", row.format.artifact_name()),
        ] {
            let Some((_, text)) = artifacts.iter().find(|(n, _)| n == &name) else {
                continue;
            };
            if let Some(binary) = &binary {
                CoreKind::SingBox
                    .check_config(binary, text)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            }
            if row.format == Format::Clash {
                let path = root.path().join("check.yaml");
                std::fs::write(&path, text).unwrap();
                let output =
                    std::process::Command::new(std::env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN"))
                        .arg("-t")
                        .arg("-d")
                        .arg(root.path())
                        .arg("-f")
                        .arg(&path)
                        .output()
                        .unwrap();
                assert!(
                    output.status.success(),
                    "{name}: Mihomo check failed (diagnostics withheld)"
                );
            }
        }
    }
}
