//! `sbctl config`: initialization, validation, regeneration and the
//! interactive wizard, and the accounting anchors they persist.

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
    let mut answers = vec![String::new(); 25];
    answers[1] = "sub.example.test".into();
    // Confirm the no-email ACME path (the wizard asks for an email and, when
    // it is empty, requires an explicit confirmation).
    answers[4] = "y".into();
    answers[17] = "www.cloudflare.com".into();
    // Decline the fresh-deployment client-template offer, then confirm the
    // summary on the following line.
    answers[23] = "n".into();
    answers[24] = "y".into();
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
    assert_eq!(value["route"]["rules"][0]["outbound"], "节点选择");

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
        clash.contains("RULE-SET,game,REJECT"),
        "the clash side must route the same set"
    );

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
