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
    system::rotate_subscription_credential,
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
        let config = sbctl::config::DeploymentStore::new(root).load();
        // An unreadable existing deployment needs repair, not fresh-install
        // choices. Only an actually missing configuration offers installation.
        let needs_install = matches!(config, Err(sbctl::config::ConfigError::Missing));
        let config = config.ok();
        let mut entries = navigation::entries(page, !needs_install);
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
            Some(Action::NodeManage) => managed_nodes_menu(root),
            Some(Action::GroupManage) => managed_groups_menu(root),
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
    let bar = sbctl::term::blue("-".repeat(70));
    println!("{bar}");
    println!("{}", sbctl::term::white("  sbctl  ·  节点与双内核订阅管理"));
    println!("{}", sbctl::term::blue("  快捷方式: ly"));
    println!(
        "{}",
        sbctl::term::blue("  项目: github.com/xiaolingxiaoying/vps-sub-meter")
    );
    println!("{bar}");
    match sbctl::nodes::read_optional(root) {
        Ok(Some(registry)) => println!(
            "运行模式: {}   启用节点: {}   节点组: {}",
            registry.mode.label(),
            registry.nodes.iter().filter(|n| n.enabled).count(),
            registry.groups.len()
        ),
        Ok(None) => println!("节点仓库: 待迁移（节点管理中的修改会自动迁移）"),
        Err(_) => println!("节点仓库: 需要修复，请检查权限或恢复事务"),
    }
    if root
        .join("var/lib/sbctl/deployment-transaction.json")
        .exists()
    {
        println!("状态: 存在未完成事务，请进入「内核与运行模式 → 恢复未完成事务」");
    }
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
        Err(sbctl::config::ConfigError::Missing) => println!(
            "{}",
            sbctl::term::red("状态: 未安装，请进入「部署与协议」安装")
        ),
        Err(_) => println!(
            "{}",
            sbctl::term::red(
                "状态: 配置不可用，请检查配置文件或访问权限；确认修复方案后再选择「高级与维护 → 备份旧部署并重新安装」"
            )
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
    let Some((_source, kernel_version)) = select_kernel() else {
        return ExitCode::SUCCESS;
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
                manifest: None,
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
    sbctl::wizard::Prompts::confirm(&mut ConsolePrompts, prompt, false).unwrap_or(false)
}

fn select_kernel() -> Option<(KernelSource, Option<String>)> {
    println!("sing-box 仅从 SagerNet 官方仓库下载：\n1. 最新稳定版（推荐）\n2. 指定版本\n0. 取消");
    match read_menu_choice("请选择 [1]: ")?.as_str() {
        "1" | "" => Some((KernelSource::Official, None)),
        "2" => {
            let version = read_menu_choice("版本号（例如 1.14.4）: ")?;
            let version = version.trim_start_matches('v').to_owned();
            if let Err(error) = sbctl::update::official_sing_box_archive_url(&version) {
                eprintln!("版本无效：{error}");
                return None;
            }
            Some((KernelSource::Official, Some(version)))
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
            let (os, kernel, cpu, bbr) = crate::cli::commands::system::system_info();
            println!("系统: {os}  内核: {kernel}  CPU: {cpu}  BBR: {bbr}");
            print_status(root);
        }
        Logs => menu_logs(root),
        KernelStatus => {
            crate::cli::commands::managed::core(root, crate::cli::args::CoreCommand::Status);
        }
        Connections => {
            sing_box(root, SingBoxCommand::Connections);
        }
        CertificateStatus => {
            certificate::run_certificate(root, CertificateCommand::Status);
        }
        SubscriptionDiagnosis => menu_subscription_diagnosis(root),
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
        NodeManage => managed_nodes_menu(root),
        GroupManage => managed_groups_menu(root),
        ManagedLinks => managed_links_menu(root),
        CoreMode => managed_mode_menu(root),
        CoreInstall => {
            println!("1. sing-box\n2. Xray\n0. 返回");
            let core = match read_menu_choice("选择内核: ").as_deref() {
                Some("1") => sbctl::core::CoreKind::SingBox,
                Some("2") => sbctl::core::CoreKind::Xray,
                _ => return,
            };
            let Some(version) = read_menu_choice("固定版本（回车使用项目推荐版本）: ")
            else {
                return;
            };
            if confirm_menu_action("确认下载并校验官方内核？已运行的该内核将独立重启")
            {
                crate::cli::commands::managed::core(
                    root,
                    crate::cli::args::CoreCommand::Install {
                        core,
                        version: (!version.is_empty()).then_some(version),
                    },
                );
            }
        }
        CoreApply => managed_edit(root, |_, _| Ok(true)),
        CoreRecover => {
            crate::cli::commands::managed::core(
                root,
                crate::cli::args::CoreCommand::Recover { boot: false },
            );
        }
        CoreRollback => {
            if confirm_menu_action("确认恢复最近一次部署？订阅凭据和流量账期不会回退")
            {
                crate::cli::commands::managed::core(root, crate::cli::args::CoreCommand::Rollback);
            }
        }
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
            let behavior = if url::Url::parse(&url).is_ok_and(|url| url.path().ends_with(".mrs")) {
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

fn menu_subscription_diagnosis(root: &Path) {
    let diagnosis = sbctl::system::diagnose_subscription(root);
    println!("订阅入口诊断（只读，不修改网络或服务配置）");
    if !diagnosis.configuration_loaded {
        println!("部署配置不可用，请先检查配置文件或访问权限。");
        return;
    }
    println!(
        "模式: {}   主机: {}   公网端口: {}",
        diagnosis.mode.as_deref().unwrap_or("未知"),
        diagnosis.host.as_deref().unwrap_or("未知"),
        diagnosis
            .public_port
            .map_or_else(|| "未知".into(), |port| port.to_string()),
    );
    for unit in std::iter::once(&diagnosis.service).chain(diagnosis.socket.as_ref()) {
        println!(
            "{}: {} / {}{}",
            unit.unit,
            unit.active_state.as_deref().unwrap_or("未知"),
            unit.sub_state.as_deref().unwrap_or("未知"),
            if unit.error.is_some() {
                "（无法查询）"
            } else {
                ""
            },
        );
    }
    if let Some(dns) = &diagnosis.dns {
        println!(
            "DNS IPv4: {}",
            if dns.ipv4.is_empty() {
                "无".into()
            } else {
                dns.ipv4.join(", ")
            }
        );
        println!(
            "DNS IPv6: {}",
            if dns.ipv6.is_empty() {
                "无".into()
            } else {
                dns.ipv6.join(", ")
            }
        );
        if dns.error.is_some() {
            println!("DNS 查询失败，请检查域名解析。");
        }
    }
    for (label, probe) in [
        ("订阅监听", &diagnosis.listener),
        ("本地 HTTPS", &diagnosis.local_tls),
    ] {
        let status = diagnosis_probe_label(probe);
        println!("{label}: {status}");
        if let Some(target) = &probe.target {
            println!("  目标: {target}   耗时: {} ms", probe.elapsed_ms);
        }
        if let Some(status) = probe.http_status {
            println!("  HTTP: {status}（探测不携带订阅凭据，根路径返回 404 属正常情况）");
        }
    }
    if let Some(certificate) = &diagnosis.certificate {
        println!(
            "证书: {}",
            if certificate.state == "ok" {
                "有效"
            } else {
                "异常，请查看证书状态"
            }
        );
    }
    println!("完整诊断: sbctl system diagnose-subscription --json");
}

fn diagnosis_probe_label(probe: &sbctl::system::ProbeDiagnosis) -> &'static str {
    match probe.status {
        "not_run" => "已跳过（隔离目录或尚无监听配置）",
        "not_applicable" => "当前模式不适用",
        "address_invalid" => "监听地址无效，无法探测",
        "curl_unavailable" => "缺少 curl，无法探测 HTTPS",
        "tcp_connected" => "TCP 已连接（尚未验证 HTTP）",
        "http_response" | "tls_verified_http_response"
            if probe.http_status.is_some_and(|status| status >= 500) =>
        {
            "已连接，但服务返回 HTTP 错误"
        }
        "http_response" => "已收到 HTTP 响应",
        "tls_verified_http_response" => "TLS 证书校验通过，已收到 HTTP 响应",
        "tls_verified_http_error" => "TLS 证书校验通过，但 HTTP 传输失败",
        "certificate_validation_failed" => "证书校验失败",
        "tcp_connect_failed" | "tcp_connect_timeout" => "连接失败，请检查服务与监听端口",
        "tls_handshake_failed" | "tls_handshake_timeout" => "TLS 握手失败",
        "http_response_timeout" | "tcp_connected_no_http_response" => "已连接，但未收到 HTTP 响应",
        "probe_failed" => "探测失败，请检查本地端点",
        _ => "未知探测状态，请运行 JSON 诊断查看详情",
    }
}

fn required_menu_text(label: &str) -> Option<String> {
    read_menu_choice(label).filter(|value| !value.is_empty())
}

fn managed_edit(
    root: &Path,
    edit: impl FnOnce(
        &sbctl::config::DeploymentConfig,
        &mut sbctl::nodes::Registry,
    ) -> sbctl::nodes::Result<bool>,
) {
    let result = (|| -> sbctl::nodes::Result<()> {
        let (config, before) = crate::cli::commands::managed::draft(root)?;
        let mut after = before.clone();
        if !edit(&config, &mut after)? {
            println!("已取消，草稿未保存");
            return Ok(());
        }
        let plan = sbctl::deployment::plan(root, &config, &before, &after)?;
        if !plan.requirements.is_empty() {
            return Err(sbctl::nodes::Error(plan.requirements.join("；")));
        }
        println!(
            "变更预览：\n{}",
            serde_json::to_string_pretty(&plan).unwrap()
        );
        for n in &after.nodes {
            if before.nodes.iter().find(|old| old.id == n.id) != Some(n) {
                println!(
                    "{} · {} · {} · {}:{} · {}（凭据隐藏）",
                    n.id,
                    n.name,
                    n.core,
                    n.listen,
                    n.port(),
                    if n.enabled { "启用" } else { "停用" }
                );
            }
        }
        if confirm_menu_action("应用以上变更？内核配置变化会中断受影响的连接")
        {
            sbctl::deployment::apply(
                &sbctl::config::DeploymentStore::new(root),
                &config,
                &before,
                after,
            )?;
            println!("已应用并发布订阅");
        } else {
            println!("已取消，草稿未保存");
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("操作失败：{error}");
    }
}
fn select_managed_node(registry: &sbctl::nodes::Registry) -> Option<String> {
    let mut page = 0usize;
    loop {
        println!(
            "选择节点，第 {} 页（n 下一页 / p 上一页 / 0 返回，也可输入完整 ID）",
            page + 1
        );
        for (i, n) in registry.nodes.iter().enumerate().skip(page * 20).take(20) {
            println!(
                "{}. {} · {} · {}:{}",
                i + 1,
                n.name,
                n.core,
                n.listen,
                n.port()
            );
        }
        let value = read_menu_choice("节点编号或 ID: ")?;
        match value.as_str() {
            "" | "0" => return None,
            "n" => {
                if (page + 1) * 20 < registry.nodes.len() {
                    page += 1;
                }
            }
            "p" => page = page.saturating_sub(1),
            _ => {
                if let Some(n) = value
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| registry.nodes.get(i))
                {
                    return Some(n.id.clone());
                }
                if registry.nodes.iter().any(|n| n.id == value) {
                    return Some(value);
                }
                println!("节点不存在，请重新输入");
            }
        }
    }
}
fn menu_port(label: &str) -> Option<Option<u16>> {
    loop {
        let value = read_menu_choice(label)?;
        if value.is_empty() {
            return Some(None);
        }
        if let Ok(port) = value.parse::<u16>()
            && port >= 10000
        {
            return Some(Some(port));
        }
        println!("请输入 10000–65535，或回车保留 / 自动分配");
    }
}
fn managed_nodes_menu(root: &Path) {
    loop {
        println!(
            "\n节点与组合\n1. 筛选 / 分页查看节点\n2. 新增节点\n3. 编辑节点\n4. 复制节点\n5. 启用节点\n6. 停用节点\n7. 删除节点\n8. 迁移节点内核\n9. 校验节点与配置\n10. 分享链接与二维码\n11. 迁移现有部署到统一仓库\n12. 真实连接测试\n0. 返回"
        );
        let Some(choice) = read_menu_choice("选择: ") else {
            return;
        };
        if choice.is_empty() || choice == "0" {
            return;
        }
        if choice == "1" {
            managed_list_menu(root);
            continue;
        }
        if choice == "9" {
            crate::cli::commands::managed::node(
                root,
                crate::cli::args::NodeCommand::Check { id: None },
            );
            pause_menu();
            continue;
        }
        if choice == "12" {
            if let Ok((_, registry)) = crate::cli::commands::managed::draft(root)
                && let Some(id) = select_managed_node(&registry)
            {
                let Some(url) = read_menu_choice("HTTP(S) 测试 URL（回车使用已配置探测地址）: ")
                else {
                    return;
                };
                crate::cli::commands::managed::node(
                    root,
                    crate::cli::args::NodeCommand::Probe {
                        id,
                        url: (!url.is_empty()).then_some(url),
                        client_bin: None,
                    },
                );
            }
            pause_menu();
            continue;
        }
        if choice == "10" {
            if let Ok((_, r)) = crate::cli::commands::managed::draft(root)
                && let Some(id) = select_managed_node(&r)
                && confirm_menu_action("显示该节点分享链接？链接包含节点凭据")
            {
                crate::cli::commands::managed::node(
                    root,
                    crate::cli::args::NodeCommand::Export { id, qr: true },
                );
            }
            pause_menu();
            continue;
        }
        managed_edit(root, |config, registry| {
            if choice == "11" {
                return Ok(true);
            }
            if choice == "2" {
                let presets = [
                    "vless-reality",
                    "vmess-websocket",
                    "hysteria2",
                    "tuic",
                    "anytls",
                    "vless-xhttp-reality",
                    "vless-xhttp-tls",
                    "vless-ws-tls",
                    "trojan-tcp-tls",
                ];
                for (i, preset) in presets.iter().enumerate() {
                    println!("{}. {preset}", i + 1);
                }
                let preset = loop {
                    let Some(v) = read_menu_choice("预设编号（0 返回）: ") else {
                        return Ok(false);
                    };
                    if v == "0" {
                        return Ok(false);
                    }
                    if let Some(preset) = v
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| i.checked_sub(1))
                        .and_then(|i| presets.get(i))
                    {
                        break *preset;
                    }
                    println!("无效预设，请重试");
                };
                let Some(name) = required_menu_text("节点名称") else {
                    return Ok(false);
                };
                let Some(port) = menu_port("端口（回车自动分配）: ") else {
                    return Ok(false);
                };
                let node = sbctl::nodes::create(registry, config, preset, &name, port, None)?;
                registry.nodes.push(node);
                return Ok(true);
            }
            let Some(id) = select_managed_node(registry) else {
                return Ok(false);
            };
            match choice.as_str() {
                "3" => {
                    let node = registry.node_mut(&id)?;
                    let Some(name) = read_menu_choice("名称（回车保留）: ") else {
                        return Ok(false);
                    };
                    if !name.is_empty() {
                        node.name = name;
                    }
                    let Some(port) = menu_port("监听端口（回车保留）: ") else {
                        return Ok(false);
                    };
                    if let Some(port) = port {
                        node.patch("port", serde_json::json!(port))?;
                    }
                    for (key, label) in [
                        ("server", "客户端连接主机（回车保留）: "),
                        ("server_name", "TLS SNI（回车保留）: "),
                    ] {
                        let Some(value) = read_menu_choice(label) else {
                            return Ok(false);
                        };
                        if !value.is_empty() {
                            node.patch(key, serde_json::json!(value))?;
                        }
                    }
                }
                "4" => {
                    let mut node = registry.node_mut(&id)?.clone();
                    let Some(name) = required_menu_text("新节点名称") else {
                        return Ok(false);
                    };
                    let Some(port) = menu_port("新端口（回车自动分配）: ") else {
                        return Ok(false);
                    };
                    node.id = sbctl::nodes::new_id();
                    node.tag = format!("node-{}", node.id);
                    node.name = name;
                    node.renew_secrets()?;
                    node.patch(
                        "port",
                        serde_json::json!(
                            port.map(Ok)
                                .unwrap_or_else(|| sbctl::nodes::free_port(registry, config))?
                        ),
                    )?;
                    registry.nodes.push(node);
                }
                "5" => registry.node_mut(&id)?.enabled = true,
                "6" => registry.node_mut(&id)?.enabled = false,
                "7" => {
                    if registry.groups.iter().any(|g| g.members.contains(&id)) {
                        return Err(sbctl::nodes::Error(
                            "节点仍被分组引用，请先修改组成员".into(),
                        ));
                    }
                    registry.nodes.retain(|n| n.id != id);
                }
                "8" => {
                    println!("1. sing-box\n2. Xray\n0. 返回");
                    let core = match read_menu_choice("目标内核: ").as_deref() {
                        Some("1") => sbctl::core::CoreKind::SingBox,
                        Some("2") => sbctl::core::CoreKind::Xray,
                        _ => return Ok(false),
                    };
                    registry.node_mut(&id)?.core = core;
                }
                _ => return Ok(false),
            }
            Ok(true)
        });
        pause_menu();
    }
}
fn managed_list_menu(root: &Path) {
    let mut core = None;
    let mut preset = None;
    let mut group = None;
    let mut enabled = None;
    println!("过滤条件：0 全部 / 1 内核 / 2 预设 / 3 分组 ID / 4 启用状态");
    match read_menu_choice("过滤 [0]: ").as_deref() {
        Some("1") => {
            core = match read_menu_choice("1 sing-box / 2 Xray: ").as_deref() {
                Some("1") => Some(sbctl::core::CoreKind::SingBox),
                Some("2") => Some(sbctl::core::CoreKind::Xray),
                _ => return,
            };
        }
        Some("2") => {
            preset = required_menu_text("预设名称");
            if preset.is_none() {
                return;
            }
        }
        Some("3") => {
            group = required_menu_text("节点组 ID");
            if group.is_none() {
                return;
            }
        }
        Some("4") => {
            enabled = match read_menu_choice("1 启用 / 2 停用: ").as_deref() {
                Some("1") => Some(true),
                Some("2") => Some(false),
                _ => return,
            };
        }
        None => return,
        _ => {}
    }
    let mut page = 1usize;
    loop {
        crate::cli::commands::managed::node(
            root,
            crate::cli::args::NodeCommand::List {
                core,
                preset: preset.clone(),
                group: group.clone(),
                enabled,
                json: false,
                page,
                per_page: 20,
            },
        );
        match read_menu_choice("n 下一页 / p 上一页 / 0 返回: ").as_deref() {
            Some("n") => page = page.saturating_add(1),
            Some("p") => page = page.saturating_sub(1).max(1),
            _ => return,
        }
    }
}
fn select_managed_group(registry: &sbctl::nodes::Registry) -> Option<String> {
    if registry.groups.is_empty() {
        println!("还没有节点组，请先创建；全量订阅仍可使用");
        return None;
    }
    for (i, g) in registry.groups.iter().enumerate() {
        println!("{}. {} · {} 个节点", i + 1, g.name, g.members.len());
    }
    loop {
        let value = read_menu_choice("组编号或 ID（0 返回）: ")?;
        if value.is_empty() || value == "0" {
            return None;
        }
        if let Some(g) = value
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| registry.groups.get(i))
        {
            return Some(g.id.clone());
        }
        if registry.groups.iter().any(|g| g.id == value) {
            return Some(value);
        }
        println!("分组不存在，请重试");
    }
}
fn managed_groups_menu(root: &Path) {
    loop {
        println!(
            "\n节点组\n1. 查看节点组\n2. 新增节点组\n3. 编辑名称、成员与策略\n4. 删除节点组\n0. 返回"
        );
        let Some(choice) = read_menu_choice("选择: ") else {
            return;
        };
        if choice.is_empty() || choice == "0" {
            return;
        }
        if choice == "1" {
            crate::cli::commands::managed::group(root, crate::cli::args::GroupCommand::List);
            pause_menu();
            continue;
        }
        managed_edit(root, |_, registry| {
            let existing = if choice == "2" {
                None
            } else {
                let Some(id) = select_managed_group(registry) else {
                    return Ok(false);
                };
                Some(id)
            };
            if choice == "4" {
                registry.groups.retain(|g| Some(&g.id) != existing.as_ref());
                return Ok(true);
            }
            if !["2", "3"].contains(&choice.as_str()) {
                return Ok(false);
            }
            let Some(name) = read_menu_choice("组名称（编辑时回车保留）: ") else {
                return Ok(false);
            };
            println!("可用节点（成员按输入顺序保存）:");
            for (i, n) in registry.nodes.iter().enumerate() {
                println!("{}. {}  {}", i + 1, n.name, n.id);
            }
            let members = loop {
                let Some(value) =
                    read_menu_choice("节点编号或 ID，逗号分隔（编辑时回车保留，- 清空）: ")
                else {
                    return Ok(false);
                };
                if value.is_empty() && existing.is_some() {
                    break None;
                }
                if value == "-" || value.is_empty() {
                    break Some(Vec::new());
                }
                let mut ids = Vec::new();
                let mut valid = true;
                for input in value.split(',').map(str::trim) {
                    let node = input
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| i.checked_sub(1))
                        .and_then(|i| registry.nodes.get(i))
                        .or_else(|| registry.nodes.iter().find(|n| n.id == input));
                    if let Some(node) = node
                        && !ids.contains(&node.id)
                    {
                        ids.push(node.id.clone());
                    } else {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    break Some(ids);
                }
                println!("成员不存在或重复，请在当前步骤重新输入");
            };
            let strategy = match read_menu_choice("策略：1 手动选择 / 2 自动测速（回车保留）: ")
                .as_deref()
            {
                Some("1") => Some(sbctl::nodes::Strategy::Select),
                Some("2") => Some(sbctl::nodes::Strategy::Urltest),
                Some("") => None,
                _ => return Ok(false),
            };
            if let Some(id) = existing {
                let g = registry.groups.iter_mut().find(|g| g.id == id).unwrap();
                if !name.is_empty() {
                    g.name = name;
                }
                if let Some(members) = members {
                    g.members = members;
                }
                if let Some(strategy) = strategy {
                    g.strategy = strategy;
                }
            } else {
                registry.groups.push(sbctl::nodes::Group {
                    id: sbctl::nodes::new_id(),
                    name,
                    members: members.unwrap_or_default(),
                    strategy: strategy.unwrap_or_default(),
                });
            }
            Ok(true)
        });
        pause_menu();
    }
}
fn managed_mode_menu(root: &Path) {
    managed_edit(root, |_, registry| {
        println!("1. sing-box\n2. Xray\n3. 双内核\n0. 返回");
        let mode = match read_menu_choice("运行模式: ").as_deref() {
            Some("1") => sbctl::nodes::Mode::SingBox,
            Some("2") => sbctl::nodes::Mode::Xray,
            Some("3") => sbctl::nodes::Mode::Dual,
            _ => return Ok(false),
        };
        for n in registry
            .nodes
            .iter()
            .filter(|n| n.enabled && !mode.allows(n.core))
        {
            println!("受影响：{} · {} · {}", n.id, n.name, n.preset());
        }
        let Some(disable) = read_menu_choice("需停用的节点 ID，逗号分隔（回车仅迁移兼容节点）: ")
        else {
            return Ok(false);
        };
        registry.switch(
            mode,
            &disable
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_owned())
                .collect::<Vec<_>>(),
        )?;
        Ok(true)
    });
}
fn managed_links_menu(root: &Path) {
    let result = (|| -> sbctl::nodes::Result<()> {
        let (config, registry) = crate::cli::commands::managed::draft(root)?;
        println!("1. sing-box\n2. Mihomo / Clash\n3. Shadowrocket\n4. V2rayN / 通用 URI\n0. 返回");
        let format = match read_menu_choice("客户端: ").as_deref() {
            Some("1") => {
                println!("0. 已固定的完整配置");
                let profiles = sbctl::subscription::all_sing_box_version_profiles();
                for (i, p) in profiles.iter().enumerate() {
                    println!("{}. sing-box {}", i + 1, p.version);
                }
                let Some(v) = read_menu_choice("目标版本 [0]: ") else {
                    return Ok(());
                };
                if v.is_empty() || v == "0" {
                    sbctl::subscription::SubscriptionFormat::SingBoxFull
                } else {
                    let p = v
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| i.checked_sub(1))
                        .and_then(|i| profiles.get(i))
                        .ok_or_else(|| sbctl::nodes::Error("版本编号无效".into()))?;
                    sbctl::subscription::SubscriptionFormat::SingBoxVersion(p.version)
                }
            }
            Some("2") => sbctl::subscription::SubscriptionFormat::Clash,
            Some("3") => sbctl::subscription::SubscriptionFormat::Shadowrocket,
            Some("4") => sbctl::subscription::SubscriptionFormat::Base64Uri,
            _ => return Ok(()),
        };
        let group = match read_menu_choice("范围：0 全量 / 1 选择节点组 [0]: ").as_deref()
        {
            Some("1") => {
                let Some(id) = select_managed_group(&registry) else {
                    return Ok(());
                };
                Some(id)
            }
            Some("0" | "") => None,
            _ => return Ok(()),
        };
        println!("0. 默认订阅凭据");
        let credentials: Vec<_> = config
            .subscription_credentials
            .iter()
            .filter(|c| c.is_active(chrono::Utc::now().timestamp()))
            .collect();
        for (i, c) in credentials.iter().enumerate() {
            println!("{}. {}", i + 1, c.name);
        }
        let Some(v) = read_menu_choice("设备凭据 [0]: ") else {
            return Ok(());
        };
        let name = if v.is_empty() || v == "0" {
            None
        } else {
            Some(
                credentials
                    .get(
                        v.parse::<usize>()
                            .ok()
                            .and_then(|i| i.checked_sub(1))
                            .ok_or_else(|| sbctl::nodes::Error("凭据编号无效".into()))?,
                    )
                    .ok_or_else(|| sbctl::nodes::Error("凭据编号无效".into()))?
                    .name
                    .as_str(),
            )
        };
        if confirm_menu_action("显示订阅链接和二维码？其中包含订阅凭据") {
            if root.join(sbctl::deployment::CURRENT).exists() {
                crate::cli::commands::managed::links(
                    root,
                    &config,
                    group.as_deref(),
                    Some(format),
                    name,
                    true,
                )?;
            } else {
                println!("现有部署尚未迁移，显示原全量订阅：");
                print_subscription_urls(root, Some(format));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("订阅获取失败：{error}");
    }
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
    match read_menu_choice("选择 [1]: ").as_deref() {
        Some("" | "1") => Some(RuleSetBehavior::Domain),
        Some("2") => Some(RuleSetBehavior::Ipcidr),
        Some(_) => {
            eprintln!("请输入 1 或 2");
            None
        }
        None => None,
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

#[cfg(test)]
mod diagnosis_tests {
    use super::diagnosis_probe_label;
    use sbctl::system::ProbeDiagnosis;

    #[test]
    fn diagnostic_labels_distinguish_transport_success_from_http_failure() {
        let mut probe = ProbeDiagnosis {
            status: "tls_verified_http_response",
            target: None,
            tcp_connected: true,
            tls_handshake: Some(true),
            trust_validation: Some(true),
            http_status: Some(404),
            elapsed_ms: 0,
        };
        assert!(diagnosis_probe_label(&probe).contains("已收到 HTTP 响应"));
        probe.http_status = Some(503);
        assert!(diagnosis_probe_label(&probe).contains("HTTP 错误"));
        probe.status = "tls_verified_http_error";
        assert!(diagnosis_probe_label(&probe).contains("HTTP 传输失败"));
        probe.status = "curl_unavailable";
        assert!(diagnosis_probe_label(&probe).contains("缺少 curl"));
        probe.status = "address_invalid";
        assert!(diagnosis_probe_label(&probe).contains("地址无效"));
    }
}
