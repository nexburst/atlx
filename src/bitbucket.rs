//! Bitbucket Cloud REST 2.0, shaped after twg's `bitbucket repo|pull-requests|pipeline|search`.

use crate::client::Client;
use crate::markup::quote;
use crate::time;
use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use serde_json::{Value, json};

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

fn web_url(path: &str) -> String {
    format!(
        "https://bitbucket.org{}",
        path.trim_start_matches("/repositories")
    )
}

/// Collect up to `limit` items, following `next` links.
fn paginate(c: &Client, path: &str, query: &[(&str, &str)], limit: usize) -> Result<Vec<Value>> {
    let pagelen = limit.clamp(1, 50).to_string();
    let mut q = query.to_vec();
    q.push(("pagelen", &pagelen));
    let mut v = c.get(path, &q)?;
    let mut out = Vec::new();
    loop {
        out.extend(v["values"].as_array().into_iter().flatten().cloned());
        match v["next"].as_str() {
            Some(next) if out.len() < limit => v = c.get(next, &[])?,
            _ => break,
        }
    }
    out.truncate(limit);
    Ok(out)
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

fn repo_summary(r: &Value) -> Value {
    json!({
        "repo": r["full_name"],
        "name": r["name"],
        "project": r["project"]["key"],
        "private": r["is_private"],
        "language": r["language"],
        "main_branch": r["mainbranch"]["name"],
        "updated": r["updated_on"],
        "url": r["links"]["html"]["href"],
    })
}

/// `bitbucket repo query`
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
    let items = paginate(c, &format!("/repositories/{}", enc(ws)), &q, o.limit)?;
    Ok(items.iter().map(repo_summary).collect())
}

/// `bitbucket repo get`
pub fn repo_get(c: &Client, path: &str) -> Result<Value> {
    let r = c.get(path, &[])?;
    let mut o = repo_summary(&r);
    o["description"] = r["description"].clone();
    o["size"] = r["size"].clone();
    o["created"] = r["created_on"].clone();
    o["clone"] = r["links"]["clone"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| json!({"name": l["name"], "href": l["href"]}))
        .collect();
    Ok(o)
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

fn dir_entry_summary(e: &Value) -> Value {
    let kind = match e["type"].as_str() {
        Some("commit_directory") => "dir",
        Some("commit_file") => "file",
        Some(other) => other,
        None => "",
    };
    json!({
        "path": e["path"],
        "type": kind,
        "size": e["size"],
    })
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
        return Ok(RepoFile::Dir(items.iter().map(dir_entry_summary).collect()));
    }
    let text = match v {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(&other)?,
    };
    Ok(RepoFile::Raw(text))
}

fn branch_summary(b: &Value) -> Value {
    let hash = b["target"]["hash"].as_str().unwrap_or_default();
    let short: String = hash.chars().take(12).collect();
    json!({
        "name": b["name"],
        "commit": short,
        "date": b["target"]["date"],
        "author": b["target"]["author"]["user"]["display_name"]
            .as_str()
            .or(b["target"]["author"]["raw"].as_str()),
        "message": b["target"]["message"].as_str().map(|m| m.lines().next().unwrap_or_default()),
        "url": b["links"]["html"]["href"],
    })
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

/// `bitbucket branch query` (and the old `repo branches`)
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
        .map(branch_summary)
        .collect())
}

// --- pull requests -----------------------------------------------------------

fn pr_summary(p: &Value) -> Value {
    json!({
        "id": p["id"],
        "title": p["title"],
        "state": p["state"],
        "draft": p["draft"],
        "author": p["author"]["display_name"],
        "source": p["source"]["branch"]["name"],
        "destination": p["destination"]["branch"]["name"],
        "updated": p["updated_on"],
        "url": p["links"]["html"]["href"],
    })
}

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

/// `bitbucket pull-requests query`
pub fn pr_query(c: &Client, path: &str, o: &PrQuery) -> Result<Value> {
    let state = o.state.unwrap_or("OPEN").to_ascii_uppercase();
    let filter = pr_filter(o)?;
    let mut q = vec![("state", state.as_str()), ("sort", "-updated_on")];
    if let Some(f) = &filter {
        q.push(("q", f));
    }
    let items = paginate(c, &format!("{path}/pullrequests"), &q, o.limit)?;
    Ok(items.iter().map(pr_summary).collect())
}

pub struct PrGet {
    pub comments: bool,
    pub diff: bool,
    pub statuses: bool,
}

/// `bitbucket pull-requests get`
pub fn pr_get(c: &Client, path: &str, id: u64, o: &PrGet) -> Result<Value> {
    let base = format!("{path}/pullrequests/{id}");
    let p = c.get(&base, &[])?;
    let mut out = pr_summary(&p);
    let names = |key: &str, only_approved: bool| -> Value {
        p[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|u| !only_approved || u["approved"] == true)
            .map(|u| {
                u["display_name"]
                    .as_str()
                    .or(u["user"]["display_name"].as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .into()
    };
    out["reviewers"] = names("reviewers", false);
    out["approved_by"] = names("participants", true);
    out["comment_count"] = p["comment_count"].clone();
    out["task_count"] = p["task_count"].clone();
    out["description"] = p["description"].clone();
    if o.comments {
        out["comments"] = paginate(c, &format!("{base}/comments"), &[], 500)?
            .iter()
            .filter(|cm| cm["deleted"] != true)
            .map(|cm| {
                json!({
                    "id": cm["id"],
                    "author": cm["user"]["display_name"],
                    "created": cm["created_on"],
                    "reply_to": cm["parent"]["id"],
                    "path": cm["inline"]["path"],
                    "line": cm["inline"]["to"],
                    "body": cm["content"]["raw"],
                })
            })
            .collect();
    }
    if o.diff {
        out["diffstat"] = paginate(c, &format!("{base}/diffstat"), &[], 500)?
            .iter()
            .map(|d| {
                json!({
                    "path": d["new"]["path"].as_str().or(d["old"]["path"].as_str()),
                    "status": d["status"],
                    "added": d["lines_added"],
                    "removed": d["lines_removed"],
                })
            })
            .collect();
    }
    if o.statuses {
        out["statuses"] = paginate(c, &format!("{base}/statuses"), &[], 100)?
            .iter()
            .map(|s| json!({"name": s["name"], "key": s["key"], "state": s["state"], "url": s["url"]}))
            .collect();
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
    Ok(pr_summary(&c.post(&format!("{path}/pullrequests"), &req)?))
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

pub struct NewComment<'a> {
    pub text: &'a str,
    pub reply_to: Option<u64>,
    pub path: Option<&'a str>,
    pub line: Option<u64>,
}

/// `bitbucket pull-requests comment create`
pub fn pr_comment(c: &Client, path: &str, id: u64, n: NewComment) -> Result<Value> {
    let mut req = json!({"content": {"raw": n.text}});
    if let Some(p) = n.reply_to {
        req["parent"] = json!({"id": p});
    }
    if let Some(file) = n.path {
        req["inline"] = json!({"path": file});
        if let Some(l) = n.line {
            req["inline"]["to"] = l.into();
        }
    }
    let v = c.post(&format!("{path}/pullrequests/{id}/comments"), &req)?;
    Ok(json!({"id": v["id"], "pull_request": id, "url": v["links"]["html"]["href"]}))
}

/// `bitbucket pull-requests diff`
pub fn pr_diff(c: &Client, path: &str, id: u64) -> Result<String> {
    match c.get(&format!("{path}/pullrequests/{id}/diff"), &[])? {
        Value::String(s) => Ok(s),
        Value::Null => Ok(String::new()),
        other => Ok(other.to_string()),
    }
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

fn activity_summary(item: &Value) -> Value {
    let (kind, ev) = activity_event(item);
    let actor = match kind {
        "update" => ev["author"]["display_name"]
            .as_str()
            .or(ev["user"]["display_name"].as_str()),
        _ => ev["user"]["display_name"]
            .as_str()
            .or(ev["author"]["display_name"].as_str()),
    };
    let timestamp = ev["created_on"].as_str().or(ev["date"].as_str());
    let details = match kind {
        "comment" => {
            let body = if ev["deleted"] == true {
                "(deleted)"
            } else {
                ev["content"]["raw"].as_str().unwrap_or_default()
            };
            let loc = ev["inline"]["path"]
                .as_str()
                .map(|p| match ev["inline"]["to"].as_u64() {
                    Some(line) => format!("[{p}:{line}] "),
                    None => format!("[{p}] "),
                })
                .unwrap_or_default();
            match ev["id"].as_u64() {
                Some(id) => format!("#{id} {loc}{body}"),
                None => format!("{loc}{body}"),
            }
        }
        "approval" => "approved".to_string(),
        "changes_requested" => "requested changes".to_string(),
        "update" => {
            let state = ev["state"].as_str().unwrap_or_default();
            let mut parts = Vec::new();
            if !state.is_empty() {
                parts.push(state.to_string());
            }
            if let Some(changes) = ev["changes"].as_object().filter(|m| !m.is_empty()) {
                let keys: Vec<&str> = changes.keys().map(String::as_str).collect();
                parts.push(format!("changed: {}", keys.join(", ")));
            }
            if let Some(reason) = ev["reason"].as_str().filter(|s| !s.is_empty()) {
                parts.push(format!("reason: {reason}"));
            }
            if parts.is_empty() {
                ev["title"].as_str().unwrap_or("updated").to_string()
            } else {
                parts.join("; ")
            }
        }
        _ => ev.to_string(),
    };
    json!({
        "type": kind,
        "actor": actor,
        "timestamp": timestamp,
        "details": details,
    })
}

/// `bitbucket pull-requests activity`
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
        .map(activity_summary)
        .collect())
}

fn task_summary(t: &Value) -> Value {
    json!({
        "id": t["id"],
        "state": t["state"],
        "body": t["content"]["raw"],
        "creator": t["creator"]["display_name"],
        "comment_id": t["comment"]["id"],
        "created": t["created_on"],
        "updated": t["updated_on"],
    })
}

/// `bitbucket pull-requests task query` (and the old `tasks`)
pub fn pr_tasks(c: &Client, path: &str, id: u64, limit: usize) -> Result<Value> {
    let items = paginate(c, &format!("{path}/pullrequests/{id}/tasks"), &[], limit)?;
    Ok(items.iter().map(task_summary).collect())
}

fn participant_summary(id: u64, v: &Value) -> Value {
    json!({
        "pull_request": id,
        "approved": v["approved"],
        "state": v["state"],
        "user": v["user"]["display_name"],
    })
}

/// `bitbucket pull-requests approve`
pub fn pr_approve(c: &Client, path: &str, id: u64) -> Result<Value> {
    let v = c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/approve"),
        &[],
        None,
    )?;
    Ok(participant_summary(id, &v))
}

/// `bitbucket pull-requests unapprove`
pub fn pr_unapprove(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.delete(&format!("{path}/pullrequests/{id}/approve"), &[])?;
    Ok(json!({
        "pull_request": id,
        "approved": false,
    }))
}

/// `bitbucket pull-requests request-changes`
pub fn pr_request_changes(c: &Client, path: &str, id: u64) -> Result<Value> {
    let v = c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/request-changes"),
        &[],
        None,
    )?;
    Ok(participant_summary(id, &v))
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
    let req = pr_merge_body(o);
    Ok(pr_summary(
        &c.post(&format!("{path}/pullrequests/{id}/merge"), &req)?,
    ))
}

/// `bitbucket pull-requests decline`
pub fn pr_decline(c: &Client, path: &str, id: u64) -> Result<Value> {
    let v = c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/decline"),
        &[],
        None,
    )?;
    Ok(pr_summary(&v))
}

// --- search ------------------------------------------------------------------

fn code_search_query(query: &str, repo: Option<&str>) -> String {
    match repo.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => {
            let slug = r
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(r)
                .trim_end_matches(".git");
            format!("{query} repo:{slug}")
        }
        None => query.to_string(),
    }
}

fn code_search_summary(r: &Value) -> Value {
    let self_href = r["file"]["links"]["self"]["href"].as_str();
    let repo = self_href.and_then(|u| {
        let rest = u.split("/repositories/").nth(1)?;
        let mut parts = rest.split('/');
        Some(format!("{}/{}", parts.next()?, parts.next()?))
    });
    let snippet = r["content_matches"]
        .as_array()
        .and_then(|ms| ms.first())
        .and_then(|m| m["lines"].as_array())
        .map(|lines| {
            lines
                .iter()
                .filter_map(|l| {
                    let text: String = l["segments"]
                        .as_array()?
                        .iter()
                        .filter_map(|s| s["text"].as_str())
                        .collect();
                    Some(match l["line"].as_u64() {
                        Some(n) => format!("{n}: {}", text.trim_end()),
                        None => text.trim_end().to_string(),
                    })
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
    json!({
        "repo": repo,
        "path": r["file"]["path"],
        "matches": r["content_match_count"],
        "snippet": snippet,
        "url": self_href,
    })
}

/// `bitbucket search code`
pub fn search_code(
    c: &Client,
    ws: &str,
    query: &str,
    repo: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let q = code_search_query(query, repo);
    let items = paginate(
        c,
        &format!("/workspaces/{}/search/code", enc(ws)),
        &[("search_query", &q)],
        limit,
    )?;
    Ok(items.iter().map(code_search_summary).collect())
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

fn pipeline_summary(path: &str, p: &Value) -> Value {
    let (state, result) = state_of(p);
    let hash = p["target"]["commit"]["hash"].as_str().unwrap_or_default();
    json!({
        "build": p["build_number"],
        "uuid": p["uuid"],
        "state": state,
        "result": result,
        "branch": p["target"]["ref_name"],
        "trigger": p["trigger"]["name"],
        "commit": &hash[..hash.len().min(12)],
        "created": p["created_on"],
        "seconds": p["duration_in_seconds"],
        "url": format!("{}/pipelines/results/{}", web_url(path), p["build_number"]),
    })
}

#[derive(Default)]
pub struct PipelineQuery<'a> {
    pub state: Option<&'a str>,
    pub status: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub trigger: Option<&'a str>,
    pub limit: usize,
}

/// `bitbucket pipeline query`. State/result/trigger are filtered client-side
/// over the most recent runs.
pub fn pipeline_query(c: &Client, path: &str, o: &PipelineQuery) -> Result<Value> {
    let mut q = vec![("sort", "-created_on")];
    if let Some(b) = o.branch {
        q.push(("target.branch", b));
    }
    let filtering = o.state.is_some() || o.status.is_some() || o.trigger.is_some();
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
                && eq(o.status, result.as_deref())
                && eq(
                    o.trigger,
                    p["trigger"]["name"]
                        .as_str()
                        .map(|t| t.trim_end_matches("_TRIGGER")),
                )
        })
        .take(o.limit)
        .map(|p| pipeline_summary(path, p))
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

/// `bitbucket pipeline get`: run details and steps, optionally with log tails.
pub fn pipeline_get(c: &Client, path: &str, pipeline: &str, o: &PipelineGet) -> Result<Value> {
    let p = c.get(
        &format!("{path}/pipelines/{}", enc(&pipeline_ref(pipeline))),
        &[],
    )?;
    let uuid = p["uuid"]
        .as_str()
        .ok_or_else(|| anyhow!("pipeline has no uuid"))?;
    let mut out = pipeline_summary(path, &p);
    let steps = paginate(
        c,
        &format!("{path}/pipelines/{}/steps", enc(uuid)),
        &[],
        100,
    )?;
    let mut rendered = Vec::new();
    for s in &steps {
        let (state, result) = state_of(s);
        let name = s["name"].as_str().unwrap_or_default();
        let mut step = json!({"name": name, "uuid": s["uuid"], "state": state, "result": result,
                              "seconds": s["duration_in_seconds"]});
        let failed = matches!(result.as_deref(), Some("FAILED" | "ERROR"));
        let wanted = o
            .step
            .is_none_or(|w| w == name || Some(w) == s["uuid"].as_str());
        if o.logs && wanted && (!o.failed_steps || failed) {
            let log_path = format!(
                "{path}/pipelines/{}/steps/{}/log",
                enc(uuid),
                enc(s["uuid"].as_str().unwrap_or(""))
            );
            step["log"] = match c.get(&log_path, &[]) {
                Ok(Value::String(text)) => tail(&text, o.lines).into(),
                Ok(Value::Null) => "".into(),
                Ok(other) => other,
                Err(e) => json!({"error": format!("{e:#}")}),
            };
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

        let dir = dir_entry_summary(&json!({"path": "src", "type": "commit_directory"}));
        assert_eq!(dir["type"], "dir");
        let file =
            dir_entry_summary(&json!({"path": "src/main.rs", "type": "commit_file", "size": 42}));
        assert_eq!(file["type"], "file");
        assert_eq!(file["size"], 42);

        let br = branch_summary(&json!({
            "name": "main",
            "target": {
                "hash": "0123456789abcdef",
                "date": "2026-01-01T00:00:00Z",
                "author": {"user": {"display_name": "Jane"}},
                "message": "Initial commit\n\nBody"
            },
            "links": {"html": {"href": "https://bitbucket.org/acme/api/branch/main"}}
        }));
        assert_eq!(br["name"], "main");
        assert_eq!(br["commit"], "0123456789ab");
        assert_eq!(br["author"], "Jane");
        assert_eq!(br["message"], "Initial commit");
    }

    #[test]
    fn activity_and_task_summaries() {
        let c = activity_summary(&json!({
            "comment": {
                "id": 7,
                "created_on": "2026-01-02T00:00:00Z",
                "user": {"display_name": "Alice"},
                "content": {"raw": "Nit here"},
                "inline": {"path": "src/lib.rs", "to": 14}
            }
        }));
        assert_eq!(c["type"], "comment");
        assert_eq!(c["actor"], "Alice");
        assert_eq!(c["details"], "#7 [src/lib.rs:14] Nit here");

        let a = activity_summary(&json!({
            "approval": {"date": "2026-01-03T00:00:00Z", "user": {"display_name": "Bob"}}
        }));
        assert_eq!(a["type"], "approval");
        assert_eq!(a["actor"], "Bob");
        assert_eq!(a["details"], "approved");

        let u = activity_summary(&json!({
            "update": {
                "date": "2026-01-04T00:00:00Z",
                "state": "MERGED",
                "author": {"display_name": "Carol"},
                "changes": {"status": {"old": "OPEN", "new": "MERGED"}}
            }
        }));
        assert_eq!(u["type"], "update");
        assert_eq!(u["actor"], "Carol");
        assert_eq!(u["details"], "MERGED; changed: status");

        let t = task_summary(&json!({
            "id": 9,
            "state": "UNRESOLVED",
            "content": {"raw": "Add test"},
            "creator": {"display_name": "Alice"},
            "comment": {"id": 7},
            "created_on": "2026-01-02T00:00:00Z",
            "updated_on": "2026-01-02T01:00:00Z"
        }));
        assert_eq!(t["id"], 9);
        assert_eq!(t["state"], "UNRESOLVED");
        assert_eq!(t["body"], "Add test");
        assert_eq!(t["comment_id"], 7);
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

        assert_eq!(code_search_query("foo bar", None), "foo bar");
        assert_eq!(code_search_query("foo", Some("api")), "foo repo:api");
        assert_eq!(code_search_query("foo", Some("acme/api")), "foo repo:api");

        let cs = code_search_summary(&json!({
            "content_match_count": 1,
            "content_matches": [{
                "lines": [{"line": 12, "segments": [{"text": "fn ", "match": false}, {"text": "foo", "match": true}]}]
            }],
            "file": {
                "path": "src/main.rs",
                "links": {"self": {"href": "https://api.bitbucket.org/2.0/repositories/acme/api/src/abc/src/main.rs"}}
            }
        }));
        assert_eq!(cs["repo"], "acme/api");
        assert_eq!(cs["path"], "src/main.rs");
        assert_eq!(cs["matches"], 1);
        assert_eq!(cs["snippet"], "12: fn foo");
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
        assert_eq!(act[0]["type"], "comment");

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
                "GET",
                "/workspaces/acme/search/code?search_query=needle+repo%3Aapi&pagelen=10",
                "",
                200,
                r#"{"values":[{"content_match_count":2,"file":{"path":"lib.rs"}}]}"#,
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
                assert_eq!(v[0]["type"], "file");
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
        assert_eq!(branches[0]["commit"], "abcdef123456");

        let hits = search_code(&c, "acme", "needle", Some("api"), 10).unwrap();
        assert_eq!(hits[0]["path"], "lib.rs");
        assert_eq!(hits[0]["matches"], 2);
        h.join().unwrap();
    }
}
