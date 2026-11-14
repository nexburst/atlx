use anyhow::{Context, Result, bail};
use atlx::{Product, bitbucket, client, config, confluence, jira, markup, output, search};
use clap::{Args, Parser, Subcommand};
use config::Config;
use markup::TextFormat;
use output::Format;
use serde_json::{Value, json};
use std::io::{IsTerminal, Read, Write};

/// Open, agent-friendly CLI for Jira, Confluence and Bitbucket Cloud.
///
/// Commands and flags follow Atlassian's `twg` CLI, so `twg jira workitem get
/// PROJ-1 --comments` becomes `atlx jira workitem get PROJ-1 --comments`.
#[derive(Parser)]
#[command(name = "atlx", version)]
struct Cli {
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
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once per run
enum Cmd {
    /// Save credentials (site, email, API token)
    Login(LoginArgs),
    /// Delete saved credentials
    Logout,
    /// Show the authenticated user
    Whoami,
    /// Optional setup: Bitbucket token, default site
    Setup {
        #[command(subcommand)]
        cmd: SetupCmd,
    },
    /// Jira work items
    Jira {
        #[command(subcommand)]
        cmd: JiraCmd,
    },
    /// Confluence search, content and spaces
    Confluence {
        #[command(subcommand)]
        cmd: ConfCmd,
    },
    /// Bitbucket repositories, pull requests and pipelines
    #[command(visible_alias = "bb")]
    Bitbucket {
        #[command(subcommand)]
        cmd: BbCmd,
    },
    /// Search Jira, Confluence and Bitbucket at once (atlx extra)
    Search {
        query: String,
        /// Max results per product
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Only search these products (comma-separated)
        #[arg(long, value_enum, value_delimiter = ',')]
        only: Vec<Product>,
    },
    /// Call any REST endpoint directly (atlx extra)
    ///
    /// Paths are relative to https://SITE (jira), https://SITE/wiki (confluence)
    /// or https://api.bitbucket.org/2.0 (bitbucket, where `{workspace}` is replaced
    /// with your default workspace).
    Api {
        product: Product,
        path: String,
        #[arg(short = 'X', long, default_value = "GET")]
        method: String,
        /// JSON body; `-` reads stdin, `@file` reads a file
        #[arg(short, long)]
        data: Option<String>,
        /// Query parameter, repeatable
        #[arg(short, long = "query", value_name = "KEY=VALUE")]
        query: Vec<String>,
    },
}

#[derive(Args)]
struct LoginArgs {
    /// API token (https://id.atlassian.com/manage-profile/security/api-tokens); `-` reads stdin
    #[arg(long)]
    token: Option<String>,
    /// Skip credential verification
    #[arg(long)]
    no_verify: bool,
    /// Re-prompt for everything, ignoring saved values
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
enum SetupCmd {
    /// Set up the Bitbucket workspace and token
    Bitbucket {
        #[arg(long)]
        workspace: Option<String>,
        /// Bitbucket-scoped API token; `-` reads stdin
        #[arg(long)]
        token: Option<String>,
        /// Bitbucket account email, if different from the Atlassian one
        #[arg(long)]
        email: Option<String>,
        /// Prompt even when a Bitbucket token is already configured
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_verify: bool,
    },
    /// Save the default Atlassian site (no re-login)
    DefaultSite { site: Option<String> },
}

// --- jira ----------------------------------------------------------------------

#[derive(Subcommand)]
enum JiraCmd {
    /// Jira work items (issues)
    Workitem {
        #[command(subcommand)]
        cmd: WorkitemCmd,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum WorkitemCmd {
    /// Get one or more work items by key, ID, ARI or URL
    Get {
        ids: Vec<String>,
        /// Issue key, ID, or ARI; repeat to batch
        #[arg(long = "id")]
        id_flags: Vec<String>,
        #[arg(long)]
        issue_id: Option<String>,
        #[arg(long)]
        key: Option<String>,
        /// Comma-separated Jira REST fields to include
        #[arg(long)]
        fields: Option<String>,
        /// Jira REST field or customfield_* ID to include; repeatable
        #[arg(long)]
        field: Vec<String>,
        /// Fetch complete comments
        #[arg(long)]
        comments: bool,
        /// Fetch remote links
        #[arg(long)]
        remote_links: bool,
        /// All fields, comments and remote links
        #[arg(long, conflicts_with_all = ["comments", "remote_links"])]
        full: bool,
        /// Fail the batch if any input fails
        #[arg(long)]
        strict: bool,
    },
    /// Find work items by fuzzy text (text ~ "…" ORDER BY updated DESC)
    Search {
        #[arg(required = true)]
        text: Vec<String>,
        #[arg(long)]
        limit: Option<usize>,
        /// Page size; alias for --limit
        #[arg(long)]
        first: Option<usize>,
        /// Pagination cursor (next_cursor from a previous response)
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        fields: Option<String>,
    },
    /// Run a JQL query
    Query {
        jql_arg: Option<String>,
        #[arg(long)]
        jql: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        first: Option<usize>,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        fields: Option<String>,
    },
    /// Create a work item
    Create {
        /// Jira space (project) key or ID
        #[arg(long)]
        space: String,
        /// Issue type name or ID
        #[arg(long = "type", default_value = "Task")]
        issue_type: String,
        #[arg(long)]
        summary: String,
        /// `-` reads stdin
        #[arg(long)]
        description: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        description_format: TextFormat,
        /// Account ID, "me", or a name/email
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        reporter: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        /// Comma-separated
        #[arg(long)]
        labels: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        /// KEY=VALUE (value parsed as JSON when valid); repeatable
        #[arg(long)]
        field: Vec<String>,
        #[arg(long)]
        fields_json: Option<String>,
        /// Accepted for twg compatibility
        #[arg(long, short)]
        yes: bool,
    },
    /// Update fields, optionally transition and comment
    Update(UpdateArgs),
    /// Comments on work items
    Comment {
        #[command(subcommand)]
        cmd: CommentCmd,
    },
    /// Transition a work item; omit --transition-id to list transitions
    Transition {
        #[arg(long)]
        id: String,
        /// Transition ID, transition name, or target status name
        #[arg(long)]
        transition_id: Option<String>,
        #[arg(long)]
        field: Vec<String>,
        #[arg(long)]
        fields_json: Option<String>,
        #[arg(long)]
        transition_comment: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        transition_comment_format: TextFormat,
    },
    /// Available transitions
    Transitions {
        #[command(subcommand)]
        cmd: TransitionsCmd,
    },
}

#[derive(Args)]
struct UpdateArgs {
    #[arg(long)]
    id: String,
    #[arg(long)]
    summary: Option<String>,
    /// `-` reads stdin
    #[arg(long)]
    description: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    description_format: TextFormat,
    /// "me", "none", an account ID, or a name/email
    #[arg(long)]
    assignee: Option<String>,
    #[arg(long = "type")]
    issue_type: Option<String>,
    #[arg(long)]
    priority: Option<String>,
    /// Comma-separated; replaces existing
    #[arg(long, conflicts_with_all = ["add_labels", "remove_labels"])]
    labels: Option<String>,
    #[arg(long)]
    add_labels: Option<String>,
    #[arg(long)]
    remove_labels: Option<String>,
    /// Parent key, or "none"
    #[arg(long)]
    parent: Option<String>,
    /// YYYY-MM-DD, or "none"
    #[arg(long)]
    due_date: Option<String>,
    #[arg(long, conflicts_with_all = ["add_fix_versions", "remove_fix_versions"])]
    fix_versions: Option<String>,
    #[arg(long)]
    add_fix_versions: Option<String>,
    #[arg(long)]
    remove_fix_versions: Option<String>,
    #[arg(long, conflicts_with_all = ["add_components", "remove_components"])]
    components: Option<String>,
    #[arg(long)]
    add_components: Option<String>,
    #[arg(long)]
    remove_components: Option<String>,
    /// Resolution name or ID, or "none"
    #[arg(long)]
    resolution: Option<String>,
    #[arg(long)]
    field: Vec<String>,
    #[arg(long)]
    fields_json: Option<String>,
    /// Transition by target status or transition name
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    transition_comment: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    transition_comment_format: TextFormat,
    /// Add a comment after the update
    #[arg(long)]
    comment: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    comment_format: TextFormat,
}

#[derive(Subcommand)]
enum CommentCmd {
    /// Add a comment
    Create {
        #[arg(long)]
        issue_id: String,
        /// `-` reads stdin
        #[arg(long)]
        body: String,
        #[arg(long, value_enum, default_value_t)]
        body_format: TextFormat,
        /// JSM internal comment
        #[arg(long)]
        internal: bool,
        #[arg(long, conflicts_with = "visibility_group")]
        visibility_role: Option<String>,
        #[arg(long)]
        visibility_group: Option<String>,
    },
}

#[derive(Subcommand)]
enum TransitionsCmd {
    /// List transitions and the fields they require
    Query {
        #[arg(long)]
        id: String,
    },
}

// --- confluence ------------------------------------------------------------------

#[derive(Subcommand)]
enum ConfCmd {
    /// CQL and text search
    Search {
        #[command(subcommand)]
        cmd: ConfSearchCmd,
    },
    /// Pages and blog posts
    Content {
        #[command(subcommand)]
        cmd: ContentCmd,
    },
    /// Spaces
    Space {
        #[command(subcommand)]
        cmd: SpaceCmd,
    },
}

#[derive(Subcommand)]
enum ConfSearchCmd {
    /// Search with CQL
    Query {
        cql_arg: Vec<String>,
        #[arg(long)]
        cql: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Include matched text excerpts
        #[arg(long)]
        excerpt: bool,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Search recent pages and blog posts with natural text
    Text {
        text: Vec<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// YYYY-MM-DD or a duration like 7d, 2w, 1m
        #[arg(long, visible_alias = "since")]
        updated_since: Option<String>,
        #[arg(long)]
        created_since: Option<String>,
        #[arg(long)]
        excerpt: bool,
        #[arg(long)]
        cursor: Option<String>,
    },
}

#[derive(Subcommand)]
enum ContentCmd {
    /// Read one or more pages/blog posts by ID or URL
    Get {
        refs: Vec<String>,
        #[arg(long = "id")]
        id_flags: Vec<String>,
        /// summary (excerpt) or full (body)
        #[arg(long, value_enum)]
        detail: Option<confluence::Detail>,
        /// Body format for --detail full: html (storage) or md
        #[arg(long, value_enum, default_value_t)]
        format: confluence::ReadFormat,
        /// Print only the body
        #[arg(long)]
        body_only: bool,
        /// Fetch the draft version
        #[arg(long)]
        draft: bool,
        /// Accepted for twg compatibility (metadata is always included)
        #[arg(long)]
        include_metadata: bool,
        /// Fail the batch if any input fails
        #[arg(long)]
        strict: bool,
    },
    /// Create a page or blog post
    Create {
        /// Space key or ID
        #[arg(long)]
        space_id: String,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long, value_enum)]
        content_type: confluence::ContentType,
        #[arg(long)]
        title: String,
        /// Inline body (`-` reads stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        format: TextFormat,
        #[arg(long)]
        draft: bool,
        /// Accepted for twg compatibility
        #[arg(long, short)]
        yes: bool,
        /// Accepted for twg compatibility
        #[arg(long)]
        ack_body_formats: bool,
    },
    /// Update a page or blog post (new version)
    Update {
        reference: Option<String>,
        #[arg(long)]
        id: Option<String>,
        /// Version from a previous get/create/update; rejects stale edits
        #[arg(long)]
        snapshot_token: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        version_message: Option<String>,
        #[arg(long)]
        minor_edit: bool,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<String>,
        #[arg(long, value_enum, default_value_t)]
        format: TextFormat,
        /// Validate and show the request without writing
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        draft: bool,
        #[arg(long, short)]
        yes: bool,
        #[arg(long)]
        ack_body_formats: bool,
    },
    /// List content in a space
    List {
        #[arg(long)]
        space: String,
        #[arg(long, value_enum, default_value_t)]
        content_type: confluence::ContentType,
        #[arg(long, default_value = "current")]
        status: String,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
}

#[derive(Subcommand)]
enum SpaceCmd {
    /// List spaces
    List {
        /// global or personal
        #[arg(long = "type")]
        kind: Option<String>,
        /// current or archived
        #[arg(long)]
        status: Option<String>,
        /// Comma-separated space keys
        #[arg(long)]
        keys: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Get a space by key or ID
    Get { identifier: String },
}

// --- bitbucket -------------------------------------------------------------------

#[derive(Args)]
struct RepoArgs {
    /// Bitbucket workspace slug
    #[arg(long)]
    workspace: Option<String>,
    /// Repository slug, workspace/repo, or Bitbucket URL
    #[arg(long)]
    repo: Option<String>,
}

#[derive(Subcommand)]
enum BbCmd {
    /// Repositories
    Repo {
        #[command(subcommand)]
        cmd: RepoCmd,
    },
    /// Pull requests
    #[command(name = "pull-requests", visible_alias = "prs")]
    PullRequests {
        #[command(subcommand)]
        cmd: PrCmd,
    },
    /// Pipelines
    Pipeline {
        #[command(subcommand)]
        cmd: PipelineCmd,
    },
}

#[derive(Subcommand)]
enum RepoCmd {
    /// List repositories in a workspace
    Query {
        /// Search by name
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(long, default_value = "-updated_on")]
        sort: String,
        /// owner, admin, contributor, member
        #[arg(long)]
        role: Option<String>,
    },
    /// Repository details
    Get {
        slug: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
enum PrCmd {
    /// List pull requests in a repository
    Query {
        text: Vec<String>,
        /// OPEN, MERGED, DECLINED, SUPERSEDED
        #[arg(long)]
        state: Option<String>,
        /// Nickname, display name, account ID, or {uuid}
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        dest: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        created_since: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long)]
        updated_since: Option<String>,
        #[arg(long)]
        updated_before: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One or more pull requests by number or URL
    Get {
        ids: Vec<String>,
        #[arg(long)]
        comments: bool,
        /// File-level diff stats
        #[arg(long)]
        diff: bool,
        /// Build/CI statuses
        #[arg(long)]
        statuses: bool,
        /// Comments, diff stats and statuses
        #[arg(long)]
        full: bool,
        #[arg(long)]
        strict: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Create a pull request
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        source: String,
        /// Defaults to the repository main branch
        #[arg(long)]
        dest: Option<String>,
        #[arg(long, conflicts_with = "description_file")]
        description: Option<String>,
        #[arg(long, visible_alias = "desc-file")]
        description_file: Option<String>,
        /// Comma-separated Bitbucket user UUIDs
        #[arg(long)]
        reviewer_uuid: Option<String>,
        /// Comma-separated Atlassian account IDs
        #[arg(long)]
        reviewer_account_id: Option<String>,
        #[arg(long)]
        close_source_branch: bool,
        #[arg(long)]
        draft: bool,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Pull request comments
    Comment {
        #[command(subcommand)]
        cmd: PrCommentCmd,
    },
}

#[derive(Subcommand)]
enum PrCommentCmd {
    /// Add a general, reply, or inline comment
    Create {
        /// Pull request number or URL
        #[arg(long)]
        pull_request: String,
        /// `-` reads stdin
        #[arg(long)]
        text: String,
        #[arg(long)]
        reply_to: Option<u64>,
        /// Inline comment on this file path
        #[arg(long)]
        path: Option<String>,
        #[arg(long, requires = "path")]
        line: Option<u64>,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

#[derive(Subcommand)]
enum PipelineCmd {
    /// Recent pipeline runs
    Query {
        /// PENDING, IN_PROGRESS, COMPLETED
        #[arg(long)]
        state: Option<String>,
        /// PASSED, FAILED, STOPPED, ERROR
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// PUSH, MANUAL, SCHEDULE, PULLREQUEST
        #[arg(long)]
        trigger: Option<String>,
        #[arg(long, default_value_t = 15)]
        limit: usize,
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// One pipeline run with its steps
    Get {
        /// Build number, {uuid}, or result URL
        #[arg(long)]
        pipeline: String,
        /// Include step logs
        #[arg(long)]
        logs: bool,
        /// With --logs, only failed steps
        #[arg(long)]
        failed_steps: bool,
        /// Max log lines per step (0 = all)
        #[arg(long, default_value_t = 100)]
        lines: usize,
        /// Limit logs to one step (name or uuid)
        #[arg(long)]
        step: Option<String>,
        #[command(flatten)]
        repo: RepoArgs,
    },
}

// --- runtime ---------------------------------------------------------------------

enum Out {
    Data(Value),
    /// Printed verbatim regardless of format (`--body-only`).
    Raw(String),
}

fn main() {
    let cli = Cli::parse();
    let fmt = cli.output.unwrap_or(if std::io::stdout().is_terminal() { Format::Text } else { Format::Json });
    let globals = Globals { site: cli.site, user: cli.user, timeout_ms: cli.timeout_ms };
    match run(&globals, cli.cmd) {
        Ok(Out::Data(v)) => output::print(&v, fmt),
        Ok(Out::Raw(s)) => println!("{s}"),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

struct Globals {
    site: Option<String>,
    user: Option<String>,
    timeout_ms: Option<u64>,
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

    fn config(&self) -> Result<Config> {
        Ok(self.apply(Config::load()?))
    }
}

fn run(g: &Globals, cmd: Cmd) -> Result<Out> {
    let data = match cmd {
        Cmd::Login(a) => login(g, a)?,
        Cmd::Logout => {
            let p = config::path()?;
            let existed = p.exists();
            if existed {
                std::fs::remove_file(&p)?;
            }
            json!({"removed": existed, "config": p.display().to_string()})
        }
        Cmd::Whoami => {
            let cfg = g.config()?;
            let me = jira::myself(&cfg.jira()?)?;
            json!({"name": me["displayName"], "email": me["emailAddress"], "account_id": me["accountId"],
                   "site": cfg.site, "bitbucket_workspace": cfg.bitbucket.workspace})
        }
        Cmd::Setup { cmd } => setup(g, cmd)?,
        Cmd::Jira { cmd: JiraCmd::Workitem { cmd } } => workitem(&g.config()?.jira()?, cmd)?,
        Cmd::Confluence { cmd } => return confluence_cmd(&g.config()?.confluence()?, cmd),
        Cmd::Bitbucket { cmd } => bb_cmd(&g.config()?, cmd)?,
        Cmd::Search { query, limit, only } => search::run(&g.config()?, &query, limit, &only),
        Cmd::Api { product, path, method, data, query } => api(&g.config()?, product, &path, &method, data, &query)?,
    };
    Ok(Out::Data(data))
}

/// `-` means read stdin; anything else is used as-is.
fn text_arg(s: String) -> Result<String> {
    if s != "-" {
        return Ok(s);
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).context("reading stdin")?;
    Ok(buf)
}

/// twg-style `--body` / `--body-file` pair.
fn body_arg(inline: Option<String>, file: Option<String>) -> Result<Option<String>> {
    match (inline, file) {
        (Some(b), _) => Ok(Some(text_arg(b)?)),
        (None, Some(f)) => Ok(Some(std::fs::read_to_string(&f).with_context(|| format!("reading {f}"))?)),
        (None, None) => Ok(None),
    }
}

#[cfg(unix)]
struct EchoGuard;

#[cfg(unix)]
impl EchoGuard {
    /// Disable terminal echo for stdin and restore it automatically on drop.
    fn disable() -> Result<Self> {
        let status = std::process::Command::new("stty")
            .arg("-echo")
            .stdin(std::process::Stdio::inherit())
            .status()
            .context("disabling terminal echo via stty")?;
        if !status.success() {
            bail!("failed to disable terminal echo via stty");
        }
        Ok(Self)
    }
}

#[cfg(unix)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("stty").arg("echo").stdin(std::process::Stdio::inherit()).status();
        eprintln!();
    }
}

#[cfg(not(unix))]
struct EchoGuard;

#[cfg(not(unix))]
impl EchoGuard {
    fn disable() -> Result<Self> {
        Ok(Self)
    }
}

fn read_prompt_line(label: &str, secret: bool) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("missing {label}; pass it as a flag");
    }
    eprint!("{label}: ");
    std::io::stderr().flush()?;
    let _guard = if secret { Some(EchoGuard::disable()?) } else { None };
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    drop(_guard);
    let line = line.trim().to_string();
    if line.is_empty() {
        bail!("{label} is required");
    }
    Ok(line)
}

fn prompt(label: &str) -> Result<String> {
    read_prompt_line(label, false)
}

fn prompt_secret(label: &str) -> Result<String> {
    read_prompt_line(label, true)
}

fn keep(saved: Option<String>, force: bool) -> Option<String> {
    saved.filter(|_| !force)
}

fn login(g: &Globals, a: LoginArgs) -> Result<Value> {
    let mut cfg = Config::load_file()?;
    let site = match g.site.clone().or(keep(cfg.site.take(), a.force)) {
        Some(s) => s,
        None => prompt("Atlassian site (e.g. acme or acme.atlassian.net)")?,
    };
    cfg.site = Some(config::normalize_site(&site));
    cfg.email = Some(match g.user.clone().or(keep(cfg.email.take(), a.force)) {
        Some(s) => s,
        None => prompt("Email")?,
    });
    cfg.token = Some(match a.token {
        Some(t) => text_arg(t)?.trim().to_string(),
        None => match keep(cfg.token.take(), a.force) {
            Some(t) => t,
            None => prompt_secret("API token (https://id.atlassian.com/manage-profile/security/api-tokens)")?,
        },
    });
    cfg.timeout_ms = g.timeout_ms;
    let mut out = json!({"site": cfg.site, "email": cfg.email});
    if !a.no_verify {
        let me = jira::myself(&cfg.jira()?).context("credentials check failed; nothing saved")?;
        out["name"] = me["displayName"].clone();
        out["account_id"] = me["accountId"].clone();
    }
    out["saved"] = cfg.save()?.display().to_string().into();
    Ok(out)
}

fn setup(g: &Globals, cmd: SetupCmd) -> Result<Value> {
    let mut cfg = Config::load_file()?;
    match cmd {
        SetupCmd::DefaultSite { site } => {
            let site = match site.or(g.site.clone()) {
                Some(s) => s,
                None => prompt("Atlassian site (e.g. acme or acme.atlassian.net)")?,
            };
            cfg.site = Some(config::normalize_site(&site));
            Ok(json!({"site": cfg.site, "saved": cfg.save()?.display().to_string()}))
        }
        SetupCmd::Bitbucket { workspace, token, email, force, no_verify } => {
            let ws = match workspace.or(keep(cfg.bitbucket.workspace.take(), force)) {
                Some(w) => w,
                None => prompt("Bitbucket workspace slug")?,
            };
            cfg.bitbucket.workspace = Some(ws.clone());
            if email.is_some() {
                cfg.bitbucket.email = email;
            }
            cfg.bitbucket.token = Some(match token {
                Some(t) => text_arg(t)?.trim().to_string(),
                None => match keep(cfg.bitbucket.token.take(), force) {
                    Some(t) => t,
                    None => prompt_secret("Bitbucket API token (scopes: account, repository, pullrequest, pipeline)")?,
                },
            });
            cfg.timeout_ms = g.timeout_ms;
            let mut out = json!({"workspace": ws});
            if !no_verify {
                let (c, _) = cfg.bitbucket()?;
                let q = bitbucket::RepoQuery { query: None, project: None, language: None, sort: "-updated_on", role: None, limit: 1 };
                bitbucket::repo_query(&c, &ws, &q).context("Bitbucket check failed; nothing saved")?;
                out["verified"] = true.into();
            }
            out["saved"] = cfg.save()?.display().to_string().into();
            Ok(out)
        }
    }
}

/// Issue key from a key, numeric ID, ARI, or `/browse/KEY` URL.
fn issue_ref(s: &str) -> String {
    let s = s.trim();
    if let Some(i) = s.find("/browse/") {
        return s[i + 8..].split(['/', '?', '#']).next().unwrap_or_default().to_string();
    }
    if s.starts_with("ari:") {
        return s.rsplit('/').next().unwrap_or(s).to_string();
    }
    s.to_string()
}

fn fields_arg(fields: Option<&str>, field: &[String]) -> Vec<String> {
    let mut all = fields.map(jira::csv).unwrap_or_default();
    all.extend(field.iter().cloned());
    all
}

/// Run `f` for each input. One input returns its result directly; several
/// return `{items: [{input, ok, data|error}]}` (like twg's batch reads).
fn batch(inputs: Vec<String>, strict: bool, f: impl Fn(&str) -> Result<Value>) -> Result<Value> {
    if inputs.is_empty() {
        bail!("pass at least one ID");
    }
    if inputs.len() == 1 {
        return f(&inputs[0]);
    }
    let mut failed = 0;
    let items: Vec<Value> = inputs
        .iter()
        .map(|i| match f(i) {
            Ok(v) => json!({"input": i, "ok": true, "data": v}),
            Err(e) => {
                failed += 1;
                json!({"input": i, "ok": false, "error": format!("{e:#}")})
            }
        })
        .collect();
    if strict && failed > 0 {
        bail!("{failed} of {} reads failed: {}", items.len(), serde_json::to_string(&items)?);
    }
    Ok(json!({"items": items}))
}

fn list_edit(set: Option<String>, add: Option<String>, remove: Option<String>) -> Option<jira::ListEdit> {
    if set.is_none() && add.is_none() && remove.is_none() {
        return None;
    }
    Some(jira::ListEdit {
        set: set.as_deref().map(jira::csv),
        add: add.as_deref().map(jira::csv).unwrap_or_default(),
        remove: remove.as_deref().map(jira::csv).unwrap_or_default(),
    })
}

fn workitem(c: &client::Client, cmd: WorkitemCmd) -> Result<Value> {
    match cmd {
        WorkitemCmd::Get { ids, id_flags, issue_id, key, fields, field, comments, remote_links, full, strict } => {
            let inputs: Vec<String> =
                ids.into_iter().chain(id_flags).chain(issue_id).chain(key).map(|s| issue_ref(&s)).collect();
            let opts = jira::GetOpts { fields: fields_arg(fields.as_deref(), &field), comments, remote_links, full };
            batch(inputs, strict, |k| jira::get(c, k, &opts))
        }
        WorkitemCmd::Search { text, limit, first, after, fields } => {
            let fields = fields_arg(fields.as_deref(), &[]);
            jira::search(c, &text.join(" "), limit.or(first).unwrap_or(25), &fields, after.as_deref())
        }
        WorkitemCmd::Query { jql_arg, jql, limit, first, after, fields } => {
            let Some(jql) = jql.or(jql_arg) else { bail!("pass --jql or a JQL argument") };
            let fields = fields_arg(fields.as_deref(), &[]);
            jira::query(c, &jql, limit.or(first).unwrap_or(25), &fields, after.as_deref())
        }
        WorkitemCmd::Create {
            space,
            issue_type,
            summary,
            description,
            description_format,
            assignee,
            reporter,
            priority,
            labels,
            parent,
            field,
            fields_json,
            yes: _,
        } => {
            let description = description.map(text_arg).transpose()?;
            jira::create(
                c,
                jira::NewWorkitem {
                    space: &space,
                    issue_type: &issue_type,
                    summary: &summary,
                    description: description.as_deref().map(|d| (d, description_format)),
                    assignee: assignee.as_deref(),
                    reporter: reporter.as_deref(),
                    priority: priority.as_deref(),
                    labels: labels.as_deref().map(jira::csv).unwrap_or_default(),
                    parent: parent.as_deref(),
                    extra: jira::extra_fields(fields_json.as_deref(), &field)?,
                },
            )
        }
        WorkitemCmd::Update(u) => {
            let description = u.description.map(text_arg).transpose()?;
            let comment = u.comment.map(text_arg).transpose()?;
            let key = issue_ref(&u.id);
            jira::update(
                c,
                &key,
                jira::Update {
                    summary: u.summary.as_deref(),
                    description: description.as_deref().map(|d| (d, u.description_format)),
                    assignee: u.assignee.as_deref(),
                    issue_type: u.issue_type.as_deref(),
                    priority: u.priority.as_deref(),
                    labels: list_edit(u.labels, u.add_labels, u.remove_labels),
                    components: list_edit(u.components, u.add_components, u.remove_components),
                    fix_versions: list_edit(u.fix_versions, u.add_fix_versions, u.remove_fix_versions),
                    parent: u.parent.as_deref(),
                    due_date: u.due_date.as_deref(),
                    resolution: u.resolution.as_deref(),
                    extra: jira::extra_fields(u.fields_json.as_deref(), &u.field)?,
                    status: u.status.as_deref(),
                    transition_comment: u.transition_comment.as_deref().map(|t| (t, u.transition_comment_format)),
                    comment: comment.as_deref().map(|t| (t, u.comment_format)),
                },
            )
        }
        WorkitemCmd::Comment {
            cmd: CommentCmd::Create { issue_id, body, body_format, internal, visibility_role, visibility_group },
        } => {
            let visibility = match (&visibility_role, &visibility_group) {
                (Some(r), _) => Some(("role", r.as_str())),
                (None, Some(g)) => Some(("group", g.as_str())),
                _ => None,
            };
            jira::comment(c, &issue_ref(&issue_id), &text_arg(body)?, body_format, visibility, internal)
        }
        WorkitemCmd::Transition { id, transition_id, field, fields_json, transition_comment, transition_comment_format } => {
            jira::transition(
                c,
                &issue_ref(&id),
                transition_id.as_deref(),
                jira::extra_fields(fields_json.as_deref(), &field)?,
                transition_comment.as_deref().map(|t| (t, transition_comment_format)),
            )
        }
        WorkitemCmd::Transitions { cmd: TransitionsCmd::Query { id } } => jira::transitions(c, &issue_ref(&id)),
    }
}

fn confluence_cmd(c: &client::Client, cmd: ConfCmd) -> Result<Out> {
    let data = match cmd {
        ConfCmd::Search { cmd: ConfSearchCmd::Query { cql_arg, cql, limit, excerpt, cursor } } => {
            let cql = cql.or_else(|| (!cql_arg.is_empty()).then(|| cql_arg.join(" ")));
            let Some(cql) = cql else { bail!("pass --cql or a CQL argument") };
            confluence::search_query(c, &cql, limit, excerpt, cursor.as_deref())?
        }
        ConfCmd::Search {
            cmd: ConfSearchCmd::Text { text, query, limit, updated_since, created_since, excerpt, cursor },
        } => {
            let text = query.or_else(|| (!text.is_empty()).then(|| text.join(" ")));
            let Some(text) = text else { bail!("pass search text or --query") };
            let cql = confluence::text_cql(&text, updated_since.as_deref(), created_since.as_deref())?;
            confluence::search_query(c, &cql, limit, excerpt, cursor.as_deref())?
        }
        ConfCmd::Content { cmd } => match cmd {
            ContentCmd::Get { refs, id_flags, detail, format, body_only, draft, include_metadata: _, strict } => {
                let inputs: Vec<String> = refs.into_iter().chain(id_flags).collect();
                let detail = if body_only { confluence::Detail::Full } else { detail.unwrap_or_default() };
                let opts = confluence::GetOpts { detail, format, draft };
                if body_only {
                    let [one] = inputs.as_slice() else { bail!("--body-only takes exactly one ID") };
                    let v = confluence::get(c, one, &opts)?;
                    return Ok(Out::Raw(v["body"].as_str().unwrap_or_default().to_string()));
                }
                batch(inputs, strict, |r| confluence::get(c, r, &opts))?
            }
            ContentCmd::Create { space_id, parent_id, content_type, title, body, body_file, format, draft, .. } => {
                let body = body_arg(body, body_file)?.unwrap_or_default();
                confluence::create(
                    c,
                    confluence::NewContent {
                        space: &space_id,
                        content_type,
                        title: &title,
                        body: &body,
                        format,
                        parent: parent_id.as_deref(),
                        draft,
                    },
                )?
            }
            ContentCmd::Update {
                reference,
                id,
                snapshot_token,
                title,
                version_message,
                minor_edit,
                body,
                body_file,
                format,
                dry_run,
                draft,
                ..
            } => {
                let Some(reference) = reference.or(id) else { bail!("pass a content ID/URL or --id") };
                let body = body_arg(body, body_file)?;
                confluence::update(
                    c,
                    &reference,
                    confluence::Edit {
                        title: title.as_deref(),
                        body: body.as_deref().map(|b| (b, format)),
                        version_message: version_message.as_deref(),
                        minor_edit,
                        snapshot_token: snapshot_token.as_deref(),
                        dry_run,
                        draft,
                    },
                )?
            }
            ContentCmd::List { space, content_type, status, limit, cursor } => {
                confluence::list(c, &space, content_type, &status, limit, cursor.as_deref())?
            }
        },
        ConfCmd::Space { cmd: SpaceCmd::List { kind, status, keys, limit, cursor } } => {
            confluence::spaces(c, kind.as_deref(), status.as_deref(), keys.as_deref(), limit, cursor.as_deref())?
        }
        ConfCmd::Space { cmd: SpaceCmd::Get { identifier } } => confluence::space(c, &identifier)?,
    };
    Ok(Out::Data(data))
}

fn bb_cmd(cfg: &Config, cmd: BbCmd) -> Result<Value> {
    let (c, default_ws) = cfg.bitbucket()?;
    let dws = default_ws.as_deref();
    let path = |r: &RepoArgs, slug: Option<&str>| {
        bitbucket::repo_path(slug.or(r.repo.as_deref()), r.workspace.as_deref(), dws)
    };
    match cmd {
        BbCmd::Repo { cmd: RepoCmd::Query { query, limit, workspace, project, language, sort, role } } => {
            let q = bitbucket::RepoQuery {
                query: query.as_deref(),
                project: project.as_deref(),
                language: language.as_deref(),
                sort: &sort,
                role: role.as_deref(),
                limit,
            };
            bitbucket::repo_query(&c, bitbucket::workspace_or(workspace.as_deref(), dws)?, &q)
        }
        BbCmd::Repo { cmd: RepoCmd::Get { slug, repo } } => bitbucket::repo_get(&c, &path(&repo, slug.as_deref())?),
        BbCmd::PullRequests { cmd } => match cmd {
            PrCmd::Query {
                text,
                state,
                author,
                source,
                dest,
                query,
                created_since,
                created_before,
                updated_since,
                updated_before,
                limit,
                repo,
            } => {
                let text = query.or_else(|| (!text.is_empty()).then(|| text.join(" ")));
                let q = bitbucket::PrQuery {
                    state: state.as_deref(),
                    author: author.as_deref(),
                    source: source.as_deref(),
                    dest: dest.as_deref(),
                    text: text.as_deref(),
                    created_since: created_since.as_deref(),
                    created_before: created_before.as_deref(),
                    updated_since: updated_since.as_deref(),
                    updated_before: updated_before.as_deref(),
                    limit,
                };
                bitbucket::pr_query(&c, &path(&repo, None)?, &q)
            }
            PrCmd::Get { ids, comments, diff, statuses, full, strict, repo } => {
                let p = path(&repo, None)?;
                let o = bitbucket::PrGet { comments: comments || full, diff: diff || full, statuses: statuses || full };
                batch(ids, strict, |id| bitbucket::pr_get(&c, &p, bitbucket::pr_number(id)?, &o))
            }
            PrCmd::Create {
                title,
                source,
                dest,
                description,
                description_file,
                reviewer_uuid,
                reviewer_account_id,
                close_source_branch,
                draft,
                repo,
            } => {
                let description = body_arg(description, description_file)?;
                bitbucket::pr_create(
                    &c,
                    &path(&repo, None)?,
                    bitbucket::NewPr {
                        title: &title,
                        source: &source,
                        dest: dest.as_deref(),
                        description: description.as_deref(),
                        reviewer_uuids: reviewer_uuid.as_deref().map(jira::csv).unwrap_or_default(),
                        reviewer_account_ids: reviewer_account_id.as_deref().map(jira::csv).unwrap_or_default(),
                        close_source_branch,
                        draft,
                    },
                )
            }
            PrCmd::Comment { cmd: PrCommentCmd::Create { pull_request, text, reply_to, path: file, line, repo } } => {
                let text = text_arg(text)?;
                bitbucket::pr_comment(
                    &c,
                    &path(&repo, None)?,
                    bitbucket::pr_number(&pull_request)?,
                    bitbucket::NewComment { text: &text, reply_to, path: file.as_deref(), line },
                )
            }
        },
        BbCmd::Pipeline { cmd: PipelineCmd::Query { state, status, branch, trigger, limit, repo } } => {
            let q = bitbucket::PipelineQuery {
                state: state.as_deref(),
                status: status.as_deref(),
                branch: branch.as_deref(),
                trigger: trigger.as_deref(),
                limit,
            };
            bitbucket::pipeline_query(&c, &path(&repo, None)?, &q)
        }
        BbCmd::Pipeline { cmd: PipelineCmd::Get { pipeline, logs, failed_steps, lines, step, repo } } => {
            let o = bitbucket::PipelineGet { logs, failed_steps, lines, step: step.as_deref() };
            bitbucket::pipeline_get(&c, &path(&repo, None)?, &pipeline, &o)
        }
    }
}

fn api(cfg: &Config, product: Product, path: &str, method: &str, data: Option<String>, query: &[String]) -> Result<Value> {
    let mut path = if path.starts_with('/') || path.starts_with("http") { path.to_string() } else { format!("/{path}") };
    let c = match product {
        Product::Jira => cfg.jira()?,
        Product::Confluence => cfg.confluence()?,
        Product::Bitbucket => {
            let (c, ws) = cfg.bitbucket()?;
            if path.contains("{workspace}") {
                path = path.replace("{workspace}", bitbucket::workspace_or(None, ws.as_deref())?);
            }
            c
        }
    };
    let body = match data {
        None => None,
        Some(d) => {
            let text = match d.strip_prefix('@') {
                Some(file) => std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?,
                None => text_arg(d)?,
            };
            Some(serde_json::from_str::<Value>(&text).context("--data is not valid JSON")?)
        }
    };
    let query: Vec<(&str, &str)> = query
        .iter()
        .map(|kv| kv.split_once('=').with_context(|| format!("query '{kv}' must be KEY=VALUE")))
        .collect::<Result<_>>()?;
    c.request(&method.to_ascii_uppercase(), &path, &query, body.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

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
            "atlx confluence search query --cql type=page --excerpt",
            "atlx confluence search text launch plan --since 7d",
            "atlx confluence content get 12345 --detail full --format md",
            "atlx confluence content create --space-id ENG --content-type page --title T --body-file f --format html --ack-body-formats -y",
            "atlx confluence content update 12345 --snapshot-token 3 --body x --format markdown --dry-run",
            "atlx confluence content list --space ENG --status archived",
            "atlx confluence space get ENG",
            "atlx bb repo query --workspace acme",
            "atlx bitbucket repo get --workspace acme --repo api",
            "atlx bb pull-requests query --workspace acme --repo api --state MERGED",
            "atlx bb prs get 930 931 --statuses --comments --repo acme/api",
            "atlx bb pull-requests create --title t --source feat --dest main --repo api",
            "atlx bb pull-requests comment create --pull-request 930 --text LGTM --repo api",
            "atlx bb pipeline query --repo api --status FAILED",
            "atlx bb pipeline get --pipeline 1234 --logs --failed-steps --repo api",
            "atlx whoami -o text",
            "atlx setup bitbucket --workspace acme",
        ] {
            let argv: Vec<String> = shell_words(args);
            if let Err(e) = Cli::try_parse_from(&argv) {
                panic!("{args}: {e}");
            }
        }
    }

    /// Minimal splitter for the test strings (single quotes only).
    fn shell_words(s: &str) -> Vec<String> {
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
    fn issue_refs() {
        assert_eq!(issue_ref("PROJ-1"), "PROJ-1");
        assert_eq!(issue_ref("https://a.atlassian.net/browse/PROJ-9?focusedCommentId=1"), "PROJ-9");
        assert_eq!(issue_ref("ari:cloud:jira:abc:issue/10001"), "10001");
    }

    #[test]
    fn rejects_conflicting_jira_update_list_flags() {
        for bad in [
            "atlx jira workitem update --id PROJ-1 --labels a --add-labels b",
            "atlx jira workitem update --id PROJ-1 --labels a --remove-labels b",
            "atlx jira workitem update --id PROJ-1 --components a --add-components b",
            "atlx jira workitem update --id PROJ-1 --components a --remove-components b",
            "atlx jira workitem update --id PROJ-1 --fix-versions a --add-fix-versions b",
            "atlx jira workitem update --id PROJ-1 --fix-versions a --remove-fix-versions b",
        ] {
            assert!(Cli::try_parse_from(shell_words(bad)).is_err(), "expected error for: {bad}");
        }
        let ok = "atlx jira workitem update --id PROJ-1 --add-labels a --remove-labels b";
        assert!(Cli::try_parse_from(shell_words(ok)).is_ok());
    }

    #[test]
    fn rejects_duplicate_bitbucket_repo_flags() {
        for bad in [
            "atlx bb repo get --repo api --repo web",
            "atlx bb prs query --repo api --repo web",
            "atlx bb pipeline query --repo api --repo web",
        ] {
            assert!(Cli::try_parse_from(shell_words(bad)).is_err(), "expected error for: {bad}");
        }
    }

    #[test]
    fn parses_confluence_content_get_strict_and_space_list_cursor() {
        let cli = Cli::try_parse_from(shell_words("atlx confluence content get 123 456 --strict")).unwrap();
        match cli.cmd {
            Cmd::Confluence { cmd: ConfCmd::Content { cmd: ContentCmd::Get { refs, strict, .. } } } => {
                assert_eq!(refs, vec!["123", "456"]);
                assert!(strict);
            }
            _ => panic!("unexpected parsed command"),
        }

        let cli = Cli::try_parse_from(shell_words("atlx confluence space list --limit 10 --cursor cur%3D")).unwrap();
        match cli.cmd {
            Cmd::Confluence { cmd: ConfCmd::Space { cmd: SpaceCmd::List { limit, cursor, .. } } } => {
                assert_eq!(limit, 10);
                assert_eq!(cursor.as_deref(), Some("cur%3D"));
            }
            _ => panic!("unexpected parsed command"),
        }
    }

    #[test]
    fn batch_respects_strict_flag() {
        let non_strict = batch(vec!["ok".into(), "bad".into()], false, |id| {
            if id == "bad" { bail!("boom") } else { Ok(json!({"id": id})) }
        })
        .unwrap();
        assert_eq!(non_strict["items"][0]["ok"], true);
        assert_eq!(non_strict["items"][1]["ok"], false);

        let err = batch(vec!["ok".into(), "bad".into()], true, |id| {
            if id == "bad" { bail!("boom") } else { Ok(json!({"id": id})) }
        })
        .unwrap_err();
        assert!(err.to_string().contains("1 of 2 reads failed"), "{err}");
    }
}
