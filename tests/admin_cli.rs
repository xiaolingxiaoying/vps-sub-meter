use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;
#[cfg(unix)]
use vps_sub_meter::secrets::SECRETS_REL;
use vps_sub_meter::{
    apply::{CURRENT_REL, SERVE_AUTH_REL},
    config::{CONFIG_REL, DEFAULT_CONFIG_REL},
};

struct Sandbox(TempDir);

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self(tempfile::tempdir().unwrap());
        sandbox.write(
            DEFAULT_CONFIG_REL,
            include_str!("../templates/default-config.toml"),
        );
        sandbox.write("sys/class/net/eth0/statistics/rx_bytes", "1000");
        sandbox.write("sys/class/net/eth0/statistics/tx_bytes", "3000");
        sandbox.write("proc/sys/kernel/random/boot_id", "test-boot");
        sandbox
    }

    fn root(&self) -> &Path {
        self.0.path()
    }

    fn write(&self, relative: &str, data: &str) -> PathBuf {
        let path = self.root().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();
        path
    }

    fn cli(&self, args: &[&str], input: Option<&Value>) -> Output {
        let mut process = Command::new(env!("CARGO_BIN_EXE_vps-sub-meter"))
            .arg("--root")
            .arg(self.root())
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = input {
            process
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())
                .unwrap();
        }
        process.wait_with_output().unwrap()
    }

    fn ok(&self, args: &[&str], input: Option<&Value>) -> String {
        let result = self.cli(args, input);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args, None)).unwrap()
    }

    fn file(&self, relative: &str) -> Vec<u8> {
        fs::read(self.root().join(relative)).unwrap()
    }

    fn prepare(&self, extra: &[&str]) -> Value {
        let mut args = vec![
            "config",
            "prepare",
            "--domain",
            "sub.example.com",
            "--public-ip",
            "203.0.113.20",
        ];
        args.extend(extra);
        self.json(&args)
    }
}

#[test]
fn config_preview_and_write_share_validated_ports() {
    let sandbox = Sandbox::new();
    assert!(!sandbox.cli(&["config", "show"], None).status.success());
    let defaults = sandbox.json(&["config", "show", "--defaults"]);
    let fixed = sandbox.prepare(&["--ports", "keep"]);
    assert_eq!(fixed["config"]["nodes"], defaults["nodes"]);
    assert_eq!(fixed["mode"], "https-shared");
    let preview = sandbox.prepare(&["--monthly-gib", "0", "--reset-day", "11"]);
    let mut ports = std::collections::BTreeSet::new();
    for node in preview["config"]["nodes"].as_object().unwrap().values() {
        let port = node["port"].as_u64().unwrap();
        assert!((20000..=29999).contains(&port));
        assert!(ports.insert(port));
    }
    assert_eq!(preview["tls_sni"], "sub.example.com");
    assert!(!sandbox.root().join(CONFIG_REL).exists());
    sandbox.ok(&["config", "write"], Some(&preview["config"]));
    assert_eq!(sandbox.json(&["config", "show"]), preview["config"]);
    assert_eq!(
        sandbox.json(&["config", "prepare"])["config"],
        preview["config"]
    );
    assert!(
        !sandbox
            .cli(&["config", "write"], Some(&fixed["config"]))
            .status
            .success()
    );
    let mut invalid = fixed["config"].clone();
    invalid["traffic"]["reset_day"] = json!(32);
    assert!(
        !sandbox
            .cli(&["config", "write", "--replace"], Some(&invalid))
            .status
            .success()
    );
    assert_eq!(sandbox.json(&["config", "show"]), preview["config"]);
    sandbox.ok(&["config", "write", "--replace"], Some(&fixed["config"]));
    assert_eq!(sandbox.json(&["config", "show"]), fixed["config"]);
}

#[test]
fn subscriptions_and_tokens_use_published_auth() {
    let sandbox = Sandbox::new();
    let mut config = sandbox.prepare(&[])["config"].clone();
    config["nodes"]["vless_reality_pqc"]["enabled"] = json!(false);
    config["nodes"]["vless_xhttp_pqc"]["enabled"] = json!(false);
    config["argo"]["enabled"] = json!(true);
    config["argo"]["tunnel_domain"] = json!("tunnel.example.com");
    config["argo"]["sub_domain"] = json!("argo.example.com");
    config["argo"]["ws_inbounds"][0]["engine"] = json!("sing-box");
    sandbox.ok(&["config", "write"], Some(&config));
    sandbox.ok(&["apply"], None);
    sandbox.ok(&["traffic", "checkpoint"], None);
    sandbox.write("sys/class/net/eth0/statistics/rx_bytes", "2000");
    let generation = sandbox.file(CURRENT_REL);
    let status = sandbox.json(&["status", "--json"]);
    assert_eq!(status["address"], "sub.example.com");
    assert!(status["generation"]["id"].is_string());
    assert_eq!(status["traffic"]["upload_bytes"], 1000);

    let original = sandbox.json(&["sub", "--json"]);
    assert!(
        original["direct"]["clash"]
            .as_str()
            .unwrap()
            .starts_with("https://sub.example.com/sub/")
    );
    assert!(
        original["argo"]["clash"]
            .as_str()
            .unwrap()
            .starts_with("https://argo.example.com/sub/")
    );
    for args in [
        &["sub", "--qr"][..],
        &["sub", "--qr-format", "sing-box"],
        &["sub", "--qr-format", "shadowrocket"],
    ] {
        assert!(sandbox.ok(args, None).contains("\u{1b}["));
    }

    let pending_auth: Value = serde_json::from_slice(&sandbox.file(SERVE_AUTH_REL)).unwrap();
    sandbox.ok(&["token", "add", "phone"], None);
    assert!(
        !sandbox
            .cli(&["token", "add", "phone"], None)
            .status
            .success()
    );
    let list = sandbox.json(&["token", "list", "--json"]);
    assert_eq!(list[1]["name"], "phone");
    assert_eq!(list[0]["default"], true);
    assert!(list[1].get("token").is_none());
    let phone = sandbox.json(&["sub", "--json", "--token-name", "phone"]);
    let phone_url = phone["direct"]["index"].as_str().unwrap();
    let token = phone_url.split('/').nth_back(1).unwrap();
    let auth: Value = serde_json::from_slice(&sandbox.file(SERVE_AUTH_REL)).unwrap();
    assert_eq!(
        auth["token_sha256"][1],
        format!("{:x}", Sha256::digest(token))
    );
    assert_eq!(auth["argo_ws"], pending_auth["argo_ws"]);
    assert_eq!(sandbox.file(CURRENT_REL), generation);
    sandbox.ok(&["token", "revoke", "phone"], None);
    assert!(
        !sandbox
            .cli(&["sub", "--token-name", "phone"], None)
            .status
            .success()
    );
    assert!(
        !sandbox
            .cli(&["token", "revoke", "default"], None)
            .status
            .success()
    );
    sandbox.ok(&["token", "rotate"], None);
    assert_ne!(sandbox.json(&["sub", "--json"]), original);
}

#[cfg(unix)]
#[test]
fn node_rotation_requires_confirmation_and_keeps_deployment_on_failed_preflight() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new();
    let mut config = sandbox.prepare(&[])["config"].clone();
    config["nodes"]["vless_reality_pqc"]["enabled"] = json!(false);
    config["nodes"]["vless_xhttp_pqc"]["enabled"] = json!(false);
    sandbox.ok(&["config", "write"], Some(&config));
    sandbox.ok(&["apply"], None);
    let secrets = sandbox.file(SECRETS_REL);
    let generation = sandbox.file(CURRENT_REL);
    let auth = sandbox.file(SERVE_AUTH_REL);
    assert!(!sandbox.cli(&["keys", "rotate"], None).status.success());
    let check = sandbox.write("usr/local/bin/sing-box", "#!/bin/sh\nexit 1\n");
    fs::set_permissions(check, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        !sandbox
            .cli(&["keys", "rotate", "--yes"], None)
            .status
            .success()
    );
    assert_eq!(sandbox.file(SECRETS_REL), secrets);
    assert_eq!(sandbox.file(CURRENT_REL), generation);
    assert_eq!(sandbox.file(SERVE_AUTH_REL), auth);

    sandbox.write("usr/local/bin/sing-box", "#!/bin/sh\nexit 0\n");
    let links = sandbox.json(&["sub", "--json"]);
    sandbox.ok(&["keys", "rotate", "--yes"], None);
    assert_eq!(sandbox.json(&["sub", "--json"]), links);
    assert_ne!(sandbox.file(SECRETS_REL), secrets);
    assert_ne!(sandbox.file(CURRENT_REL), generation);
}
