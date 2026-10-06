//! Operator-owned rule lists: the "add my own split-routing" surface.
//!
//! Files live under `etc/sbctl/rules/` and use the line format the shared
//! rule collections publish (one `TYPE,value[,outbound]` per line, `#`
//! comments), because that is what operators already keep in their own forks:
//!
//! - `direct.list`  → matched traffic bypasses the tunnel;
//! - `proxy.list`   → matched traffic always uses the manual selector group;
//! - `reject.list`  → matched traffic is blocked;
//! - `fakeip-filter.list` → domains that must keep answering with real IPs
//!   instead of fake ones (connectivity probes, NTP, LAN names).
//!
//! The lists are injected as generated rules, so they participate in the same
//! real-kernel check as everything else, and they apply to the full sing-box
//! profiles and both clash artifacts — never to the bare node-list artifact or
//! the URI artifacts, which stay byte-compatible.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::subscription::render::SELECTOR_TAG;

/// The sing-box outbound a rule names to send traffic outside the tunnel, and
/// the pre-1.11 block outbound. Both match the renderer's own vocabulary; a
/// rename there must break this module loudly rather than mis-route silently.
const DIRECT_OUTBOUND: &str = "direct";
const LEGACY_BLOCK_OUTBOUND: &str = "block-out";

/// Match types accepted in a list line. Clash's `GEOSITE`/`RULE-SET` words are
/// deliberately absent: those need a rule-set the operator has to register
/// separately, and pretending a bare name resolves would silently mis-route.
pub const SUPPORTED_MATCHERS: &[&str] = &["DOMAIN-SUFFIX", "DOMAIN", "DOMAIN-KEYWORD", "IP-CIDR"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Direct,
    Proxy,
    Reject,
    FakeIpFilter,
}

impl RuleKind {
    pub const ALL: [RuleKind; 4] = [
        RuleKind::Direct,
        RuleKind::Proxy,
        RuleKind::Reject,
        RuleKind::FakeIpFilter,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            RuleKind::Direct => "direct.list",
            RuleKind::Proxy => "proxy.list",
            RuleKind::Reject => "reject.list",
            RuleKind::FakeIpFilter => "fakeip-filter.list",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RuleKind::Direct => "直连",
            RuleKind::Proxy => "走代理",
            RuleKind::Reject => "阻断",
            RuleKind::FakeIpFilter => "fake-ip 例外",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuleListError {
    #[error("{file} line {line}: {message}")]
    Invalid {
        file: String,
        line: usize,
        message: String,
    },
    #[error("rule list read failed: {0}")]
    Io(#[from] std::io::Error),
}

/// A parsed entry: the matcher word, the value, and for IP-CIDR whether
/// `no-resolve` was requested (the client must not look up the domain of a
/// destination that is being matched by address).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleEntry {
    pub matcher: String,
    pub value: String,
    pub no_resolve: bool,
}

/// The four operator lists, in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleLists {
    pub direct: Vec<RuleEntry>,
    pub proxy: Vec<RuleEntry>,
    pub reject: Vec<RuleEntry>,
    pub fake_ip_filter: Vec<RuleEntry>,
}

impl RuleLists {
    /// Reads `etc/sbctl/rules/*.list`. Missing files are empty lists; a
    /// malformed line aborts generation so the transaction keeps the previous
    /// known-good artifacts instead of serving half a routing policy.
    pub fn load(root: &Path) -> Result<Self, RuleListError> {
        let directory = root.join("etc/sbctl/rules");
        let mut lists = RuleLists::default();
        for kind in RuleKind::ALL {
            let entries = load_list(&directory.join(kind.file_name()))?;
            match kind {
                RuleKind::Direct => lists.direct = entries,
                RuleKind::Proxy => lists.proxy = entries,
                RuleKind::Reject => lists.reject = entries,
                RuleKind::FakeIpFilter => lists.fake_ip_filter = entries,
            }
        }
        Ok(lists)
    }

    pub fn is_empty(&self) -> bool {
        self.direct.is_empty()
            && self.proxy.is_empty()
            && self.reject.is_empty()
            && self.fake_ip_filter.is_empty()
    }

    /// The entries of one list, so the CLI can read and rewrite a single file
    /// without re-discovering which field holds it.
    pub fn entries_for(&self, kind: RuleKind) -> &[RuleEntry] {
        match kind {
            RuleKind::Direct => &self.direct,
            RuleKind::Proxy => &self.proxy,
            RuleKind::Reject => &self.reject,
            RuleKind::FakeIpFilter => &self.fake_ip_filter,
        }
    }

    /// The sing-box route rules these lists add, in the order the operator
    /// wrote them: reject first (blocking wins over a later allow), then
    /// explicit proxy, then explicit direct. Injected in front of the
    /// generated verdicts by the same prepend rule the overrides use.
    /// `legacy_block` is the pre-1.11 shape, where route rule actions do not
    /// exist and a block is an outbound reference instead; the caller adds that
    /// outbound to the profile, because a rule naming it would otherwise dangle.
    pub fn sing_box_rules(&self, legacy_block: bool) -> Vec<Value> {
        let mut rules = Vec::new();
        for (entries, verdict) in [
            (
                &self.reject,
                if legacy_block {
                    json!({"outbound": LEGACY_BLOCK_OUTBOUND})
                } else {
                    json!({"action": "reject"})
                },
            ),
            (&self.proxy, json!({"outbound": SELECTOR_TAG})),
            (&self.direct, json!({"outbound": DIRECT_OUTBOUND})),
        ] {
            for entry in entries {
                let Value::Object(mut rule) = verdict.clone() else {
                    unreachable!("a verdict is built from an object")
                };
                let (field, values) = match entry.matcher.as_str() {
                    "DOMAIN-SUFFIX" => ("domain_suffix", json!([entry.value])),
                    "DOMAIN" => ("domain", json!([entry.value])),
                    "DOMAIN-KEYWORD" => ("domain_keyword", json!([entry.value])),
                    // `no-resolve` is a clash line suffix. sing-box route rules
                    // carry no such field in any version this tool supports, so
                    // emitting one would fail the real-kernel check.
                    "IP-CIDR" => ("ip_cidr", json!([entry.value])),
                    other => unreachable!("validated matcher: {other}"),
                };
                rule.insert(field.to_owned(), values);
                rules.push(Value::Object(rule));
            }
        }
        rules
    }

    /// The clash `rules:` lines these lists add, in the same order.
    pub fn clash_rules(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (entries, target) in [
            (&self.reject, "REJECT"),
            (&self.proxy, SELECTOR_TAG),
            (&self.direct, "DIRECT"),
        ] {
            for entry in entries {
                let suffix = if entry.matcher == "IP-CIDR" && entry.no_resolve {
                    ",no-resolve"
                } else {
                    ""
                };
                lines.push(format!(
                    "{},{},{}{suffix}",
                    entry.matcher, entry.value, target
                ));
            }
        }
        lines
    }

    /// Extends the generated fake-ip exception rule so the operator's names
    /// keep answering with real IPs.
    ///
    /// The rule is *found* rather than rebuilt: every sing-box version renders
    /// one DNS rule whose `domain_suffix` carries the built-in connectivity and
    /// NTP suffixes, and its server tag differs between the typed and legacy
    /// shapes. Appending there is what keeps this working for 1.10 through 1.14
    /// without hard-coding a resolver tag.
    pub fn extend_fake_ip_dns_rule(&self, profile: &mut Value) -> bool {
        if self.fake_ip_filter.is_empty() {
            return false;
        }
        let Some(rules) = profile
            .get_mut("dns")
            .and_then(Value::as_object_mut)
            .and_then(|dns| dns.get_mut("rules"))
            .and_then(Value::as_array_mut)
        else {
            return false;
        };
        // "lan" is one of the built-in suffixes, so exactly one rule matches.
        let Some(target) = rules.iter_mut().find(|rule| {
            rule.get("domain_suffix")
                .and_then(Value::as_array)
                .is_some_and(|list| list.iter().any(|value| value.as_str() == Some("lan")))
        }) else {
            return false;
        };
        let Value::Object(target) = target else {
            return false;
        };
        let existing = target
            .entry("domain_suffix")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("checked above")
            .clone();
        let mut merged = existing;
        for entry in &self.fake_ip_filter {
            if entry.matcher != "DOMAIN-SUFFIX" {
                continue;
            }
            let value = json!(entry.value);
            if !merged.contains(&value) {
                merged.push(value);
            }
        }
        target.insert("domain_suffix".to_owned(), Value::Array(merged));
        true
    }

    /// The clash `fake-ip-filter` additions (the list already carries the
    /// built-in connectivity and NTP suffixes).
    pub fn clash_fake_ip_filter(&self) -> Vec<String> {
        self.fake_ip_filter
            .iter()
            .map(|entry| match entry.matcher.as_str() {
                "DOMAIN-SUFFIX" => format!("+.{0}", entry.value),
                "DOMAIN-KEYWORD" => format!("*{}*", entry.value),
                _ => entry.value.clone(),
            })
            .collect()
    }
}

fn load_list(path: &Path) -> Result<Vec<RuleEntry>, RuleListError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(RuleListError::Io(error)),
    };
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut entries = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let (matcher, value) = line.split_once(',').ok_or_else(|| RuleListError::Invalid {
            file: file.clone(),
            line: index + 1,
            message: "expected TYPE,value, for example DOMAIN-SUFFIX,example.com".to_owned(),
        })?;
        let matcher = matcher.trim().to_ascii_uppercase();
        let value = value.trim();
        // A trailing clash-style policy word is accepted and ignored: the file
        // decides the policy by which list it is in, and operators copy lines
        // straight out of clash configs.
        let value = value
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let no_resolve = line
            .split(',')
            .skip(2)
            .any(|part| part.trim() == "no-resolve");
        if !SUPPORTED_MATCHERS.contains(&matcher.as_str()) {
            return Err(RuleListError::Invalid {
                file,
                line: index + 1,
                message: format!(
                    "unsupported match type {matcher:?}; expected one of {}",
                    SUPPORTED_MATCHERS.join("/")
                ),
            });
        }
        if value.is_empty() {
            return Err(RuleListError::Invalid {
                file,
                line: index + 1,
                message: "the value after the comma is empty".to_owned(),
            });
        }
        if matcher == "IP-CIDR" && !is_cidr(&value) {
            return Err(RuleListError::Invalid {
                file,
                line: index + 1,
                message: format!("{value:?} is not an IPv4 or IPv6 CIDR prefix"),
            });
        }
        let entry = RuleEntry {
            matcher,
            value,
            no_resolve,
        };
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// A CIDR prefix with a numeric octet/hextet body and a 0-128 prefix length.
fn is_cidr(value: &str) -> bool {
    let Some((address, prefix)) = value.rsplit_once('/') else {
        return false;
    };
    let Ok(bits) = prefix.parse::<u8>() else {
        return false;
    };
    if address.contains(':') {
        bits <= 128
            && !address
                .split(':')
                .any(|part| part.is_empty() && address != "::")
    } else {
        bits <= 32
            && address.split('.').count() == 4
            && address.split('.').all(|part| part.parse::<u8>().is_ok())
    }
}

/// Renders one entry the way an operator wrote it, for `sbctl rule list`.
impl std::fmt::Display for RuleEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.no_resolve {
            write!(formatter, "{},{}(no-resolve)", self.matcher, self.value)
        } else {
            write!(formatter, "{},{}", self.matcher, self.value)
        }
    }
}

/// Validates a domain value before it is written into a list, so `rule add`
/// cannot store a line the loader would later reject.
pub fn validate_entry(matcher: &str, value: &str) -> Result<(), String> {
    let matcher = matcher.to_ascii_uppercase();
    if !SUPPORTED_MATCHERS.contains(&matcher.as_str()) {
        return Err(format!(
            "unsupported match type {matcher:?}; expected one of {}",
            SUPPORTED_MATCHERS.join("/")
        ));
    }
    if value.trim().is_empty() {
        return Err("the value is empty".to_owned());
    }
    if matcher == "IP-CIDR" && !is_cidr(value.trim()) {
        return Err(format!("{value:?} is not an IPv4 or IPv6 CIDR prefix"));
    }
    if value.contains(',') || value.chars().any(char::is_whitespace) {
        return Err("the value may not contain commas or whitespace".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_list(root: &Path, kind: RuleKind, contents: &str) {
        let directory = root.join("etc/sbctl/rules");
        fs::create_dir_all(&directory).expect("rules directory is created");
        fs::write(directory.join(kind.file_name()), contents).expect("list is written");
    }

    #[test]
    fn missing_files_are_empty_and_the_list_can_be_distinguished() {
        let directory = tempfile::tempdir().expect("temporary root");
        let lists = RuleLists::load(directory.path()).expect("missing lists are empty");
        assert!(lists.is_empty());
        assert!(lists.sing_box_rules(false).is_empty());
    }

    #[test]
    fn comments_blank_lines_and_trailing_policy_words_are_tolerated() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::Proxy,
            "# my additions\n\nDOMAIN-SUFFIX,example.com,节点选择\n  IP-CIDR,10.0.0.0/8,no-resolve  \n",
        );
        let lists = RuleLists::load(directory.path()).expect("list loads");
        assert_eq!(lists.proxy.len(), 2);
        assert_eq!(lists.proxy[0].matcher, "DOMAIN-SUFFIX");
        assert_eq!(lists.proxy[0].value, "example.com");
        assert!(
            lists.proxy[1].no_resolve,
            "the no-resolve flag survives the trailing policy word"
        );
    }

    #[test]
    fn duplicates_are_collapsed_so_one_entry_cannot_double_a_rule() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::Direct,
            "DOMAIN-SUFFIX,example.test\nDOMAIN-SUFFIX,example.test\n",
        );
        let lists = RuleLists::load(directory.path()).expect("list loads");
        assert_eq!(lists.direct.len(), 1);
    }

    #[test]
    fn sing_box_rules_carry_the_right_field_and_group_per_list() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::Reject,
            "DOMAIN-SUFFIX,ads.example\n",
        );
        write_list(
            directory.path(),
            RuleKind::Proxy,
            "DOMAIN,api.openai.example\n",
        );
        write_list(
            directory.path(),
            RuleKind::Direct,
            "IP-CIDR,192.168.0.0/16\n",
        );
        let lists = RuleLists::load(directory.path()).expect("lists load");

        let rules = lists.sing_box_rules(false);
        assert_eq!(rules.len(), 3, "reject, proxy, direct");
        // sing-box 1.11+ blocks with a route action, not an outbound: an
        // `outbound: reject` here names something no profile declares.
        assert_eq!(rules[0]["action"], "reject");
        assert!(rules[0].get("outbound").is_none());
        assert_eq!(rules[0]["domain_suffix"], json!(["ads.example"]));
        assert_eq!(rules[1]["outbound"], SELECTOR_TAG);
        assert_eq!(rules[1]["domain"], json!(["api.openai.example"]));
        assert_eq!(rules[2]["outbound"], "direct");
        assert_eq!(rules[2]["ip_cidr"], json!(["192.168.0.0/16"]));

        // Pre-1.11 has no rule actions, so the same list blocks through the
        // `block-out` outbound the renderer declares for that profile only.
        let legacy = lists.sing_box_rules(true);
        assert_eq!(legacy[0]["outbound"], "block-out");
        assert!(legacy[0].get("action").is_none());

        // `no-resolve` is a clash suffix; sing-box route rules never carried it.
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::Direct,
            "IP-CIDR,10.0.0.0/8,no-resolve
",
        );
        let lists = RuleLists::load(directory.path()).expect("list loads");
        let rules = lists.sing_box_rules(false);
        assert_eq!(rules.len(), 1);
        assert!(
            rules[0].get("skip_resolve").is_none() && rules[0].get("skip_resolves").is_none(),
            "a sing-box rule must not grow a field the core would reject: {}",
            rules[0]
        );
        assert_eq!(
            lists.clash_rules()[0],
            "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
            "the clash form does carry the suffix"
        );
    }

    #[test]
    fn clash_rules_use_the_clash_target_names() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::Reject,
            "DOMAIN-SUFFIX,ads.example\n",
        );
        write_list(
            directory.path(),
            RuleKind::Direct,
            "IP-CIDR,10.0.0.0/8,no-resolve\n",
        );
        let lists = RuleLists::load(directory.path()).expect("lists load");

        let lines = lists.clash_rules();
        assert_eq!(
            lines,
            vec![
                "DOMAIN-SUFFIX,ads.example,REJECT".to_owned(),
                "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve".to_owned(),
            ]
        );
    }

    #[test]
    fn fake_ip_filter_entries_reach_both_formats() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::FakeIpFilter,
            "DOMAIN-SUFFIX,music.example\n",
        );
        let lists = RuleLists::load(directory.path()).expect("lists load");

        assert_eq!(lists.clash_fake_ip_filter(), vec!["+.music.example"]);
        let mut profile = json!({"dns": {"rules": [
            {"domain_suffix": ["lan", "local"], "server": "dns-direct"},
            {"query_type": ["A", "AAAA"], "server": "dns-fakeip"}
        ]}});
        assert!(
            lists.extend_fake_ip_dns_rule(&mut profile),
            "the generated exception rule must be found and extended"
        );
        let rules = profile["dns"]["rules"].as_array().expect("rules");
        assert_eq!(
            rules[0]["domain_suffix"],
            json!(["lan", "local", "music.example"]),
            "the operator's name joins the built-in suffixes in place"
        );
        assert_eq!(
            rules[1],
            json!({"query_type": ["A", "AAAA"], "server": "dns-fakeip"}),
            "only the exception rule is touched"
        );
    }

    #[test]
    fn a_profile_without_the_generated_exception_rule_is_left_alone() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(
            directory.path(),
            RuleKind::FakeIpFilter,
            "DOMAIN-SUFFIX,music.example\n",
        );
        let lists = RuleLists::load(directory.path()).expect("lists load");
        let mut profile = json!({"dns": {"rules": []}});
        assert!(
            !lists.extend_fake_ip_dns_rule(&mut profile),
            "no matching rule means no silent rewrite of the operator's intent"
        );
    }

    #[test]
    fn an_unsupported_match_type_or_bad_cidr_names_the_line() {
        let directory = tempfile::tempdir().expect("temporary root");
        write_list(directory.path(), RuleKind::Direct, "# ok\nGEOSITE,cn\n");
        let error = RuleLists::load(directory.path()).expect_err("GEOSITE is refused");
        assert!(error.to_string().contains("direct.list line 2"), "{error}");

        let directory = tempfile::tempdir().expect("temporary root");
        write_list(directory.path(), RuleKind::Direct, "IP-CIDR,not-a-prefix\n");
        let error = RuleLists::load(directory.path()).expect_err("a bad CIDR is refused");
        assert!(
            error.to_string().contains("not an IPv4 or IPv6 CIDR"),
            "{error}"
        );
    }

    #[test]
    fn validate_entry_rejects_what_the_loader_would_reject() {
        assert!(validate_entry("DOMAIN-SUFFIX", "example.com").is_ok());
        assert!(validate_entry("domain-suffix", "example.com").is_ok());
        assert!(validate_entry("IP-CIDR", "10.0.0.0/8").is_ok());
        assert!(validate_entry("IP-CIDR", "10.0.0.0").is_err());
        assert!(validate_entry("RULE-SET", "cn").is_err());
        assert!(validate_entry("DOMAIN", "has space").is_err());
    }
}
