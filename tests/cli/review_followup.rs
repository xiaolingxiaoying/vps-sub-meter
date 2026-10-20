use crate::fixture::{initialize_update_fixture, write_managed_file};
use assert_cmd::Command;
use sbctl::config::DeploymentStore;
use std::{fs, path::Path};
use tempfile::TempDir;

fn seed() -> TempDir {
    let fixture = TempDir::new().unwrap();
    initialize_update_fixture(&fixture);
    #[cfg(unix)]
    {
        script(fixture.path(), "usr/local/bin/sing-box", "exit 0\n");
        script(fixture.path(), "usr/bin/systemctl", "exit 0\n");
    }
    fixture
}

#[test]
fn invalid_rules_mode_does_not_publish_artifacts_or_change_active_configuration() {
    let fixture = seed();
    let root = fixture.path();
    let paths: Vec<_> = fs::read_dir(root.join("var/lib/sbctl/artifacts"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .chain([
            root.join("etc/sbctl/config.toml"),
            root.join("etc/sing-box/config.json"),
        ])
        .filter(|path| path.is_file())
        .collect();
    let before: Vec<_> = paths.iter().map(|path| fs::read(path).unwrap()).collect();
    for (name, text) in [
        (
            "sing-box-override.json",
            r#"{"rules_mode": false, "route":{"rules":[]}}"#,
        ),
        ("clash-override.yaml", "rules_mode: false\nrules: []\n"),
        (
            "sing-box-server.json",
            r#"{"rules_mode": null, "route":{"rules":[]}}"#,
        ),
    ] {
        let relative = format!("etc/sbctl/overrides/{name}");
        write_managed_file(&fixture, &relative, text.as_bytes());
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root.to_str().unwrap(), "regenerate"])
            .assert()
            .code(2)
            .stderr(predicates::str::contains("rules_mode"));
        for (path, bytes) in paths.iter().zip(&before) {
            assert_eq!(
                &fs::read(path).unwrap(),
                bytes,
                "failed generation changed {}",
                path.display()
            );
        }
        fs::remove_file(root.join(relative)).unwrap();
    }
}

#[cfg(unix)]
fn script(root: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
fn wait_for(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "missing {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn override_rules_modes_apply_to_generated_artifacts_and_each_layer_in_order() {
    let fixture = seed();
    let root = fixture.path();
    let config = DeploymentStore::new(root).load().unwrap();
    let baseline = sbctl::subscription::generated_artifacts(&config, root).unwrap();
    for mode in ["append", "replace"] {
        let rule = serde_json::json!({"domain": ["review.example"], "outbound": "DIRECT"});
        write_managed_file(
            &fixture,
            "etc/sbctl/overrides/sing-box-override.json",
            serde_json::to_string(&serde_json::json!({
                "rules_mode": mode, "route": {"rules": [rule]}
            }))
            .unwrap()
            .as_bytes(),
        );
        write_managed_file(
            &fixture,
            "etc/sbctl/overrides/clash-override.yaml",
            format!("rules_mode: {mode}\nrules:\n  - DOMAIN,review.example,DIRECT\n").as_bytes(),
        );
        let generated = sbctl::subscription::generated_artifacts(&config, root).unwrap();
        for (name, contents) in &generated {
            let previous = &baseline.iter().find(|(key, _)| key == name).unwrap().1;
            if name.starts_with("subscription-sing-box-") {
                let result: serde_json::Value = serde_json::from_str(contents).unwrap();
                let previous: serde_json::Value = serde_json::from_str(previous).unwrap();
                let mut expected = if mode == "replace" {
                    vec![]
                } else {
                    previous["route"]["rules"].as_array().unwrap().clone()
                };
                expected.push(rule.clone());
                assert_eq!(
                    result["route"]["rules"],
                    serde_json::json!(expected),
                    "{name} / {mode}"
                );
                assert!(result.get("rules_mode").is_none());
            } else if name.starts_with("subscription-clash") {
                let result: serde_yaml::Value = serde_yaml::from_str(contents).unwrap();
                let previous: serde_yaml::Value = serde_yaml::from_str(previous).unwrap();
                let mut expected = if mode == "replace" {
                    vec![]
                } else {
                    previous["rules"].as_sequence().unwrap().clone()
                };
                expected.push(serde_yaml::Value::String(
                    "DOMAIN,review.example,DIRECT".into(),
                ));
                assert_eq!(
                    result["rules"],
                    serde_yaml::Value::Sequence(expected),
                    "{name} / {mode}"
                );
                assert!(result.get("rules_mode").is_none());
            }
        }
    }
    for (name, mode, domain) in [
        ("sing-box-override.json", "prepend", "base.example"),
        ("sing-box.d/10-append.json", "append", "append.example"),
        ("sing-box.d/20-replace.json", "replace", "replace.example"),
        ("sing-box.d/30-prepend.json", "prepend", "first.example"),
        ("sing-box.d/40-append.json", "append", "last.example"),
    ] {
        write_managed_file(&fixture, &format!("etc/sbctl/overrides/{name}"),
            serde_json::to_string(&serde_json::json!({"rules_mode":mode,"route":{"rules":[{"domain":[domain],"outbound":"DIRECT"}]}})).unwrap().as_bytes());
    }
    let generated = sbctl::subscription::generated_artifacts(&config, root).unwrap();
    let contents = &generated
        .iter()
        .find(|(name, _)| name == "subscription-sing-box-full.json")
        .unwrap()
        .1;
    let value: serde_json::Value = serde_json::from_str(contents).unwrap();
    let domains: Vec<_> = value["route"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| rule["domain"][0].as_str().unwrap())
        .collect();
    assert_eq!(
        domains,
        ["first.example", "replace.example", "last.example"]
    );
    write_managed_file(&fixture, "etc/sbctl/overrides/sing-box-server.json", br#"{"rules_mode":"replace","route":{"rules":[{"domain":["server.example"],"outbound":"direct"}]}}"#);
    let artifacts = sbctl::subscription::generated_artifacts(&config, root).unwrap();
    let result = &artifacts
        .iter()
        .find(|(name, _)| name == "sing-box-server.json")
        .unwrap()
        .1;
    let result: serde_json::Value = serde_json::from_str(result).unwrap();
    assert_eq!(result["route"]["rules"].as_array().unwrap().len(), 1);
    assert_eq!(result["route"]["rules"][0]["domain"][0], "server.example");
}

#[cfg(unix)]
#[test]
fn api_recovery_serializes_concurrent_credential_changes() {
    use std::process::{Command as Process, Stdio};
    for revoke in [false, true] {
        let fixture = seed();
        let root = fixture.path();
        if revoke {
            Command::cargo_bin("sbctl")
                .unwrap()
                .args([
                    "--root",
                    root.to_str().unwrap(),
                    "credential",
                    "add",
                    "--name",
                    "phone",
                ])
                .assert()
                .success();
        }
        let store = DeploymentStore::new(root);
        let before = store.load().unwrap();
        script(
            root,
            "usr/bin/systemctl",
            &format!(
                "if [ \"$BLOCK_API\" = 1 ] && [ \"$1\" = restart ] && [ ! -f '{0}/failed' ]; then\n touch '{0}/ready'\n while [ ! -f '{0}/release' ]; do sleep 0.02; done\n touch '{0}/failed'\n exit 1\nfi\nexit 0\n",
                root.display()
            ),
        );
        let mut api = Process::new(assert_cmd::cargo::cargo_bin("sbctl"))
            .args([
                "--root",
                root.to_str().unwrap(),
                "sing-box",
                "api",
                "enable",
                "--port",
                "19090",
            ])
            .env("BLOCK_API", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for(&root.join("ready"));
        let mut credential = Process::new(assert_cmd::cargo::cargo_bin("sbctl"));
        credential.args(["--root", root.to_str().unwrap(), "credential"]);
        if revoke {
            credential.args(["revoke", "phone"]);
        } else {
            credential.arg("rotate");
        }
        let mut credential = credential
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let blocked = credential.try_wait().unwrap().is_none();
        fs::write(root.join("release"), "go").unwrap();
        assert_eq!(api.wait().unwrap().code(), Some(2));
        assert!(credential.wait().unwrap().success());
        assert!(blocked, "credential operation must wait for API recovery");
        let after = store.load().unwrap();
        assert!(after.server_clash_api.is_none());
        if revoke {
            assert!(
                after
                    .subscription_credentials
                    .iter()
                    .all(|entry| entry.revoked_at.is_some())
            );
        } else {
            assert_ne!(
                after.subscription_credential,
                before.subscription_credential
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn failed_editor_never_exposes_its_draft_to_regeneration_and_conflicting_edits_abort() {
    use std::process::{Command as Process, Stdio};
    for conflict in [false, true] {
        let fixture = seed();
        let root = fixture.path();
        write_managed_file(
            &fixture,
            "etc/sbctl/overrides/clash-override.yaml",
            b"mixed-port: 17890\n",
        );
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root.to_str().unwrap(), "regenerate"])
            .assert()
            .success();
        let editor = script(
            root,
            "editor",
            &format!(
                "printf 'mixed-port: 17891\\n' > \"$1\"\ntouch '{0}/ready'\nwhile [ ! -f '{0}/release' ]; do sleep 0.02; done\nexit {1}\n",
                root.display(),
                if conflict { 0 } else { 1 }
            ),
        );
        let mut editing = Process::new(assert_cmd::cargo::cargo_bin("sbctl"))
            .args([
                "--root",
                root.to_str().unwrap(),
                "config",
                "override",
                "edit",
                "clash",
            ])
            .env("VISUAL", editor)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for(&root.join("ready"));
        assert_eq!(
            fs::read_to_string(root.join("etc/sbctl/overrides/clash-override.yaml")).unwrap(),
            "mixed-port: 17890\n"
        );
        Command::cargo_bin("sbctl")
            .unwrap()
            .args([
                "--root",
                root.to_str().unwrap(),
                "rule",
                "add",
                "direct",
                "review.example",
            ])
            .assert()
            .success();
        if conflict {
            write_managed_file(
                &fixture,
                "etc/sbctl/overrides/clash-override.yaml",
                b"mixed-port: 17892\n",
            );
        }
        fs::write(root.join("release"), "go").unwrap();
        assert_eq!(editing.wait().unwrap().code(), Some(2));
        let source =
            fs::read_to_string(root.join("etc/sbctl/overrides/clash-override.yaml")).unwrap();
        assert_eq!(
            source,
            if conflict {
                "mixed-port: 17892\n"
            } else {
                "mixed-port: 17890\n"
            }
        );
        let artifact =
            fs::read_to_string(root.join("var/lib/sbctl/artifacts/subscription-clash.yaml"))
                .unwrap();
        let artifact: serde_yaml::Value = serde_yaml::from_str(&artifact).unwrap();
        assert_eq!(artifact["mixed-port"].as_u64(), Some(17890));
    }
}

#[cfg(unix)]
#[test]
fn clearing_server_overrides_activates_and_failed_activation_restores_sources_and_artifacts() {
    for fail in [false, true] {
        let fixture = seed();
        let root = fixture.path();
        write_managed_file(&fixture, "var/lib/sbctl/ownership", b"managed");
        write_managed_file(
            &fixture,
            "etc/sbctl/overrides/sing-box-server.json",
            br#"{"log":{"level":"debug"}}"#,
        );
        Command::cargo_bin("sbctl")
            .unwrap()
            .args(["--root", root.to_str().unwrap(), "regenerate"])
            .assert()
            .success();
        let active = root.join("etc/sing-box/config.json");
        let before = fs::read(&active).unwrap();
        let source = root.join("etc/sbctl/overrides/sing-box-server.json");
        let prior_source = fs::read(&source).unwrap();
        let artifact_dir = root.join("var/lib/sbctl/artifacts");
        let prior_artifacts: Vec<_> = fs::read_dir(&artifact_dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let bytes = fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        script(
            root,
            "usr/bin/systemctl",
            &format!(
                "printf '%s\\n' \"$*\" >> '{0}/calls'\nif [ {1} = 1 ] && [ \"$1\" = restart ] && [ ! -f '{0}/failed' ]; then touch '{0}/failed'; exit 1; fi\nexit 0\n",
                root.display(),
                u8::from(fail)
            ),
        );
        let mut command = Command::cargo_bin("sbctl").unwrap();
        command.args([
            "--root",
            root.to_str().unwrap(),
            "config",
            "override",
            "clear",
            "server",
        ]);
        if fail {
            command.assert().code(2);
        } else {
            command.assert().success();
        }
        assert!(
            fs::read_to_string(root.join("calls"))
                .unwrap()
                .contains("restart")
        );
        if fail {
            assert_eq!(fs::read(source).unwrap(), prior_source);
            assert_eq!(fs::read(active).unwrap(), before);
            for (path, bytes) in prior_artifacts {
                assert_eq!(fs::read(path).unwrap(), bytes);
            }
        } else {
            assert!(!source.exists());
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(active).unwrap()).unwrap();
            assert_ne!(value["log"]["level"], "debug");
        }
    }
}
