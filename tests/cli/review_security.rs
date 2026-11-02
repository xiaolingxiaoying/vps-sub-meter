use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn malformed_configuration_never_echoes_credentials_in_command_errors() {
    let fixture = TempDir::new().expect("temporary root");
    let config_path = fixture.path().join("etc/sbctl/config.toml");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let secret = "private-subscription-credential-that-must-not-appear";
    for contents in [
        format!("subscription_credential = \"{secret}\" invalid\n"),
        format!("monthly_traffic_limit = \"{secret}\"\n"),
    ] {
        fs::write(&config_path, &contents).unwrap();
        for arguments in [
            vec!["status"],
            vec!["status", "--json"],
            vec!["config", "show"],
            vec!["config", "validate"],
            vec!["credential", "list"],
        ] {
            Command::cargo_bin("sbctl")
                .expect("sbctl binary")
                .arg("--root")
                .arg(fixture.path())
                .args(&arguments)
                .assert()
                .code(2)
                .stdout(predicate::str::contains(secret).not())
                .stderr(predicate::str::contains(secret).not())
                .stderr(predicate::str::contains(
                    "could not parse deployment configuration",
                ));
            assert_eq!(fs::read_to_string(&config_path).unwrap(), contents);
        }
    }
}
