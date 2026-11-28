use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use vps_sub_meter::{
    apply,
    config::{Config, ConfigError, LIB_DIR, TrafficConfig, VAR_DIR, is_valid_hostname, pct},
    qr, render, store, traffic,
};

use crate::admin;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ports {
    Keep,
    Defaults,
    Random,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Change {
    Save,
    Apply,
    Deploy,
    BindDomain,
    ArgoSetup,
    ArgoDisable,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Preview {
        config: Box<Config>,
        ports: Ports,
    },
    Config {
        config: Box<Config>,
        revision: String,
        change: Change,
        #[serde(default)]
        email: String,
        tunnel_token: Option<String>,
    },
    Apply,
    Subscriptions {
        name: Option<String>,
    },
    TokenAdd {
        name: String,
    },
    TokenRotate,
    TokenRevoke {
        name: String,
    },
    KeysRotate,
    TrafficConfigure {
        traffic: TrafficConfig,
        revision: String,
    },
    TrafficSet {
        #[serde(default)]
        upload_gib: Option<f64>,
        #[serde(default)]
        download_gib: Option<f64>,
    },
    TrafficCheckpoint,
    DetectNetwork,
    CoreVersions,
    CoreUpdate,
    NetworkTune,
    ArgoCheck,
    ArgoRefresh,
    CertificateStatus,
    CertificateRenew,
    CertificateSync,
    CertificateIssue {
        email: String,
    },
    CertificateSelfSigned,
    CertificateAutoRenew {
        enabled: bool,
    },
    UpgradeCheck,
    UpgradeRelease {
        force_templates: bool,
    },
    Uninstall {
        keep_config: bool,
        confirmation: String,
    },
}

impl Action {
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::Preview { .. }
                | Self::Subscriptions { .. }
                | Self::DetectNetwork
                | Self::CoreVersions
                | Self::CertificateStatus
        )
    }

    pub fn closes_panel(&self) -> bool {
        matches!(self, Self::Uninstall { .. } | Self::UpgradeRelease { .. })
    }

    pub fn execute(self, root: &Path) -> Result<Value, String> {
        let mut output = String::new();
        match self {
            Self::Preview { mut config, ports } => {
                match ports {
                    Ports::Keep => {}
                    Ports::Defaults => admin::set_default_node_ports(root, &mut config)?,
                    Ports::Random => admin::randomize_node_ports(&mut config, is_live(root))?,
                }
                config.validate().map_err(|error| error.to_string())?;
                let mode = if config.is_ip_mode() {
                    "IP / HTTP"
                } else if config.xray_fronts_443() {
                    "HTTPS / SNI 复用"
                } else {
                    "HTTPS"
                };
                return Ok(json!({"config": config, "mode": mode, "tls_sni": config.tls_sni()}));
            }
            Self::Config {
                config,
                revision,
                change,
                email,
                tunnel_token,
            } => {
                return configure(root, *config, &revision, change, &email, tunnel_token);
            }
            Self::Apply => return applied(apply::apply_all(root)),
            Self::Subscriptions { name } => return subscriptions(root, name.as_deref()),
            Self::TokenAdd { name } => {
                output = cli(root, &["token", "add", "--", &name])?;
            }
            Self::TokenRotate => output = cli(root, &["token", "rotate"])?,
            Self::TokenRevoke { name } => {
                output = cli(root, &["token", "revoke", "--", &name])?;
            }
            Self::KeysRotate => return applied(apply::rotate_node_keys(root)),
            Self::TrafficConfigure {
                traffic: settings,
                revision,
            } => {
                let changed = traffic::configure_at_revision(root, Some(&revision), |config| {
                    *config = settings
                })
                .map_err(|error| error.to_string())?;
                return Ok(message(if changed {
                    "流量设置已生效"
                } else {
                    "流量设置未修改"
                }));
            }
            Self::TrafficSet {
                upload_gib,
                download_gib,
            } => {
                if upload_gib.is_none() && download_gib.is_none() {
                    return Err("请至少填写上传或下载流量中的一项".into());
                }
                let gib = 1024.0 * 1024.0 * 1024.0;
                let to_bytes = |value: Option<f64>, label: &str| -> Result<Option<u64>, String> {
                    let Some(v) = value else {
                        return Ok(None);
                    };
                    if !v.is_finite() || v < 0.0 {
                        return Err(format!("{label}必须是非负数字"));
                    }
                    let scaled = v * gib;
                    if scaled >= u64::MAX as f64 {
                        return Err(format!("{label}数值过大"));
                    }
                    Ok(Some(scaled.round() as u64))
                };
                let up_bytes = to_bytes(upload_gib, "已用上传流量")?;
                let down_bytes = to_bytes(download_gib, "已用下载流量")?;
                let snap = traffic::set_used_optional_current(root, up_bytes, down_bytes)
                    .map_err(|error| error.to_string())?;
                let summary = format!(
                    "已校准: 上传 {}, 下载 {}",
                    traffic::format_iec(snap.upload_bytes),
                    traffic::format_iec(snap.download_bytes)
                );
                return Ok(json!({"message": summary, "summary": summary}));
            }
            Self::TrafficCheckpoint => output = cli(root, &["traffic", "checkpoint"])?,
            Self::DetectNetwork => return detect_network(root),
            Self::CoreVersions => {
                return Ok(json!({
                    "xray": version(root, "xray"),
                    "sing_box": version(root, "sing-box"),
                    "cloudflared": version(root, "cloudflared"),
                }));
            }
            Self::CoreUpdate => {
                require_live(root)?;
                script(root, "cores.sh", &["update"], None, &mut output)?;
                if Config::path(root).exists() {
                    let result = applied(apply::apply_all(root)).map_err(|error| {
                        format!("内核已更新，重新应用配置失败。\n{output}\n{error}")
                    })?;
                    output.push_str(result["message"].as_str().unwrap_or_default());
                }
            }
            Self::NetworkTune => {
                require_live(root)?;
                script(root, "net.sh", &["tune-sysctl"], None, &mut output)?;
            }
            Self::ArgoCheck | Self::ArgoRefresh => {
                require_live(root)?;
                if !load_config(root)?.argo.enabled {
                    return Err("Argo 尚未启用".into());
                }
                let args: &[&str] = if matches!(self, Self::ArgoCheck) {
                    &["check"]
                } else {
                    &["refresh", "--require-token"]
                };
                script(root, "argo.sh", args, None, &mut output)?;
            }
            Self::CertificateStatus => {
                let config = load_config(root)?;
                let sni = certificate_name(&config)?;
                if root
                    .join(VAR_DIR)
                    .join("certificates")
                    .join(sni)
                    .join("fullchain.pem")
                    .is_file()
                {
                    script(
                        root,
                        "cert.sh",
                        &["status", "--root", path_arg(root)?, sni],
                        None,
                        &mut output,
                    )?;
                } else {
                    output = format!("证书名称：{sni}\n未找到当前证书。");
                }
            }
            Self::CertificateRenew | Self::CertificateSync | Self::CertificateIssue { .. } => {
                require_live(root)?;
                let config = load_config(root)?;
                if config.is_ip_mode() {
                    return Err("当前是 IP 模式，请先配置域名或使用自签证书".into());
                }
                let sni = certificate_name(&config)?;
                match self {
                    Self::CertificateRenew => {
                        allow_web_ports(root, &config, &mut output);
                        script(root, "cert.sh", &["renew", sni], None, &mut output)?;
                    }
                    Self::CertificateSync => {
                        script(root, "cert.sh", &["sync", sni], None, &mut output)?
                    }
                    Self::CertificateIssue { email } => {
                        issue(root, &config, &email, &mut output)?;
                    }
                    _ => unreachable!(),
                }
            }
            Self::CertificateSelfSigned => {
                require_live(root)?;
                let config = load_config(root)?;
                if !config.is_ip_mode() {
                    return Err("自签证书操作仅用于 IP 模式".into());
                }
                let sni = certificate_name(&config)?;
                script(root, "cert.sh", &["self-signed", sni], None, &mut output)?;
                let result =
                    applied(apply::apply_all(root)).map_err(|error| format!("{output}{error}"))?;
                if let Some(msg) = result["message"].as_str()
                    && !msg.is_empty()
                {
                    output.push_str(msg);
                    output.push('\n');
                }
                script(root, "cert.sh", &["reload", sni], None, &mut output)?;
            }
            Self::CertificateAutoRenew { enabled } => {
                require_live(root)?;
                let config = load_config(root)?;
                if config.is_ip_mode() {
                    return Err("当前是 IP 模式，不支持域名证书自动续期".into());
                }
                let subcmd = if enabled {
                    "auto-enable"
                } else {
                    "auto-disable"
                };
                script(root, "cert.sh", &[subcmd], None, &mut output)?;
            }
            Self::UpgradeCheck => {
                require_live(root)?;
                script(root, "upgrade.sh", &["--check"], None, &mut output)?;
            }
            Self::UpgradeRelease { force_templates } => {
                require_live(root)?;
                let mut args = vec!["--release-only"];
                if force_templates {
                    args.push("--force-templates");
                }
                script(root, "upgrade.sh", &args, None, &mut output)?;
            }
            Self::Uninstall {
                keep_config,
                confirmation,
            } => {
                if confirmation != "卸载" {
                    return Err("请输入「卸载」确认操作".into());
                }
                let mut args = vec!["--yes", "--root", path_arg(root)?];
                if keep_config {
                    args.push("--keep-config");
                }
                script(root, "uninstall.sh", &args, None, &mut output)?;
            }
        }
        Ok(message(if output.trim().is_empty() {
            "操作完成"
        } else {
            output.trim()
        }))
    }
}

pub fn state(root: &Path) -> Result<Value, String> {
    let _lock = store::lock_exclusive(&root.join(VAR_DIR)).map_err(|error| error.to_string())?;
    let revision = store::config_revision(root).map_err(|error| error.to_string())?;
    let (config, config_error) = match Config::load(root) {
        Ok(config) => (config, None),
        Err(error) => {
            let detail = (!matches!(error, ConfigError::Missing(_))).then(|| error.to_string());
            (
                Config::load_defaults(root).map_err(|error| error.to_string())?,
                detail,
            )
        }
    };
    drop(_lock);
    let initialized = !revision.is_empty() && config_error.is_none();
    let (status, status_error) = if initialized {
        match traffic::snapshot(root, &config)
            .map_err(|error| error.to_string())
            .and_then(|snapshot| admin::status_json(root, &config, &snapshot))
        {
            Ok(status) => (Some(status), None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };
    let generation = store::current_generation(root).map_err(|error| error.to_string())?;
    let pending_changes =
        initialized && apply::has_pending_changes(root, &config, generation.as_ref());
    let services = is_live(root).then(|| {
        json!({
            "vsm": service_state("vps-sub-meter.service"),
            "xray": service_state("vps-sub-meter-xray.service"),
            "singbox": service_state("vps-sub-meter-singbox.service"),
            "argo": service_state("vps-sub-meter-argo.service"),
            "cert_renew_timer": service_state("vps-sub-meter-cert-renew.timer"),
        })
    });
    let cert_renewal = (initialized && !config.is_ip_mode())
        .then(|| store::read_cert_renew_status_for_domain(root, config.tls_sni()))
        .flatten();
    let argo_preferred_ip_count = render::client::load_argo_preferred_ips(root, &config).len();
    Ok(json!({
        "config": config,
        "config_error": config_error,
        "status": status,
        "status_error": status_error,
        "initialized": initialized,
        "pending_changes": pending_changes,
        "revision": revision,
        "generation": generation,
        "services": services,
        "cert_renewal": cert_renewal,
        "argo_token_configured": argo_token_exists(root),
        "argo_preferred_ip_count": argo_preferred_ip_count,
        "live": is_live(root),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub fn tokens(root: &Path) -> Result<Value, String> {
    serde_json::from_str(&cli(root, &["token", "list", "--json"])?)
        .map_err(|error| error.to_string())
}

fn subscriptions(root: &Path, name: Option<&str>) -> Result<Value, String> {
    let config = load_config(root)?;
    let mut args = vec!["sub", "--json"];
    let name_arg = name.map(|name| format!("--token-name={name}"));
    args.extend(name_arg.as_deref());
    let links: Value = serde_json::from_str(&cli(root, &args)?).map_err(|e| e.to_string())?;
    let codes = |part: &str, profile: &str| -> Result<Value, String> {
        if links[part].is_null() {
            return Ok(Value::Null);
        }
        let link = |format: &str| {
            links[part][format]
                .as_str()
                .ok_or("订阅地址缺失".to_owned())
        };
        let singbox = links[part]["sing_box"]
            .as_str()
            .map(|url| {
                let payload = format!(
                    "sing-box://import-remote-profile?url={}#{}",
                    pct(url),
                    pct(profile)
                );
                qr::render_svg(&payload)
            })
            .transpose()?;
        let shadowrocket_sub = format!(
            "sub://{}#{}",
            STANDARD.encode(link("shadowrocket")?.as_bytes()),
            pct(profile)
        );
        Ok(json!({
            "clash": qr::render_svg(link("clash")?)?,
            "sing_box": singbox,
            "shadowrocket": qr::render_svg(&shadowrocket_sub)?,
        }))
    };
    let direct = codes("direct", &config.profile_name)?;
    let argo = codes("argo", &config.argo.profile_name)?;
    Ok(
        json!({"links": links, "qr": {"direct": direct, "argo": argo}, "https": !config.is_ip_mode()}),
    )
}

fn configure(
    root: &Path,
    config: Config,
    revision: &str,
    change: Change,
    email: &str,
    tunnel_token: Option<String>,
) -> Result<Value, String> {
    config.validate().map_err(|error| error.to_string())?;
    check_revision(root, revision)?;
    let previous = Config::load(root).ok();
    if matches!(change, Change::Save) {
        let _lock =
            store::lock_exclusive(&root.join(VAR_DIR)).map_err(|error| error.to_string())?;
        check_revision(root, revision)?;
        config.save(root).map_err(|error| error.to_string())?;
        return Ok(message(
            "主配置已保存。应用后会重新生成订阅并更新内核配置。",
        ));
    }
    let mut output = String::new();
    if matches!(change, Change::ArgoDisable) && config.argo.enabled {
        return Err("停用操作要求关闭 Argo 配置".into());
    }
    enum ArgoTokenAction {
        None,
        New(String),
        Reuse,
    }
    let argo_token_action = if matches!(change, Change::ArgoSetup) {
        if !config.argo.enabled {
            return Err("请先启用 Argo 配置".into());
        }
        let token = tunnel_token.unwrap_or_default();
        let token = token.trim();
        if token.is_empty() {
            if argo_token_exists(root) {
                ArgoTokenAction::Reuse
            } else {
                return Err("请输入 Cloudflare Tunnel Token".into());
            }
        } else if token.len() > 4096 || token.chars().any(char::is_whitespace) {
            return Err("Cloudflare Tunnel Token 无效".into());
        } else {
            ArgoTokenAction::New(format!("{token}\n"))
        }
    } else {
        if tunnel_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty())
        {
            return Err(
                "已填写新的 Cloudflare Tunnel Token，请使用「配置并启用」或「更新并重启连接器」保存 Token"
                    .into(),
            );
        }
        ArgoTokenAction::None
    };
    if is_live(root) {
        if matches!(change, Change::Deploy)
            || (matches!(change, Change::BindDomain) && previous.is_none())
        {
            script(root, "cores.sh", &["ensure"], None, &mut output)?;
            if let Err(error) = script(root, "net.sh", &["tune-sysctl"], None, &mut output) {
                output.push_str(&format!("内核参数调优未全部生效：{error}\n"));
            }
        }
        if matches!(change, Change::Deploy | Change::BindDomain) {
            if config.is_ip_mode() {
                script(
                    root,
                    "cert.sh",
                    &["self-signed", certificate_name(&config)?],
                    None,
                    &mut output,
                )?;
            } else {
                issue(root, &config, email, &mut output)?;
            }
        } else if previous.is_none() {
            return Err("首次初始化配置请使用「部署配置」中的「预览部署」".into());
        } else if previous
            .as_ref()
            .is_some_and(|old| old.tls_sni() != config.tls_sni())
        {
            return Err(
                "变更订阅域名或切换 IP/HTTPS 模式需要重新签发证书，请使用「部署配置」中的「预览部署」或在「域名与 HTTPS」中点击「预览并启用 HTTPS」"
                    .into(),
            );
        }
    }
    let report = applied(apply::apply_candidate_at_revision(
        root,
        config.clone(),
        Some(revision),
    ))
    .map_err(|error| format!("{output}{error}"))?;
    output.push_str(report["message"].as_str().unwrap_or_default());
    if is_live(root) {
        let ensure_newline = |buf: &mut String| {
            if !buf.is_empty() && !buf.ends_with('\n') {
                buf.push('\n');
            }
        };
        let script_result = match argo_token_action {
            ArgoTokenAction::New(ref token) => {
                let mut private_output = String::new();
                script(
                    root,
                    "argo.sh",
                    &["setup", "--token-stdin"],
                    Some(token),
                    &mut private_output,
                )
                .map_err(|_| {
                    "Argo 连接器设置失败。请检查 Tunnel Token 和 Cloudflared 服务，再重试。"
                        .to_owned()
                })
                .map(|()| {
                    ensure_newline(&mut output);
                    output.push_str("Argo Tunnel Token 已保存，Cloudflared 连接器已更新并重启。\n");
                })
            }
            ArgoTokenAction::Reuse => {
                ensure_newline(&mut output);
                script(
                    root,
                    "argo.sh",
                    &["setup", "--reuse-token"],
                    None,
                    &mut output,
                )
            }
            ArgoTokenAction::None
                if !config.argo.enabled
                    && (matches!(change, Change::ArgoDisable)
                        || previous.is_some_and(|old| old.argo.enabled)) =>
            {
                ensure_newline(&mut output);
                script(root, "argo.sh", &["disable"], None, &mut output)
            }
            ArgoTokenAction::None => Ok(()),
        };
        script_result.map_err(|error| format!("{output}\n{error}"))?;
    }
    Ok(message(output.trim()))
}

fn check_revision(root: &Path, revision: &str) -> Result<(), String> {
    if store::config_revision(root).map_err(|error| error.to_string())? != revision {
        return Err(apply::ApplyError::StaleConfig.to_string());
    }
    Ok(())
}

fn applied(result: Result<apply::ApplyReport, apply::ApplyError>) -> Result<Value, String> {
    let report = result.map_err(|error| error.to_string())?;
    let mut text = format!(
        "已应用并发布批次 {}（直连 {} 节点，sing-box {} 节点）",
        report.generation_id, report.direct_nodes, report.singbox_nodes
    );
    if let Some(warning) = report.argo_warning {
        text.push_str(&format!("\n注意：{warning}"));
    }
    Ok(json!({
        "message": text,
        "generation_id": report.generation_id,
    }))
}

fn message(text: &str) -> Value {
    json!({"message": text})
}

fn load_config(root: &Path) -> Result<Config, String> {
    Config::load(root).map_err(|error| error.to_string())
}

fn is_live(root: &Path) -> bool {
    root == Path::new("/")
}

fn require_live(root: &Path) -> Result<(), String> {
    if is_live(root) {
        Ok(())
    } else {
        Err("此操作需在 VPS 的系统根目录运行".into())
    }
}

fn path_arg(path: &Path) -> Result<&str, String> {
    path.to_str().ok_or("路径编码无效".into())
}

fn certificate_name(config: &Config) -> Result<&str, String> {
    let name = config.tls_sni();
    if is_valid_hostname(name) {
        Ok(name)
    } else {
        Err("无效证书名称".into())
    }
}

fn allow_web_ports(root: &Path, config: &Config, output: &mut String) {
    let acme = format!("{}/tcp", config.server.acme_port);
    let https = format!("{}/tcp", config.server.https_port);
    let _ = script(root, "net.sh", &["sync-ufw", &acme, &https], None, output);
}

fn issue(root: &Path, config: &Config, email: &str, output: &mut String) -> Result<(), String> {
    let email = email.trim();
    if !email.is_empty()
        && (email.len() > 254
            || !email.contains('@')
            || email.starts_with('-')
            || email.chars().any(char::is_whitespace))
    {
        return Err("证书账户邮箱格式无效".into());
    }
    let sni = certificate_name(config)?;
    allow_web_ports(root, config, output);
    script(root, "cert.sh", &["issue", sni, email], None, output)
}

fn argo_token_exists(root: &Path) -> bool {
    let path = root.join("etc/cloudflared/vps-sub-meter-argo.token");
    if !std::fs::symlink_metadata(&path)
        .is_ok_and(|meta| meta.file_type().is_file() && meta.len() > 0 && meta.len() <= 4096)
    {
        return false;
    }
    store::read_to_string_no_follow(&path).is_ok_and(|content| !content.trim().is_empty())
}

fn service_state(unit: &str) -> String {
    let systemctl = find_system_bin(&["/usr/bin/systemctl", "/bin/systemctl"], "systemctl");
    let Ok(out) = Command::new(systemctl).args(["is-active", unit]).output() else {
        return "unknown".to_owned();
    };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if text.is_empty() {
        "inactive".to_owned()
    } else {
        text
    }
}

fn find_system_bin<'a>(candidates: &[&'a str], fallback: &'a str) -> &'a str {
    candidates
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
        .unwrap_or(fallback)
}

fn parse_default_route_dev(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next() != Some("default") {
            return None;
        }
        words.find(|word| *word == "dev")?;
        words.next().filter(|dev| *dev != "lo").map(str::to_owned)
    })
}

fn detect_network(root: &Path) -> Result<Value, String> {
    require_live(root)?;
    let ip_bin = find_system_bin(
        &["/usr/sbin/ip", "/sbin/ip", "/usr/bin/ip", "/bin/ip"],
        "ip",
    );
    let curl_bin = find_system_bin(&["/usr/bin/curl", "/bin/curl"], "curl");
    let iface = ["-4", "-6"].iter().find_map(|family| {
        let out = Command::new(ip_bin)
            .args([*family, "route", "show", "default"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_default_route_dev(&String::from_utf8_lossy(&out.stdout))
    });
    let ip = ["https://api.ipify.org", "https://api64.ipify.org"]
        .iter()
        .find_map(|url| {
            let out = Command::new(curl_bin)
                .args([
                    "--disable",
                    "--noproxy",
                    "*",
                    "--fail",
                    "--silent",
                    "--connect-timeout",
                    "3",
                    "--max-time",
                    "5",
                    url,
                ])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<IpAddr>()
                .ok()
        });
    if iface.is_none() && ip.is_none() {
        return Err("未检测到公网 IP 或默认网卡，请手动填写".into());
    }
    Ok(json!({"interface": iface, "public_ip": ip}))
}

fn version(root: &Path, binary: &str) -> Option<String> {
    let mut command = Command::new(root.join("usr/local/bin").join(binary));
    command.arg(if binary == "cloudflared" {
        "--version"
    } else {
        "version"
    });
    capture(&mut command, None, 4096)
        .ok()
        .and_then(|output| output.lines().next().map(str::to_owned))
}

fn cli(root: &Path, args: &[&str]) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(exe);
    command.arg("--root").arg(root).args(args);
    capture(&mut command, None, 256 * 1024)
}

fn script(
    root: &Path,
    name: &str,
    args: &[&str],
    input: Option<&str>,
    log: &mut String,
) -> Result<(), String> {
    let path: PathBuf = root.join(LIB_DIR).join(name);
    if !path.is_file() {
        return Err(format!("找不到运维脚本 {name}"));
    }
    let mut command = Command::new("/bin/bash");
    command
        .arg(path)
        .args(args)
        .env_remove("BASH_ENV")
        .env_remove("ENV");
    match capture(&mut command, input, 16 * 1024) {
        Ok(text) => {
            if !text.is_empty() {
                log.push_str(&text);
                log.push('\n');
            }
            Ok(())
        }
        Err(error) => Err(format!("{log}{name} 执行失败：{error}")),
    }
}

fn capture(command: &mut Command, input: Option<&str>, limit: u64) -> Result<String, String> {
    let output = tempfile::tempfile().map_err(|error| error.to_string())?;
    let err = output.try_clone().map_err(|error| error.to_string())?;
    let mut reader = output.try_clone().map_err(|error| error.to_string())?;
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(output)
        .stderr(err)
        .spawn()
        .map_err(|error| error.to_string())?;
    let input_result = if let Some(input) = input {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(input.as_bytes())
    } else {
        Ok(())
    };
    let status = child.wait().map_err(|error| error.to_string())?;
    use std::io::Seek;
    reader.rewind().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let truncated = bytes.len() as u64 > limit;
    bytes.truncate(limit as usize);
    let mut text = String::from_utf8_lossy(&bytes).trim().to_owned();
    if truncated {
        text.push_str("\n[输出已截断]");
    }
    if !status.success() {
        return Err(format!("{status}\n{text}"));
    }
    input_result.map_err(|error| error.to_string())?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_and_ipv6_default_route_interface_names() {
        assert_eq!(
            parse_default_route_dev("default via 192.0.2.1 dev eth0 proto static\n"),
            Some("eth0".into())
        );
        assert_eq!(
            parse_default_route_dev(
                "default via fe80::1 dev ens3 proto ra metric 1024 expires 1799sec pref medium\n"
            ),
            Some("ens3".into())
        );
        assert_eq!(parse_default_route_dev("unreachable default\n"), None);
        assert_eq!(
            parse_default_route_dev(
                "unreachable default dev lo proto kernel metric 4294967295 error -101\n\
                 default via fe80::1 dev eth0 proto ra metric 100 pref medium\n"
            ),
            Some("eth0".into())
        );
        assert_eq!(
            parse_default_route_dev("unreachable default dev lo proto kernel\n"),
            None
        );
        assert_eq!(parse_default_route_dev("default dev lo scope link\n"), None);
        assert_eq!(parse_default_route_dev(""), None);
    }
}
