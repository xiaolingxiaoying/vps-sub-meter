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
    let mut proxies = String::from("proxies:\n");
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
            } => format!(
                "  - name: {}\n    type: vless\n    server: {host}\n    port: {port}\n    uuid: {uuid}\n    network: tcp\n    udp: true\n    flow: xtls-rprx-vision\n    tls: true\n    servername: {decoy_sni}\n    client-fingerprint: chrome\n    reality-opts:\n      public-key: {public_key}\n      short-id: {short_id}\n",
                node.tag()
            ),
            CanonicalNode::VmessWebsocket {
                host,
                port,
                tls_server_name,
                uuid,
                path,
            } => format!(
                "  - name: {}\n    type: vmess\n    server: {host}\n    port: {port}\n    uuid: {uuid}\n    alterId: 0\n    cipher: auto\n    tls: true\n    servername: {tls_server_name}\n    skip-cert-verify: {skip}\n    network: ws\n    ws-opts:\n      path: {path}\n      headers:\n        Host: {tls_server_name}\n",
                node.tag()
            ),
            CanonicalNode::Hysteria2 {
                host,
                port,
                tls_server_name,
                password,
            } => format!(
                "  - name: {}\n    type: hysteria2\n    server: {host}\n    port: {port}\n    password: {password}\n    sni: {tls_server_name}\n    skip-cert-verify: {skip}\n    up: 50 Mbps\n    down: 200 Mbps\n",
                node.tag()
            ),
            CanonicalNode::Tuic {
                host,
                port,
                tls_server_name,
                uuid,
                password,
            } => format!(
                "  - name: {}\n    type: tuic\n    server: {host}\n    port: {port}\n    uuid: {uuid}\n    password: {password}\n    sni: {tls_server_name}\n    alpn:\n      - h3\n    skip-cert-verify: {skip}\n",
                node.tag()
            ),
            CanonicalNode::Anytls {
                host,
                port,
                tls_server_name,
                password,
            } => format!(
                "  - name: {}\n    type: anytls\n    server: {host}\n    port: {port}\n    password: {password}\n    client-fingerprint: chrome\n    udp: true\n    idle-session-check-interval: 30\n    idle-session-timeout: 30\n    tls: true\n    sni: {tls_server_name}\n    skip-cert-verify: {skip}\n",
                node.tag()
            ),
        };
        proxies.push_str(&entry);
    }
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
    let mut providers =
        serde_yaml::to_string(&serde_json::json!({"rule-providers": values})).unwrap();
    for set in config
        .client_extra_rule_sets
        .iter()
        .filter(|set| set.is_mihomo())
    {
        let behavior = match set.behavior {
            crate::config::RuleSetBehavior::Domain => "domain",
            crate::config::RuleSetBehavior::Ipcidr => "ipcidr",
        };
        providers.push_str(&format!(
            "  {tag}:\n    type: http\n    behavior: {behavior}\n    format: mrs\n    url: {url}\n    path: ./ruleset/{tag}.mrs\n    interval: {interval}\n",
            tag = set.name,
            url = set.url,
            interval = clash_interval(config)
        ));
    }
    providers
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
    use super::extra_rule_set_seconds;

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
