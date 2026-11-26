//! Deep-merge helper for optional JSON overrides.
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
}
