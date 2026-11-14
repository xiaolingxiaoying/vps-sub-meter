//! Authoritative, root-private managed nodes. Legacy configuration is an import
//! source only once this document has been committed by deployment::apply.
use crate::{
    canonical::CanonicalNode,
    config::DeploymentConfig,
    core::{CoreKind, presets},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const SOURCE: &str = "etc/sbctl/nodes.json";
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self("无法读写托管节点文件".into())
    }
}
impl From<crate::config::ConfigError> for Error {
    fn from(_: crate::config::ConfigError) -> Self {
        Self("无法读写部署配置".into())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    SingBox,
    Xray,
    Dual,
}
impl Mode {
    pub fn allows(self, core: CoreKind) -> bool {
        self == Self::Dual
            || matches!(
                (self, core),
                (Self::SingBox, CoreKind::SingBox) | (Self::Xray, CoreKind::Xray)
            )
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
            Self::Dual => "dual",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "settings",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum Data {
    Legacy(CanonicalNode),
    Combination(presets::Node),
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub name: String,
    /// Client references remain stable when the display name changes.
    pub tag: String,
    pub core: CoreKind,
    pub enabled: bool,
    pub listen: String,
    pub data: Data,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    #[default]
    Select,
    Urltest,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    #[serde(default)]
    pub strategy: Strategy,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    pub revision: u64,
    pub mode: Mode,
    /// Independent of the selected server engine; pinned during migration.
    pub client_version: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("operating system random source unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn read_optional(root: &Path) -> Result<Option<Registry>> {
    let bytes = match crate::config::read_managed_file_no_follow(&root.join(SOURCE)) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error("节点仓库超过 4 MiB".into()));
    }
    let value: Registry = serde_json::from_slice(&bytes)
        .map_err(|_| Error("节点仓库格式错误；未应用任何修改".into()))?;
    if value.schema_version != 1 {
        return Err(Error("不支持的节点仓库版本".into()));
    }
    Ok(Some(value))
}
pub fn load_or_import(root: &Path, config: &DeploymentConfig) -> Result<Registry> {
    if let Some(value) = read_optional(root)? {
        value.validate(config)?;
        return Ok(value);
    }
    let mut nodes: Vec<Node> = crate::canonical::nodes(config)
        .into_iter()
        .map(|data| Node {
            id: data.tag().into(),
            name: data.tag().into(),
            tag: data.tag().into(),
            core: CoreKind::SingBox,
            enabled: true,
            listen: "::".into(),
            data: Data::Legacy(data),
        })
        .collect();
    match crate::config::read_managed_file_no_follow(&root.join("etc/sbctl/protocol-presets.json"))
    {
        Ok(bytes) => {
            let extra: Vec<presets::Node> =
                serde_json::from_slice(&bytes).map_err(|_| Error("旧组合源文件格式错误".into()))?;
            for (i, data) in extra.into_iter().enumerate() {
                nodes.push(Node {
                    id: format!("legacy-xray-{i}"),
                    name: data.name.clone(),
                    tag: data.name.clone(),
                    core: CoreKind::Xray,
                    enabled: true,
                    listen: "::".into(),
                    data: Data::Combination(data),
                });
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let kernel = root.join("usr/local/bin/sing-box");
    let legacy_nodes = crate::canonical::nodes(config);
    let profile = crate::subscription::resolve_full_profile(
        config,
        &legacy_nodes,
        kernel.is_file().then_some(kernel.as_path()),
    );
    let value = Registry {
        schema_version: 1,
        revision: 0,
        mode: if nodes.iter().any(|n| n.core == CoreKind::Xray) {
            Mode::Dual
        } else {
            Mode::SingBox
        },
        client_version: profile.version.to_string(),
        nodes,
        groups: Vec::new(),
    };
    value.validate(config)?;
    Ok(value)
}
impl Registry {
    pub fn validate(&self, config: &DeploymentConfig) -> Result<()> {
        if self.schema_version != 1 || self.nodes.len() > 512 || self.groups.len() > 128 {
            return Err(Error(
                "节点仓库版本或数量不受支持（最多 512 节点、128 组）".into(),
            ));
        }
        if !crate::subscription::all_sing_box_version_profiles()
            .iter()
            .any(|p| p.version.to_string() == self.client_version)
        {
            return Err(Error("客户端目标版本不在已验证版本表中".into()));
        }
        let mut ids = BTreeSet::new();
        let mut tags = BTreeSet::new();
        let mut ports = BTreeSet::new();
        let reserved = [
            "代理",
            "自动选择",
            "DIRECT",
            "direct",
            "REJECT",
            "block-out",
            "dns-out",
            "代理分组",
            "AI服务",
            "流媒体",
            "Telegram",
            "故障转移",
        ];
        for node in &self.nodes {
            if !identifier(&node.id)
                || !ids.insert(node.id.as_str())
                || !valid_label(&node.name)
                || !valid_label(&node.tag)
                || !tags.insert(node.tag.as_str())
                || reserved.contains(&node.tag.as_str())
            {
                return Err(Error("节点 ID、名称或客户端引用名称无效或重复".into()));
            }
            if node.listen.parse::<std::net::IpAddr>().is_err() || node.port() < 10000 {
                return Err(Error("节点监听地址无效或端口不在 10000–65535".into()));
            }
            node.validate_data()?;
            let proxy_secrets = match &node.data {
                Data::Legacy(n) => n.secrets(),
                Data::Combination(n) => vec![n.credential.as_str()],
            };
            if proxy_secrets.iter().any(|secret| {
                *secret == config.subscription_credential
                    || config
                        .subscription_credentials
                        .iter()
                        .any(|c| *secret == c.credential)
            }) {
                return Err(Error("代理凭据不得复用订阅凭据".into()));
            }
            if !node.supports(node.core) {
                return Err(Error(format!(
                    "节点 {} 没有对应内核的已验证适配器",
                    node.id
                )));
            }
            if !ports.insert(node.port())
                || config.http_port == Some(node.port())
                || config.subscription_listen_port == Some(node.port())
            {
                return Err(Error("节点端口重复或与订阅监听器冲突".into()));
            }
            if let Some(api) = &config.server_clash_api
                && api.port == node.port()
            {
                return Err(Error("节点端口与观察 API 冲突".into()));
            }
            if node.enabled && !self.mode.allows(node.core) {
                return Err(Error(format!(
                    "节点 {} 不属于目标运行模式；请迁移或显式停用",
                    node.id
                )));
            }
            if let Some(hop) = &config.hysteria2_port_hopping
                && (hop.start..=hop.end).contains(&node.port())
            {
                return Err(Error("节点端口与 Hy2 跳跃范围冲突".into()));
            }
        }
        let mut group_ids = BTreeSet::new();
        for group in &self.groups {
            if !identifier(&group.id)
                || !group_ids.insert(&group.id)
                || !valid_label(&group.name)
                || !tags.insert(group.name.as_str())
                || reserved.contains(&group.name.as_str())
            {
                return Err(Error(
                    "分组 ID 或名称无效、重复或与节点/内置策略冲突".into(),
                ));
            }
            let mut seen = BTreeSet::new();
            if group
                .members
                .iter()
                .any(|id| !ids.contains(id.as_str()) || !seen.insert(id))
            {
                return Err(Error("分组包含不存在或重复的节点 ID".into()));
            }
        }
        if config.hysteria2_port_hopping.is_some() && !self.nodes.iter().any(|n|n.enabled&&n.core==CoreKind::SingBox&&matches!(&n.data,Data::Legacy(CanonicalNode::Hysteria2{port,..}) if config.hysteria2.as_ref().is_some_and(|h|h.listen_port==*port))) {
            return Err(Error("Hy2 端口跳跃仍启用：请先在跳跃配置中关闭，再停用、移除或更改其基础监听节点".into()));
        }
        if config.certificate_mode == crate::config::CertificateMode::Domain {
            for n in &self.nodes {
                let sni = match &n.data {
                    Data::Legacy(CanonicalNode::VlessReality { .. }) => continue,
                    Data::Legacy(n) => n.tls_server_name(),
                    Data::Combination(n) if n.preset.is_reality() => continue,
                    Data::Combination(n) => &n.server_name,
                };
                if sni != config.protocol_server_name() {
                    return Err(Error("节点 TLS SNI 与托管域名证书不匹配".into()));
                }
            }
        }
        Ok(())
    }
    pub fn node_mut(&mut self, id: &str) -> Result<&mut Node> {
        self.nodes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or_else(|| Error("找不到节点 ID".into()))
    }
    pub fn choose_core(&self, node: &Node) -> Result<CoreKind> {
        [CoreKind::SingBox, CoreKind::Xray]
            .into_iter()
            .find(|c| self.mode.allows(*c) && node.supports(*c))
            .ok_or_else(|| Error("当前模式无法承载此组合；请先切换为 dual 或指定受支持内核".into()))
    }
    pub fn switch(&mut self, mode: Mode, disable: &[String]) -> Result<()> {
        for id in disable {
            self.node_mut(id)?.enabled = false;
        }
        for node in &mut self.nodes {
            if !node.enabled || mode.allows(node.core) {
                continue;
            }
            let target = if mode == Mode::Xray {
                CoreKind::Xray
            } else {
                CoreKind::SingBox
            };
            if !node.supports(target) {
                return Err(Error(format!("节点 {} 无法迁移，请显式停用", node.id)));
            }
            node.core = target;
        }
        self.mode = mode;
        Ok(())
    }
}
fn valid_label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 160 && !s.chars().any(char::is_control)
}
impl Node {
    pub fn port(&self) -> u16 {
        match &self.data {
            Data::Legacy(n) => n.port(),
            Data::Combination(n) => n.port,
        }
    }
    pub fn preset(&self) -> String {
        match &self.data {
            Data::Legacy(n) => n.protocol().to_string(),
            Data::Combination(n) => serde_json::to_value(n.preset)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
        }
    }
    pub fn supports(&self, core: CoreKind) -> bool {
        match (&self.data, core) {
            (Data::Legacy(_), CoreKind::SingBox) => true,
            (
                Data::Legacy(
                    CanonicalNode::VlessReality { .. } | CanonicalNode::VmessWebsocket { .. },
                ),
                CoreKind::Xray,
            ) => true,
            (Data::Combination(n), CoreKind::SingBox) => !n.preset.is_xhttp(),
            (Data::Combination(_), CoreKind::Xray) => true,
            _ => false,
        }
    }
    pub fn validate_data(&self) -> Result<()> {
        match &self.data {
            Data::Combination(n) => n.validate().map_err(|_| Error("组合字段或凭据无效".into())),
            Data::Legacy(n) => {
                if !crate::config::host_is_valid(n.host())
                    || !crate::config::hostname_is_valid(n.tls_server_name())
                {
                    return Err(Error("节点主机或 TLS SNI 无效".into()));
                }
                let value = serde_json::to_value(n).unwrap();
                if let Some(uuid) = value["uuid"].as_str()
                    && !valid_uuid(uuid)
                {
                    return Err(Error("节点 UUID 无效".into()));
                }
                if let Some(password) = value["password"].as_str()
                    && (password.is_empty() || password.chars().any(char::is_control))
                {
                    return Err(Error("节点密码无效".into()));
                }
                if let CanonicalNode::VlessReality {
                    private_key,
                    public_key,
                    short_id,
                    decoy_sni,
                    ..
                } = n
                {
                    let decode = |s: &str| {
                        base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .decode(s)
                            .ok()
                            .and_then(|b| <[u8; 32]>::try_from(b).ok())
                    };
                    let private =
                        decode(private_key).ok_or_else(|| Error("Reality 密钥无效".into()))?;
                    if decode(public_key)
                        != Some(x25519_dalek::x25519(
                            private,
                            x25519_dalek::X25519_BASEPOINT_BYTES,
                        ))
                        || short_id.len() > 16
                        || short_id.len() % 2 != 0
                        || !short_id.bytes().all(|b| b.is_ascii_hexdigit())
                        || !crate::config::hostname_is_valid(decoy_sni)
                    {
                        return Err(Error("Reality 公私钥、short ID 或伪装域名无效".into()));
                    }
                }
                if let Some(path) = value["path"].as_str()
                    && (!path.starts_with('/') || path.chars().any(char::is_control))
                {
                    return Err(Error("WebSocket 路径无效".into()));
                }
                Ok(())
            }
        }
    }
    /// Typed roundtrip: editing never accepts arbitrary server JSON.
    pub fn patch(&mut self, field: &str, value: serde_json::Value) -> Result<()> {
        let mut data = match &self.data {
            Data::Legacy(n) => serde_json::to_value(n),
            Data::Combination(n) => serde_json::to_value(n),
        }
        .unwrap();
        let key = match (&self.data, field) {
            (Data::Legacy(_), "port") => "port",
            (Data::Legacy(_), "server") => "host",
            (Data::Legacy(CanonicalNode::VlessReality { .. }), "server_name") => "decoy_sni",
            (Data::Legacy(_), "server_name") => "tls_server_name",
            _ => field,
        };
        if !data.as_object().unwrap().contains_key(key) {
            return Err(Error("该预设不支持此字段".into()));
        }
        data[key] = value;
        self.data = match &self.data {
            Data::Legacy(_) => Data::Legacy(
                serde_json::from_value(data).map_err(|_| Error("节点字段无效".into()))?,
            ),
            Data::Combination(_) => Data::Combination(
                serde_json::from_value(data).map_err(|_| Error("组合字段无效".into()))?,
            ),
        };
        self.validate_data()
    }
    pub fn renew_secrets(&mut self) -> Result<()> {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| Error("随机数生成失败".into()))?;
        random[6] = (random[6] & 15) | 64;
        random[8] = (random[8] & 63) | 128;
        let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let uuid = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        let password = crate::config::generate_subscription_credential()?;
        match &mut self.data {
            Data::Legacy(
                CanonicalNode::VlessReality { uuid: u, .. }
                | CanonicalNode::VmessWebsocket { uuid: u, .. },
            ) => *u = uuid,
            Data::Legacy(CanonicalNode::Tuic {
                uuid: u,
                password: p,
                ..
            }) => {
                *u = uuid;
                *p = password;
            }
            Data::Legacy(
                CanonicalNode::Hysteria2 { password: p, .. }
                | CanonicalNode::Anytls { password: p, .. },
            ) => *p = password,
            Data::Combination(n) => {
                n.credential = if n.preset.is_trojan() { password } else { uuid }
            }
        }
        // Every copy receives a fresh Reality key pair too.
        let mut private = [0u8; 32];
        getrandom::fill(&mut private).map_err(|_| Error("随机数生成失败".into()))?;
        let encode = |b| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let public = x25519_dalek::x25519(private, x25519_dalek::X25519_BASEPOINT_BYTES);
        match &mut self.data {
            Data::Legacy(CanonicalNode::VlessReality {
                private_key,
                public_key,
                short_id,
                ..
            }) => {
                *private_key = encode(private);
                *public_key = encode(public);
                *short_id = new_id()[..16].into();
            }
            Data::Combination(n) => {
                if let Some(reality) = &mut n.reality {
                    reality.private_key = encode(private);
                    reality.public_key = encode(public);
                    reality.short_id = new_id()[..16].into();
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub fn free_port(registry: &Registry, config: &DeploymentConfig) -> Result<u16> {
    for port in 20000..=65535 {
        if registry.nodes.iter().any(|n| n.port() == port)
            || config.http_port == Some(port)
            || config.subscription_listen_port == Some(port)
            || config
                .server_clash_api
                .as_ref()
                .is_some_and(|a| a.port == port)
            || config
                .hysteria2_port_hopping
                .as_ref()
                .is_some_and(|h| (h.start..=h.end).contains(&port))
        {
            continue;
        }
        if let (Ok(_tcp), Ok(_udp)) = (
            std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)),
            std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, port)),
        ) {
            return Ok(port);
        }
    }
    Err(Error("没有可用节点端口".into()))
}

pub fn create(
    registry: &Registry,
    config: &DeploymentConfig,
    preset: &str,
    name: &str,
    port: Option<u16>,
    core: Option<CoreKind>,
) -> Result<Node> {
    let id = new_id();
    let port = port
        .map(Ok)
        .unwrap_or_else(|| free_port(registry, config))?;
    let host = config
        .proxy_host
        .as_deref()
        .unwrap_or(&config.subscription_host)
        .to_string();
    let sni = config.protocol_server_name().to_string();
    let uuid = "00000000-0000-4000-8000-000000000001".to_string();
    let data = match preset {
        "vless-reality" => Data::Legacy(CanonicalNode::VlessReality {
            host,
            port,
            tls_server_name: sni,
            uuid,
            private_key: String::new(),
            public_key: String::new(),
            short_id: String::new(),
            decoy_sni: config
                .reality_decoy_sni
                .clone()
                .unwrap_or_else(|| "www.cloudflare.com".into()),
        }),
        "vmess-websocket" => Data::Legacy(CanonicalNode::VmessWebsocket {
            host,
            port,
            tls_server_name: sni,
            uuid,
            path: format!("/{id}"),
        }),
        "hysteria2" => Data::Legacy(CanonicalNode::Hysteria2 {
            host,
            port,
            tls_server_name: sni,
            password: String::new(),
        }),
        "tuic" => Data::Legacy(CanonicalNode::Tuic {
            host,
            port,
            tls_server_name: sni,
            uuid,
            password: String::new(),
        }),
        "anytls" => Data::Legacy(CanonicalNode::Anytls {
            host,
            port,
            tls_server_name: sni,
            password: String::new(),
        }),
        other => {
            let preset: presets::Preset = serde_json::from_value(serde_json::json!(other))
                .map_err(|_| Error("未知协议组合预设".into()))?;
            Data::Combination(presets::Node {
                name: id.clone(),
                preset,
                server: host,
                port,
                server_name: if preset.is_reality() {
                    config
                        .reality_decoy_sni
                        .clone()
                        .unwrap_or_else(|| "www.cloudflare.com".into())
                } else {
                    sni
                },
                credential: uuid,
                path: (!preset.is_trojan()).then(|| format!("/{id}")),
                reality: preset.is_reality().then(|| presets::Reality {
                    private_key: String::new(),
                    public_key: String::new(),
                    short_id: String::new(),
                    target: format!(
                        "{}:443",
                        config
                            .reality_decoy_sni
                            .as_deref()
                            .unwrap_or("www.cloudflare.com")
                    ),
                }),
            })
        }
    };
    let mut node = Node {
        id: id.clone(),
        tag: format!("node-{id}"),
        name: name.into(),
        core: core.unwrap_or_default(),
        enabled: true,
        listen: "::".into(),
        data,
    };
    node.renew_secrets()?;
    if core.is_none() {
        node.core = registry.choose_core(&node)?;
    }
    node.validate_data()?;
    Ok(node)
}
