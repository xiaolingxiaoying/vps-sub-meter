//! `vps-sub-meter` (`vsm`) CLI entrypoint.

mod admin;
mod panel;

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vps_sub_meter::{apply, config::Config, serve, store, traffic};

#[derive(Parser)]
#[command(
    name = "vps-sub-meter",
    version,
    about = "双内核 (Xray + sing-box) PQC 代理与私密订阅管理控制面"
)]
struct Cli {
    #[arg(long, default_value = "/")]
    root: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动 HTTPS / ACME / Argo 订阅守护进程
    Serve,
    /// 按需启动仅供 SSH 转发访问的网页管理面板
    Panel {
        #[arg(long, default_value_t = 8787, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
    },
    /// 校验配置与模板、生成新订阅批次并平滑重启变动的服务
    Apply,
    /// 查看当前运行状态、节点摘要与实时流量
    Status {
        #[arg(long)]
        json: bool,
    },
    /// 打印订阅链接 (加 --qr 输出终端二维码)
    Sub(admin::SubArgs),
    /// 查看、预览与保存部署配置
    Config {
        #[command(subcommand)]
        cmd: admin::ConfigCmd,
    },
    /// 管理订阅凭据
    Token {
        #[command(subcommand)]
        cmd: admin::TokenCmd,
    },
    /// 管理代理节点密钥
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// 流量账期操作
    Traffic {
        #[command(subcommand)]
        cmd: TrafficCmd,
    },
}

#[derive(Subcommand)]
enum KeysCmd {
    /// 轮换节点密钥并应用配置
    Rotate {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum TrafficCmd {
    /// 将内存累加刷新到磁盘
    Checkpoint,
    /// 手动校准已用上传和下载 GiB
    Set { upload_gib: f64, download_gib: f64 },
    /// 更新月流量上限与重置时间
    Configure {
        #[arg(long)]
        monthly_gib: Option<f64>,
        #[arg(long)]
        reset_day: Option<u8>,
        #[arg(long)]
        reset_time: Option<String>,
        #[arg(long)]
        reset_tz: Option<String>,
        #[arg(long)]
        display_tz: Option<String>,
    },
}

fn main() -> ExitCode {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    match run(&cli.root, cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("错误: {msg}");
            ExitCode::from(2)
        }
    }
}

fn run(root: &Path, command: Commands) -> Result<(), String> {
    match command {
        Commands::Serve => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            rt.block_on(serve::run_server(root.to_path_buf()))
                .map_err(|e| e.to_string())
        }
        Commands::Panel { port } => panel::run(root, port),
        Commands::Apply => {
            let rep = apply::apply_all(root).map_err(|e| e.to_string())?;
            println!(
                "已应用并发布批次 {}: 直连 {} 节点, sing-box {} 节点 (xray_restarted={}, singbox_restarted={})",
                rep.generation_id,
                rep.direct_nodes,
                rep.singbox_nodes,
                rep.xray_restarted,
                rep.singbox_restarted
            );
            Ok(())
        }
        Commands::Status { json } => {
            let cfg = Config::load(root).map_err(|e| e.to_string())?;
            let snap = traffic::snapshot(root, &cfg).map_err(|e| e.to_string())?;
            if json {
                return admin::print_json(&admin::status_json(root, &cfg, &snap)?);
            }
            let sub_proto = if cfg.is_ip_mode() {
                format!(
                    "HTTP (明文 :{}，未配置域名，建议运行 ly 选择 6 绑定域名启用 HTTPS)",
                    cfg.server.http_fallback_port
                )
            } else if cfg.server.https_port == 443 {
                format!("HTTPS (https://{})", cfg.domain.trim())
            } else {
                format!(
                    "HTTPS (https://{}:{})",
                    cfg.domain.trim(),
                    cfg.server.https_port
                )
            };
            println!(
                "vps-sub-meter v{}\n域名/地址: {}\n订阅协议: {}\n网卡: {}\n上传: {}\n下载: {}\n总计: {} / {}\n下次重置: {} ({})",
                env!("CARGO_PKG_VERSION"),
                if cfg.is_ip_mode() {
                    &cfg.public_ip
                } else {
                    &cfg.domain
                },
                sub_proto,
                snap.interface,
                traffic::format_iec(snap.upload_bytes),
                traffic::format_iec(snap.download_bytes),
                traffic::format_iec(snap.total_used()),
                if snap.monthly_limit_bytes > 0 {
                    traffic::format_iec(snap.monthly_limit_bytes)
                } else {
                    "不限".into()
                },
                traffic::format_time_in_tz(snap.next_reset_ts, &cfg.traffic.display_tz),
                cfg.traffic.display_tz
            );
            let gen_meta = store::current_generation(root).ok().flatten();
            if let Some(meta) = &gen_meta {
                println!(
                    "当前订阅批次: {} ({} 个节点: {})",
                    meta.id,
                    meta.direct_nodes.len(),
                    meta.direct_nodes.join(", ")
                );
            }
            if cfg.argo.enabled {
                let argo_ips =
                    vps_sub_meter::render::client::load_argo_preferred_ips(root, &cfg).len();
                let argo_tunnel = cfg.argo.tunnel_domain.trim();
                let argo_sub = cfg.argo.sub_domain.trim();
                let argo_hosts = if argo_sub.is_empty() || argo_sub == argo_tunnel {
                    argo_tunnel.to_owned()
                } else {
                    format!("{argo_tunnel} / 订阅 {argo_sub}")
                };
                println!(
                    "Argo 隧道: 已启用 ({argo_hosts}，{argo_ips} 个优选 IP，{} 个 Argo 节点)",
                    gen_meta.as_ref().map_or(0, |meta| meta.argo_nodes.len())
                );
            }
            if apply::has_pending_changes(root, &cfg, gen_meta.as_ref()) {
                if gen_meta.is_some() {
                    println!(
                        "配置状态: 检测到主配置与当前已发布批次不一致，请运行 vsm apply 或在 ly 选择「8. 应用配置」生效"
                    );
                } else {
                    println!(
                        "配置状态: 尚未发布订阅批次，请运行 vsm apply 或在 ly 选择「8. 应用配置」生效"
                    );
                }
            }
            Ok(())
        }
        Commands::Sub(args) => admin::sub(root, args),
        Commands::Config { cmd } => admin::config(root, cmd),
        Commands::Token { cmd } => admin::token(root, cmd),
        Commands::Keys {
            cmd: KeysCmd::Rotate { yes },
        } => {
            if !yes {
                return Err("确认轮换全部节点密钥时需要 --yes".into());
            }
            let report = apply::rotate_node_keys(root).map_err(|e| e.to_string())?;
            println!("节点密钥已轮换并发布批次 {}", report.generation_id);
            Ok(())
        }
        Commands::Traffic { cmd } => {
            match cmd {
                TrafficCmd::Checkpoint => {
                    let s = traffic::checkpoint_current(root).map_err(|e| e.to_string())?;
                    println!(
                        "已保存流量账本: 上传 {}, 下载 {}",
                        traffic::format_iec(s.upload_bytes),
                        traffic::format_iec(s.download_bytes)
                    );
                }
                TrafficCmd::Set {
                    upload_gib,
                    download_gib,
                } => {
                    let gib = 1024.0 * 1024.0 * 1024.0;
                    if !upload_gib.is_finite()
                        || !download_gib.is_finite()
                        || upload_gib < 0.0
                        || download_gib < 0.0
                    {
                        return Err("流量必须是非负数字".into());
                    }
                    if upload_gib * gib >= u64::MAX as f64 || download_gib * gib >= u64::MAX as f64
                    {
                        return Err("流量数值过大".into());
                    }
                    let s = traffic::set_used_current(
                        root,
                        (upload_gib * gib).round() as u64,
                        (download_gib * gib).round() as u64,
                    )
                    .map_err(|e| e.to_string())?;
                    println!(
                        "已校准: 上传 {}, 下载 {}",
                        traffic::format_iec(s.upload_bytes),
                        traffic::format_iec(s.download_bytes)
                    );
                }
                TrafficCmd::Configure {
                    monthly_gib,
                    reset_day,
                    reset_time,
                    reset_tz,
                    display_tz,
                } => {
                    let changed = traffic::configure(root, |cfg| {
                        cfg.monthly_gb = monthly_gib.unwrap_or(cfg.monthly_gb);
                        cfg.reset_day = reset_day.unwrap_or(cfg.reset_day);
                        if let Some(value) = reset_time {
                            cfg.reset_time = value;
                        }
                        if let Some(value) = reset_tz {
                            cfg.reset_tz = value;
                        }
                        if let Some(value) = display_tz {
                            cfg.display_tz = value;
                        }
                    })
                    .map_err(|e| e.to_string())?;
                    println!(
                        "{}",
                        if changed {
                            "流量设置已生效"
                        } else {
                            "流量设置未修改"
                        }
                    );
                }
            }
            Ok(())
        }
    }
}
