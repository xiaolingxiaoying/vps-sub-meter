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
    config::{CONFIG_REL, DEFAULT_CONFIG_REL},
    store::{CURRENT_REL, SERVE_AUTH_REL},
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
        let input = input.map(Value::to_string);
        self.cli_with_input(args, input.as_deref())
    }

    fn cli_with_input(&self, args: &[&str], input: Option<&str>) -> Output {
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
                .write_all(input.as_bytes())
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
    let customized = sandbox.prepare(&[
        "--ports",
        "defaults",
        "--port",
        "anytls_tcp=24443",
        "--port",
        "vless_reality_pqc=24444",
    ]);
    assert_eq!(customized["config"]["nodes"]["anytls_tcp"]["port"], 24443);
    assert_eq!(
        customized["config"]["nodes"]["vless_reality_pqc"]["port"],
        24444
    );
    assert_eq!(customized["mode"], "https");
    assert!(
        !sandbox
            .cli(
                &[
                    "config",
                    "prepare",
                    "--domain=sub.example.com",
                    "--port=anytls_tcp=2053"
                ],
                None
            )
            .status
            .success()
    );
    let preview = sandbox.prepare(&["--monthly-gib", "0", "--reset-day", "11"]);
    assert_eq!(preview["mode"], "https-shared");
    assert_eq!(preview["config"]["nodes"]["vless_reality_pqc"]["port"], 443);
    let mut ports = std::collections::BTreeSet::new();
    for (key, node) in preview["config"]["nodes"].as_object().unwrap() {
        let port = node["port"].as_u64().unwrap();
        if key == "vless_reality_pqc" {
            assert_eq!(port, 443);
        } else {
            assert!((20000..=29999).contains(&port), "{key}: {port}");
        }
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
    let argo_link = original["argo"]["shadowrocket"].as_str().unwrap();
    let direct_link = original["direct"]["shadowrocket"].as_str().unwrap();
    let output = sandbox.ok(&["sub", "--argo", "--qr-format", "shadowrocket"], None);
    assert!(output.contains(argo_link));
    assert!(!output.contains(direct_link));
    assert!(output.contains(&vps_sub_meter::qr::render_ansi(argo_link).unwrap()));

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
    assert!(auth.get("argo_ws").is_none());
    let gen_id = status["generation"]["id"].as_str().unwrap();
    assert!(
        sandbox
            .root()
            .join("var/lib/vps-sub-meter/generations")
            .join(gen_id)
            .join("argo/clash.yaml")
            .is_file()
    );
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
    let unit = sandbox
        .root()
        .join("etc/systemd/system/vps-sub-meter.service");
    let hidden_unit = unit.with_extension("saved");
    fs::rename(&unit, &hidden_unit).unwrap();
    fs::create_dir(&unit).unwrap();
    assert!(
        !sandbox
            .cli(&["keys", "rotate", "--yes"], None)
            .status
            .success()
    );
    assert_eq!(sandbox.file(SECRETS_REL), secrets);
    assert_eq!(sandbox.file(CURRENT_REL), generation);
    assert_eq!(sandbox.file(SERVE_AUTH_REL), auth);
    fs::remove_dir(&unit).unwrap();
    fs::rename(hidden_unit, unit).unwrap();

    let links = sandbox.json(&["sub", "--json"]);
    sandbox.ok(&["keys", "rotate", "--yes"], None);
    assert_eq!(sandbox.json(&["sub", "--json"]), links);
    assert_ne!(sandbox.file(SECRETS_REL), secrets);
    assert_ne!(sandbox.file(CURRENT_REL), generation);
}

#[cfg(unix)]
#[test]
fn bash_menu_cancels_before_writes_and_reuses_previewed_ports() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new();
    let script_path = "usr/local/lib/vps-sub-meter/ly.sh";
    sandbox.write(script_path, include_str!("../scripts/ly.sh"));
    let executable = sandbox.root().join("usr/local/bin/vps-sub-meter");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    symlink(env!("CARGO_BIN_EXE_vps-sub-meter"), executable).unwrap();
    let mut defaults: toml::Table =
        toml::from_str(include_str!("../templates/default-config.toml")).unwrap();
    defaults["nodes"]["vless_reality_pqc"]["enabled"] = toml::Value::Boolean(false);
    defaults["nodes"]["vless_xhttp_pqc"]["enabled"] = toml::Value::Boolean(false);
    sandbox.write(DEFAULT_CONFIG_REL, &toml::to_string(&defaults).unwrap());

    let run_menu = |input: &str| -> Output {
        let mut process = Command::new("/bin/bash")
            .arg(sandbox.root().join(script_path))
            .arg("--root")
            .arg(sandbox.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        process.wait_with_output().unwrap()
    };

    let inputs = [
        "1\n",
        "sub.example.com\n",
        "203.0.113.20\n",
        "\n",
        "\n",
        "2\n",
        "\n",
        "\n",
    ];
    let mut prefix = String::new();
    for input in inputs {
        for attempt in [prefix.clone(), format!("{prefix}0\n0\n")] {
            let output = run_menu(&attempt);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            for path in [CONFIG_REL, SECRETS_REL, CURRENT_REL, SERVE_AUTH_REL] {
                assert!(!sandbox.root().join(path).exists(), "{path}");
            }
        }
        prefix.push_str(input);
    }

    let output = run_menu(&format!("{prefix}0\n"));
    assert!(output.status.success());
    assert!(output.stderr.is_empty() || !String::from_utf8_lossy(&output.stderr).contains("错误:"));
    let screen = String::from_utf8(output.stdout).unwrap();
    let config = sandbox.json(&["config", "show"]);
    for node in config["nodes"].as_object().unwrap().values() {
        let name = node["name"].as_str().unwrap();
        let port = node["port"].as_u64().unwrap();
        assert!(screen.contains(&format!("{name}: {port}")));
        if name == "vless-reality-pqc" {
            assert_eq!(port, 443);
        } else {
            assert!((20000..=29999).contains(&port), "{name}: {port}");
        }
    }
    assert!(sandbox.root().join(CURRENT_REL).exists());

    let output = run_menu("1\n\n\n\n\n\n\n\n0\n");
    assert!(output.status.success());
    assert_eq!(sandbox.json(&["config", "show"]), config);

    let cancelled = run_menu("11\n3\n36041\n0\n0\n");
    assert!(cancelled.status.success());
    assert_eq!(sandbox.json(&["config", "show"]), config);
    let changed = run_menu("11\n3\n36041\n1\n0\n");
    assert!(changed.status.success());
    let mut expected = config;
    expected["nodes"]["anytls_tcp"]["port"] = json!(36041);
    assert_eq!(sandbox.json(&["config", "show"]), expected);
}

#[cfg(unix)]
#[test]
fn bash_menu_updates_traffic_schedule_without_losing_usage() {
    use std::os::unix::fs::symlink;
    use vps_sub_meter::traffic::TRAFFIC_REL;

    let sandbox = Sandbox::new();
    let script = sandbox.write(
        "usr/local/lib/vps-sub-meter/ly.sh",
        include_str!("../scripts/ly.sh"),
    );
    let executable = sandbox.root().join("usr/local/bin/vps-sub-meter");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    symlink(env!("CARGO_BIN_EXE_vps-sub-meter"), executable).unwrap();
    let mut config = sandbox.prepare(&["--ports", "keep"])["config"].clone();
    config["traffic"]["reset_day"] = json!(11);
    sandbox.ok(&["config", "write"], Some(&config));

    let run_menu = |input: &str| {
        let mut process = Command::new("/bin/bash")
            .arg(&script)
            .arg("--root")
            .arg(sandbox.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = process.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };

    let calibration = run_menu("10\n2\n0.0\n2.5\n1\n0\n");
    assert!(String::from_utf8_lossy(&calibration.stdout).contains("已校准"));
    sandbox.write("sys/class/net/eth0/statistics/rx_bytes", "1300");
    let before = sandbox.json(&["status", "--json"]);
    let original_state = sandbox.file(TRAFFIC_REL);
    run_menu("10\n1\n0.0\n23\n09:45\nUTC\nAsia/Tokyo\n0\n0\n");
    let invalid = run_menu("10\n1\n0.0\n23\n09:45\nInvalid/Zone\nAsia/Tokyo\n1\n0\n");
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("不是有效 IANA 时区"));
    assert_eq!(sandbox.file(TRAFFIC_REL), original_state);
    assert_eq!(sandbox.json(&["config", "show"]), config);

    let changed = run_menu("10\n1\n0.0\n23\n09:45\nUTC\nAsia/Tokyo\n1\n0\n");
    assert!(String::from_utf8_lossy(&changed.stdout).contains("流量设置已生效"));
    let after = sandbox.json(&["status", "--json"]);
    assert_eq!(after["traffic"]["upload_bytes"], 300);
    assert_eq!(after["traffic"]["download_bytes"], 2_684_354_560_u64);
    assert_eq!(after["traffic"]["monthly_limit_bytes"], 0);
    assert_eq!(after["traffic"]["display_tz"], "Asia/Tokyo");
    assert_ne!(
        after["traffic"]["next_reset_ts"],
        before["traffic"]["next_reset_ts"]
    );
    let date =
        chrono::DateTime::from_timestamp(after["traffic"]["next_reset_ts"].as_i64().unwrap(), 0)
            .unwrap();
    assert_eq!(date.format("%d %H:%M").to_string(), "23 09:45");
    assert_eq!(sandbox.json(&["config", "show"])["nodes"], config["nodes"]);
    assert!(!sandbox.root().join(CURRENT_REL).exists());

    let checkpoint = run_menu("10\n3\n1\n0\n");
    assert!(String::from_utf8_lossy(&checkpoint.stdout).contains("已保存流量账本"));
    assert_eq!(
        sandbox.json(&["status", "--json"])["traffic"],
        after["traffic"]
    );
}

#[cfg(unix)]
#[test]
fn bash_menu_domain_config_switches_ip_http_to_https() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new();
    let script_path = "usr/local/lib/vps-sub-meter/ly.sh";
    sandbox.write(script_path, include_str!("../scripts/ly.sh"));
    let executable = sandbox.root().join("usr/local/bin/vps-sub-meter");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    symlink(env!("CARGO_BIN_EXE_vps-sub-meter"), executable).unwrap();

    let mut cfg = sandbox.json(&["config", "show", "--defaults"]);
    cfg["domain"] = json!("");
    cfg["public_ip"] = json!("203.0.113.20");
    cfg["nodes"]["vless_reality_pqc"]["enabled"] = json!(false);
    cfg["nodes"]["vless_xhttp_pqc"]["enabled"] = json!(false);
    sandbox.write(
        "var/lib/vps-sub-meter/certificates/www.bing.com/fullchain.pem",
        include_str!("support/ip-mode-cert.txt"),
    );
    sandbox.ok(&["config", "write"], Some(&cfg));
    sandbox.ok(&["apply"], None);

    let before_sub = sandbox.json(&["sub", "--json"]);
    assert!(
        before_sub["direct"]["clash"]
            .as_str()
            .unwrap()
            .starts_with("http://203.0.113.20:2080/sub/")
    );
    assert_eq!(sandbox.json(&["status", "--json"])["https_enabled"], false);

    let run_menu = |input: &str| -> Output {
        let mut process = Command::new("/bin/bash")
            .arg(sandbox.root().join(script_path))
            .arg("--root")
            .arg(sandbox.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        process.wait_with_output().unwrap()
    };

    // Pure IP must be rejected when binding domain for HTTPS
    let rejected = run_menu("6\n1\n203.0.113.20\n0\n");
    assert!(rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("不能为纯 IP"));
    assert_eq!(sandbox.json(&["config", "show"])["domain"], "");

    // Valid domain switches subscription from HTTP fallback to HTTPS while preserving ports
    let switched = run_menu("6\n1\nsub.example.com\n\n1\n0\n");
    assert!(
        switched.status.success(),
        "{}",
        String::from_utf8_lossy(&switched.stderr)
    );
    let after_cfg = sandbox.json(&["config", "show"]);
    assert_eq!(after_cfg["domain"], "sub.example.com");
    assert_eq!(after_cfg["nodes"], cfg["nodes"]);

    let after_sub = sandbox.json(&["sub", "--json"]);
    assert!(
        after_sub["direct"]["clash"]
            .as_str()
            .unwrap()
            .starts_with("https://sub.example.com/sub/")
    );
    assert_eq!(sandbox.json(&["status", "--json"])["https_enabled"], true);
}

#[cfg(unix)]
#[test]
fn uninstall_script_cleans_sandbox_and_respects_keep_flags() {
    let seed_sandbox = |sandbox: &Sandbox| {
        for rel in [
            "etc/vps-sub-meter/config.toml",
            "etc/vps-sub-meter/secrets.json",
            "var/lib/vps-sub-meter/traffic-state.json",
            "var/lib/vps-sub-meter/current.json",
            "var/lib/vps-sub-meter/serve-auth.json",
            "var/lib/vps-sub-meter/generations/g1/clash.yaml",
            "var/lib/vps-sub-meter/certificates/sub.example.com/fullchain.pem",
            "etc/systemd/system/vps-sub-meter.service",
            "etc/systemd/system/vps-sub-meter-xray.service",
            "etc/systemd/system/vps-sub-meter-singbox.service",
            "etc/systemd/system/vps-sub-meter-argo.service",
            "etc/sysctl.d/99-vps-sub-meter.conf",
            "etc/letsencrypt/renewal-hooks/deploy/vps-sub-meter-deploy-hook",
            "etc/letsencrypt/live/sub.example.com/fullchain.pem",
            "etc/cloudflared/vps-sub-meter-argo.token",
            "usr/local/bin/vps-sub-meter",
            "usr/local/bin/vsm",
            "usr/local/bin/ly",
            "usr/local/bin/xray",
            "usr/local/bin/sing-box",
            "usr/local/bin/cloudflared",
            "usr/local/share/xray/geoip.dat",
            "usr/local/lib/vps-sub-meter/ly.sh",
        ] {
            sandbox.write(rel, "fixture");
        }
        sandbox.write(
            "usr/local/lib/vps-sub-meter/uninstall.sh",
            include_str!("../scripts/uninstall.sh"),
        );
    };

    let run_uninstall = |sandbox: &Sandbox, args: &[&str], stdin_data: Option<&str>| -> Output {
        let script = sandbox
            .root()
            .join("usr/local/lib/vps-sub-meter/uninstall.sh");
        let mut process = Command::new("/bin/bash")
            .arg(script)
            .arg("--root")
            .arg(sandbox.root())
            .args(args)
            .stdin(if stdin_data.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(data) = stdin_data {
            process
                .stdin
                .take()
                .unwrap()
                .write_all(data.as_bytes())
                .unwrap();
        }
        process.wait_with_output().unwrap()
    };

    // 1. 取消卸载 (输入 0 返回 10，保留所有文件)
    let sandbox = Sandbox::new();
    seed_sandbox(&sandbox);
    let cancelled = run_uninstall(&sandbox, &[], Some("0\n"));
    assert_eq!(cancelled.status.code(), Some(10));
    assert!(sandbox.root().join("usr/local/bin/vps-sub-meter").exists());
    assert!(
        sandbox
            .root()
            .join("etc/vps-sub-meter/config.toml")
            .exists()
    );

    // 2. --keep-config --keep-cores --yes 保留配置、账本和代理内核
    let keep = run_uninstall(&sandbox, &["--keep-config", "--keep-cores", "--yes"], None);
    assert!(
        keep.status.success(),
        "{}",
        String::from_utf8_lossy(&keep.stderr)
    );
    assert!(
        sandbox
            .root()
            .join("etc/vps-sub-meter/config.toml")
            .exists()
    );
    assert!(
        sandbox
            .root()
            .join("var/lib/vps-sub-meter/traffic-state.json")
            .exists()
    );
    assert!(sandbox.root().join("usr/local/bin/xray").exists());
    assert!(
        sandbox
            .root()
            .join("etc/letsencrypt/live/sub.example.com/fullchain.pem")
            .exists()
    );
    for removed in [
        "usr/local/bin/vps-sub-meter",
        "usr/local/bin/vsm",
        "usr/local/bin/ly",
        "usr/local/lib/vps-sub-meter",
        "etc/systemd/system/vps-sub-meter.service",
        "etc/sysctl.d/99-vps-sub-meter.conf",
        "etc/letsencrypt/renewal-hooks/deploy/vps-sub-meter-deploy-hook",
        "etc/cloudflared/vps-sub-meter-argo.token",
        "var/lib/vps-sub-meter/generations",
        "var/lib/vps-sub-meter/current.json",
    ] {
        assert!(!sandbox.root().join(removed).exists(), "{removed}");
    }

    // 3. 默认彻底卸载：自动备份后清理配置、账本与内核，但仍保留 Let's Encrypt 证书
    let full_sandbox = Sandbox::new();
    seed_sandbox(&full_sandbox);
    let full = run_uninstall(&full_sandbox, &["--yes"], None);
    assert!(
        full.status.success(),
        "{}",
        String::from_utf8_lossy(&full.stderr)
    );
    assert!(!full_sandbox.root().join("etc/vps-sub-meter").exists());
    assert!(!full_sandbox.root().join("var/lib/vps-sub-meter").exists());
    assert!(!full_sandbox.root().join("usr/local/bin/xray").exists());
    assert!(!full_sandbox.root().join("usr/local/bin/sing-box").exists());
    assert!(
        !full_sandbox
            .root()
            .join("usr/local/bin/cloudflared")
            .exists()
    );
    assert!(!full_sandbox.root().join("usr/local/share/xray").exists());
    assert!(
        full_sandbox
            .root()
            .join("etc/letsencrypt/live/sub.example.com/fullchain.pem")
            .exists()
    );
    let backups: Vec<_> = fs::read_dir(full_sandbox.root().join("root"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("vsm-uninstall-backup-") && n.ends_with(".tar.gz"))
        })
        .collect();
    assert_eq!(backups.len(), 1);
    let tar_list = Command::new("tar")
        .arg("-tzf")
        .arg(&backups[0])
        .output()
        .unwrap();
    assert!(tar_list.status.success());
    let listing = String::from_utf8_lossy(&tar_list.stdout);
    assert!(listing.contains("etc/vps-sub-meter/config.toml"));
    assert!(listing.contains("var/lib/vps-sub-meter/traffic-state.json"));
}
