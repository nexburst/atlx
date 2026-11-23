//! The `atlx` command line: clap definitions and dispatch.
//!
//! Each product keeps its clap enums and dispatch in its own module
//! (`jira`, `confluence`, `bitbucket`); the API calls they make live in the
//! crate-level product modules (`crate::jira`, ...). Each product module
//! also exports `META`: its commands' effects, records and examples (see
//! [`crate::schema::CommandMeta`]).

mod api;
mod auth;
mod bitbucket;
mod cache;
mod completion;
mod confluence;
mod describe;
mod doctor;
mod jira;
mod jsm;
mod search;
mod shape;
mod skill;
mod user;
mod util;

use crate::client::{ApiError, DryRun, exit};
use crate::config::{self, Config};
use crate::output::Format;
use crate::schema::{CommandMeta, Effect, registry};
use anyhow::Result;
use clap::error::ContextKind;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::Value;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

/// A command line for Jira, Confluence and Bitbucket Cloud, built for coding
/// agents and people on Atlassian's public REST APIs.
///
/// Every command prints one JSON envelope when piped (`{"ok":…,"data":…}`)
/// and tables on a terminal. Machine-readable: atlx describe <path>.
#[derive(Parser)]
#[command(
    name = "atlx",
    version,
    after_help = "Machine-readable: atlx describe <path>"
)]
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
    /// Send no write: reads run, the first write is printed instead of sent
    #[arg(long, global = true)]
    dry_run: bool,
    /// Refuse every write (also ATLX_READ_ONLY=1 or `read_only = true` in a profile)
    #[arg(long, global = true)]
    read_only: bool,
    /// Do not read or write the metadata cache (also ATLX_NO_CACHE=1)
    #[arg(long, global = true)]
    no_cache: bool,
    #[command(flatten)]
    shaping: shape::OutputArgs,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per run
enum Cmd {
    /// Sign in, sign out, check who is signed in, manage profiles
    Auth {
        #[command(subcommand)]
        cmd: auth::AuthCmd,
    },
    /// Check config, permissions, and service connectivity in depth
    Doctor {
        /// Fail on warnings or unconfigured checks
        #[arg(long)]
        strict: bool,
    },
    /// Jira issues, projects, boards, sprints, filters and dashboards
    Jira {
        #[command(subcommand)]
        cmd: jira::JiraCmd,
    },
    /// Jira Service Management requests, approvals and queues
    Jsm {
        #[command(subcommand)]
        cmd: jsm::JsmCmd,
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
    /// Search Jira, Confluence and Bitbucket at once
    Search(search::SearchArgs),
    /// Call any REST endpoint directly
    ///
    /// Paths are relative to https://SITE (jira), https://SITE/wiki (confluence)
    /// or https://api.bitbucket.org/2.0 (bitbucket, where `{workspace}` is replaced
    /// with your default workspace). The response goes in `data` untouched.
    /// Read-only mode and --dry-run apply to it like to every other command.
    Api(api::ApiArgs),
    /// Describe commands, record types and error codes as JSON
    Describe(describe::DescribeArgs),
    /// The local metadata cache (field names, space keys; never content)
    Cache {
        #[command(subcommand)]
        cmd: cache::CacheCmd,
    },
    /// Shell completion scripts
    Completion {
        #[command(subcommand)]
        cmd: completion::CompletionCmd,
    },
    /// Generate man pages (used by the release build)
    #[command(hide = true)]
    Man(completion::ManArgs),
    /// The agent skill that ships with atlx
    Skill {
        #[command(subcommand)]
        cmd: skill::SkillCmd,
    },
}

/// The core commands' registry entries.
const CORE_META: &[CommandMeta] = &[
    CommandMeta::new("doctor", Effect::Read).examples(&["atlx doctor", "atlx doctor --strict"]),
    CommandMeta::new("search", Effect::Read).examples(&[
        "atlx search 'login timeout' --limit 5",
        "atlx search deploy --only jira,confluence",
    ]),
    CommandMeta::new("api", Effect::ByMethod).examples(&[
        "atlx api jira /rest/api/3/myself",
        "atlx api confluence /api/v2/pages -q limit=5",
        "atlx api jira /rest/api/3/issue -X POST -d @issue.json --dry-run",
        "atlx api jira /rest/api/3/issue/PROJ-1 -X DELETE --yes",
    ]),
    CommandMeta::new("describe", Effect::Read).examples(&[
        "atlx describe",
        "atlx describe auth status",
        "atlx describe --errors",
    ]),
    CommandMeta::new("cache path", Effect::Read).examples(&["atlx cache path"]),
    CommandMeta::new("cache clear", Effect::Write).examples(&["atlx cache clear"]),
    CommandMeta::new("completion script", Effect::Read).examples(&["atlx completion script bash"]),
    CommandMeta::new("skill install", Effect::Write).examples(&["atlx skill install"]),
    CommandMeta::new("skill print", Effect::Read).examples(&["atlx skill print"]),
];

/// Every registry table: core, then each product's `META`.
pub(crate) const META_TABLES: &[&[CommandMeta]] = &[
    CORE_META,
    auth::META,
    jira::META,
    jsm::META,
    confluence::META,
    bitbucket::META,
];

// --- runtime ---------------------------------------------------------------------

pub(crate) enum Out {
    Data(Value),
    /// Printed verbatim regardless of format.
    Raw(String),
    /// `--body-only`: `body` is printed verbatim, or with `--output-file`
    /// written to the file.
    Body {
        body: String,
        #[allow(dead_code)]
        full: Value,
    },
    /// Text mode prints `text`; JSON and JSONL print `full`.
    Doc {
        text: String,
        full: Value,
    },
    /// The command already streamed its output; only exit with this code.
    Streamed(i32),
    /// Print structured output, then exit with a non-zero status code.
    Exit(Value, i32),
}

static DRY_RUN: AtomicBool = AtomicBool::new(false);

/// Whether this run is a `--dry-run`.
pub(crate) fn dry_run_requested() -> bool {
    DRY_RUN.load(Ordering::SeqCst)
}

/// Parse the command line, run the command, print the result and exit.
///
/// Exit codes are listed by `atlx describe --errors` (see [`exit`]).
pub fn main() {
    let matches = match Cli::command().try_get_matches() {
        Ok(m) => m,
        Err(e) => exit_clap(e),
    };
    let mut cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => exit_clap(e),
    };
    let fmt = cli.output.unwrap_or(if std::io::stdout().is_terminal() {
        Format::Text
    } else {
        Format::Json
    });
    util::set_text_output(fmt == Format::Text);
    let path = command_path(&matches);
    let leaf = leaf_matches(&matches);
    DRY_RUN.store(cli.dry_run, Ordering::SeqCst);
    if cli.no_cache {
        crate::cache::disable();
    }
    // A command may define its own flag with a global's name (Jira's
    // `--fields` names REST fields); then the global does not apply.
    for id in ["fields", "full", "raw", "max_chars", "output_file"] {
        if defines_locally(&path, id) {
            match id {
                "fields" => cli.shaping.fields = None,
                "full" => cli.shaping.full = false,
                "raw" => cli.shaping.raw = false,
                "max_chars" => cli.shaping.max_chars = None,
                _ => {}
            }
        }
    }
    let shaping = match shape::Shaping::new(cli.shaping, fmt, &path, Some(leaf)) {
        Ok(s) => s,
        Err(e) => {
            let plain = shape::Shaping::plain(fmt, &path);
            std::process::exit(plain.emit_error(&ApiError::from_anyhow(&e)));
        }
    };
    if let Err(e) = confirm(&path, leaf, cli.dry_run) {
        std::process::exit(shaping.emit_error(&ApiError::from_anyhow(&e)));
    }
    let globals = Globals {
        site: cli.site,
        user: cli.user,
        timeout_ms: cli.timeout_ms,
        profile: cli.profile,
        dry_run: cli.dry_run,
        read_only: cli.read_only,
    };
    let code = match run(&globals, cli.cmd) {
        Ok(Out::Exit(v, code)) => shaping.emit(Out::Exit(v, code), code),
        Ok(Out::Streamed(code)) => code,
        Ok(out) => {
            let code = if util::take_failed() {
                exit::FAILURE
            } else if util::take_partial() {
                exit::PARTIAL
            } else {
                exit::OK
            };
            shaping.emit(out, code)
        }
        Err(e) => match DryRun::find(&e) {
            Some(d) => shaping.emit_dry_run(d),
            None => shaping.emit_error(&ApiError::from_anyhow(&e)),
        },
    };
    std::process::exit(code);
}

/// The `--yes` rule: a command registered as destructive refuses to run
/// without `--yes`, unless this is a `--dry-run`. Nothing ever prompts.
fn confirm(path: &str, leaf: &clap::ArgMatches, dry_run: bool) -> Result<()> {
    let Some(meta) = registry::lookup(path) else {
        return Ok(());
    };
    let destructive = match meta.effect {
        Effect::ByMethod => leaf
            .try_get_one::<String>("method")
            .ok()
            .flatten()
            .is_some_and(|m| Effect::of_method(m) == Effect::Destructive),
        _ => meta.confirm(),
    };
    if !destructive || dry_run {
        return Ok(());
    }
    let yes = leaf.try_get_one::<bool>("yes").ok().flatten() == Some(&true);
    if yes {
        return Ok(());
    }
    Err(util::needs_yes(&format!("`atlx {path}`")))
}

/// Whether the command at `path` defines an argument `id` of its own
/// (rather than inheriting the global one).
fn defines_locally(path: &str, id: &str) -> bool {
    let mut cmd = Cli::command();
    for word in path.split(' ').filter(|w| !w.is_empty()) {
        let Some(sc) = cmd.find_subcommand(word).cloned() else {
            return false;
        };
        cmd = sc;
    }
    cmd.get_arguments().any(|a| a.get_id() == id)
}

/// clap errors: help and version go to stdout with exit 0. Argument errors
/// exit 2 (`usage`): in JSON modes as an error envelope on stdout with
/// `suggestions`, in text mode as clap's message on stderr.
fn exit_clap(e: clap::Error) -> ! {
    if !e.use_stderr() {
        let _ = e.print();
        std::process::exit(exit::OK);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fmt = output_from_args(&args);
    if fmt == Format::Text {
        let _ = e.print();
        std::process::exit(exit::USAGE);
    }
    let shaping = shape::Shaping::plain(fmt, "");
    std::process::exit(shaping.emit_error(&clap_usage(&e)))
}

/// A clap parse error as a structured `usage` error.
fn clap_usage(e: &clap::Error) -> ApiError {
    let text = e.render().to_string();
    let message = text
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start_matches("error: ")
        .to_string();
    let mut suggestions = Vec::new();
    for kind in [
        ContextKind::SuggestedArg,
        ContextKind::SuggestedSubcommand,
        ContextKind::SuggestedValue,
    ] {
        match e.get(kind) {
            Some(clap::error::ContextValue::String(s)) => suggestions.push(s.clone()),
            Some(clap::error::ContextValue::Strings(v)) => suggestions.extend(v.iter().cloned()),
            _ => {}
        }
    }
    if let Some(clap::error::ContextValue::Strings(v)) = e.get(ContextKind::ValidValue) {
        suggestions.extend(v.iter().cloned());
    }
    let mut api = ApiError::usage(message).with_suggestions(suggestions);
    if let Some(clap::error::ContextValue::String(u)) = e.get(ContextKind::Usage) {
        api.hint = Some(format!(
            "{}; run `atlx describe <command>` for its flags",
            u.trim().trim_start_matches("Usage: ")
        ));
    }
    api
}

/// The output format named on a command line clap could not parse: `-o`,
/// `ATLX_OUTPUT`, else text on a terminal and JSON when piped.
fn output_from_args(args: &[String]) -> Format {
    let named = args
        .iter()
        .zip(args.iter().skip(1))
        .find(|(a, _)| *a == "-o" || *a == "--output")
        .map(|(_, v)| v.clone())
        .or_else(|| {
            args.iter()
                .find_map(|a| a.strip_prefix("--output=").map(String::from))
        })
        .or_else(|| std::env::var("ATLX_OUTPUT").ok());
    match named.as_deref() {
        Some("json") => Format::Json,
        Some("jsonl") => Format::Jsonl,
        Some("ids") => Format::Ids,
        Some("text") => Format::Text,
        _ if std::io::stdout().is_terminal() => Format::Text,
        _ => Format::Json,
    }
}

/// `jira issue get` for `atlx jira issue get ...`.
fn command_path(m: &clap::ArgMatches) -> String {
    let mut parts = Vec::new();
    let mut cur = m;
    while let Some((name, sub)) = cur.subcommand() {
        parts.push(name);
        cur = sub;
    }
    // `bb` is an alias; the registry uses the command's name.
    if parts.first() == Some(&"bb") {
        parts[0] = "bitbucket";
    }
    parts.join(" ")
}

/// The matches of the command that runs.
fn leaf_matches(m: &clap::ArgMatches) -> &clap::ArgMatches {
    let mut cur = m;
    while let Some((_, sub)) = cur.subcommand() {
        cur = sub;
    }
    cur
}

pub(crate) struct Globals {
    pub(crate) site: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) timeout_ms: Option<u64>,
    /// `-p/--profile`; `ATLX_PROFILE` is read by the config loader.
    pub(crate) profile: Option<String>,
    pub(crate) dry_run: bool,
    /// `--read-only`; `ATLX_READ_ONLY` and the profile are read by the
    /// config loader.
    pub(crate) read_only: bool,
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
        cfg.dry_run = self.dry_run;
        cfg.read_only |= self.read_only;
        cfg
    }

    pub(crate) fn config(&self) -> Result<Config> {
        Ok(self.apply(Config::load(self.profile.as_deref())?))
    }

    /// `--read-only` or `ATLX_READ_ONLY` (without loading a profile).
    pub(crate) fn read_only_requested(&self) -> bool {
        self.read_only
            || std::env::var("ATLX_READ_ONLY")
                .is_ok_and(|v| !matches!(v.trim(), "" | "0" | "false" | "no" | "off"))
    }
}

fn run(g: &Globals, cmd: Cmd) -> Result<Out> {
    let data = match cmd {
        Cmd::Auth { cmd } => auth::run(g, cmd)?,
        Cmd::Doctor { strict } => return Ok(doctor::doctor(g, strict)),
        Cmd::Jira { cmd } => jira::jira_cmd(&g.config()?.jira()?, cmd)?,
        Cmd::Jsm { cmd } => jsm::jsm_cmd(&g.config()?.jira()?, cmd)?,
        Cmd::Confluence { cmd } => {
            if let Some(out) = confluence::offline(&cmd) {
                return out;
            }
            return confluence::confluence_cmd(&g.config()?.confluence()?, cmd);
        }
        Cmd::Bitbucket { cmd } => return bitbucket::bb_cmd(&g.config()?, cmd),
        Cmd::Search(a) => search::run(g, a)?,
        Cmd::Api(a) => api::run(&g.config()?, a)?,
        Cmd::Describe(a) => return describe::run(a),
        Cmd::Cache { cmd } => cache::run(cmd)?,
        Cmd::Completion { cmd } => return completion::completion_cmd(cmd),
        Cmd::Man(a) => return completion::man(a),
        Cmd::Skill { cmd } => return skill::run(cmd),
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
    fn parses_core_and_product_invocations() {
        for args in [
            "atlx jira issue get PROJ-1 PROJ-2 -o json --site acme",
            "atlx jira issue list --jql 'project = X' --limit 5",
            "atlx jira issue create --project PROJ --type Bug --summary S --body-format markdown",
            "atlx jira issue comment create --issue PROJ-1 --body hi --body-format plain",
            "atlx jira field list --query story --custom",
            "atlx jira user search alice --limit 10 -o json",
            "atlx confluence search --cql type=page",
            "atlx confluence page get 12345 --body-format markdown --full",
            "atlx confluence page update 12345 --if-version 3 --body x --body-format markdown --dry-run",
            "atlx bb repo list --workspace acme",
            "atlx bb pr merge 930 --repo acme/api --strategy squash --yes",
            "atlx bb pipeline log 1234 --failed --repo acme/api",
            "atlx doctor --strict -o json",
            "atlx jira issue get PROJ-1 -o json --fields key,status --output-file /tmp/x.json",
            "atlx jira issue get PROJ-1 -o ids --full",
            "atlx jira issue get PROJ-1 --raw --max-chars 0",
            "atlx --dry-run --read-only --no-cache jira issue get PROJ-1",
            "atlx confluence page get 12345 --body-only --output-file ./page.md -o json",
            "atlx -p work jira issue get PROJ-1",
            "atlx jira issue get PROJ-1 --profile default",
            "atlx auth login --new-profile",
            "atlx auth login --new-profile work -s acme -u me@acme.com --token-cmd 'op read op://x/y' --no-verify",
            "atlx auth login -p work --force",
            "atlx auth logout -p work",
            "atlx auth status -o json",
            "atlx auth bitbucket --workspace acme --token-cmd 'pass show bb' -p work",
            "atlx auth profile list",
            "atlx auth profile update -s acme --profile work",
            "atlx auth profile default work",
            "atlx auth profile rename default work",
            "atlx auth profile migrate --yes",
            "atlx describe",
            "atlx describe jira issue get",
            "atlx describe --records",
            "atlx describe --errors",
            "atlx describe bitbucket --all",
            "atlx cache path",
            "atlx cache clear",
        ] {
            let argv: Vec<String> = shell_words(args);
            if let Err(e) = Cli::try_parse_from(&argv) {
                panic!("{args}: {e}");
            }
        }
    }

    #[test]
    fn removed_spellings_no_longer_parse() {
        for args in [
            "atlx whoami",
            "atlx login",
            "atlx setup bitbucket",
            "atlx jira issue get P-1 --select key",
            "atlx jira issue get P-1 --output-summary",
        ] {
            assert!(Cli::try_parse_from(shell_words(args)).is_err(), "{args}");
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

    #[test]
    fn descending_sort_values_parse_as_documented() {
        for args in [
            "bb repo branch list -r acme/api --sort -name",
            "bb repo list --workspace acme --sort -updated_on",
            "jira project list --order-by -lastIssueUpdatedDate",
            "confluence page attachment list --page 1 --sort -created-date",
        ] {
            if let Err(e) = Cli::try_parse_from(["atlx"].into_iter().chain(args.split(' '))) {
                panic!("{args}: {e}");
            }
        }
    }

    #[test]
    fn local_flags_shadow_globals() {
        assert!(!defines_locally("confluence page update", "dry_run"));
        assert!(!defines_locally("auth status", "dry_run"));
        assert!(!defines_locally("nope nope", "fields"));
    }
}
