//! `atlx skill`: the agent skill in `skills/atlx`, built into the binary.

use super::Out;
use crate::client::ApiError;
use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Every file of the skill, by its path inside it.
const FILES: [(&str, &str); 5] = [
    ("SKILL.md", include_str!("../../skills/atlx/SKILL.md")),
    (
        "references/jira.md",
        include_str!("../../skills/atlx/references/jira.md"),
    ),
    (
        "references/confluence.md",
        include_str!("../../skills/atlx/references/confluence.md"),
    ),
    (
        "references/bitbucket.md",
        include_str!("../../skills/atlx/references/bitbucket.md"),
    ),
    (
        "references/output-and-errors.md",
        include_str!("../../skills/atlx/references/output-and-errors.md"),
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
    use crate::cli::tests::shell_words;
    use clap::Parser;

    /// Every `atlx ...` command in code blocks and inline code.
    fn commands(markdown: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut fenced = false;
        for line in markdown.lines() {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
            } else if fenced {
                if line.trim_start().starts_with("atlx ") {
                    out.push(line.trim().to_string());
                }
            } else {
                out.extend(
                    line.split('`')
                        .skip(1)
                        .step_by(2)
                        .filter(|code| code.starts_with("atlx "))
                        .map(String::from),
                );
            }
        }
        out
    }

    #[test]
    fn every_command_in_the_skill_parses() {
        let mut checked = 0;
        for (name, content) in FILES {
            for command in commands(content) {
                if let Err(e) = Cli::try_parse_from(shell_words(&command)) {
                    panic!("{name}: {command}: {e}");
                }
                checked += 1;
            }
        }
        assert!(checked > 50, "only {checked} commands were found");
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
