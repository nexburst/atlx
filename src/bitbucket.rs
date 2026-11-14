//! Bitbucket Cloud REST 2.0, shaped after twg's `bitbucket repo|pull-requests|pipeline`.

use crate::client::Client;
use crate::markup::quote;
use crate::time;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

/// Resolve twg-style `--workspace` / `--repo` (slug, `ws/slug`, or URL) to an API path.
pub fn repo_path(repo: Option<&str>, workspace: Option<&str>, default_ws: Option<&str>) -> Result<String> {
    let repo = repo.ok_or_else(|| anyhow!("pass --repo (slug, workspace/slug, or Bitbucket URL)"))?;
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
    Ok(format!("/repositories/{}/{}", enc(ws), enc(slug.trim_end_matches(".git"))))
}

pub fn workspace_or<'a>(ws: Option<&'a str>, default_ws: Option<&'a str>) -> Result<&'a str> {
    ws.or(default_ws)
        .ok_or_else(|| anyhow!("no Bitbucket workspace; pass --workspace or run `atlx setup bitbucket`"))
}

fn web_url(path: &str) -> String {
    format!("https://bitbucket.org{}", path.trim_start_matches("/repositories"))
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
            format!("(author.nickname = {q} OR author.display_name ~ {q})", q = quote(a))
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
            .map(|u| u["display_name"].as_str().or(u["user"]["display_name"].as_str()).unwrap_or_default().to_string())
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
    let mut reviewers: Vec<Value> = n.reviewer_uuids.iter().map(|u| json!({"uuid": u})).collect();
    reviewers.extend(n.reviewer_account_ids.iter().map(|a| json!({"account_id": a})));
    if !reviewers.is_empty() {
        req["reviewers"] = reviewers.into();
    }
    Ok(pr_summary(&c.post(&format!("{path}/pullrequests"), &req)?))
}

/// A PR number or a Bitbucket pull-request URL.
pub fn pr_number(s: &str) -> Result<u64> {
    let s = s.trim().trim_end_matches('/');
    let tail = match s.find("/pull-requests/") {
        Some(i) => s[i + 15..].split(['/', '?', '#']).next().unwrap_or_default(),
        None => s,
    };
    tail.parse().map_err(|_| anyhow!("'{s}' is not a pull request number or URL"))
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

// --- pipelines ---------------------------------------------------------------

fn state_of(p: &Value) -> (String, Option<String>) {
    let state = p["state"]["name"].as_str().unwrap_or_default().to_string();
    let result = p["state"]["result"]["name"].as_str().or(p["state"]["stage"]["name"].as_str()).map(String::from);
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
                && eq(o.trigger, p["trigger"]["name"].as_str().map(|t| t.trim_end_matches("_TRIGGER")))
        })
        .take(o.limit)
        .map(|p| pipeline_summary(path, p))
        .collect())
}

/// Build number, `{uuid}`, or a pipeline result URL -> the id the API accepts.
pub fn pipeline_ref(s: &str) -> String {
    let s = s.trim().trim_end_matches('/');
    match s.find("/results/") {
        Some(i) => s[i + 9..].split(['/', '?', '#']).next().unwrap_or_default().to_string(),
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
    let p = c.get(&format!("{path}/pipelines/{}", enc(&pipeline_ref(pipeline))), &[])?;
    let uuid = p["uuid"].as_str().ok_or_else(|| anyhow!("pipeline has no uuid"))?;
    let mut out = pipeline_summary(path, &p);
    let steps = paginate(c, &format!("{path}/pipelines/{}/steps", enc(uuid)), &[], 100)?;
    let mut rendered = Vec::new();
    for s in &steps {
        let (state, result) = state_of(s);
        let name = s["name"].as_str().unwrap_or_default();
        let mut step = json!({"name": name, "uuid": s["uuid"], "state": state, "result": result,
                              "seconds": s["duration_in_seconds"]});
        let failed = matches!(result.as_deref(), Some("FAILED" | "ERROR"));
        let wanted = o.step.is_none_or(|w| w == name || Some(w) == s["uuid"].as_str());
        if o.logs && wanted && (!o.failed_steps || failed) {
            let log_path = format!("{path}/pipelines/{}/steps/{}/log", enc(uuid), enc(s["uuid"].as_str().unwrap_or("")));
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
    let start = if n == 0 { 0 } else { lines.len().saturating_sub(n) };
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_paths() {
        assert_eq!(repo_path(Some("acme/api"), None, None).unwrap(), "/repositories/acme/api");
        assert_eq!(repo_path(Some("api"), None, Some("acme")).unwrap(), "/repositories/acme/api");
        assert_eq!(repo_path(Some("api"), Some("other"), Some("acme")).unwrap(), "/repositories/other/api");
        assert_eq!(
            repo_path(Some("https://bitbucket.org/acme/api/pull-requests/3"), None, None).unwrap(),
            "/repositories/acme/api"
        );
        assert!(repo_path(Some("api"), None, None).is_err());
        assert!(repo_path(None, Some("acme"), None).is_err());
    }

    #[test]
    fn refs() {
        assert_eq!(pr_number("42").unwrap(), 42);
        assert_eq!(pr_number("https://bitbucket.org/acme/api/pull-requests/930/diff").unwrap(), 930);
        assert!(pr_number("abc").is_err());
        assert_eq!(pipeline_ref("https://bitbucket.org/acme/api/pipelines/results/1234"), "1234");
        assert_eq!(pipeline_ref("{abc-def}"), "{abc-def}");
        assert_eq!(tail("a\nb\nc", 2), "b\nc");
        assert_eq!(tail("a\nb\nc", 0), "a\nb\nc");
    }

    #[test]
    fn pr_filters() {
        let f = pr_filter(&PrQuery { author: Some("jane"), source: Some("feat/x"), created_since: Some("2026-01-01"), ..Default::default() })
            .unwrap()
            .unwrap();
        assert_eq!(
            f,
            r#"(author.nickname = "jane" OR author.display_name ~ "jane") AND source.branch.name = "feat/x" AND created_on >= 2026-01-01"#
        );
        assert_eq!(pr_filter(&PrQuery::default()).unwrap(), None);
    }
}
