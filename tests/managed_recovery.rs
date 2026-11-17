use base64::Engine;
use sbctl::{
    config::{
        CertificateMode, DeploymentConfig, DeploymentStore, ManagedProtocol, SubscriptionMode,
    },
    deployment,
    nodes::{self, Group, Strategy},
    subscription,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

fn fixture(protocol: ManagedProtocol) -> (tempfile::TempDir, DeploymentConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = DeploymentConfig::new(
        SubscriptionMode::ExternalProxy,
        "old.example.test".into(),
        None,
        None,
        "ens3".into(),
        vec![protocol],
        Some("www.cloudflare.com".into()),
    )
    .unwrap();
    let store = DeploymentStore::new(root.path());
    store.initialize(&config).unwrap();
    subscription::regenerate(&store, &config, None, false).unwrap();
    (root, config)
}
fn migrate(root: &Path, config: &DeploymentConfig) {
    let registry = nodes::load_or_import(root, config).unwrap();
    deployment::apply(
        &DeploymentStore::new(root),
        config,
        &registry,
        registry.clone(),
    )
    .unwrap();
}
fn artifact<'a>(batch: &'a [(String, String)], name: &str) -> &'a str {
    &batch.iter().find(|(n, _)| n == name).unwrap().1
}

fn age_directory(path: &Path) {
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .access_mode(0x100)
            .custom_flags(0x02000000)
            .open(path)
            .unwrap()
    };
    #[cfg(not(windows))]
    let file = fs::File::open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(600))
        .unwrap();
}

#[test]
fn domain_rollback_restores_matching_nodes_but_keeps_current_credentials_and_accounting() {
    let (root, config) = fixture(ManagedProtocol::VmessWebsocket);
    let store = DeploymentStore::new(root.path());
    migrate(root.path(), &config);
    let mut new = config.clone();
    new.subscription_host = "new.example.test".into();
    new.protocol_sni = Some("new.example.test".into());
    new.certificate_mode = CertificateMode::Domain;
    {
        let _lock = store.acquire_operation_lock().unwrap();
        deployment::apply_config_change_locked(&store, &new).unwrap();
    }
    let mut current = new.clone();
    current.subscription_credential = sbctl::config::generate_subscription_credential().unwrap();
    current
        .subscription_credentials
        .push(sbctl::config::NamedCredential {
            name: "new-device".into(),
            credential: sbctl::config::generate_subscription_credential().unwrap(),
            revoked_at: None,
        });
    current.monthly_traffic_limit = 987654321;
    current.accounting_timezone = "Asia/Shanghai".into();
    store.replace(&current).unwrap();
    store.write_state(b"unchanged accounting state").unwrap();
    let saved =
        fs::read_to_string(root.path().join("var/lib/sbctl/deployment-rollback.json")).unwrap();
    assert!(!saved.contains(&config.subscription_credential));
    deployment::rollback(&store).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(restored.subscription_host, config.subscription_host);
    assert_eq!(restored.protocol_sni, config.protocol_sni);
    assert_eq!(restored.certificate_mode, config.certificate_mode);
    assert_eq!(
        restored.subscription_credential,
        current.subscription_credential
    );
    assert_eq!(
        restored.subscription_credentials,
        current.subscription_credentials
    );
    assert_eq!(
        restored.monthly_traffic_limit,
        current.monthly_traffic_limit
    );
    assert_eq!(restored.accounting_timezone, current.accounting_timezone);
    assert_eq!(
        store.read_state().unwrap().unwrap(),
        b"unchanged accounting state"
    );
    nodes::load_or_import(root.path(), &restored).unwrap();
    subscription::read_authorized(
        &store,
        &restored,
        &restored.subscription_credential,
        subscription::SubscriptionFormat::Uri,
    )
    .unwrap();
}

#[test]
fn plan_never_creates_missing_certificate_material_but_apply_does() {
    let (root, config) = fixture(ManagedProtocol::VmessWebsocket);
    let certs = root.path().join("var/lib/sbctl/certificates");
    fs::remove_dir_all(&certs).unwrap();
    let registry = nodes::load_or_import(root.path(), &config).unwrap();
    deployment::plan(root.path(), &config, &registry, &registry).unwrap();
    subscription::managed::generate(&config, root.path(), &registry).unwrap();
    assert!(!certs.exists());
    deployment::apply(
        &DeploymentStore::new(root.path()),
        &config,
        &registry,
        registry.clone(),
    )
    .unwrap();
    assert!(
        certs
            .join(config.protocol_server_name())
            .join("key.pem")
            .is_file()
    );
}

#[test]
fn group_strategy_changes_the_default_without_removing_either_policy_group() {
    let (root, config) = fixture(ManagedProtocol::VmessWebsocket);
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    registry.groups.push(Group {
        id: "g".into(),
        name: "常用".into(),
        members: registry.nodes.iter().map(|n| n.id.clone()).collect(),
        strategy: Strategy::Select,
    });
    for automatic in [false, true] {
        registry.groups[0].strategy = if automatic {
            Strategy::Urltest
        } else {
            Strategy::Select
        };
        let batch = subscription::managed::generate(&config, root.path(), &registry).unwrap();
        for name in [
            "group-g--subscription-sing-box-full.json",
            "group-g--subscription-sing-box-1.10.json",
        ] {
            let doc: Value = serde_json::from_str(artifact(&batch, name)).unwrap();
            assert_eq!(
                doc["outbounds"][0]["default"],
                if automatic {
                    "自动选择"
                } else {
                    &registry.nodes[0].tag
                }
            );
            assert_eq!(doc["outbounds"][0]["type"], "selector");
            assert_eq!(doc["outbounds"][1]["type"], "urltest");
        }
        let doc: serde_yaml::Value =
            serde_yaml::from_str(artifact(&batch, "group-g--subscription-clash.yaml")).unwrap();
        assert_eq!(
            doc["proxy-groups"][0]["proxies"][0].as_str().unwrap(),
            if automatic {
                "自动选择"
            } else {
                &registry.nodes[0].tag
            }
        );
        assert_eq!(doc["proxy-groups"].as_sequence().unwrap().len(), 2);
    }
}

#[test]
fn a_second_hy2_uses_its_own_port_in_all_uri_representations() {
    let (root, mut config) = fixture(ManagedProtocol::Hysteria2);
    config.hysteria2.as_mut().unwrap().listen_port = 30001;
    config.hysteria2_port_hopping = Some(sbctl::port_hopping::PortHopping {
        start: 50000,
        end: 50100,
        interval_seconds: 30,
    });
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    let mut copy = registry.nodes[0].clone();
    copy.id = "copy".into();
    copy.tag = "copy".into();
    copy.renew_secrets().unwrap();
    copy.patch("port", json!(30002)).unwrap();
    registry.nodes.push(copy);
    let batch = subscription::managed::generate(&config, root.path(), &registry).unwrap();
    for name in [
        "subscription-uri.txt",
        "subscription-base64-uri.txt",
        "subscription-shadowrocket.txt",
    ] {
        let text = if name == "subscription-uri.txt" {
            artifact(&batch, name).to_string()
        } else {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(artifact(&batch, name).trim())
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            text.lines()
                .filter(|line| line.contains(":50000-50100?"))
                .count(),
            1,
            "{name}"
        );
        assert!(text.lines().nth(1).unwrap().contains(":30002?"), "{name}");
    }
}

#[test]
fn pruning_retains_current_rollback_and_recent_readers_and_removes_old_orphans() {
    let (root, config) = fixture(ManagedProtocol::VmessWebsocket);
    let store = DeploymentStore::new(root.path());
    let mut ids = Vec::new();
    for _ in 0..3 {
        migrate(root.path(), &config);
        ids.push(deployment::current(root.path()).unwrap().unwrap().id);
    }
    let directory = root.path().join("var/lib/sbctl/generations");
    let orphan = directory.join("ffffffffffffffffffffffffffffffff");
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("subscription-uri.txt"), b"orphan").unwrap();
    let recent = directory.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::create_dir(&recent).unwrap();
    for entry in fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path != recent {
            age_directory(&path);
        }
    }
    migrate(root.path(), &config);
    assert!(!directory.join(&ids[0]).exists());
    assert!(!orphan.exists());
    assert!(recent.exists());
    assert!(directory.join(&ids[1]).exists());
    assert!(directory.join(&ids[2]).exists());
    deployment::rollback(&store).unwrap();
    assert_eq!(
        deployment::current(root.path()).unwrap().unwrap().id,
        ids[2]
    );
    subscription::read_authorized(
        &store,
        &config,
        &config.subscription_credential,
        subscription::SubscriptionFormat::Uri,
    )
    .unwrap();
}

#[test]
fn interrupted_acme_bootstrap_recovers_original_config_without_publishing_a_batch() {
    let (root, config) = fixture(ManagedProtocol::VmessWebsocket);
    let store = DeploymentStore::new(root.path());
    migrate(root.path(), &config);
    let pointer = fs::read(root.path().join(deployment::CURRENT)).unwrap();
    let source = fs::read(root.path().join(nodes::SOURCE)).unwrap();
    let mut new = config.clone();
    new.subscription_mode = SubscriptionMode::Direct;
    new.subscription_listen_port = None;
    new.certificate_mode = CertificateMode::Domain;
    {
        let _lock = store.acquire_operation_lock().unwrap();
        deployment::begin_subscription_bootstrap_locked(&store, &new).unwrap();
    }
    assert_eq!(
        fs::read(root.path().join(deployment::CURRENT)).unwrap(),
        pointer
    );
    assert_eq!(fs::read(root.path().join(nodes::SOURCE)).unwrap(), source);
    deployment::recover(&store, true).unwrap();
    assert_eq!(store.load().unwrap(), config);
    assert_eq!(
        fs::read(root.path().join(deployment::CURRENT)).unwrap(),
        pointer
    );
    assert!(
        !root
            .path()
            .join("var/lib/sbctl/deployment-transaction.json")
            .exists()
    );
}

#[cfg(unix)]
fn command(root: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(format!("usr/bin/{name}"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn boot_recovery_restores_enabled_state_and_queues_start_without_blocking() {
    let (root, _) = fixture(ManagedProtocol::VmessWebsocket);
    let unit = "etc/systemd/system/sing-box.service";
    let xray = "etc/systemd/system/sbctl-xray.service";
    fs::create_dir_all(root.path().join(unit).parent().unwrap()).unwrap();
    fs::write(root.path().join(unit), "modified unit managed by sbctl").unwrap();
    fs::write(root.path().join(xray), "new unit managed by sbctl").unwrap();
    command(
        root.path(),
        "sbctl-deployment-systemctl",
        &format!(
            "printf '%s\\n' \"$*\" >> '{}/commands'\nif [ \"$1\" = disable ]; then test -f '{}/etc/systemd/system/'\"$2\"; fi",
            root.path().display(),
            root.path().display()
        ),
    );
    let record = json!({"files":[{"path":unit,"contents":base64::engine::general_purpose::STANDARD.encode("old unit managed by sbctl")},{"path":xray,"contents":null}],
        "states":[["sing-box.service",true,true],["sbctl-xray.service",false,false]],"applied_revision":2,"committed":false});
    fs::write(
        root.path()
            .join("var/lib/sbctl/deployment-transaction.json"),
        record.to_string(),
    )
    .unwrap();
    deployment::recover(&DeploymentStore::new(root.path()), true).unwrap();
    let calls = fs::read_to_string(root.path().join("commands")).unwrap();
    assert!(calls.contains("disable sbctl-xray.service\n"));
    assert!(calls.contains("--no-block stop sbctl-xray.service\n"));
    assert!(calls.contains("enable sing-box.service\n"));
    assert!(calls.contains("--no-block start sing-box.service\n"));
    assert!(!calls.lines().any(|l| l == "start sing-box.service"));
    assert!(!root.path().join(xray).exists());
}

#[cfg(unix)]
#[test]
fn managed_hopping_enable_range_change_and_rollback_reconcile_nftables() {
    let (root, mut config) = fixture(ManagedProtocol::Hysteria2);
    config.hysteria2.as_mut().unwrap().listen_port = 30001;
    let store = DeploymentStore::new(root.path());
    store.replace(&config).unwrap();
    command(
        root.path(),
        "nft",
        &format!(
            "if [ \"$1\" = list ]; then echo 'table inet sbctl_hy2'; else cat >> '{}/nft-script'; fi",
            root.path().display()
        ),
    );
    migrate(root.path(), &config);
    config.hysteria2_port_hopping = Some(sbctl::port_hopping::PortHopping {
        start: 50000,
        end: 50100,
        interval_seconds: 30,
    });
    {
        let _lock = store.acquire_operation_lock().unwrap();
        deployment::apply_config_change_locked(&store, &config).unwrap();
    }
    let path = root.path().join("nft-script");
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("udp dport 50000-50100")
    );
    fs::write(&path, "").unwrap();
    config.hysteria2_port_hopping.as_mut().unwrap().end = 50200;
    {
        let _lock = store.acquire_operation_lock().unwrap();
        deployment::apply_config_change_locked(&store, &config).unwrap();
    }
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("udp dport 50000-50200")
    );
    fs::write(&path, "").unwrap();
    deployment::rollback(&store).unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("udp dport 50000-50100")
    );
    fs::write(&path, "").unwrap();
    fs::write(root.path().join("etc/sbctl/config.toml"), b"damaged").unwrap();
    fs::write(root.path().join(nodes::SOURCE), b"damaged").unwrap();
    sbctl::port_hopping::apply_firewall(root.path(), true).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "delete table inet sbctl_hy2\n"
    );
}

#[cfg(unix)]
#[test]
fn failed_activation_restores_previous_hopping_rules_and_configuration() {
    let (root, mut config) = fixture(ManagedProtocol::Hysteria2);
    config.hysteria2.as_mut().unwrap().listen_port = 30001;
    config.hysteria2_port_hopping = Some(sbctl::port_hopping::PortHopping {
        start: 50000,
        end: 50100,
        interval_seconds: 30,
    });
    let store = DeploymentStore::new(root.path());
    store.replace(&config).unwrap();
    command(
        root.path(),
        "nft",
        &format!(
            "if [ \"$1\" = list ]; then echo 'table inet sbctl_hy2'; else cat >> '{}/nft-script'; fi",
            root.path().display()
        ),
    );
    migrate(root.path(), &config);
    command(
        root.path(),
        "sbctl-deployment-systemctl",
        &format!(
            "if [ \"$1\" = start ] && [ -f '{}/fail-once' ]; then rm '{}/fail-once'; exit 1; fi",
            root.path().display(),
            root.path().display()
        ),
    );
    fs::write(root.path().join("fail-once"), b"").unwrap();
    let mut next = config.clone();
    next.protocol_sni = Some("next.example.test".into());
    next.hysteria2_port_hopping.as_mut().unwrap().end = 50200;
    fs::write(root.path().join("nft-script"), b"").unwrap();
    {
        let _lock = store.acquire_operation_lock().unwrap();
        assert!(deployment::apply_config_change_locked(&store, &next).is_err());
    }
    assert_eq!(store.load().unwrap(), config);
    assert_eq!(
        nodes::read_optional(root.path())
            .unwrap()
            .unwrap()
            .nodes
            .len(),
        1
    );
    let rules = fs::read_to_string(root.path().join("nft-script")).unwrap();
    assert!(
        rules.rfind("udp dport 50000-50100").unwrap()
            > rules.rfind("udp dport 50000-50200").unwrap()
    );
    assert!(
        !root
            .path()
            .join("var/lib/sbctl/deployment-transaction.json")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn generation_cleanup_refuses_links_without_touching_external_files() {
    use std::os::unix::fs::symlink;
    let (root, config) = fixture(ManagedProtocol::VmessWebsocket);
    migrate(root.path(), &config);
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("subscription-uri.txt");
    fs::write(&sentinel, b"sentinel").unwrap();
    let generation = root
        .path()
        .join("var/lib/sbctl/generations/ffffffffffffffffffffffffffffffff");
    fs::create_dir(&generation).unwrap();
    symlink(&sentinel, generation.join("subscription-uri.txt")).unwrap();
    age_directory(&generation);
    migrate(root.path(), &config);
    assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");
    assert!(
        fs::symlink_metadata(generation.join("subscription-uri.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
