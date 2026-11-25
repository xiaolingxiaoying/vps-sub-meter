//! Interface byte accounting with `boot_id` rollback protection and timezone-aware
//! monthly anchored resets.
//!
//! Client perspective convention (matching Clash/Mihomo/Shadowrocket `Subscription-Userinfo`):
//! - VPS `rx_bytes` (received by VPS)  => client `upload`
//! - VPS `tx_bytes` (transmitted by VPS) => client `download`

use chrono::{DateTime, Datelike, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::{Config, VAR_DIR};
use crate::fs_util;

pub const TRAFFIC_REL: &str = "var/lib/vps-sub-meter/traffic-state.json";

#[derive(Debug, thiserror::Error)]
pub enum TrafficError {
    #[error("读取网卡流量统计失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("解析流量状态失败: {0}")]
    Parse(String),
    #[error("无效的时间或时区配置: {0}")]
    Schedule(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrafficState {
    pub schema: u32,
    pub interface: String,
    pub boot_id: String,
    pub baseline_rx: u64,
    pub baseline_tx: u64,
    pub accumulated_rx: u64,
    pub accumulated_tx: u64,
    pub cycle_next_reset_ts: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrafficSnapshot {
    pub interface: String,
    /// Client upload = VPS rx_bytes
    pub upload_bytes: u64,
    /// Client download = VPS tx_bytes
    pub download_bytes: u64,
    pub monthly_limit_bytes: u64,
    pub next_reset_ts: i64,
}

impl TrafficSnapshot {
    pub fn total_used(&self) -> u64 {
        self.upload_bytes.saturating_add(self.download_bytes)
    }

    pub fn userinfo_header(&self) -> String {
        if self.monthly_limit_bytes > 0 {
            format!(
                "upload={}; download={}; total={}; expire={}; profile-update-interval=24",
                self.upload_bytes,
                self.download_bytes,
                self.monthly_limit_bytes,
                self.next_reset_ts
            )
        } else {
            format!(
                "upload={}; download={}; expire={}; profile-update-interval=24",
                self.upload_bytes, self.download_bytes, self.next_reset_ts
            )
        }
    }
}

pub fn state_path(root: &Path) -> PathBuf {
    root.join(TRAFFIC_REL)
}

pub fn read_iface_counters(root: &Path, iface: &str) -> Result<(u64, u64), TrafficError> {
    let base = root.join("sys/class/net").join(iface).join("statistics");
    let rx_s = fs_util::read_to_string_no_follow(&base.join("rx_bytes"))?;
    let tx_s = fs_util::read_to_string_no_follow(&base.join("tx_bytes"))?;
    let rx = rx_s
        .trim()
        .parse::<u64>()
        .map_err(|_| TrafficError::Parse("rx_bytes 非整数".into()))?;
    let tx = tx_s
        .trim()
        .parse::<u64>()
        .map_err(|_| TrafficError::Parse("tx_bytes 非整数".into()))?;
    Ok((rx, tx))
}

pub fn read_boot_id(root: &Path) -> String {
    fs_util::read_to_string_no_follow(&root.join("proc/sys/kernel/random/boot_id"))
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".into())
}

/// Computes the next reset Unix timestamp strictly greater than `now` in `reset_tz`.
pub fn next_reset_after(config: &Config, now: DateTime<Utc>) -> Result<i64, TrafficError> {
    let tz: Tz = config
        .traffic
        .reset_tz
        .parse()
        .map_err(|_| TrafficError::Schedule("无效 reset_tz".into()))?;
    let (hour, min) = config
        .parse_reset_hm()
        .map_err(|e| TrafficError::Schedule(e.to_string()))?;
    let local_now = now.with_timezone(&tz);
    let mut year = local_now.year();
    let mut month = local_now.month();
    for _ in 0..24 {
        let day = u32::from(config.traffic.reset_day).min(days_in_month(year, month));
        let dt = resolve_local_datetime(tz, year, month, day, hour, min);
        if dt.timestamp() > now.timestamp() {
            return Ok(dt.timestamp());
        }
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    Err(TrafficError::Schedule("无法计算下次重置时间".into()))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .unwrap()
        .pred_opt()
        .unwrap()
        .day()
}

fn resolve_local_datetime(
    tz: Tz,
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    min: u32,
) -> DateTime<Utc> {
    let naive = NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, min, 0)
        .unwrap();
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => dt.with_timezone(&Utc),
        LocalResult::None => {
            // Spring-forward gap: advance 1 hour to the next valid instant.
            let shifted = naive + chrono::Duration::hours(1);
            tz.from_local_datetime(&shifted)
                .earliest()
                .unwrap()
                .with_timezone(&Utc)
        }
    }
}

pub fn load_state(root: &Path) -> Result<Option<TrafficState>, TrafficError> {
    match fs_util::read_no_follow(&state_path(root)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| TrafficError::Parse(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn save_state(root: &Path, state: &TrafficState) -> Result<(), TrafficError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| TrafficError::Parse(e.to_string()))?;
    fs_util::write_atomic(&state_path(root), &bytes, 0o640)?;
    Ok(())
}

/// Pure read-only live traffic calculation for HTTP responses and menu status.
pub fn snapshot_at(
    root: &Path,
    config: &Config,
    now: DateTime<Utc>,
) -> Result<TrafficSnapshot, TrafficError> {
    let (rx, tx) = read_iface_counters(root, &config.interface)?;
    let boot_id = read_boot_id(root);
    let next_reset = next_reset_after(config, now)?;
    let Some(state) = load_state(root)? else {
        return Ok(TrafficSnapshot {
            interface: config.interface.clone(),
            upload_bytes: 0,
            download_bytes: 0,
            monthly_limit_bytes: config.monthly_limit_bytes(),
            next_reset_ts: next_reset,
        });
    };
    if state.interface != config.interface || now.timestamp() >= state.cycle_next_reset_ts {
        return Ok(TrafficSnapshot {
            interface: config.interface.clone(),
            upload_bytes: 0,
            download_bytes: 0,
            monthly_limit_bytes: config.monthly_limit_bytes(),
            next_reset_ts: next_reset,
        });
    }
    let delta_rx = if state.boot_id == boot_id && rx >= state.baseline_rx {
        rx - state.baseline_rx
    } else {
        0
    };
    let delta_tx = if state.boot_id == boot_id && tx >= state.baseline_tx {
        tx - state.baseline_tx
    } else {
        0
    };
    Ok(TrafficSnapshot {
        interface: config.interface.clone(),
        upload_bytes: state.accumulated_rx.saturating_add(delta_rx),
        download_bytes: state.accumulated_tx.saturating_add(delta_tx),
        monthly_limit_bytes: config.monthly_limit_bytes(),
        next_reset_ts: state.cycle_next_reset_ts,
    })
}

pub fn snapshot(root: &Path, config: &Config) -> Result<TrafficSnapshot, TrafficError> {
    snapshot_at(root, config, Utc::now())
}

/// Persists accumulated deltas and handles monthly reset rollover.
pub fn checkpoint_at(
    root: &Path,
    config: &Config,
    now: DateTime<Utc>,
) -> Result<TrafficSnapshot, TrafficError> {
    let _lock = fs_util::open_lock(&root.join(VAR_DIR))?;
    fs2::FileExt::lock_exclusive(&_lock)?;
    let (rx, tx) = read_iface_counters(root, &config.interface)?;
    let boot_id = read_boot_id(root);
    let next_reset = next_reset_after(config, now)?;
    let state = match load_state(root)? {
        Some(mut st)
            if st.interface == config.interface && now.timestamp() < st.cycle_next_reset_ts =>
        {
            let delta_rx = if st.boot_id == boot_id && rx >= st.baseline_rx {
                rx - st.baseline_rx
            } else {
                0
            };
            let delta_tx = if st.boot_id == boot_id && tx >= st.baseline_tx {
                tx - st.baseline_tx
            } else {
                0
            };
            st.accumulated_rx = st.accumulated_rx.saturating_add(delta_rx);
            st.accumulated_tx = st.accumulated_tx.saturating_add(delta_tx);
            st.baseline_rx = rx;
            st.baseline_tx = tx;
            st.boot_id = boot_id;
            st
        }
        _ => TrafficState {
            schema: 1,
            interface: config.interface.clone(),
            boot_id,
            baseline_rx: rx,
            baseline_tx: tx,
            accumulated_rx: 0,
            accumulated_tx: 0,
            cycle_next_reset_ts: next_reset,
        },
    };
    save_state(root, &state)?;
    Ok(TrafficSnapshot {
        interface: state.interface,
        upload_bytes: state.accumulated_rx,
        download_bytes: state.accumulated_tx,
        monthly_limit_bytes: config.monthly_limit_bytes(),
        next_reset_ts: state.cycle_next_reset_ts,
    })
}

pub fn checkpoint(root: &Path, config: &Config) -> Result<TrafficSnapshot, TrafficError> {
    checkpoint_at(root, config, Utc::now())
}

/// Manually sets the current cycle's client upload (VPS rx) and download (VPS tx) bytes.
pub fn set_used(
    root: &Path,
    config: &Config,
    upload_bytes: u64,
    download_bytes: u64,
) -> Result<TrafficSnapshot, TrafficError> {
    let _lock = fs_util::open_lock(&root.join(VAR_DIR))?;
    fs2::FileExt::lock_exclusive(&_lock)?;
    let (rx, tx) = read_iface_counters(root, &config.interface)?;
    let boot_id = read_boot_id(root);
    let next_reset = next_reset_after(config, Utc::now())?;
    let state = TrafficState {
        schema: 1,
        interface: config.interface.clone(),
        boot_id,
        baseline_rx: rx,
        baseline_tx: tx,
        accumulated_rx: upload_bytes,
        accumulated_tx: download_bytes,
        cycle_next_reset_ts: next_reset,
    };
    save_state(root, &state)?;
    Ok(TrafficSnapshot {
        interface: state.interface,
        upload_bytes,
        download_bytes,
        monthly_limit_bytes: config.monthly_limit_bytes(),
        next_reset_ts: next_reset,
    })
}

pub fn format_iec(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut val = bytes as f64;
    let mut idx = 0;
    while val >= 1024.0 && idx < UNITS.len() - 1 {
        val /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{val:.2} {}", UNITS[idx])
    }
}

pub fn format_time_in_tz(ts: i64, tz_name: &str) -> String {
    let tz: Tz = tz_name.parse().unwrap_or(chrono_tz::UTC);
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|dt| dt.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn seed_iface(root: &Path, iface: &str, boot: &str, rx: u64, tx: u64) {
        let stats = root.join("sys/class/net").join(iface).join("statistics");
        fs::create_dir_all(&stats).unwrap();
        fs::write(stats.join("rx_bytes"), rx.to_string()).unwrap();
        fs::write(stats.join("tx_bytes"), tx.to_string()).unwrap();
        let proc_dir = root.join("proc/sys/kernel/random");
        fs::create_dir_all(&proc_dir).unwrap();
        fs::write(proc_dir.join("boot_id"), boot).unwrap();
    }

    #[test]
    fn maps_vps_rx_to_client_upload_and_survives_reboot() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            domain: "sub.example.com".into(),
            ..Default::default()
        };
        cfg.traffic.monthly_gb = 500.0;
        cfg.traffic.reset_day = 11;
        cfg.traffic.reset_time = "01:32".into();

        seed_iface(dir.path(), "eth0", "boot-1", 1000, 5000);
        let t0 = DateTime::parse_from_rfc3339("2026-11-25T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let s0 = checkpoint_at(dir.path(), &cfg, t0).unwrap();
        assert_eq!(s0.upload_bytes, 0);
        assert_eq!(s0.download_bytes, 0);

        seed_iface(dir.path(), "eth0", "boot-1", 1500, 7000);
        let s1 = snapshot_at(dir.path(), &cfg, t0).unwrap();
        assert_eq!(s1.upload_bytes, 500);
        assert_eq!(s1.download_bytes, 2000);
        checkpoint_at(dir.path(), &cfg, t0).unwrap();

        // Reboot resets NIC counters and changes boot_id: accumulated values remain.
        seed_iface(dir.path(), "eth0", "boot-2", 100, 200);
        let s2 = snapshot_at(dir.path(), &cfg, t0).unwrap();
        assert_eq!(s2.upload_bytes, 500);
        assert_eq!(s2.download_bytes, 2000);
    }

    #[test]
    fn anchored_los_angeles_reset_rolls_over_and_converts_to_shanghai() {
        let mut cfg = Config::default();
        cfg.traffic.reset_day = 11;
        cfg.traffic.reset_time = "01:32".into();
        cfg.traffic.reset_tz = "America/Los_Angeles".into();
        cfg.traffic.display_tz = "Asia/Shanghai".into();
        let now = DateTime::parse_from_rfc3339("2026-11-25T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = next_reset_after(&cfg, now).unwrap();
        // 2026-12-11 01:32 PST (-08:00) == 2026-12-11 17:32 +08:00 == 1796981520
        assert_eq!(next, 1_796_981_520);
        assert_eq!(
            format_time_in_tz(next, &cfg.traffic.display_tz),
            "2026-12-11 17:32"
        );
    }
}
