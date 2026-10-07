//! `sbctl rule`: the operator-facing editor for the split-routing lists.
//!
//! The files under `etc/sbctl/rules/` are plain text on purpose (they are the
//! same `TYPE,value` lines shared rule collections publish), but appending by
//! hand through a shell redirect is how a stray duplicate or a typo'd match
//! type reaches a running deployment. This verb validates before writing,
//! deduplicates, and regenerates the subscription artifacts in the same
//! command, so `sbctl rule add` is the whole operation.

use crate::cli::args::RuleCommand;
use crate::cli::commands::config::regenerate;
use sbctl::rule_list::{RuleEntry, RuleKind, RuleLists, validate_entry};
use std::fs;
use std::io::Write;
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
        RuleCommand::Add { kind, entries } => add(root, kind.into(), &entries, sing_box_bin),
        RuleCommand::Remove { kind, entries } => remove(root, kind.into(), &entries, sing_box_bin),
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
/// `DOMAIN-SUFFIX`, the match operators almost always want).
fn parse_entry(text: &str) -> Result<(String, RuleEntry), String> {
    let (matcher, value) = match text.split_once(',') {
        Some((matcher, value)) => (matcher.trim().to_ascii_uppercase(), value.trim()),
        None => ("DOMAIN-SUFFIX".to_owned(), text.trim()),
    };
    validate_entry(&matcher, value)?;
    let entry = RuleEntry {
        matcher: matcher.clone(),
        value: value.to_owned(),
        no_resolve: matcher == "IP-CIDR" && value.contains('/'),
    };
    Ok((matcher, entry))
}

fn write_list(kind: RuleKind, entries: &[RuleEntry], path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // Write through a temporary file in the same directory: a rule list the
    // generator is about to read must never be observed half-written.
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
    writeln!(
        file,
        "# sbctl 规则列表：{kind}（TYPE,value 每行一条，# 为注释）",
        kind = kind.file_name()
    )
    .map_err(|error| error.to_string())?;
    for entry in entries {
        writeln!(file, "{entry}").map_err(|error| error.to_string())?;
    }
    drop(file);
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    Ok(())
}

fn add(root: &Path, kind: RuleKind, texts: &[String], sing_box_bin: Option<PathBuf>) -> ExitCode {
    let path = list_path(root, kind);
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
    if let Err(message) = write_list(kind, &entries, &path) {
        eprintln!("写入 {} 失败：{message}", kind.file_name());
        return ExitCode::from(2);
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
    regenerate(root, sing_box_bin)
}

fn remove(
    root: &Path,
    kind: RuleKind,
    texts: &[String],
    sing_box_bin: Option<PathBuf>,
) -> ExitCode {
    let path = list_path(root, kind);
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
    if let Err(message) = write_list(kind, &kept, &path) {
        eprintln!("写入 {} 失败：{message}", kind.file_name());
        return ExitCode::from(2);
    }
    println!("已从 {} 移除：{}", kind.file_name(), removed.join(", "));
    regenerate(root, sing_box_bin)
}
