//! `atlx jsm`: Jira Service Management. Same grammar as `atlx jira`.

use super::jira::{Confirm, Many, Paging, listed, text_input};
use crate::client::{ApiError, Client};
use crate::jira::records::ISSUE_ROW;
use crate::jsm::records::*;
use crate::schema::{CommandMeta, Effect, Record};
use crate::{jira, jsm};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum JsmCmd {
    /// Customer requests and their comments, participants, approvals and
    /// SLAs
    Request {
        #[command(subcommand)]
        cmd: RequestCmd,
    },
    /// The transitions a customer can make on a request
    Transition {
        #[command(subcommand)]
        cmd: TransitionCmd,
    },
    /// Service desks
    Desk {
        #[command(subcommand)]
        cmd: DeskCmd,
    },
    /// Request types and their fields
    RequestType {
        #[command(subcommand)]
        cmd: TypeCmd,
    },
    /// Agent queues and the issues in them
    Queue {
        #[command(subcommand)]
        cmd: QueueCmd,
    },
    /// Knowledge base articles
    Article {
        #[command(subcommand)]
        cmd: ArticleCmd,
    },
}

/// The request a sub-resource belongs to.
#[derive(Args)]
pub(crate) struct RequestArg {
    /// Request key, ID or URL
    #[arg(long)]
    request: String,
}

/// A service desk.
#[derive(Args)]
pub(crate) struct DeskArg {
    /// Service desk ID, project key, exact name or portal URL
    #[arg(long)]
    desk: String,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum RequestCmd {
    /// Requests you can see, by status, desk, type or text
    List {
        /// open, closed or all
        #[arg(long, value_parser = ["open", "closed", "all"])]
        status: Option<String>,
        /// Service desk ID, project key or exact name
        #[arg(long)]
        desk: Option<String>,
        /// Request type ID or exact name (needs --desk)
        #[arg(long = "type", requires = "desk")]
        request_type: Option<String>,
        /// Summary contains
        #[arg(short, long)]
        query: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Requests by key, ID or URL
    Get {
        /// Request keys (`-` reads them from stdin)
        #[arg(required = true)]
        keys: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
    /// Raise a request; the values are checked against the type's fields
    /// first (preview with --dry-run)
    Create(CreateArgs),
    /// Make a customer transition (customers see it)
    Transition {
        /// Request key, ID or URL
        key: String,
        /// Transition ID or exact name
        #[arg(long)]
        to: String,
        /// Comment, seen by customers
        #[arg(long)]
        comment: Option<String>,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Comments on a request
    Comment {
        #[command(subcommand)]
        cmd: CommentCmd,
    },
    /// People sharing a request
    Participant {
        #[command(subcommand)]
        cmd: ParticipantCmd,
    },
    /// Approvals on a request
    Approval {
        #[command(subcommand)]
        cmd: ApprovalCmd,
    },
    /// Service level agreements on a request
    Sla {
        #[command(subcommand)]
        cmd: SlaCmd,
    },
}

#[derive(Args)]
pub(crate) struct CreateArgs {
    #[command(flatten)]
    desk: DeskArg,
    /// Request type ID or exact name; without it the error lists the types
    #[arg(long = "type")]
    request_type: Option<String>,
    /// Summary
    #[arg(long)]
    summary: Option<String>,
    /// Description (plain text; `-` reads stdin)
    #[arg(long)]
    description: Option<String>,
    /// FIELD_ID=VALUE, sent as given (parsed as JSON when valid); repeatable
    #[arg(long)]
    field: Vec<String>,
    /// JSON object of field IDs to REST values
    #[arg(long)]
    fields_json: Option<String>,
    /// Question=value, by label or field ID; options by their label
    #[arg(long, value_name = "QUESTION=VALUE")]
    answer: Vec<String>,
    /// JSON object of answers (see `request-type get`'s `example`)
    #[arg(long, conflicts_with = "answers_file")]
    answers_json: Option<String>,
    /// File holding a JSON object of answers
    #[arg(long)]
    answers_file: Option<PathBuf>,
    /// Raise it for this customer
    #[arg(long)]
    on_behalf_of: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum CommentCmd {
    /// A request's comments, public and internal
    List {
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Add a comment; customers see it unless --internal
    Create {
        #[command(flatten)]
        request: RequestArg,
        /// Plain text (`-` reads stdin)
        #[arg(
            long,
            required_unless_present = "body_file",
            conflicts_with = "body_file"
        )]
        body: Option<String>,
        /// Read the body from this file
        #[arg(long, value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// An internal note, hidden from customers
        #[arg(long)]
        internal: bool,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum ParticipantCmd {
    /// A request's participants
    List {
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Add participants
    Create {
        /// me, an email or an account ID
        #[arg(required = true)]
        users: Vec<String>,
        #[command(flatten)]
        request: RequestArg,
    },
    /// Remove participants
    Delete {
        /// me, an email or an account ID
        #[arg(required = true)]
        users: Vec<String>,
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum ApprovalCmd {
    /// A request's approvals
    List {
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// Approvals by ID
    Get {
        /// Approval IDs (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        many: Many,
    },
    /// Approve, as yourself
    Approve {
        /// Approval ID [default: the only pending one you can answer]
        id: Option<String>,
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        confirm: Confirm,
    },
    /// Decline, as yourself
    Decline {
        /// Approval ID [default: the only pending one you can answer]
        id: Option<String>,
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        confirm: Confirm,
    },
}

#[derive(Subcommand)]
pub(crate) enum SlaCmd {
    /// A request's SLAs
    List {
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// SLAs by ID or name
    Get {
        /// SLA IDs or names (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        many: Many,
    },
}

#[derive(Subcommand)]
pub(crate) enum TransitionCmd {
    /// The transitions you can make on a request
    List {
        #[command(flatten)]
        request: RequestArg,
        #[command(flatten)]
        paging: Paging,
    },
}

#[derive(Subcommand)]
pub(crate) enum DeskCmd {
    /// Service desks, by project key or name
    List {
        /// Project key or name contains
        #[arg(short, long)]
        query: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
}

#[derive(Subcommand)]
pub(crate) enum TypeCmd {
    /// Request types, of one service desk or all
    List {
        /// Service desk ID, project key or exact name
        #[arg(long)]
        desk: Option<String>,
        /// Name or description contains
        #[arg(short, long)]
        query: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Request types by ID or exact name, with their fields and an example
    /// answers document (use --full to see them)
    Get {
        /// Request type IDs or names (`-` reads them from stdin)
        #[arg(required = true)]
        ids: Vec<String>,
        #[command(flatten)]
        desk: DeskArg,
        #[command(flatten)]
        many: Many,
    },
}

#[derive(Subcommand)]
pub(crate) enum QueueCmd {
    /// A service desk's queues, with counts
    List {
        #[command(flatten)]
        desk: DeskArg,
        #[command(flatten)]
        paging: Paging,
    },
    /// The issues in a queue
    Issue {
        #[command(subcommand)]
        cmd: QueueIssueCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum QueueIssueCmd {
    /// The issues in a queue
    List {
        /// Queue ID
        #[arg(long)]
        queue: String,
        #[command(flatten)]
        desk: DeskArg,
        #[command(flatten)]
        paging: Paging,
    },
}

#[derive(Subcommand)]
pub(crate) enum ArticleCmd {
    /// Knowledge base articles matching text, in one desk or all
    Search {
        /// Words to look for
        #[arg(required = true)]
        text: Vec<String>,
        /// Service desk ID, project key or exact name
        #[arg(long)]
        desk: Option<String>,
        #[command(flatten)]
        paging: Paging,
    },
}

const R: Effect = Effect::Read;
const W: Effect = Effect::Write;
const D: Effect = Effect::Destructive;

const fn m(
    path: &'static str,
    effect: Effect,
    record: &'static Record,
    examples: &'static [&'static str],
) -> CommandMeta {
    CommandMeta::new(path, effect)
        .record(record)
        .examples(examples)
}

/// Anything customers see (public comments, transitions, approvals) is
/// destructive: it needs `--yes`.
pub(crate) const META: &[CommandMeta] = &[
    m(
        "jsm request list",
        R,
        &REQUEST,
        &[
            "atlx jsm request list --status open --desk HELP",
            "atlx jsm request list -q vpn --limit 10",
        ],
    ),
    m(
        "jsm request get",
        R,
        &REQUEST,
        &["atlx jsm request get HELP-12"],
    ),
    m(
        "jsm request create",
        W,
        &REQUEST,
        &[
            "atlx jsm request create --desk HELP --type 'Get IT help' --summary 'VPN down' --description 'Since 9am'",
            "atlx jsm request create --desk HELP --type 'Get IT help' --answers-file answers.json --dry-run",
        ],
    ),
    m(
        "jsm request transition",
        D,
        &TRANSITIONED,
        &["atlx jsm request transition HELP-12 --to Resolve --comment 'Fixed' --yes"],
    ),
    m(
        "jsm request comment list",
        R,
        &COMMENT,
        &["atlx jsm request comment list --request HELP-12"],
    ),
    m(
        "jsm request comment create",
        D,
        &COMMENT,
        &[
            "atlx jsm request comment create --request HELP-12 --body 'Looking into it' --yes",
            "atlx jsm request comment create --request HELP-12 --body 'Agent note' --internal --yes",
        ],
    ),
    m(
        "jsm request participant list",
        R,
        &PARTICIPANT,
        &["atlx jsm request participant list --request HELP-12"],
    ),
    m(
        "jsm request participant create",
        W,
        &PARTICIPANT,
        &["atlx jsm request participant create alice@example.com --request HELP-12"],
    ),
    m(
        "jsm request participant delete",
        D,
        &PARTICIPANT,
        &["atlx jsm request participant delete alice@example.com --request HELP-12 --yes"],
    ),
    m(
        "jsm request approval list",
        R,
        &APPROVAL,
        &["atlx jsm request approval list --request HELP-12"],
    ),
    m(
        "jsm request approval get",
        R,
        &APPROVAL,
        &["atlx jsm request approval get 7 --request HELP-12"],
    ),
    m(
        "jsm request approval approve",
        D,
        &APPROVAL,
        &["atlx jsm request approval approve --request HELP-12 --yes"],
    ),
    m(
        "jsm request approval decline",
        D,
        &APPROVAL,
        &["atlx jsm request approval decline 7 --request HELP-12 --yes"],
    ),
    m(
        "jsm request sla list",
        R,
        &SLA,
        &["atlx jsm request sla list --request HELP-12"],
    ),
    m(
        "jsm request sla get",
        R,
        &SLA,
        &["atlx jsm request sla get 'Time to resolution' --request HELP-12"],
    ),
    m(
        "jsm transition list",
        R,
        &TRANSITION,
        &["atlx jsm transition list --request HELP-12"],
    ),
    m("jsm desk list", R, &DESK, &["atlx jsm desk list -q help"]),
    m(
        "jsm request-type list",
        R,
        &REQUEST_TYPE,
        &["atlx jsm request-type list --desk HELP"],
    ),
    m(
        "jsm request-type get",
        R,
        &REQUEST_TYPE,
        &["atlx jsm request-type get 'Get IT help' --desk HELP --full"],
    ),
    m(
        "jsm queue list",
        R,
        &QUEUE,
        &["atlx jsm queue list --desk HELP"],
    ),
    m(
        "jsm queue issue list",
        R,
        &ISSUE_ROW,
        &["atlx jsm queue issue list --queue 3 --desk HELP"],
    ),
    m(
        "jsm article search",
        R,
        &ARTICLE,
        &["atlx jsm article search vpn --desk HELP"],
    ),
];

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn answers(a: &CreateArgs) -> Result<Vec<(String, Value)>> {
    let mut out = Vec::new();
    let document = match (&a.answers_json, &a.answers_file) {
        (Some(text), _) => Some(text.clone()),
        (None, Some(path)) => Some(
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
        ),
        (None, None) => None,
    };
    if let Some(text) = document {
        let v = jira::json_value("the answers", &text)?;
        let map = v
            .as_object()
            .ok_or_else(|| usage("the answers must be a JSON object"))?;
        out.extend(map.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    for pair in &a.answer {
        let (q, v) = pair
            .split_once('=')
            .ok_or_else(|| usage(format!("--answer '{pair}' must be Question=value")))?;
        out.push((q.trim().to_string(), v.into()));
    }
    Ok(out)
}

fn create(c: &Client, a: CreateArgs) -> Result<Value> {
    let answers = answers(&a)?;
    let fields = jira::extra_fields(a.fields_json.as_deref(), &a.field)?;
    let description = text_input(a.description.clone(), None)?;
    let desk = jsm::resolve_desk(c, &a.desk.desk)?;
    let request_type = jsm::resolve_type(c, &desk, a.request_type.as_deref())?;
    let values = jsm::Values {
        summary: a.summary.as_deref(),
        description: description.as_deref(),
        fields,
        answers,
    };
    jsm::create(c, &desk, &request_type, &values, a.on_behalf_of.as_deref())
}

fn request(c: &Client, cmd: RequestCmd) -> Result<Value> {
    match cmd {
        RequestCmd::List {
            status,
            desk,
            request_type,
            query,
            paging,
        } => {
            let desk = desk
                .as_deref()
                .map(|d| jsm::resolve_desk(c, d))
                .transpose()?;
            let kind = match (&desk, &request_type) {
                (Some(d), Some(t)) => Some(jsm::resolve_type(c, d, Some(t))?),
                _ => None,
            };
            let q = jsm::RequestQuery {
                status: status.as_deref(),
                desk: desk.as_deref(),
                request_type: kind.as_deref(),
                search: query.as_deref(),
            };
            Ok(listed(jsm::requests(
                c,
                &q,
                paging.limit(),
                paging.cursor(),
            )?))
        }
        RequestCmd::Get { keys, many } => many.run(keys, |k| jsm::get(c, k)),
        RequestCmd::Create(a) => create(c, a),
        RequestCmd::Transition {
            key,
            to,
            comment,
            confirm: _,
        } => jsm::transition(c, &key, &to, comment.as_deref()),
        RequestCmd::Comment { cmd } => match cmd {
            CommentCmd::List { request, paging } => Ok(listed(jsm::comments(
                c,
                &request.request,
                paging.limit(),
                paging.cursor(),
            )?)),
            CommentCmd::Create {
                request,
                body,
                body_file,
                internal,
                confirm: _,
            } => {
                let body = text_input(body, body_file)?.unwrap_or_default();
                jsm::comment_create(c, &request.request, &body, internal)
            }
        },
        RequestCmd::Participant { cmd } => match cmd {
            ParticipantCmd::List { request, paging } => {
                paging.local(jsm::participants(c, &request.request)?)
            }
            ParticipantCmd::Create { users, request } => {
                jsm::participants_change(c, &request.request, &users, true)
            }
            ParticipantCmd::Delete {
                users,
                request,
                confirm: _,
            } => jsm::participants_change(c, &request.request, &users, false),
        },
        RequestCmd::Approval { cmd } => match cmd {
            ApprovalCmd::List { request, paging } => {
                paging.local(jsm::approvals(c, &request.request)?)
            }
            ApprovalCmd::Get { ids, request, many } => {
                many.run(ids, |id| jsm::approval_get(c, &request.request, id))
            }
            ApprovalCmd::Approve {
                id,
                request,
                confirm: _,
            } => jsm::approval_decide(c, &request.request, id.as_deref(), true),
            ApprovalCmd::Decline {
                id,
                request,
                confirm: _,
            } => jsm::approval_decide(c, &request.request, id.as_deref(), false),
        },
        RequestCmd::Sla { cmd } => match cmd {
            SlaCmd::List { request, paging } => paging.local(jsm::slas(c, &request.request)?),
            SlaCmd::Get { ids, request, many } => {
                many.run(ids, |id| jsm::sla(c, &request.request, id))
            }
        },
    }
}

pub(crate) fn jsm_cmd(c: &Client, cmd: JsmCmd) -> Result<Value> {
    match cmd {
        JsmCmd::Request { cmd } => request(c, cmd),
        JsmCmd::Transition {
            cmd: TransitionCmd::List { request, paging },
        } => paging.local(jsm::transitions(c, &request.request)?),
        JsmCmd::Desk {
            cmd: DeskCmd::List { query, paging },
        } => Ok(listed(jsm::desks(
            c,
            query.as_deref(),
            paging.limit(),
            paging.cursor(),
        )?)),
        JsmCmd::RequestType { cmd } => match cmd {
            TypeCmd::List {
                desk,
                query,
                paging,
            } => {
                let desk = desk
                    .as_deref()
                    .map(|d| jsm::resolve_desk(c, d))
                    .transpose()?;
                Ok(listed(jsm::request_types(
                    c,
                    desk.as_deref(),
                    query.as_deref(),
                    paging.limit(),
                    paging.cursor(),
                )?))
            }
            TypeCmd::Get { ids, desk, many } => {
                let desk = jsm::resolve_desk(c, &desk.desk)?;
                many.run(ids, |id| {
                    let kind = jsm::resolve_type(c, &desk, Some(id))?;
                    jsm::request_type(c, &desk, &kind)
                })
            }
        },
        JsmCmd::Queue { cmd } => match cmd {
            QueueCmd::List { desk, paging } => {
                let desk = jsm::resolve_desk(c, &desk.desk)?;
                Ok(listed(jsm::queues(
                    c,
                    &desk,
                    paging.limit(),
                    paging.cursor(),
                )?))
            }
            QueueCmd::Issue {
                cmd:
                    QueueIssueCmd::List {
                        queue,
                        desk,
                        paging,
                    },
            } => {
                let desk = jsm::resolve_desk(c, &desk.desk)?;
                Ok(listed(jsm::queue_issues(
                    c,
                    &desk,
                    &queue,
                    paging.limit(),
                    paging.cursor(),
                )?))
            }
        },
        JsmCmd::Article {
            cmd: ArticleCmd::Search { text, desk, paging },
        } => {
            let desk = desk
                .as_deref()
                .map(|d| jsm::resolve_desk(c, d))
                .transpose()?;
            Ok(listed(jsm::articles(
                c,
                desk.as_deref(),
                &text.join(" "),
                paging.limit(),
                paging.cursor(),
            )?))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    #[test]
    fn parses_the_jsm_tree() {
        for ok in [
            "atlx jsm request list --status open --desk HELP --type 'IT help' -q vpn --limit 5",
            "atlx jsm request get HELP-1 HELP-2",
            "atlx jsm request create --desk HELP --type 1 --summary S --answer 'Urgency=High' --on-behalf-of a@example.com",
            "atlx jsm request transition HELP-1 --to Resolve --yes",
            "atlx jsm request comment list --request HELP-1",
            "atlx jsm request comment create --request HELP-1 --body x --internal --yes",
            "atlx jsm request participant list --request HELP-1",
            "atlx jsm request participant create me a@example.com --request HELP-1",
            "atlx jsm request participant delete me --request HELP-1 --yes",
            "atlx jsm request approval list --request HELP-1",
            "atlx jsm request approval get 7 --request HELP-1",
            "atlx jsm request approval approve --request HELP-1 --yes",
            "atlx jsm request approval decline 7 --request HELP-1 --yes",
            "atlx jsm request sla list --request HELP-1",
            "atlx jsm request sla get 1 --request HELP-1",
            "atlx jsm transition list --request HELP-1",
            "atlx jsm desk list -q help",
            "atlx jsm request-type list --desk HELP",
            "atlx jsm request-type get 1 --desk HELP",
            "atlx jsm queue list --desk HELP",
            "atlx jsm queue issue list --queue 3 --desk HELP",
            "atlx jsm article search vpn reset --desk HELP",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(ok)).is_ok(),
                "{ok}: {:?}",
                Cli::try_parse_from(shell_words(ok)).err()
            );
        }
        for bad in [
            "atlx jsm request query",
            "atlx jsm servicedesk query",
            "atlx jsm request list --type x",
            "atlx jsm request comment create --request HELP-1",
        ] {
            assert!(Cli::try_parse_from(shell_words(bad)).is_err(), "{bad}");
        }
    }
}
