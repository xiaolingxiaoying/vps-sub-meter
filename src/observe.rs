//! Observation of the managed sing-box data plane: its kernel version, the
//! systemd unit's process state, and the journalctl invocation for logs.
//!
//! Everything here is read-only and degrades to `None`/advisory notes: a VPS
//! without systemd facts must not turn `sbctl sing-box status` into an error.
//! Fixture roots provide `usr/bin/systemctl` (or `systemctl.cmd` on Windows)
//! exactly like `lifecycle`'s systemctl helper, so the parsing logic is
//! unit-tested against captured output instead of a live unit manager.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The full version string the installed kernel reports, e.g. `1.14.1`.
///
/// Unlike `subscription::installed_kernel_version` (which reduces to the
/// `(major, minor)` the version registry keys on), this keeps the patch: the
/// observation verb exists to answer "what exactly is running", where a patch
/// difference is precisely the interesting part.
pub fn kernel_version_string(binary: &Path) -> Option<String> {
    let output = Command::new(binary).arg("version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    kernel_version_from_output(&stdout)
}

/// Extracts the full `X.Y.Z` from `sing-box version X.Y.Z …` output.
fn kernel_version_from_output(reported: &str) -> Option<String> {
    let line = reported
        .lines()
        .find(|line| line.trim().starts_with("sing-box version "))?;
    let version = line.trim().strip_prefix("sing-box version ")?.trim();
    // Reject anything that is not dotted digits: a kernel reporting something
    // exotic must surface as "unknown", not as a misleading version.
    let mut parts = version.split('.');
    let mut accepted = [None, None, None];
    for slot in &mut accepted {
        match parts.next() {
            Some(part) if !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) => {
                *slot = Some(part);
            }
            // A missing trailing part ("1.10") means zero; an empty or
            // non-numeric part means the string is not a version.
            None => {}
            Some(_) => return None,
        }
    }
    if parts.next().is_some() {
        return None;
    }
    let [major, minor, patch] = accepted;
    Some(format!("{}.{}.{}", major?, minor?, patch.unwrap_or("0")))
}

/// The systemd facts `sbctl sing-box status` reports for the data plane unit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SingBoxProcessObservation {
    pub active_state: Option<String>,
    pub sub_state: Option<String>,
    pub main_pid: Option<u32>,
    pub restart_count: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub cpu_time_nsecs: Option<u64>,
    pub task_count: Option<u64>,
    pub active_enter: Option<String>,
    pub uptime: Option<Duration>,
    pub error: Option<String>,
}

impl SingBoxProcessObservation {
    /// Human-readable memory, `None` when systemd did not report a value.
    pub fn memory_label(&self) -> Option<String> {
        self.memory_bytes.map(|bytes| {
            let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let mib = bytes as f64 / (1024.0 * 1024.0);
            if gib >= 1.0 {
                format!("{gib:.2} GiB ({bytes} B)")
            } else {
                format!("{mib:.1} MiB ({bytes} B)")
            }
        })
    }

    /// Human-readable CPU time from the nanosecond counter systemd reports.
    pub fn cpu_time_label(&self) -> Option<std::time::Duration> {
        self.cpu_time_nsecs
            .map(|nsecs| Duration::from_nanos(nsecs.min(u64::from(u32::MAX) * 1_000_000_000)))
    }
}

/// The sing-box systemd unit this tool installs and manages.
pub const SING_BOX_UNIT: &str = "sing-box.service";

/// Queries the managed sing-box unit's process facts through `systemctl show`.
///
/// Fixture roots (and the Windows acceptance fixtures) may provide
/// `usr/bin/systemctl` / `usr/bin/systemctl.cmd`; a live root falls back to the
/// `systemctl` on `PATH`. On a live root with no `MemoryCurrent` reported
/// (cgroup accounting disabled), `/proc/<pid>/status` VmRSS is consulted.
pub fn observe_sing_box_process(
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> SingBoxProcessObservation {
    let mut observation = match systemctl_show(root) {
        Ok(output) => parse_show_output(&output, now),
        Err(error) => SingBoxProcessObservation {
            error: Some(error),
            ..SingBoxProcessObservation::default()
        },
    };
    if observation.memory_bytes.is_none()
        && observation.main_pid.is_some_and(|pid| pid > 0)
        && root == Path::new("/")
    {
        observation.memory_bytes =
            read_proc_rss_kib(observation.main_pid.unwrap_or(0)).map(|kib| kib * 1024);
    }
    observation
}

fn parse_show_output(
    output: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> SingBoxProcessObservation {
    let mut observation = SingBoxProcessObservation::default();
    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "ActiveState" => observation.active_state = non_empty(value),
            "SubState" => observation.sub_state = non_empty(value),
            "MainPID" => observation.main_pid = value.parse().ok(),
            "NRestarts" => observation.restart_count = value.parse().ok(),
            "MemoryCurrent" if value != "[not set]" => {
                observation.memory_bytes = value.parse().ok();
            }
            "CPUUsageNSec" if value != "[not set]" => {
                observation.cpu_time_nsecs = value.parse().ok();
            }
            "TasksCurrent" if value != "[not set]" => {
                observation.task_count = value.parse().ok();
            }
            "ActiveEnterTimestamp" => observation.active_enter = non_empty(value),
            _ => {}
        }
    }
    if let Some(active_enter) = observation.active_enter.as_deref() {
        observation.uptime = uptime_since(active_enter, now);
    }
    observation
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

/// Runs journalctl for `sbctl logs`.
///
/// Like [`systemctl_show`], a fixture root may provide `usr/bin/journalctl`
/// (or `journalctl.cmd`) and then that copy is executed, which keeps the exact
/// argument surface assertable without a journald. Standard input/output are
/// inherited so `--follow` behaves as an interactive tail.
pub fn run_journalctl(root: &Path, args: &[String]) -> Result<std::process::ExitStatus, String> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let program = rooted_command(root, "journalctl")?;
    Command::new(program)
        .args(&arg_refs)
        .status()
        .map_err(|error| format!("{error}"))
}

/// Resolves a host command the way `lifecycle`'s systemctl helper does: the
/// fixture root's copy wins, otherwise the command comes from `PATH`.
fn rooted_command(root: &Path, program: &str) -> Result<PathBuf, String> {
    let rooted = root.join("usr/bin").join(program);
    #[cfg(windows)]
    let rooted = if rooted.is_file() {
        rooted
    } else {
        root.join("usr/bin").join(format!("{program}.cmd"))
    };
    if rooted.is_file() {
        return Ok(rooted);
    }
    if root != Path::new("/") {
        // A fixture root must provide its own host commands; falling back to
        // the live binary would let a test pass against the developer's host.
        return Err(format!("fixture command is missing: {program}"));
    }
    Ok(PathBuf::from(program))
}

fn systemctl_show(root: &Path) -> Result<String, String> {
    let program = rooted_command(root, "systemctl")?;
    let output = Command::new(program)
        .args([
            "show",
            "--no-pager",
            SING_BOX_UNIT,
            "-p",
            "ActiveState",
            "-p",
            "SubState",
            "-p",
            "MainPID",
            "-p",
            "NRestarts",
            "-p",
            "MemoryCurrent",
            "-p",
            "CPUUsageNSec",
            "-p",
            "TasksCurrent",
            "-p",
            "ActiveEnterTimestamp",
        ])
        .output()
        .map_err(|error| format!("systemd query failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "systemctl show {} exited with {}",
            SING_BOX_UNIT, output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `VmRSS` in KiB from `/proc/<pid>/status` — the fallback when cgroup memory
/// accounting is off on the host.
fn read_proc_rss_kib(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let value = line.split_whitespace().nth(1)?;
    value.parse().ok()
}

/// Parses systemd's `ActiveEnterTimestamp` ("Tue 2026-10-06 08:00:00 UTC" or a
/// `+08:00`-style offset suffix) into the age of the activation.
///
/// A timezone abbreviation systemd may print on non-UTC hosts (e.g. `CST`)
/// cannot be mapped to an offset without a timezone database, so the uptime is
/// simply omitted rather than computed wrongly.
fn uptime_since(timestamp: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    // "Tue 2026-10-06 08:00:00 UTC" -> keep the "2026-10-06 08:00:00 <zone>" tail.
    let rest = timestamp
        .split_once(' ')
        .map(|(_, rest)| rest)
        .unwrap_or(timestamp);
    let (datetime_text, zone) = rest.rsplit_once(' ')?;
    if zone == "UTC" {
        let naive =
            chrono::NaiveDateTime::parse_from_str(datetime_text, "%Y-%m-%d %H:%M:%S").ok()?;
        return duration_since(naive.and_utc(), now);
    }
    // Numeric offsets ("+08:00", "-0500") parse through %z directly.
    let parsed = chrono::DateTime::parse_from_str(
        &format!("{datetime_text} {zone}"),
        "%Y-%m-%d %H:%M:%S %z",
    )
    .ok()?;
    duration_since(parsed.with_timezone(&chrono::Utc), now)
}

/// The age of an activation instant, saturating at zero for clock skew.
fn duration_since(
    entered: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    Some(
        now.signed_duration_since(entered)
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
}

/// A read-only request against the data plane's loopback `clash_api`.
///
/// Hand-rolled over `TcpStream` rather than a client crate: the endpoint is
/// plain HTTP on the loopback interface, the CLI is synchronous, and the
/// alternative would add a dependency (and a TLS stack sbctl does not need here)
/// to read two JSON documents. The secret travels as a bearer token and is never
/// returned in an error, so a failure cannot paste it into the journal.
pub fn clash_api_request(
    api: &crate::config::ServerClashApi,
    path: &str,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let address: std::net::SocketAddr = format!("127.0.0.1:{}", api.port)
        .parse()
        .map_err(|error| format!("invalid observation address: {error}"))?;
    let stream = TcpStream::connect_timeout(&address, timeout).map_err(|_| {
        "sing-box 观测接口未响应（sbctl sing-box api enable 是否已启用？）".to_owned()
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        api.secret
    );
    let mut stream = stream;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "观测接口返回了不完整的响应".to_owned())?;
    if !head
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 "))
    {
        let status = head.lines().next().unwrap_or_default().to_owned();
        return Err(format!("观测接口返回 {status}"));
    }
    serde_json::from_str(body.trim()).map_err(|error| format!("观测接口 JSON 解析失败: {error}"))
}

/// Live memory of the data plane, in bytes, when the observation API is on.
pub fn clash_api_memory(api: &crate::config::ServerClashApi) -> Result<u64, String> {
    clash_api_request(api, "/memory", Duration::from_secs(3)).and_then(|value| {
        value
            .get("inuse")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "响应缺少 inuse 字段".to_owned())
    })
}

/// A short label for one live connection from `/connections`.
pub fn describe_connections(connections: &serde_json::Value) -> Vec<String> {
    connections
        .get("connections")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let metadata = &item["metadata"];
                    let destination = metadata["host"]
                        .as_str()
                        .or_else(|| metadata["destinationIP"].as_str())
                        .unwrap_or("?");
                    let chain = item["chains"]
                        .as_array()
                        .map(|chain| {
                            chain
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<Vec<_>>()
                                .join(" → ")
                        })
                        .unwrap_or_default();
                    let uploaded = item["upload"].as_u64().unwrap_or_default();
                    let downloaded = item["download"].as_u64().unwrap_or_default();
                    format!("{destination}  ↑{uploaded}B ↓{downloaded}B  [{chain}]")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The journalctl arguments for `sbctl logs`: the unit selection, the tail
/// length, and whether to follow. Split out so the exact command surface is
/// unit-testable without a journald.
pub fn journalctl_args(unit: &str, lines: u32, follow: bool) -> Vec<String> {
    let mut args = Vec::new();
    for selected in match unit {
        "sing-box" => vec!["sing-box.service"],
        "sbctl" => vec!["sbctl.service"],
        // "all" (and any other value the CLI validated away) matches the menu
        // behaviour: both units in one journalctl stream.
        _ => vec!["sbctl.service", "sing-box.service"],
    } {
        args.push("-u".to_owned());
        args.push(selected.to_owned());
    }
    args.push("-n".to_owned());
    args.push(lines.to_string());
    args.push("--no-pager".to_owned());
    if follow {
        args.push("-f".to_owned());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn the_full_kernel_version_keeps_the_patch_number() {
        assert_eq!(
            kernel_version_from_output("sing-box version 1.14.1\n"),
            Some("1.14.1".to_owned())
        );
        // Two-part versions are normalised to a zero patch, matching how the
        // registry spells its bands.
        assert_eq!(
            kernel_version_from_output("sing-box version 1.10\n"),
            Some("1.10.0".to_owned())
        );
    }

    #[test]
    fn a_kernel_that_reports_something_exotic_is_unknown() {
        assert_eq!(
            kernel_version_from_output("sing-box version 1.14.1-rc.2\n"),
            None
        );
        assert_eq!(kernel_version_from_output("unrelated output\n"), None);
        assert_eq!(
            kernel_version_from_output("sing-box version 1.14.1.9\n"),
            None
        );
    }

    #[test]
    fn systemd_show_output_is_parsed_into_process_facts() {
        let now = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
        let observation = parse_show_output(
            "ActiveState=active\n\
             SubState=running\n\
             MainPID=4242\n\
             NRestarts=2\n\
             MemoryCurrent=104857600\n\
             CPUUsageNSec=2500000000\n\
             TasksCurrent=18\n\
             ActiveEnterTimestamp=Tue 2026-10-06 08:00:00 UTC\n",
            now,
        );

        assert_eq!(observation.active_state.as_deref(), Some("active"));
        assert_eq!(observation.sub_state.as_deref(), Some("running"));
        assert_eq!(observation.main_pid, Some(4242));
        assert_eq!(observation.restart_count, Some(2));
        assert_eq!(observation.memory_bytes, Some(104857600));
        assert_eq!(observation.cpu_time_nsecs, Some(2_500_000_000));
        assert_eq!(observation.task_count, Some(18));
        assert_eq!(
            observation.uptime,
            Some(Duration::from_secs(4 * 3600)),
            "uptime = now - ActiveEnterTimestamp"
        );
        assert!(observation.error.is_none());
    }

    #[test]
    fn not_set_accounting_fields_stay_unknown_instead_of_zero() {
        let now = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
        let observation = parse_show_output(
            "ActiveState=active\nSubState=running\nMainPID=1\n\
             MemoryCurrent=[not set]\nCPUUsageNSec=[not set]\nTasksCurrent=[not set]\n",
            now,
        );

        assert_eq!(observation.memory_bytes, None);
        assert_eq!(observation.cpu_time_nsecs, None);
        assert_eq!(observation.task_count, None);
    }

    /// The full path (systemctl fixture -> parsing) is exercised on Unix where
    /// the fixture script is a real shell script; the parser itself is covered
    /// by `parse_show_output` on every platform.
    #[cfg(unix)]
    #[test]
    fn observe_sing_box_process_reads_a_rooted_systemctl_fixture() {
        use std::os::unix::fs::PermissionsExt;
        let now = Utc.with_ymd_and_hms(2026, 10, 6, 12, 0, 0).unwrap();
        let fixture = tempfile::TempDir::new().unwrap();
        let path = fixture.path().join("usr/bin/systemctl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "#!/bin/sh\ncat <<'EOF'\nActiveState=active\nSubState=running\nMainPID=7\nNRestarts=1\nMemoryCurrent=2097152\nEOF\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();

        let observation = observe_sing_box_process(fixture.path(), now);

        assert_eq!(observation.main_pid, Some(7));
        assert_eq!(observation.memory_bytes, Some(2_097_152));
        assert!(observation.error.is_none());
    }

    #[test]
    fn a_non_utc_offset_timestamp_still_yields_uptime() {
        // 20:00 at +08:00 is 12:00 UTC, which is exactly `now`: a saturating
        // zero. Shift by one more hour to also exercise the subtraction.
        let now = Utc.with_ymd_and_hms(2026, 10, 6, 13, 0, 0).unwrap();
        assert_eq!(
            uptime_since("Tue 2026-10-06 20:00:00 +08:00", now),
            Some(Duration::from_secs(3600))
        );
    }

    #[test]
    fn an_unmappable_zone_abbreviation_omits_uptime_rather_than_guessing() {
        assert_eq!(
            uptime_since("Tue 2026-10-06 08:00:00 CST", Utc::now()),
            None
        );
    }

    #[test]
    fn journalctl_arguments_select_units_tail_and_follow() {
        assert_eq!(
            journalctl_args("all", 50, false),
            [
                "-u",
                "sbctl.service",
                "-u",
                "sing-box.service",
                "-n",
                "50",
                "--no-pager"
            ]
        );
        assert_eq!(
            journalctl_args("sing-box", 100, true),
            ["-u", "sing-box.service", "-n", "100", "--no-pager", "-f"]
        );
    }
}

#[cfg(test)]
mod api_tests {
    use super::{clash_api_memory, clash_api_request, describe_connections};
    use crate::config::ServerClashApi;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    /// A one-shot stand-in for sing-box's clash API: it records the request head
    /// and answers with a fixed status line and body.
    fn mock_api(
        status: &'static str,
        response_body: &'static str,
        seen: std::sync::mpsc::Sender<String>,
    ) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock listener binds");
        let port = listener.local_addr().expect("listener address").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buffer = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let _ = seen.send(String::from_utf8_lossy(&buffer).into_owned());
                let body = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{response_body}"
                );
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        });
        port
    }

    fn api(port: u16) -> ServerClashApi {
        ServerClashApi {
            port,
            secret: "supersecretvalue".to_owned(),
        }
    }

    #[test]
    fn the_request_carries_the_bearer_token_and_parses_the_answer() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let port = mock_api("200 OK", r#"{"inuse":1048576,"inuse_one":0}"#, sender);

        assert_eq!(
            clash_api_memory(&api(port)).expect("memory reads"),
            1_048_576
        );
        let request = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the mock saw a request");
        assert!(
            request.starts_with("GET /memory HTTP/1.1")
                && request.contains("Authorization: Bearer supersecretvalue"),
            "the API is authenticated: {request}"
        );
    }

    #[test]
    fn a_non_success_answer_is_reported_without_repeating_the_secret() {
        let (sender, _receiver) = std::sync::mpsc::channel();
        let port = mock_api("401 Unauthorized", r#"{"message":"unauthorized"}"#, sender);

        let error = clash_api_request(&api(port), "/connections", Duration::from_secs(5))
            .expect_err("a non-200 answer is a failure, not a payload");
        assert!(
            error.contains("401"),
            "the status must be surfaced: {error}"
        );
        assert!(
            !error.contains("supersecretvalue"),
            "an error string must not carry the secret into the journal: {error}"
        );
    }

    #[test]
    fn a_closed_port_is_an_advisory_pointing_at_the_verb_that_fixes_it() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("address").port();
        drop(listener);

        let error = clash_api_request(&api(port), "/memory", Duration::from_millis(200))
            .expect_err("nothing is listening");
        assert!(
            error.contains("sbctl sing-box api enable"),
            "the failure must say how to fix it: {error}"
        );
    }

    #[test]
    fn connections_are_described_with_chain_and_tally() {
        let value = serde_json::json!({
            "connections": [{
                "metadata": {"host": "example.com", "destinationIP": "", "sni": "example.com"},
                "upload": 120, "download": 4800,
                "chains": ["节点选择", "sbctl-vless-reality"]
            }]
        });
        let lines = describe_connections(&value);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("example.com")
                && lines[0].contains("↑120B")
                && lines[0].contains("↓4800B")
                && lines[0].contains("节点选择 → sbctl-vless-reality"),
            "{}",
            lines[0]
        );
    }
}
