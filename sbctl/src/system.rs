//! Host facts and kernel tuning helpers for the `system` subcommands.
//!
//! BBR (Bottleneck Bandwidth and Round-trip propagation time) is Google's TCP
//! congestion control algorithm. It improves throughput and latency on lossy or
//! high-latency paths compared to loss-based algorithms such as CUBIC. sing-box-yg
//! enables it with the `fq` queueing discipline at the kernel level (see
//! `docs/sing-box-yg-port-plan.md`), which also accelerates the TCP-based Managed
//! protocols (VLESS Reality, VMess WebSocket, AnyTLS).
//!
//! The BBR helpers below apply idempotent kernel sysctl settings; subscription
//! diagnosis is read-only and never changes the sing-box configuration.

use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::runtime::Runtime;

/// Host-relative path of the `sysctl.d` drop-in that survives reboots.
const BBR_CONF_RELATIVE: &str = "etc/sysctl.d/99-sbctl-bbr.conf";

#[derive(Debug, Error)]
pub enum SystemError {
    #[error("could not read kernel setting {0}: {1}")]
    Read(&'static str, String),
    #[error("could not apply sysctl {0}: {1}")]
    Apply(String, String),
    #[error("could not persist BBR settings: {0}")]
    Persist(String),
}

/// The current kernel congestion control and default queueing discipline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BbrStatus {
    pub congestion_control: String,
    pub qdisc: String,
}

/// Reads the current TCP congestion control and default queueing discipline.
/// The congestion control is essential and must be readable; the queueing
/// discipline is best-effort (some minimal kernels omit it) and falls back to
/// `unknown`.
pub fn read_current<C: crate::runtime::Clock>(
    runtime: &Runtime<C>,
) -> Result<BbrStatus, SystemError> {
    let congestion_control = runtime
        .read_to_string("proc/sys/net/ipv4/tcp_congestion_control")
        .map_err(|error| SystemError::Read("tcp_congestion_control", error.to_string()))?
        .trim()
        .to_owned();
    let qdisc = runtime
        .read_to_string("proc/sys/net/core/default_qdisc")
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned());
    Ok(BbrStatus {
        congestion_control,
        qdisc,
    })
}

/// Enables BBR + FQ, applying only the setting that is not already correct, then
/// persists both via a `sysctl.d` drop-in so they survive a reboot. Returns the
/// resulting kernel settings.
pub fn enable_bbr<C: crate::runtime::Clock>(
    runtime: &Runtime<C>,
) -> Result<BbrStatus, SystemError> {
    let before = read_current(runtime)?;
    let mut congestion_control = before.congestion_control.clone();
    let mut qdisc = before.qdisc.clone();
    if congestion_control != "bbr" {
        apply_sysctl(runtime, "net.ipv4.tcp_congestion_control=bbr")?;
        congestion_control = "bbr".to_owned();
    }
    if qdisc != "fq" {
        apply_sysctl(runtime, "net.core.default_qdisc=fq")?;
        qdisc = "fq".to_owned();
    }
    persist(runtime)?;
    Ok(BbrStatus {
        congestion_control,
        qdisc,
    })
}

fn apply_sysctl<C: crate::runtime::Clock>(
    runtime: &Runtime<C>,
    setting: &str,
) -> Result<(), SystemError> {
    let (status, output) = runtime
        .run_command_output("sysctl", &["-w", setting])
        .map_err(|error| SystemError::Apply(setting.to_owned(), error.to_string()))?;
    if !status.success() {
        return Err(SystemError::Apply(setting.to_owned(), output));
    }
    Ok(())
}

fn persist<C: crate::runtime::Clock>(runtime: &Runtime<C>) -> Result<(), SystemError> {
    let path = runtime.root().join(Path::new(BBR_CONF_RELATIVE));
    let parent = path
        .parent()
        .expect("the sysctl drop-in has a parent directory");
    fs::create_dir_all(parent).map_err(|error| SystemError::Persist(error.to_string()))?;
    let contents = "net.ipv4.tcp_congestion_control=bbr\nnet.core.default_qdisc=fq\n";
    fs::write(&path, contents).map_err(|error| SystemError::Persist(error.to_string()))?;
    Ok(())
}

/// UDP `connect` performs a route lookup without sending a packet, which makes
/// it a cheap probe for an IPv6 default route.
///
/// It lives here rather than in the subscription renderers because it is a
/// fact about the host, not a choice about the configuration.
pub fn host_has_ipv6_route() -> bool {
    let Ok(socket) = std::net::UdpSocket::bind("[::]:0") else {
        return false;
    };
    socket.connect("[2001:4860:4860::8888]:443").is_ok()
}

/// Read-only service, DNS, certificate and local endpoint diagnostics. Output
/// intentionally excludes the generated URL and subscription credential.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubscriptionDiagnosis {
    pub configuration_loaded: bool,
    pub configuration_error: Option<&'static str>,
    pub mode: Option<String>,
    pub host: Option<String>,
    pub public_scheme: Option<&'static str>,
    pub public_port: Option<u16>,
    pub certificate: Option<crate::certificate::CertificateStatus>,
    pub service: UnitDiagnosis,
    pub socket: Option<UnitDiagnosis>,
    pub dns: Option<DnsDiagnosis>,
    pub listener: ProbeDiagnosis,
    pub local_tls: ProbeDiagnosis,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnitDiagnosis {
    pub unit: String,
    pub active_state: Option<String>,
    pub sub_state: Option<String>,
    pub restart_count: Option<u64>,
    pub error: Option<&'static str>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DnsDiagnosis {
    pub host: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub error: Option<&'static str>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeDiagnosis {
    pub status: &'static str,
    pub target: Option<String>,
    pub tcp_connected: bool,
    pub tls_handshake: Option<bool>,
    /// The local handshake skips public CA verification. Certificate validity
    /// and host coverage are checked and reported separately.
    pub trust_validation: Option<bool>,
    pub http_status: Option<u16>,
    pub elapsed_ms: u128,
}

/// Collects current unit state and makes bounded probes against only the local
/// service endpoint. Direct mode sends SNI for the configured host and an
/// unauthenticated `GET /`; it never contacts a credential-bearing URL.
pub fn diagnose_subscription(root: &Path) -> SubscriptionDiagnosis {
    use crate::config::SubscriptionMode;

    let store = crate::config::DeploymentStore::new(root);
    let is_live_root = root == Path::new("/");
    let service = diagnose_unit("sbctl.service", is_live_root, true);
    let Ok(config) = store.load() else {
        return SubscriptionDiagnosis {
            configuration_loaded: false,
            configuration_error: Some("deployment_configuration_unavailable"),
            mode: None,
            host: None,
            public_scheme: None,
            public_port: None,
            certificate: None,
            service,
            socket: None,
            dns: None,
            listener: ProbeDiagnosis::not_run(),
            local_tls: ProbeDiagnosis::not_run(),
        };
    };

    let (scheme, port, target, socket_unit) = match &config.subscription_mode {
        SubscriptionMode::Direct => (
            "https",
            Some(443),
            Some("127.0.0.1:443".to_owned()),
            Some("sbctl-http.socket"),
        ),
        SubscriptionMode::ExternalProxy => (
            "https",
            Some(443),
            config
                .subscription_listen_port
                .map(|port| format!("127.0.0.1:{port}")),
            None,
        ),
        SubscriptionMode::IpFallback => (
            "http",
            config.http_port,
            config.http_port.map(|port| {
                format!(
                    "{}:{port}",
                    crate::canonical::uri_host(&config.subscription_host)
                )
            }),
            None,
        ),
    };

    let socket = socket_unit.map(|unit| diagnose_unit(unit, is_live_root, false));
    let is_direct = matches!(&config.subscription_mode, SubscriptionMode::Direct);
    let listener = if is_live_root {
        target
            .as_deref()
            .map(|target| {
                if is_direct {
                    probe_tcp(target)
                } else {
                    probe_tcp_http(target)
                }
            })
            .unwrap_or_else(ProbeDiagnosis::not_run)
    } else {
        ProbeDiagnosis::not_run()
    };
    let local_tls = match &config.subscription_mode {
        SubscriptionMode::Direct if is_live_root => probe_direct_tls(&config.subscription_host),
        SubscriptionMode::Direct => ProbeDiagnosis::not_run(),
        _ => ProbeDiagnosis::not_applicable(),
    };
    let dns = is_live_root.then(|| resolve_host(&config.subscription_host));
    let certificate = matches!(&config.subscription_mode, SubscriptionMode::Direct)
        .then(|| crate::certificate::status(&store, &config));

    SubscriptionDiagnosis {
        configuration_loaded: true,
        configuration_error: None,
        mode: Some(config.subscription_mode.to_string()),
        host: Some(config.subscription_host),
        public_scheme: Some(scheme),
        public_port: port,
        certificate,
        service,
        socket,
        dns,
        listener,
        local_tls,
    }
}

impl ProbeDiagnosis {
    fn not_run() -> Self {
        Self {
            status: "not_run",
            target: None,
            tcp_connected: false,
            tls_handshake: None,
            trust_validation: None,
            http_status: None,
            elapsed_ms: 0,
        }
    }

    fn not_applicable() -> Self {
        Self {
            status: "not_applicable",
            ..Self::not_run()
        }
    }
}

fn diagnose_unit(unit: &str, is_live_root: bool, with_restarts: bool) -> UnitDiagnosis {
    if !is_live_root {
        return UnitDiagnosis {
            unit: unit.to_owned(),
            active_state: None,
            sub_state: None,
            restart_count: None,
            error: Some("host_probe_skipped_for_non_live_root"),
        };
    }
    let mut command = Command::new("systemctl");
    command.args([
        "show",
        "--no-pager",
        unit,
        "-p",
        "ActiveState",
        "-p",
        "SubState",
    ]);
    if with_restarts {
        command.args(["-p", "NRestarts"]);
    }
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        _ => {
            return UnitDiagnosis {
                unit: unit.to_owned(),
                active_state: None,
                sub_state: None,
                restart_count: None,
                error: Some("systemd_query_failed"),
            };
        }
    };
    let mut result = UnitDiagnosis {
        unit: unit.to_owned(),
        active_state: None,
        sub_state: None,
        restart_count: None,
        error: None,
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "ActiveState" => result.active_state = Some(value.to_owned()),
            "SubState" => result.sub_state = Some(value.to_owned()),
            "NRestarts" => result.restart_count = value.parse().ok(),
            _ => {}
        }
    }
    result
}

fn resolve_host(host: &str) -> DnsDiagnosis {
    if let Ok(address) = host.parse::<IpAddr>() {
        return dns_result(host, vec![address], None);
    }
    let query_host = host.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (query_host.as_str(), 443)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>());
        let _ = sender.send(result);
    });
    match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(addresses)) => dns_result(host, addresses, None),
        Ok(Err(_)) => dns_result(host, Vec::new(), Some("name_resolution_failed")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            dns_result(host, Vec::new(), Some("name_resolution_timed_out"))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            dns_result(host, Vec::new(), Some("name_resolution_failed"))
        }
    }
}

fn dns_result(host: &str, mut addresses: Vec<IpAddr>, error: Option<&'static str>) -> DnsDiagnosis {
    addresses.sort();
    addresses.dedup();
    DnsDiagnosis {
        host: host.to_owned(),
        ipv4: addresses
            .iter()
            .filter(|ip| ip.is_ipv4())
            .map(ToString::to_string)
            .collect(),
        ipv6: addresses
            .iter()
            .filter(|ip| ip.is_ipv6())
            .map(ToString::to_string)
            .collect(),
        error,
    }
}

fn probe_tcp_http(target: &str) -> ProbeDiagnosis {
    let started = Instant::now();
    let addresses = match target.to_socket_addrs() {
        Ok(addresses) => addresses.collect::<Vec<_>>(),
        Err(_) => {
            return ProbeDiagnosis {
                status: "address_invalid",
                target: Some(target.to_owned()),
                tcp_connected: false,
                tls_handshake: None,
                trust_validation: None,
                http_status: None,
                elapsed_ms: started.elapsed().as_millis(),
            };
        }
    };
    for address in addresses {
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let _ =
                stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
            let mut response = [0; 256];
            let status = stream
                .read(&mut response)
                .ok()
                .and_then(|count| parse_http_status(&response[..count]));
            return ProbeDiagnosis {
                status: if status.is_some() {
                    "http_response"
                } else {
                    "tcp_connected_no_http_response"
                },
                target: Some(target.to_owned()),
                tcp_connected: true,
                tls_handshake: None,
                trust_validation: None,
                http_status: status,
                elapsed_ms: started.elapsed().as_millis(),
            };
        }
    }
    ProbeDiagnosis {
        status: "tcp_connect_failed",
        target: Some(target.to_owned()),
        tcp_connected: false,
        tls_handshake: None,
        trust_validation: None,
        http_status: None,
        elapsed_ms: started.elapsed().as_millis(),
    }
}

fn parse_http_status(response: &[u8]) -> Option<u16> {
    let line = response.split(|byte| *byte == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let mut parts = line.split_ascii_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

fn probe_tcp(target: &str) -> ProbeDiagnosis {
    let started = Instant::now();
    let addresses = match target.to_socket_addrs() {
        Ok(addresses) => addresses.collect::<Vec<_>>(),
        Err(_) => {
            return ProbeDiagnosis {
                status: "address_invalid",
                target: Some(target.to_owned()),
                tcp_connected: false,
                tls_handshake: None,
                trust_validation: None,
                http_status: None,
                elapsed_ms: started.elapsed().as_millis(),
            };
        }
    };
    for address in addresses {
        if TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok() {
            return ProbeDiagnosis {
                status: "tcp_connected",
                target: Some(target.to_owned()),
                tcp_connected: true,
                tls_handshake: None,
                trust_validation: None,
                http_status: None,
                elapsed_ms: started.elapsed().as_millis(),
            };
        }
    }
    ProbeDiagnosis {
        status: "tcp_connect_failed",
        target: Some(target.to_owned()),
        tcp_connected: false,
        tls_handshake: None,
        trust_validation: None,
        http_status: None,
        elapsed_ms: started.elapsed().as_millis(),
    }
}

fn probe_direct_tls(host: &str) -> ProbeDiagnosis {
    let started = Instant::now();
    let target = format!("127.0.0.1:443 (SNI {host})");
    let url = format!("https://{host}/");
    let resolve = format!("{host}:443:127.0.0.1");
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--noproxy",
            "*",
            "--connect-timeout",
            "2",
            "--max-time",
            "5",
            "--output",
            "/dev/null",
            "--write-out",
            "%{http_code}|%{time_connect}|%{time_appconnect}",
        ])
        .arg("--resolve")
        .arg(resolve)
        .arg(url)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(_) => {
            return ProbeDiagnosis {
                status: "curl_unavailable",
                target: Some(target),
                tcp_connected: false,
                tls_handshake: None,
                trust_validation: None,
                http_status: None,
                elapsed_ms: started.elapsed().as_millis(),
            };
        }
    };
    let fields = String::from_utf8_lossy(&output.stdout);
    let mut fields = fields.trim().split('|');
    let http_status = fields.next().and_then(|field| field.parse::<u16>().ok());
    let time_connect = fields.next().and_then(|field| field.parse::<f64>().ok());
    let time_appconnect = fields.next().and_then(|field| field.parse::<f64>().ok());
    let tcp_connected = time_connect.is_some_and(|seconds| seconds > 0.0);
    let app_connected = time_appconnect.is_some_and(|seconds| seconds > 0.0);
    let curl_code = output.status.code();
    let (status, tls_handshake, trust_validation) = match curl_code {
        Some(0) if app_connected => ("tls_verified_http_response", Some(true), Some(true)),
        Some(60) => ("certificate_validation_failed", Some(false), Some(false)),
        Some(28) if !tcp_connected => ("tcp_connect_timeout", Some(false), None),
        Some(28) if !app_connected => ("tls_handshake_timeout", Some(false), None),
        Some(28) => ("http_response_timeout", Some(true), Some(true)),
        Some(35) => ("tls_handshake_failed", Some(false), None),
        Some(7) => ("tcp_connect_failed", Some(false), None),
        Some(_) if app_connected => ("tls_verified_http_error", Some(true), Some(true)),
        Some(_) if tcp_connected => ("tls_handshake_failed", Some(false), None),
        Some(_) => ("probe_failed", Some(false), None),
        None => ("probe_failed", Some(false), None),
    };
    ProbeDiagnosis {
        status,
        target: Some(target),
        tcp_connected,
        tls_handshake,
        trust_validation,
        http_status,
        elapsed_ms: started.elapsed().as_millis(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;
    use tempfile::TempDir;

    use super::{diagnose_subscription, enable_bbr, read_current};
    use crate::config::{DeploymentConfig, DeploymentStore, ManagedProtocol, SubscriptionMode};
    use crate::runtime::Runtime;

    fn write_proc(root: &PathFixture, path: &str, contents: &str) {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().expect("proc path has a parent")).unwrap();
        fs::write(path, contents).unwrap();
    }

    struct PathFixture(TempDir);

    impl PathFixture {
        fn new() -> Self {
            Self(TempDir::new().expect("temporary root is created"))
        }
        fn path(&self) -> &std::path::Path {
            self.0.path()
        }
    }

    fn write_sysctl_fixture(root: &std::path::Path) {
        let command = root.join("usr/bin/sysctl");
        fs::create_dir_all(command.parent().expect("sysctl has a parent")).unwrap();
        fs::write(&command, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    #[test]
    fn read_current_reports_the_host_kernel_settings() {
        let fixture = PathFixture::new();
        write_proc(
            &fixture,
            "proc/sys/net/ipv4/tcp_congestion_control",
            "cubic\n",
        );
        write_proc(&fixture, "proc/sys/net/core/default_qdisc", "fq_codel\n");
        let runtime = Runtime::fixture(fixture.path(), Utc::now());

        let status = read_current(&runtime).expect("kernel settings are readable");
        assert_eq!(status.congestion_control, "cubic");
        assert_eq!(status.qdisc, "fq_codel");
    }

    #[test]
    fn read_current_treats_a_missing_qdisc_as_unknown() {
        let fixture = PathFixture::new();
        write_proc(
            &fixture,
            "proc/sys/net/ipv4/tcp_congestion_control",
            "bbr\n",
        );
        let runtime = Runtime::fixture(fixture.path(), Utc::now());

        let status = read_current(&runtime).expect("kernel settings are readable");
        assert_eq!(status.congestion_control, "bbr");
        assert_eq!(status.qdisc, "unknown");
    }

    #[test]
    #[cfg(unix)]
    fn enable_bbr_applies_missing_settings_and_persists_a_drop_in() {
        let fixture = PathFixture::new();
        write_proc(
            &fixture,
            "proc/sys/net/ipv4/tcp_congestion_control",
            "cubic\n",
        );
        write_proc(&fixture, "proc/sys/net/core/default_qdisc", "fq_codel\n");
        write_sysctl_fixture(fixture.path());
        let runtime = Runtime::fixture(fixture.path(), Utc::now());

        let status = enable_bbr(&runtime).expect("BBR is enabled");
        assert_eq!(status.congestion_control, "bbr");
        assert_eq!(status.qdisc, "fq");

        let drop_in = fs::read_to_string(fixture.path().join("etc/sysctl.d/99-sbctl-bbr.conf"))
            .expect("the sysctl drop-in is persisted");
        assert!(drop_in.contains("net.ipv4.tcp_congestion_control=bbr"));
        assert!(drop_in.contains("net.core.default_qdisc=fq"));
    }

    #[test]
    fn enable_bbr_is_idempotent_when_settings_are_already_correct() {
        let fixture = PathFixture::new();
        write_proc(
            &fixture,
            "proc/sys/net/ipv4/tcp_congestion_control",
            "bbr\n",
        );
        write_proc(&fixture, "proc/sys/net/core/default_qdisc", "fq\n");
        write_sysctl_fixture(fixture.path());
        let runtime = Runtime::fixture(fixture.path(), Utc::now());

        let status = enable_bbr(&runtime).expect("BBR is already enabled");
        assert_eq!(status.congestion_control, "bbr");
        assert_eq!(status.qdisc, "fq");

        let drop_in = fs::read_to_string(fixture.path().join("etc/sysctl.d/99-sbctl-bbr.conf"))
            .expect("the sysctl drop-in is persisted");
        assert!(drop_in.contains("net.ipv4.tcp_congestion_control=bbr"));
    }

    #[test]
    fn subscription_diagnosis_reports_mode_without_serializing_credentials() {
        let fixture = PathFixture::new();
        let store = DeploymentStore::new(fixture.path());
        let config = DeploymentConfig::new(
            SubscriptionMode::Direct,
            "sub.example.test".into(),
            None,
            None,
            "ens3".into(),
            vec![ManagedProtocol::VlessReality],
            Some("www.cloudflare.com".into()),
        )
        .expect("a Direct deployment is valid");
        let secret = config.subscription_credential.clone();
        store
            .initialize(&config)
            .expect("the fixture config is initialized");

        let diagnosis = diagnose_subscription(fixture.path());
        let json = serde_json::to_string(&diagnosis).expect("diagnosis serializes as JSON");
        assert!(diagnosis.configuration_loaded);
        assert_eq!(diagnosis.mode.as_deref(), Some("direct"));
        assert_eq!(diagnosis.public_scheme, Some("https"));
        assert_eq!(diagnosis.local_tls.status, "not_run");
        assert!(
            !json.contains(&secret),
            "the credential is never serialized"
        );
        assert!(
            !json.contains("/sub/"),
            "the path-bearing subscription URL is omitted"
        );
    }
}
