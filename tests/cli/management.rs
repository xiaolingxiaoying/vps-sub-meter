use crate::fixture::initialize_update_fixture;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

#[test]
fn export_matches_preview_and_never_overwrites_an_existing_file() {
    let fixture = tempfile::tempdir().unwrap();
    initialize_update_fixture(&fixture);
    let output = fixture.path().join("server.json");
    let root = fixture.path().to_str().unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "config",
            "export",
            "--output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    let preview = Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "config", "preview"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(preview).unwrap().trim(),
        fs::read_to_string(&output).unwrap().trim()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fs::write(&output, "keep").unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "config",
            "export",
            "--output",
            output.to_str().unwrap(),
        ])
        .assert()
        .code(2);
    assert_eq!(fs::read_to_string(&output).unwrap(), "keep");
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "--root",
            root,
            "config",
            "preview",
            "--format",
            "../../config.toml",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("未知配置格式"));
}

#[test]
fn kernel_selection_rejects_a_version_for_repository_or_a_conflicting_manifest() {
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "sing-box",
            "fetch",
            "--source",
            "repository",
            "--version",
            "1.14.1",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("不支持选择版本"));
    Command::cargo_bin("sbctl")
        .unwrap()
        .args([
            "install",
            "--manifest",
            "missing.json",
            "--kernel-version",
            "1.14.1",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[cfg(unix)]
#[test]
fn smtp_failures_retry_and_success_is_deduplicated_without_logging_secrets() {
    use crate::fixture::{write_managed_file, write_systemctl_fixture, write_traffic_fixture};
    use base64::Engine;
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    initialize_update_fixture(&fixture);
    write_systemctl_fixture(&fixture, true);
    write_traffic_fixture(&fixture, 100, 200, "boot-a");
    let root = fixture.path().to_str().unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "accounting-reset"])
        .assert()
        .success();
    write_traffic_fixture(&fixture, 1100, 2200, "boot-a");
    let config = sbctl::email::EmailConfig {
        password: "smtp-secret-DO-NOT-LOG".into(),
        include_subscription: true,
        reset_reminder_hours: 0,
        ..Default::default()
    };
    write_managed_file(
        &fixture,
        sbctl::email::CONFIG_PATH,
        toml::to_string(&config).unwrap().as_bytes(),
    );
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "email", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&config.password).not());
    let curl = fixture.path().join("usr/bin/curl");
    fs::write(
        &curl,
        "#!/bin/sh\necho 'smtp-secret-DO-NOT-LOG' >&2\nexit 67\n",
    )
    .unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "email", "check"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(&config.password).not());
    assert!(
        !fixture
            .path()
            .join("var/lib/sbctl/email-state.json")
            .exists()
    );
    // This stub copies the payload for assertions and never contacts SMTP.
    fs::write(
        &curl,
        format!(
            r#"#!/usr/bin/env python3
import pathlib, sys, re
root = pathlib.Path({root:?})
assert 'smtp-secret-DO-NOT-LOG' not in ' '.join(sys.argv)
options = pathlib.Path(sys.argv[-1]).read_text()
assert 'ssl-reqd' in options
assert 'smtps://smtp.example.com:465' in options
payload = re.search(r'upload-file = "(.*)"', options).group(1)
(root / '.mail').write_bytes(pathlib.Path(payload).read_bytes())
with (root / '.sends').open('a') as file: file.write('sent\n')
"#
        ),
    )
    .unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "email", "check"])
        .assert()
        .success();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "email", "check"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(fixture.path().join(".sends")).unwrap(),
        "sent\n"
    );
    let mail = fs::read_to_string(fixture.path().join(".mail"))
        .unwrap()
        .replace("\r\n", "\n");
    let encoded = mail
        .split_once("\n\n")
        .unwrap()
        .1
        .lines()
        .collect::<String>();
    let body = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
    )
    .unwrap();
    assert!(body.contains("RX 入站") && body.contains("TX 出站"));
    let deployment = sbctl::config::DeploymentStore::new(fixture.path())
        .load()
        .unwrap();
    assert!(body.contains(&deployment.subscription_credential));
    let state = fixture.path().join("var/lib/sbctl/email-state.json");
    assert_eq!(
        fs::metadata(state).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn email_timer_is_removed_and_private_settings_follow_uninstall_policy() {
    use crate::fixture::{write_managed_file, write_systemctl_fixture};
    let fixture = tempfile::tempdir().unwrap();
    initialize_update_fixture(&fixture);
    write_systemctl_fixture(&fixture, true);
    write_managed_file(&fixture, "var/lib/sbctl/ownership", b"sbctl-managed-v1\n");
    let config = sbctl::email::EmailConfig {
        password: "private-password".into(),
        ..Default::default()
    };
    write_managed_file(
        &fixture,
        sbctl::email::CONFIG_PATH,
        toml::to_string(&config).unwrap().as_bytes(),
    );
    let root = fixture.path().to_str().unwrap();
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "email", "enable"])
        .assert()
        .success();
    let timer = fixture.path().join("etc/systemd/system/sbctl-email.timer");
    assert!(
        fs::read_to_string(&timer)
            .unwrap()
            .contains("OnCalendar=hourly")
    );
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "uninstall"])
        .assert()
        .success();
    assert!(!timer.exists());
    assert!(fixture.path().join(sbctl::email::CONFIG_PATH).exists());
    let backups = fs::read_dir(fixture.path().join("var/backups/sbctl"))
        .unwrap()
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert!(
        backups[0]
            .as_ref()
            .unwrap()
            .path()
            .join(sbctl::email::CONFIG_PATH)
            .exists()
    );
    Command::cargo_bin("sbctl")
        .unwrap()
        .args(["--root", root, "uninstall", "--purge"])
        .assert()
        .success();
    assert!(!fixture.path().join(sbctl::email::CONFIG_PATH).exists());
}
