//! Bitbucket Cloud REST 2.0, shaped after twg's `bitbucket repo|pull-requests|pipeline|search`.

use crate::client::{ApiError, Client};
use crate::markup::quote;
use crate::time;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Value, json};

pub mod deployments;
pub mod lifecycle;
pub mod pipelines;
pub mod repos;

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

/// Resolve twg-style `--workspace` / `--repo` (slug, `ws/slug`, or URL) to an API path.
pub fn repo_path(
    repo: Option<&str>,
    workspace: Option<&str>,
    default_ws: Option<&str>,
) -> Result<String> {
    let repo =
        repo.ok_or_else(|| anyhow!("pass --repo (slug, workspace/slug, or Bitbucket URL)"))?;
    let trimmed = repo
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("bitbucket.org/")
        .trim_end_matches('/');
    let mut parts = trimmed.split('/');
    let (ws, slug) = match (parts.next(), parts.next()) {
        (Some(ws), Some(slug)) if !slug.is_empty() => (ws, slug),
        (Some(slug), _) => (workspace_or(workspace, default_ws)?, slug),
        _ => bail!("invalid --repo '{repo}'"),
    };
    Ok(format!(
        "/repositories/{}/{}",
        enc(ws),
        enc(slug.trim_end_matches(".git"))
    ))
}

/// The workspace a `--repo` value points at: its own `workspace/` part or
/// URL, else `--workspace`, else the default workspace.
pub fn workspace_of(
    repo: &str,
    workspace: Option<&str>,
    default_ws: Option<&str>,
) -> Result<String> {
    let path = repo_path(Some(repo), workspace, default_ws)?;
    let ws = path
        .trim_start_matches("/repositories/")
        .split('/')
        .next()
        .unwrap_or_default();
    Ok(urlencoding::decode(ws)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| ws.to_string()))
}

pub fn workspace_or<'a>(ws: Option<&'a str>, default_ws: Option<&'a str>) -> Result<&'a str> {
    ws.or(default_ws).ok_or_else(|| {
        anyhow!("no Bitbucket workspace; pass --workspace or run `atlx setup bitbucket`")
    })
}

/// Collect up to `limit` items, following `next` links.
fn paginate(c: &Client, path: &str, query: &[(&str, &str)], limit: usize) -> Result<Vec<Value>> {
    Ok(paginate_more(c, path, query, limit)?.0)
}

/// [`paginate`], also saying whether more items exist past `limit`.
fn paginate_more(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    limit: usize,
) -> Result<(Vec<Value>, bool)> {
    let pagelen = limit.clamp(1, 50).to_string();
    let mut q = query.to_vec();
    q.push(("pagelen", &pagelen));
    let mut v = c.get(path, &q)?;
    let mut out = Vec::new();
    let more = loop {
        out.extend(v["values"].as_array().into_iter().flatten().cloned());
        match v["next"].as_str() {
            Some(next) if out.len() < limit => v = c.get(next, &[])?,
            Some(_) => break true,
            None => break false,
        }
    };
    let more = more || out.len() > limit;
    out.truncate(limit);
    Ok((out, more))
}

fn and(filters: Vec<String>) -> Option<String> {
    (!filters.is_empty()).then(|| filters.join(" AND "))
}

// --- repos -------------------------------------------------------------------

pub struct RepoQuery<'a> {
    pub query: Option<&'a str>,
    pub project: Option<&'a str>,
    pub language: Option<&'a str>,
    pub sort: &'a str,
    pub role: Option<&'a str>,
    pub limit: usize,
}

/// `bitbucket repo query`: REST repositories as a list, as twg prints them.
pub fn repo_query(c: &Client, ws: &str, o: &RepoQuery) -> Result<Value> {
    let mut f = Vec::new();
    if let Some(q) = o.query {
        f.push(format!("name ~ {}", quote(q)));
    }
    if let Some(p) = o.project {
        f.push(format!("project.key = {}", quote(p)));
    }
    if let Some(l) = o.language {
        f.push(format!("language = {}", quote(&l.to_lowercase())));
    }
    let filter = and(f);
    let mut q = vec![("sort", o.sort)];
    if let Some(f) = &filter {
        q.push(("q", f));
    }
    if let Some(r) = o.role {
        q.push(("role", r));
    }
    Ok(paginate(c, &format!("/repositories/{}", enc(ws)), &q, o.limit)?.into())
}

/// `bitbucket repo get`: the REST repository.
pub fn repo_get(c: &Client, path: &str) -> Result<Value> {
    c.get(path, &[])
}

pub enum RepoFile {
    Dir(Value),
    Raw(String),
}

fn repo_src_path(repo_path: &str, git_ref: &str, file_path: &str) -> String {
    let clean = file_path.trim().trim_start_matches('/');
    let clean = if clean == "." { "" } else { clean };
    let rel: Vec<String> = clean
        .split('/')
        .filter(|s| !s.is_empty())
        .map(enc)
        .collect();
    if rel.is_empty() {
        format!("{repo_path}/src/{}/", enc(git_ref))
    } else {
        format!("{repo_path}/src/{}/{}", enc(git_ref), rel.join("/"))
    }
}

/// `bitbucket repo file`
pub fn repo_file(
    c: &Client,
    path: &str,
    file_path: &str,
    ref_name: Option<&str>,
) -> Result<RepoFile> {
    let git_ref = match ref_name {
        Some(r) => r.to_string(),
        None => {
            let meta = c.get(path, &[])?;
            meta["mainbranch"]["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("HEAD")
                .to_string()
        }
    };
    let mut v = c.get(&repo_src_path(path, &git_ref, file_path), &[])?;
    if v["values"].is_array() {
        let mut items = Vec::new();
        loop {
            items.extend(v["values"].as_array().into_iter().flatten().cloned());
            match v["next"].as_str() {
                Some(next) if items.len() < 500 => v = c.get(next, &[])?,
                _ => break,
            }
        }
        items.truncate(500);
        return Ok(RepoFile::Dir(items.into()));
    }
    let text = match v {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(&other)?,
    };
    Ok(RepoFile::Raw(text))
}

/// Options for `bitbucket branch query`.
#[derive(Default)]
pub struct BranchQuery<'a> {
    /// Branch name substring.
    pub query: Option<&'a str>,
    /// Head commit author: display name (substring) or account ID.
    pub author: Option<&'a str>,
    /// Sort field, e.g. `name` or `-target.date`.
    pub sort: Option<&'a str>,
    pub limit: usize,
}

/// Most branches scanned when filtering by author, which Bitbucket can't do
/// server side.
const AUTHOR_SCAN: usize = 1000;

fn branch_by_author(b: &Value, who: &str) -> bool {
    let user = &b["target"]["author"]["user"];
    let who_l = who.to_lowercase();
    user["account_id"].as_str() == Some(who)
        || user["uuid"].as_str() == Some(who)
        || [
            user["display_name"].as_str(),
            user["nickname"].as_str(),
            b["target"]["author"]["raw"].as_str(),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.to_lowercase().contains(&who_l))
}

/// `bitbucket branch query` (and the old `repo branches`): REST branches as
/// a list.
pub fn branch_query(c: &Client, path: &str, o: &BranchQuery) -> Result<Value> {
    let filter = o.query.map(|q| format!("name ~ {}", quote(q)));
    let mut params = Vec::new();
    if let Some(f) = &filter {
        params.push(("q", f.as_str()));
    }
    if let Some(s) = o.sort {
        params.push(("sort", s));
    }
    let fetch = if o.author.is_some() {
        AUTHOR_SCAN.max(o.limit)
    } else {
        o.limit
    };
    let items = paginate(c, &format!("{path}/refs/branches"), &params, fetch)?;
    Ok(items
        .iter()
        .filter(|b| o.author.is_none_or(|a| branch_by_author(b, a)))
        .take(o.limit)
        .cloned()
        .collect())
}

// --- pull requests -----------------------------------------------------------

#[derive(Default)]
pub struct PrQuery<'a> {
    pub state: Option<&'a str>,
    pub author: Option<&'a str>,
    pub source: Option<&'a str>,
    pub dest: Option<&'a str>,
    pub text: Option<&'a str>,
    pub created_since: Option<&'a str>,
    pub created_before: Option<&'a str>,
    pub updated_since: Option<&'a str>,
    pub updated_before: Option<&'a str>,
    /// `{uuid}`, account ID, or exact nickname.
    pub reviewer: Option<&'a str>,
    /// One 1-based page instead of following pages up to `limit`.
    pub page: Option<usize>,
    pub limit: usize,
}

/// BBQL filter for `pull-requests query` (state is a separate query param).
fn pr_filter(o: &PrQuery) -> Result<Option<String>> {
    let mut f = Vec::new();
    if let Some(a) = o.author {
        f.push(if a.starts_with('{') {
            format!("author.uuid = {}", quote(a))
        } else if a.contains(':') {
            format!("author.account_id = {}", quote(a))
        } else {
            format!(
                "(author.nickname = {q} OR author.display_name ~ {q})",
                q = quote(a)
            )
        });
    }
    if let Some(r) = o.reviewer {
        let field = if r.starts_with('{') {
            "uuid"
        } else if r.contains(':') || (r.len() == 24 && r.bytes().all(|b| b.is_ascii_hexdigit())) {
            "account_id"
        } else {
            "nickname"
        };
        f.push(format!("reviewers.{field} = {}", quote(r)));
    }
    if let Some(s) = o.source {
        f.push(format!("source.branch.name = {}", quote(s)));
    }
    if let Some(d) = o.dest {
        f.push(format!("destination.branch.name = {}", quote(d)));
    }
    if let Some(t) = o.text {
        f.push(format!("(title ~ {q} OR description ~ {q})", q = quote(t)));
    }
    for (field, op, v) in [
        ("created_on", ">=", o.created_since),
        ("created_on", "<", o.created_before),
        ("updated_on", ">=", o.updated_since),
        ("updated_on", "<", o.updated_before),
    ] {
        if let Some(v) = v {
            f.push(format!("{field} {op} {}", time::to_date(v)?));
        }
    }
    Ok(and(f))
}

/// `bitbucket pull-requests query`: REST's page of pull requests with
/// twg's paging summary (`returned`, `hasNextPage`, `hasMore`).
pub fn pr_query(c: &Client, path: &str, o: &PrQuery) -> Result<Value> {
    let state = o.state.unwrap_or("OPEN").to_ascii_uppercase();
    let filter = pr_filter(o)?;
    let mut q = vec![("state", state.as_str()), ("sort", "-updated_on")];
    if let Some(f) = &filter {
        q.push(("q", f));
    }
    let url = format!("{path}/pullrequests");
    if let Some(page) = o.page {
        let size = o.limit.clamp(1, 50);
        let (page_s, size_s) = (page.to_string(), size.to_string());
        q.push(("page", &page_s));
        q.push(("pagelen", &size_s));
        let v = c.get(&url, &q)?;
        let items = v["values"].as_array().cloned().unwrap_or_default();
        let more = v["next"].is_string();
        return Ok(json!({
            "values": items,
            "page": page,
            "pagelen": size,
            "returned": items.len(),
            "hasNextPage": more,
            "hasMore": more,
        }));
    }
    let (items, more) = paginate_more(c, &url, &q, o.limit)?;
    Ok(json!({
        "values": items,
        "page": 1,
        "pagelen": o.limit.clamp(1, 50),
        "returned": items.len(),
        "hasNextPage": more,
        "hasMore": more,
    }))
}

pub struct PrGet {
    pub comments: bool,
    pub diff: bool,
    pub statuses: bool,
}

/// `bitbucket pull-requests get`: the REST pull request. `--comments`,
/// `--diff` and `--statuses` add `_comments`, `_diffstat` and `_statuses`
/// (REST lists, as twg names its extra sections).
pub fn pr_get(c: &Client, path: &str, id: u64, o: &PrGet) -> Result<Value> {
    let base = format!("{path}/pullrequests/{id}");
    let mut out = c.get(&base, &[])?;
    if o.comments {
        out["_comments"] = paginate(c, &format!("{base}/comments"), &[], 500)?
            .into_iter()
            .filter(|cm| cm["deleted"] != true)
            .collect();
    }
    if o.diff {
        out["_diffstat"] = paginate(c, &format!("{base}/diffstat"), &[], 500)?.into();
    }
    if o.statuses {
        out["_statuses"] = paginate(c, &format!("{base}/statuses"), &[], 100)?.into();
    }
    Ok(out)
}

pub struct NewPr<'a> {
    pub title: &'a str,
    pub source: &'a str,
    pub dest: Option<&'a str>,
    pub description: Option<&'a str>,
    pub reviewer_uuids: Vec<String>,
    pub reviewer_account_ids: Vec<String>,
    pub close_source_branch: bool,
    pub draft: bool,
}

/// `bitbucket pull-requests create`
pub fn pr_create(c: &Client, path: &str, n: NewPr) -> Result<Value> {
    let mut req = json!({
        "title": n.title,
        "source": {"branch": {"name": n.source}},
        "close_source_branch": n.close_source_branch,
        "draft": n.draft,
    });
    if let Some(d) = n.dest {
        req["destination"] = json!({"branch": {"name": d}});
    }
    if let Some(d) = n.description {
        req["description"] = d.into();
    }
    let mut reviewers: Vec<Value> = n
        .reviewer_uuids
        .iter()
        .map(|u| json!({"uuid": u}))
        .collect();
    reviewers.extend(
        n.reviewer_account_ids
            .iter()
            .map(|a| json!({"account_id": a})),
    );
    if !reviewers.is_empty() {
        req["reviewers"] = reviewers.into();
    }
    c.post(&format!("{path}/pullrequests"), &req)
}

/// A PR number or a Bitbucket pull-request URL.
pub fn pr_number(s: &str) -> Result<u64> {
    let s = s.trim().trim_end_matches('/');
    let tail = match s.find("/pull-requests/") {
        Some(i) => s[i + 15..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default(),
        None => s,
    };
    tail.parse()
        .map_err(|_| anyhow!("'{s}' is not a pull request number or URL"))
}

/// Where an inline comment sits in the diff. `line`/`from_line` pick one
/// line on the new or old side; the `start_*`/`end_*` pairs pick a range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anchor {
    pub line: Option<u64>,
    pub from_line: Option<u64>,
    pub start_line: Option<u64>,
    pub end_line: Option<u64>,
    pub start_from_line: Option<u64>,
    pub end_from_line: Option<u64>,
}

impl Anchor {
    fn is_empty(&self) -> bool {
        *self == Anchor::default()
    }
}

fn usage(msg: impl Into<String>) -> anyhow::Error {
    ApiError::usage(msg).into()
}

/// One end of a range: both or neither of `start`/`end`, with start <= end.
fn range(start: Option<u64>, end: Option<u64>, names: (&str, &str)) -> Result<Option<(u64, u64)>> {
    match (start, end) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(usage(format!("{} needs {} as well", names.0, names.1))),
        (None, Some(_)) => Err(usage(format!("{} needs {} as well", names.1, names.0))),
        (Some(s), Some(e)) if s > e => Err(usage(format!(
            "{} ({s}) is after {} ({e})",
            names.0, names.1
        ))),
        (Some(s), Some(e)) => Ok(Some((s, e))),
    }
}

/// Bitbucket's `inline` object for a comment, or `None` for a general one.
///
/// New-side lines go in `to` (`start_to` for a range start), old-side lines
/// in `from` (`start_from`). A single line can't be mixed with a range, and
/// every line flag needs `--path`.
pub fn inline_anchor(path: Option<&str>, a: &Anchor) -> Result<Option<Value>> {
    for (v, name) in [
        (a.line, "--line"),
        (a.from_line, "--from-line"),
        (a.start_line, "--start-line"),
        (a.end_line, "--end-line"),
        (a.start_from_line, "--start-from-line"),
        (a.end_from_line, "--end-from-line"),
    ] {
        if v == Some(0) {
            return Err(usage(format!("{name} must be 1 or more")));
        }
    }
    let to_range = range(a.start_line, a.end_line, ("--start-line", "--end-line"))?;
    let from_range = range(
        a.start_from_line,
        a.end_from_line,
        ("--start-from-line", "--end-from-line"),
    )?;
    let single = a.line.is_some() || a.from_line.is_some();
    if single && (to_range.is_some() || from_range.is_some()) {
        return Err(usage(
            "use either --line/--from-line or a --start-*/--end-* range, not both",
        ));
    }
    let Some(file) = path else {
        if a.is_empty() {
            return Ok(None);
        }
        return Err(usage("inline line and range flags need --path"));
    };
    let mut inline = json!({"path": file});
    if let Some(l) = a.line {
        inline["to"] = l.into();
    }
    if let Some(l) = a.from_line {
        inline["from"] = l.into();
    }
    if let Some((s, e)) = to_range {
        inline["start_to"] = s.into();
        inline["to"] = e.into();
    }
    if let Some((s, e)) = from_range {
        inline["start_from"] = s.into();
        inline["from"] = e.into();
    }
    Ok(Some(inline))
}

#[derive(Default)]
pub struct NewComment<'a> {
    pub text: &'a str,
    pub reply_to: Option<u64>,
    /// Save as unpublished review feedback.
    pub pending: bool,
    pub path: Option<&'a str>,
    pub anchor: Anchor,
}

/// The request body for `pull-requests comment create`.
pub fn comment_body(n: &NewComment) -> Result<Value> {
    let mut req = json!({"content": {"raw": n.text}});
    if let Some(p) = n.reply_to {
        req["parent"] = json!({"id": p});
    }
    if n.pending {
        req["pending"] = true.into();
    }
    if let Some(inline) = inline_anchor(n.path, &n.anchor)? {
        req["inline"] = inline;
    }
    Ok(req)
}

/// `bitbucket pull-requests comment create`: the REST comment.
pub fn pr_comment(c: &Client, path: &str, id: u64, n: NewComment) -> Result<Value> {
    let req = comment_body(&n)?;
    c.post(&format!("{path}/pullrequests/{id}/comments"), &req)
}

fn comment_path(path: &str, id: u64, cid: u64) -> String {
    format!("{path}/pullrequests/{id}/comments/{cid}")
}

/// `bitbucket pull-requests comment query`: REST comments as a list, in
/// creation order, without deleted ones (as `pull-requests get --comments`).
pub fn pr_comment_query(c: &Client, path: &str, id: u64, limit: usize) -> Result<Value> {
    let items = paginate(
        c,
        &format!("{path}/pullrequests/{id}/comments"),
        &[("sort", "created_on")],
        limit,
    )?;
    Ok(items
        .into_iter()
        .filter(|cm| cm["deleted"] != true)
        .collect())
}

/// `bitbucket pull-requests comment get`: the REST comment.
pub fn pr_comment_get(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.get(&comment_path(path, id, cid), &[])
}

/// `bitbucket pull-requests comment update`: replaces the comment text.
pub fn pr_comment_update(c: &Client, path: &str, id: u64, cid: u64, text: &str) -> Result<Value> {
    c.put(
        &comment_path(path, id, cid),
        &json!({"content": {"raw": text}}),
    )
}

/// `bitbucket pull-requests comment delete`. Bitbucket answers 204, so the
/// result names what was removed.
pub fn pr_comment_delete(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.delete(&comment_path(path, id, cid), &[])?;
    Ok(json!({"id": cid, "pullRequest": id, "deleted": true}))
}

/// `bitbucket pull-requests comment resolve`: the REST resolution object.
pub fn pr_comment_resolve(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.request(
        "POST",
        &format!("{}/resolve", comment_path(path, id, cid)),
        &[],
        None,
    )
}

/// `bitbucket pull-requests comment reopen`. Bitbucket answers 204.
pub fn pr_comment_reopen(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.delete(&format!("{}/resolve", comment_path(path, id, cid)), &[])?;
    Ok(json!({"id": cid, "pullRequest": id, "resolved": false}))
}

fn task_path(path: &str, id: u64, tid: u64) -> String {
    format!("{path}/pullrequests/{id}/tasks/{tid}")
}

/// `bitbucket pull-requests task create`: the REST task.
pub fn pr_task_create(
    c: &Client,
    path: &str,
    id: u64,
    text: &str,
    comment: Option<u64>,
) -> Result<Value> {
    let mut req = json!({"content": {"raw": text}});
    if let Some(cid) = comment {
        req["comment"] = json!({"id": cid});
    }
    c.post(&format!("{path}/pullrequests/{id}/tasks"), &req)
}

/// `bitbucket pull-requests task get`: the REST task.
pub fn pr_task_get(c: &Client, path: &str, id: u64, tid: u64) -> Result<Value> {
    c.get(&task_path(path, id, tid), &[])
}

/// Changes for `bitbucket pull-requests task update|resolve|reopen`.
#[derive(Default)]
pub struct TaskChange<'a> {
    pub text: Option<&'a str>,
    /// `Some(true)` resolves, `Some(false)` reopens.
    pub resolved: Option<bool>,
}

/// The PUT body for a task change.
pub fn task_body(t: &TaskChange) -> Result<Value> {
    let mut req = json!({});
    if let Some(text) = t.text {
        req["content"] = json!({"raw": text});
    }
    if let Some(r) = t.resolved {
        req["state"] = if r { "RESOLVED" } else { "UNRESOLVED" }.into();
    }
    if req.as_object().is_some_and(|o| o.is_empty()) {
        return Err(usage("pass --text, --resolve or --reopen"));
    }
    Ok(req)
}

/// `bitbucket pull-requests task update|resolve|reopen`: the REST task.
pub fn pr_task_update(c: &Client, path: &str, id: u64, tid: u64, t: &TaskChange) -> Result<Value> {
    let req = task_body(t)?;
    c.put(&task_path(path, id, tid), &req)
}

/// `bitbucket pull-requests task delete`. Bitbucket answers 204.
pub fn pr_task_delete(c: &Client, path: &str, id: u64, tid: u64) -> Result<Value> {
    c.delete(&task_path(path, id, tid), &[])?;
    Ok(json!({"id": tid, "pullRequest": id, "deleted": true}))
}

/// `bitbucket pull-requests diffstat`: REST diffstat entries as a list.
pub fn pr_diffstat(c: &Client, path: &str, id: u64, limit: usize) -> Result<Value> {
    Ok(paginate(c, &format!("{path}/pullrequests/{id}/diffstat"), &[], limit)?.into())
}

/// `bitbucket pull-requests commits`: REST commits as a list.
pub fn pr_commits(c: &Client, path: &str, id: u64, limit: usize) -> Result<Value> {
    Ok(paginate(c, &format!("{path}/pullrequests/{id}/commits"), &[], limit)?.into())
}

fn raw_text(c: &Client, path: &str) -> Result<String> {
    match c.get(path, &[])? {
        Value::String(s) => Ok(s),
        Value::Null => Ok(String::new()),
        other => Ok(other.to_string()),
    }
}

/// `bitbucket pull-requests diff`
pub fn pr_diff(c: &Client, path: &str, id: u64) -> Result<String> {
    raw_text(c, &format!("{path}/pullrequests/{id}/diff"))
}

/// `bitbucket pull-requests patch`: the mbox-style patch as text.
pub fn pr_patch(c: &Client, path: &str, id: u64) -> Result<String> {
    raw_text(c, &format!("{path}/pullrequests/{id}/patch"))
}

/// Parse `@@ -a,b +c,d @@` into (old start, old count, new start, new count).
fn hunk_header(line: &str) -> Option<(u64, u64, u64, u64)> {
    let mut parts = line.strip_prefix("@@ ")?.split(' ');
    let side = |s: &str, sign: char| -> Option<(u64, u64)> {
        let s = s.strip_prefix(sign)?;
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (os, oc) = side(parts.next()?, '-')?;
    let (ns, nc) = side(parts.next()?, '+')?;
    Some((os, oc, ns, nc))
}

/// Strip git's `a/`/`b/` prefix; `/dev/null` is no file.
fn diff_file(s: &str) -> Option<String> {
    let s = s.split('\t').next().unwrap_or(s).trim_end();
    if s == "/dev/null" {
        return None;
    }
    Some(
        s.strip_prefix("a/")
            .or_else(|| s.strip_prefix("b/"))
            .unwrap_or(s)
            .to_string(),
    )
}

/// Lines of a unified diff whose text contains `needle`, optionally in one
/// file. Each match says which comment flag anchors it: added and context
/// lines have a new-side `line`, removed and context lines an old-side
/// `fromLine`.
pub fn diff_lines(diff: &str, needle: &str, only_path: Option<&str>) -> Vec<Value> {
    let only_path = only_path.map(|p| p.trim_start_matches("./"));
    let mut out = Vec::new();
    let (mut old_path, mut new_path) = (None::<String>, None::<String>);
    let (mut old, mut new, mut old_left, mut new_left) = (0u64, 0u64, 0u64, 0u64);
    let mut hunk = String::new();
    for raw in diff.lines() {
        if old_left == 0 && new_left == 0 {
            if raw.starts_with("diff --git ") {
                old_path = None;
                new_path = None;
            } else if let Some(p) = raw.strip_prefix("--- ") {
                old_path = diff_file(p);
            } else if let Some(p) = raw.strip_prefix("+++ ") {
                new_path = diff_file(p);
            } else if let Some((os, oc, ns, nc)) = hunk_header(raw) {
                (old, old_left, new, new_left) = (os, oc, ns, nc);
                hunk = raw.to_string();
            }
            continue;
        }
        let (kind, from_line, line, text) = if let Some(t) = raw.strip_prefix('+') {
            new_left = new_left.saturating_sub(1);
            new += 1;
            ("added", None, Some(new - 1), t)
        } else if let Some(t) = raw.strip_prefix('-') {
            old_left = old_left.saturating_sub(1);
            old += 1;
            ("removed", Some(old - 1), None, t)
        } else if raw.starts_with('\\') {
            continue;
        } else {
            let t = raw.strip_prefix(' ').unwrap_or(raw);
            old_left = old_left.saturating_sub(1);
            new_left = new_left.saturating_sub(1);
            old += 1;
            new += 1;
            ("context", Some(old - 1), Some(new - 1), t)
        };
        let file = new_path.as_deref().or(old_path.as_deref()).unwrap_or("");
        let in_scope = only_path
            .is_none_or(|p| new_path.as_deref() == Some(p) || old_path.as_deref() == Some(p));
        if in_scope && text.contains(needle) {
            let mut m = json!({
                "path": file,
                "type": kind,
                "line": line,
                "fromLine": from_line,
                "text": text,
                "hunk": hunk,
            });
            if old_path.is_some() && old_path != new_path {
                m["oldPath"] = old_path.clone().into();
            }
            out.push(m);
        }
    }
    out
}

/// `bitbucket pull-requests diff-line`: matching diff lines with the line
/// numbers an inline comment needs.
pub fn pr_diff_line(
    c: &Client,
    path: &str,
    id: u64,
    needle: &str,
    only_path: Option<&str>,
) -> Result<Value> {
    if needle.is_empty() {
        return Err(usage("--text must not be empty"));
    }
    let diff = pr_diff(c, path, id)?;
    Ok(diff_lines(&diff, needle, only_path).into())
}

fn activity_event(item: &Value) -> (&str, &Value) {
    for kind in ["comment", "approval", "update", "changes_requested"] {
        if item[kind].is_object() {
            return (kind, &item[kind]);
        }
    }
    if let Some(obj) = item.as_object() {
        for (k, v) in obj {
            if k != "pull_request" && v.is_object() {
                return (k.as_str(), v);
            }
        }
    }
    ("unknown", item)
}

/// `bitbucket pull-requests activity`: REST activity entries as a list.
pub fn pr_activity(
    c: &Client,
    path: &str,
    id: u64,
    event_type: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let fetch = if event_type.is_some() {
        limit.max(100)
    } else {
        limit
    };
    let items = paginate(c, &format!("{path}/pullrequests/{id}/activity"), &[], fetch)?;
    Ok(items
        .iter()
        .filter(|item| {
            let (kind, _) = activity_event(item);
            event_type.is_none_or(|want| want.eq_ignore_ascii_case(kind))
        })
        .take(limit)
        .cloned()
        .collect())
}

/// `bitbucket pull-requests task query` (and the old `tasks`)
pub fn pr_tasks(c: &Client, path: &str, id: u64, limit: usize) -> Result<Value> {
    Ok(paginate(c, &format!("{path}/pullrequests/{id}/tasks"), &[], limit)?.into())
}

/// `bitbucket pull-requests approve`
pub fn pr_approve(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/approve"),
        &[],
        None,
    )
}

/// `bitbucket pull-requests unapprove`
pub fn pr_unapprove(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.delete(&format!("{path}/pullrequests/{id}/approve"), &[])?;
    Ok(json!({"id": id, "approved": false}))
}

/// `bitbucket pull-requests request-changes`
pub fn pr_request_changes(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/request-changes"),
        &[],
        None,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum MergeStrategy {
    #[value(name = "merge_commit")]
    MergeCommit,
    #[value(name = "squash")]
    Squash,
    #[value(name = "fast_forward")]
    FastForward,
}

impl MergeStrategy {
    fn as_api(self) -> &'static str {
        match self {
            MergeStrategy::MergeCommit => "merge_commit",
            MergeStrategy::Squash => "squash",
            MergeStrategy::FastForward => "fast_forward",
        }
    }
}

#[derive(Default)]
pub struct PrMerge<'a> {
    pub strategy: Option<MergeStrategy>,
    pub close_source_branch: bool,
    /// Keep the source branch even if the pull request asks to close it.
    pub keep_source_branch: bool,
    pub message: Option<&'a str>,
}

fn pr_merge_body(o: &PrMerge) -> Value {
    let mut req = json!({});
    if let Some(s) = o.strategy {
        req["merge_strategy"] = s.as_api().into();
    }
    if o.close_source_branch {
        req["close_source_branch"] = true.into();
    } else if o.keep_source_branch {
        req["close_source_branch"] = false.into();
    }
    if let Some(m) = o.message {
        req["message"] = m.into();
    }
    req
}

/// `bitbucket pull-requests merge`
pub fn pr_merge(c: &Client, path: &str, id: u64, o: &PrMerge) -> Result<Value> {
    lifecycle::merge(c, path, id, o, None)
}

/// `bitbucket pull-requests decline`
pub fn pr_decline(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/decline"),
        &[],
        None,
    )
}

// --- search ------------------------------------------------------------------

fn repo_slug(repo: &str) -> &str {
    let repo = repo.trim().trim_end_matches('/');
    let last = repo.rsplit('/').next().unwrap_or(repo);
    last.trim_end_matches(".git")
}

/// The search term and the filters. The modifiers the retired endpoint read
/// from the query (`repo:`, `project:`, `path:`, `ext:`, `lang:`) become
/// filters here; text in quotes is never read as one.
fn code_search_query(query: &str, repo: Option<&str>) -> (String, Option<Value>) {
    let mut filters = Vec::new();
    if let Some(r) = repo.map(repo_slug).filter(|r| !r.is_empty()) {
        filters.push(json!({"attribute": "repository", "eq": r}));
    }
    let mut words = Vec::new();
    let mut quoted = false;
    let mut found = false;
    for word in query.split(' ') {
        let modifier = word.split_once(':').and_then(|(key, value)| {
            let attribute = match key {
                "repo" => "repository",
                "project" => "project",
                "path" => "path",
                "ext" => "extension",
                "lang" => "language",
                _ => return None,
            };
            (!quoted && !value.is_empty() && !value.contains('"')).then_some((attribute, value))
        });
        match modifier {
            Some((attribute, value)) => {
                filters.push(json!({"attribute": attribute, "eq": value}));
                found = true;
            }
            None => words.push(word),
        }
        if word.matches('"').count() % 2 == 1 {
            quoted = !quoted;
        }
    }
    let term = if found {
        words.join(" ").trim().to_string()
    } else {
        query.trim().to_string()
    };
    let filters = match filters.len() {
        0 => None,
        1 => filters.pop(),
        _ => Some(json!({"and": filters})),
    };
    (term, filters)
}

/// A matched file, with the `file.path` and `content_match_count` keys of
/// the retired endpoint.
fn code_hit(f: &Value) -> Value {
    json!({
        "file": {"path": f["path"], "commit": {"hash": f["version"]}},
        "content_match_count": f["matched_chunks"].as_array().map_or(0, Vec::len),
        "matched_chunks": f["matched_chunks"],
        "branch": f["branch_or_tag_name"],
        "language": f["language"],
        "url": f["url"],
    })
}

/// `bitbucket search code`: `POST /workspaces/{ws}/codesearch`.
pub fn search_code(
    c: &Client,
    ws: &str,
    query: &str,
    repo: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let (term, filters) = code_search_query(query, repo);
    if term.is_empty() || term.chars().count() > 1024 {
        return Err(ApiError::usage("search needs 1 to 1024 characters of text").into());
    }
    let mut search = json!({"search_term": term});
    if let Some(f) = filters {
        search["filters"] = f;
    }
    let path = format!("/workspaces/{}/codesearch", enc(ws));
    let mut hits = Vec::new();
    let mut next: Option<(Value, Value)> = None;
    while hits.len() < limit {
        let mut body = json!({
            "query": search,
            "config": {"max_results": (limit - hits.len()).min(100)},
        });
        if let Some((id, cursor)) = &next {
            body["request_id"] = id.clone();
            body["pagination"] = json!({"next_cursor": cursor});
        }
        let v = c.post(&path, &body)?;
        let files = v["matched_files"]
            .as_array()
            .ok_or_else(|| anyhow!("Bitbucket returned no matched_files list"))?;
        hits.extend(files.iter().take(limit - hits.len()).map(code_hit));
        let page = &v["pagination"];
        if page["has_more"] != true {
            break;
        }
        let following = (v["id"].clone(), page["next_cursor"].clone());
        if following.0.is_null() || following.1.is_null() || next.as_ref() == Some(&following) {
            bail!("Bitbucket said there are more results but gave no cursor for them");
        }
        next = Some(following);
    }
    Ok(hits.into())
}

// --- pipelines ---------------------------------------------------------------

fn state_of(p: &Value) -> (String, Option<String>) {
    let state = p["state"]["name"].as_str().unwrap_or_default().to_string();
    let result = p["state"]["result"]["name"]
        .as_str()
        .or(p["state"]["stage"]["name"].as_str())
        .map(String::from);
    (state, result)
}

/// `workspace/slug` from a `/repositories/workspace/slug` path.
fn full_name(path: &str) -> String {
    path.trim_start_matches("/repositories/").to_string()
}

#[derive(Default)]
pub struct PipelineQuery<'a> {
    pub state: Option<&'a str>,
    pub status: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub trigger: Option<&'a str>,
    /// The target selector's pattern, e.g. a custom pipeline's name.
    pub pattern: Option<&'a str>,
    pub limit: usize,
}

/// `bitbucket pipeline query`: REST pipelines as a list, each with `_repo`
/// (`workspace/slug`) as twg adds. State/result/trigger are filtered
/// client-side over the most recent runs.
pub fn pipeline_query(c: &Client, path: &str, o: &PipelineQuery) -> Result<Value> {
    let mut q = vec![("sort", "-created_on")];
    if let Some(b) = o.branch {
        q.push(("target.branch", b));
    }
    let filtering =
        o.state.is_some() || o.status.is_some() || o.trigger.is_some() || o.pattern.is_some();
    let fetch = if filtering { o.limit.max(100) } else { o.limit };
    let eq = |a: Option<&str>, b: Option<&str>| match (a, b) {
        (None, _) => true,
        (Some(want), Some(got)) => want.eq_ignore_ascii_case(got),
        (Some(_), None) => false,
    };
    Ok(paginate(c, &format!("{path}/pipelines"), &q, fetch)?
        .iter()
        .filter(|p| {
            let (state, result) = state_of(p);
            eq(o.state, Some(&state))
                // twg says PASSED where a pipeline's result says SUCCESSFUL.
                && (eq(o.status, result.as_deref())
                    || (o.status.is_some_and(|s| s.eq_ignore_ascii_case("PASSED"))
                        && result.as_deref() == Some("SUCCESSFUL")))
                && eq(o.pattern, p["target"]["selector"]["pattern"].as_str())
                && eq(
                    o.trigger,
                    p["trigger"]["name"]
                        .as_str()
                        .map(|t| t.trim_end_matches("_TRIGGER")),
                )
        })
        .take(o.limit)
        .map(|p| {
            let mut p = p.clone();
            p["_repo"] = full_name(path).into();
            p
        })
        .collect())
}

/// Build number, `{uuid}`, or a pipeline result URL -> the id the API accepts.
pub fn pipeline_ref(s: &str) -> String {
    let s = s.trim().trim_end_matches('/');
    match s.find("/results/") {
        Some(i) => s[i + 9..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_string(),
        None => s.to_string(),
    }
}

pub struct PipelineGet<'a> {
    pub logs: bool,
    pub failed_steps: bool,
    pub lines: usize,
    pub step: Option<&'a str>,
}

/// `bitbucket pipeline get`: the REST pipeline with its REST `steps`; with
/// `--logs`, each selected step gets `log` (the last `lines` lines),
/// `log_total_lines` and `log_truncated`, or `log_error`.
pub fn pipeline_get(c: &Client, path: &str, pipeline: &str, o: &PipelineGet) -> Result<Value> {
    let p = c.get(
        &format!("{path}/pipelines/{}", enc(&pipeline_ref(pipeline))),
        &[],
    )?;
    let uuid = p["uuid"]
        .as_str()
        .ok_or_else(|| anyhow!("pipeline has no uuid"))?;
    let mut out = p.clone();
    let steps = paginate(
        c,
        &format!("{path}/pipelines/{}/steps", enc(uuid)),
        &[],
        100,
    )?;
    let selected: Vec<usize> = pipelines::select(&steps, o.step)?
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    let mut rendered = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        let (_, result) = state_of(s);
        let mut step = s.clone();
        let failed = matches!(result.as_deref(), Some("FAILED" | "ERROR"));
        let wanted = selected.contains(&i);
        if o.logs && wanted && (!o.failed_steps || failed) {
            let log_path = format!(
                "{path}/pipelines/{}/steps/{}/log",
                enc(uuid),
                enc(s["uuid"].as_str().unwrap_or(""))
            );
            match c.get_bytes(&log_path, &[]) {
                Ok(bytes) => {
                    // Not parsed: a log can happen to be valid JSON.
                    let text = String::from_utf8_lossy(&bytes.data);
                    let total = text.lines().count();
                    step["log"] = tail(&text, o.lines).into();
                    step["log_total_lines"] = total.into();
                    step["log_truncated"] = (o.lines > 0 && total > o.lines).into();
                }
                Err(e) => step["log_error"] = format!("{e:#}").into(),
            }
        }
        rendered.push(step);
    }
    out["steps"] = rendered.into();
    Ok(out)
}

fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = if n == 0 {
        0
    } else {
        lines.len().saturating_sub(n)
    };
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_paths() {
        assert_eq!(
            repo_path(Some("acme/api"), None, None).unwrap(),
            "/repositories/acme/api"
        );
        assert_eq!(
            repo_path(Some("api"), None, Some("acme")).unwrap(),
            "/repositories/acme/api"
        );
        assert_eq!(
            repo_path(Some("api"), Some("other"), Some("acme")).unwrap(),
            "/repositories/other/api"
        );
        assert_eq!(
            repo_path(
                Some("https://bitbucket.org/acme/api/pull-requests/3"),
                None,
                None
            )
            .unwrap(),
            "/repositories/acme/api"
        );
        assert!(repo_path(Some("api"), None, None).is_err());
        assert!(repo_path(None, Some("acme"), None).is_err());
    }

    #[test]
    fn refs() {
        assert_eq!(pr_number("42").unwrap(), 42);
        assert_eq!(
            pr_number("https://bitbucket.org/acme/api/pull-requests/930/diff").unwrap(),
            930
        );
        assert!(pr_number("abc").is_err());
        assert_eq!(
            pipeline_ref("https://bitbucket.org/acme/api/pipelines/results/1234"),
            "1234"
        );
        assert_eq!(pipeline_ref("{abc-def}"), "{abc-def}");
        assert_eq!(tail("a\nb\nc", 2), "b\nc");
        assert_eq!(tail("a\nb\nc", 0), "a\nb\nc");
    }

    #[test]
    fn pr_filters() {
        let f = pr_filter(&PrQuery {
            author: Some("jane"),
            source: Some("feat/x"),
            created_since: Some("2026-01-01"),
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            f,
            r#"(author.nickname = "jane" OR author.display_name ~ "jane") AND source.branch.name = "feat/x" AND created_on >= 2026-01-01"#
        );
        assert_eq!(pr_filter(&PrQuery::default()).unwrap(), None);
    }

    #[test]
    fn src_paths_and_summaries() {
        assert_eq!(
            repo_src_path("/repositories/acme/api", "main", "src/lib.rs"),
            "/repositories/acme/api/src/main/src/lib.rs"
        );
        assert_eq!(
            repo_src_path("/repositories/acme/api", "feat/x", "/README.md"),
            "/repositories/acme/api/src/feat%2Fx/README.md"
        );
        assert_eq!(
            repo_src_path("/repositories/acme/api", "HEAD", "."),
            "/repositories/acme/api/src/HEAD/"
        );
    }

    #[test]
    fn merge_body_and_code_search_helpers() {
        let body = pr_merge_body(&PrMerge {
            strategy: Some(MergeStrategy::Squash),
            close_source_branch: true,
            message: Some("Squash merge"),
            ..Default::default()
        });
        assert_eq!(
            body,
            json!({"merge_strategy": "squash", "close_source_branch": true, "message": "Squash merge"})
        );
        assert_eq!(pr_merge_body(&PrMerge::default()), json!({}));
        let keep = pr_merge_body(&PrMerge {
            keep_source_branch: true,
            ..Default::default()
        });
        assert_eq!(keep, json!({"close_source_branch": false}));

        let by = |name: &str, id: &str| json!({"target": {"author": {"raw": format!("{name} <x@y>"), "user": {"display_name": name, "account_id": id}}}});
        assert!(branch_by_author(&by("Alice Example", "557:a"), "alice"));
        assert!(branch_by_author(&by("Alice Example", "557:a"), "557:a"));
        assert!(!branch_by_author(&by("Bob", "557:b"), "alice"));

        let only = |a: &str, v: &str| Some(json!({"attribute": a, "eq": v}));
        assert_eq!(
            code_search_query("foo  bar", None),
            ("foo  bar".into(), None)
        );
        assert_eq!(
            code_search_query("foo", Some("acme/api.git")),
            ("foo".into(), only("repository", "api"))
        );
        assert_eq!(
            code_search_query("foo ext:rs", None),
            ("foo".into(), only("extension", "rs"))
        );
        let quoted = r#""see path:x here""#;
        assert_eq!(code_search_query(quoted, None), (quoted.into(), None));
        assert_eq!(
            code_search_query("foo lang:go", Some("api")).1.unwrap()["and"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    fn mock_server(
        routes: Vec<(&'static str, &'static str, &'static str, u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for (want_method, want_path, want_body_substr, status, resp_body) in routes {
                let (mut stream, _) = listener.accept().unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let header_end = loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0, "unexpected EOF reading headers");
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&raw[..header_end]);
                let content_len = headers
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                while raw.len() < header_end + content_len {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0, "unexpected EOF reading body");
                    raw.extend_from_slice(&buf[..n]);
                }
                let req = String::from_utf8_lossy(&raw);
                let first_line = req.lines().next().unwrap_or_default();
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or_default();
                let path = parts.next().unwrap_or_default();
                assert_eq!(
                    method, want_method,
                    "unexpected method in request: {first_line}"
                );
                assert_eq!(path, want_path, "unexpected path in request: {first_line}");
                if !want_body_substr.is_empty() {
                    let body = &req[header_end..];
                    assert!(
                        body.contains(want_body_substr),
                        "expected body containing {want_body_substr:?} in {body}"
                    );
                }
                let reason = match status {
                    200 => "OK",
                    204 => "No Content",
                    _ => "Error",
                };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                    resp_body.len()
                );
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn pr_endpoints_roundtrip() {
        let (base, h) = mock_server(vec![
            (
                "GET",
                "/repositories/acme/api/pullrequests/5/diff",
                "",
                200,
                "diff --git a/a b/a\n+hi\n",
            ),
            (
                "GET",
                "/repositories/acme/api/pullrequests/5/activity?pagelen=50",
                "",
                200,
                r#"{"values":[{"approval":{"date":"2026-01-01","user":{"display_name":"Bob"}}},{"comment":{"id":1,"created_on":"2026-01-02","user":{"display_name":"Alice"},"content":{"raw":"hi"}}}]}"#,
            ),
            (
                "GET",
                "/repositories/acme/api/pullrequests/5/tasks?pagelen=50",
                "",
                200,
                r#"{"values":[{"id":3,"state":"UNRESOLVED","content":{"raw":"Fix it"},"creator":{"display_name":"Alice"}}]}"#,
            ),
            (
                "POST",
                "/repositories/acme/api/pullrequests/5/approve",
                "",
                200,
                r#"{"approved":true,"state":"approved","user":{"display_name":"Bob"}}"#,
            ),
            (
                "DELETE",
                "/repositories/acme/api/pullrequests/5/approve",
                "",
                204,
                "",
            ),
            (
                "POST",
                "/repositories/acme/api/pullrequests/5/request-changes",
                "",
                200,
                r#"{"approved":false,"state":"changes_requested","user":{"display_name":"Bob"}}"#,
            ),
            (
                "POST",
                "/repositories/acme/api/pullrequests/5/merge",
                r#""merge_strategy":"fast_forward""#,
                200,
                r#"{"id":5,"title":"T","state":"MERGED"}"#,
            ),
            (
                "POST",
                "/repositories/acme/api/pullrequests/5/decline",
                "",
                200,
                r#"{"id":5,"title":"T","state":"DECLINED"}"#,
            ),
        ]);
        let c = Client::new(base, "u@e.com", "tok", Some(5000));
        let p = "/repositories/acme/api";

        assert_eq!(pr_diff(&c, p, 5).unwrap(), "diff --git a/a b/a\n+hi\n");
        let act = pr_activity(&c, p, 5, Some("comment"), 10).unwrap();
        assert_eq!(act.as_array().unwrap().len(), 1);
        assert_eq!(act[0]["comment"]["content"]["raw"], "hi");

        let tasks = pr_tasks(&c, p, 5, 50).unwrap();
        assert_eq!(tasks[0]["id"], 3);
        assert_eq!(tasks[0]["state"], "UNRESOLVED");

        assert_eq!(pr_approve(&c, p, 5).unwrap()["approved"], true);
        assert_eq!(pr_unapprove(&c, p, 5).unwrap()["approved"], false);
        assert_eq!(
            pr_request_changes(&c, p, 5).unwrap()["state"],
            "changes_requested"
        );

        let merged = pr_merge(
            &c,
            p,
            5,
            &PrMerge {
                strategy: Some(MergeStrategy::FastForward),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(merged["state"], "MERGED");
        assert_eq!(pr_decline(&c, p, 5).unwrap()["state"], "DECLINED");
        h.join().unwrap();
    }

    #[test]
    fn repo_and_search_endpoints_roundtrip() {
        let (base, h) = mock_server(vec![
            (
                "GET",
                "/repositories/acme/api",
                "",
                200,
                r#"{"mainbranch":{"name":"develop"}}"#,
            ),
            (
                "GET",
                "/repositories/acme/api/src/develop/README.md",
                "",
                200,
                "# Hello\n",
            ),
            (
                "GET",
                "/repositories/acme/api",
                "",
                200,
                r#"{"mainbranch":null}"#,
            ),
            (
                "GET",
                "/repositories/acme/api/src/HEAD/src",
                "",
                200,
                r#"{"values":[{"path":"src/main.rs","type":"commit_file","size":10}]}"#,
            ),
            (
                "GET",
                "/repositories/acme/api/refs/branches?q=name+%7E+%22feat%22&pagelen=10",
                "",
                200,
                r#"{"values":[{"name":"feat/x","target":{"hash":"abcdef1234567890"}}]}"#,
            ),
            (
                "POST",
                "/workspaces/acme/codesearch",
                "needle",
                200,
                r#"{"id":"r1","matched_files":[{"path":"lib.rs","matched_chunks":[{},{}]}],"pagination":{"has_more":false}}"#,
            ),
        ]);
        let c = Client::new(base, "u@e.com", "tok", Some(5000));
        let p = "/repositories/acme/api";

        match repo_file(&c, p, "README.md", None).unwrap() {
            RepoFile::Raw(s) => assert_eq!(s, "# Hello\n"),
            RepoFile::Dir(_) => panic!("expected raw file"),
        }
        match repo_file(&c, p, "src", None).unwrap() {
            RepoFile::Dir(v) => {
                assert_eq!(v[0]["path"], "src/main.rs");
                assert_eq!(v[0]["type"], "commit_file");
            }
            RepoFile::Raw(_) => panic!("expected directory listing"),
        }
        let branches = branch_query(
            &c,
            p,
            &BranchQuery {
                query: Some("feat"),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(branches[0]["name"], "feat/x");
        assert!(branches[0]["target"]["hash"].is_string());

        let hits = search_code(&c, "acme", "needle", Some("api"), 10).unwrap();
        assert_eq!(hits[0]["file"]["path"], "lib.rs");
        assert_eq!(hits[0]["content_match_count"], 2);
        h.join().unwrap();
    }
}
