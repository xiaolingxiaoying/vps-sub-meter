use super::{
    AI_DOMAIN_SUFFIXES, AI_TAG, AUTO_TAG, FALLBACK_TAG, PROXY_TAG, STREAM_TAG, TELEGRAM_TAG,
    client_skip_cert_verify,
};
use crate::canonical::CanonicalNode;
use crate::config::{ClientRuleProfile, DeploymentConfig};
use crate::subscription::artifacts::SubscriptionError;
use crate::subscription::template::GroupTag;
use crate::subscription::{GroupRole, GroupSpec, RuleMatcher, RuleSetKind, TemplateSpec};

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
                    "  - name: {tag}\n    type: select\n    url: {probe_url}\n    interval: 300\n    lazy: false\n    proxies:\n"
                ));
            }
            GroupRole::UrlTest => {
                proxies.push_str(&format!(
                    "  - name: {tag}\n    type: url-test\n    url: {probe_url}\n    interval: 300\n    lazy: false\n    tolerance: 50\n    proxies:\n"
                ));
            }
            GroupRole::Fallback => {
                proxies.push_str(&format!(
                    "  - name: {tag}\n    type: fallback\n    url: {probe_url}\n    interval: 300\n    lazy: false\n    proxies:\n"
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
    let mut members: Vec<String> = nodes.iter().map(|node| node.tag().to_owned()).collect();
    if group.tag == GroupTag::Selector {
        members.extend([AUTO_TAG.to_owned(), CLASH_BUILTIN_DIRECT.to_owned()]);
    }
    members
}

/// The `rules:` block, shared by the current and the legacy artifact. The two
/// differ only in whether external rule-sets exist at all: mihomo 1.18 has no
/// `.mrs` rule-providers to reference, which is precisely the `minimal` case, so
/// the legacy artifact renders the twins and nothing else.
fn clash_rules(spec: &TemplateSpec, remote_rule_sets: bool, config: &DeploymentConfig) -> String {
    let mut rules = String::from("rules:\n");
    // A registered rule-set is an explicit decision, so it outranks the
    // template's generic geo verdicts — same precedence as the operator lists.
    if remote_rule_sets {
        rules.push_str(&clash_extra_rules(config));
    }
    // Portal detection and the local campus subnet must precede proxy rules.
    for rule in super::baseline::MIHOMO["rules"]
        .as_array()
        .unwrap()
        .iter()
        .take(8)
    {
        rules.push_str(&format!("  - {}\n", rule.as_str().unwrap()));
    }
    for rule in spec
        .inline_rules
        .iter()
        .filter(|rule| rule.renderers.includes_clash())
    {
        // Which matcher survives to the artifact. An entry clash has no URL for
        // is not this renderer's content in either profile, and a rule-set rule
        // under `minimal` is the twin — because naming the CDN is exactly what
        // that profile forbids. Anything already inline needs no substitution.
        let matcher = match rule.matcher {
            RuleMatcher::RuleSet(tags) => {
                let referenced = tags.iter().copied().any(|tag| {
                    spec.rule_sets
                        .iter()
                        .any(|entry| entry.tag == tag && entry.clash_url.is_some())
                });
                if !referenced {
                    continue;
                }
                if remote_rule_sets {
                    rule.matcher
                } else {
                    match rule.minimal_twin {
                        Some(twin) => twin,
                        None => continue,
                    }
                }
            }
            other => other,
        };
        let target = match matcher {
            // The two built-in geo codes have always carried the core's own
            // `DIRECT` policy here, not the `全球直连` group that wraps it.
            RuleMatcher::Private | RuleMatcher::Cn => CLASH_BUILTIN_DIRECT,
            _ => clash_tag(rule.outbound),
        };
        match matcher {
            // The two built-in geo codes. `GEOIP,*` is answered from mihomo's own
            // geo database — which mihomo *downloads* from GitHub when the file is
            // absent, so under `minimal` the same verdict is spelled out from the
            // compiled-in lists instead. That is a real trade: the CN blocks are
            // the coarse /10–/11 supernets and the private names are a curated
            // list, not the full database. The alternative is a profile that
            // promises no downloads and performs two of them on first start.
            RuleMatcher::Private => {
                if remote_rule_sets {
                    rules.push_str(&format!("  - GEOIP,LAN,{target}\n"));
                } else {
                    push_inline_domains(&mut rules, RuleMatcher::PRIVATE_DOMAINS, target);
                    push_inline_cidrs(&mut rules, RuleMatcher::PRIVATE_ADDRESS_LIST, target);
                }
            }
            RuleMatcher::Cn => {
                if remote_rule_sets {
                    rules.push_str(&format!("  - GEOIP,CN,{target}\n"));
                } else {
                    push_inline_domains(&mut rules, RuleMatcher::CN_DOMAINS, target);
                    push_inline_cidrs(&mut rules, RuleMatcher::CN_ADDRESSES, target);
                }
            }
            RuleMatcher::AiDomains => {
                for suffix in AI_DOMAIN_SUFFIXES {
                    rules.push_str(&format!("  - DOMAIN-SUFFIX,{suffix},{target}\n"));
                }
            }
            RuleMatcher::DomainSuffix(suffixes) => {
                for suffix in suffixes.iter() {
                    rules.push_str(&format!("  - DOMAIN-SUFFIX,{suffix},{target}\n"));
                }
            }
            RuleMatcher::IpCidr(cidrs) => {
                for cidr in cidrs.iter() {
                    // `no-resolve` keeps an IP rule from triggering a lookup of
                    // the destination it is about to match on.
                    rules.push_str(&format!("  - IP-CIDR,{cidr},{target},no-resolve\n"));
                }
            }
            RuleMatcher::RuleSet(tags) => {
                for tag in tags.iter().copied().filter(|tag| {
                    spec.rule_sets
                        .iter()
                        .any(|entry| entry.tag == *tag && entry.clash_url.is_some())
                }) {
                    rules.push_str(&format!("  - RULE-SET,{tag},{target}\n"));
                }
            }
        }
    }
    rules.push_str(&format!(
        "  - MATCH,{final_group}\n",
        final_group = clash_tag(spec.final_group)
    ));
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
fn clash_rule_providers(spec: &TemplateSpec, config: &DeploymentConfig) -> String {
    let mut providers = String::from("rule-providers:\n");
    for entry in spec
        .rule_sets
        .iter()
        .filter(|entry| entry.clash_url.is_some())
    {
        let behavior = match entry.kind {
            RuleSetKind::Domain => "domain",
            RuleSetKind::IpCidr => "ipcidr",
        };
        let url = entry
            .clash_url
            .as_deref()
            .expect("a clash rule-set carries a URL");
        providers.push_str(&format!(
            "  {tag}:\n    type: http\n    behavior: {behavior}\n    format: mrs\n    url: {url}\n    path: ./ruleset/{tag}.mrs\n    interval: {interval}\n",
            tag = entry.tag,
            interval = clash_interval(config)
        ));
    }
    // An operator registers a URL, not a behaviour, so everything is declared as
    // a domain set: mihomo only mis-sorts a domain list, never mis-routes an IP
    // one, and `rule-set add` documents that a CIDR list belongs in
    // `etc/sbctl/rules/direct.list` instead, where it is parsed as one.
    for set in &config.client_extra_rule_sets {
        providers.push_str(&format!(
            "  {tag}:\n    type: http\n    behavior: domain\n    format: mrs\n    url: {url}\n    path: ./ruleset/{tag}.mrs\n    interval: {interval}\n",
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
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (digits, unit) = text.split_at(text.len() - 1);
    let amount = digits.parse::<u64>().ok()?;
    Some(match unit {
        "s" => amount,
        "m" => amount * 60,
        "h" => amount * 3_600,
        "d" => amount * 86_400,
        _ => return None,
    })
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
    for set in &config.client_extra_rule_sets {
        lines.push_str(&format!(
            "  - RULE-SET,{name},{target}\n",
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
    let spec = TemplateSpec::for_template(config, crate::subscription::ClientTemplate::Standard);
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
    let spec = TemplateSpec::for_template(config, crate::subscription::ClientTemplate::Standard);
    let mut output = clash_proxies(config, nodes, &spec)?;
    output.push_str(&clash_rules(&spec, false, config));
    Ok(super::baseline::clash_settings(
        &output,
        config,
        false,
        spec.dns.direct_rule_set.is_some(),
    ))
}
