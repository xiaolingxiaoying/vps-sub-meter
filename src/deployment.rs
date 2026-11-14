//! Durable managed-deployment transaction. The public generation pointer is the
//! commit point; neither accounting nor subscription credentials are snapshotted.
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
    Ok(Plan {
        previous_mode: before.mode,
        mode: after.mode,
        client_version: serde_json::json!({"before":before.client_version,"after":after.client_version}),
        details,
        groups,
        client_artifacts_changed,
        client_artifacts_removed,
        requirements,
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
    if root != Path::new("/") {
        return Ok(());
    }
    let status = Command::new("systemctl")
        .args(args)
        .status()
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
    )
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
    let states = [CoreKind::SingBox, CoreKind::Xray]
        .into_iter()
        .map(|c| {
            (
                unit(c).into(),
                state(root, unit(c), "is-enabled"),
                state(root, unit(c), "is-active"),
            )
        })
        .collect();
    Ok(Journal {
        files,
        states,
        applied_revision: revision,
        committed: false,
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
    systemctl(root, &["daemon-reload"])?;
    if !boot {
        for (name, enabled, active) in &record.states {
            if !affected.iter().any(|c| unit(*c) == name) {
                continue;
            }
            if *enabled {
                systemctl(root, &["enable", name])?;
            } else if root.join(format!("etc/systemd/system/{name}")).exists() {
                systemctl(root, &["disable", name])?;
            }
            if *active {
                systemctl(root, &["start", name])?;
                health(root, name)?;
            }
        }
        if record.files.iter().any(|f| f.path == "usr/local/bin/sbctl") {
            crate::lifecycle::restart_subscription_service(root).map_err(Error)?;
        }
    }
    Ok(())
}
fn allowed_snapshot(path: &str) -> bool {
    path == crate::config::CONFIG_RELATIVE_PATH
        || path == "usr/local/bin/sbctl"
        || path == nodes::SOURCE
        || path == CURRENT
        || path == RECOVERY_UNIT
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
    check_ports(root, expected, &next)?;
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
        for c in &changed {
            if root
                .join(format!("etc/systemd/system/{}", unit(*c)))
                .exists()
            {
                systemctl(root, &["stop", unit(*c)])?;
            }
        }
        for c in [CoreKind::SingBox, CoreKind::Xray] {
            if next.mode.allows(c) {
                store.write_relative_locked(
                    &format!("etc/systemd/system/{}", unit(c)),
                    unit_text(c, config.hysteria2_port_hopping.is_some()).as_bytes(),
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
        for c in [CoreKind::SingBox, CoreKind::Xray] {
            if next.mode.allows(c) {
                systemctl(root, &["enable", unit(c)])?;
                if changed.contains(&c) || !state(root, unit(c), "is-active") {
                    systemctl(root, &["start", unit(c)])?;
                    health(root, unit(c))?;
                } else {
                    systemctl(root, &["is-active", "--quiet", unit(c)])?;
                }
            }
        }
        if expected_config != config {
            store.replace_locked(config)?;
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
    let backup = snapshot(store.root(), &snapshot_paths(), registry.revision + 1)?;
    apply_with_config_locked(store, &old, new, &registry, registry.clone())?;
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
fn check_ports(root: &Path, before: &Registry, next: &Registry) -> Result<()> {
    if root != Path::new("/") {
        return Ok(());
    }
    for node in next.nodes.iter().filter(|n| n.enabled) {
        if before
            .nodes
            .iter()
            .any(|n| n.enabled && n.port() == node.port())
        {
            continue;
        }
        let addr: std::net::IpAddr = node
            .listen
            .parse()
            .map_err(|_| Error("监听地址无效".into()))?;
        let socket = std::net::SocketAddr::new(addr, node.port());
        let _tcp = std::net::TcpListener::bind(socket)
            .map_err(|_| Error(format!("端口 {} TCP 已被占用", node.port())))?;
        let _udp = std::net::UdpSocket::bind(socket)
            .map_err(|_| Error(format!("端口 {} UDP 已被占用", node.port())))?;
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
    let current =
        nodes::read_optional(store.root())?.ok_or_else(|| Error("没有托管节点仓库".into()))?;
    if current.revision != record.applied_revision {
        return Err(Error("恢复点已过期，拒绝覆盖后续修改".into()));
    }
    record.committed = false;
    journal(store, &record)?;
    restore(store, &record, false)?;
    io(crate::managed_fs::remove_file(store.root().join(JOURNAL)))?;
    io(crate::managed_fs::remove_file(store.root().join(LAST)))?;
    Ok(())
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
    let mut record = snapshot(store.root(), &["usr/local/bin/sbctl".into()], revision)?;
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
    if core == CoreKind::Xray {
        if version != crate::core::xray::BASELINE_VERSION {
            return Err(Error("该 Xray 版本尚未通过项目兼容验证".into()));
        }
        let (asset, digest) = xray_archive(std::env::consts::ARCH)?;
        let archive = staging.path().join("core.zip");
        let url = format!("https://github.com/XTLS/Xray-core/releases/download/v{version}/{asset}");
        let status = Command::new("curl")
            .args([
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
    if !output.status.success() || !String::from_utf8_lossy(&output.stdout).starts_with(&expected) {
        return Err(Error("候选内核身份或版本不匹配".into()));
    }
    let _lock = store.acquire_operation_lock()?;
    recover_locked(store, false)?;
    owned(store.root(), core)?;
    if let Some(text) = optional(store.root(), active(core))? {
        let text = String::from_utf8(text).map_err(|_| Error("活动配置编码错误".into()))?;
        core.check_config(&candidate, &text)
            .map_err(|_| Error("新内核不接受当前活动配置，旧内核保持运行".into()))?;
    }
    let revision = nodes::read_optional(store.root())?
        .map(|r| r.revision)
        .unwrap_or(0);
    let mut record = snapshot(store.root(), &[binary(core).into()], revision)?;
    journal(store, &record)?;
    let result = (|| -> Result<()> {
        store.write_relative_locked(binary(core), &io(std::fs::read(&candidate))?)?;
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
