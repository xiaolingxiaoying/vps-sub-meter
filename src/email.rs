//! Opt-in SMTP reports. Secrets stay in root-only files, never in argv/journald.
use crate::config::{DeploymentConfig, DeploymentStore};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

pub const CONFIG_PATH: &str = "etc/sbctl/email.toml";
const STATE_PATH: &str = "var/lib/sbctl/email-state.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    /// true: implicit TLS (465); false: mandatory STARTTLS (587).
    pub implicit_tls: bool,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: String,
    pub daily_report: bool,
    pub include_subscription: bool,
    /// Send once per accounting period within this many hours before reset.
    pub reset_reminder_hours: u32,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            implicit_tls: true,
            username: "admin@example.com".into(),
            password: "CHANGE_ME".into(),
            from: "admin@example.com".into(),
            to: "admin@example.com".into(),
            daily_report: true,
            include_subscription: false,
            reset_reminder_hours: 24,
        }
    }
}

impl EmailConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !crate::config::hostname_is_valid(&self.smtp_host) || self.smtp_port == 0 {
            return Err("SMTP 主机或端口无效".into());
        }
        // Only bare mailboxes, no display-name syntax or SMTP/header injection.
        for address in [&self.from, &self.to] {
            if !crate::certificate::acme_email_is_valid(address)
                || address.contains(['<', '>', ',', ';', '"', '\\'])
                || address.chars().any(char::is_control)
            {
                return Err("发件人 / 收件人必须是单个邮箱地址".into());
            }
        }
        if self.username.is_empty()
            || self.username.contains(['\r', '\n', '\0', ':'])
            || self.password.is_empty()
            || self.password == "CHANGE_ME"
            || self.password.contains(['\r', '\n', '\0'])
        {
            return Err("请设置有效 SMTP 用户名和授权密码".into());
        }
        if self.reset_reminder_hours > 744 {
            return Err("刷新提醒范围为 0–744 小时（0 关闭）".into());
        }
        Ok(())
    }
}

pub fn load(root: &Path) -> Result<EmailConfig, String> {
    let config: EmailConfig = toml::from_str(
        &fs::read_to_string(root.join(CONFIG_PATH))
            .map_err(|_| "邮件尚未配置；运行 sbctl email configure".to_owned())?,
    )
    .map_err(|_| "邮件配置格式无效".to_owned())?;
    config.validate()?;
    Ok(config)
}

#[derive(Default, Serialize, Deserialize)]
struct DeliveryState {
    daily: Option<String>,
    reminder: Option<String>,
}

fn due(
    state: &DeliveryState,
    config: &EmailConfig,
    now: DateTime<Utc>,
    next_reset: Option<DateTime<Utc>>,
    display_timezone: &str,
) -> (bool, bool) {
    let day = now
        .with_timezone(
            &display_timezone
                .parse::<chrono_tz::Tz>()
                .unwrap_or(chrono_tz::UTC),
        )
        .format("%Y-%m-%d")
        .to_string();
    let daily = config.daily_report && state.daily.as_deref() != Some(day.as_str());
    let reminder = next_reset.is_some_and(|reset| {
        let remaining = reset.signed_duration_since(now).num_seconds();
        config.reset_reminder_hours > 0
            && remaining > 0
            && remaining <= i64::from(config.reset_reminder_hours) * 3600
            && state.reminder.as_deref() != Some(reset.to_rfc3339().as_str())
    });
    (daily, reminder)
}

/// A read-only traffic report; sending mail never resets accounting state.
pub fn report(
    root: &Path,
    config: &DeploymentConfig,
    include_subscription: bool,
    reminder: bool,
) -> Result<String, String> {
    let mut text = format!(
        "VPS：{}\n时间：{}\n",
        config.subscription_host,
        Utc::now().to_rfc3339()
    );
    for (label, file) in [
        ("系统运行秒数", "proc/uptime"),
        ("系统负载", "proc/loadavg"),
    ] {
        if let Ok(value) = fs::read_to_string(root.join(file)) {
            text.push_str(&format!("{label}：{}\n", value.trim()));
        }
    }
    if let Ok(memory) = fs::read_to_string(root.join("proc/meminfo")) {
        for line in memory
            .lines()
            .filter(|line| line.starts_with("MemTotal:") || line.starts_with("MemAvailable:"))
        {
            text.push_str(&format!("{line}\n"));
        }
    }
    if reminder {
        text.push_str("提醒：VPS 流量账期即将刷新。\n");
    }
    for (unit, status) in crate::lifecycle::service_status_entries(root) {
        text.push_str(&format!("{unit}: {status}\n"));
    }
    let binary = root.join("usr/local/bin/sing-box");
    text.push_str(&format!(
        "内核：{}\n",
        crate::observe::kernel_version_string(&binary)
            .unwrap_or_else(|| "未安装 / 无法读取".into())
    ));
    match crate::traffic::report(&DeploymentStore::new(root), config) {
        Ok(traffic) => {
            text.push_str(&format!(
                "网卡：{}\nRX 入站：{} GiB\nTX 出站：{} GiB\n已用：{} GiB\n额度：{} GiB\n",
                traffic.interface,
                crate::traffic::format_gib(traffic.received),
                crate::traffic::format_gib(traffic.transmitted),
                crate::traffic::format_gib(traffic.total()),
                crate::traffic::format_gib(traffic.monthly_traffic_limit)
            ));
            for (label, zone) in [
                ("VPS 刷新", &config.accounting_timezone),
                ("客户端显示", &config.client_display_timezone),
            ] {
                let zone: chrono_tz::Tz = zone.parse().map_err(|_| "时区无效")?;
                text.push_str(&format!(
                    "{label}：{}\n",
                    traffic.next_reset.with_timezone(&zone)
                ));
            }
        }
        Err(_) => text.push_str("流量暂不可用，请检查网卡与账期状态。\n"),
    }
    if include_subscription {
        text.push_str("\n私有订阅链接（包含订阅凭据，请妥善保管）：\n");
        for row in crate::subscription::subscription_matrix() {
            let url = crate::subscription::subscription_url(config, row.format)
                .map_err(|_| "无法生成订阅链接")?;
            text.push_str(&format!("{}：{url}\n", row.format.display_label()));
        }
    }
    Ok(text)
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn message(config: &EmailConfig, body: &str, reminder: bool) -> String {
    let subject = if reminder {
        "sbctl VPS 状态与账期刷新提醒"
    } else {
        "sbctl VPS 状态与流量报告"
    };
    let encoded = STANDARD.encode(body.as_bytes());
    let wrapped = encoded
        .as_bytes()
        .chunks(76)
        .map(|part| std::str::from_utf8(part).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n");
    format!(
        "From: {}\r\nTo: {}\r\nSubject: =?UTF-8?B?{}?=\r\nDate: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
        config.from,
        config.to,
        STANDARD.encode(subject),
        Utc::now().to_rfc2822(),
        wrapped
    )
}

fn deliver(root: &Path, config: &EmailConfig, body: &str, reminder: bool) -> Result<(), String> {
    config.validate()?;
    let mut mail = tempfile::NamedTempFile::new().map_err(|_| "无法创建私有邮件文件")?;
    let mut options = tempfile::NamedTempFile::new().map_err(|_| "无法创建私有 SMTP 文件")?;
    mail.write_all(message(config, body, reminder).as_bytes())
        .map_err(|_| "无法写入邮件")?;
    let scheme = if config.implicit_tls { "smtps" } else { "smtp" };
    // No shell interpolation, no password in process arguments. STARTTLS is
    // mandatory; libcurl verifies the CA chain and hostname for either mode.
    write!(options, "url = {}\nuser = {}\nmail-from = {}\nmail-rcpt = {}\nupload-file = {}\nssl-reqd\nconnect-timeout = 15\nmax-time = 45\nsilent\nshow-error\n",
        quoted(&format!("{scheme}://{}:{}", config.smtp_host, config.smtp_port)),
        quoted(&format!("{}:{}", config.username, config.password)), quoted(&config.from),
        quoted(&config.to), quoted(&mail.path().to_string_lossy())).map_err(|_| "无法写入 SMTP 配置")?;
    options.flush().map_err(|_| "无法写入 SMTP 配置")?;
    let executable = root.join("usr/bin/curl");
    let executable = if executable.is_file() {
        executable
    } else {
        "curl".into()
    };
    let output = std::process::Command::new(executable)
        .arg("--disable")
        .arg("--config")
        .arg(options.path())
        .output()
        .map_err(|_| "无法运行 curl，请安装 curl")?;
    if !output.status.success() {
        // SMTP responses may contain credentials or recipients; never log them.
        return Err(format!(
            "SMTP 发送失败（退出码 {:?}），请检查服务器、TLS、授权码和收件人",
            output.status.code()
        ));
    }
    Ok(())
}

pub fn send(root: &Path, scheduled: bool) -> Result<bool, String> {
    let config = load(root)?;
    let store = DeploymentStore::new(root);
    let _lock = store
        .acquire_operation_lock()
        .map_err(|_| "无法锁定邮件任务")?;
    let deployment = store.load().map_err(|_| "部署尚未初始化")?;
    let state_file = root.join(STATE_PATH);
    let mut state: DeliveryState = match fs::read(state_file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "邮件发送状态损坏")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DeliveryState::default(),
        Err(_) => return Err("无法读取邮件发送状态".into()),
    };
    let now = Utc::now();
    let next_reset = crate::traffic::report(&store, &deployment)
        .ok()
        .map(|r| r.next_reset);
    let (daily, reminder) = due(
        &state,
        &config,
        now,
        next_reset,
        &deployment.client_display_timezone,
    );
    if scheduled && !daily && !reminder {
        return Ok(false);
    }
    let body = report(
        root,
        &deployment,
        config.include_subscription,
        scheduled && reminder,
    )?;
    deliver(root, &config, &body, scheduled && reminder)?;
    if scheduled {
        if daily {
            state.daily = Some(
                now.with_timezone(
                    &deployment
                        .client_display_timezone
                        .parse::<chrono_tz::Tz>()
                        .map_err(|_| "时区无效")?,
                )
                .format("%Y-%m-%d")
                .to_string(),
            );
        }
        if reminder {
            state.reminder = next_reset.map(|reset| reset.to_rfc3339());
        }
        store
            .write_root_only_locked(
                STATE_PATH,
                &serde_json::to_vec(&state).map_err(|_| "无法序列化邮件状态")?,
            )
            .map_err(|_| "邮件已发送，但发送状态保存失败")?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn config() -> EmailConfig {
        EmailConfig {
            password: "secret".into(),
            ..Default::default()
        }
    }
    #[test]
    fn rejects_header_and_curl_injection() {
        for bad in [
            "x@example.com\r\nBcc: leak@example.com",
            "<x@example.com>",
            "x@example.com,y@example.com",
        ] {
            let mut config = config();
            config.to = bad.into();
            assert!(config.validate().is_err());
        }
        let mut config = config();
        config.smtp_host = "smtp.test/\nuser = leak".into();
        assert!(config.validate().is_err());
        assert_eq!(quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
    #[test]
    fn reminders_are_once_per_reset_and_daily_uses_display_timezone() {
        let now = Utc.with_ymd_and_hms(2026, 10, 9, 17, 0, 0).unwrap();
        let reset = now + chrono::Duration::hours(2);
        let mut state = DeliveryState::default();
        assert_eq!(
            due(&state, &config(), now, Some(reset), "Asia/Shanghai"),
            (true, true)
        );
        state.daily = Some("2026-10-10".into());
        state.reminder = Some(reset.to_rfc3339());
        assert_eq!(
            due(&state, &config(), now, Some(reset), "Asia/Shanghai"),
            (false, false)
        );
        assert_eq!(
            due(
                &state,
                &config(),
                now,
                Some(reset + chrono::Duration::days(31)),
                "Asia/Shanghai"
            ),
            (false, false)
        );
    }
    #[test]
    fn unicode_mail_is_encoded_and_private() {
        let mail = message(&config(), "中文\n订阅链接", false);
        assert!(mail.contains("Subject: =?UTF-8?B?"));
        assert!(mail.contains("Content-Transfer-Encoding: base64"));
        assert!(!mail.contains("secret"));
    }
}
