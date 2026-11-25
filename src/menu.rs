//! Single-screen 8-item interactive terminal menu (`ly` / `vsm`).
//! Every input, selection, and confirmation accepts `0` to immediately cancel
//! and return without mutating files.

use std::net::IpAddr;
use std::path::Path;
use std::process::Command;

use crate::apply;
use crate::compiler;
use crate::config::{Config, is_valid_host, is_valid_hostname, is_valid_interface};
use crate::prompt::{PromptError, PromptIo, PromptResult, ask_menu_choice, ask_parsed, ask_text};
use crate::qr;
use crate::secrets::{Secrets, SubscriptionToken, random_urlsafe};
use crate::traffic;

pub fn run(root: &Path, io: &mut impl PromptIo) {
    loop {
        clear_screen();
        print_banner(root, io);
        io.report("  1. 快速安装 / 重新初始化向导          (4 步完成，随时按 0 取消)");
        io.report("  2. 查看订阅链接与二维码               (直连 5 节点 / Argo 优选)");
        io.report(
            "  3. 节点与端口配置                     (开关节点、改端口/SNI/Hy2跳跃、重生成密钥)",
        );
        io.report("  4. Cloudflare Argo 与优选 IP 管理     (开关隧道、编辑优选 IP 列表)");
        io.report("  5. 流量账期与校准                     (设置月额度/重置日、校准已用流量)");
        io.report("  6. 订阅凭据管理                       (新增/轮换/吊销 Token)");
        io.report(
            "  7. 系统维护与组件更新                 (更新内核、续签证书、BBR调优、查看日志)",
        );
        io.report(
            "  8. 编辑外部配置文件并校验生效         (config.toml / mihomo.yaml / sing-box.json)",
        );
        io.report("  0. 退出");

        let choice = match ask_menu_choice(io, "请选择操作", 8) {
            Ok(0) | Err(PromptError::Cancelled | PromptError::Eof) => return,
            Ok(n) => n,
            Err(e) => {
                io.report(&format!("读取失败: {e}"));
                return;
            }
        };

        let res = match choice {
            1 => action_install_wizard(root, io),
            2 => action_subscriptions(root, io),
            3 => action_nodes_menu(root, io),
            4 => action_argo_menu(root, io),
            5 => action_traffic_menu(root, io),
            6 => action_tokens_menu(root, io),
            7 => action_maintenance_menu(root, io),
            8 => action_edit_files_menu(root, io),
            _ => Ok(()),
        };

        match res {
            Ok(()) => {}
            Err(PromptError::Cancelled) => io.report("已输入 0 返回主菜单（未改动配置）。"),
            Err(PromptError::Eof) => return,
            Err(e) => io.report(&format!("操作出错: {e}")),
        }
        io.pause("按回车返回主菜单...");
    }
}

fn clear_screen() {
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        print!("\x1b[2J\x1b[H");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}

fn print_banner(root: &Path, io: &mut impl PromptIo) {
    let line = "----------------------------------------------------------------------";
    io.report(line);
    io.report(&format!(
        "  vps-sub-meter v{}  ·  双内核 PQC 节点与订阅管理 (命令: vsm / ly)",
        env!("CARGO_PKG_VERSION")
    ));
    io.report(line);
    match Config::load(root) {
        Ok(cfg) => {
            let host_lbl = if cfg.is_ip_mode() {
                format!("{} (IP 自签模式)", cfg.public_ip)
            } else {
                cfg.domain.clone()
            };
            let enabled = [
                cfg.nodes.vless_reality_pqc.enabled,
                cfg.nodes.vless_xhttp_pqc.enabled,
                cfg.nodes.anytls_tcp.enabled,
                cfg.nodes.hysteria2_hop.enabled,
                cfg.nodes.tuic_bbr.enabled,
            ]
            .into_iter()
            .filter(|x| *x)
            .count();
            let argo_ips = if cfg.argo.enabled {
                format!(
                    "开启, {} 优选IP",
                    compiler::load_argo_preferred_ips(root, &cfg).len()
                )
            } else {
                "关闭".into()
            };
            io.report(&format!(
                "  地址: {host_lbl}   节点: {enabled}/5 启用   Argo: {argo_ips}"
            ));
            if root == Path::new("/") {
                io.report(&format!(
                    "  服务: vps-sub-meter({}) | xray({}) | sing-box({})",
                    svc_mark("vps-sub-meter.service"),
                    svc_mark("vps-sub-meter-xray.service"),
                    svc_mark("vps-sub-meter-singbox.service"),
                ));
            }
            if let Ok(snap) = traffic::snapshot(root, &cfg) {
                let total = snap.total_used();
                let limit_str = if snap.monthly_limit_bytes > 0 {
                    let pct = (total as f64 / snap.monthly_limit_bytes as f64) * 100.0;
                    format!(
                        "{} ({pct:.1}%)",
                        traffic::format_iec(snap.monthly_limit_bytes)
                    )
                } else {
                    "不限".into()
                };
                io.report(&format!(
                    "  流量: {} / {limit_str}   下次重置: {} ({})",
                    traffic::format_iec(total),
                    traffic::format_time_in_tz(snap.next_reset_ts, &cfg.traffic.display_tz),
                    cfg.traffic.display_tz
                ));
            }
        }
        Err(_) => {
            io.report("  状态: 尚未初始化部署，请选择「1. 快速安装 / 重新初始化向导」");
        }
    }
    io.report(line);
}

fn svc_mark(unit: &str) -> &'static str {
    let ok = Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .is_ok_and(|s| s.success());
    if ok { "运行中" } else { "未运行" }
}

fn action_install_wizard(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    let existing = Config::load(root).ok();
    let default_iface = detect_default_interface(root);
    let default_ip = detect_public_ip().unwrap_or_else(|| "127.0.0.1".into());

    io.report("\n=== 4 步快速安装 / 初始化向导 (任何步骤输入 0 立即返回) ===");

    // Step 1: Domain or IP
    let def_host = existing
        .as_ref()
        .map(|c| {
            if c.domain.is_empty() {
                c.public_ip.clone()
            } else {
                c.domain.clone()
            }
        })
        .unwrap_or_else(|| default_ip.clone());
    let host = ask_parsed(
        io,
        "第 1/4 步 - 订阅域名（无域名可直接回车使用探测公网 IP 走自签模式）",
        Some(&def_host),
        |s| {
            if is_valid_host(s) {
                Ok(s.to_owned())
            } else {
                Err("必须是合法域名或 IP 地址".into())
            }
        },
    )?;

    let (domain, public_ip) = if host.parse::<IpAddr>().is_ok() {
        (String::new(), host)
    } else {
        let ip = ask_parsed(
            io,
            "公网 IPv4 地址（用于 VLESS Reality 直连免 DNS 解析）",
            Some(
                existing
                    .as_ref()
                    .map(|c| c.public_ip.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&default_ip),
            ),
            |s| {
                s.parse::<IpAddr>()
                    .map(|_| s.to_owned())
                    .map_err(|_| "必须是合法 IP 地址".into())
            },
        )?;
        (host, ip)
    };

    // Step 2: Traffic limit & reset day
    let def_gb = existing
        .as_ref()
        .map(|c| c.traffic.monthly_gb.to_string())
        .unwrap_or_else(|| "0".into());
    let monthly_gb = ask_parsed(
        io,
        "第 2/4 步 - 每月流量限额 GiB（0 为不限）",
        Some(&def_gb),
        |s| {
            let v: f64 = s
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .parse()
                .map_err(|_| "请输入非负数字".to_owned())?;
            if v < 0.0 || !v.is_finite() {
                return Err("不能为负数".into());
            }
            Ok(v)
        },
    )?;
    let def_day = existing
        .as_ref()
        .map(|c| c.traffic.reset_day.to_string())
        .unwrap_or_else(|| "1".into());
    let reset_day = ask_parsed(io, "每月流量重置日 (1-31)", Some(&def_day), |s| {
        let d: u8 = s.parse().map_err(|_| "请输入 1-31 的整数".to_owned())?;
        if !(1..=31).contains(&d) {
            return Err("范围须在 1-31".into());
        }
        Ok(d)
    })?;

    // Step 3: Port profile
    io.report("第 3/4 步 - 节点端口方案:");
    io.report("  1. 推荐标准端口 (443 Reality / 2053 XHTTP / 2083 AnyTLS / 34443[30000-35000] Hy2 / 2096 TUIC)");
    io.report("  2. 随机高端口方案 (避让常用端口)");
    let port_mode = ask_menu_choice(io, "选择端口方案 [1]", 2)?;
    if port_mode == 0 {
        return Err(PromptError::Cancelled);
    }

    let mut cfg = existing.unwrap_or_default();
    cfg.domain = domain.clone();
    cfg.public_ip = public_ip;
    cfg.interface = default_iface;
    cfg.traffic.monthly_gb = monthly_gb;
    cfg.traffic.reset_day = reset_day;
    if port_mode == 2 {
        cfg.nodes.vless_reality_pqc.port = 24443;
        cfg.nodes.vless_xhttp_pqc.port = 22053;
        cfg.nodes.anytls_tcp.port = 22083;
        cfg.nodes.hysteria2_hop.port = 34443;
        cfg.nodes.tuic_bbr.port = 22096;
    }

    // Step 4: Summary & confirm
    io.report("\n第 4/4 步 - 配置预览:");
    io.report(&format!(
        "  模式: {} | 网卡: {} | 月流量: {} GiB (每月 {} 日重置)",
        if cfg.is_ip_mode() {
            "IP 自签 HTTP"
        } else {
            "域名 HTTPS + 443 共用"
        },
        cfg.interface,
        cfg.traffic.monthly_gb,
        cfg.traffic.reset_day
    ));
    io.report(&format!(
        "  端口: Reality={} | XHTTP={} | AnyTLS={} | Hy2={} | TUIC={}",
        cfg.nodes.vless_reality_pqc.port,
        cfg.nodes.vless_xhttp_pqc.port,
        cfg.nodes.anytls_tcp.port,
        cfg.nodes.hysteria2_hop.port,
        cfg.nodes.tuic_bbr.port
    ));

    if !io.confirm("确认写入配置、安装核心/证书并启动服务？", true)? {
        return Err(PromptError::Cancelled);
    }

    if let Err(e) = cfg.save(root) {
        io.report(&format!("保存配置失败: {e}"));
        return Ok(());
    }

    if root == Path::new("/") {
        run_shell_helper(io, "/usr/local/lib/vps-sub-meter/cores.sh", &["ensure"]);
        run_shell_helper(io, "/usr/local/lib/vps-sub-meter/net.sh", &["tune-sysctl"]);
        if cfg.is_ip_mode() {
            run_shell_helper(
                io,
                "/usr/local/lib/vps-sub-meter/cert.sh",
                &["self-signed", cfg.tls_sni()],
            );
        } else {
            let email = ask_text(io, "ACME 邮箱（可直接回车免邮箱签发）", Some(""), true)?;
            if email.is_empty() {
                run_shell_helper(
                    io,
                    "/usr/local/lib/vps-sub-meter/cert.sh",
                    &["issue", &cfg.domain],
                );
            } else {
                run_shell_helper(
                    io,
                    "/usr/local/lib/vps-sub-meter/cert.sh",
                    &["issue", &cfg.domain, &email],
                );
            }
        }
    }

    match apply::apply_all(root) {
        Ok(rep) => {
            io.report(&format!(
                "部署完成! 批次 {}, 直连 {} 节点 (sing-box 兼容 {} 节点)",
                rep.generation_id, rep.direct_nodes, rep.singbox_nodes
            ));
            if let Ok(sec) = Secrets::load_or_default(root)
                && let Some(tok) = sec.primary_token()
            {
                io.report(&format!(
                    "订阅看板: {}/index",
                    cfg.subscription_base_url(tok)
                ));
            }
        }
        Err(e) => io.report(&format!("应用失败（已保留原有运行状态）: {e}")),
    }
    Ok(())
}

fn action_subscriptions(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    let cfg = match Config::load(root) {
        Ok(c) => c,
        Err(e) => {
            io.report(&format!("请先初始化配置: {e}"));
            return Ok(());
        }
    };
    let sec = Secrets::load_or_default(root).unwrap_or_default();
    let Some(tok) = sec.primary_token() else {
        io.report("尚未生成订阅凭据，请先执行 `vsm apply`");
        return Ok(());
    };
    let base = cfg.subscription_base_url(tok);
    io.report("\n=== 订阅链接 (已脱敏规则: 此处仅本地管理员终端可见) ===");
    io.report(&format!("  [看板主页]     {base}/index"));
    io.report(&format!("  [Clash/Mihomo] {base}/clash.yaml"));
    io.report(&format!("  [sing-box]     {base}/sing-box.json"));
    io.report(&format!("  [Shadowrocket] {base}/shadowrocket.txt"));
    io.report(&format!("  [明文 URI]     {base}/uri.txt"));
    if cfg.argo.enabled {
        let ab = format!("https://{}/sub/{tok}", cfg.argo.sub_domain);
        io.report("\n=== Cloudflare Argo 优选订阅 ===");
        io.report(&format!("  [Argo 看板]    {ab}/index"));
        io.report(&format!("  [Argo Clash]   {ab}/clash.yaml"));
        io.report(&format!("  [Argo singbox] {ab}/sing-box.json"));
    }

    io.report("\n显示二维码: 1 Clash | 2 sing-box | 3 Shadowrocket | 0 返回");
    match ask_menu_choice(io, "选择二维码", 3)? {
        0 => return Ok(()),
        1 => print_qr(io, &format!("{base}/clash.yaml")),
        2 => print_qr(
            io,
            &format!(
                "sing-box://import-remote-profile?url={}#{}",
                compiler::pct(&format!("{base}/sing-box.json")),
                compiler::pct(&cfg.profile_name)
            ),
        ),
        3 => print_qr(io, &format!("{base}/shadowrocket.txt")),
        _ => {}
    }
    Ok(())
}

fn print_qr(io: &mut impl PromptIo, payload: &str) {
    match qr::render_ansi(payload) {
        Ok(ansi) => io.report(&ansi),
        Err(e) => io.report(&format!("二维码生成失败: {e}")),
    }
}

fn action_nodes_menu(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    let mut cfg = match Config::load(root) {
        Ok(c) => c,
        Err(e) => {
            io.report(&format!("{e}"));
            return Ok(());
        }
    };
    io.report("\n=== 节点与端口配置 (输入 0 返回不保存) ===");
    io.report(&format!(
        "  1. [Xray]     {} (启用: {}, 端口: {}, 伪装: {})",
        cfg.nodes.vless_reality_pqc.name,
        cfg.nodes.vless_reality_pqc.enabled,
        cfg.nodes.vless_reality_pqc.port,
        cfg.nodes.vless_reality_pqc.decoy_sni
    ));
    io.report(&format!(
        "  2. [Xray]     {} (启用: {}, 端口: {})",
        cfg.nodes.vless_xhttp_pqc.name,
        cfg.nodes.vless_xhttp_pqc.enabled,
        cfg.nodes.vless_xhttp_pqc.port
    ));
    io.report(&format!(
        "  3. [sing-box] {} (启用: {}, 端口: {})",
        cfg.nodes.anytls_tcp.name, cfg.nodes.anytls_tcp.enabled, cfg.nodes.anytls_tcp.port
    ));
    io.report(&format!(
        "  4. [sing-box] {} (启用: {}, 端口: {}, 跳跃: {:?})",
        cfg.nodes.hysteria2_hop.name,
        cfg.nodes.hysteria2_hop.enabled,
        cfg.nodes.hysteria2_hop.port,
        cfg.nodes.hysteria2_hop.hop_range
    ));
    io.report(&format!(
        "  5. [sing-box] {} (启用: {}, 端口: {})",
        cfg.nodes.tuic_bbr.name, cfg.nodes.tuic_bbr.enabled, cfg.nodes.tuic_bbr.port
    ));
    io.report("  6. 重新生成所有节点密钥 (UUID / 密码 / PQC 密钥)");
    io.report("  0. 返回");

    let sel = ask_menu_choice(io, "选择修改项", 6)?;
    if sel == 0 {
        return Err(PromptError::Cancelled);
    }

    match sel {
        1 => {
            cfg.nodes.vless_reality_pqc.enabled = io.confirm(
                "启用 vless-reality-pqc?",
                cfg.nodes.vless_reality_pqc.enabled,
            )?;
            let port_s = cfg.nodes.vless_reality_pqc.port.to_string();
            cfg.nodes.vless_reality_pqc.port =
                ask_parsed(io, "监听端口", Some(&port_s), parse_port)?;
            cfg.nodes.vless_reality_pqc.decoy_sni = ask_parsed(
                io,
                "Reality 伪装 SNI",
                Some(&cfg.nodes.vless_reality_pqc.decoy_sni),
                |s| {
                    if is_valid_hostname(s) {
                        Ok(s.to_owned())
                    } else {
                        Err("无效域名".into())
                    }
                },
            )?;
        }
        2 => {
            cfg.nodes.vless_xhttp_pqc.enabled =
                io.confirm("启用 vless-xhttp-pqc?", cfg.nodes.vless_xhttp_pqc.enabled)?;
            let port_s = cfg.nodes.vless_xhttp_pqc.port.to_string();
            cfg.nodes.vless_xhttp_pqc.port = ask_parsed(io, "监听端口", Some(&port_s), parse_port)?;
        }
        3 => {
            cfg.nodes.anytls_tcp.enabled =
                io.confirm("启用 anytls-tcp?", cfg.nodes.anytls_tcp.enabled)?;
            let port_s = cfg.nodes.anytls_tcp.port.to_string();
            cfg.nodes.anytls_tcp.port = ask_parsed(io, "监听端口", Some(&port_s), parse_port)?;
        }
        4 => {
            cfg.nodes.hysteria2_hop.enabled =
                io.confirm("启用 hysteria2-hop?", cfg.nodes.hysteria2_hop.enabled)?;
            let port_s = cfg.nodes.hysteria2_hop.port.to_string();
            cfg.nodes.hysteria2_hop.port =
                ask_parsed(io, "基础 UDP 端口", Some(&port_s), parse_port)?;
            let def_range = cfg
                .nodes
                .hysteria2_hop
                .hop_range
                .as_deref()
                .unwrap_or("30000-35000");
            let r = ask_text(io, "跳跃端口范围 (格式 30000-35000)", Some(def_range), true)?;
            cfg.nodes.hysteria2_hop.hop_range = (!r.is_empty()).then_some(r);
        }
        5 => {
            cfg.nodes.tuic_bbr.enabled =
                io.confirm("启用 tuic-bbr?", cfg.nodes.tuic_bbr.enabled)?;
            let port_s = cfg.nodes.tuic_bbr.port.to_string();
            cfg.nodes.tuic_bbr.port = ask_parsed(io, "监听 UDP 端口", Some(&port_s), parse_port)?;
        }
        6 => {
            if !io.confirm("确认重置全部节点密钥？旧客户端链接需更新订阅", false)?
            {
                return Err(PromptError::Cancelled);
            }
            let mut sec = match Secrets::load_or_default(root) {
                Ok(s) => s,
                Err(e) => {
                    io.report(&format!("读取密钥失败: {e}"));
                    return Ok(());
                }
            };
            let tokens = sec.tokens;
            sec = Secrets::default();
            sec.tokens = tokens;
            if let Err(e) = sec.save(root) {
                io.report(&format!("保存密钥失败: {e}"));
                return Ok(());
            }
        }
        _ => {}
    }

    if let Err(e) = cfg.save(root) {
        io.report(&format!("校验未通过，已放弃: {e}"));
        return Ok(());
    }
    match apply::apply_all(root) {
        Ok(rep) => io.report(&format!("已应用! 批次 {}", rep.generation_id)),
        Err(e) => io.report(&format!("应用失败（已保留旧运行态）: {e}")),
    }
    Ok(())
}

fn action_argo_menu(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    let mut cfg = match Config::load(root) {
        Ok(c) => c,
        Err(e) => {
            io.report(&format!("{e}"));
            return Ok(());
        }
    };
    io.report("\n=== Cloudflare Argo 与优选 IP 管理 ===");
    io.report(&format!(
        "  当前状态: {} | 代理域名: {} | 订阅域名: {}",
        if cfg.argo.enabled { "启用" } else { "关闭" },
        cfg.argo.tunnel_domain,
        cfg.argo.sub_domain
    ));
    io.report("  1. 配置并启用/关闭 Argo 订阅");
    io.report("  2. 编辑优选 IP 列表 (/etc/vps-sub-meter/argo-ips.txt，保存即热生效)");
    io.report("  3. 安装/配置 cloudflared 隧道服务");
    io.report("  0. 返回");

    match ask_menu_choice(io, "请选择", 3)? {
        0 => return Err(PromptError::Cancelled),
        1 => {
            cfg.argo.enabled = io.confirm("启用 Cloudflare Argo 订阅?", cfg.argo.enabled)?;
            if cfg.argo.enabled {
                cfg.argo.tunnel_domain = ask_parsed(
                    io,
                    "Argo 代理域名",
                    Some(&cfg.argo.tunnel_domain),
                    |s| {
                        if is_valid_hostname(s) {
                            Ok(s.to_owned())
                        } else {
                            Err("无效域名".into())
                        }
                    },
                )?;
                cfg.argo.sub_domain =
                    ask_parsed(io, "Argo 订阅域名", Some(&cfg.argo.sub_domain), |s| {
                        if is_valid_hostname(s) {
                            Ok(s.to_owned())
                        } else {
                            Err("无效域名".into())
                        }
                    })?;
            }
            cfg.save(root).map_err(|e| {
                io.report(&format!("保存失败: {e}"));
                PromptError::Cancelled
            })?;
            if let Err(e) = apply::apply_all(root) {
                io.report(&format!("应用失败: {e}"));
            } else {
                io.report("Argo 配置已生效");
            }
        }
        2 => {
            let path = root.join("etc/vps-sub-meter/argo-ips.txt");
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".into());
            if root == Path::new("/") {
                let _ = Command::new(editor).arg(&path).status();
                io.report(&format!(
                    "当前生效优选 IP 数: {}",
                    compiler::load_argo_preferred_ips(root, &cfg).len()
                ));
            } else {
                io.report(&format!("测试根目录路径: {}", path.display()));
            }
        }
        3 if root == Path::new("/") => {
            let tok = ask_text(io, "Cloudflare Tunnel Token", None, false)?;
            run_shell_helper(io, "/usr/local/lib/vps-sub-meter/argo.sh", &["setup", &tok]);
        }
        _ => {}
    }
    Ok(())
}

fn action_traffic_menu(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    let mut cfg = match Config::load(root) {
        Ok(c) => c,
        Err(e) => {
            io.report(&format!("{e}"));
            return Ok(());
        }
    };
    io.report("\n=== 流量账期与校准 ===");
    io.report("  1. 修改月流量额度、重置日或监控网卡");
    io.report("  2. 手动校准当前已用上传/下载流量 (GiB)");
    io.report("  3. 立即刷新账期基线 (checkpoint)");
    io.report("  0. 返回");

    match ask_menu_choice(io, "请选择", 3)? {
        0 => return Err(PromptError::Cancelled),
        1 => {
            let def_gb = cfg.traffic.monthly_gb.to_string();
            cfg.traffic.monthly_gb =
                ask_parsed(io, "月流量额度 GiB (0 不限)", Some(&def_gb), |s| {
                    s.parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .ok_or_else(|| "请输入非负数字".into())
                })?;
            let def_day = cfg.traffic.reset_day.to_string();
            cfg.traffic.reset_day = ask_parsed(io, "重置日 (1-31)", Some(&def_day), |s| {
                s.parse::<u8>()
                    .ok()
                    .filter(|d| (1..=31).contains(d))
                    .ok_or_else(|| "范围 1-31".into())
            })?;
            cfg.interface = ask_parsed(io, "监控网卡", Some(&cfg.interface), |s| {
                if is_valid_interface(s) {
                    Ok(s.to_owned())
                } else {
                    Err("无效网卡名".into())
                }
            })?;
            if let Err(e) = cfg.save(root) {
                io.report(&format!("保存失败: {e}"));
            } else {
                let _ = apply::apply_all(root);
                io.report("流量设置已更新");
            }
        }
        2 => {
            let up_g: f64 = ask_parsed(io, "当前客户端上传流量 (GiB)", Some("0"), |s| {
                s.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .ok_or_else(|| "非负数字".into())
            })?;
            let dn_g: f64 = ask_parsed(io, "当前客户端下载流量 (GiB)", Some("0"), |s| {
                s.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .ok_or_else(|| "非负数字".into())
            })?;
            let gib = 1024.0 * 1024.0 * 1024.0;
            match traffic::set_used(root, &cfg, (up_g * gib) as u64, (dn_g * gib) as u64) {
                Ok(_) => io.report("已校准流量账本"),
                Err(e) => io.report(&format!("校准失败: {e}")),
            }
        }
        3 => match traffic::checkpoint(root, &cfg) {
            Ok(s) => io.report(&format!(
                "已刷盘: 上传 {}, 下载 {}",
                traffic::format_iec(s.upload_bytes),
                traffic::format_iec(s.download_bytes)
            )),
            Err(e) => io.report(&format!("刷新失败: {e}")),
        },
        _ => {}
    }
    Ok(())
}

fn action_tokens_menu(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    let mut sec = Secrets::load_or_default(root).unwrap_or_default();
    io.report("\n=== 订阅凭据管理 ===");
    for (i, t) in sec.tokens.iter().enumerate() {
        let masked = if t.token.len() > 8 {
            format!("{}...{}", &t.token[..4], &t.token[t.token.len() - 4..])
        } else {
            "***".into()
        };
        io.report(&format!("  [{}] {} ({masked})", i + 1, t.name));
    }
    io.report("  1. 新增设备凭据");
    io.report("  2. 轮换默认凭据 (旧默认链接立即失效)");
    io.report("  3. 按名称吊销凭据");
    io.report("  0. 返回");

    match ask_menu_choice(io, "请选择", 3)? {
        0 => return Err(PromptError::Cancelled),
        1 => {
            let name = ask_text(io, "设备名称", None, false)?;
            sec.tokens.push(SubscriptionToken {
                name,
                token: random_urlsafe(32).map_err(|_| PromptError::Cancelled)?,
                created_at: chrono::Utc::now().timestamp(),
            });
            let _ = sec.save(root);
            let _ = apply::apply_all(root);
            io.report("新凭据已创建并生效");
        }
        2 => {
            if !io.confirm("确认轮换默认订阅凭据?", false)? {
                return Err(PromptError::Cancelled);
            }
            if let Some(first) = sec.tokens.first_mut() {
                first.token = random_urlsafe(32).map_err(|_| PromptError::Cancelled)?;
                first.created_at = chrono::Utc::now().timestamp();
            }
            let _ = sec.save(root);
            let _ = apply::apply_all(root);
            io.report("默认凭据已轮换");
        }
        3 => {
            let name = ask_text(io, "要吊销的凭据名称", None, false)?;
            if sec.tokens.len() <= 1 {
                io.report("不能吊销最后一个凭据，请使用轮换功能");
                return Ok(());
            }
            let before = sec.tokens.len();
            sec.tokens.retain(|t| t.name != name);
            if sec.tokens.len() < before {
                let _ = sec.save(root);
                let _ = apply::apply_all(root);
                io.report("已吊销并生效");
            } else {
                io.report("未找到该名称的凭据");
            }
        }
        _ => {}
    }
    Ok(())
}

fn action_maintenance_menu(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    io.report("\n=== 系统维护与组件更新 ===");
    io.report("  1. 安装/更新官方 Xray 与 sing-box 内核 (cores.sh)");
    io.report("  2. 签发或续期 HTTPS 证书 (cert.sh)");
    io.report("  3. 应用 BBR / 16MiB UDP 内核调优 (net.sh)");
    io.report("  4. 查看服务近期日志 (journalctl)");
    io.report("  5. 在线检查并升级 vps-sub-meter 主程序 (upgrade.sh)");
    io.report("  0. 返回");

    match ask_menu_choice(io, "请选择", 5)? {
        0 => return Err(PromptError::Cancelled),
        1 if root == Path::new("/") => {
            run_shell_helper(io, "/usr/local/lib/vps-sub-meter/cores.sh", &["update"]);
            let _ = apply::apply_all(root);
        }
        2 if root == Path::new("/") => {
            if let Ok(cfg) = Config::load(root)
                && !cfg.is_ip_mode()
            {
                run_shell_helper(
                    io,
                    "/usr/local/lib/vps-sub-meter/cert.sh",
                    &["renew", &cfg.domain],
                );
            } else {
                io.report("当前为 IP 自签模式，无需续签域名证书");
            }
        }
        3 if root == Path::new("/") => {
            run_shell_helper(io, "/usr/local/lib/vps-sub-meter/net.sh", &["tune-sysctl"]);
        }
        4 if root == Path::new("/") => {
            let _ = Command::new("journalctl")
                .args([
                    "-u",
                    "vps-sub-meter",
                    "-u",
                    "vps-sub-meter-xray",
                    "-u",
                    "vps-sub-meter-singbox",
                    "-n",
                    "40",
                    "--no-pager",
                ])
                .status();
        }
        5 if root == Path::new("/") => {
            run_shell_helper(io, "/usr/local/lib/vps-sub-meter/upgrade.sh", &[]);
        }
        _ => io.report("仅在实机 / 环境可执行系统维护命令"),
    }
    Ok(())
}

fn action_edit_files_menu(root: &Path, io: &mut impl PromptIo) -> PromptResult<()> {
    io.report("\n=== 编辑外部配置文件并校验生效 ===");
    io.report("  1. 编辑主配置 (/etc/vps-sub-meter/config.toml)");
    io.report("  2. 编辑 Mihomo 客户端模板 (/etc/vps-sub-meter/templates/mihomo.yaml)");
    io.report("  3. 编辑 sing-box 客户端模板 (/etc/vps-sub-meter/templates/sing-box.json)");
    io.report("  4. 立即重新校验并应用全部配置 (`vsm apply`)");
    io.report("  0. 返回");

    let sel = ask_menu_choice(io, "请选择", 4)?;
    if sel == 0 {
        return Err(PromptError::Cancelled);
    }
    if sel <= 3 && root == Path::new("/") {
        let target = match sel {
            1 => "/etc/vps-sub-meter/config.toml",
            2 => "/etc/vps-sub-meter/templates/mihomo.yaml",
            _ => "/etc/vps-sub-meter/templates/sing-box.json",
        };
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".into());
        let _ = Command::new(editor).arg(target).status();
    }
    match apply::apply_all(root) {
        Ok(rep) => io.report(&format!("校验并应用成功! 批次 {}", rep.generation_id)),
        Err(e) => io.report(&format!("校验未通过（现有服务未受影响）: {e}")),
    }
    Ok(())
}

fn run_shell_helper(io: &mut impl PromptIo, script: &str, args: &[&str]) {
    match Command::new("bash").arg(script).args(args).status() {
        Ok(st) if st.success() => io.report(&format!("{script} 执行完成")),
        Ok(st) => io.report(&format!("{script} 退出码: {st}")),
        Err(e) => io.report(&format!("无法执行 {script}: {e}")),
    }
}

fn parse_port(s: &str) -> Result<u16, String> {
    let p: u16 = s.parse().map_err(|_| "端口必须是整数".to_owned())?;
    crate::config::check_node_port(p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn detect_default_interface(root: &Path) -> String {
    if root == Path::new("/")
        && let Ok(out) = Command::new("sh")
            .args(["-c", "ip -4 route show default | awk '{print $5; exit}'"])
            .output()
    {
        let iface = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if is_valid_interface(&iface) {
            return iface;
        }
    }
    "eth0".into()
}

fn detect_public_ip() -> Option<String> {
    for ep in ["https://api.ipify.org", "https://api64.ipify.org"] {
        if let Ok(out) = Command::new("curl")
            .args([
                "--disable",
                "--noproxy",
                "*",
                "--fail",
                "--silent",
                "--connect-timeout",
                "3",
                "--max-time",
                "5",
                ep,
            ])
            .output()
            && out.status.success()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if s.parse::<IpAddr>().is_ok() {
                return Some(s);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::BufferedIo;

    #[test]
    fn install_wizard_returns_cleanly_on_zero_at_any_step() {
        let dir = tempfile::tempdir().unwrap();
        // Cancel at step 2 ("0\n") after entering domain at step 1: nothing written!
        let mut io = BufferedIo::new(
            std::io::Cursor::new("sub.example.com\n1.2.3.4\n0\n"),
            Vec::new(),
        );
        let res = action_install_wizard(dir.path(), &mut io);
        assert!(matches!(res, Err(PromptError::Cancelled)));
        assert!(!Config::path(dir.path()).exists());
    }
}
