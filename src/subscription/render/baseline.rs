//! Vendored mihomo-test defaults, compiled in so VPS installs are self-contained.
//! Node credentials, policy catalogs and version-specific fields stay generated.
use std::sync::LazyLock;

use crate::config::{ClientDnsMode, DeploymentConfig};
use crate::subscription::RuleMatcher;
use serde_json::Value;

pub(super) static SING_BOX: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../templates/sing-box.json"))
        .expect("bundled sing-box template is valid JSON")
});

pub(super) static MIHOMO: LazyLock<Value> = LazyLock::new(|| {
    serde_yaml::from_str(include_str!("../templates/mihomo.yaml"))
        .expect("bundled mihomo template is valid YAML")
});

pub(super) fn portal_domains() -> Value {
    SING_BOX["dns"]["rules"][0]["domain_suffix"].clone()
}

pub(super) fn clash_settings(
    generated_yaml: &str,
    config: &DeploymentConfig,
    remote: bool,
    cn_direct: bool,
) -> String {
    let mut base = MIHOMO.clone();
    // These sections contain live nodes, selected policies and CDN configuration.
    for key in ["proxies", "proxy-groups", "rules", "rule-providers", "mode"] {
        base.as_object_mut().unwrap().remove(key);
    }
    base["dns"]["enhanced-mode"] = Value::String(
        match config.client_dns_mode {
            ClientDnsMode::FakeIp => "fake-ip",
            ClientDnsMode::RedirHost => "redir-host",
        }
        .into(),
    );
    // Keep established group names so existing administrator overrides resolve.
    for server in base["dns"]["nameserver"].as_array_mut().unwrap() {
        *server = Value::String(server.as_str().unwrap().replace("#代理", "#节点选择"));
    }
    let policies = base["dns"]["nameserver-policy"].as_object_mut().unwrap();
    let domestic = policies.remove("geosite:cn,private").unwrap();
    // rule-set policies reuse our providers rather than implicitly fetching geosite.dat.
    if remote {
        policies.insert("rule-set:geosite-private".into(), domestic.clone());
        if cn_direct {
            policies.insert("rule-set:geosite-cn".into(), domestic.clone());
        }
    }
    if !remote {
        for domain in RuleMatcher::PRIVATE_DOMAINS.iter().chain(
            if cn_direct {
                RuleMatcher::CN_DOMAINS
            } else {
                &[]
            }
            .iter(),
        ) {
            policies.insert(format!("+.{domain}"), domestic.clone());
        }
        base.as_object_mut().unwrap().remove("geox-url");
    }
    format!(
        "{}{}",
        generated_yaml,
        serde_yaml::to_string(&base).expect("template values serialize")
    )
}
