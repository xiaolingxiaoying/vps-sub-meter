//! `sbctl install`: what a fresh managed host gets written, what the ownership
//! marker records, and what an install refuses to touch.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[cfg(unix)]
use std::io::{Read, Write};

#[cfg(unix)]
use std::net::TcpListener;

#[cfg(unix)]
use std::thread;

use crate::fixture::{
    assert_existing_deployment_is_preserved, seed_service_accounts, sing_box_check_fixture,
    supported_systemd_host, write_os_release, write_systemctl_fixture,
    write_systemctl_health_failing_fixture, write_traffic_fixture,
};

// The Direct-mode certificate fixtures only exist for the Unix-only install
// tests: certbot, the live certificate and the group database are shell-level
// fakes with no Windows counterpart.
#[cfg(unix)]
use crate::fixture::{seed_live_certificate, write_managed_file};

#[test]
fn guided_install_uses_one_confirmed_wizard_and_installs_its_configuration() {
    let fixture = supported_systemd_host();
    fs::create_dir_all(fixture.path().join("proc/net")).expect("route directory is created");
    fs::write(
        fixture.path().join("proc/net/route"),
        "Iface\tDestination\tGateway\tFlags\nens3\t00000000\t00000000\t0003\n",
    )
    .expect("default route is written");
    fs::create_dir_all(fixture.path().join("sys/class/net/ens3"))
        .expect("default interface is created");
    write_systemctl_fixture(&fixture, true);
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );
    let mut answers = vec![String::new(); 25];
    answers[1] = "sub.example.test".into();
    answers[4] = "y".into();
    answers[17] = "www.cloudflare.com".into();
    // Decline the client-template offer, then confirm the summary.
    answers[23] = "n".into();
    answers[24] = "y".into();

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--guided",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
            "--no-start",
        ])
        .write_stdin(answers.join("\n") + "\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("确认提交以上配置？"))
        .stdout(predicate::str::contains(
            "subscription credential: [redacted]",
        ))
        .stdout(predicate::str::contains("安装完成"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("the confirmed guided configuration is installed");
    assert!(config.contains("subscription_host = \"sub.example.test\""));
    for protocol in [
        "vless-reality",
        "vmess-websocket",
        "hysteria2",
        "tuic",
        "anytls",
    ] {
        assert!(
            config.contains(protocol),
            "guided config includes {protocol}"
        );
    }
}

#[test]
fn cancelling_guided_install_leaves_no_managed_state() {
    let fixture = supported_systemd_host();
    fs::create_dir_all(fixture.path().join("proc/net")).expect("route directory is created");
    fs::write(
        fixture.path().join("proc/net/route"),
        "Iface\tDestination\tGateway\tFlags\nens3\t00000000\t00000000\t0003\n",
    )
    .expect("default route is written");
    fs::create_dir_all(fixture.path().join("sys/class/net/ens3"))
        .expect("default interface is created");
    let mut answers = vec![String::new(); 25];
    answers[1] = "sub.example.test".into();
    answers[4] = "y".into();
    answers[17] = "www.cloudflare.com".into();
    answers[23] = "n".into();
    answers[24] = "n".into();

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--guided",
        ])
        .write_stdin(answers.join("\n") + "\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "installation cancelled; the host was not changed",
        ));

    assert!(!fixture.path().join("etc/sbctl/config.toml").exists());
    assert!(!fixture.path().join("usr/local/bin/sing-box").exists());
}

#[test]
fn guided_install_refuses_an_existing_deployment_before_prompting() {
    let fixture = supported_systemd_host();
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
            "install",
            "--guided",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Existing deployment detected"))
        .stdout(predicate::str::contains("sbctl 配置向导").not());
}

#[cfg(unix)]
use crate::fixture::write_manifest_spec;

// The install flow ends by executing the downloaded kernel, and the fixture
// it downloads is the platform's config-check script: on Windows an
// extensionless file cannot be spawned as a Win32 process, so the full
// download-verify-install path is only exercisable on Unix.
#[cfg(unix)]
#[test]
fn install_with_a_signed_manifest_downloads_and_verifies_sing_box() {
    let fixture = supported_systemd_host();
    let sing_box_fixture = sing_box_check_fixture(&fixture, true, &["vless"]);
    let sing_box_contents = fs::read(&sing_box_fixture).expect("sing-box fixture is readable");

    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port is available");
    let port = listener.local_addr().expect("address is available").port();
    let served = sing_box_contents.clone();
    let server = thread::spawn(move || {
        for stream in listener.incoming().take(1) {
            let mut stream = stream.expect("client stream is accepted");
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                served.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&served);
        }
    });

    let manifest = fixture.path().join("release-manifest.json");
    write_manifest_spec(
        &manifest,
        1,
        "0.1.1",
        "https://example.test/sbctl",
        b"candidate sbctl",
        "1.12.0",
        &format!("http://127.0.0.1:{port}/sing-box"),
        &sing_box_contents,
        "1.12.0",
        "1.12.0",
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "203.0.113.7",
            "--http-port",
            "2080",
            "--interface",
            "ens3",
            "--disable-protocol",
            "vmess-websocket",
            "--disable-protocol",
            "hysteria2",
            "--disable-protocol",
            "tuic",
            "--disable-protocol",
            "anytls",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--manifest",
            manifest.to_str().expect("manifest path is UTF-8"),
            "--no-start",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("后续必做清单"));

    server.join().expect("byte server exits");
    assert_eq!(
        fs::read(fixture.path().join("usr/local/bin/sing-box")).expect("sing-box is installed"),
        sing_box_contents
    );
}

#[test]
fn install_reports_a_supported_systemd_fixture_as_ready() {
    let fixture = supported_systemd_host();

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("install preflight passed"));
}

#[test]
fn a_failed_install_keeps_the_management_binary_and_rolls_back_the_certificate_directory() {
    let fixture = supported_systemd_host();
    // The release installer always places the management binary before it runs
    // `sbctl install`, so a failed transaction must not delete it.
    let management_binary = fixture.path().join("usr/local/bin/sbctl");
    fs::create_dir_all(management_binary.parent().expect("binary has a parent"))
        .expect("fixture binary directory");
    fs::write(&management_binary, "installed by the bootstrap installer")
        .expect("fixture management binary");
    // A kernel whose `check` rejects the generated configuration fails the
    // install after the self-signed certificate has already been written.
    let rejecting_checker = sing_box_check_fixture(&fixture, false, &[]);

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            rejecting_checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("installation failed"));

    assert_eq!(
        fs::read_to_string(&management_binary).expect("the management binary survives"),
        "installed by the bootstrap installer",
        "a failed install must not remove the administrator's only CLI"
    );
    assert!(
        !fixture.path().join("var/lib/sbctl/certificates").exists(),
        "the certificate written before the failure belongs to the rolled-back transaction"
    );
    assert!(
        !fixture.path().join("etc/sbctl/config.toml").exists(),
        "a failed install must not leave a configuration behind"
    );
}

#[test]
fn a_failed_install_can_be_retried_without_manual_cleanup() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    let management_binary = fixture.path().join("usr/local/bin/sbctl");
    fs::create_dir_all(management_binary.parent().expect("binary has a parent"))
        .expect("fixture binary directory");
    fs::write(&management_binary, "installed by the bootstrap installer")
        .expect("fixture management binary");
    let rejecting_checker = sing_box_check_fixture(&fixture, false, &[]);
    let install_args = [
        "install",
        "--mode",
        "external-proxy",
        "--subscription-host",
        "sub.example.test",
        "--interface",
        "ens3",
        "--reality-decoy-sni",
        "www.cloudflare.com",
        "--no-start",
    ];

    let mut failing = Command::cargo_bin("sbctl").expect("sbctl binary is built");
    failing.args([
        "--root",
        fixture.path().to_str().expect("fixture path is UTF-8"),
    ]);
    failing.args(install_args);
    failing
        .args([
            "--sing-box-bin",
            rejecting_checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .code(2);

    // Preflight lists `var/lib/sbctl/certificates` as an existing deployment,
    // so a leftover certificate directory would refuse this retry.
    let accepting_checker = sing_box_check_fixture(&fixture, true, &["vless"]);
    let mut retry = Command::cargo_bin("sbctl").expect("sbctl binary is built");
    retry.args([
        "--root",
        fixture.path().to_str().expect("fixture path is UTF-8"),
    ]);
    retry.args(install_args);
    retry
        .args([
            "--sing-box-bin",
            accepting_checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("install preflight passed").not());

    assert!(
        fixture.path().join("etc/sbctl/config.toml").is_file(),
        "the retried install commits its configuration"
    );
}

#[test]
fn install_defaults_to_all_managed_protocols_writes_services_and_only_lists_firewall_ports() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
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
        .success()
        .stdout(predicate::str::contains(
            "启用协议: vless-reality, vmess-websocket, hysteria2, tuic, anytls",
        ))
        .stdout(predicate::str::contains("后续必做清单"))
        .stdout(predicate::str::contains("sudo ufw allow 80/tcp"))
        .stdout(predicate::str::contains("sudo ufw allow 443/tcp"))
        .stdout(predicate::str::contains("sbctl certificate obtain --email"));

    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("installation persists configuration");
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
    let sing_box_unit =
        fs::read_to_string(fixture.path().join("etc/systemd/system/sing-box.service"))
            .expect("sing-box unit is installed");
    let sbctl_unit = fs::read_to_string(fixture.path().join("etc/systemd/system/sbctl.service"))
        .expect("sbctl unit is installed");
    assert!(
        sing_box_unit
            .contains("ExecStart=/usr/local/bin/sing-box run -c /etc/sing-box/config.json")
    );
    assert!(sing_box_unit.contains("User=sing-box"));
    assert!(sing_box_unit.contains("Group=sing-box"));
    assert!(
        !sing_box_unit.contains("User=sbctl"),
        "the sing-box data plane runs under its own non-root account"
    );
    assert!(sbctl_unit.contains("User=sbctl"));
    assert!(
        sbctl_unit.contains("Requires=sbctl-http.socket"),
        "the Direct HTTPS service depends on the socket unit that owns 80/443"
    );
    let http_socket =
        fs::read_to_string(fixture.path().join("etc/systemd/system/sbctl-http.socket"))
            .expect("the Direct HTTPS socket unit is installed");
    assert!(http_socket.contains("ListenStream=80"));
    assert!(http_socket.contains("ListenStream=443"));
    let reset_timer = fs::read_to_string(
        fixture
            .path()
            .join("etc/systemd/system/sbctl-accounting-reset.timer"),
    )
    .expect("accounting reset timer is installed");
    let reset_service = fs::read_to_string(
        fixture
            .path()
            .join("etc/systemd/system/sbctl-accounting-reset.service"),
    )
    .expect("accounting reset service is installed");
    assert!(reset_timer.contains("Persistent=true"));
    assert!(reset_timer.contains("OnCalendar=minutely"));
    assert!(reset_service.contains("ExecStart=/usr/local/bin/sbctl accounting-reset"));
    assert!(reset_service.contains("User=sbctl"));
    assert!(fixture.path().join("etc/sing-box/config.json").is_file());
    assert!(!fixture.path().join("etc/ufw/user.rules").exists());

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "node",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("vless-reality: TCP"))
        .stdout(predicate::str::contains("hysteria2: UDP"))
        .stdout(predicate::str::contains("tuic: UDP"));
}

#[test]
fn install_writes_the_ownership_marker_after_services_start_and_pass_the_health_check() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    seed_service_accounts(&fixture);
    write_systemctl_fixture(&fixture, true);
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "external-proxy",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(fixture.path().join("var/lib/sbctl/ownership"))
            .expect("the ownership marker is written only after the health check"),
        "sbctl-managed-v1\n"
    );
}

#[test]
fn install_startup_failure_rolls_back_without_leaving_an_ownership_marker() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    seed_service_accounts(&fixture);
    write_systemctl_fixture(&fixture, false);
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "external-proxy",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("installation failed"));

    assert!(
        !fixture.path().join("var/lib/sbctl/ownership").exists(),
        "a failed installation must not leave an ownership marker"
    );
    assert!(!fixture.path().join("etc/sbctl/config.toml").exists());
    assert!(!fixture.path().join("usr/local/bin/sing-box").exists());
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sing-box.service")
            .exists()
    );
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sbctl.service")
            .exists()
    );
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sbctl-accounting-reset.timer")
            .exists()
    );
}

#[test]
fn install_health_check_failure_rolls_back_without_leaving_an_ownership_marker() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    seed_service_accounts(&fixture);
    write_systemctl_health_failing_fixture(&fixture);
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "external-proxy",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("installation failed"));

    assert!(
        !fixture.path().join("var/lib/sbctl/ownership").exists(),
        "a failed health check must not leave an ownership marker"
    );
    assert!(!fixture.path().join("etc/sbctl/config.toml").exists());
    assert!(!fixture.path().join("usr/local/bin/sing-box").exists());
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sbctl.service")
            .exists()
    );
}

#[test]
fn external_proxy_install_does_not_install_the_direct_https_socket() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    let checker = sing_box_check_fixture(&fixture, true, &["vless"]);

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "external-proxy",
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

    let sbctl_unit = fs::read_to_string(fixture.path().join("etc/systemd/system/sbctl.service"))
        .expect("sbctl unit is installed");
    assert!(
        !sbctl_unit.contains("sbctl-http.socket"),
        "an external reverse proxy keeps owning public 80/443"
    );
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sbctl-http.socket")
            .exists(),
        "no socket unit is installed outside Direct subscription mode"
    );
    assert!(
        fixture
            .path()
            .join("etc/systemd/system/sing-box.service")
            .is_file()
    );
}

#[test]
fn direct_install_writes_the_certbot_deploy_hook_but_external_proxy_does_not() {
    let direct = supported_systemd_host();
    write_traffic_fixture(&direct, 100, 200, "boot-a");
    let checker = sing_box_check_fixture(&direct, true, &["vless"]);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            direct.path().to_str().expect("fixture path is UTF-8"),
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
    let hook = direct
        .path()
        .join("etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook");
    assert!(
        hook.is_file(),
        "Direct install writes the Certbot deploy hook"
    );
    let hook_text = fs::read_to_string(&hook).expect("deploy hook is readable");
    assert!(hook_text.contains("sbctl certificate verify"));
    assert!(hook_text.contains("set -eu"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&hook)
            .expect("deploy hook has metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the deploy hook is executable");
    }

    let external = supported_systemd_host();
    write_traffic_fixture(&external, 100, 200, "boot-a");
    let checker = sing_box_check_fixture(&external, true, &["vless"]);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            external.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "external-proxy",
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
    assert!(
        !external
            .path()
            .join("etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook")
            .exists(),
        "External proxy mode never installs or touches the sbctl deploy hook"
    );
}

#[test]
fn install_rejects_an_existing_deployment_without_changing_its_configuration() {
    let fixture = supported_systemd_host();
    let configuration = fixture.path().join("etc/sing-box/config.json");
    fs::create_dir_all(configuration.parent().expect("config has a parent"))
        .expect("configuration directory is created");
    fs::write(&configuration, "preserve this deployment").expect("configuration is written");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Existing deployment detected"));

    assert_eq!(
        fs::read_to_string(configuration).expect("preflight leaves the config intact"),
        "preserve this deployment"
    );
}

#[test]
fn install_rejects_an_existing_sing_box_binary_without_changing_it() {
    let fixture = supported_systemd_host();
    let binary = fixture.path().join("opt/sing-box/sing-box");
    fs::create_dir_all(binary.parent().expect("binary has a parent"))
        .expect("binary directory is created");
    fs::write(&binary, "existing sing-box binary").expect("binary is written");

    assert_existing_deployment_is_preserved(&fixture, &binary, "existing sing-box binary");
}

#[test]
fn install_rejects_an_existing_sing_box_service_without_changing_it() {
    let fixture = supported_systemd_host();
    let service = fixture.path().join("etc/systemd/system/proxy.service");
    fs::create_dir_all(service.parent().expect("service has a parent"))
        .expect("systemd unit directory is created");
    let contents = "[Service]\nExecStart=/opt/sing-box/sing-box run\n";
    fs::write(&service, contents).expect("service is written");

    assert_existing_deployment_is_preserved(&fixture, &service, contents);
}

#[test]
fn install_explains_when_the_platform_is_not_supported() {
    let fixture = TempDir::new().expect("temporary root is created");
    write_os_release(&fixture, "ID=alpine\nVERSION_ID=3.20\n");
    fs::create_dir_all(fixture.path().join("run/systemd/system"))
        .expect("systemd runtime directory is created");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "requires Debian or Ubuntu with systemd",
        ));
}

/// The gap this closes: a Direct install used to come up with an HTTPS
/// subscription that dropped every handshake until the operator discovered the
/// missing step and ran `certificate obtain` a second time. The install
/// transaction now performs it, so a first install is actually usable.
/// The fixture counterpart of a root host's group database: with the
/// certificate group already present and populated, `ensure_certificate_group`
/// has no reason to invoke the real `groupadd`/`usermod`.
#[cfg(unix)]
fn seed_certificate_group(fixture: &TempDir) {
    write_managed_file(fixture, "etc/group", b"sbctl-cert:x:997:sbctl,sing-box\n");
}

#[cfg(unix)]
#[test]
fn direct_install_obtains_and_pins_the_subscription_certificate_in_transaction() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    seed_service_accounts(&fixture);
    seed_certificate_group(&fixture);
    write_systemctl_fixture(&fixture, true);
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );
    // A live certificate already exists, so the fake certbot only has to record
    // that it was asked; `obtain` then validates and pins what it finds.
    seed_live_certificate(&fixture, &["sub.example.test"]);
    let certbot = fixture.path().join("usr/bin/certbot");
    fs::create_dir_all(certbot.parent().expect("certbot has a parent"))
        .expect("certbot directory is created");
    fs::write(
        &certbot,
        "#!/bin/sh\nroot=$(CDPATH= cd -- \"$(dirname -- \"$0\")/../..\" && pwd)\nprintf '%s\n' \"$*\" > \"$root/.certbot-args\"\nexit 0\n",
    )
    .expect("certbot fixture is written");
    fs::set_permissions(&certbot, fs::Permissions::from_mode(0o700))
        .expect("certbot fixture is executable");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "direct",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("订阅证书已签发并固定"))
        .stdout(predicate::str::contains(
            "订阅证书已随安装签发并固定，无需再操作",
        ));

    let pinned = fixture
        .path()
        .join("var/lib/sbctl/certificates/sub.example.test");
    assert!(
        pinned.join("fullchain.pem").is_file() && pinned.join("privkey.pem").is_file(),
        "the certificate obtained during install must be pinned for the daemon"
    );
    let invoked =
        fs::read_to_string(fixture.path().join(".certbot-args")).expect("certbot was invoked");
    assert!(
        invoked.contains("certonly --webroot")
            && invoked.contains("--domain sub.example.test")
            && invoked.contains("--non-interactive"),
        "the install must ask certbot non-interactively for the subscription host: {invoked}"
    );
}

/// A certificate failure is an outstanding to-do, not a reason to tear a
/// healthy deployment down: DNS is frequently still propagating on a fresh VPS.
#[cfg(unix)]
#[test]
fn direct_install_reports_the_certificate_step_when_certbot_is_missing() {
    let fixture = supported_systemd_host();
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    seed_service_accounts(&fixture);
    seed_certificate_group(&fixture);
    write_systemctl_fixture(&fixture, true);
    let checker = sing_box_check_fixture(
        &fixture,
        true,
        &["vless", "vmess", "hysteria2", "tuic", "anytls"],
    );

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            fixture.path().to_str().expect("fixture path is UTF-8"),
            "install",
            "--mode",
            "direct",
            "--subscription-host",
            "sub.example.test",
            "--interface",
            "ens3",
            "--reality-decoy-sni",
            "www.cloudflare.com",
            "--sing-box-bin",
            checker.to_str().expect("checker path is UTF-8"),
        ])
        .assert()
        .success()
        // The install still completes and claims ownership.
        .stdout(predicate::str::contains("安装完成"))
        // …but the certificate is named as the outstanding step, with reason.
        .stdout(predicate::str::contains("订阅证书未签发（certbot 不可用"))
        .stdout(predicate::str::contains("sbctl certificate obtain --email"));

    assert!(
        fixture.path().join("var/lib/sbctl/ownership").is_file(),
        "a missing certificate must not roll back an otherwise healthy install"
    );
}
