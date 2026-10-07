//! Server-side override templates merged into the generated artifacts.
//!
//! Overrides live under `etc/sbctl/overrides/` inside the deployment root.
//! Three targets exist, each as one base document plus a drop-in directory
//! whose `*.json` / `*.yaml` files merge in filename order after it:
//!
//! - `sing-box-override.json` + `sing-box.d/` → every full sing-box client profile;
//! - `clash-override.yaml` + `clash.d/` → the current and legacy clash artifacts;
//! - `sing-box-server.json` + `sing-box-server.d/` → the server configuration
//!   this host runs, which is what turns a VPS install into something the
//!   operator can extend without patching sbctl.
//!
//! Merge semantics, documented in ADR-0021 and widened by the layered-override
//! ADR:
//!
//! - objects merge recursively; every other type replaces wholesale;
//! - arrays replace wholesale, except an array under a key literally named
//!   `rules`, which is **prepended** to the generated array (opt out per
//!   document with a top-level `rules_mode` of `append` or `replace`);
//! - `outbounds` (sing-box) and `proxies` / `proxy-groups` (clash) merge
//!   element-wise by `tag` / `name`, so one extra group no longer means
//!   restating every generated one;
//! - the historical bare-`outbounds` sing-box artifact and the URI artifacts
//!   are never overridden, so their byte compatibility is preserved;
//! - a server override may not touch inbound credentials, because those are
//!   what clients authenticate with.

use std::fs;
use std::path::{Path, PathBuf};

pub const SING_BOX_OVERRIDE_RELATIVE_PATH: &str = "etc/sbctl/overrides/sing-box-override.json";
pub const CLASH_OVERRIDE_RELATIVE_PATH: &str = "etc/sbctl/overrides/clash-override.yaml";
pub const SING_BOX_SERVER_OVERRIDE_RELATIVE_PATH: &str = "etc/sbctl/overrides/sing-box-server.json";
pub const SING_BOX_OVERRIDE_DIRECTORY: &str = "etc/sbctl/overrides/sing-box.d";
pub const CLASH_OVERRIDE_DIRECTORY: &str = "etc/sbctl/overrides/clash.d";
pub const SING_BOX_SERVER_OVERRIDE_DIRECTORY: &str = "etc/sbctl/overrides/sing-box-server.d";

/// The per-document key that selects how its `rules` array merges. Stripped
/// before merging so it never reaches sing-box or mihomo as an unknown field.
pub const RULES_MODE_KEY: &str = "rules_mode";

/// Merge policy for the client artifacts: identifier-keyed outbound arrays so a
/// template can add one group without restating the generated ones.
pub fn client_policy<'a>() -> json_merge::MergePolicy<'a> {
    json_merge::MergePolicy {
        keyed_arrays: &[("outbounds", "tag")],
        ..json_merge::MergePolicy::default()
    }
}

/// Merge policy for the clash artifacts, whose groups are keyed by `name`.
pub fn clash_policy<'a>() -> json_merge::MergePolicy<'a> {
    json_merge::MergePolicy {
        keyed_arrays: &[
            ("proxies", "name"),
            ("proxy-groups", "name"),
            ("rule-providers", "name"),
        ],
        ..json_merge::MergePolicy::default()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OverrideError {
    #[error("{path} is not valid JSON: {message}")]
    Json { path: PathBuf, message: String },
    #[error("{path} is not valid YAML: {message}")]
    Yaml { path: PathBuf, message: String },
    #[error("{path} must contain a JSON object or YAML mapping at the top level")]
    NotMapping { path: PathBuf },
    #[error("{path} has rules_mode {value:?}; expected \"prepend\", \"append\" or \"replace\"")]
    RulesMode { path: PathBuf, value: String },
    #[error(
        "{path} may not override {field}: server-side credentials belong to the \
         generated configuration (use the client overrides for your own nodes)"
    )]
    ProtectedField { path: PathBuf, field: String },
    #[error("override read failed: {0}")]
    Io(#[from] std::io::Error),
}

/// The parsed override documents for one deployment; empty when none exist.
#[derive(Debug, Default)]
pub struct Overrides {
    pub sing_box: Option<serde_json::Value>,
    pub clash: Option<serde_yaml::Value>,
    pub sing_box_server: Option<serde_json::Value>,
}

impl Overrides {
    /// Loads every override document from `<root>/etc/sbctl/overrides/`.
    /// A malformed file aborts generation so the transactional writer keeps
    /// the previous known-good artifacts. Protected server inbound fields are
    /// rejected here as well, including when no deployment has been initialized.
    pub fn load(root: &Path) -> Result<Self, OverrideError> {
        let sing_box_server = load_json_layered(
            root,
            SING_BOX_SERVER_OVERRIDE_RELATIVE_PATH,
            SING_BOX_SERVER_OVERRIDE_DIRECTORY,
            &json_merge::MergePolicy::default(),
        )?;
        if let Some(document) = sing_box_server.as_ref() {
            reject_protected_server_inbounds(
                document,
                &root.join(SING_BOX_SERVER_OVERRIDE_RELATIVE_PATH),
            )?;
        }
        Ok(Self {
            sing_box: load_json_layered(
                root,
                SING_BOX_OVERRIDE_RELATIVE_PATH,
                SING_BOX_OVERRIDE_DIRECTORY,
                &client_policy(),
            )?,
            clash: load_yaml_layered(
                root,
                CLASH_OVERRIDE_RELATIVE_PATH,
                CLASH_OVERRIDE_DIRECTORY,
                &clash_policy(),
            )?,
            sing_box_server,
        })
    }

    /// Whether any override would change the generated bytes. The regeneration
    /// transaction uses this to decide whether the extra real-kernel check of
    /// the merged client profile is worth paying for.
    pub fn is_empty(&self) -> bool {
        self.sing_box.is_none() && self.clash.is_none() && self.sing_box_server.is_none()
    }
}

/// Merge policy for the server override: `rules` only, no keyed arrays. The
/// server has no outbounds to extend by tag, and a wholesale `inbounds`
/// replace is exactly what the protected-field guard is there to catch.
pub fn server_merge_policy() -> json_merge::MergePolicy<'static> {
    json_merge::MergePolicy::default()
}

/// The drop-in files of one override layer, sorted by name so the merge order
/// is the operator's, not the filesystem's.
fn layer_files(directory: &Path, extension: &str) -> Result<Vec<PathBuf>, OverrideError> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(OverrideError::Io(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OverrideError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "override layer path is not a regular directory: {}",
                directory.display()
            ),
        )));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_file()
            && name
                .to_str()
                .is_some_and(|name| name.ends_with(extension) && !name.starts_with('.'))
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn load_json_layered(
    root: &Path,
    base_relative: &str,
    directory_relative: &str,
    policy: &json_merge::MergePolicy<'_>,
) -> Result<Option<serde_json::Value>, OverrideError> {
    let mut documents: Vec<(PathBuf, serde_json::Value)> = Vec::new();
    if let Some(value) = load_json_if_present(&root.join(base_relative))? {
        documents.push((root.join(base_relative), value));
    }
    for path in layer_files(&root.join(directory_relative), ".json")? {
        let value = load_json_if_present(&path)?
            .ok_or_else(|| OverrideError::NotMapping { path: path.clone() })?;
        documents.push((path, value));
    }
    let mut merged: Option<serde_json::Value> = None;
    for (path, document) in documents {
        let (rules_mode, document) = take_rules_mode(document, &path)?;
        if let Some(existing) = merged.as_mut() {
            let scoped = json_merge::MergePolicy {
                rules: rules_mode,
                ..*policy
            };
            json_merge::deep_merge_with(existing, &document, &scoped);
        } else {
            merged = Some(document);
        }
    }
    if merged
        .as_ref()
        .is_some_and(|value| value.as_object().is_some_and(serde_json::Map::is_empty))
    {
        return Ok(None);
    }
    Ok(merged)
}

fn load_yaml_layered(
    root: &Path,
    base_relative: &str,
    directory_relative: &str,
    policy: &json_merge::MergePolicy<'_>,
) -> Result<Option<serde_yaml::Value>, OverrideError> {
    let mut documents: Vec<(PathBuf, serde_yaml::Value)> = Vec::new();
    if let Some(value) = load_yaml_if_present(&root.join(base_relative))? {
        documents.push((root.join(base_relative), value));
    }
    for path in layer_files(&root.join(directory_relative), ".yaml")? {
        let value = load_yaml_if_present(&path)?
            .ok_or_else(|| OverrideError::NotMapping { path: path.clone() })?;
        documents.push((path, value));
    }
    let mut merged: Option<serde_yaml::Value> = None;
    for (path, document) in documents {
        let (rules_mode, document) = take_yaml_rules_mode(document, &path)?;
        if let Some(existing) = merged.as_mut() {
            let scoped = json_merge::MergePolicy {
                rules: rules_mode,
                ..*policy
            };
            json_merge::deep_merge_yaml_with(existing, &document, &scoped);
        } else {
            merged = Some(document);
        }
    }
    Ok(merged)
}

/// Reads and removes the per-document `rules_mode` key.
fn take_rules_mode(
    mut document: serde_json::Value,
    path: &Path,
) -> Result<(json_merge::RulesMode, serde_json::Value), OverrideError> {
    let Some(object) = document.as_object_mut() else {
        return Ok((json_merge::RulesMode::default(), document));
    };
    let Some(value) = object.remove(RULES_MODE_KEY) else {
        return Ok((json_merge::RulesMode::default(), document));
    };
    let text = value.as_str().unwrap_or_default().to_owned();
    match text.as_str() {
        "" | "prepend" => Ok((json_merge::RulesMode::Prepend, document)),
        "append" => Ok((json_merge::RulesMode::Append, document)),
        "replace" => Ok((json_merge::RulesMode::Replace, document)),
        _ => Err(OverrideError::RulesMode {
            path: path.to_path_buf(),
            value: text,
        }),
    }
}

fn take_yaml_rules_mode(
    document: serde_yaml::Value,
    path: &Path,
) -> Result<(json_merge::RulesMode, serde_yaml::Value), OverrideError> {
    let mut mapping = match document {
        serde_yaml::Value::Mapping(mapping) => mapping,
        other => return Ok((json_merge::RulesMode::default(), other)),
    };
    let key = serde_yaml::Value::String(RULES_MODE_KEY.to_owned());
    let Some(value) = mapping.remove(&key) else {
        return Ok((
            json_merge::RulesMode::default(),
            serde_yaml::Value::Mapping(mapping),
        ));
    };
    let text = match &value {
        serde_yaml::Value::String(text) => text.clone(),
        _ => String::new(),
    };
    match text.as_str() {
        "" | "prepend" => Ok((
            json_merge::RulesMode::Prepend,
            serde_yaml::Value::Mapping(mapping),
        )),
        "append" => Ok((
            json_merge::RulesMode::Append,
            serde_yaml::Value::Mapping(mapping),
        )),
        "replace" => Ok((
            json_merge::RulesMode::Replace,
            serde_yaml::Value::Mapping(mapping),
        )),
        _ => Err(OverrideError::RulesMode {
            path: path.to_path_buf(),
            value: text,
        }),
    }
}

/// The credential-bearing fields a server override must not introduce. The
/// server configuration holds every node's uuid, password and Reality private
/// key; letting a template rewrite them would turn an editing convenience into
/// a way to silently break or capture live subscriptions.
pub const PROTECTED_SERVER_FIELDS: &[&str] = &[
    "users",
    "password",
    "private_key",
    "short_id",
    "uuid",
    "certificate",
    "key",
    "ca_certificate_path",
];

/// Rejects a server override that names a protected field anywhere inside
/// `inbounds`. Called before the merge so a bad template aborts the
/// regeneration transaction instead of writing a broken server config.
pub fn reject_protected_server_inbounds(
    override_document: &serde_json::Value,
    path: &Path,
) -> Result<(), OverrideError> {
    let Some(inbounds) = override_document.get("inbounds") else {
        return Ok(());
    };
    // Everything under the `inbounds` key is the credential surface, so the
    // traversal starts already inside it.
    reject_protected_inbound_value(inbounds, path, true)
}

fn reject_protected_inbound_value(
    value: &serde_json::Value,
    path: &Path,
    inside_inbounds: bool,
) -> Result<(), OverrideError> {
    match value {
        serde_json::Value::Object(entries) => {
            for (key, nested) in entries {
                // `inbounds` itself is the entry point and is allowed; anything
                // named inside it is the credential surface.
                if inside_inbounds && PROTECTED_SERVER_FIELDS.contains(&key.as_str()) {
                    return Err(OverrideError::ProtectedField {
                        path: path.to_path_buf(),
                        field: key.clone(),
                    });
                }
                reject_protected_inbound_value(nested, path, inside_inbounds || key == "inbounds")?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items
            .iter()
            .try_for_each(|item| reject_protected_inbound_value(item, path, inside_inbounds)),
        _ => Ok(()),
    }
}

fn load_json_if_present(path: &Path) -> Result<Option<serde_json::Value>, OverrideError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(OverrideError::Io(error)),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| OverrideError::Json {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    match value {
        serde_json::Value::Object(_) => Ok(Some(value)),
        _ => Err(OverrideError::NotMapping {
            path: path.to_path_buf(),
        }),
    }
}

fn load_yaml_if_present(path: &Path) -> Result<Option<serde_yaml::Value>, OverrideError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(OverrideError::Io(error)),
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|error| OverrideError::Yaml {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    match value {
        serde_yaml::Value::Mapping(_) | serde_yaml::Value::Null => Ok(Some(value)),
        _ => Err(OverrideError::NotMapping {
            path: path.to_path_buf(),
        }),
    }
}

/// Deep-merges `overlay` into `base` in place under the frozen ADR-0021
/// semantics. The implementation lives in the `json-merge` crate, which the
/// client engine uses for its own per-profile override too: one set of merge
/// semantics, one implementation. Re-exported here because this module is the
/// server's override vocabulary.
pub use json_merge::{deep_merge, deep_merge_yaml};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The behaviour ADR-0021 locks is asserted through the shared table, not a
    /// copy of it: `json-merge` owns the rows, and this is the server's stake in
    /// them. If the merge semantics move, every consumer's test reddens at once.
    #[test]
    fn the_shared_merge_table_holds_for_json_and_yaml() {
        assert_eq!(json_merge::merge_semantics_failures(), Vec::<&str>::new());
        assert_eq!(
            json_merge::merge_semantics_yaml_failures(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn objects_merge_recursively_and_scalars_replace() {
        let mut base = json!({"log": {"level": "info"}, "keep": true});
        deep_merge(
            &mut base,
            &json!({"log": {"level": "warn", "timestamp": true}, "keep": false}),
        );
        assert_eq!(
            base,
            json!({"log": {"level": "warn", "timestamp": true}, "keep": false})
        );
    }

    #[test]
    fn rules_arrays_are_prepended_including_nested_dns_rules() {
        let mut base = json!({"dns": {"rules": [{"generated": true}]}});
        deep_merge(&mut base, &json!({"dns": {"rules": [{"override": true}]}}));
        assert_eq!(
            base["dns"]["rules"],
            json!([{"override": true}, {"generated": true}])
        );
    }

    #[test]
    fn drop_in_files_merge_in_filename_order_after_the_base_document() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path();
        let overrides = root.join("etc/sbctl/overrides");
        fs::create_dir_all(overrides.join("sing-box.d")).expect("directories are created");
        fs::write(
            root.join(SING_BOX_OVERRIDE_RELATIVE_PATH),
            json!({"log": {"level": "info"}}).to_string(),
        )
        .expect("base override is written");
        fs::write(
            overrides.join("sing-box.d").join("10-second.json"),
            json!({"log": {"timestamp": true}, "route": {"rules": [{"action": "sniff"}]}})
                .to_string(),
        )
        .expect("second layer is written");
        fs::write(
            overrides.join("sing-box.d").join("05-first.json"),
            json!({"log": {"level": "warn"}}).to_string(),
        )
        .expect("first layer is written");

        let overrides = Overrides::load(root).expect("layered overrides load");
        assert!(!overrides.is_empty());
        let sing_box = overrides.sing_box.expect("sing-box override present");
        // 05- sorts before 10-, so `warn` wins over the later layer's absence.
        assert_eq!(sing_box["log"]["level"], "warn");
        assert_eq!(sing_box["log"]["timestamp"], true);
    }

    #[test]
    fn a_rules_mode_of_append_is_consumed_and_not_forwarded() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path();
        let overrides = root.join("etc/sbctl/overrides");
        fs::create_dir_all(&overrides).expect("directory is created");
        fs::write(
            root.join(SING_BOX_OVERRIDE_RELATIVE_PATH),
            json!({"rules_mode": "append", "route": {"rules": [{"domain_suffix": ["later.com"]}]}})
                .to_string(),
        )
        .expect("override is written");

        let loaded = Overrides::load(root).expect("override loads");
        let document = loaded.sing_box.expect("override present");
        assert!(
            document.get(RULES_MODE_KEY).is_none(),
            "the control key must be stripped before it reaches sing-box"
        );

        // The stripped policy still governs how a generated profile merges.
        let mut base = json!({"route": {"rules": [{"action": "sniff"}]}});
        json_merge::deep_merge_with(
            &mut base,
            &document,
            &json_merge::MergePolicy {
                rules: json_merge::RulesMode::Append,
                ..client_policy()
            },
        );
        assert_eq!(base["route"]["rules"][0], json!({"action": "sniff"}));
        assert_eq!(base["route"]["rules"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn an_unknown_rules_mode_is_rejected_with_its_path() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path();
        let overrides = root.join("etc/sbctl/overrides");
        fs::create_dir_all(&overrides).expect("directory is created");
        let path = root.join(SING_BOX_OVERRIDE_RELATIVE_PATH);
        fs::write(&path, json!({"rules_mode": "sideways"}).to_string())
            .expect("override is written");

        assert!(matches!(
            Overrides::load(root),
            Err(OverrideError::RulesMode { .. })
        ));
    }

    #[test]
    fn a_server_override_may_not_reach_inbound_credentials() {
        let path = Path::new("sing-box-server.json");
        for document in [
            json!({"inbounds": [{"users": [{"uuid": "evil"}]}]}),
            json!({"inbounds": [{"tls": {"reality": {"private_key": "x"}}}]}),
            json!({"inbounds": [{"settings": {"password": "x"}}]}),
        ] {
            assert!(
                reject_protected_server_inbounds(&document, path).is_err(),
                "{document} must be refused"
            );
        }
        // Everything outside `inbounds` stays available: log levels, sniffing,
        // DNS strategy and route rules are the legitimate extension surface.
        assert!(
            reject_protected_server_inbounds(
                &json!({"log": {"level": "warn"}, "dns": {"strategy": "ipv4_only"}}),
                path
            )
            .is_ok()
        );
    }

    #[test]
    fn missing_files_are_empty_and_malformed_documents_are_rejected() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path();
        let empty = Overrides::load(root).expect("missing override files load as empty");
        assert!(empty.sing_box.is_none() && empty.clash.is_none());
        assert!(
            empty.is_empty(),
            "a deployment with no override files must not pay for the merged-profile check"
        );

        fs::create_dir_all(root.join("etc/sbctl/overrides"))
            .expect("override directory is created");
        let sing_box = root.join(SING_BOX_OVERRIDE_RELATIVE_PATH);
        fs::write(&sing_box, "{ not json").expect("malformed JSON is written");
        assert!(matches!(
            Overrides::load(root),
            Err(OverrideError::Json { .. })
        ));

        fs::write(&sing_box, "[]").expect("non-mapping JSON is written");
        assert!(matches!(
            Overrides::load(root),
            Err(OverrideError::NotMapping { .. })
        ));

        fs::remove_file(&sing_box).expect("JSON override is removed");
        fs::write(
            root.join(CLASH_OVERRIDE_RELATIVE_PATH),
            "- just\n- a\n- list\n",
        )
        .expect("non-mapping YAML is written");
        assert!(matches!(
            Overrides::load(root),
            Err(OverrideError::NotMapping { .. })
        ));
    }

    #[test]
    fn yaml_rules_are_prepended() {
        let mut base = serde_yaml::from_str("rules:\n  - generated\n").expect("yaml");
        deep_merge_yaml(
            &mut base,
            &serde_yaml::from_str("rules:\n  - override\n").expect("yaml"),
        );
        let text = serde_yaml::to_string(&base).expect("yaml round-trip");
        assert!(text.contains("- override\n- generated"), "{text}");
    }
}
