//! Opt-in local data-path tests. No systemd or public listeners are modified.
use base64::Engine;
use sbctl::core::presets::{Node, Preset, Reality};
use sbctl::core::xray::CertificatePaths;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
#[derive(Clone, Copy)]
enum ClientKind {
    Xray,
    SingBox,
    Mihomo,
}

fn start(binary: &std::path::Path, path: &std::path::Path, port: u16, kind: ClientKind) -> Process {
    let mut command = Command::new(binary);
    match kind {
        ClientKind::Xray | ClientKind::SingBox => {
            command.args(["run", "-c"]);
        }
        ClientKind::Mihomo => {
            command.arg("-d").arg(path.parent().unwrap()).arg("-f");
        }
    }
    let mut child = Process(
        command
            .arg(path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "core exited during startup"
        );
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return child;
        }
        assert!(Instant::now() < deadline, "core did not open its listener");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires official Xray via SBCTL_XRAY_BIN and curl"]
fn tls_presets_carry_http_through_a_real_socks_client() {
    exercise(
        &[
            Preset::VlessXhttpTls,
            Preset::VlessWsTls,
            Preset::TrojanTcpTls,
        ],
        None,
        ClientKind::Xray,
    );
}

#[test]
#[ignore = "requires official Xray, curl and an explicitly selected external Reality target"]
fn xhttp_reality_uses_external_decoy() {
    let target = std::env::var("SBCTL_REALITY_TARGET").expect("SBCTL_REALITY_TARGET=host:port");
    exercise(
        &[Preset::VlessXhttpReality],
        Some(&target),
        ClientKind::Xray,
    );
}

#[test]
#[ignore = "requires Xray and SBCTL_CLIENT_BIN pointing to sing-box"]
fn sing_box_compatible_presets_carry_http() {
    exercise(
        &[Preset::VlessWsTls, Preset::TrojanTcpTls],
        None,
        ClientKind::SingBox,
    );
}

#[test]
#[ignore = "requires Xray and SBCTL_CLIENT_BIN pointing to Mihomo 1.19.33"]
fn mihomo_compatible_presets_carry_http() {
    exercise(
        &[
            Preset::VlessXhttpTls,
            Preset::VlessWsTls,
            Preset::TrojanTcpTls,
        ],
        None,
        ClientKind::Mihomo,
    );
}

fn exercise(presets: &[Preset], reality_target: Option<&str>, kind: ClientKind) {
    let binary =
        std::path::PathBuf::from(std::env::var_os("SBCTL_XRAY_BIN").expect("SBCTL_XRAY_BIN"));
    let client_binary = match kind {
        ClientKind::Xray => binary.clone(),
        _ => std::path::PathBuf::from(
            std::env::var_os("SBCTL_CLIENT_BIN").expect("SBCTL_CLIENT_BIN"),
        ),
    };
    let directory = tempfile::tempdir().unwrap();
    let pair = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .self_signed(&pair)
        .unwrap();
    let cert_path = directory.path().join("cert.pem");
    let key_path = directory.path().join("key.pem");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, pair.serialize_pem()).unwrap();
    let paths = CertificatePaths {
        certificate: cert_path.to_str().unwrap(),
        private_key: key_path.to_str().unwrap(),
    };
    for &preset in presets {
        // Reality includes an external handshake and stream-one buffering;
        // give it a distinct network budget rather than a local-only timeout.
        let request_timeout = if preset.is_reality() { 30 } else { 10 };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let origin = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(request_timeout + 5);
            while Instant::now() < deadline {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 4096];
                    let _ = stream.read(&mut request);
                    stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 9\r\n\r\npreset-ok").unwrap();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let server_port = port();
        let mut socks_port = port();
        while socks_port == server_port {
            socks_port = port();
        }
        let reality = reality_target.map(|target| {
            let encode =
                |key: [u8; 32]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);
            Reality {
                private_key: encode([1; 32]),
                public_key: encode(x25519_dalek::x25519(
                    [1; 32],
                    x25519_dalek::X25519_BASEPOINT_BYTES,
                )),
                short_id: "0123456789abcdef".into(),
                target: target.into(),
            }
        });
        let server_name = match reality_target {
            Some(target) => url::Url::parse(&format!("tcp://{target}"))
                .unwrap()
                .host_str()
                .unwrap()
                .to_owned(),
            None => "localhost".into(),
        };
        let node = Node {
            name: "test".into(),
            preset,
            server: "127.0.0.1".into(),
            port: server_port,
            server_name,
            credential: if preset.is_trojan() {
                "fixture-password".into()
            } else {
                "b831381d-6324-4d53-ad4f-8cda48b30811".into()
            },
            path: (!preset.is_trojan()).then(|| "/test".into()),
            reality,
        };
        let mut inbound = node.xray_inbound(Some(&paths)).unwrap();
        inbound["listen"] = json!("127.0.0.1");
        // Test-only exception for the isolated loopback HTTP fixture. Exported
        // production configurations retain Xray's default private-target block.
        let mut server = json!({"inbounds": [inbound], "outbounds": [{"protocol": "freedom", "settings":{"finalRules":[{"action":"allow", "ip":["127.0.0.1/32"]}]}}]});
        let mut outbound = node.xray_outbound().unwrap();
        // Trust only this disposable leaf certificate; do not disable verification.
        let pin = Sha256::digest(cert.der())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if !preset.is_reality() {
            outbound["streamSettings"]["tlsSettings"]["pinnedPeerCertSha256"] = json!(pin);
        }
        let client = json!({"inbounds": [{"listen":"127.0.0.1", "port": socks_port, "protocol":"socks", "settings":{"auth":"noauth"}}], "outbounds":[outbound]});
        let mut client = match kind {
            ClientKind::Xray => client,
            ClientKind::SingBox => {
                let mut outbound = node.sing_box_outbound().unwrap();
                outbound["tls"]["certificate"] = json!([cert.pem()]);
                json!({"log":{"level":"warn"}, "inbounds":[{"type":"socks", "listen":"127.0.0.1", "listen_port":socks_port}], "outbounds":[outbound], "route":{"final":"test"}})
            }
            ClientKind::Mihomo => {
                let mut proxy = node.mihomo_proxy(true).unwrap();
                proxy["fingerprint"] = json!(pin);
                json!({"socks-port":socks_port, "allow-lan":false, "mode":"rule", "log-level":"warning", "proxies":[proxy], "rules":["MATCH,test"]})
            }
        };
        if std::env::var_os("SBCTL_TEST_DEBUG").is_some() {
            server["log"] = json!({"loglevel":"debug"});
            if matches!(kind, ClientKind::Xray) {
                client["log"] = json!({"loglevel":"debug"});
            }
        }
        let server_path = directory.path().join("server.json");
        let client_path = directory.path().join("client.json");
        std::fs::write(&server_path, server.to_string()).unwrap();
        std::fs::write(&client_path, client.to_string()).unwrap();
        let _server = start(&binary, &server_path, server_port, ClientKind::Xray);
        let _client = start(&client_binary, &client_path, socks_port, kind);
        let response = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--max-time",
                if preset.is_reality() { "30" } else { "10" },
                "--noproxy",
                "",
                "--socks5-hostname",
            ])
            .arg(format!("127.0.0.1:{socks_port}"))
            .arg(format!("http://127.0.0.1:{origin_port}/"))
            .output()
            .unwrap();
        origin.join().unwrap();
        assert!(
            response.status.success(),
            "{preset:?}: {}",
            String::from_utf8_lossy(&response.stderr)
        );
        assert_eq!(response.stdout, b"preset-ok", "{preset:?}");
    }
}
