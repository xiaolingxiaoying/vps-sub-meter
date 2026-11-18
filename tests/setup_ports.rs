use sbctl::{
    config::{DeploymentConfig, DeploymentStore, ManagedProtocol, SubscriptionMode},
    nodes,
    shared_tls::{Route, SharedTls},
    subscription,
};
use serde_json::{Value, json};
use std::fs;
#[cfg(unix)]
use std::path::Path;
fn fixture() -> (tempfile::TempDir, DeploymentConfig) {
    let root = tempfile::tempdir().unwrap();
    let config = DeploymentConfig::new(
        SubscriptionMode::Direct,
        "sub.example.test".into(),
        None,
        None,
        "ens3".into(),
        vec![
            ManagedProtocol::VlessReality,
            ManagedProtocol::VmessWebsocket,
            ManagedProtocol::Hysteria2,
            ManagedProtocol::Tuic,
            ManagedProtocol::Anytls,
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
fn completing_old_and_new_deployments_is_idempotent_and_preserves_original_node_identity() {
    let (root, config) = fixture();
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    let before = registry.clone();
    registry.complete_catalog(&config).unwrap();
    assert_eq!(registry.nodes.len(), 9);
    for old in before.nodes {
        let node = registry.nodes.iter().find(|n| n.id == old.id).unwrap();
        assert!(node.data == old.data);
        assert_eq!(node.port(), old.port());
    }
    let completed = registry.clone();
    registry.complete_catalog(&config).unwrap();
    assert!(completed == registry);
    assert!(
        registry
            .nodes
            .iter()
            .all(|n| n.tag == n.name && !n.tag.starts_with("node-"))
    );
    let batch = subscription::managed::generate(&config, root.path(), &registry).unwrap();
    let catalog: subscription::managed::Catalog = serde_json::from_str(
        &batch
            .iter()
            .find(|(n, _)| n == subscription::managed::CATALOG)
            .unwrap()
            .1,
    )
    .unwrap();
    let count = |format: &str| {
        catalog
            .reports
            .iter()
            .find(|r| r.group.is_none() && r.format == format)
            .unwrap()
            .included
    };
    assert_eq!(count("uri"), 9);
    assert_eq!(count("sing-box-full.json"), 7);
    assert!(
        catalog
            .reports
            .iter()
            .any(|r| r.group.is_none() && !r.excluded.is_empty())
    );
}
#[test]
fn fresh_setup_accepts_detected_ip_and_has_working_sni_port_and_certificate_defaults() {
    struct Defaults {
        reports: Vec<String>,
    }
    impl sbctl::wizard::Prompts for Defaults {
        fn ask(&mut self, _: &str, _: Option<&str>) -> std::io::Result<String> {
            Ok(String::new())
        }
        fn report(&mut self, s: &str) {
            self.reports.push(s.into());
        }
        fn confirm(&mut self, s: &str, default: bool) -> std::io::Result<bool> {
            Ok(s.contains("确认提交") || default)
        }
    }
    let mut prompts = Defaults { reports: vec![] };
    let outcome = sbctl::wizard::run_with_host(
        None,
        Some("ens3".into()),
        Some("1.1.1.1".into()),
        &mut prompts,
    )
    .unwrap();
    let sbctl::wizard::WizardOutcome::Changed(config) = outcome else {
        panic!("expected initialized config")
    };
    assert_eq!(config.subscription_mode, SubscriptionMode::IpFallback);
    assert_eq!(config.http_port, Some(2080));
    assert_eq!(
        config.reality_decoy_sni.as_deref(),
        Some("www.cloudflare.com")
    );
    assert_eq!(config.protocol_server_name(), "www.bing.com");
    config.validate().unwrap();
}
fn shared_fixture() -> (
    tempfile::TempDir,
    DeploymentConfig,
    nodes::Registry,
    String,
    u16,
) {
    let (root, mut config) = fixture();
    let mut registry = nodes::load_or_import(root.path(), &config).unwrap();
    let id = registry.nodes[0].id.clone();
    let port = registry.nodes[0].port();
    registry.nodes[0].patch("port", json!(443)).unwrap();
    config.shared_tls = Some(SharedTls {
        routes: vec![
            Route {
                server_name: "www.cloudflare.com".into(),
                backend: ([127, 0, 0, 1], port).into(),
                node_id: Some(id.clone()),
            },
            Route {
                server_name: "web.example.test".into(),
                backend: ([127, 0, 0, 1], 8443).into(),
                node_id: None,
            },
        ],
    });
    (root, config, registry, id, port)
}
#[test]
fn tcp_mux_udp_443_and_https_subscription_have_distinct_real_listeners() {
    let (root, config, mut registry, id, port) = shared_fixture();
    registry
        .nodes
        .iter_mut()
        .find(|n| n.preset() == "hysteria2")
        .unwrap()
        .patch("port", json!(443))
        .unwrap();
    config.validate().unwrap();
    registry.validate(&config).unwrap();
    let servers = subscription::managed::servers(&config, root.path(), &registry).unwrap();
    let server = &servers[0].1;
    let tag = &registry.nodes.iter().find(|n| n.id == id).unwrap().tag;
    let inbound = server["inbounds"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["tag"] == *tag)
        .unwrap();
    assert_eq!(inbound["listen"], "127.0.0.1");
    assert_eq!(inbound["listen_port"], port);
    let batch = subscription::managed::generate(&config, root.path(), &registry).unwrap();
    let clients: Value = serde_json::from_str(
        &batch
            .iter()
            .find(|(n, _)| n == "subscription-sing-box.json")
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(clients["outbounds"][0]["server_port"], 443);
}
#[test]
fn shared_backends_reserve_ports_even_when_offline_and_cannot_target_the_api() {
    let (_root, mut config, registry, _, _) = shared_fixture();
    let available = nodes::free_port(&registry, &config).unwrap();
    config.shared_tls.as_mut().unwrap().routes[1]
        .backend
        .set_port(available);
    assert_ne!(nodes::free_port(&registry, &config).unwrap(), available);
    config.server_clash_api = Some(sbctl::config::ServerClashApi {
        port: available,
        secret: "test-secret".into(),
    });
    assert!(
        config
            .shared_tls
            .as_ref()
            .unwrap()
            .validate(&config)
            .is_err()
    );
}

#[test]
fn observation_api_requires_a_header_safe_secret_and_a_separate_port() {
    let (_root, mut config) = fixture();
    let mut api = sbctl::config::generate_server_clash_api().unwrap();
    for secret in ["", "two words", "invalid\r\nheader"] {
        api.secret = secret.into();
        config.server_clash_api = Some(api.clone());
        assert!(config.validate().is_err());
    }
    api.secret = "valid-token".into();
    for port in [80, 443] {
        api.port = port;
        config.server_clash_api = Some(api.clone());
        assert!(config.validate().is_err());
    }
}

#[test]
fn mux_rejects_duplicate_sni_remote_backends_subscription_recursion_and_unrouted_tcp_443() {
    let (_, config, registry, _, _) = shared_fixture();
    for name in ["SUB.EXAMPLE.TEST", "www.cloudflare.com"] {
        let mut bad = config.clone();
        bad.shared_tls.as_mut().unwrap().routes.push(Route {
            server_name: name.into(),
            backend: ([127, 0, 0, 1], 8555).into(),
            node_id: None,
        });
        assert!(bad.validate().is_err());
    }
    let mut bad = config.clone();
    bad.shared_tls.as_mut().unwrap().routes[1].backend = ([1, 1, 1, 1], 8443).into();
    assert!(bad.validate().is_err());
    let mut bad = config.clone();
    bad.shared_tls = None;
    assert!(registry.validate(&bad).is_err());
    let mut bad = config.clone();
    bad.shared_tls.as_mut().unwrap().routes[0].backend = ([127, 0, 0, 1], 443).into();
    assert!(bad.validate().is_err());
}
#[test]
fn port_cli_preview_apply_and_rollback_keep_credentials_and_publish_the_advertised_port() {
    use assert_cmd::Command;
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let registry = nodes::load_or_import(root.path(), &config).unwrap();
    let id = registry.nodes[0].id.clone();
    sbctl::deployment::apply(&store, &config, &registry, registry.clone()).unwrap();
    let config_bytes = fs::read(root.path().join("etc/sbctl/config.toml")).unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root.path().to_str().unwrap(),
            "port",
            "share",
            "--node",
            &id,
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read(root.path().join("etc/sbctl/config.toml")).unwrap(),
        config_bytes
    );
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root.path().to_str().unwrap(),
            "port",
            "share",
            "--node",
            &id,
            "--website-sni",
            "web.example.test",
            "--website-port",
            "8443",
            "--apply",
        ])
        .assert()
        .success();
    let current = store.load().unwrap();
    assert_eq!(
        current.subscription_credential,
        config.subscription_credential
    );
    let nodes = nodes::load_or_import(root.path(), &current).unwrap();
    assert_eq!(nodes.nodes[0].port(), 443);
    let uri = subscription::read_authorized(
        &store,
        &current,
        &current.subscription_credential,
        subscription::SubscriptionFormat::Uri,
    )
    .unwrap();
    assert!(uri.lines().next().unwrap().contains(":443?"));
    sbctl::deployment::rollback(&store).unwrap();
    assert_eq!(store.load().unwrap(), config);
    assert_eq!(
        nodes::read_optional(root.path()).unwrap().unwrap().nodes[0].port(),
        registry.nodes[0].port()
    );
}
#[test]
fn alternate_sources_require_https_and_an_explicit_trusted_file_hash() {
    use sbctl::kernel_source::{CustomSource, validate};
    let digest = "a".repeat(64);
    for url in [
        "http://example.com/core",
        "file:///tmp/core",
        "https://u:pw@example.com/core",
        "https://example.com/core#fragment",
    ] {
        assert!(
            validate(&CustomSource {
                url,
                sha256: &digest
            })
            .is_err()
        );
    }
    assert!(
        validate(&CustomSource {
            url: "https://example.com/core.tar.gz",
            sha256: &digest
        })
        .is_ok()
    );
    assert!(
        validate(&CustomSource {
            url: "https://example.com/core",
            sha256: "bad"
        })
        .is_err()
    );
}

#[test]
fn upgrading_a_legacy_server_renders_for_the_candidate_without_changing_the_node_source() {
    let (root, mut config) = fixture();
    config.ipv4_only = true;
    let registry = nodes::load_or_import(root.path(), &config).unwrap();
    let old =
        subscription::managed::servers_for_kernel(&config, root.path(), &registry, Some(true))
            .unwrap();
    let new =
        subscription::managed::servers_for_kernel(&config, root.path(), &registry, Some(false))
            .unwrap();
    assert_eq!(old[0].1["inbounds"][0]["domain_strategy"], "ipv4_only");
    assert!(new[0].1["inbounds"][0].get("domain_strategy").is_none());
    assert!(nodes::read_optional(root.path()).unwrap().is_none());
    for index in 0..registry.nodes.len() {
        assert_eq!(
            old[0].1["inbounds"][index]["listen_port"],
            new[0].1["inbounds"][index]["listen_port"]
        );
    }
}

#[cfg(unix)]
#[test]
fn custom_download_hash_failure_and_html_never_replace_the_installed_core() {
    use assert_cmd::Command;
    use sha2::{Digest, Sha256};
    let (root, _) = fixture();
    command(
        root.path(),
        "curl",
        "while [ $# -gt 0 ]; do if [ \"$1\" = --output ]; then shift; destination=$1; fi; shift; done\nprintf '<html>not a kernel</html>' > \"$destination\"",
    );
    let path = format!(
        "{}:{}",
        root.path().join("usr/bin").display(),
        std::env::var("PATH").unwrap()
    );
    let binary = root.path().join("usr/local/bin/sing-box");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, b"original core").unwrap();
    for (hash, expected) in [
        ("a".repeat(64), "SHA-256 不匹配"),
        (
            format!("{:x}", Sha256::digest(b"<html>not a kernel</html>")),
            "不能是网页",
        ),
    ] {
        Command::cargo_bin("sbctl")
            .unwrap()
            .env("PATH", &path)
            .arg("--root")
            .arg(root.path())
            .args([
                "core",
                "install",
                "sing-box",
                "--version",
                "1.14.2",
                "--url",
                "https://example.test/core",
                "--sha256",
                &hash,
            ])
            .assert()
            .code(2)
            .stderr(predicates::str::contains(expected));
        assert_eq!(fs::read(&binary).unwrap(), b"original core");
    }
}
#[cfg(unix)]
fn command(root: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let file = root.join(format!("usr/bin/{name}"));
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "requires authenticated real sing-box 1.14.1 via SING_BOX_BIN_1_14"]
fn a_custom_core_upgrade_migrates_legacy_configuration_and_can_roll_back_every_affected_file() {
    use assert_cmd::Command;
    use sha2::{Digest, Sha256};
    let candidate = std::env::var("SING_BOX_BIN_1_14").unwrap();
    let contents = fs::read(&candidate).unwrap();
    let hash = format!("{:x}", Sha256::digest(&contents));
    let (root, mut config) = fixture();
    config.ipv4_only = true;
    DeploymentStore::new(root.path()).replace(&config).unwrap();
    let registry = nodes::load_or_import(root.path(), &config).unwrap();
    let legacy =
        subscription::managed::servers_for_kernel(&config, root.path(), &registry, Some(true))
            .unwrap()
            .remove(0)
            .1
            .to_string();
    let active = root.path().join("etc/sing-box/config.json");
    fs::create_dir_all(active.parent().unwrap()).unwrap();
    fs::write(&active, &legacy).unwrap();
    let binary = root.path().join("usr/local/bin/sing-box");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, b"broken old executable").unwrap();
    command(root.path(), "systemctl", "exit 0");
    command(
        root.path(),
        "curl",
        &format!(
            "while [ $# -gt 0 ]; do if [ \"$1\" = --output ]; then shift; destination=$1; fi; shift; done\ncp '{}' \"$destination\"",
            candidate.replace('\'', "'\\''")
        ),
    );
    let path = format!(
        "{}:{}",
        root.path().join("usr/bin").display(),
        std::env::var("PATH").unwrap()
    );
    Command::cargo_bin("sbctl")
        .unwrap()
        .env("PATH", path)
        .arg("--root")
        .arg(root.path())
        .args([
            "core",
            "install",
            "sing-box",
            "--version",
            "1.14.1",
            "--url",
            "https://example.test/core",
            "--sha256",
            &hash,
        ])
        .assert()
        .success();
    let updated: Value = serde_json::from_slice(&fs::read(&active).unwrap()).unwrap();
    assert!(updated["inbounds"][0].get("domain_strategy").is_none());
    assert_eq!(
        Sha256::digest(fs::read(&binary).unwrap())[..],
        Sha256::digest(&contents)[..]
    );
    let store = DeploymentStore::new(root.path());
    assert_eq!(store.load().unwrap(), config);
    assert!(nodes::read_optional(root.path()).unwrap().is_none());
    sbctl::deployment::rollback(&store).unwrap();
    assert_eq!(fs::read(&binary).unwrap(), b"broken old executable");
    assert_eq!(fs::read_to_string(&active).unwrap(), legacy);
    assert_eq!(store.load().unwrap(), config);
}
#[cfg(unix)]
#[test]
fn automatic_ip_discovery_rejects_private_answers_and_never_uses_proxy_environment() {
    let root = tempfile::tempdir().unwrap();
    command(
        root.path(),
        "curl",
        &format!(
            "printf '%s\\n' \"$*\" >> '{}/calls'\ncase \"$*\" in *api64*) echo 2606:4700:4700::1111;; *) echo 10.0.0.1;; esac",
            root.path().display()
        ),
    );
    assert_eq!(
        sbctl::egress::discover(root.path()).unwrap().to_string(),
        "2606:4700:4700::1111"
    );
    assert!(
        fs::read_to_string(root.path().join("calls"))
            .unwrap()
            .contains("--noproxy *")
    );
}
#[cfg(unix)]
#[test]
fn smtp_delivery_releases_the_global_lock_keeps_secrets_out_of_argv_and_deduplicates_daily_reports()
{
    let (root, config) = fixture();
    let store = DeploymentStore::new(root.path());
    let mail = sbctl::email::EmailConfig {
        password: "private-auth-code".into(),
        ..Default::default()
    };
    {
        let _guard = store.acquire_operation_lock().unwrap();
        store
            .write_root_only_locked(
                sbctl::email::CONFIG_PATH,
                toml::to_string(&mail).unwrap().as_bytes(),
            )
            .unwrap();
    }
    command(
        root.path(),
        "curl",
        &format!(
            "flock -n '{}/var/lib/sbctl/.operation.lock' -c true || exit 99\nprintf '%s\\n' \"$*\" >> '{}/argv'",
            root.path().display(),
            root.path().display()
        ),
    );
    assert!(sbctl::email::send(root.path(), false).unwrap());
    assert!(sbctl::email::send(root.path(), true).unwrap());
    assert!(!sbctl::email::send(root.path(), true).unwrap());
    assert!(sbctl::email::last_delivery(root.path()).is_some());
    assert!(
        !fs::read_to_string(root.path().join("argv"))
            .unwrap()
            .contains("private-auth-code")
    );
    let report = sbctl::email::report(root.path(), &config, false, false).unwrap();
    assert!(!report.contains(&config.subscription_credential));
}
