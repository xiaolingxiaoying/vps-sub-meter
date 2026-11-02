use super::{
    AI_TAG, AUTO_TAG, FALLBACK_TAG, PROXY_TAG, STREAM_TAG, TELEGRAM_TAG, client_skip_cert_verify,
};
use crate::canonical::CanonicalNode;
use crate::config::{ClientRuleProfile, DeploymentConfig};
use crate::subscription::artifacts::SubscriptionError;
use crate::subscription::template::{GroupMember, GroupTag};
use crate::subscription::{GroupRole, GroupSpec, RuleMatcher, TemplateSpec};

/// The supplied template group names; GLOBAL belongs to the Mihomo core.
const CLASH_SELECTOR_TAG: &str = "代理";
const CLASH_DIRECT_TAG: &str = "DIRECT";
/// The core's own direct policy, which is what a `GEOIP,LAN` / `GEOIP,CN`
/// verdict has always carried here: the pseudo-proxy, not the `全球直连`
/// group that wraps it for the user.
const CLASH_BUILTIN_DIRECT: &str = "DIRECT";

/// Maps a template group identity to the clash artifact's established tag.
fn clash_tag(tag: GroupTag) -> &'static str {
    match tag {
        GroupTag::Selector => CLASH_SELECTOR_TAG,
        GroupTag::Auto => AUTO_TAG,
        GroupTag::Direct => CLASH_DIRECT_TAG,
        GroupTag::Fallback => FALLBACK_TAG,
        GroupTag::Proxy => PROXY_TAG,
        GroupTag::Ai => AI_TAG,
        GroupTag::Stream => STREAM_TAG,
        GroupTag::Telegram => TELEGRAM_TAG,
        // mihomo's built-in block policy: a rule target, never a proxy group, so
        // no template declares a `GroupSpec` for it.
        GroupTag::Reject => "REJECT",
    }
}

/// The `proxies:` block plus every group the template declares.
fn clash_proxies(
    config: &DeploymentConfig,
    nodes: &[CanonicalNode],
    spec: &TemplateSpec,
) -> Result<String, SubscriptionError> {
    let skip = client_skip_cert_verify(config);
    let probe_url =
        serde_json::to_string(&config.client_latency_probe_url).expect("URL serializes");
    // Serialize dynamic strings instead of interpolating YAML scalars: a
    // perfectly valid password such as `null`, `1234` or `a: b` must remain
    // exactly that string, including whitespace and line breaks.
    let mut entries = Vec::with_capacity(nodes.len());
    for node in nodes {
        let entry = match &node {
            CanonicalNode::VlessReality {
                host,
                port,
                uuid,
                public_key,
                short_id,
                decoy_sni,
                ..
            } => serde_json::json!({
                "name": node.tag(), "type": "vless", "server": host,
                "port": port, "uuid": uuid, "network": "tcp", "udp": true,
                "flow": "xtls-rprx-vision", "tls": true, "servername": decoy_sni,
                "client-fingerprint": "chrome",
                "reality-opts": {"public-key": public_key, "short-id": short_id},
            }),
            CanonicalNode::VmessWebsocket {
                host,
                port,
                tls_server_name,
                uuid,
                path,
            } => serde_json::json!({
                "name": node.tag(), "type": "vmess", "server": host,
                "port": port, "uuid": uuid, "alterId": 0, "cipher": "auto",
                "tls": true, "servername": tls_server_name, "skip-cert-verify": skip,
                "network": "ws", "ws-opts": {"path": path, "headers": {"Host": tls_server_name}},
            }),
            CanonicalNode::Hysteria2 {
                host,
                port,
                tls_server_name,
                password,
            } => serde_json::json!({
                "name": node.tag(), "type": "hysteria2", "server": host,
                "port": port, "password": password, "sni": tls_server_name,
                "skip-cert-verify": skip, "up": "50 Mbps", "down": "200 Mbps",
            }),
            CanonicalNode::Tuic {
                host,
                port,
                tls_server_name,
                uuid,
                password,
            } => serde_json::json!({
                "name": node.tag(), "type": "tuic", "server": host,
                "port": port, "uuid": uuid, "password": password,
                "sni": tls_server_name, "alpn": ["h3"], "skip-cert-verify": skip,
            }),
            CanonicalNode::Anytls {
                host,
                port,
                tls_server_name,
                password,
            } => serde_json::json!({
                "name": node.tag(), "type": "anytls", "server": host,
                "port": port, "password": password, "client-fingerprint": "chrome",
                "udp": true, "idle-session-check-interval": 30, "idle-session-timeout": 30,
                "tls": true, "sni": tls_server_name, "skip-cert-verify": skip,
            }),
        };
        entries.push(entry);
    }
    let mut proxies = serde_yaml::to_string(&serde_json::json!({"proxies": entries}))
        .expect("proxy values serialize");
    proxies.push_str("mode: rule\nproxy-groups:\n");
    for group in spec
        .groups
        .iter()
        .filter(|group| matches!(group.tag, GroupTag::Selector | GroupTag::Auto))
    {
        let tag = clash_tag(group.tag);
        match group.role {
            GroupRole::Selector | GroupRole::Direct => {
                proxies.push_str(&format!(
                    "  - name: {tag}\n    type: select\n    url: {probe_url}\n    interval: 300\n    proxies:\n"
                ));
            }
            GroupRole::UrlTest => {
                proxies.push_str(&format!(
                    "  - name: {tag}\n    type: url-test\n    url: {probe_url}\n    interval: 300\n    tolerance: 50\n    proxies:\n"
                ));
            }
            GroupRole::Fallback => {
                proxies.push_str(&format!(
                    "  - name: {tag}\n    type: fallback\n    url: {probe_url}\n    interval: 300\n    proxies:\n"
                ));
            }
        }
        for member in clash_members(group, nodes) {
            proxies.push_str(&format!("      - {member}\n"));
        }
    }
    Ok(proxies)
}

/// A group's members as clash proxy names, in declaration order.
fn clash_members(group: &GroupSpec, nodes: &[CanonicalNode]) -> Vec<String> {
    let mut members = Vec::new();
    for member in &group.members {
        match member {
            GroupMember::AllNodes => members.extend(nodes.iter().map(|node| node.tag().to_owned())),
            GroupMember::Group(tag) => members.push(clash_tag(*tag).to_owned()),
            GroupMember::BuiltinDirect => members.push(CLASH_BUILTIN_DIRECT.to_owned()),
        }
    }
    members
}

/// The `rules:` block, shared by the current and the legacy artifact. The two
/// differ only in whether external rule-sets exist at all: mihomo 1.18 has no
/// `.mrs` rule-providers to reference, which is precisely the `minimal` case, so
/// the legacy artifact renders the twins and nothing else.
fn clash_rules(_spec: &TemplateSpec, remote: bool, config: &DeploymentConfig) -> String {
    let mut rules = String::from("rules:\n");
    if remote {
        rules.push_str(&clash_extra_rules(config));
    }
    for rule in super::baseline::MIHOMO["rules"].as_array().unwrap() {
        let rule = rule.as_str().unwrap();
        if !remote && rule.starts_with("RULE-SET,") {
            let tag = rule.split(',').nth(1).unwrap();
            match tag {
                "review-private-domain" => {
                    push_inline_domains(&mut rules, RuleMatcher::PRIVATE_DOMAINS, "DIRECT")
                }
                "review-cn-domain" => {
                    push_inline_domains(&mut rules, RuleMatcher::CN_DOMAINS, "DIRECT")
                }
                "review-private-ip" => {
                    push_inline_cidrs(&mut rules, RuleMatcher::PRIVATE_ADDRESS_LIST, "DIRECT")
                }
                "review-cn-ip" => {
                    push_inline_cidrs(&mut rules, RuleMatcher::CN_ADDRESSES, "DIRECT")
                }
                _ => unreachable!("bundled provider has an inline twin"),
            }
        } else {
            rules.push_str(&format!("  - {rule}\n"));
        }
    }
    rules
}

/// One compiled-in domain suffix list, in the same shape the inline rules
/// already take.
fn push_inline_domains(rules: &mut String, suffixes: &[&str], target: &str) {
    for suffix in suffixes {
        rules.push_str(&format!("  - DOMAIN-SUFFIX,{suffix},{target}\n"));
    }
}

/// One compiled-in CIDR list. IPv6 needs its own code word: mihomo parses
/// `IP-CIDR,::1/128` but Shadowrocket does not, and a rule that never matches
/// is worse than one that is missing.
fn push_inline_cidrs(rules: &mut String, cidrs: &[&str], target: &str) {
    for cidr in cidrs {
        let code = if cidr.contains(':') {
            "IP-CIDR6"
        } else {
            "IP-CIDR"
        };
        rules.push_str(&format!("  - {code},{cidr},{target},no-resolve\n"));
    }
}

/// The `rule-providers:` block for the rule-sets this template references.
fn clash_rule_providers(_spec: &TemplateSpec, config: &DeploymentConfig) -> String {
    let mut values = super::baseline::MIHOMO["rule-providers"].clone();
    let base = config.client_rule_set_base_url.trim_end_matches('/');
    for provider in values.as_object_mut().unwrap().values_mut() {
        let url = provider["url"].as_str().unwrap();
        let suffix = url.split_once("@meta/").unwrap().1;
        provider["url"] = serde_json::json!(format!("{base}@meta/{suffix}"));
        provider["interval"] = serde_json::json!(clash_interval(config));
    }
    for set in config
        .client_extra_rule_sets
        .iter()
        .filter(|set| set.is_mihomo())
    {
        let behavior = match set.behavior {
            crate::config::RuleSetBehavior::Domain => "domain",
            crate::config::RuleSetBehavior::Ipcidr => "ipcidr",
        };
        values[&set.name] = serde_json::json!({
            "type": "http", "behavior": behavior, "format": "mrs",
            "url": set.url, "path": format!("./ruleset/{}.mrs", set.name),
            "interval": clash_interval(config),
        });
    }
    serde_yaml::to_string(&serde_json::json!({"rule-providers": values}))
        .expect("rule provider values serialize")
}

/// The refresh interval in the seconds shape mihomo wants.
fn clash_interval(config: &DeploymentConfig) -> u64 {
    extra_rule_set_seconds(&config.client_rule_set_update_interval).unwrap_or(86_400)
}

/// Parses the Go-ish duration shape sbctl stores (`30m`, `12h`, `1d`, `3600s`).
pub(crate) fn extra_rule_set_seconds(text: &str) -> Option<u64> {
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let unit = text.chars().last()?;
    let digits = text.strip_suffix(unit)?;
    let amount = digits.parse::<u64>().ok()?;
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => return None,
    };
    let seconds = amount.checked_mul(multiplier)?;
    // sing-box parses a signed 64-bit count of nanoseconds (Go time.Duration).
    (seconds > 0 && seconds <= i64::MAX as u64 / 1_000_000_000).then_some(seconds)
}

/// The clash target for a registered rule-set's verdict.
fn clash_extra_rule_target(outbound: &str) -> &'static str {
    match outbound {
        "direct" => CLASH_BUILTIN_DIRECT,
        "reject" => "REJECT",
        _ => CLASH_SELECTOR_TAG,
    }
}

/// The `RULE-SET` lines for the registered sets, ahead of the template's own
/// rules so an explicit registration beats a generic geo verdict.
fn clash_extra_rules(config: &DeploymentConfig) -> String {
    let mut lines = String::new();
    for set in config
        .client_extra_rule_sets
        .iter()
        .filter(|set| set.is_mihomo())
    {
        let suffix = if set.behavior == crate::config::RuleSetBehavior::Ipcidr {
            ",no-resolve"
        } else {
            ""
        };
        lines.push_str(&format!(
            "  - RULE-SET,{name},{target}{suffix}\n",
            name = set.name,
            target = clash_extra_rule_target(&set.outbound)
        ));
    }
    lines
}

pub(crate) fn clash(
    config: &DeploymentConfig,
    nodes: &[CanonicalNode],
) -> Result<String, SubscriptionError> {
    let template = if config.client_template == crate::subscription::ClientTemplate::Compact {
        crate::subscription::ClientTemplate::Compact
    } else {
        crate::subscription::ClientTemplate::Standard
    };
    let spec = TemplateSpec::for_template(config, template);
    let remote_rule_sets = config.client_rule_profile == ClientRuleProfile::Standard;
    let mut output = clash_proxies(config, nodes, &spec)?;
    // The AI suffix rules must precede the CN rule-set so OpenAI/X domains
    // never fall into geosite-cn's direct verdict; the template's own rule order
    // already encodes that, and the renderer keeps it.
    output.push_str(&clash_rules(&spec, remote_rule_sets, config));
    if remote_rule_sets {
        output.push_str(&clash_rule_providers(&spec, config));
    }
    Ok(super::baseline::clash_settings(
        &output,
        config,
        remote_rule_sets,
        spec.dns.direct_rule_set.is_some(),
    ))
}

/// The mihomo 1.18.x compatibility artifact: same node and group layout as the
/// current artifact, but rendered without external rule-sets, because that line
/// shipped with the built-in `GEOIP` rules everywhere and must not be asked to
/// fetch an `.mrs` it cannot parse. The `minimal` twin path is therefore the
/// only routing content it carries — which is also why the two renderers share
/// [`clash_rules`] instead of keeping a second copy of the rule vocabulary.
pub(crate) fn clash_legacy(
    config: &DeploymentConfig,
    nodes: &[CanonicalNode],
) -> Result<String, SubscriptionError> {
    // mihomo 1.18 rejects the whole profile on an unknown proxy type. Filter
    // before expanding groups so they cannot retain dangling AnyTLS names.
    let compatible_nodes: Vec<_> = nodes
        .iter()
        .filter(|node| !matches!(node, CanonicalNode::Anytls { .. }))
        .cloned()
        .collect();
    let nodes = compatible_nodes.as_slice();
    let template = if config.client_template == crate::subscription::ClientTemplate::Compact {
        crate::subscription::ClientTemplate::Compact
    } else {
        crate::subscription::ClientTemplate::Standard
    };
    let spec = TemplateSpec::for_template(config, template);
    let mut output = clash_proxies(config, nodes, &spec)?;
    output.push_str(&clash_rules(&spec, false, config));
    Ok(super::baseline::clash_settings(
        &output,
        config,
        false,
        spec.dns.direct_rule_set.is_some(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{clash, clash_legacy, extra_rule_set_seconds};
    use crate::config::{DeploymentConfig, ExtraRuleSet, ManagedProtocol, SubscriptionMode};

    #[test]
    fn node_yaml_preserves_credentials_and_paths_as_strings() {
        let mut config = DeploymentConfig::new(
            SubscriptionMode::IpFallback,
            "203.0.113.7".into(),
            None,
            Some(2080),
            "ens3".into(),
            vec![
                ManagedProtocol::VlessReality,
                ManagedProtocol::VmessWebsocket,
                ManagedProtocol::Hysteria2,
                ManagedProtocol::Tuic,
                ManagedProtocol::Anytls,
            ],
            Some("www.cloudflare.com".into()),
        )
        .unwrap();
        config.vless_reality.as_mut().unwrap().short_id = "0000000000000001".into();
        for secret in [
            "null",
            "true",
            "123456",
            "0x1234",
            "a: b",
            "#comment",
            " leading and trailing ",
            "line1\nline2",
            "a\u{85}b\u{2028}c\u{2029}d",
        ] {
            config.hysteria2.as_mut().unwrap().password = secret.into();
            config.tuic.as_mut().unwrap().password = secret.into();
            config.anytls.as_mut().unwrap().password = secret.into();
            config.vmess_websocket.as_mut().unwrap().path = "/ws#fragment: value".into();
            config.validate().unwrap();
            let nodes = crate::canonical::nodes(&config);
            for body in [clash(&config, &nodes), clash_legacy(&config, &nodes)] {
                let value: serde_yaml::Value = serde_yaml::from_str(&body.unwrap()).unwrap();
                let proxies = value["proxies"].as_sequence().unwrap();
                for proxy in proxies {
                    match proxy["type"].as_str().unwrap() {
                        "hysteria2" | "tuic" | "anytls" => {
                            assert_eq!(proxy["password"].as_str(), Some(secret));
                        }
                        "vmess" => assert_eq!(
                            proxy["ws-opts"]["path"].as_str(),
                            Some("/ws#fragment: value")
                        ),
                        "vless" => assert_eq!(
                            proxy["reality-opts"]["short-id"].as_str(),
                            Some(config.vless_reality.as_ref().unwrap().short_id.as_str())
                        ),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    #[test]
    fn provider_names_and_urls_remain_yaml_strings() {
        let mut config = DeploymentConfig::new(
            SubscriptionMode::IpFallback,
            "203.0.113.7".into(),
            None,
            Some(2080),
            "ens3".into(),
            vec![ManagedProtocol::VlessReality],
            Some("www.cloudflare.com".into()),
        )
        .unwrap();
        for name in ["true", "null", "1234", "中文规则"] {
            let url = "https://example.test/rules.mrs?name=value&other=two";
            config.client_extra_rule_sets = vec![ExtraRuleSet {
                name: name.into(),
                url: url.into(),
                outbound: "proxy".into(),
                behavior: Default::default(),
            }];
            config.validate().unwrap();
            let nodes = crate::canonical::nodes(&config);
            let value: serde_yaml::Value =
                serde_yaml::from_str(&clash(&config, &nodes).unwrap()).unwrap();
            let provider = &value["rule-providers"][name];
            assert_eq!(provider["url"].as_str(), Some(url));
            assert_eq!(
                provider["path"].as_str(),
                Some(format!("./ruleset/{name}.mrs").as_str())
            );
            assert_eq!(value["rule-providers"].as_mapping().unwrap().len(), 5);
        }
    }

    #[test]
    fn rule_set_intervals_reject_overflow_zero_and_invalid_text() {
        for (text, expected) in [
            ("30m", 1800),
            ("12h", 43200),
            ("7d", 604800),
            ("9223372036s", 9223372036),
            ("153722867m", 9223372020),
        ] {
            assert_eq!(extra_rule_set_seconds(text), Some(expected), "{text}");
        }
        for text in [
            "",
            "0s",
            "0d",
            "-1h",
            "1秒",
            "秒",
            " 12h",
            "12h ",
            "soon",
            "1.5h",
            "9223372037s",
            "153722868m",
            "2562048h",
            "106752d",
            "18446744073709551615d",
            "18446744073709551615m",
            "18446744073709551615h",
        ] {
            assert_eq!(extra_rule_set_seconds(text), None, "{text}");
        }
    }
}
