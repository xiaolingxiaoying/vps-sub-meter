//! Editor discovery for `sbctl config override edit`. The administrator's
//! explicit choice (`$VISUAL`, then `$EDITOR`) wins; the stock chain prefers
//! vim, falls back to nano, and ends at the POSIX baseline `vi`.

/// Candidate editors in priority order: the explicit choices first, then the
/// stock chain. Duplicates and blank values are dropped so an `$EDITOR` that
/// names a stock editor cannot list it twice.
pub(crate) fn editor_candidates_from(
    visual: Option<String>,
    editor: Option<String>,
) -> Vec<String> {
    let mut candidates = Vec::new();
    for choice in visual.into_iter().chain(editor) {
        let choice = choice.trim().to_owned();
        if !choice.is_empty() && !candidates.contains(&choice) {
            candidates.push(choice);
        }
    }
    for fallback in stock_editors() {
        let fallback = (*fallback).to_owned();
        if !candidates.contains(&fallback) {
            candidates.push(fallback);
        }
    }
    candidates
}

fn stock_editors() -> &'static [&'static str] {
    if cfg!(windows) {
        &["notepad"]
    } else {
        &["vim", "nano", "vi"]
    }
}

pub(crate) fn editor_candidates() -> Vec<String> {
    editor_candidates_from(std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok())
}

/// Runs the first candidate the host can spawn. A binary that cannot be
/// spawned (typically "not installed") falls through to the next candidate;
/// an editor that starts and exits non-zero is the user's own editor failing,
/// so its status is surfaced instead of silently retrying with the next one.
pub(crate) fn run_editor(
    candidates: &[String],
    path: &std::path::Path,
) -> Result<std::process::ExitStatus, String> {
    let mut missing = Vec::new();
    for editor in candidates {
        match std::process::Command::new(editor).arg(path).status() {
            Ok(status) => return Ok(status),
            Err(error) => {
                eprintln!("编辑器 {editor} 不可用（{error}），尝试下一个候选。");
                missing.push(editor.clone());
            }
        }
    }
    Err(format!(
        "找不到可启动的编辑器（尝试过 {}）。可设置 EDITOR 环境变量指定。",
        missing.join("、")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_choices_win_over_the_stock_chain_in_order() {
        let candidates = editor_candidates_from(
            Some("visual-editor".into()),
            Some("editor-of-choice".into()),
        );
        let expected = ["visual-editor", "editor-of-choice"]
            .into_iter()
            .map(str::to_owned)
            .chain(stock_editors().iter().map(|editor| (*editor).to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(candidates, expected);
    }

    #[test]
    fn blank_and_duplicate_choices_are_dropped() {
        let candidates = editor_candidates_from(Some("   ".into()), Some("vim".into()));
        assert_eq!(candidates.first().map(String::as_str), Some("vim"));
        assert_eq!(
            candidates.iter().filter(|c| c.as_str() == "vim").count(),
            1,
            "a stock editor named by $EDITOR must not appear twice in the chain"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_stock_chain_prefers_vim_then_nano_then_vi() {
        assert_eq!(editor_candidates_from(None, None), ["vim", "nano", "vi"]);
    }
}
