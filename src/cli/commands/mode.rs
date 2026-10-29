//! Subscription delivery changes are lifecycle transactions, including TLS and
//! systemd state. The operation lock survives publication and recovery.
use sbctl::config::{CertificateMode, DeploymentConfig, DeploymentStore, SubscriptionMode};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

pub(crate) struct ModeOptions {
    pub mode: SubscriptionMode,
    pub listen_port: Option<u16>,
    pub subscription_host: Option<String>,
    pub http_port: Option<u16>,
    pub certificate_mode: Option<CertificateMode>,
    pub protocol_sni: Option<String>,
    pub certbot_email: Option<String>,
    pub sing_box_bin: Option<PathBuf>,
}

fn desired(prior: &DeploymentConfig, options: &ModeOptions) -> Result<DeploymentConfig, String> {
    let mut new = prior.clone();
    new.subscription_mode = options.mode.clone();
    if let Some(host) = &options.subscription_host {
        // Certbot creates its live directory from the lowercased domain;
        // normalize before validation, snapshots, and pinned-path generation.
        new.subscription_host = host.to_ascii_lowercase();
    }
    if let Some(mode) = &options.certificate_mode {
        new.certificate_mode = mode.clone();
    }
    if let Some(sni) = &options.protocol_sni {
        new.protocol_sni = Some(sni.clone());
    }
    if let Some(email) = &options.certbot_email {
        new.certbot_email = Some(email.clone());
    }
    match new.subscription_mode {
        SubscriptionMode::Direct => {
            if options.http_port.is_some() || options.listen_port.is_some() {
                return Err("Direct 不接受 --http-port 或 --listen-port".into());
            }
            new.http_port = None;
            new.subscription_listen_port = None;
        }
        SubscriptionMode::ExternalProxy => {
            if options.http_port.is_some() {
                return Err("External proxy 不接受 --http-port".into());
            }
            new.http_port = None;
            new.subscription_listen_port = options.listen_port.or_else(|| {
                (prior.subscription_mode == SubscriptionMode::ExternalProxy)
                    .then_some(prior.subscription_listen_port)
                    .flatten()
            });
        }
        SubscriptionMode::IpFallback => {
            if options.listen_port.is_some() {
                return Err("IP fallback 不接受 --listen-port".into());
            }
            if prior.certificate_mode == CertificateMode::Domain
                && options.certificate_mode != Some(CertificateMode::SelfSigned)
            {
                return Err("进入 IP fallback 需要显式指定 --certificate-mode self-signed".into());
            }
            new.certificate_mode = CertificateMode::SelfSigned;
            new.subscription_listen_port = None;
            new.http_port = options.http_port.or_else(|| {
                (prior.subscription_mode == SubscriptionMode::IpFallback)
                    .then_some(prior.http_port)
                    .flatten()
            });
        }
    }
    new.validate().map_err(|e| e.to_string())?;
    Ok(new)
}

const UNITS: &[&str] = &[
    "sing-box.service",
    "sbctl.service",
    "sbctl-http.socket",
    "sbctl-accounting-reset.service",
    "sbctl-accounting-reset.timer",
    "sbctl-credential-rotate.service",
    "sbctl-credential-rotate.timer",
];
const HOOK: &str = "etc/letsencrypt/renewal-hooks/deploy/sbctl-certificate-deploy-hook";

struct FileState {
    relative: String,
    bytes: Option<Vec<u8>>,
    metadata: Option<fs::Metadata>,
}
struct UnitState {
    name: &'static str,
    enabled: String,
    active: bool,
}
struct Snapshot {
    files: Vec<FileState>,
    directories: Vec<(String, Option<fs::Metadata>)>,
    units: Vec<UnitState>,
}

fn owned_artifact(name: &str) -> bool {
    name == "sing-box-server.json" || name.starts_with("subscription-")
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(format!("拒绝受管路径符号链接：{}", path.display()))
        }
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

impl Snapshot {
    fn capture(
        root: &Path,
        prior: &DeploymentConfig,
        new: &DeploymentConfig,
    ) -> Result<Self, String> {
        let mut paths = vec![
            "etc/sbctl/config.toml".into(),
            "etc/sing-box/config.json".into(),
            HOOK.into(),
        ];
        paths.extend(
            UNITS
                .iter()
                // Accounting and rotation tasks may be waiting on our lock.
                // Their timers are untouched by a delivery change; never wait
                // for a lock-taking oneshot while recovering this transaction.
                .filter(|&&name| {
                    matches!(
                        name,
                        "sing-box.service" | "sbctl.service" | "sbctl-http.socket"
                    )
                })
                .map(|unit| format!("etc/systemd/system/{unit}")),
        );
        let mut directories: Vec<String> = vec![
            "etc/sbctl".into(),
            "etc/sing-box".into(),
            "var/lib/sbctl".into(),
            "var/lib/sbctl/artifacts".into(),
            "var/lib/sbctl/certificates".into(),
            "var/lib/sbctl/acme-webroot".into(),
        ];
        let artifacts = root.join("var/lib/sbctl/artifacts");
        if metadata(&artifacts)?.is_some() {
            for entry in fs::read_dir(artifacts).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if owned_artifact(&name) {
                    paths.push(format!("var/lib/sbctl/artifacts/{name}"));
                }
            }
        }
        let certs = root.join("var/lib/sbctl/certificates");
        if metadata(&certs)?.is_some() {
            for entry in fs::read_dir(certs).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if metadata(&entry.path())?.is_some_and(|meta| meta.is_dir()) {
                    directories.push(format!(
                        "var/lib/sbctl/certificates/{}",
                        entry.file_name().to_string_lossy()
                    ));
                }
            }
        }
        for config in [prior, new] {
            for host in [&config.subscription_host, config.protocol_server_name()] {
                directories.push(format!("var/lib/sbctl/certificates/{host}"));
            }
        }
        directories.sort();
        directories.dedup();
        for directory in &directories {
            if directory.starts_with("var/lib/sbctl/certificates/") {
                for name in ["cert.pem", "key.pem", "fullchain.pem", "privkey.pem"] {
                    paths.push(format!("{directory}/{name}"));
                }
            }
        }
        paths.sort();
        paths.dedup();
        let files = paths
            .into_iter()
            .map(|relative| {
                let path = root.join(&relative);
                let (bytes, metadata) = match sbctl::config::open_managed_path_no_follow(&path) {
                    Ok(mut file) => {
                        use std::io::Read;
                        let meta = file.metadata().map_err(|e| e.to_string())?;
                        let mut bytes = Vec::new();
                        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                        (Some(bytes), Some(meta))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
                    Err(error) => return Err(error.to_string()),
                };
                Ok(FileState {
                    relative,
                    bytes,
                    metadata,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let directories = directories
            .into_iter()
            .map(|relative| metadata(&root.join(&relative)).map(|meta| (relative, meta)))
            .collect::<Result<Vec<_>, _>>()?;
        let units = if root == Path::new("/") {
            UNITS
                .iter()
                .map(|&name| {
                    if !root.join(format!("etc/systemd/system/{name}")).exists() {
                        return Ok(UnitState {
                            name,
                            enabled: "not-found".into(),
                            active: false,
                        });
                    }
                    let enabled = ctl_output(root, &["is-enabled", name])?;
                    let active = ctl_output(root, &["is-active", name])?;
                    Ok(UnitState {
                        name,
                        enabled,
                        active: active == "active" || active == "activating",
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
        } else {
            Vec::new()
        };
        Ok(Self {
            files,
            directories,
            units,
        })
    }

    fn restore(&self, store: &DeploymentStore, touched_services: bool) -> Result<(), String> {
        let root = store.root();
        let mut errors = Vec::new();
        if touched_services && root == Path::new("/") {
            for unit in ["sbctl.service", "sing-box.service", "sbctl-http.socket"] {
                if let Err(e) = ctl(root, &["stop", unit]) {
                    errors.push(e);
                }
            }
            for unit in &self.units {
                if unit.enabled == "not-found"
                    && root
                        .join(format!("etc/systemd/system/{}", unit.name))
                        .is_file()
                    && let Err(error) = ctl(root, &["disable", unit.name])
                {
                    errors.push(error);
                }
            }
        }
        let artifacts = root.join("var/lib/sbctl/artifacts");
        if artifacts.is_dir() {
            for entry in fs::read_dir(&artifacts).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let relative = format!("var/lib/sbctl/artifacts/{name}");
                if owned_artifact(&name)
                    && !self.files.iter().any(|file| file.relative == relative)
                    && let Err(e) = fs::remove_file(entry.path())
                {
                    errors.push(e.to_string());
                }
            }
        }
        for file in &self.files {
            let path = root.join(&file.relative);
            let result = match &file.bytes {
                Some(bytes) => store
                    .write_relative_locked(&file.relative, bytes)
                    .map_err(|e| e.to_string())
                    .and_then(|()| restore_metadata(&path, file.metadata.as_ref().unwrap())),
                None => remove_if_present(&path),
            };
            if let Err(e) = result {
                errors.push(e);
            }
        }
        for (relative, prior) in self.directories.iter().rev() {
            let path = root.join(relative);
            if let Some(meta) = prior {
                if let Err(e) = restore_metadata(&path, meta) {
                    errors.push(e);
                }
            } else if path.is_dir() {
                match fs::remove_dir(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                    Err(e) => errors.push(e.to_string()),
                }
            }
        }
        if touched_services && root == Path::new("/") {
            if let Err(e) = ctl(root, &["daemon-reload"]) {
                errors.push(e);
            }
            for unit in &self.units {
                let result = match unit.enabled.as_str() {
                    "enabled" => ctl(root, &["enable", unit.name]),
                    "enabled-runtime" => ctl(root, &["enable", "--runtime", unit.name]),
                    "disabled" => ctl(root, &["disable", unit.name]),
                    _ => Ok(()),
                };
                if let Err(e) = result {
                    errors.push(e);
                }
            }
            // Files and subscription mode are restored before the lifecycle code
            // starts the old service through its socket activation contract.
            if self.units.iter().any(|unit| {
                unit.active && matches!(unit.name, "sbctl.service" | "sing-box.service")
            }) && let Err(e) = sbctl::lifecycle::restart_services(root)
            {
                errors.push(e);
            }
            for unit in &self.units {
                if unit.enabled == "not-found" {
                    continue;
                }
                let result = if unit.active {
                    ctl(root, &["start", unit.name])
                } else {
                    ctl(root, &["stop", unit.name])
                };
                if let Err(e) = result {
                    errors.push(e);
                }
            }
        }
        if touched_services
            && root != Path::new("/")
            && let Err(error) = sbctl::lifecycle::restart_services(root)
        {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn restore_metadata(path: &Path, meta: &fs::Metadata) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::{fs::MetadataExt, io::AsRawFd};
        let file = sbctl::config::open_managed_path_no_follow(path).map_err(|e| e.to_string())?;
        if unsafe { libc::fchown(file.as_raw_fd(), meta.uid(), meta.gid()) } != 0 {
            return Err(format!("恢复属主失败：{}", path.display()));
        }
        file.set_permissions(meta.permissions())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    fs::set_permissions(path, meta.permissions()).map_err(|e| e.to_string())
}

fn ctl_program(root: &Path) -> PathBuf {
    let candidate = root.join("usr/bin/systemctl");
    if candidate.is_file() {
        return candidate;
    }
    #[cfg(windows)]
    if root.join("usr/bin/systemctl.cmd").is_file() {
        return root.join("usr/bin/systemctl.cmd");
    }
    "systemctl".into()
}
fn ctl(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(ctl_program(root))
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl {} failed: {}",
            args.join(" "),
            output.status
        ))
    }
}
fn ctl_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(ctl_program(root))
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        return Err(format!("systemctl {} returned no state", args.join(" ")));
    }
    Ok(value)
}

fn prepare_listener(
    root: &Path,
    prior: &DeploymentConfig,
    new: &DeploymentConfig,
    explicit_port: bool,
) -> Result<(), String> {
    let port = match new.subscription_mode {
        SubscriptionMode::ExternalProxy => new.subscription_listen_port,
        SubscriptionMode::IpFallback => new.http_port,
        SubscriptionMode::Direct => None,
    };
    let old_port = match prior.subscription_mode {
        SubscriptionMode::ExternalProxy => prior.subscription_listen_port,
        SubscriptionMode::IpFallback => prior.http_port,
        SubscriptionMode::Direct => None,
    };
    if let Some(port) = port {
        // When the mode changes on the same managed port, the old listener is
        // stopped by this transaction; probing it first would report a false
        // conflict. Same-mode reconfiguration still probes, because a foreign
        // process may hold the port while the managed service is stopped.
        let reuses_managed_port =
            new.subscription_mode != prior.subscription_mode && old_port == Some(port);
        if !reuses_managed_port
            && (explicit_port
                || new.subscription_mode != prior.subscription_mode
                || old_port != Some(port))
        {
            // IP fallback binds publicly; test the wildcard address, not only loopback.
            let address = if new.subscription_mode == SubscriptionMode::IpFallback {
                "0.0.0.0"
            } else {
                "127.0.0.1"
            };
            let _listener = std::net::TcpListener::bind((address, port))
                .map_err(|_| format!("subscription listener port {port} already in use"))?;
        }
    }
    if root == Path::new("/")
        && new.subscription_mode == SubscriptionMode::Direct
        && prior.subscription_mode != SubscriptionMode::Direct
    {
        sbctl::preflight::preflight_install(root, true, true).map_err(|e| e.to_string())?;
        let output = Command::new("ss")
            .args(["-H", "-ltnp", "( sport = :80 or sport = :443 )"])
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() || !output.stdout.is_empty() {
            return Err("Direct 需要空闲的公网 TCP 80/443".into());
        }
    }
    Ok(())
}

pub(crate) fn switch_mode(root: &Path, options: ModeOptions) -> ExitCode {
    let result = (|| -> Result<(), String> {
        sbctl::preflight::require_install_privileges(root).map_err(|e| e.to_string())?;
        let store = DeploymentStore::new(root);
        let _lock = store.acquire_operation_lock().map_err(|e| e.to_string())?;
        let prior = store.load().map_err(|e| e.to_string())?;
        let new = desired(&prior, &options)?;
        prepare_listener(
            root,
            &prior,
            &new,
            options.listen_port.is_some() || options.http_port.is_some(),
        )?;
        let binary = options
            .sing_box_bin
            .clone()
            .unwrap_or_else(|| root.join("usr/local/bin/sing-box"));
        if !binary.is_file() {
            return Err("模式切换需要 sing-box 内核；用 --sing-box-bin 指定".into());
        }
        let mut cert_config = new.clone();
        cert_config.subscription_mode = SubscriptionMode::Direct;
        cert_config.http_port = None;
        cert_config.subscription_listen_port = None;
        let direct = new.subscription_mode == SubscriptionMode::Direct;
        let pinned = sbctl::certificate::load_pinned(&store, &cert_config).ok();
        let live = sbctl::certificate::load(&store, &cert_config).ok();
        if new.subscription_mode == SubscriptionMode::ExternalProxy
            && new.certificate_mode == CertificateMode::Domain
            && live.is_none()
        {
            return Err(
                "External proxy 的域名协议证书缺失或无效；请先提供覆盖目标域名的证书".into(),
            );
        }
        let obtain = direct && pinned.is_none() && live.is_none();
        if obtain {
            sbctl::certificate::require_certbot(root).map_err(|e| e.to_string())?;
            if !new
                .certbot_email
                .as_deref()
                .is_some_and(sbctl::certificate::acme_email_is_valid)
            {
                return Err("签发 Direct 证书需要有效的 --certbot-email（或已保存的邮箱）".into());
            }
        }
        let snapshot = Snapshot::capture(root, &prior, &new)?;
        let mut touched_services = false;
        let commit = (|| -> Result<(), String> {
            if root == Path::new("/") {
                sbctl::lifecycle::prepare_daemon_prerequisites(root, true)?;
            }
            let live_is_newer = live.as_ref().is_some_and(|live| {
                pinned.as_ref().map(|pinned| pinned.fingerprint.as_str())
                    != Some(live.fingerprint.as_str())
            });
            if (direct && live_is_newer
                || new.subscription_mode == SubscriptionMode::ExternalProxy
                    && new.certificate_mode == CertificateMode::Domain)
                && let Some(live) = &live
            {
                sbctl::certificate::pin(&store, &new, live).map_err(|e| e.to_string())?;
            }
            // Publish strict client artifacts. Only the server gets temporary
            // TLS material while the HTTP ACME challenge listener starts.
            let bootstrap = obtain && new.certificate_mode == CertificateMode::Domain;
            let staged_server = if bootstrap {
                let mut staged = new.clone();
                staged.certificate_mode = CertificateMode::SelfSigned;
                let staged = sbctl::subscription::generated_artifacts_for_kernel(
                    &staged,
                    root,
                    Some(&binary),
                )
                .map_err(|e| e.to_string())?;
                Some(
                    staged
                        .into_iter()
                        .find(|(name, _)| name == "sing-box-server.json")
                        .ok_or_else(|| "缺少临时服务端配置".to_owned())?
                        .1,
                )
            } else {
                None
            };
            sbctl::subscription::apply_config_transaction_with_server_locked(
                &store,
                &new,
                Some(&binary),
                staged_server.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            sbctl::lifecycle::prepare_daemon_storage(root, direct)?;
            touched_services = true;
            if root != Path::new("/") && root.join("etc/systemd/system/sbctl.service").is_file() {
                sbctl::lifecycle::reconcile_subscription_units(root, direct)?;
            }
            sbctl::lifecycle::restart_services(root)?;
            if obtain {
                sbctl::certificate::obtain(&store, &new, new.certbot_email.as_deref())
                    .map_err(|e| e.to_string())?;
                sbctl::subscription::apply_config_transaction_locked(&store, &new, Some(&binary))
                    .map_err(|e| e.to_string())?;
                sbctl::lifecycle::prepare_daemon_storage(root, direct)?;
                sbctl::lifecycle::restart_services(root)?;
            }
            Ok(())
        })();
        if let Err(error) = commit {
            return Err(match snapshot.restore(&store, touched_services) {
                Ok(()) => format!("{error}；旧部署已恢复"),
                Err(recovery) => format!("{error}；部署恢复失败：{recovery}；请检查服务和配置"),
            });
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            println!("subscription mode changed; configuration and services committed");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("configuration failed: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::mode::{ModeOptions, desired, prepare_listener};

    fn config(mode: SubscriptionMode, host: &str) -> DeploymentConfig {
        let mut config = DeploymentConfig::new(
            mode.clone(),
            host.into(),
            None,
            (mode == SubscriptionMode::IpFallback).then_some(2080),
            "ens3".into(),
            vec![sbctl::config::ManagedProtocol::VlessReality],
            Some("www.cloudflare.com".into()),
        )
        .expect("fixture config");
        if mode == SubscriptionMode::ExternalProxy {
            config.subscription_listen_port = Some(2080);
        }
        config
    }

    #[test]
    fn a_mode_switch_host_is_lowercased_before_certificate_paths_are_built() {
        let prior = config(SubscriptionMode::ExternalProxy, "old.example.test");
        let options = ModeOptions {
            mode: SubscriptionMode::ExternalProxy,
            listen_port: None,
            subscription_host: Some("Sub.Example.TEST".into()),
            http_port: None,
            certificate_mode: None,
            protocol_sni: None,
            certbot_email: None,
            sing_box_bin: None,
        };
        let new = desired(&prior, &options).expect("the desired config is valid");
        assert_eq!(new.subscription_host, "sub.example.test");
    }

    #[test]
    fn reusing_the_running_subscription_port_is_not_a_false_conflict() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("blocker binds");
        let port = listener.local_addr().unwrap().port();
        let mut prior = config(SubscriptionMode::ExternalProxy, "sub.example.test");
        prior.subscription_listen_port = Some(port);
        let mut new = config(SubscriptionMode::IpFallback, "203.0.113.7");
        new.subscription_listen_port = None;
        new.http_port = Some(port);
        let root = tempfile::tempdir().expect("fixture root");
        prepare_listener(root.path(), &prior, &new, false)
            .expect("the old managed listener is stopped before the new one binds");
    }
}
