//! Additional, already activated listeners join the existing artifact set.
//! The private source is read only while regenerating, never during HTTP GET.
use super::SubscriptionError;
use crate::config::DeploymentConfig;
use crate::core::presets::{Node, validate_nodes};
use base64::Engine;
use serde_json::{Value, json};
use std::path::Path;

pub const SOURCE: &str = "etc/sbctl/protocol-presets.json";
pub const INDEX: &str = "subscription-preset-nodes.json";

fn invalid(message: &str) -> SubscriptionError {
    SubscriptionError::Override(message.into())
}

fn extend_groups(value: &mut Value, groups: &str, members: &str, old: &[String], extra: &[String]) {
    if let Some(groups) = value.get_mut(groups).and_then(Value::as_array_mut) {
        for group in groups {
            if let Some(items) = group.get_mut(members).and_then(Value::as_array_mut)
                && items.iter().any(|item| {
                    item.as_str()
                        .is_some_and(|tag| old.iter().any(|s| s == tag))
                })
            {
                for name in extra {
                    if !items.iter().any(|item| item.as_str() == Some(name)) {
                        items.push(json!(name));
                    }
                }
            }
        }
    }
}

pub fn inject(
    root: &Path,
    config: &DeploymentConfig,
    artifacts: &mut Vec<(String, String)>,
) -> Result<(), SubscriptionError> {
    let path = root.join(SOURCE);
    let bytes = match crate::config::read_managed_file_no_follow(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(SubscriptionError::Artifact(error)),
    };
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("protocol preset source exceeds 1 MiB"));
    }
    let nodes: Vec<Node> =
        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid protocol preset source"))?;
    validate_nodes(&nodes).map_err(|e| invalid(&e.to_string()))?;
    let canonical = crate::canonical::nodes(config);
    let old: Vec<String> = canonical.iter().map(|n| n.tag().into()).collect();
    for node in &nodes {
        if canonical
            .iter()
            .any(|n| n.port() == node.port || n.tag() == node.name)
            || config.http_port == Some(node.port)
            || config.subscription_listen_port == Some(node.port)
            || matches!(
                node.name.as_str(),
                "代理" | "自动选择" | "DIRECT" | "direct" | "REJECT"
            )
        {
            return Err(invalid(
                "additional node conflicts with a managed listener or group",
            ));
        }
    }
    let compatible: Vec<&Node> = nodes.iter().filter(|n| !n.preset.is_xhttp()).collect();
    for (name, text) in artifacts.iter_mut() {
        if name.starts_with("subscription-sing-box") && name.ends_with(".json") {
            let mut value: Value = serde_json::from_str(text)
                .map_err(|_| invalid("invalid generated sing-box document"))?;
            let extra = compatible
                .iter()
                .map(|n| n.name.clone())
                .collect::<Vec<_>>();
            let array = value["outbounds"]
                .as_array_mut()
                .ok_or_else(|| invalid("missing sing-box outbounds"))?;
            for n in &compatible {
                array.push(n.sing_box_outbound().map_err(|e| invalid(&e.to_string()))?);
            }
            extend_groups(&mut value, "outbounds", "outbounds", &old, &extra);
            *text = serde_json::to_string_pretty(&value)
                .map_err(|_| invalid("sing-box serialization failed"))?;
        } else if name.starts_with("subscription-clash") && name.ends_with(".yaml") {
            let legacy = name.contains("1.18");
            let selected: Vec<&Node> = nodes
                .iter()
                .filter(|n| !n.preset.is_reality() && (!legacy || !n.preset.is_xhttp()))
                .collect();
            let mut value: Value = serde_yaml::from_str(text)
                .map_err(|_| invalid("invalid generated Mihomo document"))?;
            let array = value["proxies"]
                .as_array_mut()
                .ok_or_else(|| invalid("missing Mihomo proxies"))?;
            for n in &selected {
                array.push(
                    n.mihomo_proxy(!legacy)
                        .map_err(|e| invalid(&e.to_string()))?,
                );
            }
            let extra = selected.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
            extend_groups(&mut value, "proxy-groups", "proxies", &old, &extra);
            *text = serde_yaml::to_string(&value)
                .map_err(|_| invalid("Mihomo serialization failed"))?;
        } else if name == "subscription-uri.txt" {
            for node in &nodes {
                text.push_str(&node.share_uri().map_err(|e| invalid(&e.to_string()))?);
                text.push('\n');
            }
        } else if name == "subscription-shadowrocket.txt" {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(|_| invalid("invalid Shadowrocket artifact"))?;
            let mut decoded =
                String::from_utf8(decoded).map_err(|_| invalid("invalid Shadowrocket artifact"))?;
            for node in &compatible {
                decoded.push_str(&node.share_uri().map_err(|e| invalid(&e.to_string()))?);
                decoded.push('\n');
            }
            *text = super::base64_uri(&decoded);
        }
    }
    let uri = artifacts
        .iter()
        .find(|(name, _)| name == "subscription-uri.txt")
        .map(|(_, text)| text.clone())
        .ok_or_else(|| invalid("missing URI artifact"))?;
    if let Some((_, text)) = artifacts
        .iter_mut()
        .find(|(name, _)| name == "subscription-base64-uri.txt")
    {
        *text = super::base64_uri(&uri);
    }
    let index = nodes
        .iter()
        .map(|n| {
            Ok(json!({"name":n.name,"uri":n.share_uri().map_err(|e|invalid(&e.to_string()))?}))
        })
        .collect::<Result<Vec<_>, SubscriptionError>>()?;
    artifacts.push((
        INDEX.into(),
        serde_json::to_string(&index).map_err(|_| invalid("node index serialization failed"))?,
    ));
    Ok(())
}
