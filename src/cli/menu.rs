//! The interactive terminal menu: the top-level loop, its six sections, and
//! the rendering and confirmation helpers they share. The console prompts used
//! by the non-interactive install path live in `cli::prompt`, not here.

use crate::cli::args::{
    ApiCommand, CertificateCommand, CliOverrideTarget, CliSubscriptionMode, ConfigCommand,
    EmailCommand, InstallOptions, KernelSource, OverrideCommand, SingBoxCommand,
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

pub(crate) fn menu(root: &Path) -> ExitCode {
    if !io::stdin().is_terminal() {
        eprintln!("menu requires an interactive terminal");
        return ExitCode::from(2);
    }

    loop {
        clear_menu_screen();
        print_menu_header(root);
        println!();
        println!("{}", sbctl::term::green(" 1. 安装与部署"));
        println!("{}", sbctl::term::green(" 2. 节点与协议"));
        println!("{}", sbctl::term::green(" 3. 订阅中心"));
        println!("{}", sbctl::term::green(" 4. 流量与账期"));
        println!("{}", sbctl::term::green(" 5. 服务与诊断"));
        println!("{}", sbctl::term::green(" 6. 更新与卸载"));
        println!("{}", sbctl::term::green(" 7. 配置与路由规则"));
        println!("{}", sbctl::term::green(" 8. 邮件通知"));
        println!("{}", sbctl::term::green(" 0. 退出"));
        println!();
        match read_menu_choice("请选择 [0]: ") {
            Some(choice) => match choice.as_str() {
                "0" => return ExitCode::SUCCESS,
                "1" => menu_deployment(root),
                "2" => menu_protocols(root),
                "3" => menu_subscriptions(root),
                "4" => menu_traffic(root),
                "5" => menu_services(root),
                "6" => menu_updates(root),
                "7" => menu_configuration(root),
                "8" => menu_email(root),
                _ => {
                    eprintln!("无效选择，请输入 0 到 8。");
                    pause_menu();
                }
            },
            None => return ExitCode::from(2),
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
        Err(_) => println!("{}", sbctl::term::red("状态: 未安装，请选择 1 安装")),
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

fn menu_deployment(root: &Path) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section("安装与部署");
        let installed = sbctl::config::DeploymentStore::new(root).load().is_ok();
        if installed {
            println!("1. 重新生成并校验现有配置工件");
            println!("2. 完整配置向导");
        } else {
            println!("1. 快速安装（默认配置）");
            println!("2. 引导式安装（完整配置向导）");
        }
        println!("0. 返回");
        match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") if installed => {
                regenerate(root, None);
            }
            Some("1") => {
                menu_install(root, false);
            }
            Some("2") if !installed => {
                menu_install(root, true);
            }
            Some("2") => {
                run_config_wizard(root, None);
            }
            Some(_) => eprintln!("无效选择，请输入 0 到 2。"),
        }
        pause_menu();
    }
}

fn menu_protocols(root: &Path) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section("节点与协议");
        println!("1. 查看节点与监听端口");
        println!("2. 配置协议启停、端口、SNI 和证书");
        println!("3. 查看节点分享链接（含节点凭据）");
        println!("0. 返回");
        match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") => {
                print_nodes(root, false, None, false);
                pause_menu();
            }
            Some("2") => {
                run_topic_wizard(root, sbctl::wizard::ConfigurationTopic::Protocols);
                pause_menu();
            }
            Some("3") => {
                if confirm_menu_action(
                    "确认显示节点分享链接？链接包含 Proxy credential，会显示在当前终端",
                ) {
                    print_nodes(root, true, None, false);
                }
                pause_menu();
            }
            Some(_) => {
                eprintln!("无效选择，请按菜单中的编号输入。");
                pause_menu();
            }
        }
    }
}

fn menu_subscriptions(root: &Path) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section("订阅中心");
        println!("1. 查看全部订阅链接与二维码");
        println!("2. 显示 sing-box 完整配置二维码");
        println!("3. 显示 sing-box 精简配置二维码");
        println!("4. 显示 Clash/Mihomo 二维码");
        println!("5. 显示 Clash 1.18 兼容二维码");
        println!("6. 显示 URI 二维码");
        println!("7. 显示 Base64 URI 二维码（V2rayN）");
        println!("8. 显示 Shadowrocket 二维码");
        println!("9. 配置订阅入口");
        println!("10. 轮换订阅凭据（旧链接立即失效）");
        println!("11. 重新生成订阅工件");
        println!("12. 客户端模板配置（DNS / 分流规则档位 / 延迟探测）");
        println!("13. 命名订阅凭据（按设备发放 / 吊销 / 查看，泄露时只断一台）");
        println!("0. 返回");
        match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") => {
                print_subscription_urls(root, None);
                pause_menu();
            }
            Some("2") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::SingBoxFull),
                    false,
                );
                pause_menu();
            }
            Some("3") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::SingBox),
                    false,
                );
                pause_menu();
            }
            Some("4") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::Clash),
                    false,
                );
                pause_menu();
            }
            Some("5") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::ClashLegacy(
                        sbctl::subscription::CLASH_LEGACY_VERSION,
                    )),
                    false,
                );
                pause_menu();
            }
            Some("6") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::Uri),
                    false,
                );
                pause_menu();
            }
            Some("7") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::Base64Uri),
                    false,
                );
                pause_menu();
            }
            Some("8") => {
                print_subscription_qr(
                    root,
                    Some(sbctl::subscription::SubscriptionFormat::Shadowrocket),
                    false,
                );
                pause_menu();
            }
            Some("9") => {
                run_topic_wizard(root, sbctl::wizard::ConfigurationTopic::Subscription);
                pause_menu();
            }
            Some("10") => {
                if confirm_menu_action("确认轮换订阅凭据？旧订阅 URL 将立即失效") {
                    rotate_subscription_credential(root);
                }
                pause_menu();
            }
            Some("11") => {
                if confirm_menu_action("确认重新生成并校验订阅工件？") {
                    regenerate(root, None);
                }
                pause_menu();
            }
            Some("12") => {
                run_topic_wizard(root, sbctl::wizard::ConfigurationTopic::ClientTemplate);
                pause_menu();
            }
            Some("13") => {
                menu_named_credentials(root);
                pause_menu();
            }
            Some(_) => {
                eprintln!("无效选择，请输入 0 到 13。");
                pause_menu();
            }
        }
    }
}

/// The named-credential sub-menu. It delegates to `run_credential` instead of
/// reimplementing the flows, so the same URL-safe and uniqueness rules apply
/// whether the operator types the CLI verb or walks this menu.
fn menu_named_credentials(root: &Path) {
    use crate::cli::args::CredentialCommand;
    use crate::cli::commands::system::run_credential;
    print_menu_section("命名订阅凭据");
    println!("1. 查看（密钥只显示前几位）");
    println!("2. 新增一个设备凭据");
    println!("3. 吊销某个凭据");
    println!("0. 返回");
    match read_menu_choice("请选择 [0]: ").as_deref() {
        Some("1") => {
            run_credential(root, CredentialCommand::List);
        }
        Some("2") => {
            if let Some(name) = read_menu_choice("凭据名称（例如 phone）: ") {
                run_credential(root, CredentialCommand::Add { name });
            }
        }
        Some("3") => {
            let Some(name) = read_menu_choice("要吊销的凭据名称: ") else {
                return;
            };
            let grace = read_menu_choice("宽限期（留空则立即失效，例如 30m）: ")
                .unwrap_or_default()
                .trim()
                .to_owned();
            let grace = (!grace.is_empty()).then_some(grace);
            let prompt = if grace.is_some() {
                "确认吊销该凭据？宽限期结束后旧链接失效"
            } else {
                "确认立即吊销该凭据？该设备的旧链接将马上失效"
            };
            if confirm_menu_action(prompt) {
                run_credential(root, CredentialCommand::Revoke { name, grace });
            }
        }
        _ => {}
    }
}

fn menu_traffic(root: &Path) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section("流量与账期");
        println!("1. 查看本周期流量");
        println!("2. 配置上限、已用量、时区、刷新规则和出口网卡");
        println!("3. 修正本周期总流量（GiB）");
        println!("4. 修正本周期 RX/TX 流量（GiB）");
        println!("5. 立即建立/刷新当前账期");
        println!("0. 返回");
        match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") => {
                print_traffic(root);
                pause_menu();
            }
            Some("2") => {
                run_topic_wizard(root, sbctl::wizard::ConfigurationTopic::Traffic);
                pause_menu();
            }
            Some("3") => {
                menu_total_traffic_correction(root);
                pause_menu();
            }
            Some("4") => {
                menu_direction_traffic_correction(root);
                pause_menu();
            }
            Some("5") => {
                if confirm_menu_action("确认建立/刷新当前账期？") {
                    run_accounting_reset(root);
                }
                pause_menu();
            }
            Some(_) => {
                eprintln!("无效选择，请输入 0 到 5。");
                pause_menu();
            }
        }
    }
}

fn menu_services(root: &Path) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section("服务与诊断");
        println!("1. 查看完整部署状态");
        println!("2. 校验配置并重启服务");
        println!("3. 查看运行日志");
        println!("4. 查看内核状态（版本、PID、内存、CPU、运行时长）");
        println!("5. 查看当前连接");
        println!("6. 启用连接观察 API（仅监听本机）");
        println!("7. 关闭连接观察 API");
        println!("8. 查看证书状态");
        println!("9. 签发 / 修复 HTTPS 证书");
        println!("10. 续期 HTTPS 证书");
        println!("0. 返回");
        match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") => {
                print_status(root);
                pause_menu();
            }
            Some("2") => {
                if confirm_menu_action("确认校验配置并重启服务？") {
                    restart(root, None);
                }
                pause_menu();
            }
            Some("3") => {
                menu_logs(root);
                pause_menu();
            }
            Some("4") => {
                sing_box(root, SingBoxCommand::Status);
                pause_menu();
            }
            Some("5") => {
                sing_box(root, SingBoxCommand::Connections);
                pause_menu();
            }
            Some("6") => {
                sing_box(
                    root,
                    SingBoxCommand::Api {
                        command: ApiCommand::Enable { port: None },
                    },
                );
                pause_menu();
            }
            Some("7") => {
                sing_box(
                    root,
                    SingBoxCommand::Api {
                        command: ApiCommand::Disable,
                    },
                );
                pause_menu();
            }
            Some("8") => {
                crate::cli::commands::certificate::run_certificate(
                    root,
                    CertificateCommand::Status,
                );
                pause_menu();
            }
            Some("9") => {
                if let Some(email) = read_menu_choice("ACME 邮箱: ") {
                    crate::cli::commands::certificate::run_certificate(
                        root,
                        CertificateCommand::Obtain {
                            email: Some(email),
                            no_email: false,
                        },
                    );
                }
                pause_menu();
            }
            Some("10") => {
                crate::cli::commands::certificate::run_certificate(root, CertificateCommand::Renew);
                pause_menu();
            }
            Some(_) => {
                eprintln!("无效选择，请按菜单中的编号输入。");
                pause_menu();
            }
        }
    }
}

fn menu_updates(root: &Path) {
    loop {
        clear_menu_screen();
        print_menu_header(root);
        print_menu_section("更新与卸载");
        println!("1. 更新 sbctl");
        println!("2. 更新 / 切换 sing-box 内核");
        println!("3. 卸载服务和二进制（保留备份）");
        println!("4. 彻底清理 sbctl 数据");
        println!("0. 返回");
        match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") => {
                if confirm_menu_action("确认更新 sbctl？将自动校验签名 manifest") {
                    update(root, false, None, None, None);
                }
                pause_menu();
            }
            Some("2") => {
                if sbctl::config::DeploymentStore::new(root).load().is_err() {
                    menu_install(root, true);
                } else if let Some((source, version)) = select_kernel() {
                    sing_box(root, SingBoxCommand::Fetch { source, version });
                }
                pause_menu();
            }
            Some("3") => {
                if confirm_menu_action("确认卸载服务和二进制并保留备份？") {
                    uninstall(root, false);
                    return;
                }
                pause_menu();
            }
            Some("4") => {
                if confirm_menu_action("确认彻底清理 sbctl 数据？此操作不可恢复")
                    && confirm_menu_action("再次确认：删除配置、状态、证书和备份？")
                {
                    uninstall(root, true);
                    return;
                }
                pause_menu();
            }
            Some(_) => {
                eprintln!("无效选择，请输入 0 到 4。");
                pause_menu();
            }
        }
    }
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
            let _ = commit_config_change(root, &store, &new, None);
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
    match sbctl::config::DeploymentStore::new(root).load() {
        Ok(_) => {
            eprintln!("已安装。请返回上级菜单选择完整配置向导或重新生成配置工件。");
            ExitCode::from(2)
        }
        Err(sbctl::config::ConfigError::Missing) => install(
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
                replace_existing: false,
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
        _ => None,
    }
}

fn menu_configuration(root: &Path) {
    loop {
        print_menu_section("配置与路由规则");
        println!("1. 添加入站 / 出站 / 路由规则（服务端 JSON 覆写）");
        println!("2. 编辑客户端 sing-box 出站与规则");
        println!("3. 编辑 Clash 规则");
        println!("4. 预览整体配置（含凭据）");
        println!("5. 导出 / 下载整体配置（私有文件）");
        println!("6. 查看覆写层");
        println!("0. 返回");
        let selection = read_menu_choice("请选择 [0]: ");
        let command = match selection.as_deref() {
            Some("0") | None => return,
            Some("1") => {
                println!(
                    "自定义入站使用 custom- 开头的 tag；会保留现有入站。可在 JSON 中添加 inbounds、outbounds、route.rules。"
                );
                ConfigCommand::Override {
                    command: OverrideCommand::Edit {
                        target: CliOverrideTarget::Server,
                        layer: None,
                        sing_box_bin: None,
                    },
                }
            }
            Some("2") => ConfigCommand::Override {
                command: OverrideCommand::Edit {
                    target: CliOverrideTarget::SingBox,
                    layer: None,
                    sing_box_bin: None,
                },
            },
            Some("3") => ConfigCommand::Override {
                command: OverrideCommand::Edit {
                    target: CliOverrideTarget::Clash,
                    layer: None,
                    sing_box_bin: None,
                },
            },
            Some("4") | Some("5") => {
                let export = selection.as_deref() == Some("5");
                let format = read_menu_choice("格式 server / sing-box-full / clash [server]: ")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "server".into());
                if export {
                    let Some(output) = read_menu_choice("导出到新文件路径（可使用 scp 下载）: ")
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    ConfigCommand::Export {
                        format,
                        output: output.into(),
                    }
                } else {
                    ConfigCommand::Preview { format }
                }
            }
            Some("6") => ConfigCommand::Override {
                command: OverrideCommand::Show,
            },
            _ => continue,
        };
        crate::cli::commands::config::run_config(root, command);
        pause_menu();
    }
}

fn menu_email(root: &Path) {
    loop {
        print_menu_section("邮件通知");
        println!("1. 配置 SMTP、收件人、日报和刷新提醒");
        println!("2. 查看邮件配置（隐藏密码）");
        println!("3. 立即发送状态 / 流量 / 订阅报告");
        println!("4. 启用定时邮件");
        println!("5. 关闭定时邮件");
        println!("0. 返回");
        let command = match read_menu_choice("请选择 [0]: ").as_deref() {
            Some("0") | None => return,
            Some("1") => EmailCommand::Configure,
            Some("2") => EmailCommand::Status,
            Some("3") => EmailCommand::Send,
            Some("4") => EmailCommand::Enable,
            Some("5") => EmailCommand::Disable,
            _ => continue,
        };
        crate::cli::commands::email::run_email(root, command);
        pause_menu();
    }
}
