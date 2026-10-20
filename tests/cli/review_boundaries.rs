#[cfg(target_os = "linux")]
mod linux {
    use crate::fixture::{initialize_update_fixture, seed_direct_config, write_managed_file};
    use std::fs;
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    fn script(root: &Path, name: &str, text: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{text}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn initialization_rejects_parent_links_without_writing_outside_root() {
        use std::os::unix::fs::symlink;
        for relative in ["var/lib/sbctl", "var/lib/sbctl/artifacts", "etc/sbctl"] {
            let fixture = TempDir::new().unwrap();
            let outside = TempDir::new().unwrap();
            let target = fixture.path().join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            symlink(outside.path(), &target).unwrap();
            let output = Command::new(assert_cmd::cargo::cargo_bin("sbctl"))
                .args([
                    "--root",
                    fixture.path().to_str().unwrap(),
                    "config",
                    "init",
                    "--mode",
                    "ip-fallback",
                    "--subscription-host",
                    "127.0.0.1",
                    "--http-port",
                    "2080",
                    "--interface",
                    "ens3",
                    "--protocol",
                    "vless-reality",
                    "--reality-decoy-sni",
                    "www.cloudflare.com",
                ])
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "parent link {relative} must fail initialization"
            );
            assert_eq!(
                fs::read_dir(outside.path()).unwrap().count(),
                0,
                "outside directory changed through {relative}"
            );
        }
    }

    #[test]
    fn isolated_root_cannot_restart_the_host_or_apply_host_sysctl_without_fixtures() {
        for system in [false, true] {
            let fixture = TempDir::new().unwrap();
            initialize_update_fixture(&fixture);
            let root = fixture.path();
            let host = TempDir::new().unwrap();
            let marker = root.join("host-command-called");
            script(
                host.path(),
                if system { "sysctl" } else { "systemctl" },
                "printf called > \"$AUDIT_MARKER\"\nexit 0\n",
            );
            let mut command = Command::new(assert_cmd::cargo::cargo_bin("sbctl"));
            command.args(["--root", root.to_str().unwrap()]);
            if system {
                write_managed_file(
                    &fixture,
                    "proc/sys/net/ipv4/tcp_congestion_control",
                    b"cubic\n",
                );
                write_managed_file(&fixture, "proc/sys/net/core/default_qdisc", b"pfifo_fast\n");
                command.args(["system", "bbr"]);
            } else {
                let kernel = script(root, "usr/local/bin/sing-box", "exit 0\n");
                command.args(["restart", "--sing-box-bin", kernel.to_str().unwrap()]);
            }
            let output = command
                .env("PATH", host.path())
                .env("AUDIT_MARKER", &marker)
                .output()
                .unwrap();
            assert!(
                !marker.exists(),
                "isolated root executed host {}",
                if system { "sysctl" } else { "systemctl" }
            );
            assert!(!output.status.success(), "missing fixture must fail");
            assert!(String::from_utf8_lossy(&output.stderr).contains("fixture command is missing"));
            let fixture_marker = root.join("fixture-command-called");
            script(
                root,
                if system {
                    "usr/bin/sysctl"
                } else {
                    "usr/bin/systemctl"
                },
                "printf called > \"$FIXTURE_MARKER\"\nexit 0\n",
            );
            let output = command
                .env("FIXTURE_MARKER", &fixture_marker)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "provided fixture must work: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(fixture_marker.exists());
            assert!(
                !marker.exists(),
                "provided fixture must take priority over PATH"
            );
        }
    }

    #[test]
    fn isolated_certificate_renewal_cannot_call_host_certbot() {
        let fixture = TempDir::new().unwrap();
        seed_direct_config(&fixture);
        let host = TempDir::new().unwrap();
        let marker = fixture.path().join("host-certbot-called");
        script(
            host.path(),
            "certbot",
            "printf called > \"$AUDIT_MARKER\"\nexit 0\n",
        );
        let output = Command::new(assert_cmd::cargo::cargo_bin("sbctl"))
            .args([
                "--root",
                fixture.path().to_str().unwrap(),
                "certificate",
                "renew",
            ])
            .env("PATH", host.path())
            .env("AUDIT_MARKER", &marker)
            .output()
            .unwrap();
        assert!(
            !marker.exists(),
            "isolated renewal must never reach host certbot"
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("fixture command is missing"));
    }

    #[test]
    fn artifact_only_write_failure_preserves_active_config_and_old_artifacts() {
        let fixture = TempDir::new().unwrap();
        initialize_update_fixture(&fixture);
        let root = fixture.path();
        let active = root.join("etc/sing-box/config.json");
        write_managed_file(
            &fixture,
            "etc/sing-box/config.json",
            b"untouched active configuration",
        );
        let artifacts: Vec<_> = fs::read_dir(root.join("var/lib/sbctl/artifacts"))
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        // The checker lowers only its parent's file limit after check input was
        // staged. Snapshots still read normally; the later atomic write fails.
        let kernel = script(
            root,
            "usr/local/bin/sing-box",
            "if [ \"$1\" = version ]; then echo 'sing-box version 1.14.2'; exit 0; fi\nif /bin/grep -q '\"users\"' \"$3\"; then exec /usr/bin/prlimit --pid \"$PPID\" --fsize=0:0; fi\nexit 0\n",
        );
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("sbctl"));
        command.args([
            "--root",
            root.to_str().unwrap(),
            "regenerate",
            "--sing-box-bin",
            kernel.to_str().unwrap(),
        ]);
        unsafe {
            command.pre_exec(|| {
                libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("File too large"),
            "must exercise write failure: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(&active).expect("unmodified active configuration must survive rollback"),
            b"untouched active configuration"
        );
        for (path, bytes) in artifacts {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}
