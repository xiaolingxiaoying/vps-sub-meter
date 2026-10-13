//! Host-level commands that are not about the sing-box deployment: the BBR
//! tuning helpers and the subscription credential rotation, plus the host
//! facts the menu header prints.

use crate::cli::args::{CredentialCommand, SystemCommand};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub(crate) fn system_info() -> (String, String, String, String) {
    let os = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find(|line| line.starts_with("PRETTY_NAME="))
                .map(|line| line["PRETTY_NAME=".len()..].trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let kernel =
        fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_else(|_| "unknown".to_owned());
    let bbr = fs::read_to_string("/proc/sys/net/ipv4/tcp_congestion_control")
        .unwrap_or_else(|_| "unknown".to_owned());
    let cpu = std::env::consts::ARCH.to_owned();
    (os, kernel.trim().to_owned(), cpu, bbr.trim().to_owned())
}

pub(crate) fn run_system(root: &Path, command: SystemCommand) -> ExitCode {
    if matches!(&command, SystemCommand::DiagnoseSubscription { .. }) {
        let diagnosis = sbctl::system::diagnose_subscription(root);
        return match serde_json::to_string_pretty(&diagnosis) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(_) => {
                eprintln!("system diagnosis failed: could not serialize the report");
                ExitCode::from(2)
            }
        };
    }
    let runtime = sbctl::runtime::Runtime::live(root);
    let result = match command {
        SystemCommand::Bbr => sbctl::system::enable_bbr(&runtime).map(|status| {
            println!(
                "BBR enabled: tcp_congestion_control={}, default_qdisc={}",
                status.congestion_control, status.qdisc
            );
        }),
        SystemCommand::Status => sbctl::system::read_current(&runtime).map(|status| {
            println!("tcp_congestion_control={}", status.congestion_control);
            println!("default_qdisc={}", status.qdisc);
        }),
        SystemCommand::DiagnoseSubscription { .. } => unreachable!("handled above"),
    }
    .map_err(|error| error.to_string());
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("system operation failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn run_credential(root: &Path, command: CredentialCommand) -> ExitCode {
    match command {
        CredentialCommand::Rotate {
            name,
            grace,
            install_timer,
            remove_timer,
        } => {
            if install_timer && remove_timer {
                eprintln!("credential rotate failed: --install-timer 与 --remove-timer 互斥");
                return ExitCode::from(2);
            }
            if install_timer || remove_timer {
                return set_rotation_timer(root, install_timer);
            }
            rotate_credential(root, name.as_deref(), grace.as_deref())
        }
        CredentialCommand::Add { name } => add_credential(root, &name),
        CredentialCommand::Revoke { name, grace } => {
            revoke_credential(root, &name, grace.as_deref())
        }
        CredentialCommand::List => list_credentials(root),
    }
}

/// Opt-in monthly rotation of the default credential. Unattended rotation
/// invalidates every saved link at once, so it is never implied by enabling
/// `credential` handling elsewhere: this verb, or nothing.
fn set_rotation_timer(root: &Path, enable: bool) -> ExitCode {
    match sbctl::lifecycle::set_credential_rotate_timer(root, enable) {
        Ok(()) => {
            if enable {
                println!("已启用每月自动轮换默认订阅凭据（sbctl-credential-rotate.timer）");
                println!("提醒：每次轮换都会让旧链接立即失效，所有设备需重新导入。");
                println!(
                    "用 sbctl credential rotate --name <设备> --grace 30m 做平滑的设备级轮换。"
                );
            } else {
                println!("已移除每月自动轮换定时器（sbctl-credential-rotate.timer）");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("定时轮换设置失败: {error}");
            ExitCode::from(2)
        }
    }
}

/// Parses a grace window such as `30m`, `2h` or `1d`.
fn parse_grace(text: &str) -> Result<i64, String> {
    let text = text.trim();
    let unit = text
        .as_bytes()
        .last()
        .copied()
        .filter(u8::is_ascii_alphabetic)
        .ok_or_else(|| "时长需为非负数字加 s/m/h/d 单位".to_owned())?;
    let digits = &text[..text.len() - 1];
    let amount = digits
        .parse::<i64>()
        .map_err(|_| "时长数字无效或过大".to_owned())?;
    if amount < 0 {
        return Err("宽限期不能为负".to_owned());
    }
    let multiplier = match unit.to_ascii_lowercase() {
        b's' => 1,
        b'm' => 60,
        b'h' => 3_600,
        b'd' => 86_400,
        _ => return Err("时长单位需为 s/m/h/d".to_owned()),
    };
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| "宽限期过大".to_owned())
}

/// Keeps only the credential's first six characters visible. The full value is
/// a URL path secret; `sbctl sub` is the command that prints it, to a terminal.
fn mask(credential: &str) -> String {
    let head = &credential[..credential.len().min(6)];
    format!("{head}…（{} 字符）", credential.len())
}

fn credential_status(entry: &sbctl::config::NamedCredential, now: i64) -> String {
    match entry.revoked_at {
        None => "生效中".to_owned(),
        Some(expires) if expires > now => format!("宽限至 {expires}"),
        Some(_) => "已失效".to_owned(),
    }
}

fn list_credentials(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    match store.load() {
        Ok(config) => {
            let now = chrono::Utc::now().timestamp();
            println!("default  {}  生效中", mask(&config.subscription_credential));
            for entry in &config.subscription_credentials {
                println!(
                    "{:<8} {}  {}",
                    entry.name,
                    mask(&entry.credential),
                    credential_status(entry, now)
                );
            }
            println!("\n完整链接（含凭据）只在终端输出: sbctl sub");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential list failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// Keep the lock through restart and recovery, so a failed command cannot
/// overwrite a concurrent credential change with its old snapshot.
fn credential_transaction<T>(
    root: &Path,
    change: impl FnOnce(&mut sbctl::config::DeploymentConfig) -> Result<T, sbctl::config::ConfigError>,
) -> Result<T, sbctl::config::ConfigError> {
    sbctl::preflight::require_install_privileges(root)
        .map_err(|error| credential_error(error.to_string()))?;
    let store = sbctl::config::DeploymentStore::new(root);
    let _lock = store.acquire_operation_lock()?;
    let mut config = store.load()?;
    let prior = fs::read(root.join(sbctl::config::CONFIG_RELATIVE_PATH))
        .map_err(sbctl::config::ConfigError::Storage)?;
    let output = change(&mut config)?;
    config.validate()?;
    let commit = store.replace_locked(&config).and_then(|()| {
        sbctl::lifecycle::restart_services(root).map_err(sbctl::config::ConfigError::StateContent)
    });
    if let Err(error) = commit {
        if let Err(recovery) =
            store.write_relative_locked(sbctl::config::CONFIG_RELATIVE_PATH, &prior)
        {
            return Err(sbctl::config::ConfigError::StateContent(format!(
                "{error}; 配置恢复失败：{recovery}；请检查服务与配置"
            )));
        }
        if let Err(recovery) = sbctl::lifecycle::restart_services(root) {
            return Err(sbctl::config::ConfigError::StateContent(format!(
                "{error}; 旧配置已恢复，但恢复重启失败：{recovery}"
            )));
        }
        return Err(error);
    }
    Ok(output)
}

fn credential_error(message: impl Into<String>) -> sbctl::config::ConfigError {
    sbctl::config::ConfigError::StateContent(message.into())
}

fn grace_seconds(grace: Option<&str>) -> Result<i64, sbctl::config::ConfigError> {
    grace
        .map(parse_grace)
        .transpose()
        .map(|value| value.unwrap_or(0))
        .map_err(credential_error)
}

fn expiry(now: i64, seconds: i64) -> Result<i64, sbctl::config::ConfigError> {
    now.checked_add(seconds)
        .ok_or_else(|| credential_error("宽限期到期时间溢出"))
}

fn named_url(
    config: &sbctl::config::DeploymentConfig,
    credential: &str,
) -> Result<String, sbctl::config::ConfigError> {
    sbctl::subscription::route_url_with_credential(
        config,
        credential,
        sbctl::subscription::SubscriptionRoute::Format(
            sbctl::subscription::SubscriptionFormat::SingBoxFull,
        ),
    )
    .map_err(|error| credential_error(error.to_string()))
}

fn add_credential(root: &Path, name: &str) -> ExitCode {
    let result = credential_transaction(root, |config| {
        let now = chrono::Utc::now().timestamp();
        if config
            .subscription_credentials
            .iter()
            .any(|entry| entry.name == name && entry.is_active(now))
        {
            return Err(credential_error(format!("已存在生效中的凭据 {name:?}")));
        }
        let entry = sbctl::config::NamedCredential {
            name: name.to_owned(),
            credential: sbctl::config::generate_subscription_credential()?,
            revoked_at: None,
        };
        let url = named_url(config, &entry.credential)?;
        match config
            .subscription_credentials
            .iter_mut()
            .find(|existing| existing.name == name)
        {
            Some(existing) => *existing = entry,
            None => config.subscription_credentials.push(entry),
        }
        Ok(url)
    });
    match result {
        Ok(url) => {
            println!(
                "已新增命名凭据 {name}，sbctl 服务已重载（订阅工件内容未变）\n凭据 {name} 链接: {url}"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential add failed: {error}");
            ExitCode::from(2)
        }
    }
}

fn revoke_credential(root: &Path, name: &str, grace: Option<&str>) -> ExitCode {
    let result = grace_seconds(grace).and_then(|window| {
        credential_transaction(root, |config| {
            if name == "default" {
                return Err(credential_error(
                    "默认凭据不能吊销；用 sbctl credential rotate 轮换它",
                ));
            }
            let now = chrono::Utc::now().timestamp();
            let until = expiry(now, window)?;
            let entry = config
                .subscription_credentials
                .iter_mut()
                .find(|entry| entry.name == name && entry.is_active(now))
                .ok_or_else(|| {
                    credential_error(format!(
                        "没有生效中的凭据 {name:?}（sbctl credential list 可查）"
                    ))
                })?;
            entry.revoked_at = Some(until);
            Ok(window)
        })
    });
    match result {
        Ok(window) => {
            if window > 0 {
                println!("凭据 {name} 将在宽限期结束后失效");
            } else {
                println!("凭据 {name} 已立即失效（其他链接不受影响）");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential revoke failed: {error}");
            ExitCode::from(2)
        }
    }
}

fn rotate_credential(root: &Path, name: Option<&str>, grace: Option<&str>) -> ExitCode {
    let window = match grace_seconds(grace) {
        Ok(window) => window,
        Err(error) => {
            eprintln!("credential rotate failed: {error}");
            return ExitCode::from(2);
        }
    };
    let Some(name) = name else {
        if window > 0 {
            eprintln!("credential rotate failed: 默认凭据不支持宽限期；使用 --name 轮换设备凭据");
            return ExitCode::from(2);
        }
        return rotate_subscription_credential(root);
    };
    let result = credential_transaction(root, |config| {
        let now = chrono::Utc::now().timestamp();
        let until = expiry(now, window)?;
        let index = config
            .subscription_credentials
            .iter()
            .position(|entry| entry.name == name && entry.is_active(now))
            .ok_or_else(|| {
                credential_error(format!(
                    "没有生效中的凭据 {name:?}（sbctl credential list 可查）"
                ))
            })?;
        if window > 0 {
            let mut retired = config.subscription_credentials[index].clone();
            retired.revoked_at = Some(until);
            config.subscription_credentials.push(retired);
        }
        let fresh = sbctl::config::generate_subscription_credential()?;
        let url = named_url(config, &fresh)?;
        config.subscription_credentials[index].credential = fresh;
        config.subscription_credentials[index].revoked_at = None;
        Ok(url)
    });
    match result {
        Ok(url) => {
            if window > 0 {
                println!("凭据 {name} 已轮换；旧链接在 {window} 秒内仍然有效，之后失效");
            } else {
                println!("凭据 {name} 已轮换；该设备的旧链接立即失效（其他凭据不受影响）");
            }
            println!("凭据 {name} 链接: {url}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential rotation failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn rotate_subscription_credential(root: &Path) -> ExitCode {
    let result = credential_transaction(root, |config| {
        config.subscription_credential = sbctl::config::generate_subscription_credential()?;
        Ok(())
    });
    match result {
        Ok(()) => {
            println!(
                "subscription credential rotated; all previous subscription URLs are now invalid"
            );
            println!("run 'sbctl sub' to display the new subscription URLs");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential rotation failed: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grace_validation_handles_empty_unicode_and_overflow() {
        for text in [
            "",
            " ",
            "30分",
            "-1h",
            "9223372036854775807d",
            "999999999999999999999s",
            "30x",
        ] {
            assert!(parse_grace(text).is_err(), "{text:?}");
        }
        for (text, seconds) in [("0s", 0), ("30M", 1800), (" 2h ", 7200), ("1D", 86400)] {
            assert_eq!(parse_grace(text).unwrap(), seconds);
        }
        assert!(expiry(i64::MAX, 1).is_err());
    }
}
