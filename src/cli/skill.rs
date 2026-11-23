//! `atlx skill`: the agent skill in `skills/atlx`, built into the binary.

use super::Out;
use crate::client::ApiError;
use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Every file of the skill, by its path inside it. The per-product command
/// references are generated from `atlx describe` (`tests/skill.rs`).
const FILES: [(&str, &str); 7] = [
    ("SKILL.md", include_str!("../../skills/atlx/SKILL.md")),
    (
        "references/bodies.md",
        include_str!("../../skills/atlx/references/bodies.md"),
    ),
    (
        "references/core.md",
        include_str!("../../skills/atlx/references/core.md"),
    ),
    (
        "references/jira.md",
        include_str!("../../skills/atlx/references/jira.md"),
    ),
    (
        "references/jsm.md",
        include_str!("../../skills/atlx/references/jsm.md"),
    ),
    (
        "references/confluence.md",
        include_str!("../../skills/atlx/references/confluence.md"),
    ),
    (
        "references/bitbucket.md",
        include_str!("../../skills/atlx/references/bitbucket.md"),
    ),
];

/// `--target`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum Target {
    /// ~/.claude/skills/atlx
    #[default]
    Claude,
    /// .agents/skills/atlx in the current directory
    Agents,
}

#[derive(Args)]
pub(crate) struct InstallArgs {
    /// Install into this directory instead
    #[arg(long, conflicts_with = "target")]
    dir: Option<PathBuf>,
    /// Which agent's skill directory
    #[arg(long, value_enum, default_value_t)]
    target: Target,
    /// Replace files that differ
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
pub(crate) enum SkillCmd {
    /// Write the skill where an agent finds it
    Install(InstallArgs),
    /// Print SKILL.md (all files with -o json)
    Print,
}

fn destination(dir: Option<PathBuf>, target: Target) -> Result<PathBuf> {
    if let Some(dir) = dir {
        if dir.as_os_str().is_empty() {
            return Err(ApiError::usage("--dir can't be empty").into());
        }
        return Ok(dir);
    }
    Ok(match target {
        Target::Agents => PathBuf::from(".agents").join("skills").join("atlx"),
        Target::Claude => {
            let home = ["HOME", "USERPROFILE"]
                .iter()
                .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))
                .ok_or_else(|| ApiError::config("no home directory is set; pass --dir"))?;
            PathBuf::from(home)
                .join(".claude")
                .join("skills")
                .join("atlx")
        }
    })
}

/// Writes the files that differ, and none of them when one would replace
/// other content without `force`. Files that aren't the skill's are left.
fn install(dir: &Path, force: bool) -> Result<Vec<&'static str>> {
    let mut changed = Vec::new();
    let mut conflicts = Vec::new();
    for (name, content) in FILES {
        let path = dir.join(name);
        match std::fs::read(&path) {
            Ok(now) if now == content.as_bytes() => continue,
            Ok(_) => conflicts.push(name),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        }
        changed.push((name, content));
    }
    if !force && !conflicts.is_empty() {
        return Err(ApiError::usage(format!(
            "{} already has a different {}; pass --force to replace",
            dir.display(),
            conflicts.join(", ")
        ))
        .into());
    }
    for (name, content) in &changed {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(changed.into_iter().map(|(name, _)| name).collect())
}

pub(crate) fn run(cmd: SkillCmd) -> Result<Out> {
    match cmd {
        SkillCmd::Install(a) => {
            let dir = destination(a.dir, a.target)?;
            let written = install(&dir, a.force)?;
            Ok(Out::Data(json!({
                "path": dir.display().to_string(),
                "written": written,
                "unchanged": FILES.len() - written.len(),
            })))
        }
        SkillCmd::Print => {
            let files: serde_json::Map<String, serde_json::Value> = FILES
                .iter()
                .map(|(name, content)| (name.to_string(), (*content).into()))
                .collect();
            Ok(Out::Doc {
                text: FILES[0].1.trim_end().to_string(),
                full: json!({"name": "atlx", "files": files}),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    /// The README's command lines are held to the same rule as the skill's.
    const README: &str = include_str!("../../README.md");

    /// Shell words, with single and double quotes removed.
    fn words(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut quote = None;
        let mut started = false;
        for ch in s.chars() {
            match (quote, ch) {
                (None, '\'' | '"') => {
                    quote = Some(ch);
                    started = true;
                }
                (Some(q), c) if c == q => quote = None,
                (None, ' ') => {
                    if started {
                        out.push(std::mem::take(&mut cur));
                        started = false;
                    }
                }
                _ => {
                    cur.push(ch);
                    started = true;
                }
            }
        }
        if started {
            out.push(cur);
        }
        out
    }

    /// The `atlx` invocations in one shell line: pipelines and `&&` lists
    /// are split; redirections, comments and leading `VAR=value` words are
    /// dropped.
    fn invocations(line: &str) -> Vec<String> {
        let mut segments = vec![String::new()];
        let mut quote = None;
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            match (quote, ch) {
                (None, '\'' | '"') => quote = Some(ch),
                (Some(q), c) if c == q => quote = None,
                (None, '#') if segments.last().unwrap().ends_with(' ') => break,
                (None, '|' | ';' | '&') => {
                    if chars.peek() == Some(&ch) {
                        chars.next();
                    }
                    segments.push(String::new());
                    continue;
                }
                (None, '>' | '<') => {
                    // The rest of the segment is the redirection's target.
                    while chars.peek().is_some_and(|c| !matches!(c, '|' | ';' | '&')) {
                        chars.next();
                    }
                    continue;
                }
                _ => {}
            }
            segments.last_mut().unwrap().push(ch);
        }
        segments
            .iter()
            .filter_map(|s| {
                let mut s = s.trim();
                while let Some((first, rest)) = s.split_once(' ') {
                    let env = first.split_once('=').is_some_and(|(k, _)| {
                        !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                    });
                    if !env {
                        break;
                    }
                    s = rest.trim_start();
                }
                s.starts_with("atlx ").then(|| s.to_string())
            })
            .collect()
    }

    /// Every `atlx ...` command in code blocks and inline code.
    fn commands(markdown: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut fenced = false;
        for line in markdown.lines() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
            } else if fenced {
                out.extend(invocations(line));
            } else {
                for code in line.split('`').skip(1).step_by(2) {
                    out.extend(invocations(code));
                }
            }
        }
        out
    }

    #[test]
    fn invocations_are_split_like_a_shell_would() {
        assert_eq!(
            invocations("atlx jira issue list -o ids | atlx jira issue get - # all"),
            ["atlx jira issue list -o ids", "atlx jira issue get -"]
        );
        assert_eq!(
            invocations("ATLX_READ_ONLY=1 atlx api jira /x > out.json && echo ok"),
            ["atlx api jira /x"]
        );
        assert_eq!(
            invocations("atlx search 'a | b # c'"),
            ["atlx search 'a | b # c'"]
        );
        assert_eq!(
            words(r#"atlx x --cql 'a = "b"' "c d" ''"#),
            ["atlx", "x", "--cql", "a = \"b\"", "c d", ""]
        );
    }

    #[test]
    fn every_command_in_the_skill_and_readme_parses() {
        let mut checked = 0;
        for (name, content) in FILES.into_iter().chain([("README.md", README)]) {
            for command in commands(content) {
                if let Err(e) = Cli::try_parse_from(words(&command)) {
                    panic!("{name}: {command}: {e}");
                }
                checked += 1;
            }
        }
        assert!(checked > 300, "only {checked} commands were found");
    }

    #[test]
    fn skill_md_stays_under_its_budget() {
        // About 1 500 tokens: SKILL.md is loaded whole, references on demand.
        let len = FILES[0].1.chars().count();
        assert!(
            len <= 6000,
            "SKILL.md has {len} characters; keep it at 6000 or fewer"
        );
    }

    #[test]
    fn skill_has_the_agent_skills_format() {
        let skill = FILES[0].1.replace("\r\n", "\n");
        let (front, body) = skill
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .expect("front matter");
        assert!(front.contains("name: atlx"));
        let description = front
            .lines()
            .find_map(|l| l.strip_prefix("description: "))
            .expect("description");
        assert!(description.len() <= 1024 && !description.contains(": "));
        assert!(body.lines().count() <= 150, "keep SKILL.md short");
        for (name, _) in &FILES[1..] {
            assert!(body.contains(name), "SKILL.md doesn't mention {name}");
        }
    }

    #[test]
    fn install_is_idempotent_and_never_overwrites_silently() {
        let dir = std::env::temp_dir().join(format!("atlx-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(install(&dir, false).unwrap().len(), FILES.len());
        assert!(install(&dir, false).unwrap().is_empty());

        let edited = dir.join(FILES[2].0);
        std::fs::write(&edited, "mine").unwrap();
        std::fs::remove_file(dir.join(FILES[1].0)).unwrap();
        assert!(install(&dir, false).is_err());
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), "mine");
        assert!(
            !dir.join(FILES[1].0).exists(),
            "nothing is written on a conflict"
        );

        assert_eq!(install(&dir, true).unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), FILES[2].1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
