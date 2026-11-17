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
fn all_client_profiles_route_private_ips_without_a_remote_geoip_private_dependency() {
    let (root, mut config) = fixture();
    for template in [
        subscription::ClientTemplate::Standard,
        subscription::ClientTemplate::Compact,
        subscription::ClientTemplate::Global,
        subscription::ClientTemplate::Split,
    ] {
        config.client_template = template;
        let registry = nodes::load_or_import(root.path(), &config).unwrap();
        let artifacts = subscription::managed::generate(&config, root.path(), &registry).unwrap();
        for (name, text) in artifacts
            .iter()
            .filter(|(name, _)| name.starts_with("subscription-sing-box-"))
        {
            let profile: Value = serde_json::from_str(text).unwrap();
            assert!(
                profile["route"]["rule_set"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|s| s["tag"] != "geoip-private"),
                "{name} must not download the nonexistent private-IP dataset"
            );
            assert!(
                profile["route"]["rules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["ip_is_private"] == true && r["outbound"] == "DIRECT"),
                "{name} must preserve private-IP direct routing"
            );
        }
    }
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
    assert_eq!(doc["outbounds"][0]["tag"], "代理");
    assert_eq!(doc["outbounds"][1]["tag"], "自动选择");
    assert_eq!(doc["outbounds"][2]["tag"], tag);
    for doc_name in generation
        .files
        .iter()
        .filter(|name| name.contains("subscription-sing-box-") && name.ends_with(".json"))
    {
        let profile: Value = serde_json::from_str(
            &sbctl::deployment::read_generation(root.path(), &generation, doc_name).unwrap(),
        )
        .unwrap();
        let policy_tags: Vec<_> = profile["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| matches!(v["type"].as_str(), Some("selector") | Some("urltest")))
            .map(|v| v["tag"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(policy_tags, vec!["代理", "自动选择"], "{doc_name}");
    }
    for clash_name in generation
        .files
        .iter()
        .filter(|name| name.contains("subscription-clash") && name.ends_with(".yaml"))
    {
        let clash: serde_yaml::Value = serde_yaml::from_str(
            &sbctl::deployment::read_generation(root.path(), &generation, clash_name).unwrap(),
        )
        .unwrap();
        let group_names: Vec<_> = clash["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|g| g["name"].as_str().unwrap())
            .collect();
        assert_eq!(group_names, ["代理", "自动选择"], "{clash_name}");
    }
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

#[test]
fn self_signed_certificate_is_materialized_on_demand_when_missing() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let cert_dir = root.path().join("var/lib/sbctl/certificates");
    if cert_dir.exists() {
        std::fs::remove_dir_all(&cert_dir).unwrap();
    }
    let before = nodes::load_or_import(root.path(), &config).unwrap();
    let mut after = before.clone();
    after.switch(Mode::Xray, &[]).unwrap();
    sbctl::deployment::apply(&store, &config, &before, after).unwrap();
    let leaf_dir = cert_dir.join(config.protocol_server_name());
    assert!(leaf_dir.join("cert.pem").is_file());
    assert!(leaf_dir.join("key.pem").is_file());
}

#[test]
fn port_hopping_does_not_block_switching_to_xray_mode_or_disabling_hy2() {
    let root = tempfile::tempdir().unwrap();
    let mut config = DeploymentConfig::new(
        SubscriptionMode::IpFallback,
        "203.0.113.8".into(),
        None,
        Some(2080),
        "ens3".into(),
        vec![ManagedProtocol::VlessReality, ManagedProtocol::Hysteria2],
        Some("www.cloudflare.com".into()),
    )
    .unwrap();
    config.hysteria2_port_hopping = Some(sbctl::port_hopping::PortHopping {
        start: 30000,
        end: 30010,
        interval_seconds: 10,
    });
    let store = DeploymentStore::new(root.path());
    store.initialize(&config).unwrap();
    subscription::regenerate(&store, &config, None, false).unwrap();

    let before = nodes::load_or_import(root.path(), &config).unwrap();
    assert!(before.has_active_sing_box_hysteria2(&config));
    let mut after = before.clone();
    let incompatible: Vec<String> = after
        .incompatible_enabled_nodes(Mode::Xray)
        .into_iter()
        .map(|n| n.id.clone())
        .collect();
    assert_eq!(incompatible.len(), 1);
    after.switch(Mode::Xray, &incompatible).unwrap();
    assert!(!after.has_active_sing_box_hysteria2(&config));

    let plan = sbctl::deployment::plan(root.path(), &config, &before, &after).unwrap();
    assert!(
        !plan.warnings.is_empty(),
        "switching to Xray-only VLESS Reality should warn about excluded client formats"
    );
    sbctl::deployment::apply(&store, &config, &before, after).unwrap();
}

#[test]
fn service_status_and_logs_follow_runtime_mode() {
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let first = nodes::load_or_import(root.path(), &config).unwrap();
    sbctl::deployment::apply(&store, &config, &first, first.clone()).unwrap();

    let sing_status: Vec<&str> = sbctl::lifecycle::service_status_entries(root.path())
        .into_iter()
        .map(|(u, _)| u)
        .collect();
    assert!(sing_status.contains(&"sing-box.service"));
    assert!(!sing_status.contains(&"sbctl-xray.service"));
    let sing_logs = sbctl::observe::journalctl_args_for_root(root.path(), "all", 50, false);
    assert!(sing_logs.contains(&"sing-box.service".to_owned()));
    assert!(!sing_logs.contains(&"sbctl-xray.service".to_owned()));

    let current = nodes::read_optional(root.path()).unwrap().unwrap();
    let mut xray_mode = current.clone();
    xray_mode.switch(Mode::Xray, &[]).unwrap();
    sbctl::deployment::apply(&store, &config, &current, xray_mode).unwrap();

    let xray_status: Vec<&str> = sbctl::lifecycle::service_status_entries(root.path())
        .into_iter()
        .map(|(u, _)| u)
        .collect();
    assert!(!xray_status.contains(&"sing-box.service"));
    assert!(xray_status.contains(&"sbctl-xray.service"));
    let xray_logs = sbctl::observe::journalctl_args_for_root(root.path(), "all", 50, false);
    assert!(!xray_logs.contains(&"sing-box.service".to_owned()));
    assert!(xray_logs.contains(&"sbctl-xray.service".to_owned()));

    let current = nodes::read_optional(root.path()).unwrap().unwrap();
    let mut dual_mode = current.clone();
    dual_mode.switch(Mode::Dual, &[]).unwrap();
    sbctl::deployment::apply(&store, &config, &current, dual_mode).unwrap();

    let dual_status: Vec<&str> = sbctl::lifecycle::service_status_entries(root.path())
        .into_iter()
        .map(|(u, _)| u)
        .collect();
    assert!(dual_status.contains(&"sing-box.service"));
    assert!(dual_status.contains(&"sbctl-xray.service"));
    let dual_logs = sbctl::observe::journalctl_args_for_root(root.path(), "all", 50, false);
    assert!(dual_logs.contains(&"sbctl.service".to_owned()));
    assert!(dual_logs.contains(&"sing-box.service".to_owned()));
    assert!(dual_logs.contains(&"sbctl-xray.service".to_owned()));
}
