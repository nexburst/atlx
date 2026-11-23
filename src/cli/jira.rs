//! `atlx jira`: clap definitions and dispatch.
//!
//! Grammar: `atlx jira <resource> [<sub-resource>] <verb>`. The thing a verb
//! acts on is positional; its parents are named flags (`--issue`,
//! `--project`, `--board`, ...). Lists take `--limit`, `--cursor` and
//! `--all`; destructive commands need `--yes` (see [`registry::META`]).

use super::util::{self, DEFAULT_CONCURRENCY, concurrency_arg};
use crate::client::{self, ApiError, ErrorKind};
use crate::jira::{self, Listed, records};
use crate::markup::BodyFormat;
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;
use std::path::PathBuf;

mod agile;
mod dashboard;
mod filter;
mod issue;
mod project;
mod registry;

pub(crate) use registry::META;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum JiraCmd {
    /// Issues: find, read, create, change, transition, and their comments,
    /// worklogs, attachments, links, watchers, votes and properties
    Issue {
        #[command(subcommand)]
        cmd: issue::IssueCmd,
    },
    /// Projects, their versions and components
    Project {
        #[command(subcommand)]
        cmd: project::ProjectCmd,
    },
    /// Agile boards and their quick filters
    Board {
        #[command(subcommand)]
        cmd: agile::BoardCmd,
    },
    /// Agile sprints
    Sprint {
        #[command(subcommand)]
        cmd: agile::SprintCmd,
    },
    /// Saved filters, their shares, columns and subscriptions
    Filter {
        #[command(subcommand)]
        cmd: filter::FilterCmd,
    },
    /// Dashboards, their gadgets and gadget properties
    Dashboard {
        #[command(subcommand)]
        cmd: dashboard::DashboardCmd,
    },
    /// Fields of the site, of a create screen or of an issue's edit screen
    Field {
        #[command(subcommand)]
        cmd: FieldCmd,
    },
    /// Issue types
    IssueType {
        #[command(subcommand)]
        cmd: IssueTypeCmd,
    },
    /// Issue statuses
    Status {
        #[command(subcommand)]
        cmd: StatusCmd,
    },
    /// Issue priorities
    Priority {
        #[command(subcommand)]
        cmd: PriorityCmd,
    },
    /// Issue link types
    LinkType {
        #[command(subcommand)]
        cmd: LinkTypeCmd,
    },
    /// The transitions an issue can make now
    Transition {
        #[command(subcommand)]
        cmd: TransitionCmd,
    },
    /// Atlassian accounts
    User {
        #[command(subcommand)]
        cmd: super::user::UserCmd,
    },
}

// --- shared arguments --------------------------------------------------------

/// `--limit`, `--cursor`, `--all` on every list.
#[derive(Args, Debug)]
pub(crate) struct Paging {
    /// How many records to return in total [default: 25, at most 1000]
    #[arg(long, value_name = "N")]
    limit: Option<usize>,
    /// Continue after a previous result: pass its `next`
    #[arg(long)]
    cursor: Option<String>,
    /// Return up to 1000 records
    #[arg(long, conflicts_with = "limit")]
    all: bool,
}

impl Paging {
    pub(crate) fn limit(&self) -> usize {
        crate::paging::effective_limit(self.limit, self.all)
    }

    pub(crate) fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Page through a list fetched whole.
    pub(crate) fn local(&self, items: Vec<Value>) -> Result<Value> {
        Ok(listed(jira::local_list(
            items,
            self.limit(),
            self.cursor(),
        )?))
    }
}

/// `--body-format` of a Jira rich-text input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum InFormat {
    /// Markdown, converted to ADF; refused when it would lose anything
    #[default]
    #[value(alias = "md")]
    Markdown,
    /// An ADF JSON document, sent as given
    Adf,
    /// Plain text: blank lines separate paragraphs
    Plain,
}

impl From<InFormat> for BodyFormat {
    fn from(f: InFormat) -> Self {
        match f {
            InFormat::Markdown => BodyFormat::Markdown,
            InFormat::Adf => BodyFormat::Adf,
            InFormat::Plain => BodyFormat::Plain,
        }
    }
}

/// `--body-format` of printed Jira rich text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum OutFormat {
    /// Markdown, with a marker and a `lossy_body` warning where something
    /// cannot be shown
    #[default]
    #[value(alias = "md")]
    Markdown,
    /// The ADF JSON document
    Adf,
}

impl From<OutFormat> for BodyFormat {
    fn from(f: OutFormat) -> Self {
        match f {
            OutFormat::Markdown => BodyFormat::Markdown,
            OutFormat::Adf => BodyFormat::Adf,
        }
    }
}

/// `--yes` on destructive commands. Nothing reads it here: `atlx` refuses a
/// command registered as destructive before it runs when `--yes` is
/// missing.
#[derive(Args, Debug)]
#[allow(dead_code)]
pub(crate) struct Confirm {
    /// Confirm this destructive change (required; preview with --dry-run)
    #[arg(long, short)]
    yes: bool,
}

/// `--strict` and `--concurrency` on multi-ID reads.
#[derive(Args, Debug)]
pub(crate) struct Many {
    /// Fail the whole command if any ID fails
    #[arg(long)]
    strict: bool,
    /// How many to fetch at once (1 to 20)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY, value_parser = concurrency_arg)]
    concurrency: usize,
}

impl Many {
    /// Run `f` for each ID (`-` reads them from stdin).
    pub(crate) fn run(
        &self,
        ids: Vec<String>,
        f: impl Fn(&str) -> Result<Value> + Sync,
    ) -> Result<Value> {
        util::batch(ids, self.strict, self.concurrency, f)
    }
}

/// A rich-text input: inline (`-` reads stdin) or from a file.
pub(crate) fn text_input(inline: Option<String>, file: Option<PathBuf>) -> Result<Option<String>> {
    util::body_arg(inline, file.map(|p| p.display().to_string()))
}

/// A rich-text input as ADF.
pub(crate) fn adf_input(
    inline: Option<String>,
    file: Option<PathBuf>,
    f: InFormat,
) -> Result<Option<Value>> {
    text_input(inline, file)?
        .map(|t| jira::adf_body(&t, f.into()))
        .transpose()
}

/// A list result: the items and `next`.
pub(crate) fn listed((items, next): Listed) -> Value {
    envelope().next_cursor(next).data(Value::Array(items))
}

/// The result of a multi-item write: what was done, with each failure in
/// the envelope's `errors` (exit 3). Every input failing is an error.
pub(crate) fn outcome((done, failed): jira::bulk::Outcome) -> Result<Value> {
    if done.is_empty()
        && let Some((input, mut err)) = failed.first().cloned()
    {
        err.message = format!("{input}: {}", err.message);
        return Err(err.into());
    }
    for (input, err) in &failed {
        util::push_error(input, err);
    }
    if !failed.is_empty() {
        util::mark_partial();
    }
    Ok(Value::Array(done))
}

/// Issue keys from positionals (`-` reads stdin), normalized.
pub(crate) fn issue_keys(inputs: Vec<String>) -> Result<Vec<String>> {
    let keys: Vec<String> = util::ids_arg(inputs)?
        .iter()
        .map(|s| jira::issue_ref(s))
        .collect();
    if keys.is_empty() {
        return Err(ApiError::usage(
            "pass at least one issue key (or `-` to read them from stdin)",
        )
        .into());
    }
    Ok(keys)
}

/// Read `--body-format` for the records printed by this run.
pub(crate) fn print_bodies_as(f: OutFormat) {
    records::set_body_format(f.into());
}

/// `.atlx.toml`'s `jira.project`, for commands whose project may be left
/// out.
pub(crate) fn default_project() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    crate::config::LocalDefaults::discover(&cwd)
        .ok()?
        .jira
        .project
}

pub(crate) fn not_found(what: String) -> anyhow::Error {
    ApiError::new(ErrorKind::NotFound, what).into()
}

// --- metadata resources ------------------------------------------------------

#[derive(Subcommand)]
pub(crate) enum FieldCmd {
    /// Fields: the site's, a create screen's (--project, --type) or an
    /// issue's edit screen (--issue), with what --field values they take
    List {
        /// Name or ID contains (site fields)
        #[arg(short, long)]
        query: Option<String>,
        /// Only custom fields
        #[arg(long)]
        custom: bool,
        /// The create screen of this project (with --type)
        #[arg(long, requires = "issue_type", conflicts_with = "issue")]
        project: Option<String>,
        /// Issue type name or ID (with --project)
        #[arg(long = "type", requires = "project")]
        issue_type: Option<String>,
        /// The edit screen of this issue
        #[arg(long)]
        issue: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
}

#[derive(Subcommand)]
pub(crate) enum IssueTypeCmd {
    /// Issue types: the site's, or a project's
    List {
        /// Only this project's
        #[arg(long)]
        project: Option<String>,
        /// Hierarchy level (with --project): -1 subtask, 0 standard, 1 epic
        #[arg(long, allow_hyphen_values = true, requires = "project")]
        level: Option<i64>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Issue types by ID or ARI
    Get {
        /// Issue type IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
}

#[derive(Subcommand)]
pub(crate) enum StatusCmd {
    /// Statuses: the site's, or each status a project uses
    List {
        /// Only this project's
        #[arg(long)]
        project: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Statuses by ID or ARI
    Get {
        /// Status IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
}

#[derive(Subcommand)]
pub(crate) enum PriorityCmd {
    /// The site's priorities
    List {
        #[command(flatten)]
        paging: Paging,
    },
    /// Priorities by ID or ARI
    Get {
        /// Priority IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
}

#[derive(Subcommand)]
pub(crate) enum LinkTypeCmd {
    /// The site's issue link types
    List {
        #[command(flatten)]
        paging: Paging,
    },
}

#[derive(Subcommand)]
pub(crate) enum TransitionCmd {
    /// The transitions an issue can make now, with the fields they require
    List {
        /// Issue key, ID or URL
        #[arg(long)]
        issue: String,
        #[command(flatten)]
        paging: Paging,
    },
}

pub(crate) fn jira_cmd(c: &client::Client, cmd: JiraCmd) -> Result<Value> {
    match cmd {
        JiraCmd::Issue { cmd } => issue::run(c, cmd),
        JiraCmd::Project { cmd } => project::run(c, cmd),
        JiraCmd::Board { cmd } => agile::board(c, cmd),
        JiraCmd::Sprint { cmd } => agile::sprint(c, cmd),
        JiraCmd::Filter { cmd } => filter::run(c, cmd),
        JiraCmd::Dashboard { cmd } => dashboard::run(c, cmd),
        JiraCmd::User { cmd } => super::user::user_cmd(c, cmd),
        JiraCmd::Field {
            cmd:
                FieldCmd::List {
                    query,
                    custom,
                    project,
                    issue_type,
                    issue,
                    paging,
                },
        } => {
            let all = match (project, issue_type, issue) {
                (Some(p), Some(t), _) => jira::create_fields(c, &jira::project_ref(&p), &t)?,
                (_, _, Some(i)) => jira::edit_fields(c, &jira::issue_ref(&i))?,
                _ => jira::fields(c, query.as_deref(), custom)?,
            };
            paging.local(all)
        }
        JiraCmd::IssueType { cmd } => match cmd {
            IssueTypeCmd::List {
                project,
                level,
                paging,
            } => paging.local(jira::issue_types(c, project.as_deref(), level)?),
            IssueTypeCmd::Get { ids, many } => many.run(ids, |id| jira::issue_type(c, id)),
        },
        JiraCmd::Status { cmd } => match cmd {
            StatusCmd::List { project, paging } => {
                paging.local(jira::statuses(c, project.as_deref())?)
            }
            StatusCmd::Get { ids, many } => many.run(ids, |id| jira::status(c, id)),
        },
        JiraCmd::Priority { cmd } => match cmd {
            PriorityCmd::List { paging } => Ok(listed(jira::priorities(
                c,
                paging.limit(),
                paging.cursor(),
            )?)),
            PriorityCmd::Get { ids, many } => many.run(ids, |id| jira::priority(c, id)),
        },
        JiraCmd::LinkType {
            cmd: LinkTypeCmd::List { paging },
        } => paging.local(jira::link_types(c)?),
        JiraCmd::Transition {
            cmd: TransitionCmd::List { issue, paging },
        } => paging.local(jira::transitions(c, &jira::issue_ref(&issue))?),
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    fn parses(args: &str) -> bool {
        Cli::try_parse_from(shell_words(args)).is_ok()
    }

    #[test]
    fn parses_the_jira_tree() {
        for ok in [
            "atlx jira issue list --jql 'project = P' --limit 5 --cursor t:abc",
            "atlx jira issue list --project P --status Open --status Done --assignee me --type Bug --label x --all",
            "atlx jira issue list --filter 10100",
            "atlx jira issue list --sprint 42 --jql 'status = Done'",
            "atlx jira issue list --backlog 12 --quick-filter 1 --quick-filter 2",
            "atlx jira issue search login timeout --project P",
            "atlx jira issue get P-1 P-2 --strict --concurrency 3 --body-format adf --body-offset 100",
            "atlx jira issue get -",
            "atlx jira issue create --project P --type Bug --summary S --description x --body-format markdown --label a --label b --field 'Story Points=3' --sprint 42",
            "atlx jira issue create --bulk-file issues.json",
            "atlx jira issue update P-1 --summary S --description-file d.md --add-label x --remove-label y --due none",
            "atlx jira issue update P-1 --description x --allow-lossy --yes",
            "atlx jira issue delete P-1 P-2 --delete-subtasks --yes",
            "atlx jira issue transition P-1 P-2 --to Done --comment shipped --field resolution=Fixed",
            "atlx jira issue assign P-1 --to me",
            "atlx jira issue assign P-1 --to none",
            "atlx jira issue archive P-1 --yes",
            "atlx jira issue restore P-1 --yes",
            "atlx jira issue comment list --issue P-1 --newest-first --limit 5",
            "atlx jira issue comment get 10001 --issue P-1",
            "atlx jira issue comment create --issue P-1 --body hi --body-format plain --visibility-role Developers",
            "atlx jira issue comment update 10001 --issue P-1 --body-file b.md",
            "atlx jira issue comment delete 10001 --issue P-1 --yes",
            "atlx jira issue worklog list --issue P-1 --started-after 7d",
            "atlx jira issue worklog list --changed-since 2026-01-01",
            "atlx jira issue worklog create --issue P-1 --time-spent 1h --started 2026-01-05 --comment x",
            "atlx jira issue worklog update 7 --issue P-1 --time-spent-seconds 60 --adjust-estimate leave",
            "atlx jira issue worklog get 7 --issue P-1",
            "atlx jira issue worklog delete 7 --issue P-1 --adjust-estimate manual --increase-by 1h --yes",
            "atlx jira issue attachment list --issue P-1",
            "atlx jira issue attachment get 10001 10002",
            "atlx jira issue attachment download 10001 --path a.bin --force --thumbnail",
            "atlx jira issue attachment upload --issue P-1 --file a --file b",
            "atlx jira issue attachment delete 10001 --yes",
            "atlx jira issue link list --issue P-1",
            "atlx jira issue link create --issue P-1 --target P-2 --target P-3 --type Blocks",
            "atlx jira issue link delete 10500 --yes",
            "atlx jira issue weblink list --issue P-1",
            "atlx jira issue weblink create --issue P-1 --url https://e.x --title T --relationship 'mentioned in'",
            "atlx jira issue weblink delete 7 --issue P-1 --yes",
            "atlx jira issue weblink delete --issue P-1 --url https://e.x --yes",
            "atlx jira issue watcher list --issue P-1",
            "atlx jira issue watcher create --issue P-1",
            "atlx jira issue watcher delete abc --issue P-1 --yes",
            "atlx jira issue vote get --issue P-1",
            "atlx jira issue vote create --issue P-1",
            "atlx jira issue vote delete --issue P-1 --yes",
            "atlx jira issue property list --issue P-1",
            "atlx jira issue property get k --issue P-1",
            "atlx jira issue property update k --issue P-1 --value '{\"a\":1}'",
            "atlx jira issue property delete k --issue P-1 --yes",
            "atlx jira issue changelog list --issue P-1 --limit 5",
            "atlx jira transition list --issue P-1",
            "atlx jira project list -q web --type software --status archived",
            "atlx jira project list --board 12",
            "atlx jira project get P Q",
            "atlx jira project create --key NEW --name New --type software",
            "atlx jira project update P --name N --category -1",
            "atlx jira project delete P --yes",
            "atlx jira project archive P --yes",
            "atlx jira project restore P --yes",
            "atlx jira project version list --project P --status released --order-by -releaseDate",
            "atlx jira project version get 5 6",
            "atlx jira project version create --project P --name 1.0 --released true",
            "atlx jira project version update 5 --archived false",
            "atlx jira project version delete 5 --move-fix-issues-to 6 --yes",
            "atlx jira project version merge 5 --into 6 --yes",
            "atlx jira project version move 5 --position First",
            "atlx jira project component list --project P",
            "atlx jira project component get 7",
            "atlx jira project component create --project P --name API",
            "atlx jira project component update 7 --description d",
            "atlx jira project component delete 7 --move-issues-to 8 --yes",
            "atlx jira board list --project P --type kanban -q team",
            "atlx jira board get 12 13",
            "atlx jira board create --name Team --type scrum --filter 100",
            "atlx jira board delete 12 --yes",
            "atlx jira board quick-filter list --board 12",
            "atlx jira board quick-filter get 1 --board 12",
            "atlx jira sprint list --board 12 --state active,future",
            "atlx jira sprint list --project P --current",
            "atlx jira sprint get 99",
            "atlx jira sprint create --board 12 --name S5 --goal ship",
            "atlx jira sprint update 99 --goal g",
            "atlx jira sprint start 99 --duration 2w",
            "atlx jira sprint complete 99 --move-open-to backlog",
            "atlx jira sprint delete 99 --yes",
            "atlx jira filter list -q open --owner me",
            "atlx jira filter list --favourite",
            "atlx jira filter get 9",
            "atlx jira filter create --name F --jql 'a = 1'",
            "atlx jira filter update 9 --jql 'a = 2' --favourite false --owner abc",
            "atlx jira filter delete 9 --yes",
            "atlx jira filter share list --filter 9",
            "atlx jira filter share create --filter 9 --permission-json '{\"type\":\"global\"}' --yes",
            "atlx jira filter share delete 3 --filter 9 --yes",
            "atlx jira filter column list --filter 9",
            "atlx jira filter column update --filter 9 --column summary --column status",
            "atlx jira filter column delete --filter 9 --yes",
            "atlx jira filter subscription list --filter 9",
            "atlx jira dashboard list -q team",
            "atlx jira dashboard get 10000",
            "atlx jira dashboard create --name D",
            "atlx jira dashboard update 10000 --name D2 --yes",
            "atlx jira dashboard copy 10000 --name D3",
            "atlx jira dashboard delete 10000 --yes",
            "atlx jira dashboard gadget list --dashboard 10000",
            "atlx jira dashboard gadget list --catalog",
            "atlx jira dashboard gadget create --dashboard 10000 --module-key k --row 0 --column 1",
            "atlx jira dashboard gadget update 5 --dashboard 10000 --title T",
            "atlx jira dashboard gadget delete 5 --dashboard 10000 --yes",
            "atlx jira dashboard property list --dashboard 10000 --item 5",
            "atlx jira dashboard property get k --dashboard 10000 --item 5",
            "atlx jira dashboard property update k --dashboard 10000 --item 5 --value 1",
            "atlx jira dashboard property delete k --dashboard 10000 --item 5 --yes",
            "atlx jira field list -q story --custom",
            "atlx jira field list --project P --type Bug",
            "atlx jira field list --issue P-1",
            "atlx jira issue-type list --project P --level -1",
            "atlx jira issue-type get 10001",
            "atlx jira status list --project P",
            "atlx jira status get 3 10000",
            "atlx jira priority list",
            "atlx jira priority get 1",
            "atlx jira link-type list",
            "atlx jira user search alice --limit 5",
            "atlx jira user get me abc a@example.com",
        ] {
            assert!(
                parses(ok),
                "{ok}: {:?}",
                Cli::try_parse_from(shell_words(ok)).err()
            );
        }
        for bad in [
            "atlx jira workitem get P-1",
            "atlx jira issue get P-1 --id P-2",
            "atlx jira issue list --max-results 5",
            "atlx jira issue list --limit 5 --all",
            "atlx jira issue comment list",
            "atlx jira issue create --summary S --bulk-file f",
            "atlx jira issue update P-1 --label a --add-label b",
            "atlx jira issue transition P-1",
            "atlx jira field list --type Bug",
            "atlx jira issue get P-1 --concurrency 0",
            "atlx jira space query",
        ] {
            assert!(!parses(bad), "expected an error for: {bad}");
        }
    }
}
