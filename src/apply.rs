//! Transactional configuration validation, generation publication, systemd unit
//! reconciliation, and automatic rollback on failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::config::{CONFIG_REL, Config, VAR_DIR};
use crate::render::{self, ClientBundle, CompiledDeployment};
use crate::secrets::{Secrets, random_hex};
use crate::store::{
    self, CURRENT_REL, GenerationMeta, SERVE_AUTH_REL, SINGBOX_CONFIG_REL, ServeAuth,
    XRAY_CONFIG_REL,
};

const UNIT_VSM: &str = include_str!("../templates/systemd/vps-sub-meter.service");
const UNIT_XRAY: &str = include_str!("../templates/systemd/vps-sub-meter-xray.service");
const UNIT_SINGBOX: &str = include_str!("../templates/systemd/vps-sub-meter-singbox.service");

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("配置错误: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("密钥错误: {0}")]
    Secrets(#[from] crate::secrets::SecretsError),
    #[error("编译错误: {0}")]
    Compile(#[from] render::CompileError),
    #[error("文件系统错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0} 内核配置校验失败（现网服务未受影响）: {1}")]
    KernelCheck(&'static str, String),
    #[error("执行系统操作失败: {0}")]
    System(String),
    #[error("主配置在编辑期间已修改，请刷新面板后重试")]
    StaleConfig,
}

#[derive(Clone, Debug)]
pub struct ApplyReport {
    pub generation_id: String,
    pub direct_nodes: usize,
    pub singbox_nodes: usize,
    pub xray_restarted: bool,
    pub singbox_restarted: bool,
    pub argo_warning: Option<String>,
}

pub fn check_kernel_config(binary: &Path, is_xray: bool, json_doc: &str) -> Result<(), ApplyError> {
    let mut tmp = tempfile::Builder::new().suffix(".json").tempfile()?;
    std::io::Write::write_all(&mut tmp, json_doc.as_bytes())?;
    std::io::Write::flush(&mut tmp)?;
    let args: &[&str] = if is_xray {
        &["run", "-test", "-c"]
    } else {
        &["check", "-c"]
    };
    let mut cmd = Command::new(binary);
    cmd.args(args).arg(tmp.path());
    if is_xray {
        cmd.env("XRAY_LOCATION_ASSET", "/usr/local/share/xray");
    }
    let out = cmd.output()?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let detail = stderr
            .lines()
            .chain(stdout.lines())
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" | ");
        Err(ApplyError::KernelCheck(
            if is_xray { "xray" } else { "sing-box" },
            if detail.is_empty() {
                format!("exit={}", out.status)
            } else {
                detail
            },
        ))
    }
}

fn file_sha256_opt(path: &Path) -> Option<String> {
    let bytes = store::read_no_follow(path).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
}

/// Canonical SHA-256 fingerprint of all deployment-affecting configuration
/// (everything except live-reloaded `traffic` schedule/quota settings, plus the
/// resolved Argo preferred IP list when Argo is enabled, external custom client
/// templates, and the IP-mode leaf TLS certificate when present).
pub fn deployment_fingerprint(root: &Path, config: &Config) -> String {
    #[derive(serde::Serialize)]
    struct FingerprintInput<'a> {
        profile_name: &'a str,
        domain: &'a str,
        public_ip: &'a str,
        interface: &'a str,
        ipv4_only: bool,
        latency_probe_url: &'a str,
        server: &'a crate::config::ServerConfig,
        nodes: &'a crate::config::NodesConfig,
        argo: &'a crate::config::ArgoConfig,
        argo_preferred_ips: Option<Vec<String>>,
        mihomo_template_sha256: Option<String>,
        singbox_template_sha256: Option<String>,
        ip_mode_cert_sha256: Option<String>,
    }
    let ip_mode_cert_sha256 = config
        .is_ip_mode()
        .then(|| {
            file_sha256_opt(
                &root
                    .join(VAR_DIR)
                    .join("certificates")
                    .join(config.tls_sni())
                    .join("fullchain.pem"),
            )
        })
        .flatten();
    let input = FingerprintInput {
        profile_name: &config.profile_name,
        domain: &config.domain,
        public_ip: &config.public_ip,
        interface: &config.interface,
        ipv4_only: config.ipv4_only,
        latency_probe_url: &config.latency_probe_url,
        server: &config.server,
        nodes: &config.nodes,
        argo: &config.argo,
        argo_preferred_ips: config
            .argo
            .enabled
            .then(|| render::client::load_argo_preferred_ips(root, config)),
        mihomo_template_sha256: file_sha256_opt(
            &root.join("etc/vps-sub-meter/templates/mihomo.yaml"),
        ),
        singbox_template_sha256: file_sha256_opt(
            &root.join("etc/vps-sub-meter/templates/sing-box.json"),
        ),
        ip_mode_cert_sha256,
    };
    let bytes = serde_json::to_vec(&input).expect("deployment fingerprint serializes");
    format!("{:x}", Sha256::digest(bytes))
}

/// Returns true when the current `config` has deployment changes that have not
/// yet been compiled and published into `generation`.
pub fn has_pending_changes(
    root: &Path,
    config: &Config,
    generation: Option<&GenerationMeta>,
) -> bool {
    let Some(meta) = generation else {
        return true;
    };
    if let Some(published) = meta.config_fingerprint.as_deref() {
        return published != deployment_fingerprint(root, config);
    }
    let expected_direct: Vec<&str> = [
        (
            config.nodes.vless_reality_pqc.enabled,
            config.nodes.vless_reality_pqc.name.as_str(),
        ),
        (
            config.nodes.vless_xhttp_pqc.enabled,
            config.nodes.vless_xhttp_pqc.name.as_str(),
        ),
        (
            config.nodes.anytls_tcp.enabled,
            config.nodes.anytls_tcp.name.as_str(),
        ),
        (
            config.nodes.hysteria2_hop.enabled,
            config.nodes.hysteria2_hop.name.as_str(),
        ),
        (
            config.nodes.tuic_bbr.enabled,
            config.nodes.tuic_bbr.name.as_str(),
        ),
    ]
    .into_iter()
    .filter_map(|(enabled, name)| enabled.then_some(name))
    .collect();
    let published_direct: Vec<&str> = meta.direct_nodes.iter().map(String::as_str).collect();
    expected_direct != published_direct || config.argo.enabled != !meta.argo_files.is_empty()
}

pub fn apply_all(root: &Path) -> Result<ApplyReport, ApplyError> {
    apply_with_keys(root, false, None, None)
}

pub fn rotate_node_keys(root: &Path) -> Result<ApplyReport, ApplyError> {
    apply_with_keys(root, true, None, None)
}

pub fn apply_candidate(root: &Path, config: Config) -> Result<ApplyReport, ApplyError> {
    apply_candidate_at_revision(root, config, None)
}

pub fn apply_candidate_at_revision(
    root: &Path,
    config: Config,
    revision: Option<&str>,
) -> Result<ApplyReport, ApplyError> {
    config.validate()?;
    apply_with_keys(root, false, Some(config), revision)
}

fn apply_with_keys(
    root: &Path,
    rotate_keys: bool,
    candidate: Option<Config>,
    revision: Option<&str>,
) -> Result<ApplyReport, ApplyError> {
    let _lock = store::lock_exclusive(&root.join(VAR_DIR))?;
    if let Some(revision) = revision
        && store::config_revision(root)? != revision
    {
        return Err(ApplyError::StaleConfig);
    }
    let (config, previous_config) = match candidate {
        Some(config) => (config, Some(backup(root, CONFIG_REL, 0o640)?)),
        None => (Config::load(root)?, None),
    };
    let xray_bin = find_bin(root, "usr/local/bin/xray");
    let sb_bin = find_bin(root, "usr/local/bin/sing-box");

    let previous = Secrets::load_or_default(root)?;
    let mut secrets = if rotate_keys {
        Secrets {
            tokens: previous.tokens.clone(),
            ..Secrets::default()
        }
    } else {
        previous.clone()
    };
    secrets.ensure_populated(&config, xray_bin.as_deref())?;

    let compiled = render::compile_all(root, &config, &secrets)?;

    // Pre-flight check with real kernel binaries before touching any live config or service.
    preflight_kernels(root, &compiled, xray_bin.as_deref(), sb_bin.as_deref())?;

    // Write immutable client generation (direct + optional precompiled Argo).
    let gen_id = random_hex(12)?;
    let gen_dir = root.join(VAR_DIR).join("generations").join(&gen_id);
    let files = write_client_bundle(&gen_dir, &compiled.clients)?;
    let (argo_nodes, argo_files) = match &compiled.argo_clients {
        Some(argo) => {
            let af = write_client_bundle(&gen_dir.join("argo"), argo)?;
            (argo.direct_node_names.clone(), af)
        }
        None => (Vec::new(), Vec::new()),
    };

    let meta = GenerationMeta {
        id: gen_id.clone(),
        files,
        direct_nodes: compiled.clients.direct_node_names.clone(),
        singbox_nodes: compiled.clients.singbox_node_names.clone(),
        argo_nodes,
        argo_files,
        config_fingerprint: Some(deployment_fingerprint(root, &config)),
    };

    let backups = [
        backup(root, crate::secrets::SECRETS_REL, 0o600)?,
        backup(root, SERVE_AUTH_REL, 0o640)?,
        backup(root, CURRENT_REL, 0o640)?,
    ];
    let previously_active = if previous_config.is_some() && root == Path::new("/") {
        [
            "vps-sub-meter.service",
            "vps-sub-meter-xray.service",
            "vps-sub-meter-singbox.service",
        ]
        .into_iter()
        .filter(|unit| is_active(unit))
        .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let publish = || {
        if previous_config.is_some() {
            config.save(root)?;
        }
        if secrets != previous {
            secrets.save(root)?;
        }
        let auth = ServeAuth {
            token_sha256: secrets
                .tokens
                .iter()
                .map(|t| store::hash_token(&t.token))
                .collect(),
        };
        store::write_serve_auth(root, &auth)?;
        commit_server_and_services(root, &config, &compiled, &meta)
    };
    let (xray_restarted, singbox_restarted) = match publish() {
        Ok(result) => result,
        Err(error) => {
            let error = restore_files(&backups, error);
            let error = if let Some(previous) = previous_config {
                let error = restore_files(&[previous], error);
                for unit in previously_active {
                    if unit == "vps-sub-meter.service" || !is_active(unit) {
                        let _ = systemctl(&["restart", unit]);
                    }
                }
                error
            } else {
                error
            };
            if store::current_generation(root)
                .is_ok_and(|generation| generation.is_none_or(|meta| meta.id != gen_id))
            {
                let _ = fs::remove_dir_all(&gen_dir);
            }
            return Err(error);
        }
    };

    prune_old_generations(root, &gen_id);
    let argo_warning = if root == Path::new("/") {
        fix_live_permissions(&gen_dir);
        sync_live_firewall(&config);
        refresh_live_argo(&config)
    } else {
        None
    };

    Ok(ApplyReport {
        generation_id: gen_id,
        direct_nodes: compiled.clients.direct_node_names.len(),
        singbox_nodes: compiled.clients.singbox_node_names.len(),
        xray_restarted,
        singbox_restarted,
        argo_warning,
    })
}

fn preflight_kernels(
    root: &Path,
    compiled: &CompiledDeployment,
    xray_bin: Option<&Path>,
    sb_bin: Option<&Path>,
) -> Result<(), ApplyError> {
    let require = root == Path::new("/");
    for (doc_opt, bin_opt, is_xray, err_msg) in [
        (
            &compiled.xray_server,
            xray_bin,
            true,
            "未找到 /usr/local/bin/xray，请先运行 cores.sh",
        ),
        (
            &compiled.singbox_server,
            sb_bin,
            false,
            "未找到 /usr/local/bin/sing-box，请先运行 cores.sh",
        ),
    ] {
        let Some(doc) = doc_opt else { continue };
        match bin_opt {
            Some(bin) => check_kernel_config(bin, is_xray, doc)?,
            None if require => return Err(ApplyError::System(err_msg.into())),
            None => {}
        }
    }
    Ok(())
}

fn write_client_bundle(dir: &Path, bundle: &ClientBundle) -> Result<Vec<String>, ApplyError> {
    store::ensure_dir_mode(dir, 0o750)?;
    let mut files = vec![
        "clash.yaml".to_owned(),
        "shadowrocket.txt".to_owned(),
        "uri.txt".to_owned(),
    ];
    store::write_atomic(&dir.join("clash.yaml"), bundle.clash_yaml.as_bytes(), 0o640)?;
    store::write_atomic(
        &dir.join("shadowrocket.txt"),
        bundle.shadowrocket_txt.as_bytes(),
        0o640,
    )?;
    store::write_atomic(&dir.join("uri.txt"), bundle.uri_txt.as_bytes(), 0o640)?;
    if let Some(sb_client) = &bundle.sing_box_json {
        store::write_atomic(&dir.join("sing-box.json"), sb_client.as_bytes(), 0o640)?;
        files.push("sing-box.json".to_owned());
    }
    Ok(files)
}

pub fn update_tokens(
    root: &Path,
    update: impl FnOnce(&mut Vec<crate::secrets::SubscriptionToken>) -> Result<(), String>,
) -> Result<(), ApplyError> {
    let _lock = store::lock_exclusive(&root.join(VAR_DIR))?;
    let mut secrets = Secrets::load_or_default(root)?;
    let mut auth = store::read_serve_auth(root)
        .map_err(|e| ApplyError::System(format!("serve-auth.json 不可用: {e}")))?;
    update(&mut secrets.tokens).map_err(ApplyError::System)?;
    auth.token_sha256 = secrets
        .tokens
        .iter()
        .map(|t| store::hash_token(&t.token))
        .collect();

    let previous = backup(root, crate::secrets::SECRETS_REL, 0o600)?;
    secrets.save(root)?;
    store::write_serve_auth(root, &auth)
        .map_err(|error| restore_files(&[previous], ApplyError::Io(error)))
}

struct FileBackup {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    mode: u32,
}

fn backup(root: &Path, relative: &str, mode: u32) -> std::io::Result<FileBackup> {
    let path = root.join(relative);
    let bytes = match store::read_no_follow(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    Ok(FileBackup { path, bytes, mode })
}

fn restore_files(backups: &[FileBackup], error: ApplyError) -> ApplyError {
    let failures: Vec<_> = backups
        .iter()
        .filter_map(|backup| {
            let result = match &backup.bytes {
                Some(bytes) if store::read_no_follow(&backup.path).as_ref().ok() == Some(bytes) => {
                    Ok(())
                }
                Some(bytes) => {
                    store::write_atomic_preserving_owner(&backup.path, bytes, backup.mode)
                }
                None => match fs::remove_file(&backup.path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    result => result,
                },
            };
            result
                .err()
                .map(|error| format!("{}: {error}", backup.path.display()))
        })
        .collect();
    if failures.is_empty() {
        error
    } else {
        ApplyError::System(format!("{error}，回滚失败: {}", failures.join("；")))
    }
}

fn find_bin(root: &Path, rel: &str) -> Option<PathBuf> {
    let p = root.join(rel);
    p.is_file().then_some(p)
}

fn commit_server_and_services(
    root: &Path,
    config: &Config,
    compiled: &CompiledDeployment,
    meta: &GenerationMeta,
) -> Result<(bool, bool), ApplyError> {
    let backups = [
        backup(root, XRAY_CONFIG_REL, 0o640)?,
        backup(root, SINGBOX_CONFIG_REL, 0o640)?,
        backup(root, "etc/systemd/system/vps-sub-meter.service", 0o644)?,
        backup(root, "etc/systemd/system/vps-sub-meter-xray.service", 0o644)?,
        backup(
            root,
            "etc/systemd/system/vps-sub-meter-singbox.service",
            0o644,
        )?,
    ];
    let xray_changed =
        backups[0].bytes.as_deref() != compiled.xray_server.as_deref().map(str::as_bytes);
    let sb_changed =
        backups[1].bytes.as_deref() != compiled.singbox_server.as_deref().map(str::as_bytes);
    let xray_releases_https_port = !config.is_ip_mode()
        && !config.xray_fronts_443()
        && backups[0]
            .bytes
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
            .is_some_and(|doc| {
                doc["inbounds"].as_array().is_some_and(|inbounds| {
                    inbounds.iter().any(|inbound| {
                        inbound["tag"] == "sni-demux-443"
                            && inbound["port"] == config.server.https_port
                    })
                })
            });

    let write_configs = || -> Result<(), ApplyError> {
        if let Some(doc) = &compiled.xray_server {
            store::ensure_dir_mode(&root.join("etc/vps-sub-meter/xray"), 0o750)?;
            store::write_atomic_preserving_owner(
                &root.join(XRAY_CONFIG_REL),
                doc.as_bytes(),
                0o640,
            )?;
        }
        if let Some(doc) = &compiled.singbox_server {
            store::ensure_dir_mode(&root.join("etc/vps-sub-meter/sing-box"), 0o750)?;
            store::write_atomic_preserving_owner(
                &root.join(SINGBOX_CONFIG_REL),
                doc.as_bytes(),
                0o640,
            )?;
        }
        write_systemd_units(root, config)?;
        store::write_atomic_preserving_owner(
            &root.join(CURRENT_REL),
            &serde_json::to_vec_pretty(meta).unwrap(),
            0o640,
        )?;
        Ok(())
    };
    write_configs().map_err(|error| restore_files(&backups, error))?;

    if root != Path::new("/") {
        return Ok((xray_changed, sb_changed));
    }

    fix_live_permissions(&root.join(VAR_DIR).join("generations").join(&meta.id));
    let apply_result = (|| -> Result<(bool, bool), ApplyError> {
        systemctl(&["daemon-reload"])?;
        if xray_releases_https_port && is_active("vps-sub-meter-xray.service") {
            systemctl(&["stop", "vps-sub-meter-xray.service"])?;
        }
        systemctl(&["enable", "vps-sub-meter.service"])?;
        if !is_active("vps-sub-meter.service") {
            systemctl(&["start", "vps-sub-meter.service"])?;
        } else {
            systemctl(&["restart", "vps-sub-meter.service"])?;
        }

        let xr_restarted = reconcile_core_unit(
            "vps-sub-meter-xray.service",
            compiled.xray_server.is_some(),
            xray_changed,
        )?;
        let sb_restarted = reconcile_core_unit(
            "vps-sub-meter-singbox.service",
            compiled.singbox_server.is_some(),
            sb_changed,
        )?;

        // Brief health verification
        std::thread::sleep(std::time::Duration::from_millis(800));
        for (enabled, unit) in [
            (true, "vps-sub-meter.service"),
            (compiled.xray_server.is_some(), "vps-sub-meter-xray.service"),
            (
                compiled.singbox_server.is_some(),
                "vps-sub-meter-singbox.service",
            ),
        ] {
            if enabled && !is_active(unit) {
                return Err(ApplyError::System(format!("{unit} 启动后未能保持活跃")));
            }
        }
        Ok((xr_restarted, sb_restarted))
    })();

    apply_result.map_err(|error| {
        let error = restore_files(&backups, error);
        let _ = systemctl(&["daemon-reload"]);
        for (backup, unit) in [
            (&backups[0], "vps-sub-meter-xray.service"),
            (&backups[1], "vps-sub-meter-singbox.service"),
        ] {
            if backup.bytes.is_some() {
                let _ = systemctl(&["restart", unit]);
            }
        }
        error
    })
}

fn reconcile_core_unit(unit: &str, enabled: bool, changed: bool) -> Result<bool, ApplyError> {
    if enabled {
        systemctl(&["enable", unit])?;
        if changed || !is_active(unit) {
            systemctl(&["restart", unit])?;
            return Ok(true);
        }
    } else if unit_exists(unit) {
        let _ = systemctl(&["disable", "--now", unit]);
    }
    Ok(false)
}

pub fn write_systemd_units(root: &Path, config: &Config) -> Result<(), ApplyError> {
    let unit_dir = root.join("etc/systemd/system");
    fs::create_dir_all(&unit_dir)?;

    store::write_atomic(
        &unit_dir.join("vps-sub-meter.service"),
        UNIT_VSM.as_bytes(),
        0o644,
    )?;
    store::write_atomic(
        &unit_dir.join("vps-sub-meter-xray.service"),
        UNIT_XRAY.as_bytes(),
        0o644,
    )?;

    let hop_hooks = if config.nodes.hysteria2_hop.enabled
        && let Ok(Some((start, end))) = config.parse_hop_range()
    {
        format!(
            "ExecStartPre=+/usr/local/lib/vps-sub-meter/net.sh apply-hy2 {} {}-{} {}\n\
             ExecStopPost=+/usr/local/lib/vps-sub-meter/net.sh cleanup-hy2\n",
            config.interface, start, end, config.nodes.hysteria2_hop.port
        )
    } else {
        "ExecStartPre=+/usr/local/lib/vps-sub-meter/net.sh cleanup-hy2\n".to_owned()
    };
    let sb_unit = UNIT_SINGBOX.replace("{{HY2_HOOKS}}", &hop_hooks);
    store::write_atomic(
        &unit_dir.join("vps-sub-meter-singbox.service"),
        sb_unit.as_bytes(),
        0o644,
    )?;
    Ok(())
}

fn fix_live_permissions(gen_dir: &Path) {
    store::chown_name(Path::new("/etc/vps-sub-meter"), Some("root"), Some("root"));
    store::chown_name(
        Path::new("/etc/vps-sub-meter/config.toml"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_name(
        Path::new("/etc/vps-sub-meter/argo-ips.txt"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_tree(
        Path::new("/etc/vps-sub-meter/templates"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_name(
        Path::new("/var/lib/vps-sub-meter"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_name(
        Path::new("/var/lib/vps-sub-meter/generations"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_tree(gen_dir, Some("root"), Some("vps-sub-meter"));
    store::chown_name(
        Path::new("/var/lib/vps-sub-meter/current.json"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_name(
        Path::new("/var/lib/vps-sub-meter/serve-auth.json"),
        Some("root"),
        Some("vps-sub-meter"),
    );
    store::chown_name(
        Path::new("/var/lib/vps-sub-meter/traffic-state.json"),
        Some("vps-sub-meter"),
        Some("vps-sub-meter"),
    );
    store::chown_name(
        Path::new("/var/lib/vps-sub-meter/.operation.lock"),
        Some("vps-sub-meter"),
        Some("vps-sub-meter"),
    );
    store::chown_tree(
        Path::new("/etc/vps-sub-meter/xray"),
        Some("root"),
        Some("xray"),
    );
    store::chown_tree(
        Path::new("/etc/vps-sub-meter/sing-box"),
        Some("root"),
        Some("sing-box"),
    );
    store::chown_tree(
        Path::new("/var/lib/vps-sub-meter/certificates"),
        Some("root"),
        Some("vsm-cert"),
    );
}

/// Collects TCP/UDP port and range specs required by enabled services for firewall sync.
pub(crate) fn collect_firewall_ports(config: &Config) -> Vec<String> {
    let mut specs = Vec::new();
    let mut push = |spec: String| {
        if !specs.contains(&spec) {
            specs.push(spec);
        }
    };
    if config.is_ip_mode() {
        if config.server.http_fallback_port > 0 {
            push(format!("{}/tcp", config.server.http_fallback_port));
        }
    } else {
        if config.server.acme_port > 0 {
            push(format!("{}/tcp", config.server.acme_port));
        }
        if config.server.https_port > 0 {
            push(format!("{}/tcp", config.server.https_port));
        }
    }
    if let Some(extra) = config.server.extra_https_port
        && extra > 0
    {
        push(format!("{extra}/tcp"));
    }
    if config.nodes.vless_reality_pqc.enabled {
        push(format!("{}/tcp", config.nodes.vless_reality_pqc.port));
    }
    if config.nodes.vless_xhttp_pqc.enabled {
        push(format!("{}/tcp", config.nodes.vless_xhttp_pqc.port));
    }
    if config.nodes.anytls_tcp.enabled {
        push(format!("{}/tcp", config.nodes.anytls_tcp.port));
    }
    if config.nodes.hysteria2_hop.enabled {
        push(format!("{}/udp", config.nodes.hysteria2_hop.port));
        if let Ok(Some((start, end))) = config.parse_hop_range() {
            push(format!("{start}:{end}/udp"));
        }
    }
    if config.nodes.tuic_bbr.enabled {
        push(format!("{}/udp", config.nodes.tuic_bbr.port));
    }
    specs
}

fn sync_live_firewall(config: &Config) {
    let helper = Path::new("/usr/local/lib/vps-sub-meter/net.sh");
    if !helper.is_file() {
        return;
    }
    let specs = collect_firewall_ports(config);
    if !specs.is_empty() {
        let _ = Command::new(helper).arg("sync-ufw").args(&specs).status();
    }
}

fn refresh_live_argo(config: &Config) -> Option<String> {
    let helper = Path::new("/usr/local/lib/vps-sub-meter/argo.sh");
    if config.argo.enabled
        && helper.is_file()
        && !Command::new(helper)
            .arg("refresh")
            .status()
            .is_ok_and(|status| status.success())
    {
        let warning = "Argo 回源规则更新失败，请检查隧道配置与 Cloudflared 服务".to_owned();
        eprintln!("{warning}");
        return Some(warning);
    }
    None
}

fn systemctl(args: &[&str]) -> Result<(), ApplyError> {
    let status = Command::new("systemctl").args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(ApplyError::System(format!(
            "systemctl {} 失败",
            args.join(" ")
        )))
    }
}

fn is_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .status()
        .is_ok_and(|s| s.success())
}

fn unit_exists(unit: &str) -> bool {
    Path::new("/etc/systemd/system").join(unit).exists()
}

fn prune_old_generations(root: &Path, keep_id: &str) {
    let dir = root.join(VAR_DIR).join("generations");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(id) = name.to_str() else { continue };
        if id == keep_id {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(elapsed) = meta.modified().and_then(|m| {
                m.elapsed()
                    .map_err(|e| std::io::Error::other(e.to_string()))
            })
            && elapsed.as_secs() > 300
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_all_generates_sanitized_runtime_files_and_systemd_units_in_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        cfg.argo.enabled = true;
        cfg.argo.tunnel_domain = "argo.example.com".into();
        cfg.argo.sub_domain = "sub.example.com".into();
        cfg.save(dir.path()).unwrap();

        let sec = crate::test_secrets::fixture(&cfg);
        sec.save(dir.path()).unwrap();

        let report = apply_all(dir.path()).unwrap();
        assert_eq!(report.direct_nodes, 5);
        assert_eq!(report.singbox_nodes, 3);

        // Client generations must not leak server private keys.
        let clash =
            store::read_generation_file(dir.path(), &report.generation_id, false, "clash.yaml")
                .unwrap()
                .unwrap();
        assert!(!clash.contains(&sec.vless_reality_pqc.x25519_private_key));
        assert!(!clash.contains(&sec.vless_reality_pqc.mldsa65_seed));

        // Precompiled Argo generation is also written under generations/<id>/argo/.
        let argo_clash =
            store::read_generation_file(dir.path(), &report.generation_id, true, "clash.yaml")
                .unwrap();
        assert!(argo_clash.is_some());

        // serve-auth.json must store only token SHA-256 hashes, not raw tokens or argo_ws secrets.
        let auth_text = fs::read_to_string(dir.path().join(SERVE_AUTH_REL)).unwrap();
        assert!(!auth_text.contains(sec.primary_token().unwrap()));
        assert!(!auth_text.contains("argo_ws"));

        // Systemd units use unified vps-sub-meter naming and underscore nftables hook.
        let sb_unit = fs::read_to_string(
            dir.path()
                .join("etc/systemd/system/vps-sub-meter-singbox.service"),
        )
        .unwrap();
        assert!(sb_unit.contains("net.sh apply-hy2 eth0 30000-35000 34443"));
    }

    #[test]
    fn collects_firewall_ports_for_enabled_nodes_and_acme() {
        let cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        let ports = collect_firewall_ports(&cfg);
        assert_eq!(
            ports,
            vec![
                "80/tcp",
                "443/tcp",
                "2053/tcp",
                "2083/tcp",
                "34443/udp",
                "30000:35000/udp",
                "2096/udp",
            ]
        );
    }

    #[test]
    fn collects_http_fallback_port_in_ip_mode() {
        let cfg = Config {
            domain: String::new(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        let ports = collect_firewall_ports(&cfg);
        assert_eq!(
            ports,
            vec![
                "2080/tcp",
                "443/tcp",
                "2053/tcp",
                "2083/tcp",
                "34443/udp",
                "30000:35000/udp",
                "2096/udp",
            ]
        );
    }

    #[test]
    fn pending_changes_detects_deployment_edits_but_ignores_live_traffic_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        cfg.argo.enabled = true;
        cfg.argo.tunnel_domain = "argo.example.com".into();
        cfg.argo.sub_domain = "argo.example.com".into();
        cfg.save(dir.path()).unwrap();
        crate::test_secrets::fixture(&cfg).save(dir.path()).unwrap();

        assert!(has_pending_changes(dir.path(), &cfg, None));
        apply_all(dir.path()).unwrap();
        let meta = store::current_generation(dir.path()).unwrap().unwrap();
        assert!(meta.config_fingerprint.is_some());
        assert!(!has_pending_changes(dir.path(), &cfg, Some(&meta)));

        // Traffic schedule/quota updates take effect live without `vsm apply`.
        let mut traffic_only = cfg.clone();
        traffic_only.traffic.monthly_gb = 123.0;
        traffic_only.traffic.reset_day = 15;
        assert!(!has_pending_changes(dir.path(), &traffic_only, Some(&meta)));

        // Port, domain, protocol param, preferred IP, or preferred_ips_file edits require apply.
        let mut port_changed = cfg.clone();
        port_changed.nodes.anytls_tcp.port = 24443;
        assert!(has_pending_changes(dir.path(), &port_changed, Some(&meta)));

        let mut domain_changed = cfg.clone();
        domain_changed.domain = "new.example.com".into();
        assert!(has_pending_changes(
            dir.path(),
            &domain_changed,
            Some(&meta)
        ));

        let mut proto_changed = cfg.clone();
        proto_changed.nodes.hysteria2_hop.obfs_salamander =
            !proto_changed.nodes.hysteria2_hop.obfs_salamander;
        assert!(has_pending_changes(dir.path(), &proto_changed, Some(&meta)));

        let mut ip_changed = cfg.clone();
        ip_changed.argo.preferred_ips = Some(vec!["104.16.0.1".parse().unwrap()]);
        assert!(has_pending_changes(dir.path(), &ip_changed, Some(&meta)));

        // Editing the referenced argo-ips.txt file also flags pending changes.
        let ips_file = dir.path().join("etc/vps-sub-meter/argo-ips.txt");
        fs::create_dir_all(ips_file.parent().unwrap()).unwrap();
        fs::write(&ips_file, "104.16.0.1\n").unwrap();
        assert!(has_pending_changes(dir.path(), &cfg, Some(&meta)));
        fs::remove_file(&ips_file).unwrap();
        assert!(!has_pending_changes(dir.path(), &cfg, Some(&meta)));

        // Editing external custom client templates also flags pending changes.
        let custom_mihomo = dir.path().join("etc/vps-sub-meter/templates/mihomo.yaml");
        fs::create_dir_all(custom_mihomo.parent().unwrap()).unwrap();
        fs::write(&custom_mihomo, "mixed-port: 7890\n").unwrap();
        assert!(has_pending_changes(dir.path(), &cfg, Some(&meta)));
        fs::remove_file(&custom_mihomo).unwrap();
        assert!(!has_pending_changes(dir.path(), &cfg, Some(&meta)));

        let custom_sb = dir.path().join("etc/vps-sub-meter/templates/sing-box.json");
        fs::write(&custom_sb, "{}\n").unwrap();
        assert!(has_pending_changes(dir.path(), &cfg, Some(&meta)));
        fs::remove_file(&custom_sb).unwrap();
        assert!(!has_pending_changes(dir.path(), &cfg, Some(&meta)));

        // IP-mode certificate updates change the pinned fingerprint and flag pending changes.
        let mut ip_cfg = Config {
            domain: String::new(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        ip_cfg.nodes.vless_xhttp_pqc.enabled = false;
        ip_cfg.nodes.anytls_tcp.enabled = false;
        ip_cfg.nodes.hysteria2_hop.enabled = false;
        ip_cfg.nodes.tuic_bbr.enabled = false;
        let ip_dir = tempfile::tempdir().unwrap();
        let ip_cert = ip_dir
            .path()
            .join("var/lib/vps-sub-meter/certificates/www.bing.com/fullchain.pem");
        fs::create_dir_all(ip_cert.parent().unwrap()).unwrap();
        fs::write(&ip_cert, "cert-v1").unwrap();
        ip_cfg.save(ip_dir.path()).unwrap();
        crate::test_secrets::fixture(&ip_cfg)
            .save(ip_dir.path())
            .unwrap();
        apply_all(ip_dir.path()).unwrap();
        let ip_meta = store::current_generation(ip_dir.path()).unwrap().unwrap();
        assert!(!has_pending_changes(ip_dir.path(), &ip_cfg, Some(&ip_meta)));
        fs::write(&ip_cert, "cert-v2").unwrap();
        assert!(has_pending_changes(ip_dir.path(), &ip_cfg, Some(&ip_meta)));

        // Legacy current.json without config_fingerprint falls back to node/argo comparison.
        let mut legacy = meta;
        legacy.config_fingerprint = None;
        assert!(!has_pending_changes(dir.path(), &cfg, Some(&legacy)));
        let mut argo_off = cfg.clone();
        argo_off.argo.enabled = false;
        assert!(has_pending_changes(dir.path(), &argo_off, Some(&legacy)));
    }
}
