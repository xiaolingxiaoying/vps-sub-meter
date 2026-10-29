//! The interactive terminal menu: the top-level loop, its six sections, and
//! the rendering and confirmation helpers they share. The console prompts used
//! by the non-interactive install path live in `cli::prompt`, not here.

use crate::cli::args::{
    ApiCommand, CertificateCommand, CliOverrideTarget, CliRuleKind, CliSubscriptionMode,
    ConfigCommand, CredentialCommand, EmailCommand, InstallOptions, KernelSource, OverrideCommand,
    RuleCommand, SingBoxCommand,
};
use crate::cli::commands::{
    config::{commit_config_change, regenerate, restart, run_config_wizard},
    install::install,
    serve::{print_subscription_qr, print_subscription_urls},
    status::{
        format_local_time, print_nodes, print_status, print_traffic, run_accounting_reset,
        traffic_set_used,
    },
    system::{rotate_subscription_credential, system_info},
    update::{sing_box, uninstall, update},
};
use crate::cli::prompt::ConsolePrompts;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

mod navigation;
use navigation::{Action, Page};

pub(crate) fn menu(root: &Path) -> ExitCode {
    if !io::stdin().is_terminal() {
        eprintln!("menu requires an interactive terminal");
        return ExitCode::from(2);
    }
    menu_page(root, Page::Home);
    ExitCode::SUCCESS
}

fn menu_page(root: &Path, page: Page) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section(navigation::title(page));
        let config = sbctl::config::DeploymentStore::new(root).load().ok();
        let mut entries = navigation::entries(page, config.is_some());
        if let Some(config) = &config {
            entries.retain(|entry| match entry.action {
                Action::Qr(format) => format.has_compatible_nodes(config),
                _ => true,
            });
        }
        for (i, entry) in entries.iter().enumerate() {
            println!("{}. {}", i + 1, entry.label);
        }
        println!(
            "0. {}",
            if page == Page::Home {
                "退出"
            } else {
                "返回"
            }
        );
        let Some(input) = read_menu_choice("请选择 [0]: ") else {
            return;
        };
        if input.is_empty() || input == "0" {
            return;
        }
        match navigation::select(&entries, &input) {
            Some(Action::Open(next)) => menu_page(root, next),
            Some(action) => {
                execute_action(root, action);
                pause_menu();
            }
            None => {
                eprintln!("无效选择，请按菜单中的编号输入。");
                pause_menu();
            }
        }
    }
}

fn print_menu_header(root: &Path) {
    let (os, kernel, cpu, bbr) = system_info();
    let bar = sbctl::term::blue("-".repeat(70));
    println!("{bar}");
    println!(
        "{}",
        sbctl::term::white("  sbctl  ·  私有 sing-box 订阅控制面")
    );
    println!("{}", sbctl::term::blue("  快捷方式: ly"));
    println!(
        "{}",
        sbctl::term::blue("  项目: github.com/xiaolingxiaoying/vps-sub-meter")
    );
    println!("{bar}");
    println!(
        "{}",
        sbctl::term::green(format!(
            "系统: {os}   内核: {kernel}   处理器: {cpu}   BBR: {bbr}"
        ))
    );
    let store = sbctl::config::DeploymentStore::new(root);
    match store.load() {
        Ok(config) => {
            println!(
                "{}",
                sbctl::term::yellow(format!(
                    "订阅模式: {}   订阅主机: {}   协议: {} 项",
                    config.subscription_mode,
                    config.subscription_host,
                    config.enabled_protocols.len()
                ))
            );
            let template = match config.client_template {
                sbctl::subscription::ClientTemplate::Compact => "精简 (compact)",
                sbctl::subscription::ClientTemplate::Standard => "标准 (standard)",
                sbctl::subscription::ClientTemplate::Global => "旧策略 global",
                sbctl::subscription::ClientTemplate::Split => "旧策略 split",
            };
            println!("客户端模板: {template}");
            if let Some(port) = config.http_port {
                println!("{}", sbctl::term::yellow(format!("公网回退端口: {port}")));
            }
            if let Some(port) = config.subscription_listen_port {
                println!(
                    "{}",
                    sbctl::term::yellow(format!("反向代理回环端口: {port}"))
                );
            }
            println!(
                "{}",
                sbctl::term::green(sbctl::lifecycle::service_status(root))
            );
            match sbctl::traffic::report(&store, &config) {
                Ok(report) => {
                    let total = report.total();
                    let usage = if report.monthly_traffic_limit == 0 {
                        format!("{} / 不限", sbctl::traffic::format_gib(total))
                    } else {
                        let percent = (total as f64 / report.monthly_traffic_limit as f64) * 100.0;
                        format!(
                            "{} / {} ({percent:.1}%)",
                            sbctl::traffic::format_gib(total),
                            sbctl::traffic::format_gib(report.monthly_traffic_limit)
                        )
                    };
                    println!(
                        "{}",
                        sbctl::term::green(format!(
                            "流量: {usage}   出口网卡: {}",
                            report.interface
                        ))
                    );
                    println!(
                        "{}",
                        sbctl::term::blue(format!(
                            "刷新时间: {} ({})   客户端参考: {} ({})",
                            format_local_time(report.next_reset, &config.accounting_timezone),
                            config.accounting_timezone,
                            format_local_time(report.next_reset, &config.client_display_timezone),
                            config.client_display_timezone
                        ))
                    );
                }
                Err(error) => println!(
                    "{}",
                    sbctl::term::yellow(format!("流量: 暂不可用 ({error})"))
                ),
            }
            println!("{}", sbctl::term::green("状态: 已部署"));
        }
        Err(_) => println!(
            "{}",
            sbctl::term::red("状态: 未安装，请进入「部署与协议」安装")
        ),
    }
    println!("{bar}");
}

fn clear_menu_screen() {
    print!("\x1b[2J\x1b[H");
    let _ = io::stdout().flush();
}

fn read_menu_choice(prompt: &str) -> Option<String> {
    print!("{}", sbctl::term::yellow(prompt));
    io::stdout().flush().ok()?;
    let mut choice = String::new();
    if io::stdin().read_line(&mut choice).ok()? == 0 {
        return None;
    }
    Some(choice.trim().to_owned())
}

fn pause_menu() {
    print!("\n{}", sbctl::term::yellow("按回车返回菜单..."));
    let _ = io::stdout().flush();
    let mut ignored = String::new();
    let _ = io::stdin().read_line(&mut ignored);
}

fn print_menu_section(title: &str) {
    println!("\n{}", sbctl::term::blue(format!("=== {title} ===")));
}

fn run_topic_wizard(root: &Path, topic: sbctl::wizard::ConfigurationTopic) {
    let store = sbctl::config::DeploymentStore::new(root);
    let config = match store.load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("读取部署配置失败: {error}");
            return;
        }
    };
    let mut prompts = ConsolePrompts;
    match sbctl::wizard::run_topic(&config, topic, &mut prompts) {
        Ok(sbctl::wizard::WizardOutcome::Cancelled) => {
            println!("配置未应用，现有部署保持不变");
        }
        Ok(sbctl::wizard::WizardOutcome::Unchanged) => {
            println!("配置未发生变化");
        }
        Ok(sbctl::wizard::WizardOutcome::Changed(new)) => {
            let _ = commit_config_change(root, &store, Some(&config), &new, None);
        }
        Err(error) => eprintln!("配置失败: {error}"),
    }
}

fn menu_total_traffic_correction(root: &Path) {
    let store = sbctl::config::DeploymentStore::new(root);
    if let Ok(config) = store.load()
        && let Ok(report) = sbctl::traffic::report(&store, &config)
    {
        println!(
            "当前已用总流量: {}",
            sbctl::traffic::format_gib(report.total())
        );
    }
    let Some(input) = read_menu_choice("目标总流量（GiB，支持小数；输入 0 表示 0）: ")
    else {
        return;
    };
    match sbctl::traffic::parse_traffic_amount(&input) {
        Ok(bytes) => {
            traffic_set_used(root, Some(bytes), None, None);
        }
        Err(error) => eprintln!("流量输入无效: {error}"),
    }
}

fn menu_direction_traffic_correction(root: &Path) {
    let Some(rx) = read_menu_choice("目标 RX 接收流量（GiB）: ") else {
        return;
    };
    let Some(tx) = read_menu_choice("目标 TX 发送流量（GiB）: ") else {
        return;
    };
    match (
        sbctl::traffic::parse_traffic_amount(&rx),
        sbctl::traffic::parse_traffic_amount(&tx),
    ) {
        (Ok(rx), Ok(tx)) => {
            traffic_set_used(root, None, Some(rx), Some(tx));
        }
        (Err(error), _) | (_, Err(error)) => eprintln!("流量输入无效: {error}"),
    }
}

fn menu_install(root: &Path, guided: bool) -> ExitCode {
    let conflicts = sbctl::preflight::existing_deployment_paths(root);
    let replace_existing = if conflicts.is_empty() {
        confirm_menu_action("安装时若有服务占用所需端口，是否停止并禁用这些服务？")
    } else {
        println!("发现旧部署或残留文件：\n  {}", conflicts.join("\n  "));
        if !confirm_menu_action(
            "备份旧部署，停止并禁用冲突服务，然后重新安装？订阅和协议凭据将重建",
        ) {
            return ExitCode::SUCCESS;
        }
        true
    };
    let Some((source, kernel_version)) = select_kernel() else {
        return ExitCode::SUCCESS;
    };
    let manifest_guard = if matches!(source, KernelSource::Repository) {
        let result = (|| -> Result<tempfile::TempPath, Box<dyn std::error::Error>> {
            let manifest = sbctl::update::fetch_latest_manifest()?;
            println!("本仓库固定内核：{}", manifest.sing_box.version);
            let path = tempfile::NamedTempFile::new()?.into_temp_path();
            std::fs::write(&path, serde_json::to_vec(&manifest)?)?;
            Ok(path)
        })();
        match result {
            Ok(path) => Some(path),
            Err(error) => {
                eprintln!("读取本仓库 Release 失败：{error}");
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };
    let existing_config = if replace_existing {
        // Replacement must work even when an old configuration is unreadable.
        Err(sbctl::config::ConfigError::Missing)
    } else {
        sbctl::config::DeploymentStore::new(root).load()
    };
    match existing_config {
        Ok(_) if !replace_existing => {
            eprintln!("已安装。请返回上级菜单选择完整配置向导或重新生成配置工件。");
            ExitCode::from(2)
        }
        Ok(_) | Err(sbctl::config::ConfigError::Missing) => install(
            root,
            InstallOptions {
                mode: CliSubscriptionMode::Direct,
                guided,
                subscription_host: None,
                proxy_host: None,
                http_port: None,
                interface: None,
                reality_decoy_sni: None,
                protocol_sni: None,
                disable_protocol: Vec::new(),
                vless_port: None,
                vmess_port: None,
                hysteria2_port: None,
                tuic_port: None,
                anytls_port: None,
                sing_box_bin: None,
                kernel_version,
                manifest: manifest_guard.as_ref().map(|path| path.to_path_buf()),
                replace_existing,
                manage_firewall: false,
                ipv4_only: false,
                no_start: false,
            },
        ),
        Err(error) => {
            eprintln!("检查部署状态失败: {error}");
            ExitCode::from(2)
        }
    }
}

fn menu_logs(root: &Path) {
    // The journal view is a first-class verb now; the menu just keeps its
    // historical defaults (both units, last 50 lines).
    crate::cli::commands::update::logs(root, crate::cli::args::LogUnit::All, 50, false);
}

pub(crate) fn confirm_menu_action(prompt: &str) -> bool {
    print!("{prompt} [y/N]: ");
    if io::stdout().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn select_kernel() -> Option<(KernelSource, Option<String>)> {
    println!(
        "内核下载源：1. 本仓库 Release（原版二进制，固定版本）  2. 官方仓库（可选版本）  0. 取消"
    );
    match read_menu_choice("请选择 [2]: ")?.as_str() {
        "1" => Some((KernelSource::Repository, None)),
        "2" | "" => {
            let version =
                read_menu_choice("官方版本（例如 1.14.1；latest 表示最新稳定版）[latest]: ")?;
            let version = if version.is_empty() || version == "latest" {
                None
            } else {
                Some(version.trim_start_matches('v').to_owned())
            };
            if let Some(version) = &version
                && let Err(error) = sbctl::update::official_sing_box_archive_url(version)
            {
                eprintln!("版本无效：{error}");
                return None;
            }
            Some((KernelSource::Official, version))
        }
        "0" => None,
        _ => {
            eprintln!("无效选择，请输入 0、1 或 2。");
            None
        }
    }
}

fn execute_action(root: &Path, action: Action) {
    use crate::cli::commands::config::{commit_client_template, run_config};
    use crate::cli::commands::{certificate, email, rule, ruleset, system};
    use Action::*;
    match action {
        Open(page) => menu_page(root, page),
        Status => {
            print_status(root);
        }
        Logs => menu_logs(root),
        KernelStatus => {
            sing_box(root, SingBoxCommand::Status);
        }
        Connections => {
            sing_box(root, SingBoxCommand::Connections);
        }
        CertificateStatus => {
            certificate::run_certificate(root, CertificateCommand::Status);
        }
        QuickInstall => {
            menu_install(root, false);
        }
        GuidedInstall | Reinstall => {
            menu_install(root, true);
        }
        Nodes => {
            print_nodes(root, false, None, false);
        }
        NodeLinks => {
            if confirm_menu_action("确认显示节点分享链接？其中包含 Proxy credential")
            {
                print_nodes(root, true, None, false);
            }
        }
        Topic(topic) => run_topic_wizard(root, topic),
        Template => {
            let store = sbctl::config::DeploymentStore::new(root);
            let config = match store.load() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("读取部署配置失败：{e}");
                    return;
                }
            };
            run_config(
                root,
                ConfigCommand::Override {
                    command: OverrideCommand::Show,
                },
            );
            println!("这些覆写在切换后仍会生效；失效的组或 DNS 引用将拒绝应用。");
            match sbctl::wizard::run_template_selection(&config, &mut ConsolePrompts) {
                Ok(sbctl::wizard::WizardOutcome::Changed(new)) => {
                    commit_client_template(root, new.client_template, None);
                }
                Ok(sbctl::wizard::WizardOutcome::Cancelled) => {
                    println!("模板切换已取消，原配置保留")
                }
                Ok(sbctl::wizard::WizardOutcome::Unchanged) => println!("模板未变化"),
                Err(e) => eprintln!("模板选择失败：{e}"),
            }
        }
        SubscriptionUrls => {
            print_subscription_urls(root, None);
        }
        Qr(format) => {
            print_subscription_qr(root, Some(format), false);
        }
        CredentialList => {
            system::run_credential(root, CredentialCommand::List);
        }
        CredentialAdd => {
            if let Some(name) = required_menu_text("设备凭据名称: ") {
                system::run_credential(root, CredentialCommand::Add { name });
            }
        }
        CredentialRevoke => {
            let Some(name) = required_menu_text("要吊销的凭据名称: ") else {
                return;
            };
            let Some(grace) = read_menu_choice("宽限期（留空立即失效，例如 30m）: ")
            else {
                return;
            };
            let grace = (!grace.is_empty()).then_some(grace);
            if confirm_menu_action(
                "确认吊销该凭据？其链接将在宽限期结束后失效，未设宽限期则立即失效",
            ) {
                system::run_credential(root, CredentialCommand::Revoke { name, grace });
            }
        }
        Rotate => {
            if confirm_menu_action("确认轮换默认订阅凭据？旧默认订阅 URL 将立即失效")
            {
                rotate_subscription_credential(root);
            }
        }
        Traffic => {
            print_traffic(root);
        }
        TotalCorrection => menu_total_traffic_correction(root),
        DirectionCorrection => menu_direction_traffic_correction(root),
        Reset => {
            if confirm_menu_action(
                "确认建立/刷新当前账期？仅在需要时建立新基线，不清空当前账期用量",
            ) {
                run_accounting_reset(root);
            }
        }
        Restart => {
            if confirm_menu_action("确认校验配置并重启服务？") {
                restart(root, None);
            }
        }
        ApiEnable => {
            sing_box(
                root,
                SingBoxCommand::Api {
                    command: ApiCommand::Enable { port: None },
                },
            );
        }
        ApiDisable => {
            sing_box(
                root,
                SingBoxCommand::Api {
                    command: ApiCommand::Disable,
                },
            );
        }
        CertificateObtain => {
            if let Some(email) = required_menu_text("ACME 邮箱: ") {
                certificate::run_certificate(
                    root,
                    CertificateCommand::Obtain {
                        email: Some(email),
                        no_email: false,
                    },
                );
            }
        }
        CertificateRenew => {
            if confirm_menu_action("确认续期 HTTPS 证书？") {
                certificate::run_certificate(root, CertificateCommand::Renew);
            }
        }
        Update => {
            if confirm_menu_action("确认更新 sbctl？将校验签名清单") {
                update(root, false, None, None, None);
            }
        }
        KernelUpdate => {
            if sbctl::config::DeploymentStore::new(root).load().is_err() {
                eprintln!("尚未部署，请先进入「部署与协议」安装");
            } else if let Some((source, version)) = select_kernel()
                && confirm_menu_action("确认更新/切换 sing-box 内核？")
            {
                sing_box(root, SingBoxCommand::Fetch { source, version });
            }
        }
        EditServer | EditSingBox | EditClash => {
            let target = match action {
                EditServer => CliOverrideTarget::Server,
                EditSingBox => CliOverrideTarget::SingBox,
                _ => CliOverrideTarget::Clash,
            };
            run_config(
                root,
                ConfigCommand::Override {
                    command: OverrideCommand::Edit {
                        target,
                        layer: None,
                        sing_box_bin: None,
                    },
                },
            );
        }
        OverrideShow => {
            run_config(
                root,
                ConfigCommand::Override {
                    command: OverrideCommand::Show,
                },
            );
        }
        OverrideValidate => {
            run_config(
                root,
                ConfigCommand::Override {
                    command: OverrideCommand::Validate { sing_box_bin: None },
                },
            );
        }
        Preview | Export => {
            if action == Preview && !confirm_menu_action("确认在终端显示完整配置？其中包含代理凭据")
            {
                return;
            }
            let Some(format) = read_menu_choice("格式 server / sing-box-full / clash [server]: ")
            else {
                return;
            };
            let format = if format.is_empty() {
                "server".into()
            } else {
                format
            };
            let command = if action == Export {
                let Some(output) = required_menu_text("导出到私有新文件路径: ") else {
                    return;
                };
                ConfigCommand::Export {
                    format,
                    output: output.into(),
                }
            } else {
                ConfigCommand::Preview { format }
            };
            run_config(root, command);
        }
        FullWizard => {
            run_config_wizard(root, None);
        }
        Regenerate => {
            if confirm_menu_action("确认重新生成并校验全部配置工件？") {
                regenerate(root, None);
            }
        }
        Uninstall => {
            if confirm_menu_action("确认卸载服务和二进制并保留备份？") {
                uninstall(root, false);
            }
        }
        Purge => {
            if confirm_menu_action("确认彻底清理 sbctl 数据？此操作不可恢复")
                && confirm_menu_action("再次确认：删除配置、状态、证书和备份？")
            {
                uninstall(root, true);
            }
        }
        RuleList => {
            rule::run_rule(root, RuleCommand::List { kind: None }, None);
        }
        RuleAdd | RuleRemove => {
            let Some(kind) = menu_rule_kind(true) else {
                return;
            };
            let Some(entry) = required_menu_text("规则（TYPE,value 或裸域名，一次一条）: ")
            else {
                return;
            };
            let command = if action == RuleAdd {
                RuleCommand::Add {
                    kind,
                    entries: vec![entry],
                }
            } else {
                RuleCommand::Remove {
                    kind,
                    entries: vec![entry],
                }
            };
            rule::run_rule(root, command, None);
        }
        RuleSetList => {
            ruleset::list(root);
        }
        RuleSetAdd => {
            let Some(name) = required_menu_text("规则集名称: ") else {
                return;
            };
            let Some(url) = required_menu_text("编译规则集 HTTPS URL（.srs / .mrs）: ")
            else {
                return;
            };
            let Some(target) = menu_rule_set_target() else {
                return;
            };
            let behavior = if url.ends_with(".mrs") {
                let Some(behavior) = menu_rule_set_behavior() else {
                    return;
                };
                behavior
            } else {
                sbctl::config::RuleSetBehavior::Domain
            };
            ruleset::add(root, &name, &url, target, behavior, None);
        }
        RuleSetRemove => {
            if let Some(name) = required_menu_text("要移除的规则集名称: ") {
                ruleset::remove(root, &name, None);
            }
        }
        RuleImport => {
            let Some(url) = required_menu_text("文本规则列表 HTTPS URL: ") else {
                return;
            };
            let Some(target) = menu_rule_set_target() else {
                return;
            };
            ruleset::import_list(&url, target, root, None);
        }
        RuleInterval => {
            if let Some(value) = required_menu_text("规则集刷新间隔（例如 12h / 1d）: ")
            {
                ruleset::set_interval(root, &value, None);
            }
        }
        EmailConfigure | EmailStatus | EmailSend | EmailEnable | EmailDisable => {
            if action == EmailSend
                && !confirm_menu_action("确认向已配置收件人发送状态/流量/订阅报告？")
            {
                return;
            }
            let command = match action {
                EmailConfigure => EmailCommand::Configure,
                EmailStatus => EmailCommand::Status,
                EmailSend => EmailCommand::Send,
                EmailEnable => EmailCommand::Enable,
                _ => EmailCommand::Disable,
            };
            email::run_email(root, command);
        }
    }
}
fn required_menu_text(label: &str) -> Option<String> {
    read_menu_choice(label).filter(|value| !value.is_empty())
}
fn menu_rule_kind(include_filter: bool) -> Option<CliRuleKind> {
    println!(
        "1. 直连  2. 代理  3. 拒绝{}
0. 取消",
        if include_filter {
            "  4. Fake-IP 例外"
        } else {
            ""
        }
    );
    match read_menu_choice("请选择 [0]: ").as_deref() {
        Some("1") => Some(CliRuleKind::Direct),
        Some("2") => Some(CliRuleKind::Proxy),
        Some("3") => Some(CliRuleKind::Reject),
        Some("4") if include_filter => Some(CliRuleKind::FakeipFilter),
        Some("0" | "") | None => None,
        _ => {
            eprintln!("无效规则类型");
            None
        }
    }
}
fn menu_rule_set_behavior() -> Option<sbctl::config::RuleSetBehavior> {
    use sbctl::config::RuleSetBehavior;
    println!("Mihomo 规则集类型：1) domain（域名/关键字） 2) ipcidr（IP-CIDR）");
    let value = required_menu_text("选择 [1]: ")?;
    match value.trim() {
        "" | "1" => Some(RuleSetBehavior::Domain),
        "2" => Some(RuleSetBehavior::Ipcidr),
        _ => {
            eprintln!("请输入 1 或 2");
            None
        }
    }
}

fn menu_rule_set_target() -> Option<crate::cli::commands::ruleset::RuleSetTarget> {
    use crate::cli::commands::ruleset::RuleSetTarget;
    menu_rule_kind(false).map(|kind| match kind {
        CliRuleKind::Direct => RuleSetTarget::Direct,
        CliRuleKind::Proxy => RuleSetTarget::Proxy,
        _ => RuleSetTarget::Reject,
    })
}
