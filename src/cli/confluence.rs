//! `atlx confluence`: clap definitions and dispatch.
//!
//! The tree is `confluence <resource> [<sub-resource>] <verb>`: `page` (with
//! `comment`, `attachment`, `label`, `version`, `task` and `state`),
//! `space` (with `permission`), `template`, `label` (watches) and the
//! verb-less `search`.

use super::{Out, util};
use crate::client::{self, ApiError, DryRun};
use crate::confluence::{self, records};
use crate::markup::{Body, BodyFormat, parse_body};
use crate::paging::{self, Page, Position};
use crate::schema::{CommandMeta, Effect};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

mod comment;
mod content;
mod page;
mod space;

#[derive(Subcommand)]
pub(crate) enum ConfCmd {
    /// Search content with free text or CQL
    Search(SearchArgs),
    /// Pages and blog posts, with their comments, attachments, labels,
    /// versions, tasks and states
    Page {
        #[command(subcommand)]
        cmd: page::PageCmd,
    },
    /// Spaces and their permissions
    Space {
        #[command(subcommand)]
        cmd: space::SpaceCmd,
    },
    /// Page templates and blueprints
    Template {
        #[command(subcommand)]
        cmd: space::TemplateCmd,
    },
    /// Label watches (notifications about content that gets a label)
    Label {
        #[command(subcommand)]
        cmd: space::LabelCmd,
    },
}

#[derive(Args)]
pub(crate) struct SearchArgs {
    /// Free text: searches pages and blog posts, most recently changed
    /// first
    #[arg(value_name = "TEXT", conflicts_with = "cql")]
    text: Vec<String>,
    /// A CQL query instead of free text (`type = page AND space = ENG`)
    #[arg(long)]
    cql: Option<String>,
    /// With free text: changed on or after this date (YYYY-MM-DD) or
    /// within a duration (7d, 2w, 1m)
    #[arg(long, conflicts_with = "cql")]
    updated_since: Option<String>,
    /// With free text: created on or after this date or within a duration
    #[arg(long, conflicts_with = "cql")]
    created_since: Option<String>,
    #[command(flatten)]
    paging: Paging,
}

/// `--limit`, `--cursor`, `--all` on every list.
#[derive(Args, Clone, Debug)]
pub(crate) struct Paging {
    /// How many records to return (default 25, at most 1000)
    #[arg(long, value_name = "N")]
    limit: Option<usize>,
    /// Continue from a previous result's `next`
    #[arg(long)]
    cursor: Option<String>,
    /// Return every record, up to 1000
    #[arg(long, conflicts_with = "limit")]
    all: bool,
}

fn bad_cursor() -> anyhow::Error {
    ApiError::usage("this --cursor is not from a Confluence list's `next`").into()
}

/// A list envelope: `data` and `next` (null when there is no more).
fn list_out(items: Vec<Value>, next: Option<String>) -> Value {
    json!({"ok": true, "data": items, "next": next})
}

impl Paging {
    /// Collect records from a cursor-paged REST list. `fetch(cursor, size)`
    /// returns one REST page as `{results, nextCursor}`; `size` is at most
    /// `max_page`.
    fn collect(
        &self,
        max_page: usize,
        mut fetch: impl FnMut(Option<&str>, usize) -> Result<Value>,
    ) -> Result<Value> {
        let limit = paging::effective_limit(self.limit, self.all);
        let (items, next) = paging::collect(limit, self.cursor.as_deref(), |pos, wanted| {
            let token = match pos {
                Position::Start => None,
                Position::Token(t) => Some(t.as_str()),
                _ => return Err(bad_cursor()),
            };
            let v = fetch(token, wanted.clamp(1, max_page))?;
            Ok(Page {
                items: v["results"].as_array().cloned().unwrap_or_default(),
                next: v["nextCursor"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| Position::Token(s.to_string())),
            })
        })?;
        Ok(list_out(items, next))
    }

    /// Collect from an offset-paged list: `fetch(start, size)` returns
    /// `{results, nextCursor}` with `nextCursor` the next start offset.
    fn collect_offset(
        &self,
        mut fetch: impl FnMut(usize, usize) -> Result<Value>,
    ) -> Result<Value> {
        let limit = paging::effective_limit(self.limit, self.all);
        let (items, next) = paging::collect(limit, self.cursor.as_deref(), |pos, wanted| {
            let start = match pos {
                Position::Start => 0,
                Position::Offset(o) => usize::try_from(*o).map_err(|_| bad_cursor())?,
                _ => return Err(bad_cursor()),
            };
            let v = fetch(start, wanted)?;
            Ok(Page {
                items: v["results"].as_array().cloned().unwrap_or_default(),
                next: v["nextCursor"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .map(Position::Offset),
            })
        })?;
        Ok(list_out(items, next))
    }
}

/// `--body`, `--body-file`, `--body-format` on writes.
#[derive(Args, Clone, Debug)]
pub(crate) struct BodyArgs {
    /// The body (`-` reads stdin)
    #[arg(long, conflicts_with = "body_file", allow_hyphen_values = true)]
    body: Option<String>,
    /// Read the body from a file
    #[arg(long, value_name = "PATH")]
    body_file: Option<String>,
    /// What the body is written in: markdown (default), adf (JSON), storage
    /// (Confluence XHTML, checked and sent unchanged) or plain
    #[arg(long, value_enum, default_value_t)]
    body_format: BodyFormat,
}

impl BodyArgs {
    fn given(&self) -> bool {
        self.body.is_some() || self.body_file.is_some()
    }

    /// The parsed body, if one was given. Markdown that would lose content
    /// is refused here, before anything is sent.
    fn parse(&self) -> Result<Option<Body>> {
        match util::body_arg(self.body.clone(), self.body_file.clone())? {
            Some(text) => Ok(Some(parse_body(&text, self.body_format)?)),
            None => Ok(None),
        }
    }

    fn required(&self) -> Result<Body> {
        self.parse()?
            .ok_or_else(|| ApiError::usage("pass --body or --body-file").into())
    }

    /// Whether a full replace with this body has to keep everything the
    /// current body holds (Markdown and plain text cannot carry it all).
    fn needs_loss_check(&self) -> bool {
        matches!(self.body_format, BodyFormat::Markdown | BodyFormat::Plain)
    }
}

/// `--yes` on destructive commands (checked before the command runs).
#[derive(Args, Clone, Debug)]
pub(crate) struct Yes {
    /// Confirm this destructive change (preview it with --dry-run first)
    #[arg(long, short)]
    yes: bool,
}

/// `--strict` and `--concurrency` on multi-ID reads.
#[derive(Args, Clone, Debug)]
pub(crate) struct Batch {
    /// Fail the whole command if any ID fails
    #[arg(long)]
    strict: bool,
    /// How many IDs to read at once (1-20)
    #[arg(long, value_parser = util::concurrency_arg, default_value_t = util::DEFAULT_CONCURRENCY)]
    concurrency: usize,
}

impl Batch {
    fn run(&self, ids: Vec<String>, f: impl Fn(&str) -> Result<Value> + Sync) -> Result<Value> {
        util::batch(ids, self.strict, self.concurrency, f)
    }
}

/// Send a prepared page write. Under `--dry-run` the stopped request is
/// printed with a Markdown diff of the body.
fn send_prepared(c: &client::Client, p: &confluence::Prepared) -> Result<Out> {
    match confluence::send(c, p) {
        Ok(v) => Ok(Out::Data(v)),
        Err(e) => match DryRun::find(&e) {
            Some(d) => Ok(dry_run(d, confluence::diff(p))),
            None => Err(e),
        },
    }
}

/// The dry-run result with a diff, printed as the envelope `main` prints
/// for other dry runs plus `data.diff`.
fn dry_run(d: &DryRun, diff: Option<String>) -> Out {
    if util::text_output() {
        let mut s = format!(
            "dry run: would send {} {} (nothing was sent)",
            d.method, d.url
        );
        if let Some(df) = diff.filter(|d| !d.is_empty()) {
            s.push('\n');
            s.push_str(df.trim_end());
        }
        return Out::Raw(s);
    }
    let mut data = json!({"request": d.to_json()});
    if let Some(df) = diff {
        data["diff"] = df.into();
    }
    Out::Raw(json!({"ok": true, "dryRun": true, "data": data}).to_string())
}

pub(crate) fn confluence_cmd(c: &client::Client, cmd: ConfCmd) -> Result<Out> {
    match cmd {
        ConfCmd::Search(a) => Ok(Out::Data(search(c, a)?)),
        ConfCmd::Page { cmd } => page::run(c, cmd),
        ConfCmd::Space { cmd } => Ok(Out::Data(space::run(c, cmd)?)),
        ConfCmd::Template { cmd } => Ok(Out::Data(space::template(c, cmd)?)),
        ConfCmd::Label { cmd } => Ok(Out::Data(space::label(c, cmd)?)),
    }
}

/// Transition hook kept for `cli::mod`: Confluence has no command that runs
/// without a site any more.
pub(crate) fn offline(_cmd: &ConfCmd) -> Option<Result<Out>> {
    None
}

fn search(c: &client::Client, a: SearchArgs) -> Result<Value> {
    let cql = match (a.cql, a.text.is_empty()) {
        (Some(cql), _) => cql,
        (None, false) => confluence::text_cql(
            &a.text.join(" "),
            a.updated_since.as_deref(),
            a.created_since.as_deref(),
        )?,
        (None, true) => return Err(ApiError::usage("pass search text or --cql").into()),
    };
    a.paging.collect(100, |cur, n| {
        confluence::search_query(c, &cql, n, true, cur)
    })
}

// --- registry --------------------------------------------------------------------

const fn read(path: &'static str) -> CommandMeta {
    CommandMeta::new(path, Effect::Read)
}

const fn write(path: &'static str) -> CommandMeta {
    CommandMeta::new(path, Effect::Write)
}

const fn destructive(path: &'static str) -> CommandMeta {
    CommandMeta::new(path, Effect::Destructive)
}

pub(crate) const META: &[CommandMeta] = &[
    read("confluence search")
        .record(&records::SEARCH_HIT)
        .examples(&[
            "atlx confluence search 'deploy runbook' --limit 5",
            "atlx confluence search --cql 'type = page AND space = ENG AND title ~ \"release\"'",
            "atlx confluence search onboarding --updated-since 30d --fields id,title,url",
        ]),
    read("confluence page list")
        .record(&records::PAGE_ITEM)
        .examples(&[
            "atlx confluence page list --space ENG --limit 10",
            "atlx confluence page list --parent 12345 --all -o ids",
            "atlx confluence page list --space ENG --type blogpost --status draft",
        ]),
    read("confluence page get")
        .record(&records::PAGE)
        .examples(&[
            "atlx confluence page get 12345",
            "atlx confluence page get 12345 --section Rollback --fields title,version,body",
            "atlx confluence page get 12345 --outline",
            "atlx confluence page get Runbook --space ENG --body-format adf",
        ]),
    write("confluence page create")
        .record(&records::PAGE_ITEM)
        .examples(&[
            "atlx confluence page create --space ENG --title 'Release 1.2' --body-file notes.md",
            "atlx confluence page create --space ENG --parent 12345 --title Child --body '# Hello' --dry-run",
        ]),
    write("confluence page update")
        .record(&records::PAGE_ITEM)
        .examples(&[
            "atlx confluence page update 12345 --if-version 7 --title 'New title'",
            "atlx confluence page update 12345 --if-version 7 --body-file page.md --dry-run",
            "atlx confluence page update 12345 --if-version 7 --body-file page.json --body-format adf",
        ]),
    write("confluence page edit")
        .record(&records::PAGE_ITEM)
        .examples(&[
            "atlx confluence page edit 12345 --if-version 7 --edits '[{\"name\":\"replaceText\",\"find\":\"v1.1\",\"replace\":\"v1.2\"}]'",
            "atlx confluence page edit 12345 --if-version 7 --edits-file edits.json --dry-run",
        ]),
    destructive("confluence page delete").examples(&[
        "atlx confluence page delete 12345 --yes",
        "atlx confluence page delete 12345 --purge --yes",
        "atlx confluence page delete 12345 --draft --dry-run",
    ]),
    write("confluence page restore").examples(&["atlx confluence page restore 12345"]),
    write("confluence page move").examples(&[
        "atlx confluence page move 12345 --parent 67890",
        "atlx confluence page move 12345 --before 67890",
    ]),
    write("confluence page copy").examples(&[
        "atlx confluence page copy 12345 --parent 67890 --title 'Copy of runbook'",
        "atlx confluence page copy 12345 --to-space OPS --attachments --labels",
    ]),
    destructive("confluence page archive").examples(&[
        "atlx confluence page archive 12345 --yes",
        "atlx confluence page archive 12345 --wait --yes",
    ]),
    write("confluence page publish")
        .record(&records::PAGE_ITEM)
        .examples(&["atlx confluence page publish 12345 --title 'Final title'"]),
    write("confluence page watch").examples(&["atlx confluence page watch 12345"]),
    write("confluence page unwatch").examples(&["atlx confluence page unwatch 12345"]),
    read("confluence page tree").examples(&[
        "atlx confluence page tree 12345",
        "atlx confluence page tree 12345 --depth 4 --type whiteboard",
    ]),
    read("confluence page comment list")
        .record(&records::COMMENT)
        .examples(&[
            "atlx confluence page comment list --page 12345",
            "atlx confluence page comment list --page 12345 --inline --resolution open --replies",
        ]),
    read("confluence page comment get")
        .record(&records::COMMENT)
        .examples(&[
            "atlx confluence page comment get 777",
            "atlx confluence page comment get 777 778 --replies",
        ]),
    write("confluence page comment create")
        .record(&records::COMMENT)
        .examples(&[
            "atlx confluence page comment create --page 12345 --body 'Looks good'",
            "atlx confluence page comment create --page 12345 --body 'Typo?' --text-selection 'teh'",
            "atlx confluence page comment create --comment 777 --body Thanks",
        ]),
    write("confluence page comment update")
        .record(&records::COMMENT)
        .examples(&[
            "atlx confluence page comment update 777 --body 'Fixed wording' --if-version 2",
            "atlx confluence page comment update 777 --resolve",
        ]),
    destructive("confluence page comment delete")
        .examples(&["atlx confluence page comment delete 777 --yes"]),
    read("confluence page attachment list")
        .record(&records::ATTACHMENT)
        .examples(&[
            "atlx confluence page attachment list --page 12345",
            "atlx confluence page attachment list --page 12345 --media-type image/png",
        ]),
    read("confluence page attachment get")
        .record(&records::ATTACHMENT)
        .examples(&["atlx confluence page attachment get att456"]),
    read("confluence page attachment download").examples(&[
        "atlx confluence page attachment download att456 --out diagram.png",
    ]),
    write("confluence page attachment upload")
        .record(&records::ATTACHMENT)
        .examples(&[
            "atlx confluence page attachment upload --page 12345 --file diagram.png",
            "atlx confluence page attachment upload --page 12345 --file a.pdf --file b.pdf --comment v2",
        ]),
    destructive("confluence page attachment delete").examples(&[
        "atlx confluence page attachment delete att456 --yes",
        "atlx confluence page attachment delete att456 --purge --yes",
    ]),
    read("confluence page label list")
        .record(&records::LABEL)
        .examples(&["atlx confluence page label list --page 12345"]),
    write("confluence page label create")
        .record(&records::LABEL)
        .examples(&["atlx confluence page label create runbook ops --page 12345"]),
    destructive("confluence page label delete")
        .examples(&["atlx confluence page label delete draft --page 12345 --yes"]),
    read("confluence page version list")
        .record(&records::VERSION)
        .examples(&["atlx confluence page version list --page 12345 --limit 5"]),
    read("confluence page version get")
        .record(&records::VERSION)
        .examples(&["atlx confluence page version get 3 --page 12345"]),
    read("confluence page version diff").examples(&[
        "atlx confluence page version diff --page 12345 --from 3 --to 5",
    ]),
    destructive("confluence page version restore").examples(&[
        "atlx confluence page version restore 3 --page 12345 --message 'Undo bad edit' --yes",
    ]),
    read("confluence page task list")
        .record(&records::TASK)
        .examples(&[
            "atlx confluence page task list --page 12345 --status open",
            "atlx confluence page task list --status open --limit 50",
        ]),
    read("confluence page task get")
        .record(&records::TASK)
        .examples(&["atlx confluence page task get 42"]),
    write("confluence page task update")
        .record(&records::TASK)
        .examples(&["atlx confluence page task update 42 --status complete"]),
    read("confluence page state get").examples(&["atlx confluence page state get 12345"]),
    write("confluence page state update").examples(&[
        "atlx confluence page state update 12345 --name 'In review' --color '#ffc400'",
        "atlx confluence page state update 12345 --state-id 3",
    ]),
    read("confluence page state list").examples(&["atlx confluence page state list 12345"]),
    read("confluence space list")
        .record(&records::SPACE)
        .examples(&[
            "atlx confluence space list --limit 10",
            "atlx confluence space list --type personal --all -o ids",
        ]),
    read("confluence space get")
        .record(&records::SPACE)
        .examples(&[
            "atlx confluence space get ENG",
            "atlx confluence space get ENG OPS --labels --full",
            "atlx confluence space get --personal",
        ]),
    write("confluence space create")
        .record(&records::SPACE)
        .examples(&["atlx confluence space create --key NEW --name 'New team' --dry-run"]),
    write("confluence space update")
        .record(&records::SPACE)
        .examples(&["atlx confluence space update ENG --name Engineering"]),
    destructive("confluence space archive")
        .record(&records::SPACE)
        .examples(&["atlx confluence space archive OLD --yes"]),
    write("confluence space restore")
        .record(&records::SPACE)
        .examples(&["atlx confluence space restore OLD"]),
    destructive("confluence space delete")
        .examples(&["atlx confluence space delete OLD --dry-run"]),
    write("confluence space watch").examples(&["atlx confluence space watch ENG"]),
    write("confluence space unwatch").examples(&["atlx confluence space unwatch ENG"]),
    read("confluence space tree").examples(&["atlx confluence space tree ENG --depth 1"]),
    read("confluence space permission list")
        .record(&records::PERMISSION)
        .examples(&["atlx confluence space permission list --space ENG --all"]),
    read("confluence template list")
        .record(&records::TEMPLATE)
        .examples(&[
            "atlx confluence template list",
            "atlx confluence template list --space ENG --scope available -q retro",
        ]),
    read("confluence template get")
        .record(&records::TEMPLATE)
        .examples(&["atlx confluence template get 12345"]),
    write("confluence label watch").examples(&["atlx confluence label watch runbook"]),
    write("confluence label unwatch").examples(&["atlx confluence label unwatch runbook"]),
];

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    fn parses(line: &str) -> bool {
        Cli::try_parse_from(shell_words(line)).is_ok()
    }

    #[test]
    fn the_grammar_parses() {
        for ok in [
            "atlx confluence search deploy",
            "atlx confluence search --cql 'type = page' --cursor t:abc --limit 5",
            "atlx confluence page get 1 2 3 --strict --concurrency 3",
            "atlx confluence page get - --body-format storage",
            "atlx confluence page get 1 --section A --section B --body-offset 20000",
            "atlx confluence page get 1 --body-only --output-file page.md",
            "atlx confluence page update 1 --if-version 3 --body x --allow-lossy --yes",
            "atlx confluence page update 1 --if-version 3 --body - --body-format plain",
            "atlx confluence page edit 1 --if-version 3 --edits-file e.json --title T",
            "atlx confluence page delete 1 --draft --type blogpost --yes",
            "atlx confluence page move 1 --after 2",
            "atlx confluence page comment list --page 1 --inline --resolution dangling --all",
            "atlx confluence page comment create --page 1 --body x --text-selection y --text-selection-match-index 1",
            "atlx confluence page comment update 7 --reopen --inline",
            "atlx confluence page attachment download att1 --out a.png --version 2 --force",
            "atlx confluence page label list --page Runbook --space ENG --type custom-content",
            "atlx confluence page version diff --page 1 --from 1 --to 2 --body-format adf",
            "atlx confluence page task update 4 --status open",
            "atlx confluence space get ENG --labels",
            "atlx confluence space tree ENG --type database",
            "atlx confluence template list --scope global --type blueprint -q x --cursor o:25",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
    }

    #[test]
    fn every_path_follows_the_grammar() {
        use crate::schema::{Effect, registry};
        const VERBS: &[&str] = &[
            "list", "get", "create", "update", "delete", "search", "edit", "restore", "move",
            "copy", "archive", "publish", "watch", "unwatch", "tree", "download", "upload", "diff",
        ];
        const BANNED: &[&str] = &[
            "--id",
            "--max-results",
            "--start-at",
            "--first",
            "--after-cursor",
            "--format",
            "--snapshot-token",
            "--ack-body-formats",
            "--reference-space",
            "--dry-run",
        ];
        let leaves: Vec<_> = crate::cli::describe::leaves()
            .into_iter()
            .filter(|l| l.path.starts_with("confluence "))
            .collect();
        assert!(leaves.len() > 40);
        for l in &leaves {
            let words: Vec<&str> = l.path.split(' ').collect();
            assert!(
                words.len() <= 4,
                "{}: one sub-resource level at most",
                l.path
            );
            let verb = words.last().unwrap();
            assert!(
                l.path == "confluence search" || VERBS.contains(verb),
                "{}: `{verb}` is not a verb",
                l.path
            );
            let meta = registry::lookup(&l.path)
                .unwrap_or_else(|| panic!("{} has no CommandMeta", l.path));
            assert!(!meta.examples.is_empty(), "{} has no example", l.path);
            let flags: Vec<String> = l
                .cmd
                .get_arguments()
                .filter(|a| !a.is_global_set())
                .filter_map(|a| a.get_long().map(|f| format!("--{f}")))
                .collect();
            for f in &flags {
                assert!(!BANNED.contains(&f.as_str()), "{}: {f}", l.path);
            }
            assert_eq!(
                meta.effect == Effect::Destructive,
                flags.iter().any(|f| f == "--yes") && !flags.iter().any(|f| f == "--allow-lossy"),
                "{}: --yes exactly on destructive commands (and with --allow-lossy)",
                l.path
            );
        }
    }

    #[test]
    fn removed_and_invalid_forms_do_not_parse() {
        for bad in [
            // Old paths and flags are gone, with no aliases.
            "atlx confluence content get 1",
            "atlx confluence search query --cql x",
            "atlx confluence page update 1 --snapshot-token 3 --title T",
            "atlx confluence page update 1 --if-version 3 --body x --format html",
            "atlx confluence page create --space S --title T --body x --ack-body-formats",
            // HTML is not a body format.
            "atlx confluence page create --space S --title T --body x --body-format html",
            "atlx confluence page comment create --page 1 --body x --body-format html",
            // --if-version is required.
            "atlx confluence page update 1 --title T",
            "atlx confluence page edit 1 --edits []",
            // One way to say each thing.
            "atlx confluence page edit 1 --if-version 1 --edits [] --edits-file e.json",
            "atlx confluence page move 1 --parent 2 --before 3",
            "atlx confluence page move 1",
            "atlx confluence page delete 1 --purge --draft",
            "atlx confluence page comment create --page 1 --comment 2 --body x",
            "atlx confluence page comment update 7 --resolve --reopen",
            "atlx confluence page get 1 --outline --section A",
            "atlx confluence space get ENG --personal",
            "atlx confluence page list --space S --parent 1",
            "atlx confluence page list --limit 5 --all --space S",
        ] {
            assert!(!parses(bad), "expected a usage error for: {bad}");
        }
    }
}
