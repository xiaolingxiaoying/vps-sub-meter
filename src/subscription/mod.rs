//! Subscription artefacts: the client version matrix, the generated artefacts and
//! configuration transactions, the HTTP/TLS/ACME server, and the per-format
//! renderers. Every public item of the crate-facing surface is re-exported here
//! so callers keep using `sbctl::subscription::...` unchanged.
mod artifacts;
mod profile;
pub(crate) mod render;
mod serve;
mod template;
#[cfg(test)]
mod test_support;

use base64::Engine;

pub use artifacts::{
    DeploymentSnapshot, SubscriptionError, apply_config_transaction, check_sing_box_config,
    generated_artifacts, generated_artifacts_for_kernel, read_authorized, regenerate,
    regenerate_current, resolve_full_profile, restore_config_transaction, route_url,
    route_url_with_credential, subscription_url,
};

pub use profile::{
    CLASH_LEGACY_VERSION, ClientSubscriptionFormat, ClientSubscriptionRow, ClientVersion,
    SING_BOX_VERSION_PROFILES, SingBoxVersionProfile, SubscriptionFormat, SubscriptionLinkInfo,
    SubscriptionRoute, band_warning_for, client_subscription_matrix, installed_kernel_version,
    kernel_band_warning, latest_version_profile, parse_kernel_version, subscription_matrix,
};
pub use render::{
    AI_DOMAIN_SUFFIXES, AUTO_TAG, SELECTOR_TAG, ensure_external_proxy_listener_available,
};

/// Parses the stored `client_rule_set_update_interval` into seconds.
///
/// Exported because the CLI validates an operator's answer before storing it:
/// the string goes straight into every subscriber's config as `update_interval`,
/// and mihomo wants seconds, so one parser serves both shapes.
pub fn rule_set_interval_seconds(text: &str) -> Option<u64> {
    render::extra_rule_set_seconds(text)
}
pub use serve::{redact_secret, serve};
pub use template::{
    ClientTemplate, DnsSpec, GroupRole, GroupSpec, InlineRule, OutboundRole, RuleMatcher,
    RuleRenderers, RuleSetKind, RuleSetSpec, TemplateSpec, default_client_template,
};

/// The native share link for one managed node (`vless://…`, `vmess://…`,
/// `hysteria2://…`, `tuic://…`, `anytls://…`).
///
/// This is the same string the `uri` artifact carries, exposed so the index page
/// and `sbctl node --links` can show a node's parameters without making the
/// operator download a credential'd file to see their own configuration. The
/// returned line keeps its trailing newline, matching the artifact byte-for-byte.
pub fn node_share_link(
    config: &crate::config::DeploymentConfig,
    node: &crate::canonical::CanonicalNode,
) -> String {
    render::node_uri(render::insecure_flag(config), &node.with_bracketed_host())
}

fn base64_uri(uri: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(uri.as_bytes())
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut different = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        different |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    different == 0
}

/// Whether `provided` is one of the credentials this deployment accepts: the
/// default link, plus every named credential still inside its grace window.
///
/// Every candidate is compared even after one matches, so neither the timing nor
/// the number of comparisons distinguishes "unknown link" from "expired link"
/// from "valid link" — the uniform 404 the server returns for all three is only
/// meaningful if the check behind it is uniform too.
pub fn credential_matches(
    config: &crate::config::DeploymentConfig,
    provided: &str,
    now: i64,
) -> bool {
    let provided = provided.as_bytes();
    let mut matched = constant_time_eq(provided, config.subscription_credential.as_bytes());
    for entry in &config.subscription_credentials {
        // The comparison always runs; only then is the grace window applied.
        // `is_active() && constant_time_eq(…)` would skip the work for an
        // expired link and hand back a timing signal for "this name exists".
        let equal = constant_time_eq(provided, entry.credential.as_bytes());
        matched |= equal && entry.is_active(now);
    }
    matched
}
