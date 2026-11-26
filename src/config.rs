//! Human-readable deployment configuration (`/etc/vps-sub-meter/config.toml`).
//! Long cryptographic secrets (ML-DSA-65, ML-KEM-768, passwords, tokens) live
//! separately in `/etc/vps-sub-meter/secrets.json` so `config.toml` stays concise.

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::store;

pub const ETC_DIR: &str = "etc/vps-sub-meter";
pub const VAR_DIR: &str = "var/lib/vps-sub-meter";
pub const LIB_DIR: &str = "usr/local/lib/vps-sub-meter";
pub const CONFIG_REL: &str = "etc/vps-sub-meter/config.toml";
pub const DEFAULT_CONFIG_REL: &str = "etc/vps-sub-meter/templates/default-config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("配置文件不存在: {0}")]
    Missing(PathBuf),
    #[error("无法读取或写入配置: {0}")]
    Io(#[from] std::io::Error),
    #[error("配置解析失败: {0}")]
    Parse(String),
    #[error("无法加载默认配置 {0}: {1}")]
    Defaults(PathBuf, String),
    #[error("配置校验失败: {0}")]
    Invalid(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineKind {
    Xray,
    SingBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WsProtocol {
    Vless,
    Vmess,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub profile_name: String,
    pub domain: String,
    pub public_ip: String,
    pub interface: String,
    pub ipv4_only: bool,
    pub latency_probe_url: String,
    pub server: ServerConfig,
    pub traffic: TrafficConfig,
    pub nodes: NodesConfig,
    pub argo: ArgoConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub acme_port: u16,
    pub https_port: u16,
    pub loopback_https_port: u16,
    pub extra_https_port: Option<u16>,
    pub http_fallback_port: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrafficConfig {
    pub monthly_gb: f64,
    pub reset_day: u8,
    pub reset_time: String,
    pub reset_tz: String,
    pub display_tz: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodesConfig {
    pub vless_reality_pqc: VlessRealityPqcConfig,
    pub vless_xhttp_pqc: VlessXhttpPqcConfig,
    pub anytls_tcp: AnytlsTcpConfig,
    pub hysteria2_hop: Hysteria2HopConfig,
    pub tuic_bbr: TuicBbrConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlessRealityPqcConfig {
    pub enabled: bool,
    pub name: String,
    pub port: u16,
    pub loopback_port: u16,
    pub decoy_sni: String,
    pub connect_host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlessXhttpPqcConfig {
    pub enabled: bool,
    pub name: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnytlsTcpConfig {
    pub enabled: bool,
    pub name: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hysteria2HopConfig {
    pub enabled: bool,
    pub name: String,
    pub port: u16,
    pub hop_range: Option<String>,
    pub hop_interval_sec: u16,
    pub obfs_salamander: bool,
    pub up_mbps: u32,
    pub down_mbps: u32,
    pub server_up_mbps: u32,
    pub server_down_mbps: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuicBbrConfig {
    pub enabled: bool,
    pub name: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgoConfig {
    pub enabled: bool,
    pub profile_name: String,
    pub tunnel_domain: String,
    pub sub_domain: String,
    pub loopback_sub_port: u16,
    pub preferred_ips_file: String,
    pub ws_inbounds: Vec<ArgoWsInbound>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgoWsInbound {
    pub id: String,
    pub name: String,
    pub protocol: WsProtocol,
    pub engine: EngineKind,
    pub listen: String,
    pub port: u16,
    pub tls: bool,
}

impl Config {
    pub fn path(root: &Path) -> PathBuf {
        root.join(CONFIG_REL)
    }

    pub fn load_defaults(root: &Path) -> Result<Self, ConfigError> {
        let path = root.join(DEFAULT_CONFIG_REL);
        read_defaults(&path)?
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::Defaults(path, e.to_string()))
    }

    pub fn load(root: &Path) -> Result<Self, ConfigError> {
        let path = Self::path(root);
        let text = match store::read_to_string_no_follow(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ConfigError::Missing(path));
            }
            Err(e) => return Err(e.into()),
        };
        let cfg: Self = match toml::from_str(&text) {
            Ok(cfg) => cfg,
            Err(main_error) => {
                let overrides: toml::Table =
                    toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
                let path = root.join(DEFAULT_CONFIG_REL);
                let mut defaults = read_defaults(&path)
                    .map_err(|e| ConfigError::Parse(format!("{main_error}\n{e}")))?;
                overlay_config(&mut defaults, overrides);
                defaults
                    .try_into()
                    .map_err(|e: toml::de::Error| ConfigError::Parse(e.to_string()))?
            }
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn save(&self, root: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        let text = self.to_commented_toml();
        store::write_atomic_preserving_owner(&Self::path(root), text.as_bytes(), 0o640)?;
        Ok(())
    }

    pub fn is_ip_mode(&self) -> bool {
        self.domain.trim().is_empty() || self.domain.trim().parse::<IpAddr>().is_ok()
    }

    /// Primary connect host for TLS domain nodes.
    pub fn primary_host(&self) -> &str {
        if !self.is_ip_mode() {
            self.domain.trim()
        } else {
            self.public_ip.trim()
        }
    }

    /// TLS SNI used by certificate-based nodes (XHTTP / AnyTLS / Hy2 / TUIC).
    pub fn tls_sni(&self) -> &str {
        if !self.is_ip_mode() {
            self.domain.trim()
        } else {
            "www.bing.com"
        }
    }

    /// Whether Xray fronts public TCP :443 and forwards subscription SNI to loopback.
    pub fn xray_fronts_443(&self) -> bool {
        !self.is_ip_mode()
            && self.nodes.vless_reality_pqc.enabled
            && self.nodes.vless_reality_pqc.port == self.server.https_port
    }

    pub fn any_xray_enabled(&self) -> bool {
        self.nodes.vless_reality_pqc.enabled
            || self.nodes.vless_xhttp_pqc.enabled
            || (self.argo.enabled
                && self
                    .argo
                    .ws_inbounds
                    .iter()
                    .any(|w| w.engine == EngineKind::Xray))
    }

    pub fn any_singbox_enabled(&self) -> bool {
        self.nodes.anytls_tcp.enabled
            || self.nodes.hysteria2_hop.enabled
            || self.nodes.tuic_bbr.enabled
            || (self.argo.enabled
                && self
                    .argo
                    .ws_inbounds
                    .iter()
                    .any(|w| w.engine == EngineKind::SingBox))
    }

    pub fn parse_hop_range(&self) -> Result<Option<(u16, u16)>, ConfigError> {
        let Some(raw) = self.nodes.hysteria2_hop.hop_range.as_deref() else {
            return Ok(None);
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let (a, b) = trimmed
            .split_once('-')
            .ok_or_else(|| ConfigError::Invalid("Hy2 跳跃端口范围格式应为 起始-结束".into()))?;
        let start: u16 = a
            .trim()
            .parse()
            .map_err(|_| ConfigError::Invalid("Hy2 跳跃起始端口无效".into()))?;
        let end: u16 = b
            .trim()
            .parse()
            .map_err(|_| ConfigError::Invalid("Hy2 跳跃结束端口无效".into()))?;
        if start < 1024 || start >= end {
            return Err(ConfigError::Invalid(
                "Hy2 跳跃范围须在 1024-65535 内且起始小于结束".into(),
            ));
        }
        Ok(Some((start, end)))
    }

    pub fn parse_reset_hm(&self) -> Result<(u32, u32), ConfigError> {
        let (h, m) = self
            .traffic
            .reset_time
            .trim()
            .split_once(':')
            .ok_or_else(|| ConfigError::Invalid("reset_time 格式必须为 HH:MM".into()))?;
        let hour: u32 = h
            .parse()
            .map_err(|_| ConfigError::Invalid("reset_time 小时无效".into()))?;
        let min: u32 = m
            .parse()
            .map_err(|_| ConfigError::Invalid("reset_time 分钟无效".into()))?;
        if hour > 23 || min > 59 {
            return Err(ConfigError::Invalid("reset_time 超出 00:00-23:59".into()));
        }
        Ok((hour, min))
    }

    pub fn monthly_limit_bytes(&self) -> u64 {
        if self.traffic.monthly_gb <= 0.0 {
            0
        } else {
            (self.traffic.monthly_gb * 1024.0 * 1024.0 * 1024.0).floor() as u64
        }
    }

    pub fn argo_subscription_base_url(&self, token: &str) -> String {
        format!("https://{}/sub/{token}", self.argo.sub_domain.trim())
    }

    pub fn subscription_base_url(&self, token: &str) -> String {
        if self.is_ip_mode() {
            let host = format_url_host(&self.public_ip);
            format!(
                "http://{host}:{}/sub/{token}",
                self.server.http_fallback_port
            )
        } else if self.server.https_port == 443 {
            format!("https://{}/sub/{token}", self.domain.trim())
        } else {
            format!(
                "https://{}:{}/sub/{token}",
                self.domain.trim(),
                self.server.https_port
            )
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_base()?;
        self.validate_traffic()?;

        let mut tcp_ports = BTreeSet::new();
        let mut udp_ports = BTreeSet::new();
        self.validate_server_ports(&mut tcp_ports)?;
        self.validate_nodes(&mut tcp_ports, &mut udp_ports)?;
        self.validate_argo(&mut tcp_ports)
    }

    fn validate_base(&self) -> Result<(), ConfigError> {
        if self.profile_name.trim().is_empty() || self.profile_name.chars().any(char::is_control) {
            return Err(ConfigError::Invalid("profile_name 无效".into()));
        }
        if !self.domain.trim().is_empty() && !is_valid_host(self.domain.trim()) {
            return Err(ConfigError::Invalid("domain 格式无效".into()));
        }
        if self.public_ip.trim().parse::<IpAddr>().is_err() && self.is_ip_mode() {
            return Err(ConfigError::Invalid(
                "无域名模式下必须提供有效 public_ip".into(),
            ));
        }
        if !self.public_ip.trim().is_empty() && self.public_ip.trim().parse::<IpAddr>().is_err() {
            return Err(ConfigError::Invalid("public_ip 必须是合法 IP 地址".into()));
        }
        if !is_valid_interface(self.interface.trim()) {
            return Err(ConfigError::Invalid("interface 网卡名无效".into()));
        }
        validate_url_http(&self.latency_probe_url)
    }

    fn validate_traffic(&self) -> Result<(), ConfigError> {
        if !self.traffic.monthly_gb.is_finite() || self.traffic.monthly_gb < 0.0 {
            return Err(ConfigError::Invalid("traffic.monthly_gb 不能为负数".into()));
        }
        if !(1..=31).contains(&self.traffic.reset_day) {
            return Err(ConfigError::Invalid("traffic.reset_day 须在 1..=31".into()));
        }
        self.parse_reset_hm()?;
        self.traffic
            .reset_tz
            .parse::<Tz>()
            .map_err(|_| ConfigError::Invalid("traffic.reset_tz 不是有效 IANA 时区".into()))?;
        self.traffic
            .display_tz
            .parse::<Tz>()
            .map_err(|_| ConfigError::Invalid("traffic.display_tz 不是有效 IANA 时区".into()))?;
        Ok(())
    }

    fn validate_server_ports(&self, tcp_ports: &mut BTreeSet<u16>) -> Result<(), ConfigError> {
        if self.is_ip_mode() {
            check_node_port(self.server.http_fallback_port)?;
            tcp_ports.insert(self.server.http_fallback_port);
            return Ok(());
        }
        if self.server.acme_port == 0 || self.server.https_port == 0 {
            return Err(ConfigError::Invalid(
                "acme_port / https_port 不能为 0".into(),
            ));
        }
        tcp_ports.insert(self.server.acme_port);
        if !self.xray_fronts_443() {
            tcp_ports.insert(self.server.https_port);
        } else {
            check_high_port(self.server.loopback_https_port)?;
            tcp_ports.insert(self.server.loopback_https_port);
        }
        if let Some(extra) = self.server.extra_https_port {
            check_node_port(extra)?;
            if !tcp_ports.insert(extra) {
                return Err(ConfigError::Invalid(
                    "extra_https_port 与其他 TCP 端口冲突".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_nodes(
        &self,
        tcp_ports: &mut BTreeSet<u16>,
        udp_ports: &mut BTreeSet<u16>,
    ) -> Result<(), ConfigError> {
        let enabled_count = [
            self.nodes.vless_reality_pqc.enabled,
            self.nodes.vless_xhttp_pqc.enabled,
            self.nodes.anytls_tcp.enabled,
            self.nodes.hysteria2_hop.enabled,
            self.nodes.tuic_bbr.enabled,
        ]
        .into_iter()
        .filter(|x| *x)
        .count();
        if enabled_count == 0 {
            return Err(ConfigError::Invalid("至少需要启用一个主力代理节点".into()));
        }

        let mut node_names = BTreeSet::new();
        let mut push_name = |name: &str| -> Result<(), ConfigError> {
            let trimmed = name.trim();
            if trimmed.is_empty()
                || trimmed.chars().any(char::is_control)
                || ["代理", "TCP优选", "自动选择", "DIRECT", "REJECT"].contains(&trimmed)
                || !node_names.insert(trimmed.to_owned())
            {
                return Err(ConfigError::Invalid(format!("节点名称无效或重复: {name}")));
            }
            Ok(())
        };

        if self.nodes.vless_reality_pqc.enabled {
            let n = &self.nodes.vless_reality_pqc;
            push_name(&n.name)?;
            check_node_port(n.port)?;
            if !is_valid_hostname(n.decoy_sni.trim()) {
                return Err(ConfigError::Invalid("Reality decoy_sni 无效".into()));
            }
            if let Some(host) = &n.connect_host
                && !host.trim().is_empty()
                && !is_valid_host(host.trim())
            {
                return Err(ConfigError::Invalid(
                    "vless_reality_pqc.connect_host 无效".into(),
                ));
            }
            if self.xray_fronts_443() {
                check_high_port(n.loopback_port)?;
                if !tcp_ports.insert(n.port) || !tcp_ports.insert(n.loopback_port) {
                    return Err(ConfigError::Invalid("Reality 443/loopback 端口冲突".into()));
                }
            } else if !tcp_ports.insert(n.port) {
                return Err(ConfigError::Invalid("vless_reality_pqc 端口冲突".into()));
            }
        }

        for (enabled, name, port, label) in [
            (
                self.nodes.vless_xhttp_pqc.enabled,
                &self.nodes.vless_xhttp_pqc.name,
                self.nodes.vless_xhttp_pqc.port,
                "vless_xhttp_pqc",
            ),
            (
                self.nodes.anytls_tcp.enabled,
                &self.nodes.anytls_tcp.name,
                self.nodes.anytls_tcp.port,
                "anytls_tcp",
            ),
        ] {
            if enabled {
                push_name(name)?;
                check_node_port(port)?;
                if !tcp_ports.insert(port) {
                    return Err(ConfigError::Invalid(format!("{label} 端口冲突")));
                }
            }
        }

        if self.nodes.hysteria2_hop.enabled {
            let n = &self.nodes.hysteria2_hop;
            push_name(&n.name)?;
            check_node_port(n.port)?;
            if !udp_ports.insert(n.port) {
                return Err(ConfigError::Invalid("hysteria2_hop UDP 端口冲突".into()));
            }
            if !(5..=3600).contains(&n.hop_interval_sec) {
                return Err(ConfigError::Invalid(
                    "hop_interval_sec 须在 5..=3600".into(),
                ));
            }
            if let Some((start, end)) = self.parse_hop_range()?
                && udp_ports
                    .iter()
                    .filter(|p| **p != n.port)
                    .chain(tcp_ports.iter())
                    .any(|p| (start..=end).contains(p))
            {
                return Err(ConfigError::Invalid(
                    "Hy2 跳跃范围与其他服务端口重叠".into(),
                ));
            }
        }

        if self.nodes.tuic_bbr.enabled {
            let n = &self.nodes.tuic_bbr;
            push_name(&n.name)?;
            check_node_port(n.port)?;
            if !udp_ports.insert(n.port) {
                return Err(ConfigError::Invalid("tuic_bbr UDP 端口冲突".into()));
            }
            if let Some((start, end)) = self.parse_hop_range()?
                && (start..=end).contains(&n.port)
            {
                return Err(ConfigError::Invalid(
                    "tuic_bbr 端口落入 Hy2 跳跃范围".into(),
                ));
            }
        }

        Ok(())
    }

    fn validate_argo(&self, tcp_ports: &mut BTreeSet<u16>) -> Result<(), ConfigError> {
        if !self.argo.enabled {
            return Ok(());
        }
        if !is_valid_hostname(self.argo.tunnel_domain.trim())
            || !is_valid_hostname(self.argo.sub_domain.trim())
        {
            return Err(ConfigError::Invalid(
                "启用 Argo 时 tunnel_domain 与 sub_domain 必须为有效域名".into(),
            ));
        }
        check_high_port(self.argo.loopback_sub_port)?;
        if !tcp_ports.insert(self.argo.loopback_sub_port) {
            return Err(ConfigError::Invalid(
                "argo.loopback_sub_port 端口冲突".into(),
            ));
        }
        if self.argo.ws_inbounds.is_empty() {
            return Err(ConfigError::Invalid(
                "启用 Argo 时至少需要一条 ws_inbounds".into(),
            ));
        }
        let mut ws_ids = BTreeSet::new();
        for ws in &self.argo.ws_inbounds {
            if ws.id.trim().is_empty()
                || !ws
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || !ws_ids.insert(ws.id.clone())
            {
                return Err(ConfigError::Invalid(format!(
                    "Argo WS id 无效或重复: {}",
                    ws.id
                )));
            }
            check_high_port(ws.port)?;
            if ws.listen.parse::<IpAddr>().is_err() {
                return Err(ConfigError::Invalid("Argo WS listen 地址无效".into()));
            }
            if !tcp_ports.insert(ws.port) {
                return Err(ConfigError::Invalid(format!(
                    "Argo WS 端口 {} 冲突",
                    ws.port
                )));
            }
        }
        Ok(())
    }

    pub fn to_commented_toml(&self) -> String {
        let raw = toml::to_string_pretty(self).expect("Config serializes to TOML");
        format!(
            "# vps-sub-meter v2 配置文件 (/etc/vps-sub-meter/config.toml)\n\
             # 修改后运行 `vsm apply` 自动调用官方内核校验并平滑生效。\n\
             # 密钥、UUID、密码与订阅 Token 单独存放在 /etc/vps-sub-meter/secrets.json (0600)。\n\n{raw}"
        )
    }
}

fn read_defaults(path: &Path) -> Result<toml::Table, ConfigError> {
    let text = store::read_to_string_no_follow(path)
        .map_err(|e| ConfigError::Defaults(path.to_path_buf(), e.to_string()))?;
    toml::from_str(&text).map_err(|e| ConfigError::Defaults(path.to_path_buf(), e.to_string()))
}

fn overlay_config(defaults: &mut toml::Table, overrides: toml::Table) {
    for (key, value) in overrides {
        match (defaults.get_mut(&key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(overlay)) => {
                overlay_config(base, overlay);
            }
            (_, value) => {
                defaults.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn test_defaults() -> Config {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/default-config.toml");
    read_defaults(&path).unwrap().try_into().unwrap()
}

pub fn check_node_port(port: u16) -> Result<(), ConfigError> {
    if port == 443 || port >= 1024 {
        Ok(())
    } else {
        Err(ConfigError::Invalid(format!(
            "节点端口 {port} 无效，请使用 443 或 1024-65535"
        )))
    }
}

pub fn check_high_port(port: u16) -> Result<(), ConfigError> {
    if port >= 1024 {
        Ok(())
    } else {
        Err(ConfigError::Invalid(format!(
            "内部/备用端口 {port} 必须 >= 1024"
        )))
    }
}

pub fn is_valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('.')
        && !s.ends_with('.')
        && s.parse::<IpAddr>().is_err()
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

pub fn is_valid_host(s: &str) -> bool {
    s.parse::<IpAddr>().is_ok() || is_valid_hostname(s)
}

pub fn is_valid_interface(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 15
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn validate_url_http(s: &str) -> Result<(), ConfigError> {
    let parsed =
        url::Url::parse(s).map_err(|_| ConfigError::Invalid(format!("URL 格式无效: {s}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ConfigError::Invalid(format!("仅支持 http/https URL: {s}")));
    }
    Ok(())
}

pub fn format_url_host(host: &str) -> String {
    match host.trim().parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{}]", host.trim()),
        _ => host.trim().to_owned(),
    }
}

pub fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_five_nodes_domain_config_validates_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        cfg.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded, cfg);
        assert!(loaded.xray_fronts_443());
        assert!(loaded.any_xray_enabled());
        assert!(loaded.any_singbox_enabled());
    }

    #[test]
    fn sparse_main_config_uses_external_defaults_and_preserves_explicit_values() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let defaults_path = root.join(DEFAULT_CONFIG_REL);
        std::fs::create_dir_all(defaults_path.parent().unwrap()).unwrap();
        let mut defaults = test_defaults();
        defaults.nodes.vless_reality_pqc.decoy_sni = "decoy.example.com".into();
        defaults.traffic.reset_day = 18;
        std::fs::write(&defaults_path, defaults.to_commented_toml()).unwrap();
        std::fs::write(
            Config::path(root),
            "domain = \"sub.example.com\"\n[traffic]\nreset_day = 12\n[argo]\nws_inbounds = []\n",
        )
        .unwrap();

        let loaded = Config::load(root).unwrap();
        assert_eq!(
            loaded.nodes.vless_reality_pqc.decoy_sni,
            "decoy.example.com"
        );
        assert_eq!(loaded.traffic.reset_day, 12);
        assert!(loaded.argo.ws_inbounds.is_empty());

        loaded.save(root).unwrap();
        std::fs::write(defaults_path, "invalid toml").unwrap();
        assert_eq!(Config::load(root).unwrap(), loaded);
    }

    #[test]
    fn detects_hy2_hop_port_overlap_with_tuic() {
        let mut cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        cfg.nodes.tuic_bbr.port = 31000;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn allows_standard_cloudflare_ports_2053_2083_2096() {
        let cfg = Config {
            domain: "sub.example.com".into(),
            public_ip: "203.0.113.10".into(),
            ..crate::config::test_defaults()
        };
        assert!(cfg.validate().is_ok());
    }
}
