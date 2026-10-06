//! Host-level commands that are not about the sing-box deployment: the BBR
//! tuning helpers and the subscription credential rotation, plus the host
//! facts the menu header prints.

use crate::cli::args::{CredentialCommand, SystemCommand};
use crate::cli::commands::config::restart_services_with_rollback;
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
    let (digits, unit) = text.trim().split_at(text.trim().len() - 1);
    let amount = digits
        .parse::<i64>()
        .map_err(|_| format!("无法解析时长 {text:?}"))?;
    if amount < 0 {
        return Err("宽限期不能为负".to_owned());
    }
    Ok(match unit.to_ascii_lowercase().as_str() {
        "s" => amount,
        "m" => amount * 60,
        "h" => amount * 3_600,
        "d" => amount * 86_400,
        _ => return Err(format!("时长单位需为 s/m/h/d，收到 {text:?}")),
    })
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

fn add_credential(root: &Path, name: &str) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = (|| -> Result<(), sbctl::config::ConfigError> {
        let mut config = store.load()?;
        let now = chrono::Utc::now().timestamp();
        if config
            .subscription_credentials
            .iter()
            .any(|entry| entry.name == name && entry.is_active(now))
        {
            return Err(sbctl::config::ConfigError::StateContent(format!(
                "已存在生效中的凭据 {name:?}"
            )));
        }
        let entry = sbctl::config::NamedCredential {
            name: name.to_owned(),
            credential: sbctl::config::generate_subscription_credential()?,
            revoked_at: None,
        };
        println!(
            "凭据 {name} 链接: {}",
            sbctl::subscription::route_url_with_credential(
                &config,
                &entry.credential,
                sbctl::subscription::SubscriptionRoute::Format(
                    sbctl::subscription::SubscriptionFormat::SingBoxFull,
                ),
            )
            .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?
        );
        match config
            .subscription_credentials
            .iter_mut()
            .find(|existing| existing.name == name)
        {
            Some(existing) => *existing = entry,
            None => config.subscription_credentials.push(entry),
        }
        config.validate()?;
        let previous = config.subscription_credentials.clone();
        store.replace(&config)?;
        // The credential is a URL path secret, not artifact content: nothing is
        // regenerated, the daemon just has to reload the accepted set, which it
        // reads once at startup.
        restart_services_with_rollback(root, || {
            let mut rollback = config.clone();
            rollback.subscription_credentials = previous;
            if let Err(error) = store.replace(&rollback) {
                eprintln!(
                    "warning: restoring the previous credential list failed ({error}); \
                     the new link may already be active"
                );
            }
        })?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            println!("已新增命名凭据 {name}，sbctl 服务已重载（订阅工件内容未变）");
            println!("run 'sbctl sub' to display the default subscription URLs");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential add failed: {error}");
            ExitCode::from(2)
        }
    }
}

fn revoke_credential(root: &Path, name: &str, grace: Option<&str>) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = (|| -> Result<(), sbctl::config::ConfigError> {
        let mut config = store.load()?;
        if name == "default" {
            return Err(sbctl::config::ConfigError::StateContent(
                "默认凭据不能吊销；用 sbctl credential rotate 轮换它".to_owned(),
            ));
        }
        let window = grace
            .map(parse_grace)
            .transpose()
            .map_err(sbctl::config::ConfigError::StateContent)?
            .unwrap_or(0);
        let now = chrono::Utc::now().timestamp();
        let Some(entry) = config
            .subscription_credentials
            .iter_mut()
            .find(|entry| entry.name == name && entry.is_active(now))
        else {
            return Err(sbctl::config::ConfigError::StateContent(format!(
                "没有生效中的凭据 {name:?}（sbctl credential list 可查）"
            )));
        };
        // An immediate revoke is an expiry in the past-by-construction, not a
        // missing field: `revoked_at = None` means "never expires", so writing
        // None here would have kept serving the link the operator just retired.
        entry.revoked_at = Some(if window > 0 { now + window } else { now });
        config.validate()?;
        let previous = config.subscription_credentials.clone();
        store.replace(&config)?;
        restart_services_with_rollback(root, || {
            let mut rollback = config.clone();
            rollback.subscription_credentials = previous;
            if let Err(error) = store.replace(&rollback) {
                eprintln!(
                    "warning: restoring the previous credential list failed ({error}); \
                     the revocation may already be in effect"
                );
            }
        })?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            if window_of(grace) > 0 {
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

/// The grace window in seconds, for the message that reports it.
fn window_of(grace: Option<&str>) -> i64 {
    grace.and_then(|text| parse_grace(text).ok()).unwrap_or(0)
}

/// `sbctl credential rotate [OPTIONS]`: the default link when no `--name` is
/// given (the historical one-shot behaviour), or one named link, whose previous
/// secret optionally survives until the grace window expires.
fn rotate_credential(root: &Path, name: Option<&str>, grace: Option<&str>) -> ExitCode {
    let Some(name) = name else {
        return rotate_subscription_credential(root);
    };
    let window = match grace.map(parse_grace).transpose() {
        Ok(window) => window.unwrap_or(0),
        Err(message) => {
            eprintln!("credential rotate failed: {message}");
            return ExitCode::from(2);
        }
    };
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|mut config| {
        let now = chrono::Utc::now().timestamp();
        let Some(index) = config
            .subscription_credentials
            .iter()
            .position(|entry| entry.name == name && entry.is_active(now))
        else {
            return Err(sbctl::config::ConfigError::StateContent(format!(
                "没有生效中的凭据 {name:?}（sbctl credential list 可查）"
            )));
        };
        let previous = config.subscription_credentials[index].credential.clone();
        // A grace window is kept by *remembering the retiring secret*: the live
        // record takes the fresh value, and the old one stays as a second
        // record that expires on its own. Dropping the old secret here would
        // make `--grace` a promise nothing holds.
        if window > 0 {
            config
                .subscription_credentials
                .push(sbctl::config::NamedCredential {
                    name: name.to_owned(),
                    credential: previous.clone(),
                    revoked_at: Some(now + window),
                });
        }
        let entry = &mut config.subscription_credentials[index];
        entry.credential = sbctl::config::generate_subscription_credential()?;
        entry.revoked_at = None;
        config.validate()?;
        store.replace(&config)?;
        restart_services_with_rollback(root, || {
            let mut rollback = config.clone();
            // Undo both halves: drop the retired record and put the old secret
            // back on the live one.
            rollback
                .subscription_credentials
                .retain(|entry| !(entry.name == name && entry.credential == previous));
            if let Some(entry) = rollback
                .subscription_credentials
                .iter_mut()
                .find(|entry| entry.name == name)
            {
                entry.credential = previous.clone();
            }
            if let Err(error) = store.replace(&rollback) {
                eprintln!(
                    "warning: restoring the previous credential failed ({error}); \
                     the rotated link may already be active"
                );
            }
        })?;
        Ok(())
    });
    match result {
        Ok(()) => {
            if window > 0 {
                println!("凭据 {name} 已轮换；旧链接在 {window} 秒内仍然有效，之后失效");
            } else {
                println!("凭据 {name} 已轮换；该设备的旧链接立即失效（其他凭据不受影响）");
            }
            println!("run 'sbctl sub' to display the subscription URLs");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("credential rotation failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn rotate_subscription_credential(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|mut config| {
        let previous = config.subscription_credential.clone();
        config.subscription_credential = sbctl::config::generate_subscription_credential()?;
        store.replace(&config)?;
        restart_services_with_rollback(root, || {
            config.subscription_credential = previous;
            if let Err(error) = store.replace(&config) {
                eprintln!(
                    "warning: restoring the previous subscription credential failed ({error}); \
                     the rotated credential may still be active"
                );
            }
        })?;
        Ok(config)
    });
    match result {
        Ok(_) => {
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
