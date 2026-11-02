//! Reporting and accounting: `sbctl status` (human and JSON), `sbctl node`,
//! `sbctl traffic`, and the periodic accounting reset the timer runs.

use std::path::Path;
use std::process::ExitCode;

pub(crate) fn format_local_time(instant: chrono::DateTime<chrono::Utc>, timezone: &str) -> String {
    timezone
        .parse::<chrono_tz::Tz>()
        .map(|timezone| {
            instant
                .with_timezone(&timezone)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| instant.to_rfc3339())
}

fn format_local_time_rfc3339(instant: chrono::DateTime<chrono::Utc>, timezone: &str) -> String {
    timezone
        .parse::<chrono_tz::Tz>()
        .map(|timezone| instant.with_timezone(&timezone).to_rfc3339())
        .unwrap_or_else(|_| instant.to_rfc3339())
}

struct TrafficAllowance {
    remaining_bytes: Option<u64>,
    over_limit_bytes: u64,
    usage_percent: Option<f64>,
}

impl TrafficAllowance {
    fn new(total: u64, limit: u64) -> Self {
        if limit == 0 {
            return Self {
                remaining_bytes: None,
                over_limit_bytes: 0,
                usage_percent: None,
            };
        }
        Self {
            remaining_bytes: Some(limit.saturating_sub(total)),
            over_limit_bytes: total.saturating_sub(limit),
            usage_percent: Some(total as f64 / limit as f64 * 100.0),
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut amount = bytes as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"] {
        amount /= 1024.0;
        unit = next;
        if amount < 1024.0 {
            break;
        }
    }
    format!("{amount:.2} {unit}")
}

fn traffic_allowance_summary(report: &sbctl::traffic::TrafficReport) -> String {
    let total = report.total();
    let allowance = TrafficAllowance::new(total, report.monthly_traffic_limit);
    let Some(percent) = allowance.usage_percent else {
        return format!("本周期已用: {}；额度: 不限额", format_bytes(total));
    };
    let balance = if allowance.over_limit_bytes > 0 {
        format!(
            "超额: {}（仅统计，不自动断开连接）",
            format_bytes(allowance.over_limit_bytes)
        )
    } else {
        format!(
            "剩余: {}",
            format_bytes(allowance.remaining_bytes.unwrap_or(0))
        )
    };
    format!(
        "本周期已用: {} / {}（{percent:.1}%）；{balance}",
        format_bytes(total),
        format_bytes(report.monthly_traffic_limit)
    )
}

fn print_traffic_report(
    report: &sbctl::traffic::TrafficReport,
    config: &sbctl::config::DeploymentConfig,
) {
    println!(
        "{}\n{}\n下一次刷新（VPS: {}）: {}\n下一次刷新（客户端: {}）: {}",
        report.summary(),
        traffic_allowance_summary(report),
        config.accounting_timezone,
        format_local_time_rfc3339(report.next_reset, &config.accounting_timezone),
        config.client_display_timezone,
        format_local_time_rfc3339(report.next_reset, &config.client_display_timezone)
    );
}

fn format_uptime(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let days = seconds / 86_400;
    let hours = seconds / 3_600 % 24;
    let minutes = seconds / 60 % 60;
    let seconds = seconds % 60;
    if days > 0 {
        format!("{days} 天 {hours} 小时 {minutes} 分 {seconds} 秒")
    } else if hours > 0 {
        format!("{hours} 小时 {minutes} 分 {seconds} 秒")
    } else if minutes > 0 {
        format!("{minutes} 分 {seconds} 秒")
    } else {
        format!("{seconds} 秒")
    }
}

pub(crate) fn print_nodes(
    root: &Path,
    links: bool,
    protocol: Option<sbctl::config::ManagedProtocol>,
    qr: bool,
) -> ExitCode {
    match sbctl::config::DeploymentStore::new(root).load() {
        Ok(config) => {
            let protocol_name = protocol.as_ref().map(ToString::to_string);
            let nodes = sbctl::canonical::nodes(&config)
                .into_iter()
                .filter(|node| {
                    protocol
                        .as_ref()
                        .is_none_or(|selected| &node.protocol() == selected)
                })
                .collect::<Vec<_>>();
            if let Some(protocol) = &protocol_name
                && nodes.is_empty()
            {
                eprintln!("未启用协议：{protocol}");
                return ExitCode::from(2);
            }
            let summary = sbctl::lifecycle::enabled_nodes_for_protocol(&config, protocol.as_ref());
            println!("{summary}");
            if links && !nodes.is_empty() {
                if qr {
                    for node in &nodes {
                        let link = sbctl::subscription::node_share_link(&config, node);
                        println!("\n{}", node.protocol().label_zh());
                        println!("{}", link.trim_end());
                        match sbctl::qr::render_ansi(link.trim()) {
                            Ok(rendered) => print!("{rendered}"),
                            Err(error) => {
                                eprintln!("节点二维码生成失败：{error}");
                                return ExitCode::from(2);
                            }
                        }
                    }
                } else {
                    println!("\n原生分享链接（含节点凭据，仅输出到本终端）:");
                    for node in &nodes {
                        println!("{}:", node.protocol().label_zh());
                        print!("{}", sbctl::subscription::node_share_link(&config, node));
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("节点命令失败：{error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn days_until(timestamp: i64) -> Option<i64> {
    let now = chrono::Utc::now().timestamp();
    days_until_at(timestamp, now)
}

fn days_until_at(timestamp: i64, now: i64) -> Option<i64> {
    timestamp
        .checked_sub(now)
        .map(|seconds| seconds.div_euclid(86_400))
}

pub(crate) fn print_status(root: &Path) -> ExitCode {
    match sbctl::config::DeploymentStore::new(root).load() {
        Ok(config) => {
            println!("{}", config.summary());
            println!("\n{}", sbctl::lifecycle::service_status(root));
            if config.subscription_mode == sbctl::config::SubscriptionMode::Direct {
                let status =
                    sbctl::certificate::status(&sbctl::config::DeploymentStore::new(root), &config);
                if status.state == "ok"
                    && let Some(not_after) = status.not_after
                    && let Some(days) = days_until(not_after)
                {
                    if days < 0 {
                        println!("\n证书已过期，请运行 sbctl certificate renew");
                    } else if days == 0 {
                        println!("\n证书剩余有效期: 不足 1 天（即将到期，请检查 certbot.timer）");
                    } else {
                        let hint = if days < 14 {
                            "（即将到期，请检查 certbot.timer）"
                        } else {
                            ""
                        };
                        println!("\n证书剩余有效期: {days} 天{hint}");
                    }
                } else if let Some(error) = &status.error {
                    println!("\n证书状态: 异常（{error}）；运行 sbctl certificate status 查看详情");
                }
            }
            match sbctl::traffic::report(&sbctl::config::DeploymentStore::new(root), &config) {
                Ok(report) => {
                    println!();
                    print_traffic_report(&report, &config);
                }
                Err(error) => println!("\nVPS traffic: unavailable ({error})"),
            }
            // The data plane's process facts, one line: state, PID, memory,
            // uptime. Every unknown fact prints as 未知 rather than failing the
            // whole status command.
            {
                let observation =
                    sbctl::observe::observe_sing_box_process(root, chrono::Utc::now());
                if observation.error.is_none() {
                    let uptime = match observation.active_state.as_deref() {
                        Some("active") => format!(
                            "已运行 {}",
                            observation
                                .uptime
                                .map(format_uptime)
                                .unwrap_or_else(|| "未知".into())
                        ),
                        Some("inactive" | "failed") => "当前未运行".into(),
                        _ => "运行时长未知".into(),
                    };
                    println!(
                        "\nsing-box: {} / {}  PID {}  内存 {}  {}",
                        observation.active_state.as_deref().unwrap_or("未知"),
                        observation.sub_state.as_deref().unwrap_or("未知"),
                        observation
                            .main_pid
                            .map(|pid| pid.to_string())
                            .as_deref()
                            .unwrap_or("未知"),
                        observation.memory_label().unwrap_or_else(|| "未知".into()),
                        uptime,
                    );
                } else if let Some(error) = &observation.error {
                    println!(
                        "\nsing-box 进程状态: 无法查询（{error}）；运行 sbctl sing-box status 查看详情"
                    );
                }
            }
            // Advisory only: an installed kernel newer than the version table is
            // a gap in this tool's registry, not a fault in the deployment, so
            // it is printed here and never changes the exit status.
            if let Some(warning) = sbctl::subscription::kernel_band_warning(
                super::config::resolve_sing_box_bin(root, None).as_deref(),
            ) {
                println!("\n{warning}");
            }
            ExitCode::SUCCESS
        }
        Err(sbctl::config::ConfigError::Missing) => {
            println!("sbctl status: unmanaged (not installed)");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("status failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn print_status_json(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    match store.load() {
        Ok(config) => {
            let traffic = sbctl::traffic::report(&store, &config)
                .map(|report| {
                    let allowance =
                        TrafficAllowance::new(report.total(), report.monthly_traffic_limit);
                    serde_json::json!({
                        "interface": report.interface,
                        "received": report.received,
                        "transmitted": report.transmitted,
                        "total_adjustment": report.total_adjustment,
                        "total": report.total(),
                        "monthly_traffic_limit": report.monthly_traffic_limit,
                        "remaining_bytes": allowance.remaining_bytes,
                        "over_limit_bytes": allowance.over_limit_bytes,
                        "usage_percent": allowance.usage_percent,
                        "accounting_period": report.accounting_period,
                        "next_reset": report.next_reset.to_rfc3339(),
                        "next_reset_vps_refresh": format_local_time(
                            report.next_reset,
                            &config.accounting_timezone
                        ),
                        "next_reset_client_display": format_local_time(
                            report.next_reset,
                            &config.client_display_timezone
                        ),
                        "next_reset_vps_refresh_rfc3339": format_local_time_rfc3339(
                            report.next_reset,
                            &config.accounting_timezone
                        ),
                        "next_reset_client_display_rfc3339": format_local_time_rfc3339(
                            report.next_reset,
                            &config.client_display_timezone
                        ),
                    })
                })
                .unwrap_or_else(|error| serde_json::json!({ "error": error.to_string() }));
            let services = sbctl::lifecycle::service_status_entries(root)
                .into_iter()
                .map(|(unit, state)| (unit.to_owned(), state))
                .collect::<std::collections::BTreeMap<_, _>>();
            let certificate = (config.subscription_mode == sbctl::config::SubscriptionMode::Direct)
                .then(|| sbctl::certificate::status(&store, &config));
            let kernel_warning = sbctl::subscription::kernel_band_warning(
                super::config::resolve_sing_box_bin(root, None).as_deref(),
            );
            let kernel_version = super::config::resolve_sing_box_bin(root, None)
                .and_then(|binary| sbctl::observe::kernel_version_string(&binary));
            let sing_box = sing_box_status_json(root);
            let status = serde_json::json!({
                "configured": true,
                "mode": config.subscription_mode.to_string(),
                "subscription_host": config.subscription_host,
                "proxy_host": config.proxy_host.as_deref().unwrap_or(&config.subscription_host),
                "interface": config.interface,
                "monthly_traffic_limit": config.monthly_traffic_limit,
                "accounting_policy": config.accounting_policy.to_string(),
                "accounting_timezone": config.accounting_timezone,
                "client_display_timezone": config.client_display_timezone,
                "enabled_protocols": config
                    .enabled_protocols
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                "services": services,
                "traffic": traffic,
                "certificate": certificate,
                "kernel_version": kernel_version,
                "sing_box": sing_box,
                "kernel_version_warning": kernel_warning,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&status).expect("status JSON serializes")
            );
            ExitCode::SUCCESS
        }
        Err(sbctl::config::ConfigError::Missing) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "configured": false }))
                    .expect("status JSON serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("status failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// The `sing_box` object in `sbctl status --json`: process facts from systemd,
/// each `null` when the fact is unavailable so a degraded host still parses.
fn sing_box_status_json(root: &Path) -> serde_json::Value {
    let observation = sbctl::observe::observe_sing_box_process(root, chrono::Utc::now());
    serde_json::json!({
        "active_state": observation.active_state,
        "sub_state": observation.sub_state,
        "main_pid": observation.main_pid,
        "restart_count": observation.restart_count,
        "memory_bytes": observation.memory_bytes,
        "cpu_time_nsecs": observation.cpu_time_nsecs,
        "task_count": observation.task_count,
        "active_enter": observation.active_enter,
        "uptime_secs": observation.uptime.map(|duration| duration.as_secs()),
        "error": observation.error,
    })
}

pub(crate) fn print_traffic(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = match store.load() {
        Ok(config) => match sbctl::traffic::report(&store, &config) {
            Ok(report) => {
                print_traffic_report(&report, &config);
                return ExitCode::SUCCESS;
            }
            Err(error) => Err(error.to_string()),
        },
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("traffic failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn traffic_set_used(
    root: &Path,
    bytes: Option<u64>,
    rx: Option<u64>,
    tx: Option<u64>,
) -> ExitCode {
    let target = if let Some(bytes) = bytes {
        sbctl::traffic::CorrectionTarget::Total(bytes)
    } else {
        sbctl::traffic::CorrectionTarget::Directions {
            rx: rx.expect("validated: --rx requires --tx"),
            tx: tx.expect("validated: --tx requires --rx"),
        }
    };
    let store = sbctl::config::DeploymentStore::new(root);
    let result = match store.load() {
        Ok(config) => {
            sbctl::traffic::set_used(&store, &config, target).map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("traffic correction failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn run_accounting_reset(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = match store.load() {
        Ok(config) => sbctl::traffic::reset(&store, &config).map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(report) => {
            println!(
                "accounting period: {}; received: {} bytes; transmitted: {} bytes",
                report.accounting_period, report.received, report.transmitted
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("accounting reset failed: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_until_does_not_overflow_and_keeps_expired_fractional_days_negative() {
        assert_eq!(days_until_at(i64::MIN, 1), None);
        assert_eq!(days_until_at(i64::MAX, -1), None);
        assert_eq!(days_until_at(99, 100), Some(-1));
        assert_eq!(days_until_at(101, 100), Some(0));
        assert_eq!(days_until_at(86_500, 100), Some(1));
    }

    #[test]
    fn reset_time_offsets_distinguish_both_los_angeles_fall_back_hours() {
        let first = chrono::DateTime::parse_from_rfc3339("2026-11-01T08:30:00Z")
            .unwrap()
            .to_utc();
        let second = chrono::DateTime::parse_from_rfc3339("2026-11-01T09:30:00Z")
            .unwrap()
            .to_utc();
        assert_eq!(
            format_local_time(first, "America/Los_Angeles"),
            format_local_time(second, "America/Los_Angeles")
        );
        assert_eq!(
            format_local_time_rfc3339(first, "America/Los_Angeles"),
            "2026-11-01T01:30:00-07:00"
        );
        assert_eq!(
            format_local_time_rfc3339(second, "America/Los_Angeles"),
            "2026-11-01T01:30:00-08:00"
        );
    }

    #[test]
    fn usage_percent_is_finite_at_extreme_totals_and_unlimited_has_no_balance() {
        let unlimited = TrafficAllowance::new(u64::MAX, 0);
        assert_eq!(unlimited.remaining_bytes, None);
        assert_eq!(unlimited.usage_percent, None);
        assert_eq!(unlimited.over_limit_bytes, 0);
        let extreme = TrafficAllowance::new(u64::MAX, 1);
        assert_eq!(extreme.remaining_bytes, Some(0));
        assert_eq!(extreme.over_limit_bytes, u64::MAX - 1);
        assert!(extreme.usage_percent.unwrap().is_finite());
        assert_eq!(
            format_uptime(std::time::Duration::from_secs(90_061)),
            "1 天 1 小时 1 分 1 秒"
        );
    }
}
