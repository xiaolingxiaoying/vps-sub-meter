use sbctl::config::{DeploymentConfig, DeploymentStore, ManagedProtocol, SubscriptionMode};
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[test]
#[ignore = "requires a real sing-box core via SING_BOX_BIN_1_14 and curl"]
fn no_proxy_cannot_turn_a_broken_node_into_a_successful_probe() {
    let root = tempfile::tempdir().unwrap();
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = origin.local_addr().unwrap().port();
    origin.set_nonblocking(true).unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let direct_requests = Arc::new(AtomicUsize::new(0));
    let thread = {
        let stopped = stopped.clone();
        let direct_requests = direct_requests.clone();
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = origin.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 4096];
                    if let Ok(n) = stream.read(&mut request) {
                        if request[..n].starts_with(b"GET ") {
                            direct_requests.fetch_add(1, Ordering::Relaxed);
                        }
                        let _ = stream
                            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        })
    };
    let mut config = DeploymentConfig::new(
        SubscriptionMode::IpFallback,
        "127.0.0.1".into(),
        None,
        Some(if port == 2080 { 2081 } else { 2080 }),
        "ens3".into(),
        vec![ManagedProtocol::VmessWebsocket],
        None,
    )
    .unwrap();
    // This endpoint speaks plain HTTP, so a VMess/TLS connection must fail.
    // A bypassed request would instead receive the origin's HTTP 204.
    config.vmess_websocket.as_mut().unwrap().listen_port = port;
    DeploymentStore::new(root.path())
        .initialize(&config)
        .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_sbctl"))
        .arg("--root")
        .arg(root.path())
        .args(["node", "probe", "sbctl-vmess-websocket", "--client-bin"])
        .arg(std::env::var_os("SING_BOX_BIN_1_14").expect("SING_BOX_BIN_1_14"))
        .args(["--url", &format!("http://127.0.0.1:{port}/")])
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .output()
        .unwrap();
    stopped.store(true, Ordering::Relaxed);
    thread.join().unwrap();
    assert!(
        !result.status.success(),
        "a broken node must not pass using direct HTTP"
    );
    assert_eq!(direct_requests.load(Ordering::Relaxed), 0);
    assert!(String::from_utf8_lossy(&result.stderr).contains("真实请求失败"));
}
