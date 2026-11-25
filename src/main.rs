//! `vps-sub-meter` (`vsm` / `ly`) CLI entrypoint.

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vps_sub_meter::{
    apply, config::Config, menu, prompt::ConsoleIo, qr, release, secrets::Secrets, serve, traffic,
};

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
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 打开交互管理菜单 (等价于直接运行 `ly` 或 `vsm`)
    Menu,
    /// 启动 HTTPS / ACME / Argo 订阅守护进程
    Serve,
    /// 校验配置与模板、生成新订阅批次并平滑重启变动的服务
    Apply,
    /// 查看当前运行状态、节点摘要与实时流量
    Status,
    /// 打印订阅链接 (加 --qr 输出终端二维码)
    Sub {
        #[arg(long)]
        qr: bool,
    },
    /// 流量账期操作
    Traffic {
        #[command(subcommand)]
        cmd: TrafficCmd,
    },
    /// 发布清单签名与校验
    Release {
        #[command(subcommand)]
        cmd: ReleaseCmd,
    },
}

#[derive(Subcommand)]
enum TrafficCmd {
    /// 将内存累加刷新到磁盘
    Checkpoint,
    /// 手动校准已用上传和下载 GiB
    Set { upload_gib: f64, download_gib: f64 },
}

#[derive(Subcommand)]
enum ReleaseCmd {
    Sign {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        private_key: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Verify {
        #[arg(long)]
        manifest: PathBuf,
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

fn run(root: &Path, command: Option<Commands>) -> Result<(), String> {
    match command.unwrap_or(Commands::Menu) {
        Commands::Menu => {
            let mut io = ConsoleIo;
            menu::run(root, &mut io);
            Ok(())
        }
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
        Commands::Status => {
            let cfg = Config::load(root).map_err(|e| e.to_string())?;
            let snap = traffic::snapshot(root, &cfg).map_err(|e| e.to_string())?;
            println!(
                "vps-sub-meter v{}\n域名/地址: {}\n网卡: {}\n上传: {}\n下载: {}\n总计: {} / {}\n下次重置: {} ({})",
                env!("CARGO_PKG_VERSION"),
                if cfg.is_ip_mode() {
                    &cfg.public_ip
                } else {
                    &cfg.domain
                },
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
            if let Ok(Some(gen_meta)) = apply::current_generation(root) {
                println!(
                    "当前订阅批次: {} ({} 个节点: {})",
                    gen_meta.id,
                    gen_meta.direct_nodes.len(),
                    gen_meta.direct_nodes.join(", ")
                );
            }
            Ok(())
        }
        Commands::Sub { qr: show_qr } => {
            let cfg = Config::load(root).map_err(|e| e.to_string())?;
            let sec = Secrets::load_or_default(root).map_err(|e| e.to_string())?;
            let tok = sec
                .primary_token()
                .ok_or_else(|| "尚未生成订阅凭据".to_owned())?;
            let base = cfg.subscription_base_url(tok);
            println!("看板:         {base}/index");
            println!("Clash/Mihomo: {base}/clash.yaml");
            println!("sing-box:     {base}/sing-box.json");
            println!("Shadowrocket: {base}/shadowrocket.txt");
            println!("URI:          {base}/uri.txt");
            if show_qr {
                let ansi = qr::render_ansi(&format!("{base}/clash.yaml"))?;
                print!("{ansi}");
            }
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
        Commands::Release { cmd } => match cmd {
            ReleaseCmd::Sign {
                manifest,
                private_key,
                output,
            } => release::sign_manifest_file(&manifest, &private_key, &output)
                .map_err(|e| e.to_string()),
            ReleaseCmd::Verify { manifest } => {
                let m = release::verify_manifest_file(&manifest).map_err(|e| e.to_string())?;
                println!("清单签名有效: vps-sub-meter {}", m.vps_sub_meter.version);
                Ok(())
            }
        },
    }
}
