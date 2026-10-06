//! The one deep-merge every sbctl config override uses, so the server (ADR-0021,
//! `docs/adr/0021-server-side-override-templates.md`) and the clients cannot
//! drift on what "覆写" means. The semantics, quoted from the ADR:
//!
//! - objects merge recursively; every other type replaces wholesale;
//! - arrays replace wholesale, except an array under a key literally named
//!   `rules`, which is **prepended** to the merged array so an override rule
//!   wins against the generated verdicts without restating them.
//!
//! [`MERGE_SEMANTICS`] is the shared truth table: the server, `client-core` and
//! this crate each assert against it, so changing the semantics reddens every
//! consumer's tests at once instead of silently forking one of them.

use serde_json::Value;

/// One row of the shared semantics table: merging `overlay` onto `base` must
/// produce `expected`. Documented as JSON text so a failing case prints as
/// readable JSON rather than a `Value` debug dump.
#[derive(Clone, Copy, Debug)]
pub struct MergeCase {
    pub name: &'static str,
    pub base: &'static str,
    pub overlay: &'static str,
    pub expected: &'static str,
}

/// The behaviour ADR-0021 locks, as data. Every consumer of [`deep_merge`]
/// asserts against this same list (see [`merge_semantics_failures`]).
pub const MERGE_SEMANTICS: &[MergeCase] = &[
    MergeCase {
        name: "objects-merge-recursively",
        base: r#"{"log":{"level":"info"},"keep":true}"#,
        overlay: r#"{"log":{"level":"warn","timestamp":true},"keep":false}"#,
        expected: r#"{"log":{"level":"warn","timestamp":true},"keep":false}"#,
    },
    MergeCase {
        name: "absent-keys-are-added",
        base: r#"{"dns":{"tag":"mine"}}"#,
        overlay: r#"{"dns":{"servers":["223.5.5.5"]},"experimental":{"cache_file":{"enabled":true}}}"#,
        expected: r#"{"dns":{"tag":"mine","servers":["223.5.5.5"]},"experimental":{"cache_file":{"enabled":true}}}"#,
    },
    MergeCase {
        name: "arrays-replace-by-default",
        base: r#"{"outbounds":["a"]}"#,
        overlay: r#"{"outbounds":["b","c"]}"#,
        expected: r#"{"outbounds":["b","c"]}"#,
    },
    MergeCase {
        name: "rules-arrays-are-prepended",
        base: r#"{"rules":["generated"]}"#,
        overlay: r#"{"rules":["override"]}"#,
        expected: r#"{"rules":["override","generated"]}"#,
    },
    MergeCase {
        name: "nested-dns-rules-are-prepended",
        base: r#"{"dns":{"rules":[{"generated":true}]}}"#,
        overlay: r#"{"dns":{"rules":[{"override":true}]}}"#,
        expected: r#"{"dns":{"rules":[{"override":true},{"generated":true}]}}"#,
    },
    MergeCase {
        name: "a-rules-key-that-is-not-an-array-replaces",
        base: r#"{"route":{"rules":"generated"}}"#,
        overlay: r#"{"route":{"rules":[{"action":"direct"}]}}"#,
        expected: r#"{"route":{"rules":[{"action":"direct"}]}}"#,
    },
    MergeCase {
        name: "an-object-over-a-scalar-merges-into-the-replacement",
        base: r#"{"dns":"system"}"#,
        overlay: r#"{"dns":{"servers":["1.1.1.1"]}}"#,
        expected: r#"{"dns":{"servers":["1.1.1.1"]}}"#,
    },
    MergeCase {
        name: "an-empty-overlay-changes-nothing",
        base: r#"{"log":{"level":"info"},"route":{"rules":["generated"]}}"#,
        overlay: r#"{}"#,
        expected: r#"{"log":{"level":"info"},"route":{"rules":["generated"]}}"#,
    },
];

/// Runs [`MERGE_SEMANTICS`] through [`deep_merge`] and returns the names of the
/// rows that do not hold, so a consumer's test can assert on the shared table
/// without restating it. An empty vector means the semantics are intact.
pub fn merge_semantics_failures() -> Vec<&'static str> {
    MERGE_SEMANTICS
        .iter()
        .filter(|case| {
            let mut base: Value =
                serde_json::from_str(case.base).expect("the shared table holds valid JSON base");
            let overlay: Value = serde_json::from_str(case.overlay)
                .expect("the shared table holds valid JSON overlay");
            let expected: Value =
                serde_json::from_str(case.expected).expect("the shared table holds valid JSON");
            deep_merge(&mut base, &overlay);
            base != expected
        })
        .map(|case| case.name)
        .collect()
}

/// Deep-merges `overlay` into `base` in place. Objects merge recursively and
/// other types replace — except an array under a key literally named `rules`,
/// which the overlay prepends to so an override rule wins over the generated
/// verdicts without restating the whole list.
///
/// This is ADR-0021 frozen: it is exactly [`deep_merge_with`] under the default
/// policy, and a test walks [`MERGE_SEMANTICS`] through both to prove the
/// default never drifts from what the clients assert.
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    deep_merge_with(base, overlay, &MergePolicy::default());
}

/// The YAML twin of [`deep_merge`], operating on `serde_yaml::Value`. Same
/// semantics, same `rules` prepend, and the same table behind it: see
/// [`merge_semantics_yaml_failures`].
#[cfg(feature = "yaml")]
pub fn deep_merge_yaml(base: &mut serde_yaml::Value, overlay: &serde_yaml::Value) {
    deep_merge_yaml_with(base, overlay, &MergePolicy::default());
}

/// The same shared table, run through [`deep_merge_yaml`] by round-tripping each
/// row through YAML. A row whose JSON is not expressible as a YAML mapping at
/// the top level is skipped rather than pretended to be covered.
#[cfg(feature = "yaml")]
pub fn merge_semantics_yaml_failures() -> Vec<&'static str> {
    MERGE_SEMANTICS
        .iter()
        .filter(|case| {
            let Ok(mut base) = serde_yaml::from_str::<serde_yaml::Value>(case.base) else {
                return false;
            };
            let Ok(overlay) = serde_yaml::from_str::<serde_yaml::Value>(case.overlay) else {
                return false;
            };
            let Ok(expected) = serde_yaml::from_str::<serde_yaml::Value>(case.expected) else {
                return false;
            };
            deep_merge_yaml(&mut base, &overlay);
            base != expected
        })
        .map(|case| case.name)
        .collect()
}

/// Where an overlay's `rules` array lands relative to the generated one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RulesMode {
    /// ADR-0021: the overlay wins against the generated verdicts without
    /// restating them. This is the frozen default.
    #[default]
    Prepend,
    /// The overlay only catches what the generated rules did not decide.
    Append,
    /// The overlay replaces the generated list entirely.
    Replace,
}

/// How [`deep_merge_with`] treats arrays, beyond the frozen default.
///
/// Every field defaults to the ADR-0021 semantics [`deep_merge`] implements, so
/// a policy is an opt-in widening for one caller. The default path must stay
/// byte-identical: `client-core` asserts the same [`MERGE_SEMANTICS`] table, and
/// a changed default would silently fork what "覆写" means for installed clients.
#[derive(Clone, Copy, Debug, Default)]
pub struct MergePolicy<'a> {
    pub rules: RulesMode,
    /// Arrays merged element-wise instead of wholesale, as `(array key,
    /// identifier field)`: `("outbounds", "tag")` for a sing-box profile,
    /// `("proxies", "name")` for a clash one. An overlay element whose
    /// identifier matches a base element is deep-merged into it; a new
    /// identifier is appended. Elements without the identifier field fall back
    /// to wholesale replacement, which is what a malformed override deserves.
    pub keyed_arrays: &'a [(&'a str, &'a str)],
}

impl<'a> MergePolicy<'a> {
    fn identifier_field(&self, key: &str) -> Option<&'a str> {
        self.keyed_arrays
            .iter()
            .find(|(array_key, _)| *array_key == key)
            .map(|(_, field)| *field)
    }
}

/// [`deep_merge`] with an explicit policy. See [`MergePolicy`].
pub fn deep_merge_with(base: &mut Value, overlay: &Value, policy: &MergePolicy<'_>) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) => {
                        if existing.is_array() && value.is_array() {
                            if key == "rules" {
                                merge_rules(existing, value, policy.rules);
                                continue;
                            }
                            if let Some(field) = policy.identifier_field(key)
                                && merge_keyed(existing, value, field, policy)
                            {
                                continue;
                            }
                        }
                        deep_merge_with(existing, value, policy);
                    }
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

fn merge_rules(base: &mut Value, overlay: &Value, mode: RulesMode) {
    let Value::Array(base_rules) = base else {
        return;
    };
    let Value::Array(overlay_rules) = overlay else {
        return;
    };
    match mode {
        RulesMode::Prepend => {
            let mut merged = overlay_rules.clone();
            merged.extend(base_rules.iter().cloned());
            *base_rules = merged;
        }
        RulesMode::Append => base_rules.extend(overlay_rules.iter().cloned()),
        RulesMode::Replace => *base_rules = overlay_rules.clone(),
    }
}

/// Whether one array element can take part in an identifier merge.
fn has_identifier(item: &Value, field: &str) -> bool {
    item.is_object() && item.get(field).and_then(Value::as_str).is_some()
}

/// Merges two arrays of objects by identifier. Returns `false` when the arrays
/// are not shaped for it, so the caller falls back to a plain merge.
fn merge_keyed(base: &mut Value, overlay: &Value, field: &str, policy: &MergePolicy<'_>) -> bool {
    let (Value::Array(base_items), Value::Array(overlay_items)) = (base, overlay) else {
        return false;
    };
    let shaped = base_items
        .iter()
        .chain(overlay_items.iter())
        .all(|item| has_identifier(item, field));
    if !shaped {
        return false;
    }
    for incoming in overlay_items {
        let id = incoming[field].as_str().expect("checked above");
        match base_items
            .iter_mut()
            .find(|existing| existing.get(field).and_then(Value::as_str) == Some(id))
        {
            Some(existing) => deep_merge_with(existing, incoming, policy),
            None => base_items.push(incoming.clone()),
        }
    }
    true
}

/// The YAML twin of [`deep_merge_with`].
#[cfg(feature = "yaml")]
pub fn deep_merge_yaml_with(
    base: &mut serde_yaml::Value,
    overlay: &serde_yaml::Value,
    policy: &MergePolicy<'_>,
) {
    use serde_yaml::Value as Yaml;
    match (base, overlay) {
        (Yaml::Mapping(base_map), Yaml::Mapping(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) => {
                        if key.as_str() == Some("rules")
                            && existing.is_sequence()
                            && value.is_sequence()
                        {
                            merge_yaml_rules(existing, value, policy.rules);
                            continue;
                        }
                        if let Some(field) = policy.identifier_field(key.as_str().unwrap_or(""))
                            && merge_yaml_keyed(existing, value, field, policy)
                        {
                            continue;
                        }
                        deep_merge_yaml_with(existing, value, policy);
                    }
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

#[cfg(feature = "yaml")]
fn merge_yaml_rules(base: &mut serde_yaml::Value, overlay: &serde_yaml::Value, mode: RulesMode) {
    let (serde_yaml::Value::Sequence(base_rules), serde_yaml::Value::Sequence(overlay_rules)) =
        (base, overlay)
    else {
        return;
    };
    match mode {
        RulesMode::Prepend => {
            let mut merged = overlay_rules.clone();
            merged.extend(base_rules.iter().cloned());
            *base_rules = merged;
        }
        RulesMode::Append => base_rules.extend(overlay_rules.iter().cloned()),
        RulesMode::Replace => *base_rules = overlay_rules.clone(),
    }
}

#[cfg(feature = "yaml")]
fn merge_yaml_keyed(
    base: &mut serde_yaml::Value,
    overlay: &serde_yaml::Value,
    field: &str,
    policy: &MergePolicy<'_>,
) -> bool {
    use serde_yaml::Value as Yaml;
    let (Yaml::Sequence(base_items), Yaml::Sequence(overlay_items)) = (base, overlay) else {
        return false;
    };
    let identifier = |item: &Yaml| -> Option<String> {
        match item.get(field) {
            Some(Yaml::String(text)) => Some(text.clone()),
            _ => None,
        }
    };
    if base_items
        .iter()
        .chain(overlay_items.iter())
        .any(|item| !item.is_mapping() || identifier(item).is_none())
    {
        return false;
    }
    for incoming in overlay_items {
        let id = identifier(incoming).expect("checked above");
        match base_items
            .iter_mut()
            .find(|existing| identifier(existing).as_deref() == Some(id.as_str()))
        {
            Some(existing) => deep_merge_yaml_with(existing, incoming, policy),
            None => base_items.push(incoming.clone()),
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_shared_table_holds_for_json() {
        assert_eq!(merge_semantics_failures(), Vec::<&str>::new());
    }

    #[test]
    fn merging_is_in_place_and_leaves_the_overlay_usable() {
        let overlay = json!({"route": {"rules": [{"action": "direct"}]}});
        let mut base = json!({"route": {"rules": [{"action": "proxy"}]}});
        deep_merge(&mut base, &overlay);
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base,
            json!({"route": {"rules": [{"action": "direct"}, {"action": "direct"}, {"action": "proxy"}]}}),
            "the overlay must not be consumed by the merge"
        );
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn the_shared_table_holds_for_yaml() {
        assert_eq!(merge_semantics_yaml_failures(), Vec::<&str>::new());
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn yaml_rules_are_prepended() {
        let mut base = serde_yaml::from_str("rules:\n  - generated\n").expect("yaml");
        let overlay = serde_yaml::from_str("rules:\n  - override\n").expect("yaml");
        deep_merge_yaml(&mut base, &overlay);
        let text = serde_yaml::to_string(&base).expect("yaml round-trip");
        assert!(text.contains("- override\n- generated"), "{text}");
    }

    #[test]
    fn the_default_policy_reproduces_every_frozen_merge_semantics_row() {
        // The frozen path now runs through the policy engine; this proves the
        // opt-in widening did not change what `client-core` relies on.
        for case in MERGE_SEMANTICS {
            let mut base: Value = serde_json::from_str(case.base).expect("base");
            let overlay: Value = serde_json::from_str(case.overlay).expect("overlay");
            let expected: Value = serde_json::from_str(case.expected).expect("expected");
            deep_merge_with(&mut base, &overlay, &MergePolicy::default());
            assert_eq!(base, expected, "row {}", case.name);
        }
    }

    #[test]
    fn an_appended_rules_mode_keeps_generated_verdicts_first() {
        let mut base = json!({"route": {"rules": [{"action": "direct"}]}});
        let overlay = json!({"route": {"rules": [{"action": "proxy"}]}});
        deep_merge_with(
            &mut base,
            &overlay,
            &MergePolicy {
                rules: RulesMode::Append,
                ..MergePolicy::default()
            },
        );
        assert_eq!(
            base,
            json!({"route": {"rules": [{"action": "direct"}, {"action": "proxy"}]}}),
            "append must not shadow the generated verdicts"
        );
    }

    #[test]
    fn a_replace_rules_mode_drops_the_generated_verdicts() {
        let mut base = json!({"route": {"rules": [{"action": "direct"}]}});
        let overlay = json!({"route": {"rules": [{"action": "proxy"}]}});
        deep_merge_with(
            &mut base,
            &overlay,
            &MergePolicy {
                rules: RulesMode::Replace,
                ..MergePolicy::default()
            },
        );
        assert_eq!(base, json!({"route": {"rules": [{"action": "proxy"}]}}));
    }

    #[test]
    fn a_keyed_array_merges_by_tag_and_appends_new_members() {
        let mut base = json!({"outbounds": [
            {"tag": "节点选择", "type": "selector", "outbounds": ["a"]},
            {"tag": "direct", "type": "direct"}
        ]});
        let overlay = json!({"outbounds": [
            {"tag": "节点选择", "outbounds": ["a", "b"]},
            {"tag": "my-group", "type": "urltest", "outbounds": ["a"]}
        ]});
        deep_merge_with(
            &mut base,
            &overlay,
            &MergePolicy {
                keyed_arrays: &[("outbounds", "tag")],
                ..MergePolicy::default()
            },
        );
        let outbounds = base["outbounds"].as_array().expect("array");
        assert_eq!(outbounds.len(), 3, "merged, appended, untouched");
        assert_eq!(
            outbounds[0],
            json!({"tag": "节点选择", "type": "selector", "outbounds": ["a", "b"]}),
            "a matching tag is deep-merged, keeping fields the overlay omits"
        );
        assert_eq!(outbounds[1], json!({"tag": "direct", "type": "direct"}));
        assert_eq!(outbounds[2]["tag"], "my-group");
    }

    #[test]
    fn a_keyed_array_without_identifiers_falls_back_to_replacement() {
        let mut base = json!({"outbounds": ["a"]});
        let overlay = json!({"outbounds": ["b"]});
        deep_merge_with(
            &mut base,
            &overlay,
            &MergePolicy {
                keyed_arrays: &[("outbounds", "tag")],
                ..MergePolicy::default()
            },
        );
        assert_eq!(base, json!({"outbounds": ["b"]}));
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn yaml_keyed_arrays_merge_by_name() {
        let mut base: serde_yaml::Value =
            serde_yaml::from_str("proxies:\n  - name: a\n    server: 1.2.3.4\n").expect("yaml");
        let overlay: serde_yaml::Value =
            serde_yaml::from_str("proxies:\n  - name: a\n    port: 8443\n  - name: b\n")
                .expect("yaml");
        deep_merge_yaml_with(
            &mut base,
            &overlay,
            &MergePolicy {
                keyed_arrays: &[("proxies", "name")],
                ..MergePolicy::default()
            },
        );
        let text = serde_yaml::to_string(&base).expect("yaml round-trip");
        assert!(
            text.contains("name: a") && text.contains("port: 8443") && text.contains("name: b"),
            "{text}"
        );
        assert!(
            text.contains("server: 1.2.3.4"),
            "the merged entry keeps fields the override omits: {text}"
        );
    }
}
