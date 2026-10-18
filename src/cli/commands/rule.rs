//! `sbctl rule`: the operator-facing editor for the split-routing lists.
//!
//! The files under `etc/sbctl/rules/` are plain text on purpose (they are the
//! same `TYPE,value` lines shared rule collections publish), but appending by
//! hand through a shell redirect is how a stray duplicate or a typo'd match
//! type reaches a running deployment. This verb validates before writing,
//! deduplicates, and regenerates the subscription artifacts in the same
//! command, so `sbctl rule add` is the whole operation.

use crate::cli::args::RuleCommand;
use crate::cli::commands::config::regenerate_locked;
use sbctl::rule_list::{RuleEntry, RuleKind, RuleLists, validate_entry};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn list_path(root: &Path, kind: RuleKind) -> PathBuf {
    root.join("etc/sbctl/rules").join(kind.file_name())
}

pub(crate) fn run_rule(
    root: &Path,
    command: RuleCommand,
    sing_box_bin: Option<PathBuf>,
) -> ExitCode {
    match command {
        RuleCommand::List { kind } => list(root, kind.map(Into::into)),
        RuleCommand::Add { kind, entries } => with_list_transaction(root, kind.into(), || {
            add(root, kind.into(), &entries, sing_box_bin)
        }),
        RuleCommand::Remove { kind, entries } => with_list_transaction(root, kind.into(), || {
            remove(root, kind.into(), &entries, sing_box_bin)
        }),
    }
}

fn kinds_for(kind: Option<RuleKind>) -> Vec<RuleKind> {
    kind.map(|one| vec![one]).unwrap_or(RuleKind::ALL.to_vec())
}

fn list(root: &Path, kind: Option<RuleKind>) -> ExitCode {
    match RuleLists::load(root) {
        Ok(lists) => {
            for selected in kinds_for(kind) {
                let entries = lists.entries_for(selected);
                println!(
                    "{}（{}）:",
                    selected.label(),
                    list_path(root, selected).display()
                );
                if entries.is_empty() {
                    println!("  （空）");
                }
                for entry in entries {
                    println!("  {entry}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("规则列表读取失败：{error}");
            ExitCode::from(2)
        }
    }
}

/// Accepts `DOMAIN-SUFFIX,example.com` or the bare value (which means
/// `DOMAIN-SUFFIX`, the match operators almost always want). A trailing Clash
/// policy word (e.g. `,DIRECT`) and `,no-resolve` are accepted.
fn parse_entry(text: &str) -> Result<(String, RuleEntry), String> {
    let mut parts = text.split(',');
    let first = parts.next().unwrap_or_default().trim();
    let (matcher, value) = match parts.next() {
        Some(second) => (first.to_ascii_uppercase(), second.trim()),
        None => ("DOMAIN-SUFFIX".to_owned(), first),
    };
    validate_entry(&matcher, value)?;
    // IP-CIDR operator rules default to no-resolve (address matches should
    // bypass client domain lookups), and an explicit `,no-resolve` or trailing
    // Clash policy word (e.g. `,DIRECT`) is accepted.
    let no_resolve = matcher == "IP-CIDR";
    let entry = RuleEntry {
        matcher: matcher.clone(),
        value: value.to_owned(),
        no_resolve,
    };
    Ok((matcher, entry))
}

fn write_list(root: &Path, kind: RuleKind, entries: &[RuleEntry]) -> Result<(), String> {
    let mut contents = format!(
        "# sbctl 规则列表：{kind}（TYPE,value 每行一条，# 为注释）",
        kind = kind.file_name()
    );
    contents.push('\n');
    for entry in entries {
        contents.push_str(&format!("{entry}\n"));
    }
    sbctl::config::DeploymentStore::new(root)
        .write_relative_locked(
            &format!("etc/sbctl/rules/{}", kind.file_name()),
            contents.as_bytes(),
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Serialize the complete read/modify/check operation and restore the original
/// source bytes when generation rejects a change.
pub(crate) fn with_list_transaction(
    root: &Path,
    kind: RuleKind,
    change: impl FnOnce() -> ExitCode,
) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let _lock = match store.acquire_operation_lock() {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("规则事务失败：{error}");
            return ExitCode::from(2);
        }
    };
    let path = list_path(root, kind);
    let prior = match sbctl::config::read_managed_file_no_follow(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            eprintln!("规则列表读取失败：{error}");
            return ExitCode::from(2);
        }
    };
    let result = change();
    if result != ExitCode::SUCCESS {
        let restored = match prior {
            Some(bytes) => store
                .write_relative_locked(&format!("etc/sbctl/rules/{}", kind.file_name()), &bytes)
                .map_err(|error| error.to_string()),
            None => match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            },
        };
        if let Err(error) = restored {
            eprintln!("规则列表回滚失败：{error}");
        }
    }
    result
}

fn add(root: &Path, kind: RuleKind, texts: &[String], sing_box_bin: Option<PathBuf>) -> ExitCode {
    let mut entries = match RuleLists::load(root) {
        Ok(lists) => lists.entries_for(kind).to_vec(),
        Err(error) => {
            eprintln!("规则列表读取失败：{error}");
            return ExitCode::from(2);
        }
    };
    let mut added = Vec::new();
    for text in texts {
        match parse_entry(text) {
            Ok((_, entry)) if !entries.contains(&entry) => {
                entries.push(entry.clone());
                added.push(entry);
            }
            Ok(_) => println!("{text} 已存在于 {}，未重复添加", kind.file_name()),
            Err(message) => {
                eprintln!("拒绝 {text}：{message}");
                return ExitCode::from(2);
            }
        }
    }
    if added.is_empty() {
        return ExitCode::SUCCESS;
    }
    if let Err(message) = write_list(root, kind, &entries) {
        eprintln!("写入 {} 失败：{message}", kind.file_name());
        return ExitCode::from(2);
    }
    let result = regenerate_locked(root, sing_box_bin);
    if result != ExitCode::SUCCESS {
        return result;
    }
    println!(
        "已写入 {}：{}",
        kind.file_name(),
        added
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    result
}

fn remove(
    root: &Path,
    kind: RuleKind,
    texts: &[String],
    sing_box_bin: Option<PathBuf>,
) -> ExitCode {
    let entries = match RuleLists::load(root) {
        Ok(lists) => lists.entries_for(kind).to_vec(),
        Err(error) => {
            eprintln!("规则列表读取失败：{error}");
            return ExitCode::from(2);
        }
    };
    let mut removed = Vec::new();
    let mut kept = entries.clone();
    for text in texts {
        let (_, target) = match parse_entry(text) {
            Ok(parsed) => parsed,
            Err(message) => {
                eprintln!("拒绝 {text}：{message}");
                return ExitCode::from(2);
            }
        };
        // Match on the value so `sbctl rule remove direct example.com` clears
        // the entry whatever match type it was written with.
        let before = kept.len();
        kept.retain(|entry| entry.value != target.value);
        if kept.len() != before {
            removed.push(target.value);
        } else {
            println!("{} 中不存在 {text}", kind.file_name());
        }
    }
    if removed.is_empty() {
        return ExitCode::SUCCESS;
    }
    if let Err(message) = write_list(root, kind, &kept) {
        eprintln!("写入 {} 失败：{message}", kind.file_name());
        return ExitCode::from(2);
    }
    let result = regenerate_locked(root, sing_box_bin);
    if result == ExitCode::SUCCESS {
        println!("已从 {} 移除：{}", kind.file_name(), removed.join(", "));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entry_supports_bare_values_and_typed_matchers() {
        let (matcher, entry) = parse_entry("example.com").expect("valid bare entry");
        assert_eq!(matcher, "DOMAIN-SUFFIX");
        assert_eq!(entry.value, "example.com");
        assert!(!entry.no_resolve);

        let (matcher, entry) = parse_entry("DOMAIN,example.com").expect("valid DOMAIN entry");
        assert_eq!(matcher, "DOMAIN");
        assert_eq!(entry.value, "example.com");
        assert!(!entry.no_resolve);

        let (matcher, entry) = parse_entry("DOMAIN-KEYWORD,google").expect("valid keyword entry");
        assert_eq!(matcher, "DOMAIN-KEYWORD");
        assert_eq!(entry.value, "google");
        assert!(!entry.no_resolve);
    }

    #[test]
    fn parse_entry_handles_trailing_clash_policy_and_no_resolve() {
        // Clash-style policy trailing word is accepted and dropped
        let (matcher, entry) =
            parse_entry("DOMAIN-SUFFIX,google.com,DIRECT").expect("valid clash rule");
        assert_eq!(matcher, "DOMAIN-SUFFIX");
        assert_eq!(entry.value, "google.com");
        assert!(!entry.no_resolve);

        // IP-CIDR defaults to no_resolve = true
        let (matcher, entry) = parse_entry("IP-CIDR,192.168.1.0/24").expect("valid IP-CIDR");
        assert_eq!(matcher, "IP-CIDR");
        assert_eq!(entry.value, "192.168.1.0/24");
        assert!(entry.no_resolve);

        // IP-CIDR with no-resolve flag sets no_resolve = true
        let (matcher, entry) =
            parse_entry("IP-CIDR,10.0.0.0/8,no-resolve").expect("valid no-resolve CIDR");
        assert_eq!(matcher, "IP-CIDR");
        assert_eq!(entry.value, "10.0.0.0/8");
        assert!(entry.no_resolve);

        // IP-CIDR with policy and no-resolve
        let (matcher, entry) =
            parse_entry("IP-CIDR,172.16.0.0/12,DIRECT,no-resolve").expect("valid combo");
        assert_eq!(matcher, "IP-CIDR");
        assert_eq!(entry.value, "172.16.0.0/12");
        assert!(entry.no_resolve);
    }

    #[test]
    fn parse_entry_rejects_malformed_inputs() {
        assert!(parse_entry("").is_err());
        assert!(parse_entry("  ").is_err());
        assert!(parse_entry("UNKNOWN-TYPE,example.com").is_err());
        assert!(parse_entry("IP-CIDR,not-a-cidr").is_err());
        assert!(parse_entry("IP-CIDR,1.1.1.1/33").is_err());
    }
}
