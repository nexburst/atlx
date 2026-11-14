//! The `atlx` command line: clap definitions and dispatch.
//!
//! Each product keeps its clap enums and dispatch in its own module
//! (`jira`, `confluence`, `bitbucket`); the API calls they make live in the
//! crate-level product modules (`crate::jira`, ...).

mod api;
mod auth;
mod bitbucket;
mod completion;
mod confluence;
mod doctor;
mod jira;
mod search;
mod user;
mod util;

use crate::client::{ApiError, exit};
use crate::config::{self, Config};
use crate::output::{self, Format};
use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::Value;
use std::io::IsTerminal;

/// Open, agent-friendly CLI for Jira, Confluence and Bitbucket Cloud.
///
/// Commands and flags follow Atlassian's `twg` CLI, so `twg jira workitem get
/// PROJ-1 --comments` becomes `atlx jira workitem get PROJ-1 --comments`.
#[derive(Parser)]
#[command(name = "atlx", version)]
pub struct Cli {
    /// Output format [default: text on a terminal, json when piped]
    #[arg(short, long, global = true, value_enum, env = "ATLX_OUTPUT")]
    output: Option<Format>,
    /// Atlassian site prefix or hostname (e.g. acme or acme.atlassian.net)
    #[arg(short, long, global = true)]
    site: Option<String>,
    /// Atlassian account email for basic auth
    #[arg(short, long, global = true)]
    user: Option<String>,
    /// Per-request HTTP timeout in milliseconds
    #[arg(long, global = true)]
    timeout_ms: Option<u64>,
    /// Saved profile to use; `default` means the saved default [env: ATLX_PROFILE]
    #[arg(short, long, global = true, value_name = "NAME")]
    profile: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per run
enum Cmd {
    /// Save credentials (site, email, API token)
    Login(auth::LoginArgs),
    /// Remove the selected profile and its saved credentials
    Logout,
    /// Show the authenticated user
    Whoami,
    /// Check config, permissions, and service connectivity (atlx extra)
    Doctor {
        /// Fail on warnings or unconfigured checks
        #[arg(long)]
        strict: bool,
    },
    /// Saved credentials and login profiles
    Auth {
        #[command(subcommand)]
        cmd: auth::AuthCmd,
    },
    /// Optional setup: Bitbucket token, default site
    Setup {
        #[command(subcommand)]
        cmd: auth::SetupCmd,
    },
    /// Jira work items, metadata, boards and sprints
    Jira {
        #[command(subcommand)]
        cmd: jira::JiraCmd,
    },
    /// Atlassian users
    User {
        #[command(subcommand)]
        cmd: user::UserCmd,
    },
    /// Confluence search, content and spaces
    Confluence {
        #[command(subcommand)]
        cmd: confluence::ConfCmd,
    },
    /// Bitbucket repositories, pull requests and pipelines
    #[command(visible_alias = "bb")]
    Bitbucket {
        #[command(subcommand)]
        cmd: bitbucket::BbCmd,
    },
    /// Search Jira, Confluence and Bitbucket at once (atlx extra)
    Search(search::SearchArgs),
    /// Call any REST endpoint directly (atlx extra)
    ///
    /// Paths are relative to https://SITE (jira), https://SITE/wiki (confluence)
    /// or https://api.bitbucket.org/2.0 (bitbucket, where `{workspace}` is replaced
    /// with your default workspace).
    Api(api::ApiArgs),
    /// Shell completion scripts
    Completion {
        #[command(subcommand)]
        cmd: completion::CompletionCmd,
    },
    /// Generate man pages (used by the release build)
    #[command(hide = true)]
    Man(completion::ManArgs),
}

// --- runtime ---------------------------------------------------------------------

pub(crate) enum Out {
    Data(Value),
    /// Printed verbatim regardless of format (`--body-only`).
    Raw(String),
    /// Print structured output, then exit with a non-zero status code.
    Exit(Value, i32),
}

/// Parse the command line, run the command, print the result and exit.
///
/// Exit codes follow twg (see [`exit`]): 0 on success, 1 on any failure
/// (including argument errors), 3 when a batch read partially failed.
pub fn main() {
    let matches = match Cli::command().try_get_matches() {
        Ok(m) => m,
        Err(e) => exit_clap(e),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => exit_clap(e),
    };
    let fmt = cli.output.unwrap_or(if std::io::stdout().is_terminal() {
        Format::Text
    } else {
        Format::Json
    });
    let globals = Globals {
        site: cli.site,
        user: cli.user,
        timeout_ms: cli.timeout_ms,
        profile: cli.profile,
    };
    let code = match run(&globals, cli.cmd) {
        Ok(Out::Data(v)) => {
            output::print(&v, fmt);
            if util::take_partial() {
                exit::PARTIAL
            } else {
                exit::OK
            }
        }
        Ok(Out::Raw(s)) => {
            println!("{s}");
            exit::OK
        }
        Ok(Out::Exit(v, code)) => {
            output::print(&v, fmt);
            code
        }
        Err(e) => report_error(&e, fmt, &command_path(&matches)),
    };
    std::process::exit(code);
}

/// clap errors: help and version go to stdout with exit 0; argument errors go
/// to stderr with exit 1, as in twg.
fn exit_clap(e: clap::Error) -> ! {
    let _ = e.print();
    std::process::exit(if e.use_stderr() {
        exit::FAILURE
    } else {
        exit::OK
    })
}

/// `jira.workitem.get` for `atlx jira workitem get ...`.
fn command_path(m: &clap::ArgMatches) -> String {
    let mut parts = Vec::new();
    let mut cur = m;
    while let Some((name, sub)) = cur.subcommand() {
        parts.push(name);
        cur = sub;
    }
    parts.join(".")
}

/// Print an error the way twg does: in JSON modes the `{"ok": false,
/// "error": ...}` envelope goes to stdout, and a readable report always goes
/// to stderr. Returns the exit code.
fn report_error(e: &anyhow::Error, fmt: Format, path: &str) -> i32 {
    let api = ApiError::from_anyhow(e);
    match fmt {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&api.envelope()).unwrap_or_default()
        ),
        Format::Jsonl => println!("{}", api.envelope()),
        Format::Text => eprintln!("✗ Failed {path}."),
    }
    eprint!("{}", api.report());
    api.exit_code()
}

pub(crate) struct Globals {
    pub(crate) site: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    /// `-p/--profile`; `ATLX_PROFILE` is read by the config loader.
    pub(crate) profile: Option<String>,
}

impl Globals {
    fn apply(&self, mut cfg: Config) -> Config {
        if let Some(s) = &self.site {
            cfg.site = Some(config::normalize_site(s));
        }
        if let Some(u) = &self.user {
            cfg.email = Some(u.clone());
        }
        cfg.timeout_ms = self.timeout_ms;
        cfg
    }

    pub(crate) fn config(&self) -> Result<Config> {
        Ok(self.apply(Config::load(self.profile.as_deref())?))
    }
}

fn run(g: &Globals, cmd: Cmd) -> Result<Out> {
    let data = match cmd {
        Cmd::Login(a) => auth::login(g, a)?,
        Cmd::Logout => auth::logout(g)?,
        Cmd::Auth { cmd } => auth::auth_cmd(g, cmd)?,
        Cmd::Whoami => auth::whoami(g)?,
        Cmd::Doctor { strict } => return Ok(doctor::doctor(g, strict)),
        Cmd::Setup { cmd } => auth::setup(g, cmd)?,
        Cmd::Jira { cmd } => jira::jira_cmd(&g.config()?.jira()?, cmd)?,
        Cmd::User { cmd } => user::user_cmd(&g.config()?.jira()?, cmd)?,
        Cmd::Confluence { cmd } => {
            return confluence::confluence_cmd(&g.config()?.confluence()?, cmd);
        }
        Cmd::Bitbucket { cmd } => return bitbucket::bb_cmd(&g.config()?, cmd),
        Cmd::Search(a) => search::run(g, a)?,
        Cmd::Api(a) => api::run(&g.config()?, a)?,
        Cmd::Completion { cmd } => return completion::completion_cmd(cmd),
        Cmd::Man(a) => return completion::man(a),
    };
    Ok(Out::Data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_twg_style_invocations() {
        for args in [
            "atlx jira workitem get PROJ-1 PROJ-2 --comments -o json --site acme",
            "atlx jira workitem query --jql 'project = X' --limit 5",
            "atlx jira workitem create --space PROJ --type Bug --summary S --description-format markdown",
            "atlx jira workitem update --id PROJ-1 --add-labels a,b --status Done --comment hi --comment-format plain",
            "atlx jira workitem comment create --issue-id PROJ-1 --body hi --body-format plain",
            "atlx jira workitem transition --id PROJ-1 --transition-id 31",
            "atlx jira workitem transitions query --id PROJ-1",
            "atlx jira workitem create-meta --space PROJ --type Task",
            "atlx jira workitem create-meta --space PROJ",
            "atlx jira workitem link types",
            "atlx jira workitem link create --inward PROJ-1 --outward PROJ-2 --type Blocks --comment 'blocked by'",
            "atlx jira workitem link remote --id PROJ-1 --url https://example.com/spec --title Spec --relationship 'Wiki Page'",
            "atlx jira field list --query story --custom-only",
            "atlx jira project list --query PROJ --limit 10",
            "atlx jira project get PROJ",
            "atlx jira board list --project PROJ --type scrum --limit 10",
            "atlx jira sprint list --board 12 --state active,future --limit 5",
            "atlx jira sprint issues --sprint 99 --jql 'status != Done' --limit 20",
            "atlx user search alice --limit 10 -o json",
            "atlx user search 'alice smith' -o jsonl --site acme --user me@acme.com --timeout-ms 5000",
            "atlx confluence search query --cql type=page --excerpt",
            "atlx confluence search text launch plan --since 7d",
            "atlx confluence content get 12345 --detail full --format md",
            "atlx confluence content get 'Runbook' --reference-space ENG",
            "atlx confluence content create --space-id ENG --content-type page --title T --body-file f --format html --ack-body-formats -y",
            "atlx confluence content update 12345 --snapshot-token 3 --body x --format markdown --dry-run",
            "atlx confluence content list --space ENG --status archived",
            "atlx confluence content comments list 12345 --inline --format html --limit 10 --cursor c1",
            "atlx confluence content comments create 12345 --body 'Looks good' --format markdown",
            "atlx confluence content children 12345 --limit 10 --cursor c1",
            "atlx confluence content labels 12345 --limit 20",
            "atlx confluence content delete 12345 --yes --purge",
            "atlx confluence tree 12345 --depth 3",
            "atlx confluence space get ENG",
            "atlx confluence space me",
            "atlx bb repo query --workspace acme",
            "atlx bitbucket repo get --workspace acme --repo api",
            "atlx bb repo file src/main.rs --repo acme/api --ref main",
            "atlx bitbucket repo file src --repo api",
            "atlx bb repo branches --repo acme/api --query feat --limit 10",
            "atlx bitbucket repo branches --repo api",
            "atlx bb pull-requests query --workspace acme --repo api --state MERGED",
            "atlx bb prs get 930 931 --statuses --comments --repo acme/api",
            "atlx bb pull-requests create --title t --source feat --dest main --repo api",
            "atlx bb pull-requests comment create --pull-request 930 --text LGTM --repo api",
            "atlx bb pull-requests diff 930 --repo acme/api",
            "atlx bitbucket pull-requests diff https://bitbucket.org/acme/api/pull-requests/930 --repo acme/api",
            "atlx bb pull-requests activity 930 --repo api --type comment --limit 10",
            "atlx bitbucket pull-requests tasks 930 --repo api",
            "atlx bb pull-requests approve 930 --repo api",
            "atlx bitbucket pull-requests unapprove 930 --repo api",
            "atlx bb pull-requests request-changes 930 --repo api",
            "atlx bb pull-requests merge 930 --repo api --strategy squash --close-source-branch --message 'Ship it' --yes",
            "atlx bitbucket pull-requests merge 930 --repo api --strategy fast_forward -y",
            "atlx bb pull-requests decline 930 --repo api --yes",
            "atlx bitbucket pull-requests decline 930 --repo api -y",
            "atlx bb pipeline query --repo api --status FAILED",
            "atlx bb pipeline get --pipeline 1234 --logs --failed-steps --repo api",
            "atlx bb search code 'AuthService' --workspace acme --repo api --limit 10",
            "atlx bitbucket search code AuthService --repo api",
            "atlx whoami -o text",
            "atlx doctor",
            "atlx doctor --strict -o json",
            "atlx setup bitbucket --workspace acme",
            "atlx setup bitbucket --workspace acme --token-cmd 'pass show bb' -p work",
            "atlx setup default-site acme --profile work",
            "atlx -p work jira workitem get PROJ-1",
            "atlx jira workitem get PROJ-1 --profile default",
            "atlx login --new-profile",
            "atlx login --new-profile work -s acme -u me@acme.com --token-cmd 'op read op://x/y' --no-verify",
            "atlx login -p work --force",
            "atlx logout -p work",
            "atlx auth profile list",
            "atlx auth profile default work",
            "atlx auth profile rename default work",
            "atlx auth profile migrate --yes",
        ] {
            let argv: Vec<String> = shell_words(args);
            if let Err(e) = Cli::try_parse_from(&argv) {
                panic!("{args}: {e}");
            }
        }
    }

    /// Minimal splitter for the test strings (single quotes only).
    pub(crate) fn shell_words(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        for ch in s.chars() {
            match ch {
                '\'' => quoted = !quoted,
                ' ' if !quoted => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(ch),
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    #[test]
    fn parses_doctor_command() {
        let normal = Cli::try_parse_from(["atlx", "doctor"]).unwrap();
        assert!(matches!(normal.cmd, Cmd::Doctor { strict: false }));

        let strict = Cli::try_parse_from(["atlx", "doctor", "--strict"]).unwrap();
        assert!(matches!(strict.cmd, Cmd::Doctor { strict: true }));
    }
}
