//! `atlx jsm`: clap definitions and dispatch.

use super::util::{require_yes, text_arg};
use crate::client::{ApiError, Client};
use crate::output::envelope::envelope;
use crate::{jira, jsm};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) const META: &[crate::schema::CommandMeta] = &[];

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum JsmCmd {
    /// Customer requests
    Request {
        #[command(subcommand)]
        cmd: RequestCmd,
    },
    /// Request types and their fields
    RequestType {
        #[command(subcommand)]
        cmd: TypeCmd,
    },
    /// Approvals on requests
    Approval {
        #[command(subcommand)]
        cmd: ApprovalCmd,
    },
    /// Customer portals
    Portal {
        #[command(subcommand)]
        cmd: DeskListCmd,
    },
    /// Service desks (atlx extra)
    Servicedesk {
        #[command(subcommand)]
        cmd: DeskListCmd,
    },
    /// Agent queues (atlx extra)
    Queue {
        #[command(subcommand)]
        cmd: QueueCmd,
    },
    /// Service level agreements
    Sla {
        #[command(subcommand)]
        cmd: SlaCmd,
    },
    /// Knowledge base articles
    KnowledgeArticle {
        #[command(subcommand)]
        cmd: ArticleCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum DeskListCmd {
    /// List them
    Query {
        /// Project key or name contains
        #[arg(long)]
        query_term: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Zero-based offset (atlx extra)
        #[arg(long, default_value_t = 0)]
        start: usize,
    },
}

#[derive(Args)]
pub(crate) struct DeskArg {
    /// Service desk ID, project key, exact name or portal URL
    #[arg(long, aliases = ["service-desk-id", "servicedesk"])]
    service_desk: String,
}

#[derive(Args)]
pub(crate) struct TypeIds {
    /// Service desk ID, project key or exact name
    #[arg(long, alias = "service-desk")]
    service_desk_id: String,
    /// Request type ID or exact name
    #[arg(long, alias = "request-type")]
    request_type_id: String,
}

#[derive(Subcommand)]
pub(crate) enum TypeCmd {
    /// List request types, of one service desk or all
    Query {
        /// Portal (service desk) ID or URL
        #[arg(long, aliases = ["service-desk", "servicedesk"])]
        portal_id: Option<String>,
        /// Name or description contains
        #[arg(long)]
        query_term: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// A request type's fields
    Field {
        #[command(subcommand)]
        cmd: FieldsCmd,
    },
    /// Same as `field`
    Fields {
        #[command(subcommand)]
        cmd: FieldsCmd,
    },
    /// The form behind a request type
    Form {
        #[command(subcommand)]
        cmd: FormCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum FieldsCmd {
    /// Get them, with allowed values
    Get(TypeIds),
}

#[derive(Subcommand)]
pub(crate) enum FormCmd {
    /// Its fields, and an example answers document
    Get {
        #[command(flatten)]
        desk: DeskArg,
        /// Request type ID or exact name
        #[arg(long)]
        request_type: Option<String>,
        /// Only the example answers document
        #[arg(long)]
        example_only: bool,
    },
}

#[derive(Args)]
pub(crate) struct NewRequestArgs {
    #[command(flatten)]
    desk: DeskArg,
    /// Request type ID or exact name; without it the error lists the types
    #[arg(long, aliases = ["request-type-id"])]
    request_type: Option<String>,
    /// Summary
    #[arg(long)]
    summary: Option<String>,
    /// Description; `-` reads stdin
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
    /// JSON object of answers
    #[arg(long, conflicts_with = "answers_file")]
    answers_json: Option<String>,
    /// File holding a JSON object of answers
    #[arg(long)]
    answers_file: Option<PathBuf>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum RequestCmd {
    /// Create a request
    Create {
        #[command(flatten)]
        new: NewRequestArgs,
        /// Raise it for this customer (atlx extra)
        #[arg(long)]
        on_behalf_of: Option<String>,
    },
    /// Check a request's values without creating it
    Validate {
        #[command(flatten)]
        new: NewRequestArgs,
    },
    /// One request (atlx extra)
    Get {
        /// Request key, ID or URL
        request: String,
    },
    /// Your requests (atlx extra)
    Query {
        /// open, closed or all
        #[arg(long, value_parser = ["open", "closed", "all"])]
        status: Option<String>,
        /// Service desk ID, project key or exact name
        #[arg(long, alias = "servicedesk")]
        service_desk: Option<String>,
        /// Request type ID or exact name (needs --service-desk)
        #[arg(long, requires = "service_desk")]
        request_type: Option<String>,
        /// Summary contains
        #[arg(long)]
        query: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Zero-based offset
        #[arg(long, default_value_t = 0)]
        start: usize,
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
    /// Customer transitions (atlx extra)
    Transition {
        #[command(subcommand)]
        cmd: TransitionCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum CommentCmd {
    /// List comments, public and internal
    List {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
        /// Zero-based offset
        #[arg(long, default_value_t = 0)]
        start: usize,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Add a comment: customers see it unless --internal
    Create {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
        /// Plain text or wiki markup; `-` reads stdin
        #[arg(long)]
        body: String,
        /// An internal note, hidden from customers
        #[arg(long)]
        internal: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ParticipantCmd {
    /// List participants
    List {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
    },
    /// Add participants
    Add {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
        /// me, an email or an account ID; repeatable
        #[arg(long, required = true)]
        participant: Vec<String>,
    },
    /// Remove participants
    Remove {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
        /// me, an email or an account ID; repeatable
        #[arg(long, required = true)]
        participant: Vec<String>,
        /// Confirm the removal (required)
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum TransitionCmd {
    /// The transitions you can make
    Query {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
    },
    /// Make one
    Create {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
        /// Transition ID or exact name
        #[arg(long)]
        transition: String,
        /// Comment, seen by customers
        #[arg(long)]
        comment: Option<String>,
    },
}

#[derive(Args)]
pub(crate) struct DecideArgs {
    /// Request key, ID or URL
    #[arg(long)]
    request: String,
    /// Approval ID [default: the only pending one you can answer]
    #[arg(long)]
    approval_id: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum ApprovalCmd {
    /// List a request's approvals
    List {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
    },
    /// One approval
    Get {
        /// Request key, ID or URL
        #[arg(long)]
        request: String,
        /// Approval ID
        #[arg(long)]
        approval_id: String,
    },
    /// Approve, as yourself
    Approve(DecideArgs),
    /// Decline, as yourself
    Reject(DecideArgs),
}

#[derive(Subcommand)]
pub(crate) enum QueueCmd {
    /// A service desk's queues, with counts
    Query {
        #[command(flatten)]
        desk: DeskArg,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Zero-based offset
        #[arg(long, default_value_t = 0)]
        start: usize,
    },
    /// What is in a queue
    Workitems {
        #[command(subcommand)]
        cmd: QueueItemsCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum QueueItemsCmd {
    /// List the work items
    Query {
        #[command(flatten)]
        desk: DeskArg,
        /// Queue ID
        #[arg(long)]
        queue: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Zero-based offset
        #[arg(long, default_value_t = 0)]
        start: usize,
    },
}

#[derive(Subcommand)]
pub(crate) enum SlaCmd {
    /// A work item's SLAs
    Get {
        /// Work item key
        #[arg(long)]
        workitem: String,
        /// Only this SLA: name or ID
        #[arg(long)]
        sla: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ArticleCmd {
    /// Search a project's knowledge base
    Query {
        /// Project ID, key, ARI or URL
        #[arg(long)]
        project_id: String,
        /// Text to search for; REST needs one (atlx extra)
        #[arg(long)]
        query: Option<String>,
        /// Most articles to return (atlx extra)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn out(c: &Client, command: &str, kind: &str, request: Value, data: Value) -> Result<Value> {
    let mut request = request;
    request["site"] = jira::site(c).into();
    Ok(envelope(&format!("jsm.{command}"), &format!("jsm.{kind}"))
        .request(request)
        .data(data))
}

fn answers(a: &NewRequestArgs) -> Result<Vec<(String, Value)>> {
    let mut out = Vec::new();
    let document = match (&a.answers_json, &a.answers_file) {
        (Some(text), _) => Some(text.clone()),
        (None, Some(path)) => Some(
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
        ),
        (None, None) => None,
    };
    if let Some(text) = document {
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| usage(format!("the answers are not valid JSON: {e}")))?;
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

/// `request create`, or `validate` when there is no `create`.
fn new_request(c: &Client, a: NewRequestArgs, create: Option<Option<String>>) -> Result<Value> {
    let answers = answers(&a)?;
    let fields = jira::extra_fields(a.fields_json.as_deref(), &a.field)?;
    let description = a.description.map(text_arg).transpose()?;
    let desk = jsm::resolve_desk(c, &a.desk.service_desk)?;
    let request_type = jsm::resolve_type(c, &desk, a.request_type.as_deref())?;
    let values = jsm::Values {
        summary: a.summary.as_deref(),
        description: description.as_deref(),
        fields,
        answers,
    };
    let (verb, data) = match create {
        Some(who) => (
            "create",
            jsm::create(c, &desk, &request_type, &values, who.as_deref())?,
        ),
        None => ("validate", jsm::validate(c, &desk, &request_type, &values)?),
    };
    let echo = json!({"serviceDeskId": desk, "requestTypeId": request_type});
    out(c, &format!("request.{verb}"), "request", echo, data)
}

fn request(c: &Client, cmd: RequestCmd) -> Result<Value> {
    match cmd {
        RequestCmd::Create { new, on_behalf_of } => new_request(c, new, Some(on_behalf_of)),
        RequestCmd::Validate { new } => new_request(c, new, None),
        RequestCmd::Get { request } => {
            let data = jsm::get(c, &request)?;
            out(
                c,
                "request.get",
                "request",
                json!({"request": request}),
                data,
            )
        }
        RequestCmd::Query {
            status,
            service_desk,
            request_type,
            query,
            limit,
            start,
        } => {
            let desk = service_desk
                .as_deref()
                .map(|d| jsm::resolve_desk(c, d))
                .transpose()?;
            let kind = match (&desk, &request_type) {
                (Some(d), Some(t)) => Some(jsm::resolve_type(c, d, Some(t))?),
                _ => None,
            };
            let data = jsm::query(
                c,
                &jsm::RequestQuery {
                    status: status.as_deref(),
                    desk: desk.as_deref(),
                    request_type: kind.as_deref(),
                    search: query.as_deref(),
                    start,
                    limit,
                },
            )?;
            out(
                c,
                "request.query",
                "request",
                json!({"status": status, "serviceDeskId": desk}),
                data,
            )
        }
        RequestCmd::Comment { cmd } => match cmd {
            CommentCmd::List {
                request,
                start,
                limit,
            } => {
                let data = jsm::comments(c, &request, start, limit)?;
                let echo = json!({"request": request, "start": start, "limit": limit});
                out(c, "request.comment.list", "comment", echo, data)
            }
            CommentCmd::Create {
                request,
                body,
                internal,
            } => {
                let data = jsm::comment_create(c, &request, &text_arg(body)?, internal)?;
                let echo = json!({"request": request, "internal": internal});
                out(c, "request.comment.create", "comment", echo, data)
            }
        },
        RequestCmd::Participant { cmd } => {
            let (verb, request, data) = match cmd {
                ParticipantCmd::List { request } => {
                    let data = jsm::participants(c, &request)?;
                    ("list", request, data)
                }
                ParticipantCmd::Add {
                    request,
                    participant,
                } => {
                    let data = jsm::participants_change(c, &request, &participant, true)?;
                    ("add", request, data)
                }
                ParticipantCmd::Remove {
                    request,
                    participant,
                    yes,
                } => {
                    require_yes(yes, "removing the participants")?;
                    let data = jsm::participants_change(c, &request, &participant, false)?;
                    ("remove", request, data)
                }
            };
            let command = format!("request.participant.{verb}");
            out(
                c,
                &command,
                "participant",
                json!({"request": request}),
                data,
            )
        }
        RequestCmd::Transition { cmd } => match cmd {
            TransitionCmd::Query { request } => {
                let data = jsm::transitions(c, &request)?;
                out(
                    c,
                    "request.transition.query",
                    "transition",
                    json!({"request": request}),
                    data,
                )
            }
            TransitionCmd::Create {
                request,
                transition,
                comment,
            } => {
                let data = jsm::transition(c, &request, &transition, comment.as_deref())?;
                let echo = json!({"request": request, "transition": transition});
                out(c, "request.transition.create", "transition", echo, data)
            }
        },
    }
}

fn request_type(c: &Client, cmd: TypeCmd) -> Result<Value> {
    let fields = |ids: TypeIds, group: &str| {
        let desk = jsm::resolve_desk(c, &ids.service_desk_id)?;
        let kind = jsm::resolve_type(c, &desk, Some(&ids.request_type_id))?;
        let data = jsm::fields(c, &desk, &kind)?;
        let echo = json!({"serviceDeskId": desk, "requestTypeId": kind});
        out(
            c,
            &format!("request-type.{group}.get"),
            "request-type-field",
            echo,
            data,
        )
    };
    match cmd {
        TypeCmd::Query {
            portal_id,
            query_term,
            limit,
        } => {
            let desk = portal_id
                .as_deref()
                .map(|p| jsm::resolve_desk(c, p))
                .transpose()?;
            let data = jsm::request_types(c, desk.as_deref(), query_term.as_deref(), limit)?;
            out(
                c,
                "request-type.query",
                "request-type",
                json!({"portalId": desk}),
                data,
            )
        }
        TypeCmd::Field {
            cmd: FieldsCmd::Get(ids),
        } => fields(ids, "field"),
        TypeCmd::Fields {
            cmd: FieldsCmd::Get(ids),
        } => fields(ids, "fields"),
        TypeCmd::Form {
            cmd:
                FormCmd::Get {
                    desk,
                    request_type,
                    example_only,
                },
        } => {
            let desk = jsm::resolve_desk(c, &desk.service_desk)?;
            let kind = jsm::resolve_type(c, &desk, request_type.as_deref())?;
            let mut data = jsm::fields(c, &desk, &kind)?;
            let example = jsm::example_answers(&data)?;
            if example_only {
                data = example;
            } else {
                data["exampleAnswers"] = example;
            }
            let echo = json!({"serviceDeskId": desk, "requestTypeId": kind});
            out(c, "request-type.form.get", "request-type-form", echo, data)
        }
    }
}

fn approval(c: &Client, cmd: ApprovalCmd) -> Result<Value> {
    let (verb, request, data) = match cmd {
        ApprovalCmd::List { request } => {
            let data = jsm::approvals(c, &request)?;
            ("list", request, data)
        }
        ApprovalCmd::Get {
            request,
            approval_id,
        } => {
            let data = jsm::approval_get(c, &request, &approval_id)?;
            ("get", request, data)
        }
        ApprovalCmd::Approve(a) => {
            let data = jsm::approval_decide(c, &a.request, a.approval_id.as_deref(), true)?;
            ("approve", a.request, data)
        }
        ApprovalCmd::Reject(a) => {
            let data = jsm::approval_decide(c, &a.request, a.approval_id.as_deref(), false)?;
            ("reject", a.request, data)
        }
    };
    out(
        c,
        &format!("approval.{verb}"),
        "approval",
        json!({"request": request}),
        data,
    )
}

fn desks(c: &Client, cmd: DeskListCmd, group: &str) -> Result<Value> {
    let DeskListCmd::Query {
        query_term,
        limit,
        start,
    } = cmd;
    let data = jsm::desks(c, query_term.as_deref(), start, limit)?;
    let echo = json!({"queryTerm": query_term, "limit": limit});
    out(c, &format!("{group}.query"), group, echo, data)
}

pub(crate) fn jsm_cmd(c: &Client, cmd: JsmCmd) -> Result<Value> {
    match cmd {
        JsmCmd::Request { cmd } => request(c, cmd),
        JsmCmd::RequestType { cmd } => request_type(c, cmd),
        JsmCmd::Approval { cmd } => approval(c, cmd),
        JsmCmd::Portal { cmd } => desks(c, cmd, "portal"),
        JsmCmd::Servicedesk { cmd } => desks(c, cmd, "servicedesk"),
        JsmCmd::Queue {
            cmd: QueueCmd::Query { desk, limit, start },
        } => {
            let desk = jsm::resolve_desk(c, &desk.service_desk)?;
            let data = jsm::queues(c, &desk, start, limit)?;
            out(
                c,
                "queue.query",
                "queue",
                json!({"serviceDeskId": desk}),
                data,
            )
        }
        JsmCmd::Queue {
            cmd:
                QueueCmd::Workitems {
                    cmd:
                        QueueItemsCmd::Query {
                            desk,
                            queue,
                            limit,
                            start,
                        },
                },
        } => {
            let desk = jsm::resolve_desk(c, &desk.service_desk)?;
            let data = jsm::queue_issues(c, &desk, &queue, start, limit)?;
            let echo = json!({"serviceDeskId": desk, "queueId": queue});
            out(c, "queue.workitems.query", "workitem", echo, data)
        }
        JsmCmd::Sla {
            cmd: SlaCmd::Get { workitem, sla },
        } => {
            let data = jsm::sla(c, &workitem, sla.as_deref())?;
            out(
                c,
                "sla.get",
                "sla",
                json!({"workitem": workitem, "sla": sla}),
                data,
            )
        }
        JsmCmd::KnowledgeArticle {
            cmd:
                ArticleCmd::Query {
                    project_id,
                    query,
                    limit,
                },
        } => {
            let query = query.ok_or_else(|| {
                usage("pass --query: the REST API can only search articles, not list them")
            })?;
            let desk = jsm::desk_of_project(c, &project_id)?;
            let data = jsm::articles(c, &desk, &query, limit)?;
            let echo = json!({"projectId": project_id, "query": query});
            out(
                c,
                "knowledge-article.query",
                "knowledge-article",
                echo,
                data,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use crate::cli::tests::shell_words;
    use clap::Parser;

    #[test]
    fn parses_jsm() {
        for ok in [
            "atlx jsm servicedesk query",
            "atlx jsm portal query --query-term IT --limit 5",
            "atlx jsm request-type query --portal-id 1",
            "atlx jsm request-type field get --service-desk-id 1 --request-type-id 10",
            "atlx jsm request-type fields get --service-desk-id 1 --request-type-id 10",
            "atlx jsm request-type form get --service-desk IT --request-type Laptop --example-only",
            "atlx jsm request create --service-desk-id 1 --request-type-id 10 --summary Laptop --answer 'Model=Pro' --on-behalf-of ann",
            "atlx jsm request validate --service-desk IT --request-type Laptop --answers-json {}",
            "atlx jsm request get SD-1",
            "atlx jsm request query --status open --service-desk 1 --request-type 10",
            "atlx jsm request comment list --request SD-1",
            "atlx jsm request comment create --request SD-1 --body hi --internal",
            "atlx jsm request participant list --request SD-1",
            "atlx jsm request participant add --request SD-1 --participant me",
            "atlx jsm request participant remove --request SD-1 --participant me --yes",
            "atlx jsm request transition query --request SD-1",
            "atlx jsm request transition create --request SD-1 --transition Resolve",
            "atlx jsm approval list --request SD-1",
            "atlx jsm approval get --request SD-1 --approval-id 3",
            "atlx jsm approval approve --request SD-1",
            "atlx jsm approval reject --request SD-1 --approval-id 3",
            "atlx jsm queue query --service-desk 1",
            "atlx jsm queue workitems query --service-desk 1 --queue 4",
            "atlx jsm sla get --workitem SD-1 --sla 'Time to resolution'",
            "atlx jsm knowledge-article query --project-id SD --query vpn",
        ] {
            if let Err(e) = Cli::try_parse_from(shell_words(ok)) {
                panic!("{ok}: {e}");
            }
        }
        for bad in [
            "atlx jsm request create --summary x",
            "atlx jsm request participant add --request SD-1",
            "atlx jsm request query --request-type 10",
            "atlx jsm approval get --request SD-1",
        ] {
            assert!(
                Cli::try_parse_from(shell_words(bad)).is_err(),
                "expected error for: {bad}"
            );
        }
    }
}
