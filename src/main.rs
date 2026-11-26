//! `vps-sub-meter` (`vsm`) CLI entrypoint.

mod admin;

use clap::{Parser, Subcommand};
use serde_json::json;
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
                let generation = store::current_generation(root).map_err(|e| e.to_string())?;
                return admin::print_json(&json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "address": cfg.primary_host(),
                    "https_enabled": !cfg.is_ip_mode(),
                    "interface": snap.interface,
                    "traffic": {
                        "upload_bytes": snap.upload_bytes,
                        "download_bytes": snap.download_bytes,
                        "total_used_bytes": snap.total_used(),
                        "monthly_limit_bytes": snap.monthly_limit_bytes,
                        "next_reset_ts": snap.next_reset_ts,
                        "display_tz": cfg.traffic.display_tz,
                    },
                    "generation": generation,
                    "argo_enabled": cfg.argo.enabled,
                }));
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
            if let Ok(Some(gen_meta)) = store::current_generation(root) {
                println!(
                    "当前订阅批次: {} ({} 个节点: {})",
                    gen_meta.id,
                    gen_meta.direct_nodes.len(),
                    gen_meta.direct_nodes.join(", ")
                );
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
            let cfg = Config::load(root).map_err(|e| e.to_string())?;
            match cmd {
                TrafficCmd::Checkpoint => {
                    let s = traffic::checkpoint(root, &cfg).map_err(|e| e.to_string())?;
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
                    if !upload_gib.is_finite()
                        || !download_gib.is_finite()
                        || upload_gib < 0.0
                        || download_gib < 0.0
                    {
                        return Err("流量必须是非负数字".into());
                    }
                    let gib = 1024.0 * 1024.0 * 1024.0;
                    let s = traffic::set_used(
                        root,
                        &cfg,
                        (upload_gib * gib) as u64,
                        (download_gib * gib) as u64,
                    )
                    .map_err(|e| e.to_string())?;
                    println!(
                        "已校准: 上传 {}, 下载 {}",
                        traffic::format_iec(s.upload_bytes),
                        traffic::format_iec(s.download_bytes)
                    );
                }
            }
            Ok(())
        }
    }
}
