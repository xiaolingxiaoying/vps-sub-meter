//! The deployment configuration surface: `sbctl config`, its override
//! templates sub-tree, the interactive wizard, the regeneration transaction and
//! the service restart with rollback that both the wizard and credential
//! rotation rely on.

use crate::cli::args::{CliOverrideClearTarget, CliOverrideTarget, ConfigCommand, OverrideCommand};
use crate::cli::prompt::{ConsolePrompts, protocol_ports};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) fn restart(root: &Path, sing_box_bin: Option<PathBuf>) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|config| {
        let binary = sing_box_bin.unwrap_or_else(|| root.join("usr/local/bin/sing-box"));
        let server =
            std::fs::read_to_string(root.join("var/lib/sbctl/artifacts/sing-box-server.json"))
                .map_err(sbctl::config::ConfigError::Storage)?;
        sbctl::subscription::check_sing_box_config(&binary, &server)
            .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
        sbctl::lifecycle::restart_services(root)
            .map_err(sbctl::config::ConfigError::StateContent)?;
        Ok(config)
    });
    match result {
        Ok(_) => {
            println!("sing-box and sbctl services restarted");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("restart failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// Manages client subscription overrides and the local server override
/// (`sbctl config override ...`). Client files merge into generated artifacts;
/// the server file merges into the sing-box configuration this host runs.
fn run_config_override(root: &Path, command: OverrideCommand) -> ExitCode {
    let targets = [
        override_target_spec(CliOverrideTarget::SingBox),
        override_target_spec(CliOverrideTarget::Clash),
        override_target_spec(CliOverrideTarget::Server),
    ];
    match command {
        OverrideCommand::Show => {
            for spec in targets {
                if let Err(error) = show_override_target(root, spec) {
                    eprintln!("override 列表读取失败：{error}");
                    return ExitCode::from(2);
                }
            }
            println!(
                "\n合并语义：对象递归合并；默认数组整体替换；rules 默认前插，也可在每个文件中用 rules_mode = prepend|append|replace 覆盖。"
            );
            println!(
                "客户端 sing-box 的 outbounds 按 tag 合并；Clash 的 proxies、proxy-groups、rule-providers 按 name 合并。"
            );
            println!(
                "每个目标先应用基础文件，再按文件名字典序合并 drop-in 层；服务端覆写不得更改入站凭据字段。"
            );
            println!(
                "影响工件：sing-box-server.json、sing-box-full.json、sing-box-<版本>.json、clash.yaml、clash-1.18.yaml。sing-box.json 与 URI 格式不受客户端覆写影响。"
            );
            ExitCode::SUCCESS
        }
        OverrideCommand::Validate { sing_box_bin } => {
            if let Err(error) = sbctl::override_template::Overrides::load(root) {
                eprintln!("override 校验失败：{error}");
                return ExitCode::from(2);
            }
            let store = sbctl::config::DeploymentStore::new(root);
            let config = match store.load() {
                Ok(config) => config,
                Err(sbctl::config::ConfigError::Missing) => {
                    println!("override 模板结构有效（部署尚未初始化，跳过合并后真核 check）。");
                    return ExitCode::SUCCESS;
                }
                Err(error) => {
                    eprintln!("override 校验失败：无法读取部署配置：{error}");
                    return ExitCode::from(2);
                }
            };
            let artifacts = match sbctl::subscription::generated_artifacts(&config, root) {
                Ok(artifacts) => artifacts,
                Err(error) => {
                    eprintln!("override 合并失败：{error}");
                    return ExitCode::from(2);
                }
            };
            let Some(binary) = resolve_sing_box_bin(root, sing_box_bin) else {
                println!(
                    "override 模板和合并结果有效；未找到 sing-box 内核（用 --sing-box-bin 指定），跳过服务端与客户端真核 check。"
                );
                return ExitCode::SUCCESS;
            };
            let full_name = sbctl::subscription::SubscriptionFormat::SingBoxFull
                .artifact_name()
                .into_owned();
            for (name, label) in [
                ("sing-box-server.json", "服务端配置"),
                (full_name.as_str(), "客户端 sing-box-full 配置"),
            ] {
                let Some((_, merged)) = artifacts.iter().find(|(artifact, _)| artifact == name)
                else {
                    eprintln!("override 校验失败：缺少 {name} 工件");
                    return ExitCode::from(2);
                };
                if let Err(error) = sbctl::subscription::check_sing_box_config(&binary, merged) {
                    eprintln!(
                        "{label}真核 check 失败（内核 {}）：{error}",
                        binary.display()
                    );
                    return ExitCode::from(2);
                }
            }
            println!(
                "override 模板有效；合并后的服务端和客户端 sing-box 配置均通过真核 check（{}）。",
                binary.display()
            );
            ExitCode::SUCCESS
        }
        OverrideCommand::Edit {
            target,
            layer,
            sing_box_bin,
        } => {
            let spec = override_target_spec(target);
            let (path, sample) = match override_edit_path(root, spec, layer.as_deref()) {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("override 编辑失败：{error}");
                    return ExitCode::from(2);
                }
            };
            if let Err(error) = prepare_override_edit_path(&path, sample) {
                eprintln!("override 编辑失败：{error}");
                return ExitCode::from(2);
            }
            let prior = match fs::read(&path) {
                Ok(prior) => prior,
                Err(error) => {
                    eprintln!("无法备份覆写文件：{error}");
                    return ExitCode::from(2);
                }
            };
            let result = (|| {
                match crate::cli::editor::run_editor(
                    &crate::cli::editor::editor_candidates(),
                    &path,
                ) {
                    Ok(status) if status.success() => {}
                    Ok(status) => {
                        eprintln!("编辑器退出码 {status}");
                        return ExitCode::from(2);
                    }
                    Err(message) => {
                        eprintln!("{message}");
                        return ExitCode::from(2);
                    }
                }
                if let Err(error) = sbctl::override_template::Overrides::load(root) {
                    eprintln!("override 校验失败：{error}");
                    return ExitCode::from(2);
                }
                let store = sbctl::config::DeploymentStore::new(root);
                if root == Path::new("/") || root.join("var/lib/sbctl/ownership").is_file() {
                    match store.load() {
                        Ok(config) => commit_config_change(root, &store, &config, sing_box_bin),
                        Err(error) => {
                            eprintln!("读取部署失败：{error}");
                            ExitCode::from(2)
                        }
                    }
                } else {
                    regenerate(root, sing_box_bin)
                }
            })();
            if result != ExitCode::SUCCESS
                && let Err(error) = fs::write(&path, prior)
            {
                eprintln!("覆写文件恢复失败：{error}");
            }
            result
        }
        OverrideCommand::Clear { target } => {
            let selected = match target {
                None => vec![CliOverrideTarget::SingBox, CliOverrideTarget::Clash],
                Some(CliOverrideClearTarget::SingBox) => vec![CliOverrideTarget::SingBox],
                Some(CliOverrideClearTarget::Clash) => vec![CliOverrideTarget::Clash],
                Some(CliOverrideClearTarget::Server) => vec![CliOverrideTarget::Server],
                Some(CliOverrideClearTarget::All) => {
                    targets.iter().map(|spec| spec.target).collect()
                }
            };
            clear_override_targets(root, &selected)
        }
    }
}

#[derive(Clone, Copy)]
struct OverrideTargetSpec {
    target: CliOverrideTarget,
    label: &'static str,
    base_relative: &'static str,
    directory_relative: &'static str,
    extension: &'static str,
    base_sample: &'static str,
    layer_sample: &'static str,
}

fn override_target_spec(target: CliOverrideTarget) -> OverrideTargetSpec {
    use sbctl::override_template::{
        CLASH_OVERRIDE_DIRECTORY, CLASH_OVERRIDE_RELATIVE_PATH, SING_BOX_OVERRIDE_DIRECTORY,
        SING_BOX_OVERRIDE_RELATIVE_PATH, SING_BOX_SERVER_OVERRIDE_DIRECTORY,
        SING_BOX_SERVER_OVERRIDE_RELATIVE_PATH,
    };

    match target {
        CliOverrideTarget::SingBox => OverrideTargetSpec {
            target,
            label: "客户端 sing-box",
            base_relative: SING_BOX_OVERRIDE_RELATIVE_PATH,
            directory_relative: SING_BOX_OVERRIDE_DIRECTORY,
            extension: ".json",
            base_sample: "{\n  \"log\": {\"level\": \"warn\"},\n  \"route\": {\n    \"rules\": [\n      {\"domain_suffix\": [\"example.com\"], \"outbound\": \"节点选择\"}\n    ]\n  }\n}\n",
            layer_sample: "{}\n",
        },
        CliOverrideTarget::Clash => OverrideTargetSpec {
            target,
            label: "客户端 Clash",
            base_relative: CLASH_OVERRIDE_RELATIVE_PATH,
            directory_relative: CLASH_OVERRIDE_DIRECTORY,
            extension: ".yaml",
            base_sample: "# 键名为 rules 的数组会前插到生成规则之前。\nrules:\n  - DOMAIN-SUFFIX,example.com,节点选择\n",
            layer_sample: "{}\n",
        },
        CliOverrideTarget::Server => OverrideTargetSpec {
            target,
            label: "服务端 sing-box",
            base_relative: SING_BOX_SERVER_OVERRIDE_RELATIVE_PATH,
            directory_relative: SING_BOX_SERVER_OVERRIDE_DIRECTORY,
            extension: ".json",
            base_sample: "{\n  \"log\": {\"level\": \"warn\"}\n}\n",
            layer_sample: "{}\n",
        },
    }
}

fn override_layer_files(directory: &Path, extension: &str) -> std::io::Result<Vec<PathBuf>> {
    let directory_metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "override layer path is not a regular directory: {}",
                directory.display()
            ),
        ));
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name
            .to_str()
            .is_some_and(|name| !name.starts_with('.') && name.ends_with(extension))
        {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn show_override_target(root: &Path, spec: OverrideTargetSpec) -> std::io::Result<()> {
    let base = root.join(spec.base_relative);
    let base_status = if base.is_file() {
        "已启用"
    } else {
        "未创建"
    };
    println!("{}:", spec.label);
    println!("  {}  [{base_status}]", base.display());

    let directory = root.join(spec.directory_relative);
    let layers = override_layer_files(&directory, spec.extension)?;
    if layers.is_empty() {
        println!(
            "  {}  [没有生效的 {} drop-in 层]",
            directory.display(),
            spec.extension
        );
    } else {
        for layer in layers {
            println!("  {}  [已启用]", layer.display());
        }
    }
    Ok(())
}

fn override_edit_path(
    root: &Path,
    spec: OverrideTargetSpec,
    layer: Option<&str>,
) -> Result<(PathBuf, &'static str), String> {
    if let Some(layer) = layer {
        let stem = layer
            .strip_suffix(spec.extension)
            .filter(|stem| !stem.is_empty());
        if layer.starts_with('.')
            || stem.is_none()
            || !layer
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err(format!(
                "层文件名必须是单一文件名，且以 {} 结尾",
                spec.extension
            ));
        }
        Ok((
            root.join(spec.directory_relative).join(layer),
            spec.layer_sample,
        ))
    } else {
        Ok((root.join(spec.base_relative), spec.base_sample))
    }
}

fn prepare_override_edit_path(path: &Path, sample: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("拒绝编辑符号链接：{}", path.display()));
        }
        Ok(metadata) if metadata.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        Ok(_) => return Err(format!("覆写路径不是常规文件：{}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("读取 {} 失败：{error}", path.display())),
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("覆写路径没有父目录：{}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 {} 失败：{error}", parent.display()))?;
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(sample.as_bytes()))
        .map_err(|error| format!("创建 {} 失败：{error}", path.display()))
}

fn clear_override_targets(root: &Path, targets: &[CliOverrideTarget]) -> ExitCode {
    let overrides_dir = root.join("etc/sbctl/overrides");
    let mut sources = Vec::new();
    for target in targets {
        let spec = override_target_spec(*target);
        let base = root.join(spec.base_relative);
        match fs::symlink_metadata(&base) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                sources.push(base);
            }
            Ok(_) => {
                eprintln!("override 清理失败：基础路径不是文件：{}", base.display());
                return ExitCode::from(2);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!("override 清理失败：读取 {} 失败：{error}", base.display());
                return ExitCode::from(2);
            }
        }
        match override_layer_sources_for_clear(&root.join(spec.directory_relative), spec.extension)
        {
            Ok(mut layers) => sources.append(&mut layers),
            Err(error) => {
                eprintln!(
                    "override 清理失败：读取 {} 失败：{error}",
                    spec.directory_relative
                );
                return ExitCode::from(2);
            }
        }
    }
    sources.sort();
    sources.dedup();

    let backup = if sources.is_empty() {
        None
    } else {
        match tempfile::Builder::new()
            .prefix(".override-clear-")
            .tempdir_in(&overrides_dir)
        {
            Ok(backup) => Some(backup),
            Err(error) => {
                eprintln!("override 清理失败：无法创建回滚目录：{error}");
                return ExitCode::from(2);
            }
        }
    };

    let mut moved = Vec::new();
    let mut move_error = None;
    if let Some(backup) = &backup {
        for source in sources {
            let relative = match source.strip_prefix(&overrides_dir) {
                Ok(relative) => relative,
                Err(error) => {
                    move_error = Some(format!("覆写路径超出根目录：{error}"));
                    break;
                }
            };
            let backup_path = backup.path().join(relative);
            if let Some(parent) = backup_path.parent()
                && let Err(error) = fs::create_dir_all(parent)
            {
                move_error = Some(format!("创建回滚路径失败：{error}"));
                break;
            }
            if let Err(error) = fs::rename(&source, &backup_path) {
                move_error = Some(format!("移走 {} 失败：{error}", source.display()));
                break;
            }
            moved.push((source, backup_path));
        }
    }
    if let Some(error) = move_error {
        eprintln!("override 清理失败：{error}");
        return restore_clear_backup(backup, &moved, ExitCode::from(2));
    }

    if !moved.is_empty() {
        println!("已暂存 {} 个覆写文件，正在重新生成并验证……", moved.len());
    } else {
        println!("未发现已启用的覆写文件，正在检查并重新生成工件……");
    }
    restore_clear_backup(backup, &moved, regenerate(root, None))
}

fn override_layer_sources_for_clear(
    directory: &Path,
    extension: &str,
) -> std::io::Result<Vec<PathBuf>> {
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Ok(vec![directory.to_path_buf()])
        }
        Ok(_) => override_layer_files(directory, extension),
    }
}

fn restore_clear_backup(
    backup: Option<tempfile::TempDir>,
    moved: &[(PathBuf, PathBuf)],
    result: ExitCode,
) -> ExitCode {
    if result == ExitCode::SUCCESS || moved.is_empty() {
        drop(backup);
        return result;
    }
    let Some(backup) = backup else {
        eprintln!("override 清理回滚失败：找不到暂存目录");
        return ExitCode::from(2);
    };
    let mut restore_errors = Vec::new();
    for (original, saved) in moved.iter().rev() {
        if let Some(parent) = original.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            restore_errors.push(format!("创建 {} 失败：{error}", parent.display()));
            continue;
        }
        if let Err(error) = fs::rename(saved, original) {
            restore_errors.push(format!("恢复 {} 失败：{error}", original.display()));
        }
    }
    if restore_errors.is_empty() {
        drop(backup);
        return result;
    }
    let backup_path = backup.keep();
    eprintln!(
        "override 清理回滚未完成；剩余备份保存在 {}：{}",
        backup_path.display(),
        restore_errors.join("；")
    );
    ExitCode::from(2)
}

/// Resolves the sing-box binary for an override validation: an explicit path,
/// the managed installation path, or a `sing-box` available on `PATH`.
pub(crate) fn resolve_sing_box_bin(root: &Path, explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(binary) = explicit {
        return Some(binary);
    }
    let managed = root.join("usr/local/bin/sing-box");
    if managed.is_file() {
        return Some(managed);
    }
    let on_path = PathBuf::from(if cfg!(windows) {
        "sing-box.exe"
    } else {
        "sing-box"
    });
    std::process::Command::new(&on_path)
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()
        .map(|_| on_path)
}

pub(crate) fn regenerate(root: &Path, sing_box_bin: Option<PathBuf>) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    // Regenerate always re-syncs the active sing-box configuration on a live
    // host (root "/"), even for an installation that never wrote the ownership
    // marker (a `--no-start` install). Fixture roots are left untouched.
    let update_active_config =
        root == std::path::Path::new("/") || root.join("var/lib/sbctl/ownership").is_file();
    let binary = sing_box_bin.unwrap_or_else(|| root.join("usr/local/bin/sing-box"));
    let result = sbctl::subscription::regenerate_current(
        &store,
        Some(binary.as_path()),
        update_active_config,
    )
    .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))
    .and_then(|config| {
        let direct = config.subscription_mode == sbctl::config::SubscriptionMode::Direct;
        // Always restore daemon-storage permissions after a live regeneration so
        // rewritten artifacts and the active sing-box configuration stay readable
        // by their service accounts, even for an installation that never wrote the
        // ownership marker (a `--no-start` install) (issue #5). `prepare_daemon_storage`
        // is a no-op for non-live helper roots.
        sbctl::lifecycle::prepare_daemon_storage(root, direct)
            .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
        Ok(())
    });
    match result {
        Ok(()) => {
            println!("canonical protocol artifacts regenerated and validated");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("regenerate failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn run_config(root: &Path, command: ConfigCommand) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = match command {
        ConfigCommand::Preview { format } => return configuration_artifact(root, &format, None),
        ConfigCommand::Export { format, output } => return configuration_artifact(root, &format, Some(&output)),
        ConfigCommand::Wizard { sing_box_bin } => return run_config_wizard(root, sing_box_bin),
        ConfigCommand::Override { command } => return run_config_override(root, command),
        ConfigCommand::Init {
            mode,
            subscription_host,
            proxy_host,
            http_port,
            listen_port,
            interface,
            protocols,
            reality_decoy_sni,
            protocol_sni,
            monthly_traffic_limit,
            accounting_policy,
            accounting_timezone,
            client_display_timezone,
            anchored_reset_at,
            sing_box_bin,
            vless_port,
            vmess_port,
            hysteria2_port,
            tuic_port,
            anytls_port,
        } => {
            let interface = interface.map(Ok).unwrap_or_else(|| {
                sbctl::traffic::detect_default_route_interface(root).map_err(|error| {
                    sbctl::config::ConfigError::StateContent(format!(
                        "could not detect a default-route interface ({error}); specify --interface"
                    ))
                })
            });
            interface.and_then(|interface| {
                let mut config = sbctl::config::DeploymentConfig::new_with_ports(
                    mode.into(),
                    subscription_host,
                    proxy_host,
                    http_port,
                    interface,
                    protocols.into_iter().map(Into::into).collect(),
                    reality_decoy_sni,
                    protocol_ports(vless_port, vmess_port, hysteria2_port, tuic_port, anytls_port),
                )?;
                config.protocol_sni = protocol_sni;
                config.monthly_traffic_limit = monthly_traffic_limit;
                config.accounting_policy = accounting_policy.into();
                if let Some(timezone) = accounting_timezone {
                    config.accounting_timezone = timezone;
                }
                if let Some(timezone) = client_display_timezone {
                    config.client_display_timezone = timezone;
                }
                config.anchored_reset_at = anchored_reset_at;
                // A missing --listen-port keeps the 2080 loopback default that a
                // fresh external-proxy deployment already carries; an explicit
                // value (or an explicit non-external-proxy mode) decides below.
                if listen_port.is_some() {
                    config.subscription_listen_port = listen_port;
                }
                config.validate()?;
                if let Some(port) = config.subscription_listen_port {
                    sbctl::subscription::ensure_external_proxy_listener_available(port)
                        .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
                }
                let generated_artifacts = if config
                    .enabled_protocols
                    .iter()
                    .any(sbctl::config::ManagedProtocol::has_generated_subscription_artifacts)
                {
                    sbctl::subscription::generated_artifacts(&config, root).map_err(|error| {
                        sbctl::config::ConfigError::StateContent(error.to_string())
                    })?
                } else {
                    Vec::new()
                };
                let artifact_references = generated_artifacts
                    .iter()
                    .map(|(name, contents)| (name.clone(), contents.as_bytes()))
                    .collect::<Vec<_>>();
                let requires_sing_box_check = config.enabled_protocols.iter().any(|protocol| {
                    matches!(
                        protocol,
                        sbctl::config::ManagedProtocol::VmessWebsocket
                            | sbctl::config::ManagedProtocol::Hysteria2
                            | sbctl::config::ManagedProtocol::Tuic
                            | sbctl::config::ManagedProtocol::Anytls
                    )
                });
                if requires_sing_box_check && sing_box_bin.is_none() {
                    return Err(sbctl::config::ConfigError::InvalidValue(
                        "certificate-based Managed protocols require --sing-box-bin for configuration validation",
                    ));
                }
                if let Some(sing_box_bin) = sing_box_bin {
                    let server_config = generated_artifacts
                        .iter()
                        .find(|(name, _)| *name == "sing-box-server.json")
                        .map(|(_, contents)| contents)
                        .ok_or(sbctl::config::ConfigError::InvalidValue(
                            "no generated sing-box server configuration is available to check",
                        ))?;
                    sbctl::subscription::check_sing_box_config(&sing_box_bin, server_config)
                        .map_err(|error| {
                            sbctl::config::ConfigError::StateContent(error.to_string())
                        })?;
                }
                store.initialize_with_artifacts(&config, &artifact_references)
            })
        }
        .map(|_| "deployment configuration initialized".to_owned()),
        ConfigCommand::SwitchMode { mode, listen_port } => store.load().and_then(|mut config| {
            config.subscription_mode = mode.into();
            config.subscription_listen_port = listen_port;
            if config.subscription_mode != sbctl::config::SubscriptionMode::IpFallback {
                config.http_port = None;
            }
            config.validate()?;
            if let Some(port) = config.subscription_listen_port {
                sbctl::subscription::ensure_external_proxy_listener_available(port)
                    .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
            }
            store.replace(&config)
        })
        .map(|_| "subscription mode changed".to_owned()),
        ConfigCommand::Show => store.load().map(|config| config.summary()),
        ConfigCommand::Validate => store
            .load()
            .map(|_| "deployment configuration is valid".to_owned()),
    };
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("configuration failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn run_config_wizard(root: &Path, sing_box_bin: Option<PathBuf>) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let existing = match store.load() {
        Ok(config) => Some(config),
        Err(sbctl::config::ConfigError::Missing) => None,
        Err(error) => {
            eprintln!("configuration wizard failed: {error}");
            return ExitCode::from(2);
        }
    };
    let default_interface = if existing.is_none() {
        sbctl::traffic::detect_default_route_interface(root).ok()
    } else {
        None
    };
    let mut prompts = ConsolePrompts;
    let outcome = match sbctl::wizard::run(existing.as_ref(), default_interface, &mut prompts) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("configuration wizard failed: {error}");
            return ExitCode::from(2);
        }
    };
    match outcome {
        sbctl::wizard::WizardOutcome::Cancelled => {
            println!("configuration wizard cancelled; the existing deployment is unchanged");
            ExitCode::SUCCESS
        }
        sbctl::wizard::WizardOutcome::Unchanged => {
            println!("deployment configuration is unchanged");
            ExitCode::SUCCESS
        }
        sbctl::wizard::WizardOutcome::Changed(config) => {
            commit_config_change(root, &store, &config, sing_box_bin)
        }
    }
}

/// Commits a confirmed wizard configuration through the artifact/check/health
/// transaction. A fresh deployment initializes artifacts and configuration;
/// an existing deployment atomically replaces the changed files, restarts the
/// managed services, and re-establishes accounting state when the schedule or
/// interface changed. Any failure restores the previous known-good deployment.
pub(crate) fn commit_config_change(
    root: &Path,
    store: &sbctl::config::DeploymentStore,
    new: &sbctl::config::DeploymentConfig,
    sing_box_bin: Option<PathBuf>,
) -> ExitCode {
    let existing = match store.load() {
        Ok(config) => Some(config),
        Err(sbctl::config::ConfigError::Missing) => None,
        Err(error) => {
            eprintln!("configuration wizard failed: {error}");
            return ExitCode::from(2);
        }
    };
    let result = (|| -> Result<(), sbctl::config::ConfigError> {
        if !sbctl::traffic::interface_exists(root, &new.interface) {
            return Err(sbctl::config::ConfigError::InvalidValue(
                "the selected traffic interface does not exist on this host",
            ));
        }
        let binary = sing_box_bin.unwrap_or_else(|| root.join("usr/local/bin/sing-box"));
        match existing {
            None => {
                let artifacts = sbctl::subscription::generated_artifacts_for_kernel(
                    new,
                    root,
                    Some(binary.as_path()),
                )
                .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
                let server = artifacts
                    .iter()
                    .find(|(name, _)| *name == "sing-box-server.json")
                    .map(|(_, contents)| contents)
                    .ok_or(sbctl::config::ConfigError::InvalidValue(
                        "no generated sing-box server configuration is available to check",
                    ))?;
                if !binary.is_file() {
                    return Err(sbctl::config::ConfigError::InvalidValue(
                        "a new deployment requires --sing-box-bin for configuration validation",
                    ));
                }
                sbctl::subscription::check_sing_box_config(&binary, server)
                    .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
                let references = artifacts
                    .iter()
                    .map(|(name, contents)| (name.clone(), contents.as_bytes()))
                    .collect::<Vec<_>>();
                store.initialize_with_artifacts(new, &references)?;
                Ok(())
            }
            Some(prior) => {
                let snapshot = sbctl::subscription::apply_config_transaction(
                    store,
                    new,
                    binary.is_file().then_some(binary.as_path()),
                )
                .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
                // Re-applying the daemon storage permissions after a transactional
                // configuration change keeps the rewritten active sing-box
                // configuration readable by the sing-box service account. Without
                // this, the rewritten /etc/sing-box/config.json stays 0600
                // root:root and the service cannot start (deployment issue #5).
                let direct = new.subscription_mode == sbctl::config::SubscriptionMode::Direct;
                if root == Path::new("/") {
                    sbctl::lifecycle::prepare_daemon_prerequisites(root, direct)
                        .map_err(sbctl::config::ConfigError::StateContent)?;
                }
                sbctl::lifecycle::prepare_daemon_storage(root, direct)
                    .map_err(|error| sbctl::config::ConfigError::StateContent(error.to_string()))?;
                restart_services_with_rollback(root, || {
                    let _ = sbctl::subscription::restore_config_transaction(store, &snapshot);
                })?;
                if accounting_schedule_changed(&prior, new)
                    && let Err(error) = sbctl::traffic::reset(store, new)
                {
                    eprintln!(
                        "warning: could not establish the new accounting state now ({error}); the next accounting reset timer run will establish it"
                    );
                }
                Ok(())
            }
        }
    })();
    match result {
        Ok(()) => {
            println!("deployment configuration committed\n{}", new.summary());
            super::install::print_firewall_review(new);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("configuration wizard failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// A policy, timezone, first reset instant, or interface change alters the
/// accounting cycle, so the wizard establishes a new accounting state instead
/// of carrying the previous period's accumulated traffic forward.
fn accounting_schedule_changed(
    prior: &sbctl::config::DeploymentConfig,
    new: &sbctl::config::DeploymentConfig,
) -> bool {
    prior.accounting_policy != new.accounting_policy
        || prior.accounting_timezone != new.accounting_timezone
        || prior.anchored_reset_at != new.anchored_reset_at
        || prior.interface != new.interface
}

/// Restarts the managed services after a configuration commit. If the health
/// check fails, the rollback closure restores the previous known-good files,
/// the services are restarted again, and the failure is reported.
pub(crate) fn restart_services_with_rollback(
    root: &Path,
    rollback: impl FnOnce(),
) -> Result<(), sbctl::config::ConfigError> {
    if let Err(error) = sbctl::lifecycle::restart_services(root) {
        rollback();
        if let Err(rollback_error) = sbctl::lifecycle::restart_services(root) {
            eprintln!(
                "warning: the rollback restart failed too ({rollback_error}); inspect \
                 `systemctl status sing-box.service sbctl.service` before retrying"
            );
        }
        return Err(sbctl::config::ConfigError::StateContent(error));
    }
    Ok(())
}

fn configuration_artifact(root: &Path, format: &str, output: Option<&Path>) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let name = if format == "server" {
            "sing-box-server.json".into()
        } else {
            sbctl::subscription::subscription_matrix()
                .into_iter()
                .find(|row| row.format.path_name() == format)
                .ok_or("未知配置格式")?
                .format
                .artifact_name()
                .into_owned()
        };
        let contents = fs::read(root.join("var/lib/sbctl/artifacts").join(name))?;
        if let Some(output) = output {
            use std::io::Write;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(output)?;
            file.write_all(&contents)?;
            file.sync_all()?;
            println!(
                "已导出 {}（包含凭据，请使用 scp 下载并妥善保管）",
                output.display()
            );
        } else {
            println!("{}", String::from_utf8(contents)?);
        }
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("配置读取 / 导出失败：{error}");
            ExitCode::from(2)
        }
    }
}
