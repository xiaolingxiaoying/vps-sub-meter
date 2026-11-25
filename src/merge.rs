//! Deep-merge helper for optional JSON and YAML overrides.
//! Objects/mappings merge recursively; `rules` arrays prepend so override rules
//! take precedence over generated rules; other arrays replace.

use serde_json::Value;

pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing) if key == "rules" && existing.is_array() && value.is_array() => {
                        let mut merged = value.as_array().cloned().unwrap_or_default();
                        merged.extend(existing.as_array().cloned().unwrap_or_default());
                        *existing = Value::Array(merged);
                    }
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

pub fn deep_merge_yaml(base: &mut serde_yaml::Value, overlay: &serde_yaml::Value) {
    use serde_yaml::Value as Yaml;
    match (base, overlay) {
        (Yaml::Mapping(base_map), Yaml::Mapping(overlay_map)) => {
            for (key, value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(existing)
                        if key.as_str() == Some("rules")
                            && existing.is_sequence()
                            && value.is_sequence() =>
                    {
                        let mut merged = value.as_sequence().cloned().unwrap_or_default();
                        merged.extend(existing.as_sequence().cloned().unwrap_or_default());
                        *existing = Yaml::Sequence(merged);
                    }
                    Some(existing) => deep_merge_yaml(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_merge_recurses_and_prepends_rules() {
        let mut base = json!({"log":{"level":"warn"},"route":{"rules":["match"]}});
        let overlay = json!({"log":{"access":"none"},"route":{"rules":["block"]}});
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base,
            json!({"log":{"level":"warn","access":"none"},"route":{"rules":["block","match"]}})
        );
    }

    #[test]
    fn yaml_merge_recurses_and_prepends_rules() {
        let mut base: serde_yaml::Value =
            serde_yaml::from_str("dns:\n  enable: true\nrules:\n  - MATCH,代理\n").unwrap();
        let overlay: serde_yaml::Value =
            serde_yaml::from_str("dns:\n  ipv6: false\nrules:\n  - DOMAIN,a.test,DIRECT\n")
                .unwrap();
        deep_merge_yaml(&mut base, &overlay);
        let text = serde_yaml::to_string(&base).unwrap();
        assert!(text.contains("- DOMAIN,a.test,DIRECT\n- MATCH,代理"));
        assert!(text.contains("ipv6: false"));
    }
}
