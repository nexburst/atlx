//! `atlx completion script <shell>` and the hidden `atlx man` generator.
//!
//! Both are built from the clap tree at run time, so they never drift from the
//! commands the binary actually accepts. The release workflow runs them to put
//! completions and man pages into every archive.

use super::{Cli, Out};
use anyhow::{Context, Result};
use clap::{Args, CommandFactory, Subcommand};
use clap_complete::Shell;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub(crate) enum CompletionCmd {
    /// Print a completion script for a shell to stdout
    ///
    /// Bash: `atlx completion script bash > ~/.local/share/bash-completion/completions/atlx`.
    /// Zsh: write `_atlx` into a directory on your `fpath`.
    /// Fish: `atlx completion script fish > ~/.config/fish/completions/atlx.fish`.
    /// PowerShell: add `atlx completion script powershell | Out-String | Invoke-Expression`
    /// to your profile.
    Script {
        /// Shell to generate the script for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Args)]
pub(crate) struct ManArgs {
    /// Write atlx.1 plus one page per subcommand into this directory instead of
    /// printing the top-level page to stdout
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

pub(crate) fn completion_cmd(cmd: CompletionCmd) -> Result<Out> {
    match cmd {
        CompletionCmd::Script { shell } => Ok(Out::Raw(script(shell))),
    }
}

/// The completion script for `shell`, without its trailing newline (the
/// caller prints one).
pub(crate) fn script(shell: Shell) -> String {
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "atlx", &mut buf);
    let s = String::from_utf8_lossy(&buf);
    s.trim_end_matches('\n').to_string()
}

pub(crate) fn man(args: ManArgs) -> Result<Out> {
    match args.dir {
        None => {
            let mut buf = Vec::new();
            clap_mangen::Man::new(Cli::command()).render(&mut buf)?;
            let s = String::from_utf8_lossy(&buf);
            Ok(Out::Raw(s.trim_end_matches('\n').to_string()))
        }
        Some(dir) => {
            let files = write_man_pages(&dir)?;
            Ok(Out::Data(json!({
                "dir": dir.display().to_string(),
                "pages": files.len(),
            })))
        }
    }
}

/// Write `atlx.1` and `atlx-<path>.1` for every visible subcommand into `dir`
/// and return the file names, sorted.
pub(crate) fn write_man_pages(dir: &Path) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    clap_mangen::generate_to(Cli::command(), dir)
        .with_context(|| format!("writing man pages to {}", dir.display()))?;
    let mut files: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".1"))
        .collect();
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_completion_and_man() {
        for args in [
            &["atlx", "completion", "script", "bash"][..],
            &["atlx", "completion", "script", "zsh"],
            &["atlx", "completion", "script", "fish"],
            &["atlx", "completion", "script", "powershell"],
            &["atlx", "completion", "script", "elvish"],
            &["atlx", "man"],
            &["atlx", "man", "--dir", "target/man"],
        ] {
            if let Err(e) = Cli::try_parse_from(args) {
                panic!("{args:?}: {e}");
            }
        }
        assert!(Cli::try_parse_from(["atlx", "completion", "script", "tcsh"]).is_err());
    }

    #[test]
    fn scripts_cover_nested_commands() {
        for shell in [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Elvish,
        ] {
            let s = script(shell);
            assert!(s.contains("atlx"), "{shell}: no command name");
            assert!(s.contains("workitem"), "{shell}: missing nested commands");
        }
    }

    #[test]
    fn man_pages_written_per_subcommand() {
        let dir = std::env::temp_dir().join(format!("atlx-man-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = write_man_pages(&dir).unwrap();
        assert!(files.contains(&"atlx.1".to_string()), "{files:?}");
        assert!(files.contains(&"atlx-jira-workitem-get.1".to_string()));
        assert!(
            !files.iter().any(|f| f == "atlx-man.1"),
            "hidden command got a page"
        );
        let top = std::fs::read_to_string(dir.join("atlx.1")).unwrap();
        assert!(top.starts_with(".ie") || top.contains(".TH atlx"), "{top}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
