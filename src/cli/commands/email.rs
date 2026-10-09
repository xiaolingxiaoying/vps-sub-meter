use crate::cli::args::EmailCommand;
use std::{path::Path, process::ExitCode};

pub(crate) fn run_email(root: &Path, command: EmailCommand) -> ExitCode {
    let result = (|| -> Result<(), String> {
        let store = sbctl::config::DeploymentStore::new(root);
        match command {
            EmailCommand::Status => {
                let config = sbctl::email::load(root)?;
                println!(
                    "SMTP：{}:{}（{}）\n发件人：{}\n收件人：{}\n日报：{}\n包含订阅：{}\n刷新提前提醒：{} 小时\n定时任务：{}",
                    config.smtp_host,
                    config.smtp_port,
                    if config.implicit_tls {
                        "TLS"
                    } else {
                        "STARTTLS"
                    },
                    config.from,
                    config.to,
                    config.daily_report,
                    config.include_subscription,
                    config.reset_reminder_hours,
                    if root.join("etc/systemd/system/sbctl-email.timer").exists() {
                        "已配置"
                    } else {
                        "关闭"
                    }
                );
            }
            EmailCommand::Configure => {
                sbctl::preflight::require_install_privileges(root).map_err(|e| e.to_string())?;
                store.load().map_err(|_| "请先安装部署，再配置邮件")?;
                let temporary = tempfile::Builder::new()
                    .suffix(".toml")
                    .tempfile()
                    .map_err(|e| e.to_string())?
                    .into_temp_path();
                let contents = std::fs::read_to_string(root.join(sbctl::email::CONFIG_PATH))
                    .unwrap_or(
                        toml::to_string_pretty(&sbctl::email::EmailConfig::default())
                            .map_err(|e| e.to_string())?,
                    );
                std::fs::write(&temporary, contents).map_err(|e| e.to_string())?;
                let status = crate::cli::editor::run_editor(
                    &crate::cli::editor::editor_candidates(),
                    &temporary,
                )?;
                if !status.success() {
                    return Err("编辑器退出失败；未保存配置".into());
                }
                let contents = std::fs::read_to_string(&temporary).map_err(|e| e.to_string())?;
                let config: sbctl::email::EmailConfig =
                    toml::from_str(&contents).map_err(|_| "邮件 TOML 格式无效；未保存")?;
                config.validate()?;
                let _lock = store.acquire_operation_lock().map_err(|e| e.to_string())?;
                store
                    .write_root_only_locked(sbctl::email::CONFIG_PATH, contents.as_bytes())
                    .map_err(|e| e.to_string())?;
                println!(
                    "邮件配置已保存（0600）。运行 email send 测试；email enable 启用定时通知。"
                );
            }
            EmailCommand::Send | EmailCommand::Check => {
                let sent = sbctl::email::send(root, matches!(command, EmailCommand::Check))?;
                if sent {
                    println!("邮件已发送");
                }
            }
            EmailCommand::Enable => {
                sbctl::preflight::require_install_privileges(root).map_err(|e| e.to_string())?;
                store.load().map_err(|e| e.to_string())?;
                sbctl::email::load(root)?;
                sbctl::lifecycle::set_email_timer(root, true)?;
                println!("已启用邮件定时任务（每小时检查；日报和每期提醒自动去重）");
            }
            EmailCommand::Disable => {
                sbctl::preflight::require_install_privileges(root).map_err(|e| e.to_string())?;
                sbctl::lifecycle::set_email_timer(root, false)?;
                println!("邮件定时任务已关闭");
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("邮件操作失败：{error}");
            ExitCode::from(2)
        }
    }
}
