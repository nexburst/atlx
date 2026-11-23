//! What Bitbucket commands return: one [`Record`] per resource, read from
//! the REST 2.0 objects.
//!
//! People are `{id, name}`: the Bitbucket user UUID (which every command
//! that takes a user accepts) and the display name.

use crate::schema::{Budget, Field, Record, Source};
use serde_json::{Value, json};

/// A user object at `path` as `{id, name}`; `null` when absent.
fn user_at(v: &Value, path: &str) -> Value {
    let u = crate::schema::path(v, path).unwrap_or(&Value::Null);
    if !u.is_object() {
        return Value::Null;
    }
    let id = u["uuid"].clone();
    let name = u["display_name"]
        .as_str()
        .or(u["nickname"].as_str())
        .map_or(Value::Null, Value::from);
    if id.is_null() && name.is_null() {
        return Value::Null;
    }
    json!({"id": id, "name": name})
}

fn first_line(v: Option<&Value>) -> Value {
    v.and_then(Value::as_str)
        .and_then(|m| m.lines().next())
        .map_or(Value::Null, |l| l.trim().into())
}

fn html(v: &Value) -> Value {
    v["links"]["html"]["href"].clone()
}

// --- shared ------------------------------------------------------------------

/// `{id, deleted}` after a delete.
pub const DELETED: Record = Record {
    name: "bitbucket.deleted",
    description: "what a delete removed",
    primary: "id",
    fields: &[
        Field::new("id", "string", "the removed item's ID or name", "id"),
        Field::new("deleted", "boolean", "always true", "deleted"),
    ],
};

/// A Bitbucket user (default reviewers, PR reviewers).
pub const USER: Record = Record {
    name: "bitbucket.user",
    description: "a Bitbucket user",
    primary: "id",
    fields: &[
        Field::new("id", "string", "user UUID", "").source(Source::First(&["user.uuid", "uuid"])),
        Field::new("name", "string", "display name", "")
            .source(Source::First(&["user.display_name", "display_name"])),
        Field::new("nickname", "string", "nickname", "")
            .source(Source::First(&["user.nickname", "nickname"])),
        Field::new("accountId", "string", "Atlassian account ID", "")
            .source(Source::First(&["user.account_id", "account_id"])),
        Field::new(
            "reviewerType",
            "string",
            "repository or project (effective default reviewers)",
            "reviewer_type",
        )
        .full(),
    ],
};

// --- repositories ------------------------------------------------------------

fn repo_workspace(v: &Value) -> Value {
    match v["workspace"]["slug"].as_str() {
        Some(s) => s.into(),
        None => v["full_name"]
            .as_str()
            .and_then(|f| f.split('/').next())
            .map_or(Value::Null, Value::from),
    }
}

pub const REPO: Record = Record {
    name: "bitbucket.repo",
    description: "a repository",
    primary: "fullName",
    fields: &[
        Field::new(
            "fullName",
            "string",
            "workspace/slug, what --repo takes",
            "full_name",
        ),
        Field::new("name", "string", "display name", "name"),
        Field::new("workspace", "string", "workspace slug", "").source(Source::Fn(repo_workspace)),
        Field::new("project", "string", "project key", "project.key"),
        Field::new("private", "boolean", "private repository", "is_private"),
        Field::new("language", "string", "main language", "language"),
        Field::new("mainBranch", "string", "main branch", "mainbranch.name"),
        Field::new("updated", "datetime", "last update", "updated_on"),
        Field::new("url", "string", "web URL", "").source(Source::Fn(html)),
        Field::new("description", "string", "description", "description").full(),
        Field::new("created", "datetime", "creation time", "created_on").full(),
        Field::new("size", "integer", "size in bytes", "size").full(),
        Field::new("uuid", "string", "repository UUID", "uuid").full(),
        Field::new("forkPolicy", "string", "fork policy", "fork_policy").full(),
    ],
};

fn author_name(v: &Value) -> Value {
    v["author"]["user"]["display_name"]
        .as_str()
        .or(v["author"]["raw"].as_str())
        .map_or(Value::Null, Value::from)
}

fn branch_author(v: &Value) -> Value {
    author_name(&v["target"])
}

fn branch_title(v: &Value) -> Value {
    first_line(v.pointer("/target/message"))
}

pub const BRANCH: Record = Record {
    name: "bitbucket.branch",
    description: "a branch and its head commit",
    primary: "name",
    fields: &[
        Field::new("name", "string", "branch name", "name"),
        Field::new("commit", "string", "head commit hash", "target.hash"),
        Field::new("date", "datetime", "head commit date", "target.date"),
        Field::new("author", "string", "head commit author", "").source(Source::Fn(branch_author)),
        Field::new("title", "string", "head commit message, first line", "")
            .source(Source::Fn(branch_title)),
        Field::new("url", "string", "web URL", "")
            .source(Source::Fn(html))
            .full(),
        Field::new(
            "mergeStrategies",
            "string[]",
            "merge strategies allowed into it",
            "merge_strategies",
        )
        .full(),
        Field::new(
            "defaultMergeStrategy",
            "string",
            "default merge strategy",
            "default_merge_strategy",
        )
        .full(),
    ],
};

fn commit_title(v: &Value) -> Value {
    first_line(v.get("message"))
}

fn parents(v: &Value) -> Value {
    v["parents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["hash"].clone())
        .collect()
}

pub const COMMIT: Record = Record {
    name: "bitbucket.commit",
    description: "a commit",
    primary: "hash",
    fields: &[
        Field::new("hash", "string", "commit hash", "hash"),
        Field::new("date", "datetime", "commit date", "date"),
        Field::new("author", "string", "author name", "").source(Source::Fn(author_name)),
        Field::new("title", "string", "message, first line", "").source(Source::Fn(commit_title)),
        Field::new("url", "string", "web URL", "").source(Source::Fn(html)),
        Field::new("message", "string", "whole message", "message").full(),
        Field::new(
            "authorRaw",
            "string",
            "author as `Name <email>`",
            "author.raw",
        )
        .full(),
        Field::new("parents", "string[]", "parent hashes", "")
            .source(Source::Fn(parents))
            .full(),
    ],
};

pub const COMMIT_RESULT: Record = Record {
    name: "bitbucket.commit-result",
    description: "a commit made through the API",
    primary: "hash",
    fields: &[
        Field::new("hash", "string", "the new commit", "hash"),
        Field::new(
            "branch",
            "string",
            "branch committed to (null: the main branch)",
            "branch",
        ),
        Field::new("message", "string", "commit message", "message"),
        Field::new(
            "files",
            "string[]",
            "repository paths written or deleted",
            "files",
        ),
    ],
};

/// Bitbucket files: 20 000 characters, continued with `--content-offset`.
pub const CONTENT: Budget = Budget::chars(20_000, "content-offset");

pub const FILE: Record = Record {
    name: "bitbucket.file",
    description: "a file with its text",
    primary: "path",
    fields: &[
        Field::new("path", "string", "path in the repository", "path"),
        Field::new("ref", "string", "branch, tag or commit read", "ref"),
        Field::new("commit", "string", "commit the file was read at", "commit"),
        Field::new("size", "integer", "size in bytes", "size"),
        Field::new(
            "content",
            "string",
            "the text (null for a binary file)",
            "content",
        )
        .truncated(CONTENT),
    ],
};

fn entry_type(v: &Value) -> Value {
    match v["type"].as_str() {
        Some("commit_directory") => "dir".into(),
        Some("commit_file") => "file".into(),
        Some(other) => other.into(),
        None => Value::Null,
    }
}

pub const FILE_ENTRY: Record = Record {
    name: "bitbucket.file-entry",
    description: "a directory entry",
    primary: "path",
    fields: &[
        Field::new("path", "string", "path in the repository", "path"),
        Field::new("type", "string", "file or dir", "").source(Source::Fn(entry_type)),
        Field::new("size", "integer", "size in bytes (files)", "size"),
        Field::new("commit", "string", "commit it was listed at", "commit.hash").full(),
        Field::new("mimetype", "string", "MIME type (files)", "mimetype").full(),
    ],
};

pub const DOWNLOAD: Record = Record {
    name: "bitbucket.download",
    description: "a file saved to disk",
    primary: "file",
    fields: &[
        Field::new("path", "string", "path in the repository", "path"),
        Field::new("file", "string", "local file written", "file"),
        Field::new("bytes", "integer", "bytes written", "bytes"),
    ],
};

fn chunk_count(v: &Value) -> Value {
    v["matched_chunks"].as_array().map_or(0, Vec::len).into()
}

/// `workspace/slug` from a hit's web URL.
fn hit_repo(v: &Value) -> Value {
    let Some(url) = v["url"].as_str() else {
        return Value::Null;
    };
    let rest = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let mut parts = rest.split('/').skip(1);
    match (parts.next(), parts.next()) {
        (Some(ws), Some(slug)) if !ws.is_empty() && !slug.is_empty() => {
            format!("{ws}/{slug}").into()
        }
        _ => Value::Null,
    }
}

pub const CODE_HIT: Record = Record {
    name: "bitbucket.code-hit",
    description: "a file that matched a code search",
    primary: "path",
    fields: &[
        Field::new("path", "string", "file path", "path"),
        Field::new("repo", "string", "repository (from the URL)", "").source(Source::Fn(hit_repo)),
        Field::new(
            "ref",
            "string",
            "branch or tag searched",
            "branch_or_tag_name",
        ),
        Field::new("matches", "integer", "matching chunks", "").source(Source::Fn(chunk_count)),
        Field::new("url", "string", "web URL", "url"),
        Field::new("language", "string", "language", "language").full(),
        Field::new("commit", "string", "commit searched", "version").full(),
        Field::new(
            "chunks",
            "object[]",
            "the matching chunks",
            "matched_chunks",
        )
        .full(),
    ],
};

fn contributor(v: &Value) -> Value {
    author_name(v)
}

pub const CONTRIBUTOR: Record = Record {
    name: "bitbucket.contributor",
    description: "a commit author and how many commits they made",
    primary: "name",
    fields: &[
        Field::new("name", "string", "author", "").source(Source::Fn(contributor)),
        Field::new(
            "id",
            "string",
            "Bitbucket user UUID, if linked",
            "author.user.uuid",
        ),
        Field::new("commits", "integer", "commits counted", "commits"),
        Field::new("raw", "string", "author as `Name <email>`", "author.raw").full(),
    ],
};

pub const REPORT: Record = Record {
    name: "bitbucket.report",
    description: "a Code Insights report on a commit",
    primary: "id",
    fields: &[
        Field::new("id", "string", "report ID", "external_id"),
        Field::new("title", "string", "title", "title"),
        Field::new(
            "type",
            "string",
            "SECURITY, COVERAGE, TEST or BUG",
            "report_type",
        ),
        Field::new("result", "string", "PASSED, FAILED or PENDING", "result"),
        Field::new("reporter", "string", "who reported it", "reporter"),
        Field::new("updated", "datetime", "last update", "updated_on"),
        Field::new("link", "string", "link to the report", "link"),
        Field::new("details", "string", "details", "details").full(),
        Field::new("data", "object[]", "report data", "data").full(),
        Field::new("uuid", "string", "report UUID", "uuid").full(),
    ],
};

// --- workspaces --------------------------------------------------------------

pub const WORKSPACE: Record = Record {
    name: "bitbucket.workspace",
    description: "a workspace",
    primary: "slug",
    fields: &[
        Field::new(
            "slug",
            "string",
            "workspace slug, what --workspace takes",
            "slug",
        ),
        Field::new("name", "string", "display name", "name"),
        Field::new("private", "boolean", "private workspace", "is_private"),
        Field::new("url", "string", "web URL", "").source(Source::Fn(html)),
        Field::new("uuid", "string", "workspace UUID", "uuid").full(),
        Field::new("created", "datetime", "creation time", "created_on").full(),
    ],
};

pub const MEMBER: Record = Record {
    name: "bitbucket.member",
    description: "a workspace member",
    primary: "id",
    fields: &[
        Field::new("id", "string", "user UUID", "user.uuid"),
        Field::new("name", "string", "display name", "user.display_name"),
        Field::new("nickname", "string", "nickname", "user.nickname"),
        Field::new(
            "accountId",
            "string",
            "Atlassian account ID",
            "user.account_id",
        ),
        Field::new(
            "permission",
            "string",
            "owner, collaborator or member (with --permissions)",
            "permission",
        ),
    ],
};

// --- pull requests -----------------------------------------------------------

const PR_AUTHOR: Source = Source::Object(&[("id", "author.uuid"), ("name", "author.display_name")]);

fn reviewers(v: &Value) -> Value {
    let people = v["participants"].as_array();
    v["reviewers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let p = people
                .into_iter()
                .flatten()
                .find(|p| p["user"]["uuid"] == r["uuid"]);
            json!({
                "id": r["uuid"],
                "name": r["display_name"],
                "approved": p.is_some_and(|p| p["approved"] == true),
                "state": p.map_or(Value::Null, |p| p["state"].clone()),
            })
        })
        .collect()
}

fn approvals(v: &Value) -> Value {
    v["participants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["approved"] == true)
        .count()
        .into()
}

fn participants(v: &Value) -> Value {
    v["participants"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            json!({
                "id": p["user"]["uuid"], "name": p["user"]["display_name"],
                "role": p["role"], "approved": p["approved"], "state": p["state"],
            })
        })
        .collect()
}

/// The fields every pull request view has, in order.
macro_rules! pr_fields {
    ($($extra:expr),* $(,)?) => {
        &[
            Field::new("id", "integer", "pull request number", "id"),
            Field::new("title", "string", "title", "title"),
            Field::new("state", "string", "OPEN, MERGED, DECLINED or SUPERSEDED", "state"),
            Field::new("author", "user", "author ({id, name})", "").source(PR_AUTHOR),
            Field::new("source", "string", "source branch", "source.branch.name"),
            Field::new("destination", "string", "destination branch", "destination.branch.name"),
            Field::new("head", "string", "source head commit (for --if-head)", "source.commit.hash"),
            Field::new("draft", "boolean", "draft", "draft"),
            Field::new("comments", "integer", "comment count", "comment_count"),
            Field::new("tasks", "integer", "open task count", "task_count"),
            Field::new("created", "datetime", "creation time", "created_on"),
            Field::new("updated", "datetime", "last update", "updated_on"),
            Field::new("url", "string", "web URL", "").source(Source::Fn(html)),
            $($extra,)*
            Field::new("repo", "string", "repository (workspace/slug)", "destination.repository.full_name").full(),
            Field::new("closeSourceBranch", "boolean", "the source branch is deleted on merge", "close_source_branch").full(),
            Field::new("mergeCommit", "string", "merge commit hash", "merge_commit.hash").full(),
            Field::new("closedBy", "user", "who merged or declined it", "")
                .source(Source::Object(&[("id", "closed_by.uuid"), ("name", "closed_by.display_name")]))
                .full(),
            Field::new("reason", "string", "decline reason", "reason").full(),
            Field::new("destinationCommit", "string", "destination commit when last updated", "destination.commit.hash").full(),
            Field::new("mergedOn", "datetime", "merge time (merge-date filters only)", "merged_on").full(),
        ]
    };
}

/// A pull request as lists show it: no body.
pub const PR_ROW: Record = Record {
    name: "bitbucket.pr-row",
    description: "a pull request in a list (no description)",
    primary: "id",
    fields: pr_fields!(),
};

/// A pull request read by itself, with its description and reviewers.
pub const PR: Record = Record {
    name: "bitbucket.pr",
    description: "a pull request",
    primary: "id",
    fields: pr_fields!(
        Field::new(
            "reviewers",
            "object[]",
            "reviewers: {id, name, approved, state}",
            ""
        )
        .source(Source::Fn(reviewers)),
        Field::new("approvals", "integer", "approvals", "").source(Source::Fn(approvals)),
        Field::new("body", "markdown", "description", "description")
            .truncated(crate::schema::truncate::BODY),
        Field::new("participants", "object[]", "everyone who took part", "")
            .source(Source::Fn(participants))
            .full(),
    ),
};

fn comment_resolved(v: &Value) -> Value {
    v["resolution"].is_object().into()
}

pub const PR_COMMENT: Record = Record {
    name: "bitbucket.pr-comment",
    description: "a pull request comment",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "comment ID", "id"),
        Field::new("author", "user", "author ({id, name})", "").source(Source::Object(&[
            ("id", "user.uuid"),
            ("name", "user.display_name"),
        ])),
        Field::new("body", "markdown", "text", "content.raw")
            .truncated(crate::schema::truncate::BODY),
        Field::new(
            "path",
            "string",
            "file, for an inline comment",
            "inline.path",
        ),
        Field::new(
            "line",
            "integer",
            "new-side line, for an inline comment",
            "inline.to",
        ),
        Field::new(
            "fromLine",
            "integer",
            "old-side line, for an inline comment",
            "inline.from",
        ),
        Field::new("parent", "integer", "comment it replies to", "parent.id"),
        Field::new("resolved", "boolean", "its thread is resolved", "")
            .source(Source::Fn(comment_resolved)),
        Field::new(
            "pending",
            "boolean",
            "unpublished review feedback",
            "pending",
        ),
        Field::new("created", "datetime", "creation time", "created_on"),
        Field::new("updated", "datetime", "last update", "updated_on").full(),
        Field::new("url", "string", "web URL", "")
            .source(Source::Fn(html))
            .full(),
    ],
};

pub const PR_TASK: Record = Record {
    name: "bitbucket.pr-task",
    description: "a pull request task",
    primary: "id",
    fields: &[
        Field::new("id", "integer", "task ID", "id"),
        Field::new("body", "markdown", "text", "content.raw"),
        Field::new("state", "string", "RESOLVED or UNRESOLVED", "state"),
        Field::new("creator", "user", "creator ({id, name})", "").source(Source::Object(&[
            ("id", "creator.uuid"),
            ("name", "creator.display_name"),
        ])),
        Field::new(
            "comment",
            "integer",
            "comment it is attached to",
            "comment.id",
        ),
        Field::new("created", "datetime", "creation time", "created_on"),
        Field::new("updated", "datetime", "last update", "updated_on").full(),
        Field::new("resolvedBy", "user", "who resolved it", "")
            .source(Source::Object(&[
                ("id", "resolved_by.uuid"),
                ("name", "resolved_by.display_name"),
            ]))
            .full(),
    ],
};

pub const DIFFSTAT: Record = Record {
    name: "bitbucket.diffstat",
    description: "a file a pull request changes",
    primary: "path",
    fields: &[
        Field::new("path", "string", "path (the new one for a rename)", "")
            .source(Source::First(&["new.path", "old.path"])),
        Field::new(
            "status",
            "string",
            "added, removed, modified, renamed, merge conflict, ...",
            "status",
        ),
        Field::new("added", "integer", "lines added", "lines_added"),
        Field::new("removed", "integer", "lines removed", "lines_removed"),
        Field::new("oldPath", "string", "path before (renames)", "old.path").full(),
    ],
};

/// Pull request diffs: 40 000 characters, continued with `--diff-offset`.
pub const DIFF_BUDGET: Budget = Budget::chars(40_000, "diff-offset");

pub const PR_DIFF: Record = Record {
    name: "bitbucket.pr-diff",
    description: "a pull request's diff, or the diff lines that match --grep",
    primary: "pr",
    fields: &[
        Field::new("pr", "integer", "pull request number", "pr"),
        Field::new("path", "string", "the one file diffed (--path)", "path"),
        Field::new(
            "diff",
            "string",
            "unified diff, or mbox patch with --patch",
            "diff",
        )
        .truncated(DIFF_BUDGET),
        Field::new(
            "matches",
            "object[]",
            "with --grep: {path, type, line, fromLine, text, hunk}; line/fromLine anchor an inline comment",
            "matches",
        ),
    ],
};

fn activity_type(v: &Value) -> Value {
    crate::bitbucket::activity_kind(v).into()
}

fn activity_body(v: &Value) -> &Value {
    let kind = crate::bitbucket::activity_kind(v);
    &v[kind]
}

fn activity_detail(v: &Value) -> Value {
    activity_body(v).clone()
}

fn activity_date(v: &Value) -> Value {
    let b = activity_body(v);
    ["created_on", "date", "updated_on"]
        .iter()
        .find_map(|k| b[*k].as_str())
        .map_or(Value::Null, Value::from)
}

fn activity_user(v: &Value) -> Value {
    let b = activity_body(v);
    for k in ["user", "author"] {
        let u = user_at(b, k);
        if !u.is_null() {
            return u;
        }
    }
    Value::Null
}

fn activity_text(v: &Value) -> Value {
    let b = activity_body(v);
    if let Some(raw) = b["content"]["raw"].as_str() {
        return raw.into();
    }
    match crate::bitbucket::activity_kind(v) {
        "update" => b["state"].clone(),
        _ => Value::Null,
    }
}

pub const ACTIVITY: Record = Record {
    name: "bitbucket.pr-activity",
    description: "an event on a pull request",
    primary: "date",
    fields: &[
        Field::new(
            "type",
            "string",
            "comment, approval, update, changes_requested, ...",
            "",
        )
        .source(Source::Fn(activity_type)),
        Field::new("date", "datetime", "when", "").source(Source::Fn(activity_date)),
        Field::new("user", "user", "who ({id, name})", "").source(Source::Fn(activity_user)),
        Field::new(
            "text",
            "string",
            "comment text, or the new state of an update",
            "",
        )
        .source(Source::Fn(activity_text)),
        Field::new("comment", "integer", "comment ID", "comment.id"),
        Field::new("detail", "object", "the REST event", "")
            .source(Source::Fn(activity_detail))
            .full(),
    ],
};

fn statuses(v: &Value) -> Value {
    v["statuses"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            json!({
                "key": s["key"], "name": s["name"], "state": s["state"],
                "url": s["url"], "updated": s["updated_on"],
            })
        })
        .collect()
}

pub const CHECKS: Record = Record {
    name: "bitbucket.pr-checks",
    description: "what gates a pull request's merge",
    primary: "pr",
    fields: &[
        Field::new("pr", "integer", "pull request number", "pr"),
        Field::new(
            "head",
            "string",
            "source head commit (for merge --if-head)",
            "head",
        ),
        Field::new("mergeable", "boolean", "no blocker found", "mergeable"),
        Field::new(
            "approvals",
            "integer",
            "approvals other than the author's",
            "approvals",
        ),
        Field::new(
            "blockers",
            "object[]",
            "{kind, message} per blocker",
            "blockers",
        ),
        Field::new(
            "statuses",
            "object[]",
            "build statuses: {key, name, state, url, updated}",
            "",
        )
        .source(Source::Fn(statuses)),
        Field::new(
            "unchecked",
            "object[]",
            "{kind, reason} for checks atlx could not make",
            "unchecked",
        ),
    ],
};

pub const REVIEW: Record = Record {
    name: "bitbucket.review",
    description: "your review state on a pull request",
    primary: "pr",
    fields: &[
        Field::new("pr", "integer", "pull request number", "pr"),
        Field::new("approved", "boolean", "approved", "approved"),
        Field::new(
            "state",
            "string",
            "approved, changes_requested or null",
            "state",
        ),
        Field::new("user", "user", "you ({id, name})", "").source(Source::Object(&[
            ("id", "user.uuid"),
            ("name", "user.display_name"),
        ])),
        Field::new("date", "datetime", "when", "participated_on"),
    ],
};

pub const REVIEWER: Record = Record {
    name: "bitbucket.reviewer",
    description: "a reviewer of a pull request",
    primary: "id",
    fields: &[
        Field::new("id", "string", "user UUID", "uuid"),
        Field::new("name", "string", "display name", "display_name"),
        Field::new("approved", "boolean", "approved", "approved"),
        Field::new(
            "state",
            "string",
            "approved, changes_requested or null",
            "state",
        ),
        Field::new("accountId", "string", "Atlassian account ID", "account_id").full(),
        Field::new("nickname", "string", "nickname", "nickname").full(),
    ],
};

// --- pipelines ---------------------------------------------------------------

fn result_of(v: &Value) -> Value {
    crate::bitbucket::state_of(v)
        .1
        .map_or(Value::Null, Value::from)
}

fn pipeline_url(v: &Value) -> Value {
    match (
        v["repository"]["full_name"].as_str(),
        v["build_number"].as_u64(),
    ) {
        (Some(repo), Some(n)) => {
            format!("https://bitbucket.org/{repo}/pipelines/results/{n}").into()
        }
        _ => Value::Null,
    }
}

fn trigger(v: &Value) -> Value {
    v["trigger"]["name"]
        .as_str()
        .map_or(Value::Null, |t| t.trim_end_matches("_TRIGGER").into())
}

fn variables(v: &Value) -> Value {
    v["variables"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| json!({"key": x["key"], "value": x.get("value"), "secured": x["secured"]}))
        .collect()
}

pub const PIPELINE: Record = Record {
    name: "bitbucket.pipeline",
    description: "a pipeline run",
    primary: "buildNumber",
    fields: &[
        Field::new(
            "buildNumber",
            "integer",
            "build number, what --pipeline takes",
            "build_number",
        ),
        Field::new(
            "state",
            "string",
            "PENDING, IN_PROGRESS or COMPLETED",
            "state.name",
        ),
        Field::new(
            "result",
            "string",
            "SUCCESSFUL, FAILED, ERROR, STOPPED, or the stage while running",
            "",
        )
        .source(Source::Fn(result_of)),
        Field::new("ref", "string", "branch or tag", "target.ref_name"),
        Field::new("commit", "string", "commit hash", "target.commit.hash"),
        Field::new(
            "pattern",
            "string",
            "custom pipeline name or selector pattern",
            "target.selector.pattern",
        ),
        Field::new(
            "trigger",
            "string",
            "PUSH, MANUAL, SCHEDULE, PULLREQUEST, ...",
            "",
        )
        .source(Source::Fn(trigger)),
        Field::new("creator", "user", "who started it ({id, name})", "").source(Source::Object(&[
            ("id", "creator.uuid"),
            ("name", "creator.display_name"),
        ])),
        Field::new("created", "datetime", "start time", "created_on"),
        Field::new("completed", "datetime", "end time", "completed_on"),
        Field::new("duration", "integer", "seconds", "duration_in_seconds"),
        Field::new("url", "string", "web URL", "").source(Source::Fn(pipeline_url)),
        Field::new("uuid", "string", "pipeline UUID", "uuid").full(),
        Field::new("refType", "string", "branch, tag, ...", "target.ref_type").full(),
        Field::new(
            "variables",
            "object[]",
            "variables (secured values left out)",
            "",
        )
        .source(Source::Fn(variables))
        .full(),
        Field::new(
            "rerunOf",
            "integer",
            "the run this one repeats (run --rerun)",
            "rerunOf",
        )
        .full(),
    ],
};

fn test_cases(v: &Value) -> Value {
    match v["test_cases"].as_array() {
        None => Value::Null,
        Some(cases) => cases
            .iter()
            .map(|t| {
                json!({
                    "name": t["fully_qualified_name"].as_str().or(t["name"].as_str()),
                    "status": t["status"], "duration": t["duration"],
                })
            })
            .collect(),
    }
}

pub const STEP: Record = Record {
    name: "bitbucket.pipeline-step",
    description: "a step of a pipeline run",
    primary: "uuid",
    fields: &[
        Field::new(
            "index",
            "integer",
            "1-based position, what --step takes",
            "index",
        ),
        Field::new("name", "string", "step name", "name"),
        Field::new(
            "state",
            "string",
            "PENDING, IN_PROGRESS or COMPLETED",
            "state.name",
        ),
        Field::new(
            "result",
            "string",
            "SUCCESSFUL, FAILED, ERROR, STOPPED, ...",
            "",
        )
        .source(Source::Fn(result_of)),
        Field::new("started", "datetime", "start time", "started_on"),
        Field::new("completed", "datetime", "end time", "completed_on"),
        Field::new("duration", "integer", "seconds", "duration_in_seconds"),
        Field::new("uuid", "string", "step UUID", "uuid"),
        Field::new(
            "testReport",
            "object",
            "test report summary (step get; null if none)",
            "test_report",
        ),
        Field::new(
            "testCases",
            "object[]",
            "{name, status, duration} (step get --test-cases)",
            "",
        )
        .source(Source::Fn(test_cases)),
        Field::new("image", "string", "build image", "image.name").full(),
    ],
};

/// Pipeline logs: the last 200 lines, continued with `--log-offset`.
pub const LOG_BUDGET: Budget = Budget::tail_lines(200, "log-offset");

pub const LOG: Record = Record {
    name: "bitbucket.pipeline-log",
    description: "a step's log, or the log lines that match --grep",
    primary: "stepUuid",
    fields: &[
        Field::new("pipeline", "integer", "build number", "pipeline"),
        Field::new("step", "string", "step name", "step"),
        Field::new("stepIndex", "integer", "1-based step position", "stepIndex"),
        Field::new("state", "string", "step state", "state"),
        Field::new("result", "string", "step result", "result"),
        Field::new("log", "string", "the log; the last lines when long", "log")
            .truncated(LOG_BUDGET),
        Field::new(
            "matches",
            "object[]",
            "with --grep: {line, text, before, after}",
            "matches",
        ),
        Field::new("stepUuid", "string", "step UUID", "stepUuid"),
        Field::new("pipelineUuid", "string", "pipeline UUID", "pipelineUuid").full(),
    ],
};

pub const PIPELINE_CONFIG: Record = Record {
    name: "bitbucket.pipeline-config",
    description: "a repository's Pipelines settings",
    primary: "repo",
    fields: &[
        Field::new("enabled", "boolean", "Pipelines runs", "enabled"),
        Field::new("repo", "string", "repository", "repository.full_name"),
    ],
};

// --- deployments -------------------------------------------------------------

fn deployment_state(v: &Value) -> Value {
    v["state"]["status"]["name"]
        .as_str()
        .or(v["state"]["name"].as_str())
        .map_or(Value::Null, Value::from)
}

pub const DEPLOYMENT: Record = Record {
    name: "bitbucket.deployment",
    description: "a deployment to an environment",
    primary: "uuid",
    fields: &[
        Field::new("uuid", "string", "deployment UUID", "uuid"),
        Field::new(
            "environment",
            "string",
            "environment UUID",
            "environment.uuid",
        ),
        Field::new(
            "state",
            "string",
            "SUCCESSFUL, FAILED, IN_PROGRESS, ...",
            "",
        )
        .source(Source::Fn(deployment_state)),
        Field::new("started", "datetime", "start time", "state.started_on"),
        Field::new("completed", "datetime", "end time", "state.completed_on"),
        Field::new("commit", "string", "commit deployed", "").source(Source::First(&[
            "release.commit.hash",
            "deployable.commit.hash",
        ])),
        Field::new("pipeline", "string", "pipeline UUID", "").source(Source::First(&[
            "release.pipeline.uuid",
            "deployable.pipeline.uuid",
        ])),
        Field::new("name", "string", "release name", "")
            .source(Source::First(&["release.name", "deployable.name"]))
            .full(),
        Field::new("url", "string", "release URL", "")
            .source(Source::First(&["release.url", "deployable.url"]))
            .full(),
    ],
};

pub const DEPLOYMENT_DIFF: Record = Record {
    name: "bitbucket.deployment-commits",
    description: "what a deployment changed since its environment's previous successful one",
    primary: "deployment",
    fields: &[
        Field::new("deployment", "string", "deployment UUID", "deployment"),
        Field::new("environment", "string", "environment UUID", "environment"),
        Field::new(
            "previous",
            "string",
            "previous successful deployment (null: none)",
            "previous",
        ),
        Field::new(
            "added",
            "object[]",
            "commits added: {hash, date, author, title}",
            "added",
        ),
        Field::new(
            "removed",
            "object[]",
            "commits rolled back: {hash, date, author, title}",
            "removed",
        ),
    ],
};

pub const ENVIRONMENT: Record = Record {
    name: "bitbucket.environment",
    description: "a deployment environment",
    primary: "uuid",
    fields: &[
        Field::new("uuid", "string", "environment UUID", "uuid"),
        Field::new("name", "string", "name, what --environment takes", "name"),
        Field::new("slug", "string", "slug", "slug"),
        Field::new(
            "type",
            "string",
            "Test, Staging or Production",
            "environment_type.name",
        ),
        Field::new("rank", "integer", "position", "rank").full(),
        Field::new("locked", "boolean", "locked", "lock.name").full(),
    ],
};

pub const VARIABLE: Record = Record {
    name: "bitbucket.variable",
    description: "a deployment variable (secured values are never shown)",
    primary: "key",
    fields: &[
        Field::new("key", "string", "name", "key"),
        Field::new("value", "string", "value (null when secured)", "value"),
        Field::new("secured", "boolean", "secured", "secured"),
        Field::new("uuid", "string", "variable UUID", "uuid").full(),
    ],
};
