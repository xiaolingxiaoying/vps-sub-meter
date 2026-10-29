//! `sbctl config`: initialization, validation, regeneration and the
//! interactive wizard, and the accounting anchors they persist.

#[cfg(unix)]
use crate::fixture::write_command_fixture;
use assert_cmd::Command;
use base64::Engine;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

use crate::fixture::{
    free_high_tcp_port, read_subscription_credential, read_vless_uuid, sing_box_check_fixture,
    supported_systemd_host, write_managed_file, write_systemctl_fixture, write_traffic_fixture,
};

#[test]
fn configuration_initialization_persists_a_redacted_deployment_summary() {
    let fixture = TempDir::new().expect("temporary root is created");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "status",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("sbctl status: configured"))
        .stdout(predicate::str::contains("mode: ip-fallback"))
        .stdout(predicate::str::contains("subscription host: 203.0.113.7"))
        .stdout(predicate::str::contains("interface: ens3"))
        .stdout(predicate::str::contains("vless-reality"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is persisted");
    assert!(config.contains("subscription_credential"));
    assert!(!config.contains("[redacted]"));

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "show",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "subscription credential: [redacted]",
        ))
        .stdout(predicate::str::contains("subscription_credential =").not());
}

#[test]
fn configuration_validation_rejects_an_ip_fallback_host_that_is_not_an_ip_address() {
    let fixture = TempDir::new().expect("temporary root is created");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "sub.example.test",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "hysteria2",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "IP fallback subscription requires an IP address",
        ));

    assert!(!fixture.path().join("etc/sbctl/config.toml").exists());
}

#[test]
fn config_init_persists_explicit_ports_for_all_five_managed_protocols() {
    let fixture = TempDir::new().expect("temporary root is created");
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );
    let ports = [
        free_high_tcp_port(),
        free_high_tcp_port(),
        free_high_tcp_port(),
        free_high_tcp_port(),
        free_high_tcp_port(),
    ];

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "direct",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--protocol",
            "vmess-websocket",
            "--protocol",
            "hysteria2",
            "--protocol",
            "tuic",
            "--protocol",
            "anytls",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--vless-port",
            &ports[0].to_string(),
            "--vmess-port",
            &ports[1].to_string(),
            "--hysteria2-port",
            &ports[2].to_string(),
            "--tuic-port",
            &ports[3].to_string(),
            "--anytls-port",
            &ports[4].to_string(),
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success();

    let persisted: sbctl::config::DeploymentConfig = toml::from_str(
        &fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
            .expect("configuration is persisted"),
    )
    .expect("persisted configuration is valid TOML");
    assert_eq!(persisted.vless_reality.unwrap().listen_port, ports[0]);
    assert_eq!(persisted.vmess_websocket.unwrap().listen_port, ports[1]);
    assert_eq!(persisted.hysteria2.unwrap().listen_port, ports[2]);
    assert_eq!(persisted.tuic.unwrap().listen_port, ports[3]);
    assert_eq!(persisted.anytls.unwrap().listen_port, ports[4]);
}

#[test]
fn regenerate_validates_before_replacing_artifacts_and_the_active_config() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    let root = fixture.path().to_str().expect("fixture path is UTF-8");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "install",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
            "--no-start",
        ])
        .assert()
        .success();

    // A `--no-start` fixture install never commits ownership. Seed the marker
    // so `regenerate` exercises the fully-managed active-config path.
    write_managed_file(&fixture, "var/lib/sbctl/ownership", b"sbctl-managed-v1\n");

    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let configuration = fs::read_to_string(&config_path).expect("configuration is persisted");
    let changed = configuration.replace(
        "reality_decoy_sni = \"www.cloudflare.com\"",
        "reality_decoy_sni = \"www.apple.com\"",
    );
    assert_ne!(changed, configuration, "the canonical node field is edited");
    fs::write(&config_path, changed).expect("configuration is edited");

    let artifacts = fixture.path().join("var/lib/sbctl/artifacts");
    let active = fixture.path().join("etc/sing-box/config.json");
    let artifact_names = [
        "sing-box-server.json",
        "subscription-sing-box.json",
        "subscription-clash.yaml",
        "subscription-uri.txt",
        "subscription-base64-uri.txt",
    ];
    let snapshot = || {
        let mut files = Vec::new();
        for name in artifact_names {
            files.push(fs::read(artifacts.join(name)).expect("artifact is readable"));
        }
        files.push(fs::read(&active).expect("active configuration is readable"));
        files
    };
    let before = snapshot();

    let rejecting_fixture = TempDir::new().expect("rejecting checker root is created");
    let rejecting = sing_box_check_fixture(&rejecting_fixture, false, &[]);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "regenerate",
            "--sing-box-bin",
            rejecting.to_str().expect("rejecting checker path is UTF-8"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "sing-box configuration check failed",
        ));
    assert_eq!(
        snapshot(),
        before,
        "a rejected regeneration leaves every artifact and the active config unchanged"
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "regenerate",
            "--sing-box-bin",
            checker.to_str().expect("accepting checker path is UTF-8"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("regenerated and validated"));
    assert_ne!(
        snapshot(),
        before,
        "a passing regeneration atomically replaces artifacts and the active config"
    );
    for name in artifact_names {
        let contents = fs::read_to_string(artifacts.join(name)).expect("artifact is readable");
        if name == "subscription-base64-uri.txt" {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(contents.as_bytes())
                .expect("base64 URI subscription is valid standard Base64");
            let decoded = String::from_utf8(decoded).expect("base64 URI subscription is UTF-8");
            assert!(
                decoded.contains("www.apple.com"),
                "{name} carries the new canonical node field"
            );
        } else {
            assert!(
                contents.contains("www.apple.com"),
                "{name} carries the new canonical node field"
            );
        }
    }
    assert!(
        fs::read_to_string(&active)
            .expect("active configuration is readable")
            .contains("www.apple.com"),
        "the active configuration follows the regenerated server configuration"
    );
}

#[test]
fn configuration_initialization_checks_generated_sing_box_config_before_persisting() {
    let unchecked = TempDir::new().expect("temporary root is created");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            unchecked.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "direct",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--protocol",
            "vmess-websocket",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("require --sing-box-bin"));
    assert!(!unchecked.path().join("etc/sbctl/config.toml").exists());

    let fixture = TempDir::new().expect("temporary root is created");
    let checker = sing_box_check_fixture(&fixture, true, &["vmess"]);
    let root = fixture.path().to_str().expect("fixture path is UTF-8");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "config",
            "init",
            "--mode",
            "direct",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--protocol",
            "vmess-websocket",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success();
    assert!(fixture.path().join("etc/sbctl/config.toml").is_file());

    let rejected = TempDir::new().expect("temporary root is created");
    let rejecting_checker = sing_box_check_fixture(&rejected, false, &[]);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            rejected.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "direct",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--protocol",
            "vmess-websocket",
            "--sing-box-bin",
            rejecting_checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "sing-box configuration check failed",
        ));
    assert!(!rejected.path().join("etc/sbctl/config.toml").exists());
}

#[test]
fn configuration_validation_does_not_echo_a_secret_from_a_malformed_file() {
    let fixture = TempDir::new().expect("temporary root is created");
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    fs::create_dir_all(config_path.parent().expect("config path has a parent"))
        .expect("configuration directory is created");
    let secret = "a-very-sensitive-subscription-credential";
    fs::write(
        &config_path,
        format!("subscription_credential = \"{secret}\"\nnot valid TOML"),
    )
    .expect("malformed configuration is written");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "validate",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "could not parse deployment configuration",
        ))
        .stderr(predicate::str::contains(secret).not());
}

#[test]
fn configuration_init_defaults_the_refresh_and_client_display_timezones() {
    let fixture = TempDir::new().expect("temporary root is created");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is persisted");
    assert!(config.contains("accounting_timezone = \"America/Los_Angeles\""));
    assert!(config.contains("client_display_timezone = \"Asia/Shanghai\""));
}

#[test]
fn config_wizard_with_empty_answers_leaves_an_existing_deployment_unchanged() {
    let fixture = TempDir::new().expect("temporary root is created");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&config_path).expect("configuration is readable");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
        ])
        .write_stdin("\n".repeat(25))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "deployment configuration is unchanged",
        ));

    assert_eq!(
        fs::read(&config_path).expect("configuration remains readable"),
        before,
        "empty answers must keep every current value"
    );
}

#[test]
fn config_wizard_cancelled_leaves_the_existing_deployment_unchanged() {
    let fixture = TempDir::new().expect("temporary root is created");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&config_path).expect("configuration is readable");
    let mut answers = vec![String::new(); 17];
    answers[1] = "198.51.100.9".into();
    answers.push("n".into());
    let input = answers.join("\n") + "\n";

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
        ])
        .write_stdin(input)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "configuration wizard cancelled; the existing deployment is unchanged",
        ));

    assert_eq!(
        fs::read(&config_path).expect("configuration remains readable"),
        before,
        "an unconfirmed summary must not change the deployment"
    );
}

#[test]
fn config_wizard_without_input_aborts_without_changing_the_deployment() {
    let fixture = TempDir::new().expect("temporary root is created");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&config_path).expect("configuration is readable");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
        ])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("wizard input ended"));

    assert_eq!(
        fs::read(&config_path).expect("configuration remains readable"),
        before,
        "an interrupted wizard must not change the deployment"
    );
}

#[test]
fn config_wizard_rejects_an_ambiguous_dst_anchored_reset_before_committing() {
    let fixture = TempDir::new().expect("temporary root is created");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&config_path).expect("configuration is readable");
    let mut answers = vec![String::new(); 18];
    answers[14] = "America/New_York".into();
    answers[16] = "anchored-month".into();
    answers[17] = "2024-11-03T01:30".into();
    answers.push("y".into());
    let input = answers.join("\n") + "\n";

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
        ])
        .write_stdin(input)
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "anchored reset time is ambiguous in the accounting timezone",
        ));

    assert_eq!(
        fs::read(&config_path).expect("configuration remains readable"),
        before,
        "a DST-ambiguous schedule must be rejected before any commit"
    );
}

#[test]
fn config_wizard_commits_a_timezone_change_and_establishes_new_accounting_state() {
    let fixture = TempDir::new().expect("temporary root is created");
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    write_systemctl_fixture(&fixture, true);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--accounting-timezone",
            "UTC",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "accounting-reset",
        ])
        .assert()
        .success();
    let state_path = fixture.path().join("var/lib/sbctl/state.json");
    let state_before = fs::read_to_string(&state_path).expect("state is established");
    assert!(
        state_before.contains("+00:00"),
        "the initial UTC period is established"
    );

    let mut answers = vec![String::new(); 17];
    answers[14] = "Asia/Tokyo".into();
    answers.push("y".into());
    let input = answers.join("\n") + "\n";
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
        ])
        .write_stdin(input)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "deployment configuration committed",
        ))
        .stdout(predicate::str::contains("防火墙端口核对"))
        .stdout(predicate::str::contains("sudo ufw allow 2080/tcp"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is committed");
    assert!(config.contains("accounting_timezone = \"Asia/Tokyo\""));
    let state_after = fs::read_to_string(&state_path).expect("new state is established");
    assert!(
        state_after.contains("+09:00"),
        "changing the accounting timezone establishes a new accounting state"
    );
    assert_ne!(state_after, state_before);
}

#[test]
fn config_wizard_creates_a_new_deployment_with_secure_defaults() {
    let fixture = TempDir::new().expect("temporary root is created");
    fs::create_dir_all(fixture.path().join("proc/net")).expect("route directory is created");
    fs::write(
        fixture.path().join("proc/net/route"),
        "Iface\tDestination\tGateway\tFlags\nens3\t00000000\t00000000\t0003\n",
    )
    .expect("route fixture is written");
    fs::create_dir_all(fixture.path().join("sys/class/net/ens3"))
        .expect("interface fixture is created");
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );
    let mut answers = vec![String::new(); 22];
    answers[1] = "sub.example.test".into();
    // Confirm the no-email ACME path (the wizard asks for an email and, when
    // it is empty, requires an explicit confirmation).
    answers[4] = "y".into();
    answers[17] = "www.cloudflare.com".into();
    // Decline the fresh-deployment client-template offer, then confirm the
    // summary on the following line.
    answers[19] = "n".into();
    answers[20] = "n".into();
    answers[21] = "y".into();
    let input = answers.join("\n") + "\n";

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .write_stdin(input)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "deployment configuration committed",
        ))
        .stdout(predicate::str::contains("防火墙端口核对"))
        .stdout(predicate::str::contains("sudo ufw allow 80/tcp"))
        .stdout(predicate::str::contains("sudo ufw allow 443/tcp"))
        .stdout(predicate::str::contains("# vless-reality"))
        .stdout(predicate::str::contains("# vmess-websocket"))
        .stdout(predicate::str::contains("# hysteria2"))
        .stdout(predicate::str::contains("# tuic"))
        .stdout(predicate::str::contains("# anytls"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("a fresh wizard deployment is committed");
    assert!(config.contains("subscription_mode = \"direct\""));
    assert!(config.contains("subscription_host = \"sub.example.test\""));
    assert!(config.contains("interface = \"ens3\""));
    assert!(config.contains("accounting_timezone = \"America/Los_Angeles\""));
    assert!(config.contains("client_display_timezone = \"Asia/Shanghai\""));
    assert!(config.contains("accounting_policy = \"natural-month\""));
    for protocol in [
        "vless-reality",
        "vmess-websocket",
        "hysteria2",
        "tuic",
        "anytls",
    ] {
        assert!(
            config.contains(protocol),
            "{protocol} is enabled by default"
        );
    }
}

#[cfg(unix)]
#[test]
fn client_template_switch_changes_only_client_artifacts_without_restarting_or_resetting_accounting()
{
    let fixture = TempDir::new().expect("temporary root is created");
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    // A failing systemctl stub proves this client-only operation never calls it.
    write_command_fixture(
        &fixture,
        "usr/bin/systemctl",
        1,
        "unexpected systemctl call",
    );
    seed_ip_fallback(fixture.path().to_str().unwrap());
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            fixture.path().to_str().unwrap(),
            "accounting-reset",
        ])
        .assert()
        .success();
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    let state_path = fixture.path().join("var/lib/sbctl/state.json");
    let state_before = fs::read(&state_path).expect("accounting state exists");
    let server_path = fixture
        .path()
        .join("var/lib/sbctl/artifacts/sing-box-server.json");
    let mut server_before = fs::read(&server_path).expect("server artifact exists");
    // Simulate retained bytes from a prior release rather than an already
    // synchronized deployment. A preset must not reconcile server drift.
    server_before.extend_from_slice(b"\n");
    fs::write(&server_path, &server_before).unwrap();
    let active_path = fixture.path().join("etc/sing-box/config.json");
    fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    let mut active_before: serde_json::Value = serde_json::from_slice(&server_before).unwrap();
    active_before["log"]["level"] = serde_json::json!("warn");
    let active_before = serde_json::to_vec_pretty(&active_before).unwrap();
    fs::write(&active_path, &active_before).unwrap();
    let bare_before: Vec<_> = [
        "subscription-sing-box.json",
        "subscription-uri.txt",
        "subscription-base64-uri.txt",
        "subscription-shadowrocket.txt",
    ]
    .into_iter()
    .map(|name| {
        let path = fixture.path().join("var/lib/sbctl/artifacts").join(name);
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n");
        fs::write(&path, &bytes).unwrap();
        (path, bytes)
    })
    .collect();
    let full_path = fixture
        .path()
        .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json");
    let full_before = fs::read(&full_path).expect("client artifact exists");

    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            fixture.path().to_str().unwrap(),
            "config",
            "template",
            "compact",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("VPS 代理服务未重启"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml")).unwrap();
    assert!(config.contains("client_template = \"compact\""));
    assert_eq!(fs::read(&state_path).unwrap(), state_before);
    assert_eq!(fs::read(&server_path).unwrap(), server_before);
    assert_eq!(fs::read(&active_path).unwrap(), active_before);
    for (path, bytes) in bare_before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert_ne!(
        fs::read(&full_path).unwrap(),
        full_before,
        "selected template changes the client artifact"
    );
}

#[cfg(unix)]
#[test]
fn client_template_switch_rolls_back_config_and_artifacts_for_invalid_override_or_failed_kernel_check()
 {
    for reject_by_override in [true, false] {
        let fixture = TempDir::new().expect("temporary root is created");
        seed_ip_fallback(fixture.path().to_str().unwrap());
        let config_path = fixture.path().join("etc/sbctl/config.toml");
        let config_before = fs::read(&config_path).unwrap();
        let artifact_dir = fixture.path().join("var/lib/sbctl/artifacts");
        let artifact_before = fs::read_dir(&artifact_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        if reject_by_override {
            let overrides = fixture.path().join("etc/sbctl/overrides");
            fs::create_dir_all(&overrides).unwrap();
            fs::write(overrides.join("sing-box-override.json"), r#"{"route":{"rules":[{"domain_suffix":["example.test"],"outbound":"missing-group"}]}}"#).unwrap();
        }
        let checker = sing_box_check_fixture(&fixture, reject_by_override, &["vless"]);
        Command::cargo_bin("sbctl")
            .unwrap()
            .args([
                "--root",
                fixture.path().to_str().unwrap(),
                "config",
                "template",
                "compact",
                "--sing-box-bin",
                checker.to_str().unwrap(),
            ])
            .assert()
            .failure();
        assert_eq!(fs::read(&config_path).unwrap(), config_before);
        let after = fs::read_dir(&artifact_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            after, artifact_before,
            "failed validation preserves every generated artifact"
        );
    }
}

#[test]
fn config_wizard_output_does_not_leak_credentials() {
    let fixture = TempDir::new().expect("temporary root is created");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
    let credential = read_subscription_credential(&fixture);
    let proxy_uuid = read_vless_uuid(&fixture);
    let mut answers = vec![String::new(); 17];
    answers[1] = "198.51.100.9".into();
    answers.push("n".into());
    let input = answers.join("\n") + "\n";

    let output = Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "config",
            "wizard",
        ])
        .write_stdin(input)
        .output()
        .expect("wizard output is captured");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for secret in [&credential, &proxy_uuid] {
        assert!(!stdout.contains(secret), "stdout must not expose {secret}");
        assert!(!stderr.contains(secret), "stderr must not expose {secret}");
    }
    assert!(stdout.contains("subscription credential: [redacted]"));
}

#[cfg(unix)]
#[test]
fn override_edit_falls_back_to_nano_when_vim_is_missing() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TempDir::new().expect("temporary root is created");
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();

    // The checker greps the generated server config for the inbound's wire
    // type ("vless"), not the deployment protocol id.
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);

    // A minimal PATH that deliberately omits `vim`: the fixture scripts need
    // `touch`/`grep`, the chain must fall through to the fake `nano` when vim
    // cannot be spawned, and the real vim on the host must not launch (it
    // would block on the closed stdin of the test harness).
    let bin_dir = fixture.path().join("editor-bin");
    fs::create_dir_all(&bin_dir).expect("editor bin directory is created");
    for tool in ["touch", "grep"] {
        let source = Command::new("which")
            .arg(tool)
            .output()
            .expect("which runs")
            .stdout;
        let source = String::from_utf8(source)
            .expect("which output is UTF-8")
            .trim()
            .to_owned();
        std::os::unix::fs::symlink(&source, bin_dir.join(tool))
            .unwrap_or_else(|error| panic!("symlink {tool} from {source}: {error}"));
    }
    let marker = fixture.path().join("nano-invoked.marker");
    let nano = bin_dir.join("nano");
    fs::write(
        &nano,
        format!("#!/bin/sh\ntouch {}\nexit 0\n", marker.display()),
    )
    .expect("fake nano is written");
    fs::set_permissions(&nano, fs::Permissions::from_mode(0o700)).expect("fake nano is executable");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "config",
            "override",
            "edit",
            "sing-box",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("PATH", bin_dir.to_str().expect("bin path is UTF-8"))
        .assert()
        .success();

    assert!(
        marker.is_file(),
        "the fallback chain must have invoked nano after vim failed to spawn"
    );
    assert!(
        fixture
            .path()
            .join("etc/sbctl/overrides/sing-box-override.json")
            .is_file(),
        "the edit command must have seeded the sample override before editing"
    );
}

#[test]
fn rule_add_writes_the_list_and_regenerates_the_subscription() {
    let fixture = TempDir::new().expect("temporary root is created");
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule",
            "add",
            "proxy",
            "example.com",
            "DOMAIN,need-proxy.test",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success();

    let list = fs::read_to_string(fixture.path().join("etc/sbctl/rules/proxy.list"))
        .expect("the proxy list is written");
    assert!(
        list.contains("DOMAIN-SUFFIX,example.com") && list.contains("DOMAIN,need-proxy.test"),
        "a bare domain is stored as DOMAIN-SUFFIX: {list}"
    );

    // The generated artifact carries the operator's rule in front of the
    // built-in verdicts, which is the whole point of the verb.
    let full = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json"),
    )
    .expect("the full profile is regenerated");
    let value: serde_json::Value = serde_json::from_str(&full).expect("profile is JSON");
    assert_eq!(
        value["route"]["rules"][0]["domain_suffix"][0], "example.com",
        "the added rule must be the first verdict: {}",
        value["route"]["rules"]
    );
    assert_eq!(value["route"]["rules"][0]["outbound"], "代理");

    // Re-adding is a no-op with an explanation, not a duplicated line.
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule",
            "add",
            "proxy",
            "example.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("已存在"));
    let list = fs::read_to_string(fixture.path().join("etc/sbctl/rules/proxy.list"))
        .expect("the proxy list is written");
    assert_eq!(list.matches("example.com").count(), 1, "{list}");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "rule", "list", "proxy"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DOMAIN-SUFFIX,example.com"))
        .stdout(predicates::str::contains("need-proxy.test"));

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule",
            "remove",
            "proxy",
            "example.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("已从 proxy.list 移除"));
    let list = fs::read_to_string(fixture.path().join("etc/sbctl/rules/proxy.list"))
        .expect("the proxy list is written");
    assert!(
        !list.contains("example.com") && list.contains("need-proxy.test"),
        "only the named value leaves the list: {list}"
    );
}

#[test]
fn rule_add_refuses_a_match_type_the_generator_cannot_honour() {
    let fixture = TempDir::new().expect("temporary root is created");
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    seed_ip_fallback(root);

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "rule", "add", "direct", "GEOSITE,cn"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported match type"));
    assert!(
        !fixture.path().join("etc/sbctl/rules/direct.list").exists(),
        "a refused entry must not leave a partial file behind"
    );
}

#[test]
fn rule_changes_restore_sources_and_artifacts_after_a_failed_check() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, false, &[]);
    let list = fixture.path().join("etc/sbctl/rules/direct.list");
    let artifact = fixture
        .path()
        .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json");
    let prior_artifact = fs::read(&artifact).unwrap();
    let prior_config = fs::read(fixture.path().join("etc/sbctl/config.toml")).unwrap();
    for prior in [None, Some("# keep my comment\nDOMAIN,existing.example\n")] {
        if let Some(text) = prior {
            fs::create_dir_all(list.parent().unwrap()).unwrap();
            fs::write(&list, text).unwrap();
        }
        for (verb, entry) in [("add", "new.example"), ("remove", "existing.example")] {
            if prior.is_none() && verb == "remove" {
                continue;
            }
            Command::cargo_bin("sbctl")
                .unwrap()
                .args([
                    "--root",
                    root,
                    "rule",
                    verb,
                    "direct",
                    entry,
                    "--sing-box-bin",
                    checker.to_str().unwrap(),
                ])
                .assert()
                .failure();
            assert_eq!(fs::read_to_string(&list).ok().as_deref(), prior);
            assert_eq!(fs::read(&artifact).unwrap(), prior_artifact);
            assert_eq!(
                fs::read(fixture.path().join("etc/sbctl/config.toml")).unwrap(),
                prior_config
            );
        }
    }
}

#[test]
fn rule_add_cidr_can_be_listed_updated_and_removed() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    for (verb, entry) in [
        ("add", "IP-CIDR,2001:db8::/32"),
        ("add", "IP-CIDR,10.0.0.0/8"),
        ("remove", "IP-CIDR,2001:db8::/32"),
    ] {
        Command::cargo_bin("sbctl")
            .unwrap()
            .args([
                "--root",
                root,
                "rule",
                verb,
                "direct",
                entry,
                "--sing-box-bin",
                checker.to_str().unwrap(),
            ])
            .assert()
            .success();
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root, "rule", "list", "direct"])
            .assert()
            .success()
            .stdout(predicate::str::contains("no-resolve"));
    }
    let lists = sbctl::rule_list::RuleLists::load(fixture.path()).unwrap();
    assert_eq!(lists.direct.len(), 1);
    assert_eq!(lists.direct[0].value, "10.0.0.0/8");
}

#[test]
fn rule_add_handles_no_resolve_suffix_and_clash_policy_words() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);

    // Add entry with explicit ,no-resolve
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule",
            "add",
            "direct",
            "IP-CIDR,192.168.0.0/16,no-resolve",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Add entry with trailing Clash policy
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule",
            "add",
            "direct",
            "DOMAIN-SUFFIX,clash.example,DIRECT",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .assert()
        .success();

    let lists = sbctl::rule_list::RuleLists::load(fixture.path()).unwrap();
    assert_eq!(lists.direct.len(), 2);
    assert_eq!(lists.direct[0].value, "192.168.0.0/16");
    assert!(lists.direct[0].no_resolve);
    assert_eq!(lists.direct[1].value, "clash.example");

    // Remove entry using the exact line with ,no-resolve
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule",
            "remove",
            "direct",
            "IP-CIDR,192.168.0.0/16,no-resolve",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .assert()
        .success();

    let lists = sbctl::rule_list::RuleLists::load(fixture.path()).unwrap();
    assert_eq!(lists.direct.len(), 1);
    assert_eq!(lists.direct[0].value, "clash.example");
}

#[test]
fn rule_set_changes_keep_configuration_and_artifacts_after_a_failed_check() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let accepting = sing_box_check_fixture(&fixture, true, &["vless"]);
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule-set",
            "add",
            "game",
            "--url",
            "https://cdn.example/game.srs",
            "--sing-box-bin",
            accepting.to_str().unwrap(),
        ])
        .assert()
        .success();
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let prior = fs::read(&config_path).unwrap();
    let artifact_path = fixture
        .path()
        .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json");
    let artifact = fs::read(&artifact_path).unwrap();
    let rejecting = sing_box_check_fixture(&fixture, false, &[]);
    for args in [
        vec!["add", "new", "--url", "https://cdn.example/new.srs"],
        vec!["remove", "game"],
        vec!["interval", "12h"],
    ] {
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root, "rule-set"])
            .args(args)
            .args(["--sing-box-bin", rejecting.to_str().unwrap()])
            .assert()
            .failure();
        assert_eq!(fs::read(&config_path).unwrap(), prior);
        assert_eq!(fs::read(&artifact_path).unwrap(), artifact);
    }
}

#[cfg(unix)]
#[test]
fn rule_set_change_checks_the_merged_client_before_publishing() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let prior = fs::read(&config_path).unwrap();
    let checker = fixture.path().join("check-merged");
    fs::write(&checker, "#!/bin/sh\nif [ \"$1\" = version ]; then echo 'sing-box version 1.14.0'; exit 0; fi\nif grep -q 'game.srs' \"$3\"; then exit 1; fi\nexit 0\n").unwrap();
    fs::set_permissions(&checker, fs::Permissions::from_mode(0o700)).unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule-set",
            "add",
            "game",
            "--url",
            "https://cdn.example/game.srs",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .assert()
        .failure();
    assert_eq!(fs::read(&config_path).unwrap(), prior);
}

#[test]
fn concurrent_rule_adds_keep_both_sources_and_published_rules() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    let mut children = Vec::new();
    for domain in ["first.example", "second.example"] {
        children.push(
            std::process::Command::new(assert_cmd::cargo::cargo_bin!("sbctl"))
                .args([
                    "--root",
                    root,
                    "rule",
                    "add",
                    "direct",
                    domain,
                    "--sing-box-bin",
                    checker.to_str().unwrap(),
                ])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let lists = sbctl::rule_list::RuleLists::load(fixture.path()).unwrap();
    assert_eq!(lists.direct.len(), 2);
    let artifact = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json"),
    )
    .unwrap();
    assert!(artifact.contains("first.example") && artifact.contains("second.example"));
}

/// Shared setup: an ip-fallback deployment a verb can regenerate against.
fn seed_ip_fallback(root: &str) {
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "config",
            "init",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--protocol",
            "vless-reality",
            "--reality-decoy-sni",
            "www.cloudflare.com",
        ])
        .assert()
        .success();
}

#[test]
fn rule_set_add_persists_renders_and_refuses_an_uncompilable_url() {
    let fixture = TempDir::new().expect("temporary root is created");
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    let checker_path = checker.to_str().expect("checker path is UTF-8");

    // A text list is not a rule-set: registering one would hand every client a
    // file their core cannot parse, so it is refused with the verb that fits.
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule-set",
            "add",
            "game",
            "--url",
            "https://cdn.example/lists/game.list",
            "--sing-box-bin",
            checker_path,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("import-list"));

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule-set",
            "add",
            "game",
            "--url",
            "https://cdn.example/geo/game.srs",
            "--outbound",
            "reject",
            "--sing-box-bin",
            checker_path,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("已注册规则集 game -> reject"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is readable");
    assert!(config.contains("game.srs"), "{config}");
    let full = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json"),
    )
    .expect("the full profile regenerated with the registered set");
    assert!(full.contains("\"tag\": \"game\""), "{full}");
    let clash = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-clash.yaml"),
    )
    .expect("the clash artifact regenerated too");
    assert!(
        !clash.contains("RULE-SET,game,REJECT") && !clash.contains("game.srs"),
        "the clash side must never try to load a sing-box binary set"
    );

    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule-set",
            "add",
            "game-mihomo",
            "--url",
            "https://cdn.example/geo/game.mrs",
            "--outbound",
            "reject",
            "--behavior",
            "ipcidr",
            "--sing-box-bin",
            checker_path,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("仅用于 Mihomo"));
    let clash = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-clash.yaml"),
    )
    .unwrap();
    assert!(clash.contains("RULE-SET,game-mihomo,REJECT,no-resolve"));
    assert!(clash.contains("game.mrs"));
    assert!(clash.contains("behavior: ipcidr"));
    let full = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json"),
    )
    .unwrap();
    assert!(!full.contains("game.mrs") && !full.contains("game-mihomo"));

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "rule-set", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("game"))
        .stdout(predicate::str::contains("刷新间隔: 1d"));

    // Re-registering the same name re-points it rather than declaring it twice.
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule-set",
            "add",
            "game",
            "--url",
            "https://cdn.example/other/game.srs",
            "--sing-box-bin",
            checker_path,
        ])
        .assert()
        .success();
    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is readable");
    assert_eq!(
        config.matches("name = \"game\"").count(),
        1,
        "a repeated add must update, not duplicate: {config}"
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule-set",
            "interval",
            "12h",
            "--sing-box-bin",
            checker_path,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("12h"));
    let full = fs::read_to_string(
        fixture
            .path()
            .join("var/lib/sbctl/artifacts/subscription-sing-box-full.json"),
    )
    .expect("profile regenerated");
    assert!(full.contains("\"update_interval\": \"12h\""), "{full}");

    // A non-duration argument is refused rather than shipped to every client.
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "rule-set", "interval", "soon"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("不是合法时长"));

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "rule-set",
            "remove",
            "game",
            "--sing-box-bin",
            checker_path,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("已移除规则集 game"));
    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is readable");
    assert!(!config.contains("game.srs"), "{config}");
}

#[test]
fn invalid_rule_set_intervals_leave_configuration_and_artifacts_untouched() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&config_path).unwrap();
    let artifact = fixture
        .path()
        .join("var/lib/sbctl/artifacts/subscription-clash.yaml");
    let artifact_before = fs::read(&artifact).unwrap();
    for input in ["18446744073709551615d", "9223372037s", "0s", "1秒"] {
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root, "rule-set", "interval", input])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("不是合法时长"));
        assert_eq!(fs::read(&config_path).unwrap(), before);
        assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
    }
    // A manually edited value follows the same validation path as CLI input.
    let text = String::from_utf8(before).unwrap().replace(
        "client_rule_set_update_interval = \"1d\"",
        "client_rule_set_update_interval = \"18446744073709551615d\"",
    );
    fs::write(&config_path, text).unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "config", "validate"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("positive duration"));
}

#[test]
fn fake_ip_filter_add_honours_all_domain_matchers_and_rejects_cidr() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule",
            "add",
            "fakeip-filter",
            "DOMAIN,exact.example",
            "DOMAIN-KEYWORD,music",
            "DOMAIN-SUFFIX,suffix.example",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .assert()
        .success();
    let artifacts = fixture.path().join("var/lib/sbctl/artifacts");
    for entry in fs::read_dir(&artifacts).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        if name.starts_with("subscription-sing-box-") && name.ends_with(".json") {
            let config: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let dns = config["dns"].to_string();
            for expected in ["exact.example", "music", "suffix.example"] {
                assert!(dns.contains(expected), "{name} misses {expected}");
            }
        }
    }
    let source = fixture.path().join("etc/sbctl/rules/fakeip-filter.list");
    let before = fs::read(&source).unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "rule",
            "add",
            "fakeip-filter",
            "IP-CIDR,10.0.0.0/8",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fake-ip"));
    assert_eq!(fs::read(source).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn an_open_wizard_cannot_restore_rotated_or_revoked_subscription_credentials() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::{Command as ProcessCommand, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    for revoke_named in [false, true] {
        let fixture = TempDir::new().unwrap();
        let root = fixture.path().to_str().unwrap();
        seed_ip_fallback(root);
        write_systemctl_fixture(&fixture, true);
        if revoke_named {
            Command::cargo_bin("sbctl")
                .unwrap()
                .args(["--root", root, "credential", "add", "--name", "phone"])
                .assert()
                .success();
        }
        let mut wizard = ProcessCommand::new(assert_cmd::cargo::cargo_bin("sbctl"))
            .args(["--root", root, "config", "wizard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = wizard.stdout.take().unwrap();
        let (ready, received) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut output = String::new();
            reader.read_line(&mut output).unwrap();
            ready.send(()).unwrap();
            reader.read_to_string(&mut output).unwrap();
            output
        });
        received
            .recv_timeout(Duration::from_secs(10))
            .expect("wizard reads its initial snapshot");
        let operation = if revoke_named {
            vec!["credential", "revoke", "phone"]
        } else {
            vec!["credential", "rotate"]
        };
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root])
            .args(operation)
            .assert()
            .success();
        let config_path = fixture.path().join("etc/sbctl/config.toml");
        let latest = fs::read(&config_path).unwrap();
        let mut answers = vec![""; 17];
        answers[14] = "Asia/Tokyo";
        answers.push("y");
        wizard
            .stdin
            .take()
            .unwrap()
            .write_all((answers.join("\n") + "\n").as_bytes())
            .unwrap();
        let result = wizard.wait_with_output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("configuration changed while editing")
        );
        assert_eq!(
            fs::read(config_path).unwrap(),
            latest,
            "the latest credential state must survive"
        );
        reader.join().unwrap();
    }
}

#[test]
fn wizard_service_failure_restores_configuration_and_generated_artifacts() {
    let fixture = TempDir::new().unwrap();
    let root = fixture.path().to_str().unwrap();
    seed_ip_fallback(root);
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    write_systemctl_fixture(&fixture, false);
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&config_path).unwrap();
    let artifact_dir = fixture.path().join("var/lib/sbctl/artifacts");
    let artifacts: Vec<_> = fs::read_dir(&artifact_dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let mut answers = vec![""; 17];
    answers[2] = "203.0.113.8";
    answers.push("y");
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "config",
            "wizard",
            "--sing-box-bin",
            checker.to_str().unwrap(),
        ])
        .write_stdin(answers.join("\n") + "\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("configuration wizard failed"));
    assert_eq!(fs::read(config_path).unwrap(), before);
    for (path, bytes) in artifacts {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
