//! `--select` and `--agent-fields`: keep only some paths of a payload.
//!
//! A path is a dot-separated list of object keys, such as `issues.status.name`.
//! When a step meets an array, the rest of the path is applied to every
//! element, so the result keeps the payload's own structure, cut down to the
//! selected leaves. Paths that are missing are skipped; if no path matches at
//! all, the caller keeps the full payload.
//!
//! `--select` paths are relative to the payload data. When the payload is a
//! twg-style envelope (`{"apiVersion", "command", "meta", "data"}`), a leading
//! `data.` is optional, the envelope keys stay in the result, and paths whose
//! first key is an envelope key select from the envelope itself. When the
//! payload is not wrapped, a leading `data.` is dropped so the same paths work
//! on both shapes.
//!
//! `@name` tokens expand to per-command presets (see [`presets`]).

use crate::client::ApiError;
use serde_json::{Map, Value};

/// Top-level keys of a twg-style envelope that are not part of the data.
pub const ENVELOPE_KEYS: [&str; 4] = ["apiVersion", "command", "meta", "resourceType"];

/// Split `a.b, c.d` into `[["a","b"], ["c","d"]]`, trimming each step and
/// dropping empty steps and duplicate paths.
pub fn parse_paths<'a>(specs: impl IntoIterator<Item = &'a str>) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for spec in specs {
        let path: Vec<String> = spec
            .split('.')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if !path.is_empty() && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// A resolved `--select` or `--agent-fields` value.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Fields {
    /// Every path to keep, with presets expanded, in order.
    pub paths: Vec<Vec<String>>,
    /// Extra preset paths for atlx's unwrapped output; see [`Preset`].
    pub atlx: Vec<Vec<String>>,
    /// `@preset` tokens the command has no table for.
    pub unresolved: Vec<String>,
}

impl Fields {
    /// The paths joined back into dotted strings.
    pub fn dotted(&self) -> Vec<String> {
        self.paths.iter().map(|p| p.join(".")).collect()
    }

    /// The paths to apply to `v`: the atlx extras join in when `v` is not
    /// wrapped in a `data` envelope.
    pub fn for_payload(&self, v: &Value) -> Vec<Vec<String>> {
        let mut out = self.paths.clone();
        if !has_key(v, "data") {
            for p in &self.atlx {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
        }
        out
    }
}

/// Presets that every command accepts; a command without its own table
/// warns and ignores them instead of failing.
const UNIVERSAL: [&str; 2] = ["compact", "evidence"];

/// Resolve a comma-separated `--select`/`--agent-fields` value for
/// `command` (a dotted command path such as `jira.workitem.get`). An unknown
/// preset is a usage error.
pub fn resolve(flag: &str, spec: &str, command: &str) -> anyhow::Result<Fields> {
    let table = presets(command);
    let mut raw: Vec<String> = Vec::new();
    let mut extra: Vec<&str> = Vec::new();
    let mut unresolved = Vec::new();
    for token in spec.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let Some(name) = token.strip_prefix('@') else {
            raw.push(token.to_string());
            continue;
        };
        match table {
            Some(t) => match t.iter().find(|p| p.name == name) {
                Some(p) => {
                    raw.extend(p.twg.iter().map(|p| p.to_string()));
                    extra.extend(p.atlx);
                }
                None => {
                    let names: Vec<String> = t.iter().map(|p| format!("@{}", p.name)).collect();
                    return Err(ApiError::usage(format!(
                        "{flag}: no preset {token} for `{}`; this command has {}",
                        command.replace('.', " "),
                        names.join(", ")
                    ))
                    .into());
                }
            },
            None if UNIVERSAL.contains(&name) => unresolved.push(token.to_string()),
            None => {
                return Err(ApiError::usage(format!(
                    "{flag}: no preset {token} for `{}`; this command has no presets",
                    command.replace('.', " ")
                ))
                .into());
            }
        }
    }
    Ok(Fields {
        paths: parse_paths(raw.iter().map(String::as_str)),
        atlx: parse_paths(extra),
        unresolved,
    })
}

pub(crate) fn is_error_envelope(v: &Value) -> bool {
    v.get("ok") == Some(&Value::Bool(false))
}

/// Project `v` down to one path. `None` means the path did not match.
/// A `null` met along the way is kept, as is an empty array.
fn pick(v: &Value, path: &[String]) -> Option<Value> {
    if path.is_empty() || v.is_null() {
        return Some(v.clone());
    }
    match v {
        Value::Array(items) => {
            if items.is_empty() {
                return Some(Value::Array(Vec::new()));
            }
            let picked: Vec<Option<Value>> = items.iter().map(|i| pick(i, path)).collect();
            if picked.iter().all(Option::is_none) {
                return None;
            }
            Some(Value::Array(
                picked
                    .into_iter()
                    .map(|p| p.unwrap_or(Value::Null))
                    .collect(),
            ))
        }
        Value::Object(m) => {
            let inner = pick(m.get(&path[0])?, &path[1..])?;
            let mut out = Map::new();
            out.insert(path[0].clone(), inner);
            Some(Value::Object(out))
        }
        _ => None,
    }
}

/// Deep-merge two projections of the same value: objects key by key (first
/// seen order), arrays element by element.
fn merge(a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            let n = x.len().max(y.len());
            let mut x = x.into_iter();
            let mut y = y.into_iter();
            Value::Array(
                (0..n)
                    .map(|_| match (x.next(), y.next()) {
                        (Some(Value::Null), Some(q)) => q,
                        (Some(p), Some(Value::Null)) => p,
                        (Some(p), Some(q)) => merge(p, q),
                        (Some(p), None) => p,
                        (None, Some(q)) => q,
                        (None, None) => Value::Null,
                    })
                    .collect(),
            )
        }
        (Value::Object(mut x), Value::Object(y)) => {
            for (k, v) in y {
                match x.get_mut(&k) {
                    Some(old) => *old = merge(old.take(), v),
                    None => {
                        x.insert(k, v);
                    }
                }
            }
            Value::Object(x)
        }
        (_, b) => b,
    }
}

/// Apply every path to `v` and merge the results. `None` when nothing matched.
pub fn project(v: &Value, paths: &[Vec<String>]) -> Option<Value> {
    if v.as_array().is_some_and(Vec::is_empty) {
        return Some(Value::Array(Vec::new()));
    }
    paths.iter().filter_map(|p| pick(v, p)).reduce(merge)
}

/// The data part of `v`: `v.data` for an envelope, else `v` without any
/// envelope keys.
fn data_part(v: &Value) -> Option<Value> {
    match v {
        Value::Object(m) if m.contains_key("data") => Some(m["data"].clone()),
        Value::Object(m) => {
            let rest: Map<String, Value> = m
                .iter()
                .filter(|(k, _)| !ENVELOPE_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            (!rest.is_empty()).then_some(Value::Object(rest))
        }
        other => Some(other.clone()),
    }
}

/// The outcome of [`select`].
#[derive(Debug, PartialEq)]
pub enum Selected {
    /// Nothing to do (no paths, or an error envelope).
    Unchanged(Value),
    /// At least one path matched.
    Matched(Value),
    /// No path matched; the value is the full payload.
    Unmatched(Value),
}

/// Apply `--select` paths to a command result.
pub fn select(v: Value, paths: &[Vec<String>]) -> Selected {
    if paths.is_empty() || is_error_envelope(&v) {
        return Selected::Unchanged(v);
    }
    let wrapped = v.as_object().is_some_and(|m| m.contains_key("data"));
    let (envelope_paths, data_paths): (Vec<_>, Vec<_>) = paths
        .iter()
        .cloned()
        .partition(|p| ENVELOPE_KEYS.contains(&p[0].as_str()));
    // A leading `data` step is optional: on an envelope it names the data
    // part, and an unwrapped payload has no `data` key to collide with. A
    // bare `data` becomes the empty path, which keeps the whole data part.
    let data_paths: Vec<Vec<String>> = data_paths
        .into_iter()
        .map(|p| if p[0] == "data" { p[1..].to_vec() } else { p })
        .collect();

    let mut out: Option<Value> = None;
    if !data_paths.is_empty()
        && let Some(data) = data_part(&v)
    {
        let projected = if data_paths.iter().any(Vec::is_empty) {
            Some(data)
        } else {
            project(&data, &data_paths)
        };
        if let Some(p) = projected {
            out = Some(if wrapped {
                Value::Object(Map::from_iter([("data".to_string(), p)]))
            } else {
                p
            });
        }
    }
    if !envelope_paths.is_empty()
        && let Some(env) = project(&v, &envelope_paths)
    {
        out = Some(match out {
            Some(Value::Object(mut a)) if env.is_object() => {
                a.extend(env.as_object().unwrap().clone());
                Value::Object(a)
            }
            _ => env,
        });
    }
    match out {
        None => Selected::Unmatched(v),
        Some(u @ Value::Array(_)) => Selected::Matched(u),
        Some(u) if !u.is_object() => Selected::Matched(v),
        Some(Value::Object(u)) => {
            // Keep the envelope keys, then the projection.
            let mut m: Map<String, Value> = v
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter(|(k, _)| ENVELOPE_KEYS.contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default();
            m.extend(u);
            Selected::Matched(Value::Object(m))
        }
        Some(_) => unreachable!(),
    }
}

fn has_key(v: &Value, k: &str) -> bool {
    v.as_object().is_some_and(|m| m.contains_key(k))
}

/// Project a whole payload for `--agent-fields` (paths are relative to the
/// full payload, `data.` included). On an unwrapped payload a leading `data.`
/// is dropped. When nothing matches, an error payload keeps its code and
/// message and anything else becomes `{}`.
pub fn agent_fields(v: &Value, paths: &[Vec<String>]) -> Value {
    let paths = adapt(v, paths);
    if v.as_array().is_some_and(Vec::is_empty) {
        return Value::Array(Vec::new());
    }
    project(v, &paths).unwrap_or_else(|| match v.get("error") {
        Some(e) if e.is_object() => serde_json::json!({
            "error": {"code": e["code"], "message": e["message"]}
        }),
        _ => Value::Object(Map::new()),
    })
}

/// `--agent-fields` paths that are missing although their parent exists
/// (a path whose parent is missing too is just absent from this payload).
pub fn unresolved_fields(v: &Value, paths: &[Vec<String>]) -> Vec<String> {
    adapt(v, paths)
        .iter()
        .zip(paths)
        .filter(|(p, _)| {
            pick(v, p).is_none() && (p.len() <= 1 || pick(v, &p[..p.len() - 1]).is_some())
        })
        .map(|(_, orig)| orig.join("."))
        .collect()
}

fn adapt(v: &Value, paths: &[Vec<String>]) -> Vec<Vec<String>> {
    if has_key(v, "data") {
        return paths.to_vec();
    }
    paths
        .iter()
        .map(|p| {
            if p[0] == "data" && p.len() > 1 {
                p[1..].to_vec()
            } else {
                p.clone()
            }
        })
        .collect()
}

/// One `@name` preset: twg's paths, plus paths for atlx's current
/// unwrapped output shape that are only used when the payload has no `data`
/// envelope.
pub struct Preset {
    pub name: &'static str,
    pub twg: &'static [&'static str],
    pub atlx: &'static [&'static str],
}

type Table = &'static [Preset];

/// Per-command presets, keyed by the dotted command path.
///
/// The `twg` lists match what twg 1.4.4 expands each preset to, so output in
/// twg's envelope (#21) projects exactly as in twg. The `atlx` lists cover
/// the keys atlx prints today without the envelope; drop them once every
/// command emits twg's shape.
pub fn presets(command: &str) -> Option<Table> {
    let cmd = command.replacen("bb.", "bitbucket.", 1);
    Some(match cmd.as_str() {
        "bitbucket.pipeline.get" => BITBUCKET_PIPELINE_GET,
        "bitbucket.pipeline.query" => BITBUCKET_PIPELINE_QUERY,
        "bitbucket.pull-requests.activity" => BITBUCKET_PULL_REQUESTS_ACTIVITY,
        "bitbucket.pull-requests.get" => BITBUCKET_PULL_REQUESTS_GET,
        "bitbucket.pull-requests.query" => BITBUCKET_PULL_REQUESTS_QUERY,
        "bitbucket.repo.query" => BITBUCKET_REPO_QUERY,
        "confluence.content.get" => CONFLUENCE_CONTENT_GET,
        "confluence.search.query" => CONFLUENCE_SEARCH_QUERY,
        "confluence.search.text" => CONFLUENCE_SEARCH_TEXT,
        "jira.workitem.get" => JIRA_WORKITEM_GET,
        "jira.workitem.query" => JIRA_WORKITEM_QUERY,
        "jira.workitem.search" => JIRA_WORKITEM_SEARCH,
        "user.search" => USER_SEARCH,
        _ => return None,
    })
}

const BITBUCKET_PIPELINE_GET: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "uuid",
            "build_number",
            "repository.full_name",
            "state.name",
            "state.result.name",
            "created_on",
            "completed_on",
            "duration_in_seconds",
            "target.ref_name",
            "target.commit.hash",
            "target.pullrequest.id",
            "trigger.name",
            "creator.display_name",
            "steps.uuid",
            "steps.name",
            "steps.state.name",
            "steps.state.result.name",
            "steps.duration_in_seconds",
            "steps.log",
            "steps.log_error",
            "steps.log_total_lines",
            "steps.log_truncated",
            "log_error",
            "test_result_selection",
            "test_results.step.uuid",
            "test_results.step.name",
            "test_results.test_report",
            "test_results.test_report_error",
            "test_results.test_cases.name",
            "test_results.test_cases.status",
            "test_results.test_cases.duration",
            "test_results.test_cases_truncated",
            "test_results.test_cases_error",
        ],
        atlx: &[
            "build",
            "state",
            "result",
            "branch",
            "url",
            "steps.state",
            "steps.result",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "uuid",
            "build_number",
            "repository.full_name",
            "state.name",
            "state.result.name",
            "created_on",
            "completed_on",
            "duration_in_seconds",
            "target.ref_name",
            "target.commit.hash",
            "target.pullrequest.id",
            "trigger.name",
            "creator.display_name",
            "steps.uuid",
            "steps.name",
            "steps.state.name",
            "steps.state.result.name",
            "steps.duration_in_seconds",
            "steps.log",
            "steps.log_error",
            "steps.log_total_lines",
            "steps.log_truncated",
            "log_error",
            "test_result_selection",
            "test_results.step.uuid",
            "test_results.step.name",
            "test_results.test_report",
            "test_results.test_report_error",
            "test_results.test_cases.name",
            "test_results.test_cases.status",
            "test_results.test_cases.duration",
            "test_results.test_cases_truncated",
            "test_results.test_cases_error",
        ],
        atlx: &[
            "build",
            "state",
            "result",
            "branch",
            "url",
            "steps.state",
            "steps.result",
        ],
    },
];

const BITBUCKET_PIPELINE_QUERY: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "uuid",
            "build_number",
            "_repo",
            "repository.full_name",
            "state.name",
            "state.result.name",
            "state.stage.name",
            "created_on",
            "completed_on",
            "duration_in_seconds",
            "target.type",
            "target.ref_type",
            "target.ref_name",
            "target.source",
            "target.destination",
            "target.pullrequest.id",
            "target.selector.type",
            "target.selector.pattern",
            "target.commit.hash",
            "trigger.name",
            "links.html.href",
        ],
        atlx: &["build", "state", "result", "branch", "url"],
    },
    Preset {
        name: "evidence",
        twg: &[
            "uuid",
            "build_number",
            "_repo",
            "repository.full_name",
            "state.name",
            "state.result.name",
            "state.stage.name",
            "created_on",
            "completed_on",
            "duration_in_seconds",
            "target.type",
            "target.ref_type",
            "target.ref_name",
            "target.source",
            "target.destination",
            "target.pullrequest.id",
            "target.selector.type",
            "target.selector.pattern",
            "target.commit.hash",
            "trigger.name",
            "links.html.href",
            "creator.account_id",
            "creator.display_name",
            "target.commit.message",
            "target.destination_commit.hash",
            "target.pullrequest.title",
            "run_number",
            "build_seconds_used",
            "first_successful",
            "expired",
            "has_variables",
        ],
        atlx: &[
            "build", "state", "result", "branch", "url", "trigger", "commit", "created", "seconds",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "uuid",
            "build_number",
            "_repo",
            "repository.full_name",
            "state.name",
            "state.result.name",
            "state.stage.name",
            "created_on",
            "completed_on",
            "duration_in_seconds",
            "target.type",
            "target.ref_type",
            "target.ref_name",
            "target.source",
            "target.destination",
            "target.pullrequest.id",
            "target.selector.type",
            "target.selector.pattern",
            "target.commit.hash",
            "trigger.name",
            "links.html.href",
        ],
        atlx: &["build", "state", "result", "branch", "url"],
    },
];

const BITBUCKET_PULL_REQUESTS_ACTIVITY: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "pull_request.id",
            "update.date",
            "update.state",
            "update.author.display_name",
            "update.reason",
            "update.changes.reviewers.added.display_name",
            "update.changes.reviewers.removed.display_name",
            "update.source.commit.hash",
            "approval.date",
            "approval.user.display_name",
            "changes_requested.date",
            "changes_requested.user.display_name",
            "comment.id",
            "comment.created_on",
            "comment.user.display_name",
            "comment.content.raw",
            "comment.inline.path",
        ],
        atlx: &[],
    },
    Preset {
        name: "rows",
        twg: &[
            "pull_request.id",
            "update.date",
            "update.state",
            "update.author.display_name",
            "update.reason",
            "update.changes.reviewers.added.display_name",
            "update.changes.reviewers.removed.display_name",
            "update.source.commit.hash",
            "approval.date",
            "approval.user.display_name",
            "changes_requested.date",
            "changes_requested.user.display_name",
            "comment.id",
            "comment.created_on",
            "comment.user.display_name",
            "comment.content.raw",
            "comment.inline.path",
        ],
        atlx: &[],
    },
];

const BITBUCKET_PULL_REQUESTS_GET: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "id",
            "title",
            "links.html.href",
            "state",
            "author.display_name",
            "updated_on",
            "source.branch.name",
            "destination.branch.name",
            "destination.repository.full_name",
            "_statuses.name",
            "_statuses.state",
            "_tasks.state",
            "_tasks.content.raw",
            "_commits.hash",
            "_commits.message",
            "_commits.date",
            "_comments.content.raw",
            "_comments.user.display_name",
            "_failures.source",
            "_failures.operation",
            "_failures.message",
            "_failures.statusCode",
        ],
        atlx: &["author", "source", "destination", "updated", "url"],
    },
    Preset {
        name: "evidence",
        twg: &[
            "id",
            "title",
            "description",
            "links.html.href",
            "state",
            "author.display_name",
            "created_on",
            "updated_on",
            "source.branch.name",
            "destination.branch.name",
            "destination.repository.full_name",
            "_statuses.name",
            "_statuses.state",
            "_tasks.state",
            "_tasks.content.raw",
            "_commits.hash",
            "_commits.message",
            "_commits.date",
            "_comments.content.raw",
            "_comments.user.display_name",
            "_failures",
        ],
        atlx: &["author", "source", "destination", "updated", "url"],
    },
    Preset {
        name: "rows",
        twg: &[
            "id",
            "title",
            "links.html.href",
            "state",
            "author.display_name",
            "updated_on",
            "source.branch.name",
            "destination.branch.name",
            "destination.repository.full_name",
            "_statuses.name",
            "_statuses.state",
            "_tasks.state",
            "_tasks.content.raw",
            "_commits.hash",
            "_commits.message",
            "_commits.date",
            "_comments.content.raw",
            "_comments.user.display_name",
            "_failures.source",
            "_failures.operation",
            "_failures.message",
            "_failures.statusCode",
        ],
        atlx: &["author", "source", "destination", "updated", "url"],
    },
];

const BITBUCKET_PULL_REQUESTS_QUERY: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "id",
            "title",
            "links.html.href",
            "state",
            "updated_on",
            "destination.repository.name",
            "values.id",
            "values.title",
            "values.links.html.href",
            "values.state",
            "values.updated_on",
            "values.destination.repository.name",
            "page",
            "pagelen",
            "size",
            "hasNextPage",
            "returned",
            "hasMore",
            "pageAppliesTo",
            "partial",
            "warnings",
            "failures",
        ],
        atlx: &["author", "updated", "url"],
    },
    Preset {
        name: "evidence",
        twg: &[
            "id",
            "title",
            "links.html.href",
            "state",
            "updated_on",
            "destination.repository.name",
            "description",
            "author.display_name",
            "values.id",
            "values.title",
            "values.links.html.href",
            "values.state",
            "values.updated_on",
            "values.destination.repository.name",
            "values.description",
            "values.author.display_name",
            "page",
            "pagelen",
            "size",
            "hasNextPage",
            "returned",
            "hasMore",
            "pageAppliesTo",
            "partial",
            "warnings",
            "failures",
        ],
        atlx: &["author", "updated", "url", "source", "destination"],
    },
    Preset {
        name: "rows",
        twg: &[
            "id",
            "title",
            "links.html.href",
            "state",
            "updated_on",
            "destination.repository.name",
            "values.id",
            "values.title",
            "values.links.html.href",
            "values.state",
            "values.updated_on",
            "values.destination.repository.name",
            "page",
            "pagelen",
            "size",
            "hasNextPage",
            "returned",
            "hasMore",
            "pageAppliesTo",
            "partial",
            "warnings",
            "failures",
        ],
        atlx: &["author", "updated", "url"],
    },
];

const BITBUCKET_REPO_QUERY: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "uuid",
            "slug",
            "name",
            "full_name",
            "links.html.href",
            "language",
            "updated_on",
            "project.key",
        ],
        atlx: &["repo", "project", "updated", "url"],
    },
    Preset {
        name: "evidence",
        twg: &[
            "uuid",
            "slug",
            "name",
            "full_name",
            "description",
            "links.html.href",
            "language",
            "is_private",
            "created_on",
            "updated_on",
            "project.key",
            "project.name",
            "mainbranch.name",
        ],
        atlx: &[
            "repo",
            "project",
            "updated",
            "url",
            "private",
            "main_branch",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "uuid",
            "slug",
            "name",
            "full_name",
            "links.html.href",
            "language",
            "updated_on",
            "project.key",
        ],
        atlx: &["repo", "project", "updated", "url"],
    },
];

const CONFLUENCE_CONTENT_GET: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "data.id",
            "data.type",
            "data.title",
            "data.status",
            "data.spaceId",
            "data.externalUrl",
            "data.detail",
            "data.snapshotToken",
            "data.aiSummary",
            "data.summary.excerpt",
            "data.summary.wordCount",
            "data.summary.sectionCount",
            "data.body.format",
            "data.body.lossyConversion",
            "data.body.lostFeatures",
            "data.body.outputFile",
            "data.summary.requested",
            "data.summary.succeeded",
            "data.summary.failed",
            "data.items.input",
            "data.items.ok",
            "data.items.error",
            "data.items.meta",
            "data.items.data.id",
            "data.items.data.type",
            "data.items.data.title",
            "data.items.data.status",
            "data.items.data.spaceId",
            "data.items.data.externalUrl",
            "data.items.data.detail",
            "data.items.data.snapshotToken",
            "data.items.data.aiSummary",
            "data.items.data.summary.excerpt",
            "data.items.data.summary.wordCount",
            "data.items.data.summary.sectionCount",
            "data.items.data.body.format",
            "data.items.data.body.lossyConversion",
            "data.items.data.body.lostFeatures",
            "data.items.data.body.outputFile",
        ],
        atlx: &[
            "data.space_id",
            "data.version",
            "data.snapshot_token",
            "data.updated",
            "data.url",
            "data.format",
            "data.items.error.message",
            "data.items.data.space_id",
            "data.items.data.snapshot_token",
            "data.items.data.url",
        ],
    },
    Preset {
        name: "evidence",
        twg: &[
            "data.id",
            "data.type",
            "data.title",
            "data.status",
            "data.spaceId",
            "data.externalUrl",
            "data.detail",
            "data.snapshotToken",
            "data.aiSummary",
            "data.summary",
            "data.outline.level",
            "data.outline.text",
            "data.outline.anchor",
            "data.body.format",
            "data.body.lossyConversion",
            "data.body.lostFeatures",
            "data.body.value",
            "data.body.outputFile",
            "data.summary.requested",
            "data.summary.succeeded",
            "data.summary.failed",
            "data.items.input",
            "data.items.ok",
            "data.items.error",
            "data.items.meta",
            "data.items.data.id",
            "data.items.data.type",
            "data.items.data.title",
            "data.items.data.status",
            "data.items.data.spaceId",
            "data.items.data.externalUrl",
            "data.items.data.detail",
            "data.items.data.snapshotToken",
            "data.items.data.aiSummary",
            "data.items.data.summary",
            "data.items.data.outline.level",
            "data.items.data.outline.text",
            "data.items.data.outline.anchor",
            "data.items.data.body.format",
            "data.items.data.body.lossyConversion",
            "data.items.data.body.lostFeatures",
            "data.items.data.body.value",
            "data.items.data.body.outputFile",
        ],
        atlx: &[
            "data.space_id",
            "data.version",
            "data.snapshot_token",
            "data.updated",
            "data.url",
            "data.format",
            "data.excerpt",
            "data.body",
            "data.items.error.message",
            "data.items.data.space_id",
            "data.items.data.snapshot_token",
            "data.items.data.url",
        ],
    },
    Preset {
        name: "metadata",
        twg: &[
            "data.id",
            "data.type",
            "data.title",
            "data.status",
            "data.spaceId",
            "data.externalUrl",
            "data.detail",
            "data.snapshotToken",
            "data.aiSummary",
            "data.summary.excerpt",
            "data.summary.wordCount",
            "data.summary.sectionCount",
            "data.body.format",
            "data.body.lossyConversion",
            "data.body.lostFeatures",
            "data.body.outputFile",
            "data.summary.requested",
            "data.summary.succeeded",
            "data.summary.failed",
            "data.items.input",
            "data.items.ok",
            "data.items.error",
            "data.items.meta",
            "data.items.data.id",
            "data.items.data.type",
            "data.items.data.title",
            "data.items.data.status",
            "data.items.data.spaceId",
            "data.items.data.externalUrl",
            "data.items.data.detail",
            "data.items.data.snapshotToken",
            "data.items.data.aiSummary",
            "data.items.data.summary.excerpt",
            "data.items.data.summary.wordCount",
            "data.items.data.summary.sectionCount",
            "data.items.data.body.format",
            "data.items.data.body.lossyConversion",
            "data.items.data.body.lostFeatures",
            "data.items.data.body.outputFile",
        ],
        atlx: &[
            "data.space_id",
            "data.version",
            "data.snapshot_token",
            "data.updated",
            "data.url",
            "data.format",
            "data.items.error.message",
            "data.items.data.space_id",
            "data.items.data.snapshot_token",
            "data.items.data.url",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "data.id",
            "data.type",
            "data.title",
            "data.status",
            "data.spaceId",
            "data.externalUrl",
            "data.detail",
            "data.snapshotToken",
            "data.aiSummary",
            "data.summary.excerpt",
            "data.summary.wordCount",
            "data.summary.sectionCount",
            "data.body.format",
            "data.body.lossyConversion",
            "data.body.lostFeatures",
            "data.body.outputFile",
            "data.summary.requested",
            "data.summary.succeeded",
            "data.summary.failed",
            "data.items.input",
            "data.items.ok",
            "data.items.error",
            "data.items.meta",
            "data.items.data.id",
            "data.items.data.type",
            "data.items.data.title",
            "data.items.data.status",
            "data.items.data.spaceId",
            "data.items.data.externalUrl",
            "data.items.data.detail",
            "data.items.data.snapshotToken",
            "data.items.data.aiSummary",
            "data.items.data.summary.excerpt",
            "data.items.data.summary.wordCount",
            "data.items.data.summary.sectionCount",
            "data.items.data.body.format",
            "data.items.data.body.lossyConversion",
            "data.items.data.body.lostFeatures",
            "data.items.data.body.outputFile",
        ],
        atlx: &[
            "data.space_id",
            "data.version",
            "data.snapshot_token",
            "data.updated",
            "data.url",
            "data.format",
            "data.items.error.message",
            "data.items.data.space_id",
            "data.items.data.snapshot_token",
            "data.items.data.url",
        ],
    },
];

const CONFLUENCE_SEARCH_QUERY: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "request.cql",
            "request.limit",
            "request.cursor",
            "data.results.entityType",
            "data.results.content.id",
            "data.results.content.title",
            "data.results.content.type",
            "data.results.content._links.webui",
            "data.results.space.key",
            "data.results.space.name",
            "data.results.space.type",
            "data.results.space.status",
            "data.results.url",
            "data.results.lastModified",
            "data.totalSize",
            "data.size",
            "data.nextCursor",
            "meta.acquisition",
            "meta.page",
            "meta.evidence",
        ],
        atlx: &[
            "data.results.id",
            "data.results.title",
            "data.results.type",
            "data.results.space",
            "data.next_cursor",
        ],
    },
    Preset {
        name: "evidence",
        twg: &[
            "request.cql",
            "request.limit",
            "request.cursor",
            "data.results.entityType",
            "data.results.content.id",
            "data.results.content.title",
            "data.results.content.type",
            "data.results.content._links.webui",
            "data.results.space.key",
            "data.results.space.name",
            "data.results.space.type",
            "data.results.space.status",
            "data.results.url",
            "data.results.lastModified",
            "data.results.friendlyLastModified",
            "data.results.excerpt",
            "data.results.breadcrumbs",
            "data.totalSize",
            "data.size",
            "data.nextCursor",
            "meta.acquisition",
            "meta.page",
            "meta.evidence",
        ],
        atlx: &[
            "data.results.id",
            "data.results.title",
            "data.results.type",
            "data.results.space",
            "data.next_cursor",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "request.cql",
            "request.limit",
            "request.cursor",
            "data.results.entityType",
            "data.results.content.id",
            "data.results.content.title",
            "data.results.content.type",
            "data.results.content._links.webui",
            "data.results.space.key",
            "data.results.space.name",
            "data.results.space.type",
            "data.results.space.status",
            "data.results.url",
            "data.results.lastModified",
            "data.totalSize",
            "data.size",
            "data.nextCursor",
            "meta.acquisition",
            "meta.page",
            "meta.evidence",
        ],
        atlx: &[
            "data.results.id",
            "data.results.title",
            "data.results.type",
            "data.results.space",
            "data.next_cursor",
        ],
    },
];

const CONFLUENCE_SEARCH_TEXT: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "request.query",
            "request.cql",
            "request.limit",
            "request.cursor",
            "data.results.entityType",
            "data.results.content.id",
            "data.results.content.title",
            "data.results.content.type",
            "data.results.content._links.webui",
            "data.results.space.key",
            "data.results.space.name",
            "data.results.space.type",
            "data.results.space.status",
            "data.results.url",
            "data.results.lastModified",
            "data.totalSize",
            "data.size",
            "data.nextCursor",
            "meta.acquisition",
            "meta.page",
            "meta.evidence",
        ],
        atlx: &[
            "data.results.id",
            "data.results.title",
            "data.results.type",
            "data.results.space",
            "data.next_cursor",
        ],
    },
    Preset {
        name: "evidence",
        twg: &[
            "request.query",
            "request.cql",
            "request.limit",
            "request.cursor",
            "data.results.entityType",
            "data.results.content.id",
            "data.results.content.title",
            "data.results.content.type",
            "data.results.content._links.webui",
            "data.results.space.key",
            "data.results.space.name",
            "data.results.space.type",
            "data.results.space.status",
            "data.results.url",
            "data.results.lastModified",
            "data.results.friendlyLastModified",
            "data.results.excerpt",
            "data.results.breadcrumbs",
            "data.totalSize",
            "data.size",
            "data.nextCursor",
            "meta.acquisition",
            "meta.page",
            "meta.evidence",
        ],
        atlx: &[
            "data.results.id",
            "data.results.title",
            "data.results.type",
            "data.results.space",
            "data.next_cursor",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "request.query",
            "request.cql",
            "request.limit",
            "request.cursor",
            "data.results.entityType",
            "data.results.content.id",
            "data.results.content.title",
            "data.results.content.type",
            "data.results.content._links.webui",
            "data.results.space.key",
            "data.results.space.name",
            "data.results.space.type",
            "data.results.space.status",
            "data.results.url",
            "data.results.lastModified",
            "data.totalSize",
            "data.size",
            "data.nextCursor",
            "meta.acquisition",
            "meta.page",
            "meta.evidence",
        ],
        atlx: &[
            "data.results.id",
            "data.results.title",
            "data.results.type",
            "data.results.space",
            "data.next_cursor",
        ],
    },
];

const JIRA_WORKITEM_GET: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "data.key",
            "data.summary",
            "data.status.name",
            "data.assignee.displayName",
            "data.url",
        ],
        atlx: &[
            "data.status",
            "data.assignee",
            "data.items.input",
            "data.items.ok",
            "data.items.error.message",
            "data.items.data.key",
            "data.items.data.summary",
            "data.items.data.status",
            "data.items.data.assignee",
            "data.items.data.url",
        ],
    },
    Preset {
        name: "evidence",
        twg: &[
            "data.key",
            "data.summary",
            "data.description",
            "data.status",
            "data.assignee",
            "data.creator",
            "data.reporter",
            "data.updated",
            "data.url",
        ],
        atlx: &[
            "data.items.input",
            "data.items.ok",
            "data.items.error.message",
            "data.items.data.key",
            "data.items.data.summary",
            "data.items.data.status",
            "data.items.data.assignee",
            "data.items.data.url",
            "data.items.data.description",
            "data.items.data.reporter",
            "data.items.data.updated",
        ],
    },
    Preset {
        name: "rows",
        twg: &[
            "data.key",
            "data.summary",
            "data.status.name",
            "data.assignee.displayName",
            "data.url",
        ],
        atlx: &[
            "data.status",
            "data.assignee",
            "data.items.input",
            "data.items.ok",
            "data.items.error.message",
            "data.items.data.key",
            "data.items.data.summary",
            "data.items.data.status",
            "data.items.data.assignee",
            "data.items.data.url",
        ],
    },
];

const JIRA_WORKITEM_QUERY: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "data.issues.key",
            "data.issues.summary",
            "data.issues.status.name",
            "data.issues.status.statusCategory.key",
            "data.issues.url",
            "data.issues.updated",
            "data.issues.issuetype.name",
            "data.issues.issueType.name",
            "data.issues.priority.name",
            "data.issues.assignee.displayName",
            "data.issues.assignee.accountId",
            "data.issues.reporter.displayName",
            "data.issues.creator.displayName",
            "data.issues.parent.key",
            "data.issues.created",
            "data.issues.issuelinks.type.name",
            "data.issues.issuelinks.inwardIssue.key",
            "data.issues.issuelinks.outwardIssue.key",
        ],
        atlx: &[
            "data.issues.status",
            "data.issues.assignee",
            "data.issues.type",
            "data.issues.priority",
            "data.next_cursor",
        ],
    },
    Preset {
        name: "evidence",
        twg: &[
            "data.issues.key",
            "data.issues.summary",
            "data.issues.status",
            "data.issues.assignee",
            "data.issues.creator",
            "data.issues.url",
            "data.issues.updated",
        ],
        atlx: &["data.next_cursor"],
    },
    Preset {
        name: "rows",
        twg: &[
            "data.issues.key",
            "data.issues.summary",
            "data.issues.status.name",
            "data.issues.status.statusCategory.key",
            "data.issues.url",
            "data.issues.updated",
            "data.issues.issuetype.name",
            "data.issues.issueType.name",
            "data.issues.priority.name",
            "data.issues.assignee.displayName",
            "data.issues.assignee.accountId",
            "data.issues.reporter.displayName",
            "data.issues.creator.displayName",
            "data.issues.parent.key",
            "data.issues.created",
            "data.issues.issuelinks.type.name",
            "data.issues.issuelinks.inwardIssue.key",
            "data.issues.issuelinks.outwardIssue.key",
        ],
        atlx: &[
            "data.issues.status",
            "data.issues.assignee",
            "data.issues.type",
            "data.issues.priority",
            "data.next_cursor",
        ],
    },
];

const JIRA_WORKITEM_SEARCH: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "data.issues.key",
            "data.issues.summary",
            "data.issues.status",
            "data.issues.url",
            "data.issues.updated",
        ],
        atlx: &["data.next_cursor"],
    },
    Preset {
        name: "evidence",
        twg: &[
            "data.issues.key",
            "data.issues.summary",
            "data.issues.status",
            "data.issues.assignee",
            "data.issues.creator",
            "data.issues.url",
            "data.issues.updated",
        ],
        atlx: &["data.next_cursor"],
    },
    Preset {
        name: "rows",
        twg: &[
            "data.issues.key",
            "data.issues.summary",
            "data.issues.status",
            "data.issues.url",
            "data.issues.updated",
        ],
        atlx: &["data.next_cursor"],
    },
];

const USER_SEARCH: Table = &[
    Preset {
        name: "compact",
        twg: &[
            "data.name",
            "data.email",
            "data.accountId",
            "data.userAri",
            "data.sources",
            "data.slackStatus",
            "meta.guidance",
        ],
        atlx: &["data.account_id", "data.display_name"],
    },
    Preset {
        name: "evidence",
        twg: &[
            "data.name",
            "data.email",
            "data.accountId",
            "data.userAri",
            "data.sources",
            "data.slackStatus",
            "data.slack",
            "meta.guidance",
        ],
        atlx: &["data.account_id", "data.display_name", "data.active"],
    },
    Preset {
        name: "rows",
        twg: &[
            "data.name",
            "data.email",
            "data.accountId",
            "data.userAri",
            "data.sources",
            "data.slackStatus",
            "meta.guidance",
        ],
        atlx: &["data.account_id", "data.display_name"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paths(s: &str) -> Vec<Vec<String>> {
        parse_paths(s.split(','))
    }

    fn sel(v: Value, s: &str) -> Selected {
        select(v, &paths(s))
    }

    #[test]
    fn parses_and_dedupes_paths() {
        assert_eq!(
            paths(" a.b , a.b,c..d,"),
            vec![vec!["a", "b"], vec!["c", "d"]]
        );
    }

    #[test]
    fn projects_nested_objects_and_arrays() {
        let v = json!({"issues": [
            {"key": "A-1", "status": {"name": "Done", "id": "3"}, "x": 1},
            {"key": "A-2", "status": {"name": "To Do", "id": "1"}, "x": 2}
        ], "total": 2});
        assert_eq!(
            sel(v, "issues.key,issues.status.name"),
            Selected::Matched(json!({"issues": [
                {"key": "A-1", "status": {"name": "Done"}},
                {"key": "A-2", "status": {"name": "To Do"}}
            ]}))
        );
    }

    #[test]
    fn path_order_sets_key_order() {
        let v = json!({"status": {"id": "3", "name": "Done"}});
        let Selected::Matched(out) = sel(v, "status.name,status") else {
            panic!()
        };
        assert_eq!(
            serde_json::to_string(&out).unwrap(),
            r#"{"status":{"name":"Done","id":"3"}}"#
        );
    }

    #[test]
    fn missing_paths_are_skipped_or_fall_back() {
        let v = json!({"issues": [{"key": "A-1", "summary": "s"}]});
        assert_eq!(
            sel(v.clone(), "issues.key,issues.nope"),
            Selected::Matched(json!({"issues": [{"key": "A-1"}]}))
        );
        assert_eq!(sel(v.clone(), "nope"), Selected::Unmatched(v.clone()));
        // A path that runs past a scalar does not match.
        assert_eq!(
            sel(v.clone(), "issues.summary.x"),
            Selected::Unmatched(v.clone())
        );
        // Indexes are not paths.
        assert_eq!(sel(v.clone(), "issues.0.key"), Selected::Unmatched(v));
    }

    #[test]
    fn unmatched_elements_become_null_and_nulls_are_kept() {
        let v = json!([{"a": {"b": 1}}, {"c": 2}, {"a": null}]);
        assert_eq!(
            sel(v, "a.b"),
            Selected::Matched(json!([{"a": {"b": 1}}, null, {"a": null}]))
        );
    }

    #[test]
    fn data_prefix_is_optional_on_both_shapes() {
        let flat = json!({"issues": [{"key": "A-1", "summary": "s"}]});
        let want = Selected::Matched(json!({"issues": [{"key": "A-1"}]}));
        assert_eq!(sel(flat.clone(), "data.issues.key"), want);
        assert_eq!(sel(flat.clone(), "issues.key"), want);
        assert_eq!(sel(flat.clone(), "data"), Selected::Matched(flat));

        let env = json!({
            "apiVersion": "v2", "command": "jira.workitem.query",
            "request": {"jql": "x"}, "data": {"issues": [{"key": "A-1", "summary": "s"}]},
            "pageInfo": {"hasNextPage": false}, "meta": {"resourceClass": "entity"}
        });
        let want = json!({
            "apiVersion": "v2", "command": "jira.workitem.query",
            "meta": {"resourceClass": "entity"},
            "data": {"issues": [{"key": "A-1"}]}
        });
        assert_eq!(
            sel(env.clone(), "data.issues.key"),
            Selected::Matched(want.clone())
        );
        assert_eq!(sel(env.clone(), "issues.key"), Selected::Matched(want));
        assert_eq!(sel(env.clone(), "pageInfo"), Selected::Unmatched(env));
    }

    #[test]
    fn error_envelopes_pass_through() {
        let e = json!({"ok": false, "error": {"code": "NOT_FOUND"}});
        assert_eq!(sel(e.clone(), "key"), Selected::Unchanged(e));
    }

    #[test]
    fn presets_expand_per_command() {
        let f = resolve("--select", "@compact", "jira.workitem.get").unwrap();
        assert!(f.dotted().contains(&"data.status.name".to_string()));
        let v = json!({"key": "A-1", "summary": "s", "status": "Done", "type": "Bug",
                       "assignee": null, "url": "u", "updated": "t"});
        assert_eq!(
            select(v.clone(), &f.for_payload(&v)),
            Selected::Matched(
                json!({"key": "A-1", "summary": "s", "status": "Done", "assignee": null, "url": "u"})
            )
        );
        // Mixed presets and paths.
        let f = resolve("--select", "@evidence,fields.x", "jira.workitem.search").unwrap();
        assert_eq!(f.paths.last().unwrap(), &vec!["fields", "x"]);
        // `bb` and `bitbucket` share the table.
        assert!(presets("bb.repo.query").is_some());
    }

    #[test]
    fn unknown_presets() {
        let e = resolve("--select", "@bogus", "jira.workitem.get").unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("@compact, @evidence, @rows"), "{msg}");
        // Commands without a table: universal presets warn, others fail.
        let f = resolve(
            "--select",
            "@compact,a.b",
            "jira.workitem.transitions.query",
        )
        .unwrap();
        assert_eq!(f.unresolved, vec!["@compact"]);
        assert_eq!(f.dotted(), vec!["a.b"]);
        assert!(resolve("--select", "@rows", "jira.workitem.transitions.query").is_err());
    }

    #[test]
    fn agent_fields_project_the_whole_payload() {
        let env = json!({"command": "c", "data": {"issues": [{"key": "A", "x": 1}]}, "request": {"q": 1}});
        assert_eq!(
            agent_fields(&env, &paths("data.issues.key,request.q")),
            json!({"data": {"issues": [{"key": "A"}]}, "request": {"q": 1}})
        );
        let flat = json!({"issues": [{"key": "A", "x": 1}]});
        assert_eq!(
            agent_fields(&flat, &paths("data.issues.key")),
            json!({"issues": [{"key": "A"}]})
        );
        assert_eq!(
            unresolved_fields(&flat, &paths("issues.nope,nope.deep")),
            vec!["issues.nope"]
        );
        let err = json!({"ok": false, "error": {"code": "E", "message": "m", "kind": "k"}});
        assert_eq!(
            agent_fields(&err, &paths("data.key")),
            json!({"error": {"code": "E", "message": "m"}})
        );
    }
}
