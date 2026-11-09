//! Opt-in real-core configuration checks, separate from service/connection tests.
use sbctl::canonical::CanonicalNode;
use sbctl::core::CoreKind;
use sbctl::core::xray::{BASELINE_VERSION, CertificatePaths, render_server};

#[test]
#[ignore = "requires the authenticated official Xray baseline via SBCTL_XRAY_BIN"]
fn generated_servers_pass_the_real_xray_baseline() {
    use base64::Engine;
    let binary =
        std::path::PathBuf::from(std::env::var_os("SBCTL_XRAY_BIN").expect("SBCTL_XRAY_BIN"));
    let version = std::process::Command::new(&binary)
        .arg("version")
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8_lossy(&version.stdout).starts_with(&format!("Xray {BASELINE_VERSION} "))
    );
    let directory = tempfile::tempdir().unwrap();
    let pair = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["example.com".into()])
        .unwrap()
        .self_signed(&pair)
        .unwrap();
    let cert_path = directory.path().join("cert.pem");
    let key_path = directory.path().join("key.pem");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, pair.serialize_pem()).unwrap();
    let nodes = [
        CanonicalNode::VlessReality {
            host: "example.com".into(),
            port: 23456,
            tls_server_name: "example.com".into(),
            uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
            private_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8; 32]),
            public_key: String::new(),
            short_id: "0123456789abcdef".into(),
            decoy_sni: "example.com".into(),
        },
        CanonicalNode::VmessWebsocket {
            host: "example.com".into(),
            port: 23457,
            tls_server_name: "example.com".into(),
            uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
            path: "/test".into(),
        },
    ];
    // Exercise each protocol and both IPv4 policies independently, plus the
    // combined document. A successful parse is not an interoperability claim.
    for ipv4_only in [false, true] {
        for selected in [&nodes[..1], &nodes[1..], &nodes[..]] {
            let document = render_server(
                selected,
                Some(CertificatePaths {
                    certificate: cert_path.to_str().unwrap(),
                    private_key: key_path.to_str().unwrap(),
                }),
                ipv4_only,
            )
            .unwrap();
            let rendered = serde_json::to_string(&document).unwrap();
            if let Err(error) = CoreKind::Xray.check_config(&binary, &rendered) {
                // This document contains only disposable test credentials.
                // Keep diagnostic reruns here, never in the production adapter.
                let diagnostic = directory.path().join("diagnostic.json");
                std::fs::write(&diagnostic, rendered).unwrap();
                let output = std::process::Command::new(&binary)
                    .args(["run", "-test", "-c"])
                    .arg(diagnostic)
                    .output()
                    .unwrap();
                panic!(
                    "{error}: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
    use sbctl::core::presets::{Node, Preset, Reality};
    for preset in [
        Preset::VlessXhttpReality,
        Preset::VlessXhttpTls,
        Preset::VlessWsTls,
        Preset::TrojanTcpTls,
    ] {
        let private = [1u8; 32];
        let encode = |key: [u8; 32]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);
        let node = Node {
            name: "candidate".into(),
            preset,
            server: "example.com".into(),
            port: 23458,
            server_name: "example.com".into(),
            credential: if preset.is_trojan() {
                "test-password".into()
            } else {
                "b831381d-6324-4d53-ad4f-8cda48b30811".into()
            },
            path: (!preset.is_trojan()).then(|| "/candidate".into()),
            reality: preset.is_reality().then(|| Reality {
                private_key: encode(private),
                public_key: encode(x25519_dalek::x25519(
                    private,
                    x25519_dalek::X25519_BASEPOINT_BYTES,
                )),
                short_id: "0123456789abcdef".into(),
                target: "example.com:443".into(),
            }),
        };
        let paths = CertificatePaths {
            certificate: cert_path.to_str().unwrap(),
            private_key: key_path.to_str().unwrap(),
        };
        let server = serde_json::json!({"inbounds": [node.xray_inbound(Some(&paths)).unwrap()], "outbounds": [{"protocol": "freedom"}]});
        let client = serde_json::json!({"outbounds": [node.xray_outbound().unwrap()]});
        CoreKind::Xray
            .check_config(&binary, &server.to_string())
            .unwrap();
        CoreKind::Xray
            .check_config(&binary, &client.to_string())
            .unwrap();
    }
}
