use assert_cmd::Command;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nodes.json");
    std::fs::write(&path, serde_json::to_vec(&json!([
        {"name":"xhttp", "preset":"vless-xhttp-tls", "server":"example.com", "port":23456, "server_name":"example.com", "credential":"b831381d-6324-4d53-ad4f-8cda48b30811", "path":"/xhttp"},
        {"name":"trojan", "preset":"trojan-tcp-tls", "server":"example.com", "port":23457, "server_name":"example.com", "credential":"fixture-password"}
    ])).unwrap()).unwrap();
    (dir, path)
}

#[test]
fn export_filters_incompatible_nodes_and_check_does_not_echo_secrets() {
    let (directory, input) = fixture();
    let check = Command::new(assert_cmd::cargo::cargo_bin!("sbctl"))
        .args(["protocol", "check", "--input"])
        .arg(&input)
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(!String::from_utf8_lossy(&check.stdout).contains("fixture-password"));
    let export = Command::new(assert_cmd::cargo::cargo_bin!("sbctl"))
        .args(["protocol", "export", "--format", "sing-box", "--input"])
        .arg(&input)
        .assert()
        .success()
        .get_output()
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(value["outbounds"].as_array().unwrap().len(), 1);
    assert_eq!(value["outbounds"][0]["type"], "trojan");
    assert!(String::from_utf8_lossy(&export.stderr).contains("XHTTP"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn bad_input_and_server_export_without_certificates_fail_with_no_artifact() {
    let (_directory, input) = fixture();
    Command::new(assert_cmd::cargo::cargo_bin!("sbctl"))
        .args(["protocol", "export", "--format", "xray-server", "--input"])
        .arg(&input)
        .assert()
        .failure()
        .stdout("");
    std::fs::write(&input, r#"[{"PRIVATE-CREDENTIAL":"secret"}]"#).unwrap();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("sbctl"))
        .args(["protocol", "check", "--input"])
        .arg(&input)
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE-CREDENTIAL"));
    assert!(output.stdout.is_empty());
}
