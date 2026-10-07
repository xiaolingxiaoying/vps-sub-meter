//! `sbctl sing-box version|status` and `sbctl logs`: the observation verbs.
//!
//! The systemd-dependent paths are Unix-only (Windows has no systemctl, and
//! the kernel resolver only recognizes `sing-box.exe` there); the WSL gate
//! runs them. The clap surface itself is checked on every platform.

use assert_cmd::Command;

#[test]
fn the_observation_verbs_are_wired_into_the_clap_surface() {
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["sing-box", "version", "--help"])
        .assert()
        .success();
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["sing-box", "status", "--help"])
        .assert()
        .success();
    Command::cargo_bin("sbctl")
        .expect("sbctl binary is built")
        .args(["logs", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--unit"))
        // The defaults keep the old menu behaviour: both units, last 50 lines.
        .stdout(predicates::str::contains("[default: all]"))
        .stdout(predicates::str::contains("[default: 50]"));
}

#[cfg(unix)]
mod unix {
    use super::Command;
    use predicates::prelude::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    /// A sing-box fixture that reports a full version string on `version`.
    fn versioned_sing_box_fixture(fixture: &TempDir, version: &str) {
        let path = fixture.path().join("usr/local/bin/sing-box");
        fs::create_dir_all(path.parent().unwrap()).expect("bin directory is created");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = version ]; then\n  echo \"sing-box version {version}\"\n  exit 0\nfi\nexit 0\n"
            ),
        )
        .expect("versioned sing-box fixture is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("versioned sing-box fixture is executable");
    }

    #[test]
    fn sing_box_version_prints_the_installed_kernel_with_patch() {
        let fixture = TempDir::new().expect("temporary root is created");
        versioned_sing_box_fixture(&fixture, "1.14.1");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "sing-box", "version"])
            .assert()
            .success()
            // The full version with patch must be visible: the registry band
            // already drops it, so this verb exists precisely to keep it.
            .stdout(predicate::str::contains("sing-box 1.14.1"));
    }

    #[test]
    fn sing_box_status_reports_systemd_facts_from_the_fixture_root() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");
        systemctl_show_fixture(&fixture);

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "sing-box", "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("active / running"))
            .stdout(predicate::str::contains("4242"))
            .stdout(predicate::str::contains("100.0 MiB (104857600 B)"))
            .stdout(predicate::str::contains("18"));
    }

    /// A `usr/bin/systemctl` fixture answering `show` with one frozen set of
    /// unit facts.
    fn systemctl_show_fixture(fixture: &TempDir) {
        let systemctl = fixture.path().join("usr/bin/systemctl");
        fs::create_dir_all(systemctl.parent().unwrap()).expect("directory is created");
        fs::write(
            &systemctl,
            "#!/bin/sh\ncat <<'EOF'\nActiveState=active\nSubState=running\nMainPID=4242\nNRestarts=2\nMemoryCurrent=104857600\nCPUUsageNSec=2500000000\nTasksCurrent=18\nActiveEnterTimestamp=Tue 2026-10-06 08:00:00 UTC\nEOF\nexit 0\n",
        )
        .expect("systemctl show fixture is written");
        fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o700))
            .expect("systemctl fixture is executable");
    }

    #[test]
    fn sing_box_status_fails_cleanly_when_systemd_cannot_be_queried() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");

        // A fixture root never falls back to the developer host's systemctl: the
        // rooted-command rule makes the missing fixture an explicit failure,
        // which is what keeps this assertion deterministic on every platform.
        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "sing-box", "status"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("sing-box 状态查询失败"))
            .stderr(predicate::str::contains(
                "fixture command is missing: systemctl",
            ));
    }

    #[test]
    fn logs_runs_journalctl_with_the_selected_units() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");
        let calls = fixture.path().join("journalctl-calls");
        let journalctl = fixture.path().join("usr/bin/journalctl");
        fs::create_dir_all(journalctl.parent().unwrap()).expect("directory is created");
        fs::write(
            &journalctl,
            format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", calls.display()),
        )
        .expect("journalctl fixture is written");
        fs::set_permissions(&journalctl, fs::Permissions::from_mode(0o700))
            .expect("journalctl fixture is executable");

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args([
                "--root", root, "logs", "--unit", "sing-box", "--lines", "10",
            ])
            .assert()
            .success();

        let recorded = fs::read_to_string(&calls).expect("journalctl call is recorded");
        assert!(
            recorded.contains("-u sing-box.service")
                && recorded.contains("-n 10")
                && recorded.contains("--no-pager")
                && !recorded.contains("-f"),
            "the journalctl invocation must select the unit, tail and no follow: {recorded}"
        );

        // The follow flag is the only difference the verb adds to the menu's
        // historical fixed tail-50 invocation.
        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "logs", "--follow"])
            .assert()
            .success();
        let recorded = fs::read_to_string(&calls).expect("second journalctl call is recorded");
        assert!(
            recorded.ends_with("-f\n")
                && recorded.contains("-u sbctl.service")
                && recorded.contains("-u sing-box.service"),
            "the default unit is both services and --follow appends -f: {recorded}"
        );
    }

    #[test]
    fn status_json_carries_the_kernel_version_and_sing_box_process_facts() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");
        versioned_sing_box_fixture(&fixture, "1.14.1");
        init_ip_fallback(root);

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "status", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"kernel_version\": \"1.14.1\""))
            .stdout(predicate::str::contains("\"sing_box\":"));
    }

    #[test]
    fn human_status_prints_the_sing_box_process_line() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");
        systemctl_show_fixture(&fixture);
        init_ip_fallback(root);

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("sing-box: active / running"));
    }

    pub(super) fn init_ip_fallback(root: &str) {
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
    fn connections_reports_the_verb_to_enable_when_the_api_is_off() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");
        init_ip_fallback(root);

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "sing-box", "connections"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("sbctl sing-box api enable"));
    }

    #[test]
    fn api_status_reports_the_disabled_state_without_touching_the_host() {
        let fixture = TempDir::new().expect("temporary root is created");
        let root = fixture.path().to_str().expect("fixture path is UTF-8");
        init_ip_fallback(root);

        Command::cargo_bin("sbctl")
            .expect("sbctl binary is built")
            .args(["--root", root, "sing-box", "api", "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("未启用"));
    }
}
