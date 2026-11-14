//! A single registry renders both data planes and every client representation.
//! All compatibility filtering happens before groups, overrides and rule lists.
use super::{SubscriptionError, SubscriptionFormat as Format};
use crate::{
    canonical::CanonicalNode,
    config::DeploymentConfig,
    core::{CoreKind, xray::CertificatePaths},
    nodes::{Data, Node, Registry, Strategy},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const CATALOG: &str = "subscription-catalog.json";
pub fn format_url(
    config: &DeploymentConfig,
    credential: &str,
    format: Format,
) -> Result<String, SubscriptionError> {
    let base =
        super::route_url_with_credential(config, credential, super::SubscriptionRoute::Index)?;
    Ok(format!(
        "{}/{}",
        base.trim_end_matches("/index"),
        format.path_name()
    ))
}
pub fn group_url(
    config: &DeploymentConfig,
    credential: &str,
    id: &str,
    format: Format,
) -> Result<String, SubscriptionError> {
    if !crate::nodes::identifier(id) {
        return Err(err("分组 ID 无效"));
    }
    let base =
        super::route_url_with_credential(config, credential, super::SubscriptionRoute::Index)?;
    Ok(format!(
        "{}/groups/{id}/{}",
        base.trim_end_matches("/index"),
        format.path_name()
    ))
}
pub fn catalog(
    root: &Path,
    generation: &crate::deployment::Generation,
) -> Result<Catalog, SubscriptionError> {
    let text = crate::deployment::read_generation(root, generation, CATALOG)
        .map_err(|e| err(e.to_string()))?;
    serde_json::from_str(&text).map_err(|_| err("订阅目录损坏"))
}
pub fn index(
    store: &crate::config::DeploymentStore,
    config: &DeploymentConfig,
    credential: &str,
    generation: &crate::deployment::Generation,
) -> Result<String, SubscriptionError> {
    let catalog = catalog(store.root(), generation)?;
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    };
    let mut body = format!(
        "<!doctype html><html lang=zh-CN><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><meta name=referrer content=no-referrer><title>sbctl 订阅中心</title><style>body{{font:16px/1.6 system-ui;max-width:900px;margin:32px auto;padding:0 20px}}a{{overflow-wrap:anywhere}}details{{margin:16px 0}}img{{max-width:220px}}code{{overflow-wrap:anywhere}}</style><h1>订阅中心</h1><p>运行模式：{} · 客户端目标版本：{}</p>",
        catalog.mode.label(),
        esc(&catalog.client_version)
    );
    match crate::traffic::report(store, config) {
        Ok(r) => body.push_str(&format!(
            "<p>本周期 VPS 流量：{} GiB</p>",
            r.total() / (1024 * 1024 * 1024)
        )),
        Err(_) => body.push_str("<p>流量暂不可用</p>"),
    }
    body.push_str(
        "<p>先选择客户端格式，再选择全量或节点组。节点组用于内容筛选，不隔离设备权限。</p>",
    );
    for (id, label) in std::iter::once((None, "全部节点".to_string())).chain(
        catalog
            .groups
            .iter()
            .map(|g| (Some(g.id.as_str()), g.name.clone())),
    ) {
        body.push_str(&format!("<h2>{}</h2>", esc(&label)));
        for row in super::subscription_matrix() {
            let report = catalog
                .reports
                .iter()
                .find(|r| r.group.as_deref() == id && r.format == row.format.path_name());
            let Some(report) = report else {
                continue;
            };
            if report.included == 0 {
                body.push_str(&format!("<p>{}：无兼容节点</p>", esc(&row.label)));
                continue;
            }
            let url = if let Some(id) = id {
                group_url(config, credential, id, row.format)?
            } else {
                format_url(config, credential, row.format)?
            };
            let qr = if id.is_some() {
                let prefix = url.rsplit_once('/').unwrap().0;
                format!("{prefix}/qr/{}", row.format.path_name())
            } else {
                super::route_url_with_credential(
                    config,
                    credential,
                    super::SubscriptionRoute::Qr(row.format),
                )?
            };
            let advanced = matches!(
                row.format,
                Format::SingBox | Format::ClashLegacy(_) | Format::SingBoxVersion(_)
            );
            body.push_str(&format!("<details{}><summary>{} · {} 个节点 / 排除 {} 个</summary><p>{}</p><a href=\"{}\">订阅链接</a><p><code>{}</code></p><a href=\"{}\">二维码</a>",if advanced{""}else{" open"},esc(&row.label),report.included,report.excluded.len(),esc(&row.note),esc(&url),esc(&url),esc(&qr)));
            for (node, reason) in &report.excluded {
                body.push_str(&format!("<p>{}：{}</p>", esc(node), esc(reason)));
            }
            body.push_str("</details>");
        }
    }
    body.push_str("<details><summary>节点分享链接（含节点凭据）</summary>");
    for n in &catalog.nodes {
        if n["enabled"] == true {
            body.push_str(&format!(
                "<p>{} <code>{}</code></p>",
                esc(n["name"].as_str().unwrap_or("")),
                esc(n["link"].as_str().unwrap_or(""))
            ));
        }
    }
    body.push_str("</details></html>");
    Ok(body)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Compatibility {
    pub format: String,
    pub group: Option<String>,
    pub included: usize,
    pub excluded: Vec<(String, String)>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub revision: u64,
    pub mode: crate::nodes::Mode,
    pub client_version: String,
    pub groups: Vec<crate::nodes::Group>,
    pub reports: Vec<Compatibility>,
    /// Share links contain proxy credentials, never server private keys.
    pub nodes: Vec<Value>,
}
fn err(s: impl Into<String>) -> SubscriptionError {
    SubscriptionError::Override(s.into())
}
fn parse(text: &str, yaml: bool) -> Result<Value, SubscriptionError> {
    if yaml {
        serde_yaml::from_str(text).map_err(|_| err("生成的 YAML 无效"))
    } else {
        serde_json::from_str(text).map_err(|_| err("生成的 JSON 无效"))
    }
}
fn serialize(value: &Value, yaml: bool) -> Result<String, SubscriptionError> {
    if yaml {
        serde_yaml::to_string(value).map_err(|_| err("YAML 序列化失败"))
    } else {
        serde_json::to_string_pretty(value).map_err(|_| err("JSON 序列化失败"))
    }
}
fn seed(config: &DeploymentConfig) -> CanonicalNode {
    crate::canonical::nodes(config)
        .into_iter()
        .next()
        .unwrap_or(CanonicalNode::VmessWebsocket {
            host: "example.com".into(),
            port: 23456,
            tls_server_name: "example.com".into(),
            uuid: "00000000-0000-4000-8000-000000000001".into(),
            path: "/".into(),
        })
}
fn profile(
    registry: &Registry,
    format: Format,
) -> Result<super::SingBoxVersionProfile, SubscriptionError> {
    let version = if let Format::SingBoxVersion(v) = format {
        v.to_string()
    } else {
        registry.client_version.clone()
    };
    super::all_sing_box_version_profiles()
        .into_iter()
        .find(|p| p.version.to_string() == version)
        .ok_or_else(|| err("未知客户端版本档"))
}
pub fn exclusion(node: &Node, format: Format, registry: &Registry) -> Option<String> {
    if !node.enabled {
        return Some("节点已停用".into());
    }
    let sb = matches!(
        format,
        Format::SingBox | Format::SingBoxFull | Format::SingBoxVersion(_)
    );
    let clash = matches!(format, Format::Clash | Format::ClashLegacy(_));
    match &node.data {
        Data::Combination(n)
            if n.preset.is_xhttp()
                && (sb || matches!(format, Format::Shadowrocket | Format::ClashLegacy(_))) =>
        {
            Some("目标客户端没有经过验证的 XHTTP 支持".into())
        }
        Data::Combination(n) if n.preset.is_reality() && clash => {
            Some("当前 Xray Reality 与该 Mihomo 档互通未验证".into())
        }
        Data::Legacy(CanonicalNode::VlessReality { .. })
            if node.core == CoreKind::Xray && (clash || sb || format == Format::Shadowrocket) =>
        {
            Some("固定 Xray Reality 基线仅通过 Xray 客户端互通；该客户端档保守排除".into())
        }
        Data::Legacy(CanonicalNode::Anytls { .. })
            if matches!(format, Format::ClashLegacy(_))
                || (matches!(format, Format::SingBoxFull | Format::SingBoxVersion(_))
                    && profile(registry, format).is_ok_and(|p| !p.supports_anytls)) =>
        {
            Some("客户端版本不支持 AnyTLS".into())
        }
        _ => None,
    }
}
pub fn share_link(
    config: &DeploymentConfig,
    node: &Node,
    shadow: bool,
) -> Result<String, SubscriptionError> {
    match &node.data {
        Data::Combination(n) => {
            let mut n = n.clone();
            n.name = node.tag.clone();
            let mut uri = n.share_uri().map_err(|_| err("组合分享链接生成失败"))?;
            if config.certificate_mode == crate::config::CertificateMode::SelfSigned
                && !n.preset.is_reality()
            {
                let (head, fragment) = uri.split_once('#').unwrap_or((&uri, ""));
                uri = format!("{head}&allowInsecure=1&insecure=1#{fragment}");
            }
            Ok(uri + "\n")
        }
        Data::Legacy(n) => {
            let text = if shadow {
                super::render::shadowrocket(config, std::slice::from_ref(n))?
            } else {
                super::render::uri(config, std::slice::from_ref(n))?
            };
            let decoded = if shadow {
                String::from_utf8(
                    base64::engine::general_purpose::STANDARD
                        .decode(text.trim())
                        .map_err(|_| err("分享链接解码失败"))?,
                )
                .map_err(|_| err("分享链接编码无效"))?
            } else {
                text
            };
            let xray_vmess =
                node.core == CoreKind::Xray && matches!(n, CanonicalNode::VmessWebsocket { .. });
            if node.tag == n.tag() && !xray_vmess {
                return Ok(decoded);
            }
            let mut lines = String::new();
            for line in decoded.lines() {
                if let Some(payload) = line.strip_prefix("vmess://") {
                    let mut v: Value = serde_json::from_slice(
                        &base64::engine::general_purpose::STANDARD
                            .decode(payload)
                            .map_err(|_| err("VMess 分享链接无效"))?,
                    )
                    .map_err(|_| err("VMess 分享链接无效"))?;
                    v["ps"] = json!(node.tag);
                    if xray_vmess {
                        v["scy"] = json!("aes-128-gcm");
                    }
                    lines.push_str("vmess://");
                    lines.push_str(
                        &base64::engine::general_purpose::STANDARD
                            .encode(serde_json::to_vec(&v).unwrap()),
                    );
                } else {
                    lines.push_str(line.split('#').next().unwrap_or(line));
                    lines.push('#');
                    lines.push_str(
                        &url::form_urlencoded::byte_serialize(node.tag.as_bytes())
                            .collect::<String>(),
                    );
                }
                lines.push('\n');
            }
            Ok(lines)
        }
    }
}
pub(crate) fn client_node(
    config: &DeploymentConfig,
    node: &Node,
    format: Format,
) -> Result<Value, SubscriptionError> {
    let mut node_config = config.clone();
    if !matches!(&node.data,Data::Legacy(CanonicalNode::Hysteria2{port,..}) if config.hysteria2.as_ref().is_some_and(|h|h.listen_port==*port))
    {
        node_config.hysteria2_port_hopping = None;
    }
    let config = &node_config;
    let clash = matches!(format, Format::Clash | Format::ClashLegacy(_));
    let mut value = match &node.data {
        Data::Combination(n) => if clash {
            n.mihomo_proxy(format == Format::Clash)
        } else {
            n.sing_box_outbound()
        }
        .map_err(|_| err("组合与目标客户端不兼容"))?,
        Data::Legacy(n) => {
            if clash {
                let text = if matches!(format, Format::ClashLegacy(_)) {
                    super::render::clash_legacy(config, std::slice::from_ref(n))?
                } else {
                    super::render::clash(config, std::slice::from_ref(n))?
                };
                parse(&text, true)?["proxies"][0].clone()
            } else {
                super::render::client_outbounds(config, std::slice::from_ref(n)).remove(0)
            }
        }
    };
    value[if clash { "name" } else { "tag" }] = json!(node.tag);
    // sing-box 1.14 optimizes VMess "auto" over TLS to its "zero" cipher;
    // Xray rejects that wire security type. Explicit AEAD interoperates.
    if node.core == CoreKind::Xray
        && matches!(
            &node.data,
            Data::Legacy(CanonicalNode::VmessWebsocket { .. })
        )
    {
        value[if clash { "cipher" } else { "security" }] = json!("aes-128-gcm");
    }
    if config.certificate_mode == crate::config::CertificateMode::SelfSigned
        && matches!(&node.data,Data::Combination(n) if !n.preset.is_reality())
    {
        if clash {
            value["skip-cert-verify"] = json!(true);
        } else {
            value["tls"]["insecure"] = json!(true);
        }
    }
    if let Format::SingBoxVersion(v) = format
        && v.minor < 11
        && value["type"] == "hysteria2"
    {
        let object = value.as_object_mut().unwrap();
        object.remove("server_ports");
        object.remove("hop_interval");
        value["server_port"] = json!(node.port());
    }
    Ok(value)
}
fn replace_members(value: &mut Value, old: &str, tags: &[String]) {
    match value {
        Value::Array(items) => {
            if items.iter().any(|v| v.as_str() == Some(old)) {
                let mut next = Vec::new();
                for item in items.drain(..) {
                    if item.as_str() == Some(old) {
                        next.extend(tags.iter().map(|s| json!(s)));
                    } else {
                        next.push(item);
                    }
                }
                *items = next;
            }
            for v in items {
                replace_members(v, old, tags);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                replace_members(v, old, tags);
            }
        }
        _ => {}
    }
}
fn external_template(root: &Path, yaml: bool) -> Result<Option<Value>, SubscriptionError> {
    let path = root.join(if yaml {
        "etc/sbctl/templates/client/mihomo.yaml"
    } else {
        "etc/sbctl/templates/client/sing-box.json"
    });
    let bytes = match crate::config::read_managed_file_no_follow(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if bytes.len() > 1024 * 1024 {
        return Err(err("客户端模板超过 1 MiB"));
    }
    let text = String::from_utf8(bytes).map_err(|_| err("客户端模板必须为 UTF-8"))?;
    let v = parse(&text, yaml)?;
    if !v.is_object() {
        return Err(err("客户端模板必须为对象"));
    }
    // Node credentials and group membership always come from the registry.
    if ["proxies", "proxy-groups", "outbounds"]
        .iter()
        .any(|k| v.get(k).is_some())
    {
        return Err(err("基础模板不得包含节点或策略组；请使用节点/分组命令"));
    }
    Ok(Some(v))
}
fn render_format(
    config: &DeploymentConfig,
    registry: &Registry,
    format: Format,
    group: Option<&crate::nodes::Group>,
    templates: &(Option<Value>, Option<Value>),
) -> Result<(Option<String>, Compatibility), SubscriptionError> {
    let candidates: Vec<_> = registry
        .nodes
        .iter()
        .filter(|n| group.is_none_or(|g| g.members.contains(&n.id)))
        .collect();
    let mut selected = Vec::new();
    let mut excluded = Vec::new();
    for n in candidates {
        if let Some(reason) = exclusion(n, format, registry) {
            excluded.push((n.id.clone(), reason));
        } else {
            selected.push(n);
        }
    }
    if let Some(group) = group {
        selected.sort_by_key(|n| group.members.iter().position(|id| id == &n.id).unwrap());
    }
    let report = Compatibility {
        format: format.path_name(),
        group: group.map(|g| g.id.clone()),
        included: selected.len(),
        excluded,
    };
    if selected.is_empty() {
        return Ok((None, report));
    }
    if matches!(
        format,
        Format::Uri | Format::Base64Uri | Format::Shadowrocket
    ) {
        let mut uris = String::new();
        for node in selected {
            uris.push_str(&share_link(config, node, format == Format::Shadowrocket)?);
        }
        return Ok((
            Some(if format == Format::Uri {
                uris
            } else {
                base64::engine::general_purpose::STANDARD.encode(uris)
            }),
            report,
        ));
    }
    let yaml = matches!(format, Format::Clash | Format::ClashLegacy(_));
    let tags = selected.iter().map(|n| n.tag.clone()).collect::<Vec<_>>();
    let clients = selected
        .iter()
        .map(|n| client_node(config, n, format))
        .collect::<Result<Vec<_>, _>>()?;
    if format == Format::SingBox {
        return Ok((
            Some(serialize(&json!({"outbounds":clients}), false)?),
            report,
        ));
    }
    // A singleton drives the established template/version renderer. Replace its
    // complete node set before merging anything administrator-owned.
    let mut seed = seed(config);
    if matches!(seed, CanonicalNode::Anytls { .. }) {
        seed = CanonicalNode::VmessWebsocket {
            host: "example.com".into(),
            port: 23456,
            tls_server_name: "example.com".into(),
            uuid: "00000000-0000-4000-8000-000000000001".into(),
            path: "/".into(),
        };
    }
    let text = if yaml {
        if matches!(format, Format::ClashLegacy(_)) {
            super::render::clash_legacy(config, std::slice::from_ref(&seed))?
        } else {
            super::render::clash(config, std::slice::from_ref(&seed))?
        }
    } else {
        super::render::sing_box_full(
            config,
            std::slice::from_ref(&seed),
            &profile(registry, format)?,
        )?
    };
    let mut value = parse(&text, yaml)?;
    if yaml {
        value["proxies"] = json!(clients);
    } else {
        value["outbounds"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v["tag"] != seed.tag());
        value["outbounds"]
            .as_array_mut()
            .unwrap()
            .splice(0..0, clients);
    }
    replace_members(&mut value, seed.tag(), &tags);
    let groups: Vec<_> = registry
        .groups
        .iter()
        .filter(|g| group.is_none_or(|one| one.id == g.id))
        .collect();
    for g in groups {
        let members: Vec<_> = g
            .members
            .iter()
            .filter_map(|id| selected.iter().find(|n| &n.id == id).map(|n| n.tag.clone()))
            .collect();
        if members.is_empty() {
            continue;
        }
        let entry = if yaml {
            json!({"name":g.name,"type":if g.strategy==Strategy::Select{"select"}else{"url-test"},"proxies":members,"url":config.client_latency_probe_url,"interval":300,"lazy":false})
        } else {
            let mut v = json!({"tag":g.name,"type":if g.strategy==Strategy::Select{"selector"}else{"urltest"},"outbounds":members});
            if g.strategy == Strategy::Urltest {
                v["url"] = json!(config.client_latency_probe_url);
                v["interval"] = json!("5m");
            }
            v
        };
        let key = if yaml { "proxy-groups" } else { "outbounds" };
        let items = value[key].as_array_mut().unwrap();
        if let Some(main) = items
            .iter_mut()
            .find(|v| v[if yaml { "name" } else { "tag" }] == "代理")
        {
            main[if yaml { "proxies" } else { "outbounds" }]
                .as_array_mut()
                .unwrap()
                .push(json!(g.name));
        }
        items.push(entry);
    }
    if let Some(template) = if yaml { &templates.1 } else { &templates.0 } {
        json_merge::deep_merge(&mut value, template);
    }
    Ok((Some(serialize(&value, yaml)?), report))
}
pub fn generate(
    config: &DeploymentConfig,
    root: &Path,
    registry: &Registry,
) -> Result<Vec<(String, String)>, SubscriptionError> {
    registry.validate(config).map_err(|e| err(e.to_string()))?;
    let templates = (
        external_template(root, false)?,
        external_template(root, true)?,
    );
    let mut artifacts = Vec::new();
    let mut reports = Vec::new();
    for group in std::iter::once(None).chain(registry.groups.iter().map(Some)) {
        let mut batch = Vec::new();
        for row in super::subscription_matrix() {
            let (text, report) = render_format(config, registry, row.format, group, &templates)?;
            reports.push(report);
            if let Some(text) = text {
                batch.push((row.format.artifact_name().into_owned(), text));
            }
        }
        super::artifacts::apply_client_overrides(root, &mut batch)?;
        super::artifacts::apply_rule_lists(root, &mut batch, registry.client_version == "1.10")?;
        super::artifacts::validate_client_references(&batch)?;
        for (name, text) in &batch {
            validate_shape(name, text, registry)?;
        }
        for (name, text) in batch {
            artifacts.push((
                group
                    .map(|g| format!("group-{}--{name}", g.id))
                    .unwrap_or(name),
                text,
            ));
        }
    }
    let catalog=Catalog{revision:registry.revision,mode:registry.mode,client_version:registry.client_version.clone(),groups:registry.groups.clone(),reports,
        nodes:registry.nodes.iter().map(|n|Ok(json!({"id":n.id,"name":n.name,"tag":n.tag,"core":n.core,"enabled":n.enabled,"port":n.port(),"preset":n.preset(),"link":if n.enabled{share_link(config,n,false)?}else{String::new()}}))).collect::<Result<Vec<_>,SubscriptionError>>()?};
    artifacts.push((
        CATALOG.into(),
        serde_json::to_string_pretty(&catalog).unwrap(),
    ));
    for (core, doc) in servers(config, root, registry)? {
        artifacts.push((server_name(core).into(), serialize(&doc, false)?));
    }
    Ok(artifacts)
}
pub fn server_name(core: CoreKind) -> &'static str {
    match core {
        CoreKind::SingBox => "sing-box-server.json",
        CoreKind::Xray => "xray-server.json",
    }
}

fn validate_shape(name: &str, text: &str, registry: &Registry) -> Result<(), SubscriptionError> {
    let singbox = name.starts_with("subscription-sing-box");
    let mihomo = name.starts_with("subscription-clash");
    if !singbox && !mihomo {
        return Ok(());
    }
    let value = parse(text, mihomo)?;
    let invalid = || err(format!("{name} 的客户端结构或版本字段不兼容"));
    if !value.is_object() {
        return Err(invalid());
    }
    if singbox {
        let outbounds = value["outbounds"].as_array().ok_or_else(invalid)?;
        if outbounds
            .iter()
            .any(|v| !v["type"].is_string() || !v["tag"].is_string())
        {
            return Err(invalid());
        }
        for key in ["dns", "route", "experimental", "log", "ntp"] {
            if value.get(key).is_some_and(|v| !v.is_object()) {
                return Err(invalid());
            }
        }
        for (section, field) in [
            ("dns", "servers"),
            ("dns", "rules"),
            ("route", "rules"),
            ("route", "rule_set"),
        ] {
            if value[section].get(field).is_some_and(|v| {
                !v.is_array()
                    || v.as_array()
                        .is_some_and(|a| a.iter().any(|v| !v.is_object()))
            }) {
                return Err(invalid());
            }
        }
        if value.get("inbounds").is_some_and(|v| {
            !v.is_array()
                || v.as_array()
                    .is_some_and(|a| a.iter().any(|v| !v["type"].is_string()))
        }) {
            return Err(invalid());
        }
        if let Some(servers) = value["dns"]["servers"].as_array() {
            let version = if name == "subscription-sing-box-full.json" {
                registry.client_version.as_str()
            } else {
                name.trim_start_matches("subscription-sing-box-")
                    .trim_end_matches(".json")
            };
            let typed = super::all_sing_box_version_profiles()
                .into_iter()
                .find(|p| p.version.to_string() == version)
                .map(|p| p.typed_dns);
            for server in servers {
                if typed.is_some_and(|typed| {
                    if typed {
                        !server["type"].is_string()
                    } else {
                        !server["address"].is_string()
                    }
                }) {
                    return Err(invalid());
                }
            }
        }
    } else {
        if !value["proxies"].is_array() || !value["proxy-groups"].is_array() {
            return Err(invalid());
        }
        for key in ["dns", "tun", "sniffer", "profile", "rule-providers"] {
            if value.get(key).is_some_and(|v| !v.is_object()) {
                return Err(invalid());
            }
        }
        if value.get("rules").is_some_and(|v| {
            !v.is_array()
                || v.as_array()
                    .is_some_and(|a| a.iter().any(|v| !v.is_string()))
        }) {
            return Err(invalid());
        }
        for proxy in value["proxies"].as_array().unwrap() {
            if !proxy["name"].is_string() || !proxy["type"].is_string() {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
pub fn servers(
    config: &DeploymentConfig,
    root: &Path,
    registry: &Registry,
) -> Result<Vec<(CoreKind, Value)>, SubscriptionError> {
    if !registry.mode.allows(CoreKind::SingBox)
        && let Some(document) = crate::override_template::Overrides::load(root)
            .map_err(|e| err(e.to_string()))?
            .sing_box_server
    {
        let mut contents = json!({});
        document.apply_to(
            &mut contents,
            &crate::override_template::server_merge_policy(),
        );
        if contents != json!({}) {
            return Err(err(
                "Xray 单内核模式无法应用 sing-box 服务端覆写；请先迁移或清除覆写",
            ));
        }
    }
    let tls = super::render::existing_certificate_tls_config(config, root)?;
    let cert = CertificatePaths {
        certificate: tls["certificate_path"].as_str().unwrap(),
        private_key: tls["key_path"].as_str().unwrap(),
    };
    let legacy_server =
        super::installed_kernel_version(&root.join(crate::deployment::binary(CoreKind::SingBox)))
            .is_some_and(|v| v.major == 1 && v.minor == 10);
    let mut result = Vec::new();
    for core in [CoreKind::SingBox, CoreKind::Xray] {
        if !registry.mode.allows(core) {
            continue;
        }
        let mut server = if core == CoreKind::SingBox {
            parse(
                &super::render::sing_box_server(config, &[], root, legacy_server)?,
                false,
            )?
        } else {
            json!({"log":{"loglevel":"warning"},"inbounds":[],"outbounds":[{"tag":"direct","protocol":"freedom","settings":{"domainStrategy":if config.ipv4_only{"ForceIPv4"}else{"AsIs"}}}]})
        };
        let mut inbounds = Vec::new();
        for n in registry
            .nodes
            .iter()
            .filter(|n| n.enabled && n.core == core)
        {
            let mut inbound = match (&n.data, core) {
                (Data::Legacy(node), CoreKind::SingBox) => parse(
                    &super::render::sing_box_server(
                        config,
                        std::slice::from_ref(node),
                        root,
                        legacy_server,
                    )?,
                    false,
                )?["inbounds"][0]
                    .clone(),
                (Data::Legacy(node), CoreKind::Xray) => crate::core::xray::render_server(
                    std::slice::from_ref(node),
                    Some(CertificatePaths {
                        certificate: cert.certificate,
                        private_key: cert.private_key,
                    }),
                    config.ipv4_only,
                )
                .map_err(|_| err("Xray 不支持该节点"))?["inbounds"][0]
                    .clone(),
                (Data::Combination(node), CoreKind::Xray) => node
                    .xray_inbound(Some(&cert))
                    .map_err(|_| err("Xray 组合配置无效"))?,
                (Data::Combination(node), CoreKind::SingBox) => {
                    let mut v = json!({"type":if node.preset.is_trojan(){"trojan"}else{"vless"},"listen_port":node.port,"users":[{}],"tls":tls});
                    v["users"][0][if node.preset.is_trojan() {
                        "password"
                    } else {
                        "uuid"
                    }] = json!(node.credential);
                    v["tls"]["server_name"] = json!(node.server_name);
                    if !node.preset.is_trojan() {
                        v["transport"] = json!({"type":"ws","path":node.path});
                    }
                    v
                }
            };
            inbound["tag"] = json!(n.tag);
            inbound["listen"] = json!(n.listen);
            inbounds.push(inbound);
        }
        server["inbounds"] = json!(inbounds);
        let managed_inbounds = server["inbounds"].clone();
        if core == CoreKind::SingBox {
            if (config.ipv4_only || !crate::system::host_has_ipv6_route()) && !legacy_server {
                server["route"] = json!({"rules":server["inbounds"].as_array().unwrap().iter().map(|n|json!({"inbound":n["tag"],"action":"resolve","strategy":"ipv4_only"})).collect::<Vec<_>>()});
            }
            server = parse(
                &super::artifacts::apply_server_override(root, &serialize(&server, false)?)?,
                false,
            )?;
            // Advanced listeners may extend the service, but cannot silently
            // change a managed node while clients still use the registry.
            for original in managed_inbounds.as_array().unwrap() {
                let merged = server["inbounds"]
                    .as_array()
                    .and_then(|items| items.iter().find(|n| n["tag"] == original["tag"]))
                    .ok_or_else(|| err("服务端覆写移除了托管节点；请使用 node 命令"))?;
                for field in ["type", "listen", "listen_port", "users", "tls", "transport"] {
                    if merged.get(field) != original.get(field) {
                        return Err(err(format!(
                            "服务端覆写修改了托管节点的 {field}；请使用 node 命令，独立监听器使用 custom-* tag"
                        )));
                    }
                }
            }
        }
        result.push((core, server));
    }
    Ok(result)
}
