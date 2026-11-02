//! Human status additions preserve exact accounting and the existing JSON fields.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use crate::fixture::write_traffic_fixture;

fn run(root: &Path, arguments: &[&str]) -> std::process::Output {
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .arg("--root")
        .arg(root)
        .args(arguments)
        .assert()
        .success()
        .get_output()
        .clone()
}

fn initialize(limit: u64, total: u64) -> TempDir {
    let fixture = TempDir::new().expect("temporary root is created");
    write_traffic_fixture(&fixture, 100, 200, "boot-status-details");
    run(
        fixture.path(),
        &[
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
            "--monthly-traffic-limit",
            &limit.to_string(),
        ],
    );
    run(fixture.path(), &["accounting-reset"]);
    write_traffic_fixture(&fixture, 100 + total, 200, "boot-status-details");
    fixture
}

#[test]
fn status_and_traffic_show_remaining_over_limit_and_unlimited_without_writing_state() {
    for (limit, total, remaining, over_limit, percent, expected_summary) in [
        (
            100,
            50,
            Some(50),
            0,
            Some(50.0),
            "本周期已用: 50 B / 100 B（50.0%）；剩余: 50 B",
        ),
        (
            100,
            100,
            Some(0),
            0,
            Some(100.0),
            "本周期已用: 100 B / 100 B（100.0%）；剩余: 0 B",
        ),
        (
            100,
            120,
            Some(0),
            20,
            Some(120.0),
            "本周期已用: 120 B / 100 B（120.0%）；超额: 20 B",
        ),
        (0, 120, None, 0, None, "本周期已用: 120 B；额度: 不限额"),
    ] {
        let fixture = initialize(limit, total);
        let state = fixture.path().join("var/lib/sbctl/state.json");
        let before = fs::read(&state).expect("accounting state is readable");
        for command in [&["status"][..], &["traffic"][..]] {
            let output = run(fixture.path(), command);
            let text = String::from_utf8(output.stdout).expect("status uses UTF-8");
            assert!(
                text.contains(expected_summary),
                "missing human usage summary: {text}"
            );
            assert!(text.contains(&format!("total: {total} bytes")));
            assert!(text.contains("下一次刷新（VPS: America/Los_Angeles）:"));
            assert!(text.contains("下一次刷新（客户端: Asia/Shanghai）:"));
        }
        let output = run(fixture.path(), &["status", "--json"]);
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("stdout remains JSON only");
        assert_eq!(json["traffic"]["total"], total);
        assert_eq!(json["traffic"]["received"], total);
        assert_eq!(json["traffic"]["transmitted"], 0);
        assert_eq!(json["traffic"]["remaining_bytes"].as_u64(), remaining);
        assert_eq!(json["traffic"]["over_limit_bytes"], over_limit);
        assert_eq!(json["traffic"]["usage_percent"].as_f64(), percent);
        let utc =
            chrono::DateTime::parse_from_rfc3339(json["traffic"]["next_reset"].as_str().unwrap())
                .unwrap();
        for key in [
            "next_reset_vps_refresh_rfc3339",
            "next_reset_client_display_rfc3339",
        ] {
            let local =
                chrono::DateTime::parse_from_rfc3339(json["traffic"][key].as_str().unwrap())
                    .unwrap();
            assert_eq!(
                local, utc,
                "display times must represent the same reset instant"
            );
        }
        for key in ["next_reset_vps_refresh", "next_reset_client_display"] {
            assert_eq!(
                json["traffic"][key].as_str().unwrap().len(),
                19,
                "existing JSON timestamp fields keep their format"
            );
        }
        assert_eq!(
            fs::read(&state).unwrap(),
            before,
            "status additions must remain read-only"
        );
    }
}

#[test]
fn remaining_traffic_uses_the_corrected_total_instead_of_directional_counters() {
    let fixture = initialize(100, 120);
    run(fixture.path(), &["traffic", "set-used", "--bytes", "25"]);
    let output = run(fixture.path(), &["status", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["traffic"]["received"], 120);
    assert_eq!(json["traffic"]["transmitted"], 0);
    assert_eq!(json["traffic"]["total"], 25);
    assert_eq!(json["traffic"]["remaining_bytes"], 75);
    assert_eq!(json["traffic"]["over_limit_bytes"], 0);
    assert_eq!(json["traffic"]["usage_percent"], 25.0);
}

#[test]
fn unavailable_process_observation_is_visible_but_keeps_status_successful_and_json_valid() {
    let fixture = initialize(0, 0);
    // No rooted systemctl fixture: querying the host is forbidden, and this
    // should explain the missing process facts instead of silently hiding them.
    let output = run(fixture.path(), &["status"]);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("sing-box 进程状态: 无法查询"));
    assert!(text.contains("sbctl sing-box status"));
    let output = run(fixture.path(), &["status", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["sing_box"]["error"].is_string());
    assert!(json["sing_box"]["main_pid"].is_null());
}

#[cfg(unix)]
#[test]
fn inactive_process_does_not_present_its_old_activation_as_running_uptime() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = initialize(0, 0);
    let systemctl = fixture.path().join("usr/bin/systemctl");
    fs::create_dir_all(systemctl.parent().unwrap()).unwrap();
    fs::write(
        &systemctl,
        "#!/bin/sh\nif [ \"$1\" = show ]; then\nprintf 'ActiveState=inactive\\nSubState=dead\\nMainPID=0\\nActiveEnterTimestamp=Tue 2026-10-06 08:00:00 UTC\\n'\nexit 0\nfi\nexit 3\n",
    )
    .unwrap();
    fs::set_permissions(systemctl, fs::Permissions::from_mode(0o700)).unwrap();
    let output = run(fixture.path(), &["status"]);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("sing-box: inactive / dead"));
    assert!(text.contains("当前未运行"));
    assert!(!text.contains("已运行"));
}
