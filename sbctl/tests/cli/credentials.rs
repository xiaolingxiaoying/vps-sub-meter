//! Named subscription credentials: per-device links, grace windows, and the
//! rule that retiring one link never disturbs the others.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

use crate::fixture::{
    free_high_tcp_port, http_get, initialize_ip_fallback_subscription,
    read_subscription_credential, spawn_sbctl_serve, write_systemctl_fixture,
};

/// Every persisted record for one credential name, as (secret, revoked_at).
/// A grace rotation leaves two records under one name: the live link and the
/// retiring previous secret, so a test that reads only the first would be
/// asserting on whichever happens to sort first.
fn named_records(fixture: &TempDir, name: &str) -> Vec<(String, Option<i64>)> {
    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is readable");
    config
        .split("[[subscription_credentials]]")
        .skip(1)
        .filter(|block| block.contains(&format!("name = \"{}\"", name)))
        .map(|block| {
            // Split on `=` and trim quotes rather than matching a quoted
            // literal: the same helper reads both fields, and the escaping for
            // an embedded double quote here is more trouble than it is worth.
            let mut credential = String::new();
            let mut revoked_at = None;
            for line in block.lines() {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                match key.trim() {
                    "credential" => credential = value.trim().trim_matches('"').to_owned(),
                    "revoked_at" => revoked_at = value.trim().parse::<i64>().ok(),
                    _ => {}
                }
            }
            (credential, revoked_at)
        })
        .collect()
}

fn read_named_credential(fixture: &TempDir, name: &str) -> (String, Option<i64>) {
    let records = named_records(fixture, name);
    // The live record: the one with no expiry.
    records
        .iter()
        .find(|(_, revoked)| revoked.is_none())
        .cloned()
        .unwrap_or_else(|| records.first().cloned().expect("a record exists"))
}

#[test]
fn named_credentials_serve_side_by_side_and_revoke_touches_only_one_link() {
    let fixture = TempDir::new().expect("temporary root is created");
    let port = free_high_tcp_port();
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    write_systemctl_fixture(&fixture, true);
    let default_credential = initialize_ip_fallback_subscription(&fixture, port);

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "add", "--name", "phone"])
        .assert()
        .success()
        .stdout(predicate::str::contains("/sub/"));

    let (phone, revoked) = read_named_credential(&fixture, "phone");
    assert_ne!(phone, default_credential, "a named link has its own secret");
    assert!(
        revoked.is_none(),
        "a freshly issued credential carries no grace window"
    );

    let mut stderr_log = fixture.path().join("serve-named.stderr");
    let mut server = spawn_sbctl_serve(&fixture, port, 2, &stderr_log);
    assert!(
        http_get(port, &format!("/sub/{phone}/uri")).starts_with("HTTP/1.1 200"),
        "the named credential must serve its own subscription"
    );
    assert!(
        http_get(port, &format!("/sub/{default_credential}/uri")).starts_with("HTTP/1.1 200"),
        "the default credential keeps serving alongside it"
    );
    let _ = server.kill();
    let _ = server.wait();

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "revoke", "phone"])
        .assert()
        .success();

    stderr_log = fixture.path().join("serve-after-revoke.stderr");
    let mut server = spawn_sbctl_serve(&fixture, port, 2, &stderr_log);
    assert!(
        http_get(port, &format!("/sub/{phone}/uri")).starts_with("HTTP/1.1 404"),
        "the revoked named link must stop working"
    );
    assert!(
        http_get(port, &format!("/sub/{default_credential}/uri")).starts_with("HTTP/1.1 200"),
        "revoking one device must not cut off the default link"
    );
    let _ = server.kill();
    let _ = server.wait();

    let stderr = fs::read_to_string(&stderr_log).expect("stderr is readable");
    assert!(
        !stderr.contains(&phone) && !stderr.contains(&default_credential),
        "the journal must not carry either path secret"
    );
}

#[test]
fn a_grace_window_keeps_the_old_link_until_it_expires_and_rotate_spares_the_default() {
    let fixture = TempDir::new().expect("temporary root is created");
    let port = free_high_tcp_port();
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    write_systemctl_fixture(&fixture, true);
    let default_credential = initialize_ip_fallback_subscription(&fixture, port);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "add", "--name", "laptop"])
        .assert()
        .success();
    let (first, _) = read_named_credential(&fixture, "laptop");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "credential",
            "rotate",
            "--name",
            "laptop",
            "--grace",
            "30m",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("旧链接在 1800 秒内仍然有效"));

    let (second, live_expiry) = read_named_credential(&fixture, "laptop");
    assert_ne!(second, first, "rotation replaced the named secret");
    assert_eq!(live_expiry, None, "the live record has no expiry");
    // The grace promise is only real if the *previous* secret is still remembered
    // somewhere, as its own record with a future expiry.
    let records = named_records(&fixture, "laptop");
    assert_eq!(
        records.len(),
        2,
        "one live plus one retiring record: {records:?}"
    );
    let retiring = records
        .iter()
        .find(|(secret, _)| secret == &first)
        .expect("the retiring record keeps the previous secret");
    let revoked_at = retiring.1.expect("the retiring record carries an expiry");
    assert!(
        revoked_at > chrono::Utc::now().timestamp(),
        "the window closes in the future"
    );
    assert_eq!(
        read_subscription_credential(&fixture),
        default_credential,
        "rotating one device link must leave the default credential alone"
    );

    // Inside the window both generations serve: the device can refresh before
    // its old URL dies, which is the entire purpose of --grace.
    let mut stderr_log = fixture.path().join("serve-grace.stderr");
    let mut server = spawn_sbctl_serve(&fixture, port, 2, &stderr_log);
    assert!(http_get(port, &format!("/sub/{first}/uri")).starts_with("HTTP/1.1 200"));
    assert!(http_get(port, &format!("/sub/{second}/uri")).starts_with("HTTP/1.1 200"));
    let _ = server.kill();
    let _ = server.wait();

    // Close the window by rewriting only that field instead of sleeping: the
    // comparator's contract is the instant, not the wall clock.
    let config = fs::read_to_string(fixture.path().join("etc/sbctl/config.toml"))
        .expect("configuration is readable");
    let expired = config.replace(&format!("revoked_at = {revoked_at}"), "revoked_at = 1");
    assert_ne!(config, expired, "the grace field was found and rewritten");
    fs::write(fixture.path().join("etc/sbctl/config.toml"), expired)
        .expect("an expired window is written");

    stderr_log = fixture.path().join("serve-expired.stderr");
    let mut server = spawn_sbctl_serve(&fixture, port, 2, &stderr_log);
    assert!(
        http_get(port, &format!("/sub/{first}/uri")).starts_with("HTTP/1.1 404"),
        "an expired grace window stops serving"
    );
    assert!(
        http_get(port, &format!("/sub/{second}/uri")).starts_with("HTTP/1.1 200"),
        "the current link is unaffected by its own expired predecessor"
    );
    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn credential_list_masks_every_secret_and_refuses_ambiguous_names() {
    let fixture = TempDir::new().expect("temporary root is created");
    let port = free_high_tcp_port();
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    write_systemctl_fixture(&fixture, true);
    let default_credential = initialize_ip_fallback_subscription(&fixture, port);

    let output = Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "list"])
        .output()
        .expect("list runs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("default"), "{stdout}");
    assert!(
        !stdout.contains(&default_credential),
        "the list masks the path secret; `sbctl sub` is the command that prints it"
    );

    // The default link is what every existing client holds; letting an operator
    // "revoke" it into a 404 for everyone is a foot-gun, not a feature.
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "revoke", "default"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("默认凭据不能吊销"));

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "revoke", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("没有生效中的凭据"));
}

#[test]
fn a_malformed_grace_duration_is_refused_before_anything_changes() {
    let fixture = TempDir::new().expect("temporary root is created");
    let port = free_high_tcp_port();
    let root = fixture.path().to_str().expect("fixture path is UTF-8");
    write_systemctl_fixture(&fixture, true);
    initialize_ip_fallback_subscription(&fixture, port);
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["--root", root, "credential", "add", "--name", "tv"])
        .assert()
        .success();
    let (before, _) = read_named_credential(&fixture, "tv");

    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args([
            "--root",
            root,
            "credential",
            "rotate",
            "--name",
            "tv",
            "--grace",
            "30x",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("时长单位"));

    let (after, revoked) = read_named_credential(&fixture, "tv");
    assert_eq!(after, before, "a refused command must not rotate anything");
    assert!(
        revoked.is_none(),
        "a refused command must not revoke anything"
    );
}
