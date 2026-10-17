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
    serde_json::json!([
        "msftconnecttest.com",
        "msftncsi.com",
        "captive.apple.com",
        "connectivitycheck.gstatic.com",
        "detectportal.firefox.com",
        "connectivity-check.ubuntu.com",
        "networkcheck.kde.org"
    ])
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
    let policies = base["dns"]["nameserver-policy"].as_object_mut().unwrap();
    if !remote {
        let domestic = policies.remove("rule-set:review-cn-domain").unwrap();
        let private = policies.remove("rule-set:review-private-domain").unwrap();
        for domain in RuleMatcher::PRIVATE_DOMAINS {
            policies.insert(format!("+.{domain}"), private.clone());
        }
        if cn_direct {
            for domain in RuleMatcher::CN_DOMAINS {
                policies.insert(format!("+.{domain}"), domestic.clone());
            }
        }
        let filters = base["dns"]["fake-ip-filter"].as_array_mut().unwrap();
        filters.retain(|v| !v.as_str().unwrap().starts_with("rule-set:"));
        filters.extend(
            RuleMatcher::PRIVATE_DOMAINS
                .iter()
                .map(|d| Value::String(format!("+.{d}"))),
        );
        base.as_object_mut().unwrap().remove("geox-url");
    }
    format!(
        "{}{}",
        generated_yaml,
        serde_yaml::to_string(&base).expect("template values serialize")
    )
}
