//! The self-update and release tooling: `sbctl update`, `sbctl sing-box`,
//! `sbctl release`, `sbctl logs` and `sbctl uninstall`.

use crate::cli::args::{ApiCommand, KernelSource, LogUnit, ReleaseCommand, SingBoxCommand};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

pub(crate) fn sing_box(root: &Path, command: SingBoxCommand) -> ExitCode {
    let result = match command {
        SingBoxCommand::Version => return sing_box_version(root),
        SingBoxCommand::Status => return sing_box_status(root),
        SingBoxCommand::Api { command } => return sing_box_api(root, command),
        SingBoxCommand::Connections => return sing_box_connections(root),
        SingBoxCommand::Download { manifest, output } => sbctl::update::read_manifest(&manifest)
            .and_then(|manifest| sbctl::update::download_sing_box(&manifest, &output))
            .map(|_| format!("sing-box downloaded and verified: {}", output.display())),
        SingBoxCommand::Install { manifest, artifact } => sbctl::update::read_manifest(&manifest)
            .and_then(|manifest| {
                sbctl::update::install_signed_sing_box(
                    &sbctl::config::DeploymentStore::new(root),
                    &manifest,
                    &artifact,
                )
            })
            .map(|rollback| format!("sing-box installed; rollback point: {}", rollback.display())),
        SingBoxCommand::Update { manifest, artifact } => match manifest {
            Some(path) => {
                // 签名 manifest 流程：URL 与摘要全部固定并校验签名后才会使用。
                sbctl::update::read_manifest(&path)
                    .and_then(|manifest| {
                        let temporary = tempfile::NamedTempFile::new().map_err(|error| {
                            sbctl::update::UpdateError::DownloadFailed(
                                "sing-box",
                                error.to_string(),
                            )
                        })?;
                        // Holds the candidate path without an open write handle:
                        // Linux refuses to execute a file that is still open for
                        // writing (`Text file busy`, ETXTBSY).
                        let mut guard = None;
                        let candidate = match artifact {
                            Some(candidate) => {
                                sbctl::update::verify_sing_box_artifact(&manifest, &candidate)?;
                                candidate
                            }
                            None => {
                                let path = temporary.into_temp_path();
                                let candidate = path.to_path_buf();
                                sbctl::update::download_sing_box(&manifest, &candidate)?;
                                guard = Some(path);
                                candidate
                            }
                        };
                        let result = sbctl::update::apply_sing_box(
                            &sbctl::config::DeploymentStore::new(root),
                            &manifest,
                            &candidate,
                        );
                        drop(guard);
                        result
                    })
                    .map(|rollback| {
                        format!("sing-box updated; rollback point: {}", rollback.display())
                    })
            }
            None => update_sing_box_official(root, artifact.as_deref(), None),
        },
        SingBoxCommand::Fetch { source, version } => match source {
            KernelSource::Official => update_sing_box_official(root, None, version.as_deref()),
            KernelSource::Repository => {
                if version.is_some() {
                    eprintln!("本仓库原版二进制由签名 manifest 固定版本，不支持选择版本");
                    return ExitCode::from(2);
                }
                sbctl::update::fetch_latest_manifest().and_then(|manifest| {
                    let temporary = tempfile::NamedTempFile::new()?.into_temp_path();
                    sbctl::update::download_sing_box(&manifest, &temporary)?;
                    sbctl::update::install_signed_sing_box(
                        &sbctl::config::DeploymentStore::new(root),
                        &manifest,
                        &temporary,
                    )
                    .map(|path| format!("本仓库内核已安装；回滚点：{}", path.display()))
                })
            }
        },
        SingBoxCommand::Remove => sbctl::lifecycle::remove_managed_sing_box(root)
            .map(|_| "sing-box removed".to_owned())
            .map_err(sbctl::update::UpdateError::Operation),
    };
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("sing-box operation failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// Updates the managed sing-box kernel to the latest stable release from the
/// official SagerNet repository (github.com/SagerNet/sing-box), or installs a
/// locally supplied candidate artifact. Both paths run the same configuration
/// check and health-check rollback flow as the signed-manifest update.
fn update_sing_box_official(
    root: &Path,
    artifact: Option<&Path>,
    requested_version: Option<&str>,
) -> Result<String, sbctl::update::UpdateError> {
    let store = sbctl::config::DeploymentStore::new(root);
    let temporary = tempfile::NamedTempFile::new().map_err(|error| {
        sbctl::update::UpdateError::DownloadFailed("sing-box", error.to_string())
    })?;
    // Keeps the downloaded candidate on disk until the update finishes, while
    // holding no open write handle. On Linux a file that is still open for
    // writing cannot be executed (`Text file busy`, ETXTBSY), and the candidate
    // is executed for the pre-install `sing-box check`.
    let mut candidate_guard: Option<tempfile::TempPath> = None;
    let (candidate, version_note) = match artifact {
        Some(path) => (path.to_path_buf(), "本地 sing-box 候选".to_owned()),
        None => {
            let version = requested_version
                .map(str::to_owned)
                .map(Ok)
                .unwrap_or_else(sbctl::update::fetch_latest_official_sing_box_version)?;
            println!("官方版本：sing-box {version}，开始下载并校验…");
            sbctl::update::download_sing_box_official(&version, temporary.path())?;
            let path = temporary.into_temp_path();
            let candidate = path.to_path_buf();
            candidate_guard = Some(path);
            (candidate, format!("sing-box {version}（官方仓库）"))
        }
    };
    // The candidate is verified again here: it must run and pass a
    // `sing-box check` against the active server configuration before the
    // managed binary is replaced.
    let contents = fs::read(&candidate)?;
    let rollback = sbctl::update::install_candidate_sing_box(&store, &candidate, &contents)?;
    drop(candidate_guard);
    Ok(format!(
        "{version_note} 更新完成，已通过配置检查与服务健康检查；回滚点：{}",
        rollback.display()
    ))
}

/// `sbctl sing-box version`: the installed kernel's full version (patch
/// included) and, when the network allows, the official latest stable version
/// with an upgrade hint. Both are advisory — a VPS without network must still
/// learn what is running.
fn sing_box_version(root: &Path) -> ExitCode {
    use super::config::resolve_sing_box_bin;
    let Some(binary) = resolve_sing_box_bin(root, None) else {
        eprintln!("未找到 sing-box 内核（未安装或不在 PATH）；用 sbctl sing-box update 安装。");
        return ExitCode::from(2);
    };
    match sbctl::observe::kernel_version_string(&binary) {
        Some(version) => {
            println!("已安装内核: sing-box {version}（{}）", binary.display());
            match sbctl::update::fetch_latest_official_sing_box_version() {
                Ok(latest) if latest != version => {
                    println!("官方最新稳定版: sing-box {latest}");
                    println!(
                        "提示: 运行 sbctl sing-box update 升级（更新前会做配置检查与健康回滚）。"
                    );
                }
                Ok(latest) => println!("官方最新稳定版: sing-box {latest}（已是最新）"),
                Err(error) => println!("官方最新稳定版: 查询失败（{error}）"),
            }
            ExitCode::SUCCESS
        }
        None => {
            eprintln!(
                "无法从内核获取版本（{} version 失败或输出无法解析）。",
                binary.display()
            );
            ExitCode::from(2)
        }
    }
}

/// `sbctl sing-box status`: the data-plane unit's systemd facts.
fn sing_box_status(root: &Path) -> ExitCode {
    let observation = sbctl::observe::observe_sing_box_process(root, chrono::Utc::now());
    if let Some(error) = &observation.error {
        eprintln!("sing-box 状态查询失败: {error}");
        return ExitCode::from(2);
    }
    println!(
        "服务状态: {} / {}",
        observation.active_state.as_deref().unwrap_or("未知"),
        observation.sub_state.as_deref().unwrap_or("未知")
    );
    println!(
        "主进程 PID: {}   重启次数: {}   任务数: {}",
        observation
            .main_pid
            .map(|pid| pid.to_string())
            .as_deref()
            .unwrap_or("未知"),
        observation
            .restart_count
            .map(|count| count.to_string())
            .as_deref()
            .unwrap_or("未知"),
        observation
            .task_count
            .map(|count| count.to_string())
            .as_deref()
            .unwrap_or("未知"),
    );
    println!(
        "内存占用: {}   CPU 时间: {}",
        observation.memory_label().unwrap_or_else(|| "未知".into()),
        observation
            .cpu_time_label()
            .map(|duration| format!("{duration:?}"))
            .unwrap_or_else(|| "未知".into()),
    );
    println!(
        "本次启动: {}   已运行: {}",
        observation.active_enter.as_deref().unwrap_or("未知"),
        observation
            .uptime
            .map(humantime_like)
            .unwrap_or_else(|| "未知".into()),
    );
    // The cgroup figure above is what systemd accounted for; with the
    // observation endpoint on, sing-box can also report its own live heap.
    if let Ok(store) = sbctl::config::DeploymentStore::new(root).load()
        && let Some(api) = &store.server_clash_api
        && let Ok(bytes) = sbctl::observe::clash_api_memory(api)
    {
        println!("内核实时内存 (inuse): {bytes} 字节");
    }
    ExitCode::SUCCESS
}

/// A compact `1d 2h 3m 4s` duration label without a new dependency.
fn humantime_like(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {secs}s")
    } else {
        format!("{minutes}m {secs}s")
    }
}

/// `sbctl sing-box api enable|disable|status`: the loopback observation
/// endpoint, changed through the same check-and-rollback transaction the
/// configuration wizard uses.
fn sing_box_api(root: &Path, command: ApiCommand) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = match command {
        ApiCommand::Status => store.load().map(|config| (config, None)),
        ApiCommand::Enable { port } => store
            .load()
            .and_then(|mut config| {
                let api = match port {
                    Some(port) => sbctl::config::ServerClashApi {
                        port,
                        ..sbctl::config::generate_server_clash_api()?
                    },
                    None => sbctl::config::generate_server_clash_api()?,
                };
                config.server_clash_api = Some(api);
                config.validate()?;
                apply_and_restart(root, &store, &config)?;
                Ok(config)
            })
            .map(|config| (config, Some("启用"))),
        ApiCommand::Disable => store
            .load()
            .and_then(|mut config| {
                config.server_clash_api = None;
                config.validate()?;
                apply_and_restart(root, &store, &config)?;
                Ok(config)
            })
            .map(|config| (config, Some("关闭"))),
    };
    match result {
        Ok((config, action)) => {
            if let Some(action) = action {
                println!("sing-box 观测接口已{action}");
            }
            match &config.server_clash_api {
                None => println!("当前状态: 未启用（服务端配置保持生成原样）"),
                Some(api) => {
                    println!("监听: {}", api.listener());
                    println!(
                        "密钥: {}…（保存在 /etc/sbctl/config.toml，0600）",
                        &api.secret[..6.min(api.secret.len())]
                    );
                    match sbctl::observe::clash_api_memory(api) {
                        Ok(bytes) => println!("接口可达，当前内存: {bytes} 字节"),
                        Err(error) => println!("接口尚未响应: {error}（服务重启后生效）"),
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("sing-box 观测接口操作失败: {error}");
            ExitCode::from(2)
        }
    }
}

/// Writes one set of server configuration and restarts the data plane,
/// restoring the previous deployment through the transaction snapshot when the
/// restart does not come back healthy.
fn apply_and_restart(
    root: &Path,
    store: &sbctl::config::DeploymentStore,
    config: &sbctl::config::DeploymentConfig,
) -> Result<(), sbctl::config::ConfigError> {
    let binary = root.join("usr/local/bin/sing-box");
    let snapshot = sbctl::subscription::apply_config_transaction(
        store,
        config,
        binary.is_file().then_some(binary.as_path()),
    )
    .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
    if root == Path::new("/") {
        sbctl::lifecycle::prepare_daemon_prerequisites(
            root,
            config.subscription_mode == sbctl::config::SubscriptionMode::Direct,
        )
        .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
    }
    sbctl::lifecycle::prepare_daemon_storage(root, false)
        .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
    super::config::restart_services_with_rollback(root, || {
        let _ = sbctl::subscription::restore_config_transaction(store, &snapshot);
    })
}

/// `sbctl sing-box connections`: the live proxied connections, when the
/// observation endpoint is on.
fn sing_box_connections(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|config| {
        let api = config.server_clash_api.ok_or_else(|| {
            sbctl::config::ConfigError::StateContent(
                "观测接口未启用；运行 sbctl sing-box api enable".to_owned(),
            )
        })?;
        sbctl::observe::clash_api_request(&api, "/connections", std::time::Duration::from_secs(3))
            .map_err(sbctl::config::ConfigError::StateContent)
    });
    match result {
        Ok(value) => {
            let lines = sbctl::observe::describe_connections(&value);
            println!("活跃连接: {}", lines.len());
            for line in lines {
                println!("  {line}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("连接列表读取失败: {error}");
            ExitCode::from(2)
        }
    }
}

/// `sbctl logs`: the journalctl view the menu already offers, as a first-class
/// verb so it can be scripted and followed.
pub(crate) fn logs(root: &Path, unit: LogUnit, lines: u32, follow: bool) -> ExitCode {
    let unit = match unit {
        LogUnit::SingBox => "sing-box",
        LogUnit::Sbctl => "sbctl",
        LogUnit::All => "all",
    };
    let args = sbctl::observe::journalctl_args(unit, lines, follow);
    match sbctl::observe::run_journalctl(root, &args) {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("journalctl 退出码 {status}");
            ExitCode::from(2)
        }
        Err(message) => {
            eprintln!("查看日志失败: {message}（需要 systemd/journald）");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn release(command: ReleaseCommand) -> ExitCode {
    let result: Result<String, String> = match command {
        ReleaseCommand::Sign {
            manifest,
            private_key,
            output,
        } => sbctl::release::sign_manifest_file(&manifest, &private_key, &output)
            .map(|_| format!("signed release manifest written to {}", output.display()))
            .map_err(|error| error.to_string()),
        ReleaseCommand::Verify { manifest } => sbctl::release::verify_manifest(&manifest)
            .map(|_| "release manifest verified against the built-in public key".to_owned())
            .map_err(|error| error.to_string()),
        ReleaseCommand::Keygen { output } => (|| {
            let (public, secret) = sbctl::release::generate_keypair();
            let secret_path = output.join("sbctl-release-secret.hex");
            sbctl::release::write_secret_file(&secret_path, &secret)
                .map_err(|error| error.to_string())?;
            let hex =
                |bytes: &[u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
            Ok(format!(
                "secret seed written to {} (0600)\npublic key hex: {}\npublic key PEM:\n{}",
                secret_path.display(),
                hex(&public),
                sbctl::release::public_key_pem(&public)
            ))
        })(),
    };
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("release operation failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn uninstall(root: &Path, purge: bool) -> ExitCode {
    match sbctl::lifecycle::uninstall(root, purge) {
        Ok(Some(backup)) => {
            println!(
                "sbctl services and binaries removed; backup preserved at {}",
                backup.display()
            );
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("sbctl services and binaries removed; persistent sbctl data purged");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("uninstall failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn update(
    root: &Path,
    check: bool,
    manifest_path: Option<&Path>,
    sbctl_artifact: Option<&Path>,
    sing_box_artifact: Option<&Path>,
) -> ExitCode {
    let result = update_impl(
        root,
        check,
        manifest_path,
        sbctl_artifact,
        sing_box_artifact,
    );
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("update failed: {error}");
            ExitCode::from(2)
        }
    }
}

fn update_impl(
    root: &Path,
    check: bool,
    manifest_path: Option<&Path>,
    sbctl_artifact: Option<&Path>,
    sing_box_artifact: Option<&Path>,
) -> Result<String, sbctl::update::UpdateError> {
    let manifest = match manifest_path {
        Some(path) => sbctl::update::read_manifest(path)?,
        None => sbctl::update::fetch_latest_manifest()?,
    };
    if check {
        return Ok(format!(
            "update check completed without downloading or changing the host\n{}",
            sbctl::update::available_versions(&manifest)
        ));
    }
    let sbctl_download = tempfile::NamedTempFile::new()
        .map_err(|error| sbctl::update::UpdateError::DownloadFailed("sbctl", error.to_string()))?;
    let sing_box_download = tempfile::NamedTempFile::new().map_err(|error| {
        sbctl::update::UpdateError::DownloadFailed("sing-box", error.to_string())
    })?;
    // Keep both candidates on disk without open write handles: the update runs
    // them for the pre-install checks, and Linux refuses to execute a file that
    // is still open for writing (`Text file busy`, ETXTBSY).
    let mut sbctl_guard = None;
    let mut sing_box_guard = None;
    let sbctl_artifact = match sbctl_artifact {
        Some(path) => path.to_path_buf(),
        None => {
            let path = sbctl_download.into_temp_path();
            let artifact = path.to_path_buf();
            sbctl::update::download_sbctl(&manifest, &artifact)?;
            sbctl_guard = Some(path);
            artifact
        }
    };
    let sing_box_artifact = match sing_box_artifact {
        Some(path) => path.to_path_buf(),
        None => {
            let path = sing_box_download.into_temp_path();
            let artifact = path.to_path_buf();
            sbctl::update::download_sing_box(&manifest, &artifact)?;
            sing_box_guard = Some(path);
            artifact
        }
    };
    let rollback = sbctl::update::apply(
        &sbctl::config::DeploymentStore::new(root),
        &manifest,
        &sbctl_artifact,
        &sing_box_artifact,
    )?;
    drop(sbctl_guard);
    drop(sing_box_guard);
    Ok(format!(
        "update completed after verified validation and service health checks\nrollback point: {}",
        rollback.display()
    ))
}
