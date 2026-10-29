//! Real-core validation of the generated clash/mihomo artifacts.
//!
//! Ignored by default because it needs a mihomo binary that is not installed
//! with the suite. The CI `mihomo-profiles` job downloads a pinned mihomo and
//! sets `MIHOMO_BIN`, then runs `cargo test --test clash_mihomo -- --ignored`.

use std::fs;
use std::process::Command;

use sbctl::config::{ClientRuleProfile, DeploymentConfig, ManagedProtocol, SubscriptionMode};
use sbctl::subscription::{
    CLASH_LEGACY_VERSION, ClientTemplate, SubscriptionFormat, generated_artifacts,
};

#[test]
#[ignore = "requires a real mihomo binary; run by the CI mihomo-profiles job"]
fn generated_clash_artifacts_load_in_a_real_mihomo() {
    let Ok(binary) = std::env::var("MIHOMO_BIN") else {
        panic!("MIHOMO_BIN must point at a mihomo binary");
    };
    let legacy_binary = std::env::var("MIHOMO_LEGACY_BIN")
        .expect("MIHOMO_LEGACY_BIN must point at a mihomo 1.18 core");
    let version = Command::new(&legacy_binary).arg("-v").output().unwrap();
    assert!(
        String::from_utf8_lossy(&version.stdout).contains("v1.18."),
        "the legacy profile must be checked by a real 1.18 core"
    );
    let root = tempfile::tempdir().expect("temporary root is created");
    // One run per combination: 3 templates x 2 rule profiles x 2 clash
    // artifacts. Before the templates carried different content all three
    // rendered the same bytes, so a single run appeared to prove the whole axis.
    for (template, rule_profile) in [
        (ClientTemplate::Standard, ClientRuleProfile::Standard),
        (ClientTemplate::Global, ClientRuleProfile::Standard),
        (ClientTemplate::Split, ClientRuleProfile::Standard),
        (ClientTemplate::Compact, ClientRuleProfile::Standard),
        // `minimal` is the profile that promises no download at all, and its
        // clash artifact now spells CN/LAN out as compiled-in lists instead of
        // the `GEOIP,*` codes mihomo answers from a database it fetches on first
        // use. Only a real core can say whether those lists parse.
        (ClientTemplate::Standard, ClientRuleProfile::Minimal),
        (ClientTemplate::Global, ClientRuleProfile::Minimal),
        (ClientTemplate::Split, ClientRuleProfile::Minimal),
        (ClientTemplate::Compact, ClientRuleProfile::Minimal),
    ] {
        let mut config = DeploymentConfig::new(
            SubscriptionMode::IpFallback,
            "127.0.0.1".into(),
            None,
            Some(2080),
            "ens3".into(),
            vec![
                ManagedProtocol::VlessReality,
                ManagedProtocol::VmessWebsocket,
                ManagedProtocol::Hysteria2,
                ManagedProtocol::Tuic,
                ManagedProtocol::Anytls,
            ],
            Some("www.cloudflare.com".into()),
        )
        .expect("a five-protocol IP fallback deployment is valid");
        config.client_template = template.clone();
        config.client_rule_profile = rule_profile.clone();
        // Validate both the baseline and the file users can paste into the
        // runtime override editor, including the no-CDN compatibility output.
        for example in [false, true] {
            let directory = root.path().join("etc/sbctl/overrides");
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("clash-override.yaml");
            if example {
                fs::write(
                    &path,
                    include_str!("../examples/overrides/clash-override.yaml"),
                )
                .unwrap();
            } else if path.exists() {
                fs::remove_file(&path).unwrap();
            }
            let artifacts = generated_artifacts(&config, root.path()).expect("artifacts generate");

            for format in [
                SubscriptionFormat::Clash,
                SubscriptionFormat::ClashLegacy(CLASH_LEGACY_VERSION),
            ] {
                let name = format.artifact_name().into_owned();
                let contents = artifacts
                    .iter()
                    .find(|(artifact, _)| *artifact == name)
                    .map(|(_, contents)| contents.clone())
                    .unwrap_or_else(|| panic!("missing clash artifact {name}"));
                let config_path = root.path().join(&name);
                fs::write(&config_path, contents).expect("clash artifact is written");
                let checker = if matches!(format, SubscriptionFormat::ClashLegacy(_)) {
                    &legacy_binary
                } else {
                    &binary
                };
                let output = Command::new(checker)
                    .arg("-t")
                    .arg("-d")
                    .arg(root.path())
                    .arg("-f")
                    .arg(&config_path)
                    .output()
                    .expect("mihomo runs");
                assert!(
                    output.status.success(),
                    "mihomo rejected {name} (template={template}, rules={rule_profile}, example={example}):\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                eprintln!(
                    "mihomo accepted {name} (template={template}, rules={rule_profile}, example={example})"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires a real mihomo binary; run by the CI mihomo-profiles job"]
fn a_registered_mrs_provider_loads_actual_binary_rules() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    struct Core(Child);
    impl Drop for Core {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let binary = std::env::var("MIHOMO_BIN").expect("MIHOMO_BIN must point at a mihomo binary");
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("domains.txt");
    let compiled = root.path().join("domains.mrs");
    fs::write(&source, "+.review.example\n").unwrap();
    let converted = Command::new(&binary)
        .args(["convert-ruleset", "domain", "text"])
        .arg(&source)
        .arg(&compiled)
        .output()
        .unwrap();
    assert!(
        converted.status.success(),
        "{}",
        String::from_utf8_lossy(&converted.stderr)
    );
    let mut config = DeploymentConfig::new(
        SubscriptionMode::IpFallback,
        "127.0.0.1".into(),
        None,
        Some(2080),
        "ens3".into(),
        vec![ManagedProtocol::VlessReality],
        Some("www.cloudflare.com".into()),
    )
    .unwrap();
    config.client_extra_rule_sets = vec![
        sbctl::config::ExtraRuleSet {
            name: "review-srs".into(),
            url: "https://cdn.example/review.srs".into(),
            outbound: "reject".into(),
            behavior: Default::default(),
        },
        sbctl::config::ExtraRuleSet {
            name: "review-mrs".into(),
            url: "https://cdn.example/review.mrs".into(),
            outbound: "reject".into(),
            behavior: Default::default(),
        },
    ];
    let artifacts = generated_artifacts(&config, root.path()).unwrap();
    let (_, clash) = artifacts
        .iter()
        .find(|(name, _)| name == "subscription-clash.yaml")
        .unwrap();
    let value: serde_yaml::Value = serde_yaml::from_str(clash).unwrap();
    assert!(value["rule-providers"].get("review-srs").is_none());
    let mut provider = value["rule-providers"]["review-mrs"].clone();
    assert_eq!(
        provider["url"].as_str(),
        Some("https://cdn.example/review.mrs")
    );
    // Use the generated format/behavior with a real compiled payload. File
    // loading isolates parser compatibility from HTTPS/CDN availability.
    provider["type"] = serde_yaml::Value::String("file".into());
    provider["path"] = serde_yaml::Value::String(compiled.to_str().unwrap().into());
    provider
        .as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::String("url".into()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let runtime = serde_json::json!({
        "external-controller": address.to_string(),
        "log-level": "error",
        "rule-providers": {"review-mrs": provider},
        "rules": ["RULE-SET,review-mrs,REJECT", "MATCH,DIRECT"]
    });
    let path = root.path().join("runtime.yaml");
    fs::write(&path, serde_yaml::to_string(&runtime).unwrap()).unwrap();
    let log_path = root.path().join("core.log");
    let log = fs::File::create(&log_path).unwrap();
    drop(listener);
    let mut core = Core(
        Command::new(&binary)
            .arg("-d")
            .arg(root.path())
            .arg("-f")
            .arg(&path)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_response = String::new();
    loop {
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            stream
                .write_all(b"GET /providers/rules HTTP/1.0\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut response = String::new();
            if stream.read_to_string(&mut response).is_ok()
                && let Some((headers, body)) = response.split_once("\r\n\r\n")
                && headers.contains("200 OK")
                && let Ok(body) = serde_json::from_str::<serde_json::Value>(body)
                && body["providers"]["review-mrs"]["ruleCount"]
                    .as_u64()
                    .is_some_and(|count| count > 0)
            {
                break;
            }
            last_response = response;
        }
        assert!(
            core.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "MRS provider never loaded: {}\nlast API response: {last_response}",
            fs::read_to_string(&log_path).unwrap()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
