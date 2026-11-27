use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vps_sub_meter::apply;
use vps_sub_meter::config::{Config, DEFAULT_CONFIG_REL};

#[path = "support/secrets.rs"]
mod secrets;

struct Panel {
    child: Child,
    addr: SocketAddr,
    key: String,
    _stderr: BufReader<ChildStderr>,
}

impl Panel {
    fn launch(root: &std::path::Path) -> Self {
        let addr = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_vps-sub-meter"))
            .args(["--root"])
            .arg(root)
            .args(["panel", "--port", &addr.port().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let mut panel = Self {
            child,
            addr,
            key: String::new(),
            _stderr: stderr,
        };
        for line in (&mut panel._stderr).lines().take(2) {
            let line = line.unwrap();
            if let Some(key) = line.strip_prefix("本次会话密钥：") {
                panel.key = key.to_owned();
            }
        }
        assert_eq!(panel.key.len(), 43, "panel did not start");
        panel
    }

    fn request(
        &self,
        path: &str,
        authorized: bool,
        headers: &str,
        json_body: Option<Value>,
    ) -> (u16, String) {
        self.request_with_host(&self.addr.to_string(), path, authorized, headers, json_body)
    }

    fn request_with_host(
        &self,
        host: &str,
        path: &str,
        authorized: bool,
        headers: &str,
        json_body: Option<Value>,
    ) -> (u16, String) {
        let mut socket = TcpStream::connect_timeout(&self.addr, Duration::from_secs(5)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let method = if json_body.is_some() { "POST" } else { "GET" };
        write!(
            socket,
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{headers}"
        )
        .unwrap();
        if authorized {
            write!(socket, "Authorization: Bearer {}\r\n", self.key).unwrap();
        }
        if let Some(body) = json_body {
            let body = body.to_string();
            write!(
                socket,
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        } else {
            socket.write_all(b"\r\n").unwrap();
        }
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert!(headers.contains("cache-control: no-store"));
        (status, body.to_string())
    }

    fn json(&self, path: &str, body: Option<Value>, expected: u16) -> Value {
        let (status, response) = self.request(path, true, "", body);
        assert_eq!(status, expected, "{response}");
        serde_json::from_str(&response).unwrap()
    }

    fn job(&self, body: Value) -> Value {
        let job = self.json("/api/action", Some(body), 202);
        let path = format!("/api/jobs/{}", job["id"].as_str().unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let job = self.json(&path, None, 200);
            if job["state"] != "running" {
                return job;
            }
            assert!(Instant::now() < deadline, "job did not finish");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn panel_auth_preview_apply_and_concurrent_edit_protection() {
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path();
    for (relative, content) in [
        (
            DEFAULT_CONFIG_REL,
            include_str!("../templates/default-config.toml"),
        ),
        ("sys/class/net/eth0/statistics/rx_bytes", "1000"),
        ("sys/class/net/eth0/statistics/tx_bytes", "3000"),
        ("proc/sys/kernel/random/boot_id", "test-boot"),
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let panel = Panel::launch(root);
    let (status, page) = panel.request("/", false, "", None);
    assert_eq!(status, 200);
    assert!(page.contains("会话密钥"));
    assert!(!page.contains(&panel.key));
    assert_eq!(panel.request("/api/state", false, "", None).0, 401);
    assert_eq!(
        panel
            .request(
                "/api/state",
                true,
                "Origin: https://foreign.example\r\n",
                None
            )
            .0,
        403
    );
    assert_eq!(
        panel
            .request("/api/state", true, "Sec-Fetch-Site: cross-site\r\n", None)
            .0,
        403
    );

    let state = panel.json("/api/state", None, 200);
    assert_eq!(state["initialized"], false);
    assert_eq!(state["revision"], "");
    let mut config: Config = serde_json::from_value(state["config"].clone()).unwrap();
    config.domain = "sub.example.com".into();
    config.traffic.monthly_gb = 200.0;
    secrets::fixture(&config).save(root).unwrap();
    let preview = panel.json(
        "/api/action",
        Some(json!({"action": "preview", "config": config, "ports": "keep"})),
        200,
    );
    assert_eq!(preview["tls_sni"], "sub.example.com");
    assert!(!Config::path(root).exists());
    let initial = panel.job(json!({
        "action": "config", "change": "deploy", "config": preview["config"], "revision": "",
    }));
    assert_eq!(initial["state"], "done", "{initial}");
    let current = panel.json("/api/state", None, 200);
    assert_eq!(current["initialized"], true);
    assert_eq!(current["status"]["traffic"]["total_used_bytes"], 0);
    let checkpoint = panel.job(json!({"action": "traffic-checkpoint"}));
    assert_eq!(checkpoint["state"], "done", "{checkpoint}");
    fs::write(root.join("sys/class/net/eth0/statistics/rx_bytes"), "1500").unwrap();
    let traffic = panel.json("/api/state", None, 200);
    assert_eq!(traffic["status"]["traffic"]["upload_bytes"], 500);
    let calibration = panel.job(json!({
        "action": "traffic-set", "upload_gib": 1.25, "download_gib": 2.5
    }));
    assert_eq!(calibration["state"], "done", "{calibration}");
    let summary = calibration["result"]["summary"].as_str().unwrap();
    assert!(summary.contains("1.25 GiB") && summary.contains("2.50 GiB"));
    let checkpoint = panel.job(json!({"action": "traffic-checkpoint"}));
    assert_eq!(checkpoint["state"], "done", "{checkpoint}");
    let traffic = panel.json("/api/state", None, 200);
    assert_eq!(
        traffic["status"]["traffic"]["upload_bytes"],
        1_342_177_280_u64
    );
    assert_eq!(
        traffic["status"]["traffic"]["download_bytes"],
        2_684_354_560_u64
    );
    assert!(!current["generation"]["id"].as_str().unwrap().is_empty());

    let subs = panel.json(
        "/api/action",
        Some(json!({"action": "subscriptions", "name": null})),
        200,
    );
    assert!(
        subs["links"]["direct"]["clash"]
            .as_str()
            .unwrap()
            .starts_with("https://sub.example.com/sub/")
    );
    assert!(
        subs["qr"]["direct"]["sing_box"]
            .as_str()
            .unwrap()
            .contains("<svg")
    );

    config.traffic.monthly_gb = 80.0;
    apply::apply_candidate(root, config.clone()).unwrap();
    let stale = panel.job(json!({
        "action": "config", "change": "apply", "config": preview["config"], "revision": current["revision"],
    }));
    assert_eq!(stale["state"], "failed");
    assert!(
        stale["result"]["message"]
            .as_str()
            .unwrap()
            .contains("已修改")
    );
    assert_eq!(Config::load(root).unwrap().traffic.monthly_gb, 80.0);
    let stale_traffic = panel.job(json!({
        "action": "traffic-configure", "traffic": preview["config"]["traffic"], "revision": current["revision"],
    }));
    assert_eq!(stale_traffic["state"], "failed");
    let latest = panel.json("/api/state", None, 200);
    let settings = panel.job(json!({
        "action": "traffic-configure", "traffic": preview["config"]["traffic"], "revision": latest["revision"],
    }));
    assert_eq!(settings["state"], "done", "{settings}");
    assert_eq!(Config::load(root).unwrap().traffic.monthly_gb, 200.0);

    let token = panel.job(json!({"action": "token-add", "name": "手机"}));
    assert_eq!(token["state"], "done", "{token}");
    let tokens = panel.json("/api/tokens", None, 200);
    assert_eq!(tokens[1]["name"], "手机");
    assert!(tokens[1]["masked"].as_str().unwrap().contains("..."));
}

#[test]
fn panel_allows_localhost_and_forwarded_ports_and_reuses_argo_token() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use vps_sub_meter::{config::pct, qr};

    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path();
    for (relative, content) in [
        (
            DEFAULT_CONFIG_REL,
            include_str!("../templates/default-config.toml"),
        ),
        ("sys/class/net/eth0/statistics/rx_bytes", "1000"),
        ("sys/class/net/eth0/statistics/tx_bytes", "3000"),
        ("proc/sys/kernel/random/boot_id", "test-boot"),
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let panel = Panel::launch(root);

    // Localhost and SSH-forwarded local ports are allowed when Origin matches Host.
    assert_eq!(
        panel
            .request_with_host(
                &format!("localhost:{}", panel.addr.port()),
                "/api/session",
                true,
                &format!("Origin: http://localhost:{}\r\n", panel.addr.port()),
                None
            )
            .0,
        200
    );
    assert_eq!(
        panel
            .request_with_host(
                "127.0.0.1:19876",
                "/api/session",
                true,
                "Origin: http://127.0.0.1:19876\r\n",
                None
            )
            .0,
        200
    );
    // Foreign host or mismatched local port Origin is rejected.
    assert_eq!(
        panel
            .request_with_host("evil.example:8787", "/api/session", true, "", None)
            .0,
        403
    );
    assert_eq!(
        panel
            .request_with_host(
                "127.0.0.1:19876",
                "/api/session",
                true,
                "Origin: http://127.0.0.1:3000\r\n",
                None
            )
            .0,
        403
    );

    // Initialize deployment, then verify Argo token reuse and Shadowrocket sub:// QR.
    let state = panel.json("/api/state", None, 200);
    assert_eq!(state["argo_token_configured"], false);
    let mut config: Config = serde_json::from_value(state["config"].clone()).unwrap();
    config.domain = "sub.example.com".into();
    config.public_ip = "203.0.113.10".into();
    config.nodes.vless_reality_pqc.enabled = false;
    config.nodes.vless_xhttp_pqc.enabled = false;
    config.argo.ws_inbounds[0].engine = vps_sub_meter::config::EngineKind::SingBox;
    secrets::fixture(&config).save(root).unwrap();
    let initial = panel.job(json!({
        "action": "config", "change": "deploy", "config": config, "revision": "",
    }));
    assert_eq!(initial["state"], "done", "{initial}");

    // Shadowrocket QR should encode sub://<base64>#<profile> for direct app scan import.
    let subs = panel.json(
        "/api/action",
        Some(json!({"action": "subscriptions", "name": null})),
        200,
    );
    let sr_url = subs["links"]["direct"]["shadowrocket"].as_str().unwrap();
    let expected_sr_sub = format!(
        "sub://{}#{}",
        STANDARD.encode(sr_url.as_bytes()),
        pct(&config.profile_name)
    );
    assert_eq!(
        subs["qr"]["direct"]["shadowrocket"].as_str().unwrap(),
        qr::render_svg(&expected_sr_sub).unwrap()
    );

    // Enabling Argo without any saved token and empty token input fails cleanly.
    let latest = panel.json("/api/state", None, 200);
    let mut argo_cfg: Config = serde_json::from_value(latest["config"].clone()).unwrap();
    argo_cfg.argo.enabled = true;
    argo_cfg.argo.tunnel_domain = "argo.example.com".into();
    argo_cfg.argo.sub_domain = "argo.example.com".into();
    argo_cfg.argo.preferred_ips = Some(vec!["104.16.0.1".parse().unwrap()]);
    let missing_tok = panel.job(json!({
        "action": "config", "change": "argo-setup", "config": argo_cfg,
        "revision": latest["revision"], "tunnel_token": null,
    }));
    assert_eq!(missing_tok["state"], "failed");
    assert!(
        missing_tok["result"]["message"]
            .as_str()
            .unwrap()
            .contains("Token")
    );

    // Whitespace-only token file must not be treated as a configured token.
    let token_file = root.join("etc/cloudflared/vps-sub-meter-argo.token");
    fs::create_dir_all(token_file.parent().unwrap()).unwrap();
    fs::write(&token_file, "   \n").unwrap();
    let blank_tok_state = panel.json("/api/state", None, 200);
    assert_eq!(blank_tok_state["argo_token_configured"], false);

    // Once token file exists with a non-empty token, ArgoSetup with empty token succeeds and reports token/IP count.
    fs::write(&token_file, "saved-token\n").unwrap();
    let latest = panel.json("/api/state", None, 200);
    assert_eq!(latest["argo_token_configured"], true);
    let reused = panel.job(json!({
        "action": "config", "change": "argo-setup", "config": argo_cfg,
        "revision": latest["revision"], "tunnel_token": null,
    }));
    assert_eq!(reused["state"], "done", "{reused}");
    let after = panel.json("/api/state", None, 200);
    assert_eq!(after["argo_preferred_ip_count"], 1);
    assert_eq!(after["config"]["argo"]["enabled"], true);
}

#[test]
fn panel_pending_changes_partial_traffic_set_and_argo_token_guard() {
    let sandbox = tempfile::tempdir().unwrap();
    let root = sandbox.path();
    for (relative, content) in [
        (
            DEFAULT_CONFIG_REL,
            include_str!("../templates/default-config.toml"),
        ),
        ("sys/class/net/eth0/statistics/rx_bytes", "1000"),
        ("sys/class/net/eth0/statistics/tx_bytes", "3000"),
        ("proc/sys/kernel/random/boot_id", "test-boot"),
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let panel = Panel::launch(root);
    let state = panel.json("/api/state", None, 200);
    let mut config: Config = serde_json::from_value(state["config"].clone()).unwrap();
    config.domain = "sub.example.com".into();
    config.public_ip = "203.0.113.10".into();
    secrets::fixture(&config).save(root).unwrap();

    let deploy = panel.job(json!({
        "action": "config", "change": "deploy", "config": config, "revision": "",
    }));
    assert_eq!(deploy["state"], "done", "{deploy}");
    let deployed = panel.json("/api/state", None, 200);
    assert_eq!(deployed["pending_changes"], false);

    // Traffic schedule update does not mark deployment as pending.
    let mut next_traffic = config.traffic.clone();
    next_traffic.monthly_gb = 300.0;
    let tc = panel.job(json!({
        "action": "traffic-configure", "traffic": next_traffic, "revision": deployed["revision"],
    }));
    assert_eq!(tc["state"], "done", "{tc}");
    let after_tc = panel.json("/api/state", None, 200);
    assert_eq!(after_tc["pending_changes"], false);

    // Saving a port change without apply MUST flag pending_changes.
    let mut saved_cfg: Config = serde_json::from_value(after_tc["config"].clone()).unwrap();
    saved_cfg.nodes.anytls_tcp.port = 24443;
    let save = panel.job(json!({
        "action": "config", "change": "save", "config": saved_cfg, "revision": after_tc["revision"],
    }));
    assert_eq!(save["state"], "done", "{save}");
    let after_save = panel.json("/api/state", None, 200);
    assert_eq!(after_save["pending_changes"], true);

    // Submitting a non-empty tunnel_token with change=apply must be rejected instead of silently ignored.
    let rejected_tok = panel.job(json!({
        "action": "config", "change": "apply", "config": saved_cfg,
        "revision": after_save["revision"], "tunnel_token": "secret-new-token",
    }));
    assert_eq!(rejected_tok["state"], "failed");
    assert!(
        rejected_tok["result"]["message"]
            .as_str()
            .unwrap()
            .contains("Token")
    );

    // Applying clears pending_changes.
    let applied = panel.job(json!({"action": "apply"}));
    assert_eq!(applied["state"], "done", "{applied}");
    let after_apply = panel.json("/api/state", None, 200);
    assert_eq!(after_apply["pending_changes"], false);

    // Partial traffic calibration preserves exact byte count for the omitted direction.
    let full_set = panel.job(json!({
        "action": "traffic-set", "upload_gib": 1.25, "download_gib": 2.5
    }));
    assert_eq!(full_set["state"], "done", "{full_set}");
    // Simulate +777 raw rx bytes (client upload) so upload is non-round (1_342_177_280 + 777).
    fs::write(root.join("sys/class/net/eth0/statistics/rx_bytes"), "1777").unwrap();
    let partial = panel.job(json!({
        "action": "traffic-set", "upload_gib": null, "download_gib": 3.0
    }));
    assert_eq!(partial["state"], "done", "{partial}");
    let after_partial = panel.json("/api/state", None, 200);
    assert_eq!(
        after_partial["status"]["traffic"]["upload_bytes"],
        1_342_177_280_u64 + 777
    );
    assert_eq!(
        after_partial["status"]["traffic"]["download_bytes"],
        3_221_225_472_u64
    );

    // Empty or negative traffic-set requests are rejected cleanly.
    let empty_set = panel.job(json!({
        "action": "traffic-set", "upload_gib": null, "download_gib": null
    }));
    assert_eq!(empty_set["state"], "failed");
    let neg_set = panel.job(json!({
        "action": "traffic-set", "upload_gib": -1.0, "download_gib": null
    }));
    assert_eq!(neg_set["state"], "failed");

    // Xray-only deployment omits sing_box links & QR without erroring.
    let mut xray_only: Config = serde_json::from_value(after_partial["config"].clone()).unwrap();
    xray_only.nodes.anytls_tcp.enabled = false;
    xray_only.nodes.hysteria2_hop.enabled = false;
    xray_only.nodes.tuic_bbr.enabled = false;
    let apply_xray = panel.job(json!({
        "action": "config", "change": "apply", "config": xray_only,
        "revision": after_partial["revision"],
    }));
    assert_eq!(apply_xray["state"], "done", "{apply_xray}");
    let subs = panel.json(
        "/api/action",
        Some(json!({"action": "subscriptions", "name": null})),
        200,
    );
    assert!(subs["links"]["direct"].get("sing_box").is_none());
    assert!(subs["qr"]["direct"]["sing_box"].is_null());
}
