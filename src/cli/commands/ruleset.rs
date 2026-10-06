//! `sbctl rule-set`: register remote rule-sets, or import a plain-text list.
//!
//! Two different things live under "add a rule source", and conflating them is
//! how a subscription ends up referencing a file no core can parse:
//!
//! - `add` registers a **compiled rule-set URL** (`.srs` for sing-box, `.mrs`
//!   for mihomo). It is rendered into every full client profile and both clash
//!   artifacts, so subscribers download it themselves on their own schedule.
//! - `import-list` fetches a **plain-text list** (the `DOMAIN-SUFFIX,x` line
//!   format shared rule collections publish) and appends it to one of the
//!   operator lists under `etc/sbctl/rules/`. The conversion happens here, at
//!   the command line, because generation must stay free of network access for
//!   the artifacts to remain reproducible.

use crate::cli::commands::config::regenerate;
use sbctl::rule_list::{RuleKind, RuleLists, validate_entry};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuleSetTarget {
    Direct,
    Proxy,
    Reject,
}

impl RuleSetTarget {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Proxy => "proxy",
            Self::Reject => "reject",
        }
    }

    /// The operator list a text import lands in.
    fn rule_kind(self) -> RuleKind {
        match self {
            Self::Direct => RuleKind::Direct,
            Self::Proxy => RuleKind::Proxy,
            Self::Reject => RuleKind::Reject,
        }
    }
}

/// Validates a registration before it is stored: an absolute https URL with no
/// whitespace, and the compiled form the subscribers' cores actually accept.
pub(crate) fn validate_rule_set_url(url: &str) -> Result<(), String> {
    if url.contains(char::is_whitespace) {
        return Err("URL 不能包含空白字符".to_owned());
    }
    if !url.starts_with("https://") {
        // Subscribers fetch this URL themselves, from whatever network they are
        // on. A plain-http rule-set can be replaced in flight by anyone between
        // the CDN and the client, and the payload steers all their traffic.
        return Err("规则集 URL 必须是 https://（订阅端会自行下载它）".to_owned());
    }
    let name = url
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !name.ends_with(".srs") && !name.ends_with(".mrs") {
        return Err(
            "规则集 URL 必须以 .srs（sing-box）或 .mrs（mihomo）结尾；文本列表请用 \
             sbctl rule-set import-list"
                .to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn list(root: &Path) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    match store.load() {
        Ok(config) => {
            if config.client_extra_rule_sets.is_empty() {
                println!("（未注册远程规则集）");
            }
            for set in &config.client_extra_rule_sets {
                println!("{}  ->  {}  [{}]", set.name, set.outbound, set.url);
            }
            println!("\n刷新间隔: {}", config.client_rule_set_update_interval);
            match RuleLists::load(root) {
                Ok(lists) => {
                    for kind in RuleKind::ALL {
                        let entries = lists.entries_for(kind);
                        if !entries.is_empty() {
                            println!(
                                "{}（{}）: {}",
                                kind.label(),
                                kind.file_name(),
                                entries.len()
                            );
                        }
                    }
                }
                Err(error) => eprintln!("本地规则列表读取失败：{error}"),
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rule-set list failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// Registers (or re-points) one named rule-set and regenerates the artifacts.
pub(crate) fn add(
    root: &Path,
    name: &str,
    url: &str,
    target: RuleSetTarget,
    sing_box_bin: Option<std::path::PathBuf>,
) -> ExitCode {
    if name.is_empty() || name.contains(char::is_whitespace) {
        eprintln!("规则集名称不能为空或含空白");
        return ExitCode::from(2);
    }
    if let Err(message) = validate_rule_set_url(url) {
        eprintln!("拒绝 {url}：{message}");
        return ExitCode::from(2);
    }
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|mut config| {
        let entry = sbctl::config::ExtraRuleSet {
            name: name.to_owned(),
            url: url.to_owned(),
            outbound: target.word().to_owned(),
        };
        match config
            .client_extra_rule_sets
            .iter_mut()
            .find(|set| set.name == name)
        {
            Some(existing) => *existing = entry,
            None => config.client_extra_rule_sets.push(entry),
        }
        config.validate()?;
        store.replace(&config)
    });
    match result {
        Ok(()) => {
            println!("已注册规则集 {name} -> {}", target.word());
            regenerate(root, sing_box_bin)
        }
        Err(error) => {
            eprintln!("rule-set add failed: {error}");
            ExitCode::from(2)
        }
    }
}

pub(crate) fn remove(
    root: &Path,
    name: &str,
    sing_box_bin: Option<std::path::PathBuf>,
) -> ExitCode {
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|mut config| {
        let before = config.client_extra_rule_sets.len();
        config.client_extra_rule_sets.retain(|set| set.name != name);
        if config.client_extra_rule_sets.len() == before {
            return Err(sbctl::config::ConfigError::StateContent(format!(
                "未注册名为 {name:?} 的规则集"
            )));
        }
        config.validate()?;
        store.replace(&config)
    });
    match result {
        Ok(()) => {
            println!("已移除规则集 {name}");
            regenerate(root, sing_box_bin)
        }
        Err(error) => {
            eprintln!("rule-set remove failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// The largest text list accepted for import: a rule collection is lines of
/// domains, and anything this big is either hostile or the wrong URL.
const MAX_LIST_BYTES: u64 = 1 << 20;

/// Downloads one plain-text rule list and appends it to an operator list file.
///
/// https-only, size-capped, and never piped to a shell: the fetched lines become
/// routing decisions for every subscriber, so the transport has to be as
/// trustworthy as the destination.
pub(crate) fn import_list(
    url: &str,
    target: RuleSetTarget,
    root: &Path,
    sing_box_bin: Option<std::path::PathBuf>,
) -> ExitCode {
    if url.contains(char::is_whitespace) || !url.starts_with("https://") {
        eprintln!("导入源必须是 https:// 开头且不含空白的 URL");
        return ExitCode::from(2);
    }
    let temporary = match tempfile::NamedTempFile::new() {
        Ok(temporary) => temporary,
        Err(error) => {
            eprintln!("无法创建临时文件: {error}");
            return ExitCode::from(2);
        }
    };
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "60",
            "--max-filesize",
            &MAX_LIST_BYTES.to_string(),
            "--user-agent",
            "sbctl",
            "--output",
        ])
        .arg(temporary.path())
        .arg(url)
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            eprintln!("下载失败（{detail}）");
            return ExitCode::from(2);
        }
        Err(error) => {
            eprintln!("下载失败，需要 curl：{error}");
            return ExitCode::from(2);
        }
    };
    drop(output);
    let text = match fs::read_to_string(temporary.path()) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("读取下载内容失败: {error}");
            return ExitCode::from(2);
        }
    };
    // A failed import must not fall through to a regeneration of half-written
    // rules: report, and leave the previous lists and artifacts in place.
    if let Err(message) = append_lines(&text, target, root) {
        eprintln!("导入失败: {message}");
        return ExitCode::from(2);
    }
    println!("已导入 {url} -> {}", target.rule_kind().file_name());
    regenerate(root, sing_box_bin)
}

/// Stores the subscriber refresh interval and regenerates.
pub(crate) fn set_interval(
    root: &Path,
    value: &str,
    sing_box_bin: Option<std::path::PathBuf>,
) -> ExitCode {
    if sbctl::rule_list::SUPPORTED_MATCHERS.contains(&value)
        || sbctl::subscription::rule_set_interval_seconds(value).is_none()
    {
        // The stored string is handed to every subscriber's core as
        // `update_interval`, and mihomo parses it as seconds. Accepting "soon"
        // would ship a value that breaks the rule machinery on every client at
        // once, so the shape is checked before it is stored.
        eprintln!("{value:?} 不是合法时长；请用 30m / 12h / 7d 这类写法");
        return ExitCode::from(2);
    }
    let store = sbctl::config::DeploymentStore::new(root);
    let result = store.load().and_then(|mut config| {
        config.client_rule_set_update_interval = value.to_owned();
        config.validate()?;
        store.replace(&config)
    });
    match result {
        Ok(()) => {
            println!("订阅端规则集刷新间隔已设为 {value}");
            regenerate(root, sing_box_bin)
        }
        Err(error) => {
            eprintln!("rule-set interval failed: {error}");
            ExitCode::from(2)
        }
    }
}

/// Parses fetched lines and appends the valid ones to the operator list.
pub(crate) fn append_lines(text: &str, target: RuleSetTarget, root: &Path) -> Result<(), String> {
    let path = root
        .join("etc/sbctl/rules")
        .join(target.rule_kind().file_name());
    let mut existing = if path.is_file() {
        fs::read_to_string(&path).map_err(|error| error.to_string())?
    } else {
        fs::create_dir_all(path.parent().expect("list path has a parent"))
            .map_err(|error| error.to_string())?;
        String::from("# sbctl 规则列表：TYPE,value 每行一条，# 为注释\n")
    };
    let mut added = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        // A clash line already carries a policy word (`DOMAIN-SUFFIX,x,REJECT`);
        // keep the two parts this tool needs and drop the third, because which
        // file it lands in is what decides the verdict here.
        let mut parts = line.split(',');
        let matcher = parts.next().unwrap_or_default().trim().to_ascii_uppercase();
        let value = parts.next().unwrap_or_default().trim();
        if value.is_empty() {
            return Err(format!("无法解析行 {line:?}"));
        }
        validate_entry(&matcher, value).map_err(|error| format!("{line:?}: {error}"))?;
        let entry = format!("{matcher},{value}");
        if existing.contains(&entry) || existing.contains(&format!("{entry},")) {
            continue;
        }
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(&entry);
        existing.push('\n');
        added += 1;
    }
    let mut file = fs::File::create(&path).map_err(|error| error.to_string())?;
    file.write_all(existing.as_bytes())
        .map_err(|error| error.to_string())?;
    println!("新增 {added} 条规则");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{RuleSetTarget, append_lines, validate_rule_set_url};

    #[test]
    fn a_registered_rule_set_must_be_https_and_a_compiled_form() {
        assert!(validate_rule_set_url("https://cdn.example/geo/game.srs").is_ok());
        assert!(validate_rule_set_url("https://cdn.example/geo/game.mrs").is_ok());
        // Subscribers fetch this themselves; a plain-http rule-set can be
        // swapped in flight and it steers all of their traffic.
        let http =
            validate_rule_set_url("http://cdn.example/geo/game.srs").expect_err("http is refused");
        assert!(http.contains("https"), "{http}");
        let text = validate_rule_set_url("https://raw.example/lists/direct.list")
            .expect_err("a text list is not a rule-set");
        assert!(text.contains("import-list"), "{text}");
        assert!(
            validate_rule_set_url("https://cdn.example/a srs.srs").is_err(),
            "whitespace would break the URL the client is handed"
        );
    }

    #[test]
    fn an_imported_list_lands_in_the_file_that_decides_its_verdict() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path();
        let text = "# comment\n\nDOMAIN-SUFFIX,game.example,REJECT\nDOMAIN,api.example\n\
                    IP-CIDR,10.20.0.0/16,DIRECT,no-resolve\n";

        append_lines(text, RuleSetTarget::Reject, root).expect("import succeeds");
        let written =
            std::fs::read_to_string(root.join("etc/sbctl/rules/reject.list")).expect("list exists");
        // The trailing `,REJECT` on the first line is dropped: which file the
        // line went into is what decides the verdict here.
        assert!(
            written.contains("DOMAIN-SUFFIX,game.example\n"),
            "{written}"
        );
        assert!(written.contains("DOMAIN,api.example\n"), "{written}");
        assert!(written.contains("IP-CIDR,10.20.0.0/16\n"), "{written}");

        // A second import of the same lines adds nothing.
        append_lines(text, RuleSetTarget::Reject, root).expect("re-import succeeds");
        let again =
            std::fs::read_to_string(root.join("etc/sbctl/rules/reject.list")).expect("list exists");
        assert_eq!(again, written, "an import must not duplicate rules");
    }

    #[test]
    fn an_unparsable_imported_line_aborts_before_writing() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path();
        let error = append_lines(
            "DOMAIN-SUFFIX,ok.example\nGEOSITE,cn\n",
            RuleSetTarget::Direct,
            root,
        )
        .expect_err("GEOSITE needs a rule-set registration, not a list line");
        assert!(error.contains("GEOSITE"), "{error}");
        assert!(
            !root.join("etc/sbctl/rules/direct.list").exists(),
            "a refused import must not leave a partial file behind"
        );
    }
}
