use crate::fixture::{
    command_fixture, free_high_tcp_port, initialize_uninstall_fixture, seed_live_certificate,
    write_systemctl_fixture,
};
use assert_cmd::Command;
use predicates::prelude::*;
use sbctl::config::{
    CertificateMode, DeploymentConfig, DeploymentStore, ManagedProtocol, SubscriptionMode,
};
use std::{fs, path::PathBuf};
use tempfile::TempDir;

fn mode_fixture(mode: SubscriptionMode) -> (TempDir, PathBuf) {
    let fixture = TempDir::new().unwrap();
    let binary = command_fixture(&fixture, "usr/local/bin/sing-box", true, &[]);
    let host = if mode == SubscriptionMode::IpFallback {
        "127.0.0.1"
    } else {
        "sub.example.test"
    };
    let mut config = DeploymentConfig::new(
        mode.clone(),
        host.into(),
        None,
        (mode == SubscriptionMode::IpFallback).then(free_high_tcp_port),
        "ens3".into(),
        vec![ManagedProtocol::VlessReality, ManagedProtocol::Anytls],
        Some("www.cloudflare.com".into()),
    )
    .unwrap();
    config.certificate_mode = CertificateMode::SelfSigned;
    if mode == SubscriptionMode::ExternalProxy {
        config.subscription_listen_port = Some(free_high_tcp_port());
    }
    let store = DeploymentStore::new(fixture.path());
    let artifacts = sbctl::subscription::generated_artifacts(&config, fixture.path()).unwrap();
    let references = artifacts
        .iter()
        .map(|(name, body)| (name.clone(), body.as_bytes()))
        .collect::<Vec<_>>();
    store
        .initialize_with_artifacts(&config, &references)
        .unwrap();
    let server = artifacts
        .iter()
        .find(|(name, _)| name == "sing-box-server.json")
        .unwrap();
    sbctl::lifecycle::install_units(&store, &server.1, mode == SubscriptionMode::Direct).unwrap();
    sbctl::lifecycle::write_ownership_marker(&store).unwrap();
    fs::write(
        fixture.path().join("var/lib/sbctl/state.json"),
        "preserve accounting bytes",
    )
    .unwrap();
    fs::create_dir_all(fixture.path().join("etc/sbctl/overrides")).unwrap();
    fs::write(
        fixture
            .path()
            .join("etc/sbctl/overrides/sing-box-server.json"),
        "{}",
    )
    .unwrap();
    write_systemctl_fixture(&fixture, true);
    seed_live_certificate(&fixture, &["sub.example.test"]);
    (fixture, binary)
}

fn switch(fixture: &TempDir, binary: &std::path::Path, mode: &str, port: u16) -> Command {
    let mut command = Command::cargo_bin("sbctl").unwrap();
    command.args([
        "--root",
        fixture.path().to_str().unwrap(),
        "config",
        "switch-mode",
        "--mode",
        mode,
        "--sing-box-bin",
        binary.to_str().unwrap(),
    ]);
    if mode == "ip-fallback" {
        command.args([
            "--subscription-host",
            "127.0.0.1",
            "--http-port",
            &port.to_string(),
            "--certificate-mode",
            "self-signed",
        ]);
    } else {
        command.args(["--subscription-host", "sub.example.test"]);
        if mode == "external-proxy" {
            command.args(["--listen-port", &port.to_string()]);
        }
    }
    command
}

#[test]
fn all_six_mode_transitions_preserve_credentials_accounting_and_overrides() {
    for old in [
        SubscriptionMode::Direct,
        SubscriptionMode::ExternalProxy,
        SubscriptionMode::IpFallback,
    ] {
        for (name, target) in [
            ("direct", SubscriptionMode::Direct),
            ("external-proxy", SubscriptionMode::ExternalProxy),
            ("ip-fallback", SubscriptionMode::IpFallback),
        ] {
            if old == target {
                continue;
            }
            let (fixture, binary) = mode_fixture(old.clone());
            let store = DeploymentStore::new(fixture.path());
            let before = store.load().unwrap();
            switch(&fixture, &binary, name, free_high_tcp_port())
                .assert()
                .success();
            let after = store.load().unwrap();
            assert_eq!(after.subscription_mode, target);
            assert_eq!(
                after.subscription_credential,
                before.subscription_credential
            );
            assert_eq!(after.vless_reality, before.vless_reality);
            assert_eq!(after.anytls, before.anytls);
            assert_eq!(
                fs::read_to_string(fixture.path().join("var/lib/sbctl/state.json")).unwrap(),
                "preserve accounting bytes"
            );
            assert_eq!(
                fs::read_to_string(
                    fixture
                        .path()
                        .join("etc/sbctl/overrides/sing-box-server.json")
                )
                .unwrap(),
                "{}"
            );
            let unit = fs::read_to_string(fixture.path().join("etc/systemd/system/sbctl.service"))
                .unwrap();
            assert_eq!(
                unit.contains("Requires=sbctl-http.socket"),
                target == SubscriptionMode::Direct
            );
            assert_eq!(
                fixture
                    .path()
                    .join("etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook")
                    .exists(),
                target == SubscriptionMode::Direct
            );
            let artifacts = fs::read_to_string(
                fixture
                    .path()
                    .join("var/lib/sbctl/artifacts/subscription-uri.txt"),
            )
            .unwrap();
            assert!(artifacts.contains(&after.subscription_host));
        }
    }
}

#[test]
fn switching_back_to_direct_repins_a_renewed_live_certificate() {
    let (fixture, binary) = mode_fixture(SubscriptionMode::Direct);
    let store = DeploymentStore::new(fixture.path());
    let config = store.load().unwrap();
    let initial = sbctl::certificate::load(&store, &config).unwrap();
    sbctl::certificate::pin(&store, &config, &initial).unwrap();
    let pinned = fixture
        .path()
        .join("var/lib/sbctl/certificates/sub.example.test/fullchain.pem");
    let old_pinned = fs::read(&pinned).unwrap();

    switch(&fixture, &binary, "external-proxy", free_high_tcp_port())
        .assert()
        .success();
    seed_live_certificate(&fixture, &["sub.example.test"]);
    let new_live = fs::read(
        fixture
            .path()
            .join("etc/letsencrypt/live/sub.example.test/fullchain.pem"),
    )
    .unwrap();
    assert_ne!(old_pinned, new_live, "the renewal must produce new bytes");

    switch(&fixture, &binary, "direct", free_high_tcp_port())
        .assert()
        .success();
    assert_eq!(
        fs::read(&pinned).unwrap(),
        new_live,
        "switching back to Direct must pin the renewed live certificate, not keep the stale copy"
    );
}

#[test]
fn mode_restart_failure_restores_files_and_removes_new_certificates() {
    let (fixture, binary) = mode_fixture(SubscriptionMode::ExternalProxy);
    let paths = [
        "etc/sbctl/config.toml",
        "etc/sing-box/config.json",
        "etc/systemd/system/sbctl.service",
        "var/lib/sbctl/artifacts/subscription-uri.txt",
        "var/lib/sbctl/state.json",
    ];
    let before = paths
        .iter()
        .map(|path| fs::read(fixture.path().join(path)).unwrap())
        .collect::<Vec<_>>();
    write_systemctl_fixture(&fixture, false);
    switch(&fixture, &binary, "direct", free_high_tcp_port())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("恢复"))
        .stdout(predicate::str::contains("committed").not());
    for (path, bytes) in paths.iter().zip(before) {
        assert_eq!(fs::read(fixture.path().join(path)).unwrap(), bytes);
    }
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sbctl-http.socket")
            .exists()
    );
    assert!(
        !fixture
            .path()
            .join("var/lib/sbctl/certificates/sub.example.test/fullchain.pem")
            .exists()
    );
}

#[test]
fn mode_validation_and_kernel_failure_leave_config_unchanged() {
    let (fixture, binary) = mode_fixture(SubscriptionMode::ExternalProxy);
    let store = DeploymentStore::new(fixture.path());
    let mut config = store.load().unwrap();
    config.certificate_mode = CertificateMode::Domain;
    store.replace(&config).unwrap();
    let path = fixture.path().join("etc/sbctl/config.toml");
    let before = fs::read(&path).unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            fixture.path().to_str().unwrap(),
            "config",
            "switch-mode",
            "--mode",
            "ip-fallback",
            "--subscription-host",
            "127.0.0.1",
            "--http-port",
            &free_high_tcp_port().to_string(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("显式指定"));
    assert_eq!(fs::read(&path).unwrap(), before);
    let bad = command_fixture(&fixture, "usr/local/bin/bad-kernel", false, &[]);
    switch(&fixture, &bad, "ip-fallback", free_high_tcp_port())
        .assert()
        .code(2);
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(binary.is_file());
}

#[test]
fn uninstall_removes_opt_in_units_and_refuses_foreign_email_units() {
    for purge in [false, true] {
        let fixture = TempDir::new().unwrap();
        initialize_uninstall_fixture(&fixture);
        write_systemctl_fixture(&fixture, true);
        sbctl::lifecycle::set_credential_rotate_timer(fixture.path(), true).unwrap();
        sbctl::lifecycle::set_email_timer(fixture.path(), true).unwrap();
        let unit = fs::read_to_string(
            fixture
                .path()
                .join("etc/systemd/system/sbctl-credential-rotate.service"),
        )
        .unwrap();
        assert!(unit.contains("User=root"));
        let mut command = Command::cargo_bin("sbctl").unwrap();
        command.args(["--root", fixture.path().to_str().unwrap(), "uninstall"]);
        if purge {
            command.arg("--purge");
        }
        command.assert().success();
        for name in [
            "sbctl-credential-rotate.service",
            "sbctl-credential-rotate.timer",
            "sbctl-email.service",
            "sbctl-email.timer",
        ] {
            assert!(
                !fixture
                    .path()
                    .join("etc/systemd/system")
                    .join(name)
                    .exists()
            );
        }
    }
    let fixture = TempDir::new().unwrap();
    initialize_uninstall_fixture(&fixture);
    write_systemctl_fixture(&fixture, true);
    let path = fixture.path().join("etc/systemd/system/sbctl-email.timer");
    fs::write(&path, "[Unit]\nDescription=other administrator's timer\n").unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            fixture.path().to_str().unwrap(),
            "uninstall",
            "--purge",
        ])
        .assert()
        .code(2);
    assert!(path.exists());
    assert!(fixture.path().join("usr/local/bin/sbctl").exists());
}

#[test]
fn rotation_migration_never_enables_a_timer_or_installs_one_unasked() {
    let fixture = TempDir::new().unwrap();
    write_systemctl_fixture(&fixture, true);
    sbctl::lifecycle::migrate_credential_rotate_service(fixture.path()).unwrap();
    let path = fixture
        .path()
        .join("etc/systemd/system/sbctl-credential-rotate.service");
    assert!(!path.exists());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "[Unit]\nDescription=sbctl scheduled subscription credential rotation\n[Service]\nUser=sbctl\n").unwrap();
    sbctl::lifecycle::migrate_credential_rotate_service(fixture.path()).unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("User=root"));
    assert!(
        !fixture
            .path()
            .join("etc/systemd/system/sbctl-credential-rotate.timer")
            .exists()
    );
    fs::write(&path, "[Unit]\nDescription=foreign\n").unwrap();
    assert!(sbctl::lifecycle::migrate_credential_rotate_service(fixture.path()).is_err());
    assert!(
        fs::read_to_string(path)
            .unwrap()
            .contains("Description=foreign")
    );
}

#[cfg(unix)]
#[test]
fn direct_acme_bootstrap_success_and_failure_are_transactional() {
    use std::os::unix::fs::PermissionsExt;
    for succeeds in [true, false] {
        let (fixture, binary) = mode_fixture(SubscriptionMode::ExternalProxy);
        fs::remove_dir_all(fixture.path().join("etc/letsencrypt/live")).unwrap();
        let config_path = fixture.path().join("etc/sbctl/config.toml");
        let before = fs::read(&config_path).unwrap();
        let certificate =
            rcgen::generate_simple_self_signed(vec!["sub.example.test".into()]).unwrap();
        fs::write(fixture.path().join("issued.pem"), certificate.cert.pem()).unwrap();
        fs::write(
            fixture.path().join("issued.key"),
            certificate.signing_key.serialize_pem(),
        )
        .unwrap();
        let certbot = fixture.path().join("usr/bin/certbot");
        let script = if succeeds {
            "root=$(CDPATH= cd -- \"$(dirname -- \"$0\")/../..\" && pwd)\nif grep -q '\"insecure\": true' \"$root/var/lib/sbctl/artifacts/subscription-sing-box.json\"; then exit 21; fi\nmkdir -p \"$root/etc/letsencrypt/live/sub.example.test\"\ncp \"$root/issued.pem\" \"$root/etc/letsencrypt/live/sub.example.test/fullchain.pem\"\ncp \"$root/issued.key\" \"$root/etc/letsencrypt/live/sub.example.test/privkey.pem\"\n"
        } else {
            "exit 1\n"
        };
        fs::write(&certbot, format!("#!/bin/sh\n{script}")).unwrap();
        fs::set_permissions(&certbot, fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = switch(&fixture, &binary, "direct", free_high_tcp_port());
        command.args([
            "--certificate-mode",
            "domain",
            "--protocol-sni",
            "sub.example.test",
            "--certbot-email",
            "admin@example.test",
        ]);
        if succeeds {
            command.assert().success();
            let config = DeploymentStore::new(fixture.path()).load().unwrap();
            assert_eq!(config.certificate_mode, CertificateMode::Domain);
            sbctl::certificate::load_pinned(&DeploymentStore::new(fixture.path()), &config)
                .unwrap();
            let client = fs::read_to_string(
                fixture
                    .path()
                    .join("var/lib/sbctl/artifacts/subscription-sing-box.json"),
            )
            .unwrap();
            assert!(client.contains("\"insecure\": false"));
            let server =
                fs::read_to_string(fixture.path().join("etc/sing-box/config.json")).unwrap();
            assert!(server.contains("fullchain.pem"));
        } else {
            command
                .assert()
                .code(2)
                .stderr(predicate::str::contains("旧部署已恢复"));
            assert_eq!(fs::read(config_path).unwrap(), before);
            assert!(
                !fixture
                    .path()
                    .join("etc/systemd/system/sbctl-http.socket")
                    .exists()
            );
            assert!(
                !fixture
                    .path()
                    .join("var/lib/sbctl/certificates/sub.example.test/fullchain.pem")
                    .exists()
            );
        }
    }
}
