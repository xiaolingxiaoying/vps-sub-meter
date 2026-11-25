//! Transactional configuration validation, generation publication, systemd unit
//! reconciliation, and automatic rollback on failure.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::compiler::{self, CompiledDeployment};
use crate::config::{Config, VAR_DIR};
use crate::fs_util;
use crate::secrets::{Secrets, random_hex};

pub const CURRENT_REL: &str = "var/lib/vps-sub-meter/current.json";
pub const SERVE_AUTH_REL: &str = "var/lib/vps-sub-meter/serve-auth.json";
pub const XRAY_CONFIG_REL: &str = "etc/vps-sub-meter/xray/config.json";
pub const SINGBOX_CONFIG_REL: &str = "etc/vps-sub-meter/sing-box/config.json";

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("配置错误: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("密钥错误: {0}")]
    Secrets(#[from] crate::secrets::SecretsError),
    #[error("编译错误: {0}")]
    Compile(#[from] compiler::CompileError),
    #[error("文件系统错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0} 内核配置校验失败，已拒绝应用（现网服务未受影响）")]
    KernelCheck(&'static str),
    #[error("执行系统操作失败: {0}")]
    System(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationMeta {
    pub id: String,
    pub files: Vec<String>,
    pub direct_nodes: Vec<String>,
    pub singbox_nodes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ServeAuth {
    pub token_sha256: Vec<String>,
    pub argo_ws: std::collections::BTreeMap<String, crate::secrets::ArgoWsSecret>,
}

#[derive(Clone, Debug)]
pub struct ApplyReport {
    pub generation_id: String,
    pub direct_nodes: usize,
    pub singbox_nodes: usize,
    pub xray_restarted: bool,
    pub singbox_restarted: bool,
}

pub fn current_generation(root: &Path) -> Result<Option<GenerationMeta>, ApplyError> {
    match fs_util::read_no_follow(&root.join(CURRENT_REL)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| ApplyError::System(format!("current.json 损坏: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn read_generation_file(
    root: &Path,
    gen_id: &str,
    name: &str,
) -> Result<Option<String>, ApplyError> {
    if !gen_id.bytes().all(|b| b.is_ascii_hexdigit())
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
    {
        return Ok(None);
    }
    let path = root
        .join(VAR_DIR)
        .join("generations")
        .join(gen_id)
        .join(name);
    match fs_util::read_to_string_no_follow(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
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
    let out = Command::new(binary).args(args).arg(tmp.path()).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(ApplyError::KernelCheck(if is_xray {
            "xray"
        } else {
            "sing-box"
        }))
    }
}

pub fn apply_all(root: &Path) -> Result<ApplyReport, ApplyError> {
    let lock_file = fs_util::open_lock(&root.join(VAR_DIR))?;
    fs2::FileExt::lock_exclusive(&lock_file)?;

    let config = Config::load(root)?;
    let xray_bin = find_bin(root, "usr/local/bin/xray");
    let sb_bin = find_bin(root, "usr/local/bin/sing-box");

    let mut secrets = Secrets::load_or_default(root)?;
    let secrets_before = secrets.clone();
    secrets.ensure_populated(&config, xray_bin.as_deref())?;
    if secrets != secrets_before {
        secrets.save(root)?;
    }

    let compiled = compiler::compile_all(root, &config, &secrets)?;

    // Pre-flight check with real kernel binaries before touching any live config or service.
    if root == Path::new("/") {
        if let Some(doc) = &compiled.xray_server {
            let bin = xray_bin.as_deref().ok_or_else(|| {
                ApplyError::System("未找到 /usr/local/bin/xray，请先运行 cores.sh".into())
            })?;
            check_kernel_config(bin, true, doc)?;
        }
        if let Some(doc) = &compiled.singbox_server {
            let bin = sb_bin.as_deref().ok_or_else(|| {
                ApplyError::System("未找到 /usr/local/bin/sing-box，请先运行 cores.sh".into())
            })?;
            check_kernel_config(bin, false, doc)?;
        }
    } else {
        if let (Some(bin), Some(doc)) = (&xray_bin, &compiled.xray_server) {
            check_kernel_config(bin, true, doc)?;
        }
        if let (Some(bin), Some(doc)) = (&sb_bin, &compiled.singbox_server) {
            check_kernel_config(bin, false, doc)?;
        }
    }

    // Write immutable client generation.
    let gen_id = random_hex(12)?;
    let gen_dir = root.join(VAR_DIR).join("generations").join(&gen_id);
    fs_util::ensure_dir_mode(&gen_dir, 0o750)?;
    let mut files = vec![
        "clash.yaml".to_owned(),
        "shadowrocket.txt".to_owned(),
        "uri.txt".to_owned(),
    ];
    fs_util::write_atomic(
        &gen_dir.join("clash.yaml"),
        compiled.clients.clash_yaml.as_bytes(),
        0o640,
    )?;
    fs_util::write_atomic(
        &gen_dir.join("shadowrocket.txt"),
        compiled.clients.shadowrocket_txt.as_bytes(),
        0o640,
    )?;
    fs_util::write_atomic(
        &gen_dir.join("uri.txt"),
        compiled.clients.uri_txt.as_bytes(),
        0o640,
    )?;
    if let Some(sb_client) = &compiled.clients.sing_box_json {
        fs_util::write_atomic(&gen_dir.join("sing-box.json"), sb_client.as_bytes(), 0o640)?;
        files.push("sing-box.json".to_owned());
    }

    // Write derived auth file for the non-root subscription daemon (only token SHA-256 hashes + Argo WS client fields).
    let serve_auth = ServeAuth {
        token_sha256: secrets
            .tokens
            .iter()
            .map(|t| {
                Sha256::digest(t.token.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            })
            .collect(),
        argo_ws: secrets.argo_ws.clone(),
    };
    fs_util::write_atomic(
        &root.join(SERVE_AUTH_REL),
        &serde_json::to_vec_pretty(&serve_auth).unwrap(),
        0o640,
    )?;

    let meta = GenerationMeta {
        id: gen_id.clone(),
        files,
        direct_nodes: compiled.clients.direct_node_names.clone(),
        singbox_nodes: compiled.clients.singbox_node_names.clone(),
    };

    let (xray_restarted, singbox_restarted) =
        commit_server_and_services(root, &config, &compiled, &meta)?;

    prune_old_generations(root, &gen_id);
    if root == Path::new("/") {
        fix_live_permissions();
    }

    Ok(ApplyReport {
        generation_id: gen_id,
        direct_nodes: compiled.clients.direct_node_names.len(),
        singbox_nodes: compiled.clients.singbox_node_names.len(),
        xray_restarted,
        singbox_restarted,
    })
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
    let prev_xray = fs_util::read_to_string_no_follow(&root.join(XRAY_CONFIG_REL)).ok();
    let prev_sb = fs_util::read_to_string_no_follow(&root.join(SINGBOX_CONFIG_REL)).ok();

    let xray_changed = prev_xray.as_deref() != compiled.xray_server.as_deref();
    let sb_changed = prev_sb.as_deref() != compiled.singbox_server.as_deref();

    if let Some(doc) = &compiled.xray_server {
        fs_util::ensure_dir_mode(&root.join("etc/vps-sub-meter/xray"), 0o750)?;
        fs_util::write_atomic(&root.join(XRAY_CONFIG_REL), doc.as_bytes(), 0o640)?;
    }
    if let Some(doc) = &compiled.singbox_server {
        fs_util::ensure_dir_mode(&root.join("etc/vps-sub-meter/sing-box"), 0o750)?;
        fs_util::write_atomic(&root.join(SINGBOX_CONFIG_REL), doc.as_bytes(), 0o640)?;
    }

    write_systemd_units(root, config)?;
    fs_util::write_atomic(
        &root.join(CURRENT_REL),
        &serde_json::to_vec_pretty(meta).unwrap(),
        0o640,
    )?;

    if root != Path::new("/") {
        return Ok((xray_changed, sb_changed));
    }

    fix_live_permissions();
    systemctl(&["daemon-reload"])?;

    let apply_result = (|| -> Result<(bool, bool), ApplyError> {
        systemctl(&["enable", "vps-sub-meter.service"])?;
        if !is_active("vps-sub-meter.service") {
            systemctl(&["start", "vps-sub-meter.service"])?;
        } else {
            systemctl(&["restart", "vps-sub-meter.service"])?;
        }

        let mut xr_restarted = false;
        if compiled.xray_server.is_some() {
            systemctl(&["enable", "vps-sub-meter-xray.service"])?;
            if xray_changed || !is_active("vps-sub-meter-xray.service") {
                systemctl(&["restart", "vps-sub-meter-xray.service"])?;
                xr_restarted = true;
            }
        } else if unit_exists("vps-sub-meter-xray.service") {
            let _ = systemctl(&["disable", "--now", "vps-sub-meter-xray.service"]);
        }

        let mut sb_restarted = false;
        if compiled.singbox_server.is_some() {
            systemctl(&["enable", "vps-sub-meter-singbox.service"])?;
            if sb_changed || !is_active("vps-sub-meter-singbox.service") {
                systemctl(&["restart", "vps-sub-meter-singbox.service"])?;
                sb_restarted = true;
            }
        } else if unit_exists("vps-sub-meter-singbox.service") {
            let _ = systemctl(&["disable", "--now", "vps-sub-meter-singbox.service"]);
        }

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

    if let Err(err) = apply_result {
        // Automatic rollback of previous server configs
        if let Some(old) = prev_xray {
            let _ = fs_util::write_atomic(&root.join(XRAY_CONFIG_REL), old.as_bytes(), 0o640);
            let _ = systemctl(&["restart", "vps-sub-meter-xray.service"]);
        }
        if let Some(old) = prev_sb {
            let _ = fs_util::write_atomic(&root.join(SINGBOX_CONFIG_REL), old.as_bytes(), 0o640);
            let _ = systemctl(&["restart", "vps-sub-meter-singbox.service"]);
        }
        return Err(err);
    }

    Ok(apply_result.unwrap())
}

pub fn write_systemd_units(root: &Path, config: &Config) -> Result<(), ApplyError> {
    let unit_dir = root.join("etc/systemd/system");
    fs::create_dir_all(&unit_dir)?;

    let vsm_unit = "[Unit]
Description=vps-sub-meter HTTPS Subscription & Traffic Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=vps-sub-meter
Group=vps-sub-meter
SupplementaryGroups=vsm-cert
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ReadWritePaths=/var/lib/vps-sub-meter
ReadOnlyPaths=/var/lib/vps-sub-meter/certificates /var/lib/vps-sub-meter/generations
ExecStart=/usr/local/bin/vps-sub-meter serve
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
";
    fs_util::write_atomic(
        &unit_dir.join("vps-sub-meter.service"),
        vsm_unit.as_bytes(),
        0o644,
    )?;

    let xray_unit = "[Unit]
Description=vps-sub-meter Xray Core (VLESS REALITY PQC + XHTTP PQC)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=xray
Group=xray
SupplementaryGroups=vsm-cert
Environment=XRAY_LOCATION_ASSET=/usr/local/share/xray
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ExecStart=/usr/local/bin/xray run -c /etc/vps-sub-meter/xray/config.json
Restart=on-failure
RestartSec=3
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
";
    fs_util::write_atomic(
        &unit_dir.join("vps-sub-meter-xray.service"),
        xray_unit.as_bytes(),
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

    let sb_unit = format!(
        "[Unit]
Description=vps-sub-meter sing-box Core (AnyTLS + Hysteria2 + TUIC v5)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=sing-box
Group=sing-box
SupplementaryGroups=vsm-cert
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
{hop_hooks}ExecStart=/usr/local/bin/sing-box run -c /etc/vps-sub-meter/sing-box/config.json
Restart=on-failure
RestartSec=3
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
"
    );
    fs_util::write_atomic(
        &unit_dir.join("vps-sub-meter-singbox.service"),
        sb_unit.as_bytes(),
        0o644,
    )?;

    Ok(())
}

fn fix_live_permissions() {
    let cmds = [
        "chown root:root /etc/vps-sub-meter && chmod 0755 /etc/vps-sub-meter 2>/dev/null || true",
        "chown root:vps-sub-meter /etc/vps-sub-meter/config.toml /etc/vps-sub-meter/argo-ips.txt 2>/dev/null || true",
        "chmod 0640 /etc/vps-sub-meter/config.toml /etc/vps-sub-meter/argo-ips.txt 2>/dev/null || true",
        "chown -R root:vps-sub-meter /etc/vps-sub-meter/templates /etc/vps-sub-meter/overrides /var/lib/vps-sub-meter/generations 2>/dev/null || true",
        "chmod 0750 /etc/vps-sub-meter/templates /etc/vps-sub-meter/overrides /var/lib/vps-sub-meter/generations 2>/dev/null || true",
        "chown root:vps-sub-meter /var/lib/vps-sub-meter /var/lib/vps-sub-meter/current.json /var/lib/vps-sub-meter/serve-auth.json 2>/dev/null || true",
        "chmod 1775 /var/lib/vps-sub-meter 2>/dev/null || true",
        "chown vps-sub-meter:vps-sub-meter /var/lib/vps-sub-meter/traffic-state.json /var/lib/vps-sub-meter/.operation.lock 2>/dev/null || true",
        "chown -R root:xray /etc/vps-sub-meter/xray 2>/dev/null || true",
        "chown -R root:sing-box /etc/vps-sub-meter/sing-box 2>/dev/null || true",
        "chown -R root:vsm-cert /var/lib/vps-sub-meter/certificates 2>/dev/null || true",
    ];
    for c in cmds {
        let _ = Command::new("sh").args(["-c", c]).status();
    }
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
        let cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..Default::default()
        };
        cfg.save(dir.path()).unwrap();

        let sec = Secrets::generate_for_test(&cfg);
        sec.save(dir.path()).unwrap();

        let report = apply_all(dir.path()).unwrap();
        assert_eq!(report.direct_nodes, 5);
        assert_eq!(report.singbox_nodes, 3);

        // Client generations must not leak server private keys.
        let clash = read_generation_file(dir.path(), &report.generation_id, "clash.yaml")
            .unwrap()
            .unwrap();
        assert!(!clash.contains(&sec.vless_reality_pqc.x25519_private_key));
        assert!(!clash.contains(&sec.vless_reality_pqc.mldsa65_seed));

        // serve-auth.json must store only token SHA-256 hashes, not raw tokens.
        let auth_text = fs::read_to_string(dir.path().join(SERVE_AUTH_REL)).unwrap();
        assert!(!auth_text.contains(sec.primary_token().unwrap()));

        // Systemd units use unified vps-sub-meter naming and underscore nftables hook.
        let sb_unit = fs::read_to_string(
            dir.path()
                .join("etc/systemd/system/vps-sub-meter-singbox.service"),
        )
        .unwrap();
        assert!(sb_unit.contains("net.sh apply-hy2 eth0 30000-35000 34443"));
    }
}
