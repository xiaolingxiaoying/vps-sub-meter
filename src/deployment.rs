//! Durable managed-deployment transaction. The public generation pointer is the
//! commit point. Historical rollback preserves current accounting and subscription
//! credentials; interrupted in-flight operations restore their locked snapshot.
use crate::{
    config::{DeploymentConfig, DeploymentStore},
    core::CoreKind,
    nodes::{self, Error, Registry, Result},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, process::Command};

pub const CURRENT: &str = "var/lib/sbctl/current-generation.json";
const JOURNAL: &str = "var/lib/sbctl/deployment-transaction.json";
const LAST: &str = "var/lib/sbctl/deployment-rollback.json";
const RECOVERY_UNIT: &str = "etc/systemd/system/sbctl-recover.service";
#[derive(Clone, Serialize, Deserialize)]
pub struct Generation {
    pub id: String,
    pub files: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    path: String,
    contents: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    files: Vec<Saved>,
    states: Vec<(String, bool, bool)>,
    applied_revision: u64,
    committed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback_config: Option<toml::Table>,
}
#[derive(Serialize)]
pub struct Plan {
    pub previous_mode: nodes::Mode,
    pub mode: nodes::Mode,
    pub client_version: serde_json::Value,
    pub details: Vec<serde_json::Value>,
    pub groups: Vec<serde_json::Value>,
    pub client_artifacts_changed: Vec<String>,
    pub client_artifacts_removed: Vec<String>,
    pub requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub restart: Vec<String>,
    pub artifacts: usize,
    pub interruption: bool,
}
pub fn binary(core: CoreKind) -> &'static str {
    match core {
        CoreKind::SingBox => "usr/local/bin/sing-box",
        CoreKind::Xray => "usr/local/bin/sbctl-xray",
    }
}
pub fn active(core: CoreKind) -> &'static str {
    match core {
        CoreKind::SingBox => "etc/sing-box/config.json",
        CoreKind::Xray => "etc/sbctl-xray/config.json",
    }
}
pub fn unit(core: CoreKind) -> &'static str {
    match core {
        CoreKind::SingBox => "sing-box.service",
        CoreKind::Xray => "sbctl-xray.service",
    }
}
fn io<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(Into::into)
}
fn json<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec_pretty(v).expect("typed document serializes")
}
fn optional(root: &Path, relative: &str) -> Result<Option<Vec<u8>>> {
    match crate::config::read_managed_file_no_follow(&root.join(relative)) {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn current(root: &Path) -> Result<Option<Generation>> {
    optional(root, CURRENT)?
        .map(|v| {
            let g: Generation =
                serde_json::from_slice(&v).map_err(|_| Error("订阅生成批次指针损坏".into()))?;
            if !nodes::identifier(&g.id)
                || g.files.iter().any(|n| {
                    crate::managed_fs::validate_file_name(std::ffi::OsStr::new(n)).is_err()
                })
            {
                return Err(Error("订阅生成批次指针无效".into()));
            }
            Ok(g)
        })
        .transpose()
}
pub fn read_generation(root: &Path, generation: &Generation, name: &str) -> Result<String> {
    if !generation.files.iter().any(|n| n == name) {
        return Err(Error("此订阅没有兼容节点或已删除".into()));
    }
    if crate::managed_fs::validate_file_name(std::ffi::OsStr::new(name)).is_err() {
        return Err(Error("非法工件名称".into()));
    }
    let bytes = io(crate::config::read_managed_file_no_follow(&root.join(
        format!("var/lib/sbctl/generations/{}/{name}", generation.id),
    )))?;
    String::from_utf8(bytes).map_err(|_| Error("订阅工件不是 UTF-8".into()))
}
pub fn read_published(root: &Path, name: &str) -> Result<Option<String>> {
    current(root)?
        .map(|g| read_generation(root, &g, name))
        .transpose()
}
fn changed_cores(root: &Path, artifacts: &[(String, String)]) -> Result<Vec<CoreKind>> {
    let mut result = Vec::new();
    for core in [CoreKind::SingBox, CoreKind::Xray] {
        let target = artifacts
            .iter()
            .find(|(name, _)| name == crate::subscription::managed::server_name(core))
            .map(|(_, s)| s.as_bytes());
        if optional(root, active(core))?.as_deref() != target {
            result.push(core);
        }
    }
    Ok(result)
}
pub fn plan(
    root: &Path,
    config: &DeploymentConfig,
    before: &Registry,
    after: &Registry,
) -> Result<Plan> {
    after.validate(config)?;
    let artifacts = crate::subscription::managed::generate(config, root, after)
        .map_err(|e| Error(e.to_string()))?;
    let cores = changed_cores(root, &artifacts)?;
    let summary = |n: &nodes::Node| {
        let (server, sni) = match &n.data {
            nodes::Data::Legacy(crate::canonical::CanonicalNode::VlessReality {
                host,
                decoy_sni,
                ..
            }) => (host.as_str(), decoy_sni.as_str()),
            nodes::Data::Legacy(v) => (v.host(), v.tls_server_name()),
            nodes::Data::Combination(v) => (v.server.as_str(), v.server_name.as_str()),
        };
        serde_json::json!({"id":n.id,"name":n.name,"preset":n.preset(),"core":n.core,"enabled":n.enabled,"listen":n.listen,"server":server,"port":n.port(),"server_name":sni})
    };
    let ids: std::collections::BTreeSet<_> = before
        .nodes
        .iter()
        .chain(&after.nodes)
        .map(|n| n.id.as_str())
        .collect();
    let details=ids.into_iter().filter_map(|id|{let old=before.nodes.iter().find(|n|n.id==id);let new=after.nodes.iter().find(|n|n.id==id);(old!=new).then(||serde_json::json!({"before":old.map(summary),"after":new.map(summary),"protocol_settings_changed":old.map(|n|&n.data)!=new.map(|n|&n.data)}))}).collect();
    let group_ids: std::collections::BTreeSet<_> = before
        .groups
        .iter()
        .chain(&after.groups)
        .map(|g| g.id.as_str())
        .collect();
    let groups = group_ids
        .into_iter()
        .filter_map(|id| {
            let old = before.groups.iter().find(|g| g.id == id);
            let new = after.groups.iter().find(|g| g.id == id);
            (old != new).then(|| serde_json::json!({"before":old,"after":new}))
        })
        .collect();
    let old_generation = current(root).ok().flatten();
    let clients: Vec<_> = artifacts
        .iter()
        .filter(|(name, _)| !name.ends_with("-server.json"))
        .collect();
    let client_artifacts_changed = clients
        .iter()
        .filter(|(name, text)| {
            old_generation
                .as_ref()
                .and_then(|g| read_generation(root, g, name).ok())
                .is_none_or(|old| old != *text)
        })
        .map(|(name, _)| name.clone())
        .collect();
    let client_artifacts_removed = old_generation
        .as_ref()
        .map(|g| {
            g.files
                .iter()
                .filter(|name| !clients.iter().any(|(n, _)| n == *name))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut requirements = Vec::new();
    if root == Path::new("/") {
        for core in [CoreKind::SingBox, CoreKind::Xray] {
            if !after.mode.allows(core) {
                continue;
            }
            let path = root.join(binary(core));
            if !path.is_file() {
                requirements.push(format!("先执行 core install {core} 安装已验证内核"));
            } else if let Err(e) = validate_core_version(core, &path, after) {
                requirements.push(e.to_string());
            }
        }
    }
    let mut warnings = Vec::new();
    if after.nodes.iter().any(|n| n.enabled) {
        let missing: Vec<&str> = [
            ("sing-box", "subscription-sing-box-full.json"),
            ("clash", "subscription-clash.yaml"),
            ("shadowrocket", "subscription-shadowrocket.txt"),
            ("uri", "subscription-uri.txt"),
        ]
        .into_iter()
        .filter(|(_, artifact)| !clients.iter().any(|(name, _)| name == artifact))
        .map(|(label, _)| label)
        .collect();
        if !missing.is_empty() {
            warnings.push(format!(
                "当前启用节点仅生成部分客户端订阅（缺失: {}）；例如 Xray 下的 vless-reality / vless-xhttp 节点会从不兼容客户端保守排除。如需全客户端互通，建议启用 vless-ws-tls 或 trojan-tcp-tls",
                missing.join(", ")
            ));
        }
    }
    Ok(Plan {
        previous_mode: before.mode,
        mode: after.mode,
        client_version: serde_json::json!({"before":before.client_version,"after":after.client_version}),
        details,
        groups,
        client_artifacts_changed,
        client_artifacts_removed,
        requirements,
        warnings,
        added: after
            .nodes
            .iter()
            .filter(|n| !before.nodes.iter().any(|b| b.id == n.id))
            .map(|n| n.id.clone())
            .collect(),
        removed: before
            .nodes
            .iter()
            .filter(|n| !after.nodes.iter().any(|b| b.id == n.id))
            .map(|n| n.id.clone())
            .collect(),
        changed: after
            .nodes
            .iter()
            .filter(|n| before.nodes.iter().any(|b| b.id == n.id && b != *n))
            .map(|n| n.id.clone())
            .collect(),
        restart: cores.iter().map(|c| unit(*c).into()).collect(),
        artifacts: artifacts.len(),
        interruption: !cores.is_empty(),
    })
}
fn systemctl(root: &Path, args: &[&str]) -> Result<()> {
    // Isolated deployment roots historically skip data-plane systemd calls.
    // A dedicated fixture executable opts into recording these transactions;
    // do not reuse lifecycle's systemctl stub, which has a different contract.
    let status = if root == Path::new("/") {
        Command::new("systemctl").args(args).status()
    } else {
        let fixture = root.join("usr/bin/sbctl-deployment-systemctl");
        if !fixture.is_file() {
            return Ok(());
        }
        Command::new(fixture).args(args).status()
    }
    .map_err(|_| Error("无法执行 systemctl".into()))?;
    if !status.success() {
        return Err(Error(format!(
            "systemctl {} 失败",
            args.first().unwrap_or(&"operation")
        )));
    }
    Ok(())
}
fn state(root: &Path, name: &str, verb: &str) -> bool {
    root == Path::new("/")
        && Command::new("systemctl")
            .args([verb, "--quiet", name])
            .status()
            .is_ok_and(|s| s.success())
}
fn owned(root: &Path, core: CoreKind) -> Result<()> {
    let path = format!("etc/systemd/system/{}", unit(core));
    if let Some(bytes) = optional(root, &path)? {
        let text = String::from_utf8_lossy(&bytes);
        if !(text.contains("managed by sbctl") || text.contains("sbctl managed protocol companion"))
        {
            return Err(Error(format!("{} 不属于 sbctl，拒绝接管", unit(core))));
        }
    } else if root == Path::new("/")
        && root.join(binary(core)).exists()
        && !root.join("var/lib/sbctl/ownership").is_file()
    {
        return Err(Error("已有内核二进制没有托管归属证明，拒绝接管".into()));
    }
    Ok(())
}
fn unit_text(core: CoreKind, hopping: bool) -> String {
    let (program, config) = (binary(core), active(core));
    let extra = if core == CoreKind::SingBox && hopping {
        "ExecStartPre=+/usr/local/bin/sbctl sing-box port-hopping-firewall\nExecStopPost=+/usr/local/bin/sbctl sing-box port-hopping-firewall --cleanup\n"
    } else {
        ""
    };
    format!(
        "[Unit]\nDescription={core} data plane managed by sbctl\nAfter=network-online.target sbctl-recover.service\nWants=network-online.target\nRequires=sbctl-recover.service\n\n[Service]\nType=simple\nUser=sing-box\nGroup=sing-box\nExecStart=/{program} run -c /{config}\n{extra}Restart=on-failure\nRestartSec=2\nNoNewPrivileges=true\nPrivateTmp=true\nProtectHome=true\nProtectSystem=strict\n\n[Install]\nWantedBy=multi-user.target\n"
    ).replace("NoNewPrivileges=true\n", "AmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE\nNoNewPrivileges=true\n")
}
fn health(root: &Path, name: &str) -> Result<()> {
    if root != Path::new("/") {
        return Ok(());
    }
    for _ in 0..3 {
        systemctl(root, &["is-active", "--quiet", name])?;
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    Ok(())
}

fn validate_core_version(core: CoreKind, binary: &Path, registry: &Registry) -> Result<()> {
    let output = Command::new(binary)
        .arg("version")
        .output()
        .map_err(|_| Error(format!("无法识别 {core} 版本")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = if core == CoreKind::Xray {
        text.split_whitespace().nth(1)
    } else {
        text.split_whitespace().nth(2)
    }
    .unwrap_or("");
    let known = if core == CoreKind::Xray {
        version == crate::core::xray::BASELINE_VERSION
    } else {
        [
            "1.10.7", "1.11.15", "1.12.25", "1.13.21", "1.14.1", "1.14.2", "1.14.4",
        ]
        .contains(&version)
    };
    if !output.status.success() || !known {
        return Err(Error(format!(
            "{core} 版本尚未验证，请选择 core install 提供的固定版本"
        )));
    }
    if core == CoreKind::SingBox
        && (version.starts_with("1.10.") || version.starts_with("1.11."))
        && registry.nodes.iter().any(|n| {
            n.enabled
                && n.core == core
                && matches!(
                    n.data,
                    nodes::Data::Legacy(crate::canonical::CanonicalNode::Anytls { .. })
                )
        })
    {
        return Err(Error("sing-box 1.12 之前不能运行 AnyTLS 节点".into()));
    }
    Ok(())
}
fn journal(store: &DeploymentStore, record: &Journal) -> Result<()> {
    store.write_root_only_locked(JOURNAL, &json(record))?;
    Ok(())
}
fn snapshot(root: &Path, paths: &[String], revision: u64) -> Result<Journal> {
    let files = paths
        .iter()
        .map(|p| {
            Ok(Saved {
                path: p.clone(),
                contents: optional(root, p)?
                    .map(|b| base64::engine::general_purpose::STANDARD.encode(b)),
            })
        })
        .collect::<Result<_>>()?;
    let states = [
        unit(CoreKind::SingBox),
        unit(CoreKind::Xray),
        "sbctl.service",
        "sbctl-http.socket",
    ]
    .into_iter()
    .map(|name| {
        (
            name.into(),
            state(root, name, "is-enabled"),
            state(root, name, "is-active"),
        )
    })
    .collect();
    Ok(Journal {
        files,
        states,
        applied_revision: revision,
        committed: false,
        rollback_config: None,
    })
}
fn restore(store: &DeploymentStore, record: &Journal, boot: bool) -> Result<()> {
    let root = store.root();
    let affected: Vec<_> = [CoreKind::SingBox, CoreKind::Xray]
        .into_iter()
        .filter(|c| {
            record.files.iter().any(|f| {
                f.path == active(*c)
                    || f.path == binary(*c)
                    || f.path == format!("etc/systemd/system/{}", unit(*c))
            })
        })
        .collect();
    let mut affected_units: Vec<&str> = affected.iter().map(|c| unit(*c)).collect();
    for name in ["sbctl.service", "sbctl-http.socket"] {
        if record
            .files
            .iter()
            .any(|f| f.path == format!("etc/systemd/system/{name}"))
        {
            affected_units.push(name);
        }
    }
    // Stop only owned data planes; never stop the subscription service.
    if !boot {
        for c in &affected {
            owned(root, *c)?;
            if root
                .join(format!("etc/systemd/system/{}", unit(*c)))
                .exists()
            {
                systemctl(root, &["stop", unit(*c)])?;
            }
        }
    }
    // Disable before removing a newly-created unit, while systemd can still
    // resolve its [Install] section and remove the corresponding wants links.
    for (name, enabled, _) in &record.states {
        if affected_units.contains(&name.as_str())
            && !enabled
            && root.join(format!("etc/systemd/system/{name}")).exists()
        {
            systemctl(root, &["disable", name])?;
        }
        if boot
            && affected_units.contains(&name.as_str())
            && !enabled
            && !record
                .states
                .iter()
                .any(|(n, _, active)| n == name && *active)
            && root.join(format!("etc/systemd/system/{name}")).exists()
        {
            systemctl(root, &["--no-block", "stop", name])?;
        }
    }
    let subscription_changed = record.files.iter().any(|f| {
        f.path == "usr/local/bin/sbctl"
            || f.path == "etc/systemd/system/sbctl.service"
            || f.path == "etc/systemd/system/sbctl-http.socket"
            || (f.path == crate::config::CONFIG_RELATIVE_PATH
                && optional(root, &f.path)
                    .ok()
                    .flatten()
                    .map(|b| base64::engine::general_purpose::STANDARD.encode(b))
                    != f.contents)
    });
    for entry in &record.files {
        // Journal paths are fixed owned paths, never arbitrary JSON path input.
        if !allowed_snapshot(&entry.path) {
            return Err(Error("恢复记录包含非托管路径".into()));
        }
        if let Some(encoded) = &entry.contents {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| Error("恢复记录损坏".into()))?;
            if entry.path == nodes::SOURCE {
                store.write_root_only_locked(&entry.path, &bytes)?;
            } else {
                store.write_relative_locked(&entry.path, &bytes)?;
            }
        } else if optional(root, &entry.path)?.is_some() {
            io(crate::managed_fs::remove_file(root.join(&entry.path)))?;
        }
    }
    let restored_config = if root.join(crate::config::CONFIG_RELATIVE_PATH).is_file() {
        Some(store.load()?)
    } else {
        None
    };
    if root == Path::new("/")
        && record
            .files
            .iter()
            .any(|f| f.path == crate::config::CONFIG_RELATIVE_PATH)
        && root.join("etc/systemd/system/sbctl.service").is_file()
        && let Some(config) = &restored_config
    {
        // A delivery-mode change also changes certificate group access. Restore
        // that policy before starting either non-root consumer of the old files.
        crate::lifecycle::prepare_daemon_storage(
            root,
            config.subscription_mode == crate::config::SubscriptionMode::Direct,
        )
        .map_err(Error)?;
    }
    if crate::port_hopping::has_firewall_runtime(root) && affected.contains(&CoreKind::SingBox) {
        let hopping = if let Some(config) = &restored_config {
            nodes::read_optional(root)?.map_or(config.hysteria2_port_hopping.is_some(), |r| {
                r.has_active_sing_box_hysteria2(config)
            })
        } else {
            false
        };
        crate::port_hopping::apply_firewall_config(
            root,
            restored_config.as_ref().filter(|_| hopping),
        )
        .map_err(Error)?;
    }
    systemctl(root, &["daemon-reload"])?;
    for (name, enabled, active) in &record.states {
        if !affected_units.contains(&name.as_str()) {
            continue;
        }
        if *enabled {
            systemctl(root, &["enable", name])?;
        }
        if boot && (*enabled || *active) {
            // The units are ordered After this recovery oneshot. Waiting
            // synchronously here would deadlock the boot transaction.
            if name != "sbctl.service"
                || store.load()?.subscription_mode != crate::config::SubscriptionMode::Direct
            {
                systemctl(root, &["--no-block", "start", name])?;
            }
        } else if !boot && *active && affected.iter().any(|c| unit(*c) == name) {
            systemctl(root, &["start", name])?;
            health(root, name)?;
        }
    }
    if !boot && subscription_changed {
        crate::lifecycle::restart_subscription_service(root).map_err(Error)?;
    }
    Ok(())
}
fn allowed_snapshot(path: &str) -> bool {
    path == crate::config::CONFIG_RELATIVE_PATH
        || path == "usr/local/bin/sbctl"
        || path == nodes::SOURCE
        || path == CURRENT
        || path == RECOVERY_UNIT
        || path == "etc/systemd/system/sbctl.service"
        || path == "etc/systemd/system/sbctl-http.socket"
        || path == crate::lifecycle::CERTBOT_DEPLOY_HOOK_RELATIVE_PATH
        || path == "var/lib/sbctl/artifacts/sing-box-server.json"
        || [CoreKind::SingBox, CoreKind::Xray].into_iter().any(|c| {
            path == active(c)
                || path == binary(c)
                || path == format!("etc/systemd/system/{}", unit(c))
        })
}
pub fn recover(store: &DeploymentStore, boot: bool) -> Result<bool> {
    if boot {
        let guard = io(crate::managed_fs::open_lock(
            &store.root().join("var/lib/sbctl"),
        ))?;
        match fs2::FileExt::try_lock_exclusive(&guard) {
            Ok(()) => return recover_locked(store, true),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e.into()),
        }
    }
    let _lock = store.acquire_operation_lock()?;
    recover_locked(store, boot)
}
fn recover_locked(store: &DeploymentStore, boot: bool) -> Result<bool> {
    let Some(bytes) = optional(store.root(), JOURNAL)? else {
        return Ok(false);
    };
    let record: Journal = serde_json::from_slice(&bytes)
        .map_err(|_| Error("事务恢复记录损坏；需要人工修复".into()))?;
    if !record.committed {
        restore(store, &record, boot)
            .map_err(|e| Error(format!("需要修复：{e}；恢复记录已保留")))?;
    }
    io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
    Ok(true)
}

/// Bootstrap only the ACME subscription listener. Keep the running data planes,
/// registry and published clients untouched until a domain certificate exists.
pub fn begin_subscription_bootstrap_locked(
    store: &DeploymentStore,
    config: &DeploymentConfig,
) -> Result<()> {
    recover_locked(store, false)?;
    let paths = [
        crate::config::CONFIG_RELATIVE_PATH,
        "etc/systemd/system/sbctl.service",
        "etc/systemd/system/sbctl-http.socket",
        crate::lifecycle::CERTBOT_DEPLOY_HOOK_RELATIVE_PATH,
    ]
    .map(str::to_owned);
    let record = snapshot(store.root(), &paths, 0)?;
    journal(store, &record)?;
    store.replace_locked(config)?;
    Ok(())
}
pub fn finish_subscription_bootstrap_locked(store: &DeploymentStore) -> Result<()> {
    recover_locked(store, false)?;
    Ok(())
}
/// Caller holds the same operation lock as legacy credential/config writers.
pub fn apply_locked(
    store: &DeploymentStore,
    config: &DeploymentConfig,
    expected: &Registry,
    next: Registry,
) -> Result<Plan> {
    apply_with_config_locked(store, config, config, expected, next)
}
fn apply_with_config_locked(
    store: &DeploymentStore,
    expected_config: &DeploymentConfig,
    config: &DeploymentConfig,
    expected: &Registry,
    mut next: Registry,
) -> Result<Plan> {
    recover_locked(store, false)?;
    if store.load()? != *expected_config
        || nodes::load_or_import(store.root(), expected_config)? != *expected
    {
        return Err(Error("配置已变化，请重新打开编辑或重新生成计划".into()));
    }
    next.revision = expected
        .revision
        .checked_add(1)
        .ok_or_else(|| Error("节点仓库修订号溢出".into()))?;
    next.validate(config)?;
    let root = store.root();
    // Before a new transaction: retain its original generation through any
    // outer activation rollback, and give in-flight readers a grace period.
    if let Err(error) = prune_generations(root) {
        eprintln!("warning: 旧订阅批次清理失败：{error}");
    }
    // Certificate creation belongs to the locked apply path, never to plan.
    crate::subscription::render::certificate_tls_config(config, root)
        .map_err(|e| Error(e.to_string()))?;
    let plan = plan(root, config, expected, &next)?;
    let artifacts = crate::subscription::managed::generate(config, root, &next)
        .map_err(|e| Error(e.to_string()))?;
    let changed = changed_cores(root, &artifacts)?;
    for c in [CoreKind::SingBox, CoreKind::Xray] {
        owned(root, c)?;
    }
    // Validate every selected core before stopping any running service. Helper
    // roots may omit binaries, but a live deployment never skips validation.
    for c in [CoreKind::SingBox, CoreKind::Xray] {
        if let Some((_, text)) = artifacts
            .iter()
            .find(|(n, _)| n == crate::subscription::managed::server_name(c))
        {
            let path = root.join(binary(c));
            if path.is_file() {
                if root == Path::new("/") {
                    validate_core_version(c, &path, &next)?;
                }
                c.check_config(&path, text)
                    .map_err(|_| Error(format!("{c} 配置检查失败；旧服务尚未停止")))?;
            } else if root == Path::new("/") {
                return Err(Error(format!(
                    "先执行 core install {} 安装已验证内核",
                    if c == CoreKind::SingBox {
                        "sing-box"
                    } else {
                        "xray"
                    }
                )));
            }
        }
    }
    check_ports(root, expected_config, config, expected, &next)?;
    validate_current_client(root, &next, &artifacts)?;
    let paths = snapshot_paths();
    let mut record = snapshot(root, &paths, next.revision)?;
    let id = nodes::new_id();
    let generation = Generation {
        id: id.clone(),
        files: artifacts
            .iter()
            .filter(|(n, _)| !n.ends_with("-server.json"))
            .map(|(n, _)| n.clone())
            .collect(),
    };
    // Complete immutable generation, unreachable from HTTP until commit.
    for (name, text) in &artifacts {
        if name.ends_with("-server.json") {
            continue;
        }
        store.write_relative_locked(
            &format!("var/lib/sbctl/generations/{id}/{name}"),
            text.as_bytes(),
        )?;
    }
    protect_generation(store, &id)?;
    journal(store, &record)?;
    let outcome = (|| -> Result<()> {
        store.write_root_only_locked(nodes::SOURCE, &json(&next))?;
        if root == Path::new("/") {
            crate::lifecycle::prepare_daemon_prerequisites(
                root,
                config.subscription_mode == crate::config::SubscriptionMode::Direct,
            )
            .map_err(Error)?;
        }
        let active_hopping = next.has_active_sing_box_hysteria2(config);
        let sing_box_unit_changed = next.mode.allows(CoreKind::SingBox)
            && optional(
                root,
                &format!("etc/systemd/system/{}", unit(CoreKind::SingBox)),
            )?
            .as_deref()
                != Some(unit_text(CoreKind::SingBox, active_hopping).as_bytes());
        for c in [CoreKind::SingBox, CoreKind::Xray] {
            let needs_stop =
                changed.contains(&c) || (c == CoreKind::SingBox && sing_box_unit_changed);
            if needs_stop
                && root
                    .join(format!("etc/systemd/system/{}", unit(c)))
                    .exists()
            {
                systemctl(root, &["stop", unit(c)])?;
            }
        }
        for c in [CoreKind::SingBox, CoreKind::Xray] {
            if next.mode.allows(c) {
                store.write_relative_locked(
                    &format!("etc/systemd/system/{}", unit(c)),
                    unit_text(c, active_hopping).as_bytes(),
                )?;
                let text = &artifacts
                    .iter()
                    .find(|(n, _)| n == crate::subscription::managed::server_name(c))
                    .unwrap()
                    .1;
                store.write_relative_locked(active(c), text.as_bytes())?;
            } else if root
                .join(format!("etc/systemd/system/{}", unit(c)))
                .exists()
            {
                systemctl(root, &["disable", unit(c)])?;
                io(crate::managed_fs::remove_file(root.join(active(c)))).or_else(|_| {
                    if optional(root, active(c))?.is_none() {
                        Ok(())
                    } else {
                        Err(Error("旧内核配置移除失败".into()))
                    }
                })?;
            }
        }
        store.write_relative_locked(RECOVERY_UNIT,b"[Unit]\nDescription=Recover interrupted deployment managed by sbctl\nDefaultDependencies=no\nAfter=local-fs.target\nBefore=sing-box.service sbctl-xray.service sbctl.service\n\n[Service]\nType=oneshot\nExecStart=/usr/local/bin/sbctl core recover --boot\nRemainAfterExit=yes\n\n[Install]\nWantedBy=multi-user.target\n")?;
        systemctl(root, &["daemon-reload"])?;
        systemctl(root, &["enable", "sbctl-recover.service"])?;
        if root == Path::new("/") {
            crate::lifecycle::prepare_daemon_storage(
                root,
                config.subscription_mode == crate::config::SubscriptionMode::Direct,
            )
            .map_err(Error)?;
        }
        // Pre-start hooks must read the candidate configuration. Range-only
        // changes do not change server bytes, so reconcile nftables explicitly.
        if expected_config != config {
            store.replace_locked(config)?;
        }
        if crate::port_hopping::has_firewall_runtime(root) {
            crate::port_hopping::apply_firewall_config(root, active_hopping.then_some(config))
                .map_err(Error)?;
        }
        for c in [CoreKind::SingBox, CoreKind::Xray] {
            if next.mode.allows(c) {
                systemctl(root, &["enable", unit(c)])?;
                if changed.contains(&c)
                    || (c == CoreKind::SingBox && sing_box_unit_changed)
                    || !state(root, unit(c), "is-active")
                {
                    systemctl(root, &["start", unit(c)])?;
                    health(root, unit(c))?;
                } else {
                    systemctl(root, &["is-active", "--quiet", unit(c)])?;
                }
            }
        }
        store.write_relative_locked(CURRENT, &json(&generation))?;
        record.committed = true;
        if let Err(e) = journal(store, &record) {
            record.committed = false;
            return Err(e);
        }
        let mut historical = record.clone();
        historical
            .files
            .retain(|f| f.path != crate::config::CONFIG_RELATIVE_PATH);
        historical.rollback_config = Some(rollback_settings(expected_config)?);
        if store
            .write_root_only_locked(LAST, &json(&historical))
            .is_err()
        {
            eprintln!("warning: 部署已提交，但最近恢复点未能保存");
        }
        if crate::managed_fs::remove_file(root.join(JOURNAL)).is_err() {
            eprintln!("warning: 已提交事务记录留待下次清理");
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        restore(store, &record, false)
            .map_err(|r| Error(format!("应用失败：{e}；需要修复：{r}；执行 core recover")))?;
        io(crate::managed_fs::remove_file(root.join(JOURNAL)))?;
        return Err(e);
    }
    Ok(plan)
}
fn snapshot_paths() -> Vec<String> {
    [
        nodes::SOURCE,
        CURRENT,
        RECOVERY_UNIT,
        crate::config::CONFIG_RELATIVE_PATH,
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(
        [CoreKind::SingBox, CoreKind::Xray]
            .into_iter()
            .flat_map(|c| [active(c).into(), format!("etc/systemd/system/{}", unit(c))]),
    )
    .collect()
}

fn prune_generations(root: &Path) -> Result<()> {
    let mut retained = std::collections::BTreeSet::new();
    if let Some(generation) = current(root)? {
        retained.insert(generation.id);
    }
    for name in [LAST, JOURNAL] {
        if let Some(bytes) = optional(root, name)? {
            let record: Journal = serde_json::from_slice(&bytes)
                .map_err(|_| Error("恢复记录损坏，暂停清理批次".into()))?;
            for saved in record.files.iter().filter(|f| f.path == CURRENT) {
                if let Some(contents) = &saved.contents {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(contents)
                        .map_err(|_| Error("恢复批次指针损坏".into()))?;
                    let generation: Generation = serde_json::from_slice(&bytes)
                        .map_err(|_| Error("恢复批次指针损坏".into()))?;
                    retained.insert(generation.id);
                }
            }
        }
    }
    let directory = root.join("var/lib/sbctl/generations");
    let names = match crate::managed_fs::directory_entries(&directory) {
        Ok(names) => names,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for name in names {
        let Some(id) = name.to_str() else { continue };
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || retained.contains(id) {
            continue;
        }
        let path = directory.join(&name);
        let metadata = io(crate::managed_fs::directory_metadata(&path))?;
        if !metadata.is_dir()
            || io(metadata.modified())?.elapsed().unwrap_or_default()
                < std::time::Duration::from_secs(300)
        {
            continue;
        }
        io(crate::managed_fs::remove_flat_directory(&path))?;
    }
    Ok(())
}
pub fn apply_config_change_locked(
    store: &DeploymentStore,
    new: &DeploymentConfig,
) -> Result<Vec<u8>> {
    let old = store.load()?;
    let registry = nodes::load_or_import(store.root(), &old)?;
    if old.enabled_protocols != new.enabled_protocols
        || old.vless_reality != new.vless_reality
        || old.vmess_websocket != new.vmess_websocket
        || old.hysteria2 != new.hysteria2
        || old.tuic != new.tuic
        || old.anytls != new.anytls
    {
        return Err(Error(
            "已迁移部署的节点由 nodes.json 管理；请使用 node 命令修改协议、端口和凭据".into(),
        ));
    }
    let mut next = registry.clone();
    let old_host = old.proxy_host.as_deref().unwrap_or(&old.subscription_host);
    let new_host = new.proxy_host.as_deref().unwrap_or(&new.subscription_host);
    let old_sni = old.protocol_server_name();
    let new_sni = new.protocol_server_name();
    for node in &mut next.nodes {
        if old_host != new_host {
            let current_host = match &node.data {
                nodes::Data::Legacy(n) => n.host(),
                nodes::Data::Combination(n) => n.server.as_str(),
            };
            if current_host == old_host {
                node.patch("server", serde_json::json!(new_host))?;
            }
        }
        let is_reality = matches!(
            &node.data,
            nodes::Data::Legacy(crate::canonical::CanonicalNode::VlessReality { .. })
        ) || matches!(&node.data, nodes::Data::Combination(n) if n.preset.is_reality());
        if !is_reality
            && (old_sni != new_sni
                || new.certificate_mode == crate::config::CertificateMode::Domain)
        {
            let current_sni = match &node.data {
                nodes::Data::Legacy(n) => n.tls_server_name(),
                nodes::Data::Combination(n) => n.server_name.as_str(),
            };
            if current_sni == old_sni
                || new.certificate_mode == crate::config::CertificateMode::Domain
            {
                node.patch("server_name", serde_json::json!(new_sni))?;
            }
        }
    }
    let backup = snapshot(store.root(), &snapshot_paths(), registry.revision + 1)?;
    apply_with_config_locked(store, &old, new, &registry, next)?;
    Ok(json(&backup))
}
pub fn restore_snapshot_locked(store: &DeploymentStore, bytes: &[u8]) -> Result<()> {
    let record: Journal =
        serde_json::from_slice(bytes).map_err(|_| Error("恢复记录损坏".into()))?;
    restore(store, &record, false)
}
pub fn apply(
    store: &DeploymentStore,
    config: &DeploymentConfig,
    expected: &Registry,
    next: Registry,
) -> Result<Plan> {
    let _lock = store.acquire_operation_lock()?;
    apply_locked(store, config, expected, next)
}
pub fn apply_config_and_nodes(
    store: &DeploymentStore,
    expected_config: &DeploymentConfig,
    config: &DeploymentConfig,
    expected: &Registry,
    next: Registry,
) -> Result<Plan> {
    let _lock = store.acquire_operation_lock()?;
    config.validate()?;
    apply_with_config_locked(store, expected_config, config, expected, next)
}
fn check_ports(
    root: &Path,
    previous_config: &DeploymentConfig,
    config: &DeploymentConfig,
    before: &Registry,
    next: &Registry,
) -> Result<()> {
    if root != Path::new("/") {
        return Ok(());
    }
    for node in next.nodes.iter().filter(|n| n.enabled) {
        if before.nodes.iter().any(|n| {
            n.enabled
                && crate::shared_tls::binding(previous_config, n).port()
                    == crate::shared_tls::binding(config, node).port()
                && crate::catalog::udp(&n.preset()) == crate::catalog::udp(&node.preset())
        }) {
            continue;
        }
        let socket = crate::shared_tls::binding(config, node);
        if crate::catalog::udp(&node.preset()) {
            let _guard = std::net::UdpSocket::bind(socket)
                .map_err(|_| Error(format!("UDP {} 已被占用", socket.port())))?;
        } else {
            let _guard = std::net::TcpListener::bind(socket)
                .map_err(|_| Error(format!("TCP {} 已被占用", socket.port())))?;
        }
    }
    Ok(())
}
fn protect_generation(store: &DeploymentStore, id: &str) -> Result<()> {
    if store.root() != Path::new("/") {
        return Ok(());
    }
    for path in [
        "var/lib/sbctl/generations".to_string(),
        format!("var/lib/sbctl/generations/{id}"),
    ] {
        crate::config::set_owner_mode_no_follow(
            store.root(),
            &store.root().join(path),
            "root:sbctl",
            0o750,
        )
        .map_err(|_| Error("订阅生成目录权限设置失败".into()))?;
    }
    Ok(())
}
pub fn rollback(store: &DeploymentStore) -> Result<()> {
    let _lock = store.acquire_operation_lock()?;
    recover_locked(store, false)?;
    let bytes = optional(store.root(), LAST)?.ok_or_else(|| Error("没有可用恢复点".into()))?;
    let mut record: Journal =
        serde_json::from_slice(&bytes).map_err(|_| Error("恢复点损坏".into()))?;
    let current_revision = nodes::read_optional(store.root())?.map_or(0, |r| r.revision);
    if current_revision != record.applied_revision {
        return Err(Error("恢复点已过期，拒绝覆盖后续修改".into()));
    }
    let mut config =
        toml::Table::try_from(store.load()?).map_err(|_| Error("部署配置序列化失败".into()))?;
    if let Some(settings) = record.rollback_config.take() {
        for key in ROLLBACK_SETTINGS {
            config.remove(*key);
        }
        config.extend(settings);
    }
    let config_text = toml::to_string(&config).map_err(|_| Error("恢复配置无效".into()))?;
    let restored: DeploymentConfig =
        toml::from_str(&config_text).map_err(|_| Error("恢复配置无效".into()))?;
    restored.validate()?;
    if let Some(saved) = record.files.iter().find(|f| f.path == nodes::SOURCE)
        && let Some(contents) = &saved.contents
    {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(contents)
            .map_err(|_| Error("恢复点损坏".into()))?;
        let registry: Registry =
            serde_json::from_slice(&bytes).map_err(|_| Error("恢复节点仓库无效".into()))?;
        registry.validate(&restored)?;
    }
    record.files.push(Saved {
        path: crate::config::CONFIG_RELATIVE_PATH.into(),
        contents: Some(base64::engine::general_purpose::STANDARD.encode(config_text)),
    });
    record.committed = false;
    journal(store, &record)?;
    restore(store, &record, false)?;
    io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
    io(crate::managed_fs::remove_file(store.root().join(LAST)))?;
    Ok(())
}

// Only deployment settings roll back. Subscription credentials, accounting
// settings/state and legacy import credentials are never historical authority.
const ROLLBACK_SETTINGS: &[&str] = &[
    "shared_tls",
    "subscription_mode",
    "subscription_host",
    "proxy_host",
    "http_port",
    "subscription_listen_port",
    "certbot_email",
    "certificate_mode",
    "protocol_sni",
    "ipv4_only",
    "hysteria2_port_hopping",
    "server_clash_api",
    "client_clash_api_secret",
    "client_dns_mode",
    "client_dns_preset",
    "client_rule_profile",
    "client_template",
    "client_rule_set_base_url",
    "client_extra_rule_sets",
    "client_rule_set_update_interval",
    "client_latency_probe_url",
];
fn rollback_settings(config: &DeploymentConfig) -> Result<toml::Table> {
    let mut settings =
        toml::Table::try_from(config).map_err(|_| Error("部署配置序列化失败".into()))?;
    settings.retain(|key, _| ROLLBACK_SETTINGS.contains(&key));
    Ok(settings)
}
pub fn status(root: &Path) -> Result<serde_json::Value> {
    let registry = nodes::read_optional(root)?;
    let mut cores = BTreeMap::new();
    for core in [CoreKind::SingBox, CoreKind::Xray] {
        let mut properties = BTreeMap::new();
        if root == Path::new("/") {
            let output=Command::new("systemctl").args(["show",unit(core),"--property=ActiveState,SubState,MainPID,MemoryCurrent,CPUUsageNSec,ExecMainStatus"]).output().map_err(|_|Error("内核状态查询失败".into()))?;
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((k, v)) = line.split_once('=') {
                    properties.insert(k.to_string(), v.to_string());
                }
            }
        }
        let path = root.join(binary(core));
        let version = if path.is_file() {
            Command::new(&path)
                .arg("version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_owned()
                })
        } else {
            None
        };
        cores.insert(core.to_string(),serde_json::json!({"installed":path.is_file(),"version":version,"service":properties,"nodes":registry.as_ref().map(|r|r.nodes.iter().filter(|n|n.enabled&&n.core==core).map(|n|serde_json::json!({"id":n.id,"port":n.port()})).collect::<Vec<_>>())}));
    }
    Ok(
        serde_json::json!({"mode":registry.as_ref().map(|r|r.mode),"needs_repair":optional(root,JOURNAL)?.is_some(),"cores":cores}),
    )
}
pub fn fingerprint(registry: &Registry) -> String {
    format!("{:x}", Sha256::digest(json(registry)))
}

fn validate_current_client(
    root: &Path,
    registry: &Registry,
    artifacts: &[(String, String)],
) -> Result<()> {
    let explicit = root.join(format!(
        "usr/local/lib/sbctl/validators/sing-box-{}",
        registry.client_version
    ));
    let server = root.join(binary(CoreKind::SingBox));
    let validator = if explicit.is_file() {
        Some(explicit)
    } else {
        crate::subscription::installed_kernel_version(&server)
            .filter(|v| v.to_string() == registry.client_version)
            .map(|_| server)
    };
    let documents: Vec<_> = artifacts
        .iter()
        .filter(|(name, _)| name.ends_with("subscription-sing-box-full.json"))
        .collect();
    if documents.is_empty() {
        return Ok(());
    }
    if let Some(binary) = validator {
        for (_, text) in documents {
            CoreKind::SingBox.check_config(&binary, text).map_err(|_| {
                Error("最终客户端完整配置没有通过对应 sing-box 内核检查；旧批次保留".into())
            })?;
        }
    } else {
        eprintln!(
            "客户端配置已做结构、引用和版本能力检查；本机没有对应版本校验器，未执行客户端真核检查。"
        );
    }
    Ok(())
}

/// The caller has authenticated these exact candidate bytes using the signed
/// release manifest. Updating the manager never changes either data-plane core.
pub fn install_manager_locked(
    store: &DeploymentStore,
    candidate: &Path,
    contents: &[u8],
) -> Result<()> {
    recover_locked(store, false)?;
    let help = Command::new(candidate)
        .args(["node", "migrate", "--help"])
        .output()
        .map_err(|_| Error("无法检查候选管理程序".into()))?;
    if !help.status.success() || !String::from_utf8_lossy(&help.stdout).contains("--apply") {
        return Err(Error("候选管理程序不支持统一节点仓库，拒绝降级".into()));
    }
    let revision = nodes::read_optional(store.root())?
        .map(|r| r.revision)
        .unwrap_or(0);
    let mut record = snapshot(
        store.root(),
        &[
            "usr/local/bin/sbctl".into(),
            "etc/systemd/system/sbctl.service".into(),
            "etc/systemd/system/sbctl-http.socket".into(),
            crate::lifecycle::CERTBOT_DEPLOY_HOOK_RELATIVE_PATH.into(),
        ],
        revision,
    )?;
    journal(store, &record)?;
    let result = (|| -> Result<()> {
        store.write_relative_locked("usr/local/bin/sbctl", contents)?;
        crate::lifecycle::restart_subscription_service(store.root()).map_err(Error)?;
        record.committed = true;
        journal(store, &record)?;
        store.write_root_only_locked(LAST, &json(&record))?;
        io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
        Ok(())
    })();
    if let Err(e) = result {
        restore(store, &record, false)?;
        crate::lifecycle::restart_subscription_service(store.root()).map_err(Error)?;
        io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
        return Err(e);
    }
    Ok(())
}

pub fn restart(store: &DeploymentStore, selected: Option<CoreKind>) -> Result<()> {
    let _lock = store.acquire_operation_lock()?;
    recover_locked(store, false)?;
    let config = store.load()?;
    let registry = nodes::load_or_import(store.root(), &config)?;
    let mut targets = Vec::new();
    for core in [CoreKind::SingBox, CoreKind::Xray] {
        if selected.is_some_and(|c| c != core) || !registry.mode.allows(core) {
            continue;
        }
        owned(store.root(), core)?;
        let text = optional(store.root(), active(core))?
            .ok_or_else(|| Error("活动内核配置不存在".into()))?;
        let text = String::from_utf8(text).map_err(|_| Error("内核配置编码错误".into()))?;
        core.check_config(&store.root().join(binary(core)), &text)
            .map_err(|_| Error(format!("{core} 配置检查失败，未重启")))?;
        targets.push(core);
    }
    if targets.is_empty() {
        return Err(Error("所选内核不属于当前运行模式".into()));
    }
    for core in targets {
        systemctl(store.root(), &["restart", unit(core)])?;
        health(store.root(), unit(core))?;
    }
    Ok(())
}

/// Official release archive digests, retrieved from XTLS/Xray-core's fixed
/// v26.10.31 GitHub release API. Unknown versions require new compatibility CI.
pub fn xray_archive(architecture: &str) -> Result<(&'static str, &'static str)> {
    match architecture {
        "x86_64" => Ok((
            "Xray-linux-64.zip",
            "e4bd9119f50ae7c41658b1abd0679ae06f98bd864a2907ff33f2f2cf7081b214",
        )),
        "aarch64" => Ok((
            "Xray-linux-arm64-v8a.zip",
            "1811c8f1da0c3b6dc707ebaf1c8390e987ba39859a8f0a7aadd8526157f2ec8d",
        )),
        _ => Err(Error("Xray 仅支持已验证的 Linux amd64/arm64 工件".into())),
    }
}
pub fn install_core(store: &DeploymentStore, core: CoreKind, version: Option<&str>) -> Result<()> {
    install_core_from_source(store, core, version, None)
}
pub fn install_core_from_source(
    store: &DeploymentStore,
    core: CoreKind,
    version: Option<&str>,
    source: Option<&crate::kernel_source::CustomSource<'_>>,
) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(Error(
            "官方服务端内核安装需要 Linux；Windows 仅用于开发与测试".into(),
        ));
    }
    let staging = io(tempfile::tempdir())?;
    let candidate = staging.path().join("candidate");
    let version = version.unwrap_or(if core == CoreKind::Xray {
        crate::core::xray::BASELINE_VERSION
    } else {
        "1.14.2"
    });
    if let Some(source) = source {
        if (core == CoreKind::Xray && version != crate::core::xray::BASELINE_VERSION)
            || (core == CoreKind::SingBox
                && ![
                    "1.10.7", "1.11.15", "1.12.25", "1.13.21", "1.14.1", "1.14.2", "1.14.4",
                ]
                .contains(&version))
        {
            return Err(Error("自定义来源仍须使用已验证的内核版本".into()));
        }
        eprintln!(
            "自定义来源：请确认链接与 SHA-256 来自可信发布者。文件一致性不等于官方来源；下载后仍检查版本、配置与服务，失败自动回滚。"
        );
        crate::kernel_source::download(source, &candidate, core, version).map_err(Error)?;
    } else if core == CoreKind::Xray {
        if version != crate::core::xray::BASELINE_VERSION {
            return Err(Error("该 Xray 版本尚未通过项目兼容验证".into()));
        }
        let (asset, digest) = xray_archive(std::env::consts::ARCH)?;
        let archive = staging.path().join("core.zip");
        let url = format!("https://github.com/XTLS/Xray-core/releases/download/v{version}/{asset}");
        let status = Command::new("curl")
            .args([
                "--disable",
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "15",
                "--max-time",
                "180",
                "--max-filesize",
                "134217728",
                "--output",
            ])
            .arg(&archive)
            .arg(url)
            .status()
            .map_err(|_| Error("内核下载失败".into()))?;
        if !status.success() {
            return Err(Error("官方 Xray 下载失败".into()));
        }
        let bytes = io(std::fs::read(&archive))?;
        if format!("{:x}", Sha256::digest(&bytes)) != digest {
            return Err(Error("Xray 官方归档摘要不匹配，拒绝安装".into()));
        }
        let output = Command::new("unzip")
            .arg("-p")
            .arg(&archive)
            .arg("xray")
            .output()
            .map_err(|_| Error("提取 Xray 需要系统 unzip，请安装 unzip 后重试".into()))?;
        if !output.status.success()
            || output.stdout.is_empty()
            || output.stdout.len() > 128 * 1024 * 1024
        {
            return Err(Error("Xray 归档中的可执行文件无效".into()));
        }
        io(std::fs::write(&candidate, output.stdout))?;
    } else {
        if ![
            "1.10.7", "1.11.15", "1.12.25", "1.13.21", "1.14.1", "1.14.2", "1.14.4",
        ]
        .contains(&version)
        {
            return Err(Error("该 sing-box 版本不在已验证的安装列表中".into()));
        }
        crate::update::download_sing_box_official(version, &candidate)
            .map_err(|_| Error("官方 sing-box 下载或摘要验证失败".into()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        io(std::fs::set_permissions(
            &candidate,
            std::fs::Permissions::from_mode(0o755),
        ))?;
    }
    let output = Command::new(&candidate)
        .arg("version")
        .output()
        .map_err(|_| Error("候选内核无法执行".into()))?;
    let expected = if core == CoreKind::Xray {
        format!("Xray {version}")
    } else {
        format!("sing-box version {version}")
    };
    let identity = String::from_utf8_lossy(&output.stdout);
    if !output.status.success()
        || !identity.starts_with(&expected)
        || identity[expected.len()..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace())
    {
        return Err(Error("候选内核身份或版本不匹配".into()));
    }
    let _lock = store.acquire_operation_lock()?;
    recover_locked(store, false)?;
    owned(store.root(), core)?;
    let mut migrated_server = None;
    if let Some(text) = optional(store.root(), active(core))? {
        let mut text = String::from_utf8(text).map_err(|_| Error("活动配置编码错误".into()))?;
        if core == CoreKind::SingBox {
            let config = store.load()?;
            let registry = nodes::load_or_import(store.root(), &config)?;
            if let Some((_, document)) = crate::subscription::managed::servers_for_kernel(
                &config,
                store.root(),
                &registry,
                Some(version.starts_with("1.10.")),
            )
            .map_err(|e| Error(e.to_string()))?
            .into_iter()
            .find(|(c, _)| *c == core)
            {
                text = serde_json::to_string_pretty(&document)
                    .map_err(|_| Error("内核配置序列化失败".into()))?;
                migrated_server = Some(text.clone());
            }
        }
        core.check_config(&candidate, &text)
            .map_err(|_| Error("新内核不接受当前活动配置，旧内核保持运行".into()))?;
    }
    let revision = nodes::read_optional(store.root())?
        .map(|r| r.revision)
        .unwrap_or(0);
    let mut paths = vec![binary(core).into()];
    if migrated_server.is_some() {
        paths.push(active(core).into());
        if nodes::read_optional(store.root())?.is_none() {
            paths.push("var/lib/sbctl/artifacts/sing-box-server.json".into());
        }
    }
    let mut record = snapshot(store.root(), &paths, revision)?;
    journal(store, &record)?;
    let result = (|| -> Result<()> {
        store.write_relative_locked(binary(core), &io(std::fs::read(&candidate))?)?;
        if let Some(server) = &migrated_server {
            store.write_relative_locked(active(core), server.as_bytes())?;
            if nodes::read_optional(store.root())?.is_none() {
                store.write_relative_locked(
                    "var/lib/sbctl/artifacts/sing-box-server.json",
                    server.as_bytes(),
                )?;
            }
        }
        if state(store.root(), unit(core), "is-active") {
            systemctl(store.root(), &["restart", unit(core)])?;
            health(store.root(), unit(core))?;
        }
        record.committed = true;
        journal(store, &record)?;
        store.write_root_only_locked(LAST, &json(&record))?;
        io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
        Ok(())
    })();
    if let Err(e) = result {
        restore(store, &record, false)?;
        io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
        return Err(e);
    }
    Ok(())
}
