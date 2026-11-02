#[cfg(target_os = "linux")]
mod linux {
    use crate::fixture::{initialize_update_fixture, write_managed_file};
    use assert_cmd::Command;
    use predicates::prelude::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    fn executable(root: &Path, name: &str, contents: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{contents}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn restart_rejects_invalid_active_configuration_before_touching_services() {
        let fixture = tempfile::tempdir().unwrap();
        initialize_update_fixture(&fixture);
        let root = fixture.path();
        let cache = root.join("var/lib/sbctl/artifacts/sing-box-server.json");
        let before = fs::read(&cache).unwrap();
        write_managed_file(
            &fixture,
            "etc/sing-box/config.json",
            b"invalid-active-config",
        );
        executable(
            root,
            "usr/local/bin/sing-box",
            "! grep -q invalid-active-config \"$3\"",
        );
        executable(
            root,
            "usr/bin/systemctl",
            &format!("touch '{}/services-called'", root.display()),
        );
        Command::cargo_bin("sbctl")
            .unwrap()
            .arg("--root")
            .arg(root)
            .arg("restart")
            .assert()
            .code(2)
            .stderr(predicate::str::contains("restart failed"));
        assert!(!root.join("services-called").exists());
        assert_eq!(fs::read(cache).unwrap(), before);
        assert_eq!(
            fs::read(root.join("etc/sing-box/config.json")).unwrap(),
            b"invalid-active-config"
        );
    }

    #[test]
    fn restart_uses_active_configuration_and_keeps_the_operation_lock_through_activation() {
        assert!(
            std::process::Command::new("flock")
                .arg("--version")
                .output()
                .unwrap()
                .status
                .success()
        );
        let fixture = tempfile::tempdir().unwrap();
        initialize_update_fixture(&fixture);
        let root = fixture.path();
        write_managed_file(
            &fixture,
            "etc/sing-box/config.json",
            br#"{"active-only":true}"#,
        );
        write_managed_file(
            &fixture,
            "var/lib/sbctl/artifacts/sing-box-server.json",
            b"stale-invalid-cache",
        );
        let lock_check = format!(
            "if flock -n '{0}/var/lib/sbctl/.operation.lock' -c true; then touch '{0}/unlocked'; exit 73; fi",
            root.display()
        );
        executable(
            root,
            "usr/local/bin/sing-box",
            &format!("{lock_check}\ngrep -q active-only \"$3\""),
        );
        executable(
            root,
            "usr/bin/systemctl",
            &format!(
                "{lock_check}\nprintf '%s\\n' \"$*\" >> '{}/service-calls'\nexit 0",
                root.display()
            ),
        );
        Command::cargo_bin("sbctl")
            .unwrap()
            .arg("--root")
            .arg(root)
            .arg("restart")
            .assert()
            .success();
        assert!(!root.join("unlocked").exists());
        assert!(
            fs::read_to_string(root.join("service-calls"))
                .unwrap()
                .contains("restart sing-box.service sbctl.service")
        );
        assert_eq!(
            fs::read(root.join("var/lib/sbctl/artifacts/sing-box-server.json")).unwrap(),
            b"stale-invalid-cache"
        );
    }

    #[test]
    fn restart_refuses_a_symlinked_active_configuration_without_calling_the_core() {
        use std::os::unix::fs::symlink;
        let fixture = tempfile::tempdir().unwrap();
        initialize_update_fixture(&fixture);
        let root = fixture.path();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"private sentinel").unwrap();
        fs::create_dir_all(root.join("etc/sing-box")).unwrap();
        symlink(outside.path(), root.join("etc/sing-box/config.json")).unwrap();
        executable(
            root,
            "usr/local/bin/sing-box",
            &format!("touch '{}/core-called'", root.display()),
        );
        executable(
            root,
            "usr/bin/systemctl",
            &format!("touch '{}/services-called'", root.display()),
        );
        Command::cargo_bin("sbctl")
            .unwrap()
            .arg("--root")
            .arg(root)
            .arg("restart")
            .assert()
            .code(2);
        assert!(!root.join("core-called").exists());
        assert!(!root.join("services-called").exists());
        assert_eq!(fs::read(outside.path()).unwrap(), b"private sentinel");
    }
}
