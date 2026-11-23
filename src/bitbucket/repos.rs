//! Branch and commit writes, commit history, Code Insights, workspaces and
//! repository extras.

use super::{Listed, Paging, enc, lifecycle, list, list_where, repo_src_path, scan};
use crate::client::{ApiError, Client, ErrorKind, Part};
use crate::markup::quote;
use anyhow::{Result, anyhow};
use clap::ValueEnum;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How many commits `repo contributors` reads at most.
pub const COMMIT_SCAN: usize = 1000;

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

fn not_found(e: &anyhow::Error) -> bool {
    ApiError::find(e).is_some_and(|a| a.kind == ErrorKind::NotFound)
}

fn main_branch(c: &Client, path: &str) -> Result<String> {
    c.get(path, &[])?["mainbranch"]["name"]
        .as_str()
        .filter(|b| !b.is_empty())
        .map(String::from)
        .ok_or_else(|| usage("the repository has no main branch; name one"))
}

// --- branches ----------------------------------------------------------------

fn branch_path(path: &str, name: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err(usage("the branch name can't be empty"));
    }
    Ok(format!("{path}/refs/branches/{}", enc(name)))
}

/// The commit a branch points at, or the revision itself when it is a hash.
fn resolve_commit(c: &Client, path: &str, rev: &str) -> Result<String> {
    match c.get(&branch_path(path, rev)?, &[]) {
        Ok(v) => v["target"]["hash"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| anyhow!("branch '{rev}' has no commit")),
        Err(e) if not_found(&e) => {
            if (7..=40).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_hexdigit()) {
                Ok(rev.to_string())
            } else {
                Err(usage(format!("'{rev}' is not a branch or a commit hash")))
            }
        }
        Err(e) => Err(e),
    }
}

/// `from` defaults to the main branch.
pub fn branch_create(c: &Client, path: &str, name: &str, from: Option<&str>) -> Result<Value> {
    if name.trim().is_empty() {
        return Err(usage("the branch name can't be empty"));
    }
    let from = match from {
        Some(f) => f.to_string(),
        None => main_branch(c, path)?,
    };
    let hash = resolve_commit(c, path, &from)?;
    c.post(
        &format!("{path}/refs/branches"),
        &json!({"name": name, "target": {"hash": hash}}),
    )
}

pub fn branch_get(c: &Client, path: &str, name: &str) -> Result<Value> {
    c.get(&branch_path(path, name)?, &[])
}

pub fn branch_delete(c: &Client, path: &str, name: &str) -> Result<Value> {
    c.delete(&branch_path(path, name)?, &[])?;
    Ok(json!({"id": name, "deleted": true}))
}

// --- commits -----------------------------------------------------------------

#[derive(Debug, Default)]
pub struct CommitQuery<'a> {
    pub include: &'a [String],
    pub exclude: &'a [String],
    pub file_path: Option<&'a str>,
}

/// Without `include` the main branch's history, not every branch's.
pub fn commit_list(c: &Client, path: &str, o: &CommitQuery, p: &Paging) -> Result<Listed> {
    let mut include = o.include.to_vec();
    if include.is_empty() {
        include.push(main_branch(c, path)?);
    }
    let mut q: Vec<(&str, &str)> = include.iter().map(|r| ("include", r.as_str())).collect();
    q.extend(o.exclude.iter().map(|r| ("exclude", r.as_str())));
    if let Some(p) = o.file_path {
        q.push(("path", p));
    }
    list(c, &format!("{path}/commits"), &q, p)
}

pub fn commit_get(c: &Client, path: &str, hash: &str) -> Result<Value> {
    if hash.trim().is_empty() {
        return Err(usage("pass a commit hash"));
    }
    c.get(&format!("{path}/commit/{}", enc(hash.trim())), &[])
}

/// What happens to one file in `commit create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Local { repo_path: String, local: PathBuf },
    Text { repo_path: String, text: String },
    Delete { repo_path: String },
}

impl Change {
    fn repo_path(&self) -> &str {
        match self {
            Change::Local { repo_path, .. }
            | Change::Text { repo_path, .. }
            | Change::Delete { repo_path } => repo_path,
        }
    }
}

/// A path inside the repository, as given.
pub fn check_repo_path(p: &str) -> Result<()> {
    let bad_segment = p
        .split('/')
        .any(|s| matches!(s, "" | "." | "..") || s.eq_ignore_ascii_case(".git"));
    let bad_char = p
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '"' | '\\'));
    if bad_segment || bad_char {
        return Err(usage(format!(
            "{p:?} can't be a repository path: use a relative path with forward slashes and no '.', '..' or .git"
        )));
    }
    Ok(())
}

#[derive(Debug)]
pub struct NewCommit<'a> {
    pub message: &'a str,
    /// `None` commits to the default branch.
    pub branch: Option<&'a str>,
    pub author: Option<&'a str>,
    /// The head the branch must still be at.
    pub parent: Option<&'a str>,
    pub changes: &'a [Change],
}

/// A commit through `POST /src`. File fields get a leading `/`, so a
/// file called `message` or `branch` can't pass for that field.
pub fn commit_create(c: &Client, path: &str, n: &NewCommit) -> Result<Value> {
    if n.message.trim().is_empty() {
        return Err(usage("--message can't be empty"));
    }
    if n.branch.is_some_and(|b| b.trim().is_empty()) {
        return Err(usage("--branch can't be empty"));
    }
    if n.changes.is_empty() {
        return Err(usage(
            "nothing to commit: pass --file, --file-local, --files-from or --delete",
        ));
    }
    let mut seen = HashMap::new();
    let mut parts = vec![Part::text("message", n.message)];
    for (key, value) in [
        ("branch", n.branch),
        ("author", n.author),
        ("parents", n.parent),
    ] {
        if let Some(v) = value {
            parts.push(Part::text(key, v));
        }
    }
    for change in n.changes {
        let repo_path = change.repo_path();
        check_repo_path(repo_path)?;
        if seen.insert(repo_path, ()).is_some() {
            return Err(usage(format!("{repo_path} is given twice")));
        }
        let field = format!("/{repo_path}");
        parts.push(match change {
            Change::Local { local, .. } => {
                if !local.is_file() {
                    return Err(usage(format!("{} is not a readable file", local.display())));
                }
                Part::File {
                    field,
                    filename: repo_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(repo_path)
                        .to_string(),
                    content_type: None,
                    path: local.clone(),
                }
            }
            Change::Text { text, .. } => Part::text(field, text.as_str()),
            // A path under `files` with no content of its own is deleted.
            Change::Delete { .. } => Part::text("files", field),
        });
    }
    let resp = c.post_multipart_response(&format!("{path}/src"), &parts)?;
    let hash = resp
        .location
        .as_deref()
        .and_then(|l| l.trim_end_matches('/').rsplit('/').next());
    Ok(json!({
        "hash": hash,
        "branch": n.branch,
        "message": n.message,
        "files": n.changes.iter().map(Change::repo_path).collect::<Vec<_>>(),
    }))
}

// --- Code Insights -----------------------------------------------------------

pub fn report_create(
    c: &Client,
    path: &str,
    commit: &str,
    id: &str,
    report: &Value,
) -> Result<Value> {
    if !report.is_object() {
        return Err(usage("--report must be a JSON object"));
    }
    if commit.trim().is_empty() || id.trim().is_empty() {
        return Err(usage("pass a commit and a report ID"));
    }
    c.put(
        &format!("{path}/commit/{}/reports/{}", enc(commit), enc(id)),
        report,
    )
}

pub fn reports(c: &Client, path: &str, commit: &str, p: &Paging) -> Result<Listed> {
    if commit.trim().is_empty() {
        return Err(usage("pass --commit"));
    }
    list(c, &format!("{path}/commit/{}/reports", enc(commit)), &[], p)
}

// --- workspaces --------------------------------------------------------------

pub fn workspace_get(c: &Client, ws: &str) -> Result<Value> {
    c.get(&format!("/workspaces/{}", enc(ws)), &[])
}

/// Members; `name` keeps those whose nickname or display name contains
/// it, `permissions` reads the permission list instead (needs admin).
pub fn members(
    c: &Client,
    ws: &str,
    name: Option<&str>,
    permissions: bool,
    p: &Paging,
) -> Result<Listed> {
    let what = if permissions {
        "permissions"
    } else {
        "members"
    };
    let url = format!("/workspaces/{}/{what}", enc(ws));
    let Some(needle) = name.map(str::to_lowercase) else {
        return list(c, &url, &[], p);
    };
    list_where(c, &url, &[], p, |m| {
        Ok(["nickname", "display_name"].iter().any(|k| {
            m["user"][*k]
                .as_str()
                .is_some_and(|v| v.to_lowercase().contains(&needle))
        }))
    })
}

/// A member by `me`, UUID, account ID, nickname or display name.
pub fn member(c: &Client, ws: &str, who: &str) -> Result<Value> {
    let repos = format!("/repositories/{}", enc(ws));
    let user = lifecycle::resolve_user(c, &repos, lifecycle::User::Any(who.trim()))?;
    let id = user["uuid"]
        .as_str()
        .or(user["account_id"].as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| usage("pass a member"))?;
    c.get(&format!("/workspaces/{}/members/{}", enc(ws), enc(id)), &[])
}

// --- repositories ------------------------------------------------------------

#[derive(Debug)]
pub struct NewRepo<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub language: Option<&'a str>,
    pub project: Option<&'a str>,
    pub private: bool,
}

pub fn repo_create(c: &Client, path: &str, n: &NewRepo) -> Result<Value> {
    let mut body = Map::new();
    body.insert("scm".into(), "git".into());
    body.insert("is_private".into(), n.private.into());
    for (k, v) in [
        ("name", n.name),
        ("description", n.description),
        ("language", n.language),
    ] {
        if let Some(v) = v {
            body.insert(k.into(), v.into());
        }
    }
    if let Some(p) = n.project {
        body.insert("project".into(), json!({"key": p}));
    }
    c.post(path, &Value::Object(body))
}

/// A ref as `/src` takes it: the main branch when none is given, and the
/// commit of a branch whose name has a slash.
fn src_ref(c: &Client, path: &str, git_ref: Option<&str>) -> Result<String> {
    let git_ref = match git_ref {
        Some(r) => r.to_string(),
        None => main_branch(c, path)?,
    };
    if git_ref.contains('/') {
        resolve_commit(c, path, &git_ref)
    } else {
        Ok(git_ref)
    }
}

/// The entries of a directory (`max_depth` descends).
pub fn file_list(
    c: &Client,
    path: &str,
    git_ref: Option<&str>,
    dir: Option<&str>,
    max_depth: Option<u32>,
    p: &Paging,
) -> Result<Listed> {
    let at = src_ref(c, path, git_ref)?;
    let mut url = repo_src_path(path, &at, dir.unwrap_or_default());
    if !url.ends_with('/') {
        url.push('/');
    }
    let depth = max_depth.map(|d| d.to_string());
    let q: Vec<(&str, &str)> = depth.iter().map(|d| ("max_depth", d.as_str())).collect();
    list(c, &url, &q, p)
}

/// A file with its text: `{path, ref, commit, size, type, content}`. A
/// directory is a usage error pointing at `file list`; a file that is not
/// UTF-8 text has `content: null` and a warning.
pub fn file_get(c: &Client, path: &str, file: &str, git_ref: Option<&str>) -> Result<Value> {
    let clean = file.trim().trim_matches('/');
    if clean.is_empty() || clean == "." {
        return Err(usage(
            "pass a file path; list a directory with `repo file list`",
        ));
    }
    let at = src_ref(c, path, git_ref)?;
    let url = repo_src_path(path, &at, clean);
    let meta = c.get(&url, &[("format", "meta")])?;
    if meta["type"] == "commit_directory" {
        return Err(ApiError::usage(format!("{clean} is a directory"))
            .with_hint(format!(
                "list it with `atlx bitbucket repo file list --path {clean}`"
            ))
            .into());
    }
    let bytes = c.get_bytes(&url, &[])?;
    let content = match String::from_utf8(bytes.data) {
        Ok(text) => Value::String(text),
        Err(_) => {
            crate::output::notes::warn(
                "binary_file",
                format!(
                    "{clean} is not UTF-8 text; save it with `atlx bitbucket repo file download {clean}`"
                ),
            );
            Value::Null
        }
    };
    Ok(json!({
        "path": meta["path"].as_str().unwrap_or(clean),
        "ref": git_ref.map_or(at.clone(), String::from),
        "commit": meta["commit"]["hash"],
        "size": meta["size"],
        "type": meta["type"],
        "content": content,
    }))
}

/// Save a file to `dest`: `{path, file, bytes}`.
pub fn file_download(
    c: &Client,
    path: &str,
    file: &str,
    git_ref: Option<&str>,
    dest: &Path,
    overwrite: bool,
) -> Result<Value> {
    let clean = file.trim().trim_matches('/');
    if clean.is_empty() {
        return Err(usage("pass a file path"));
    }
    let at = src_ref(c, path, git_ref)?;
    let bytes = c.download_to(&repo_src_path(path, &at, clean), &[], dest, overwrite)?;
    Ok(json!({"path": clean, "file": dest.display().to_string(), "bytes": bytes}))
}

#[derive(Debug, Default)]
pub struct Contributors<'a> {
    pub git_ref: Option<&'a str>,
    /// `from..to`
    pub range: Option<&'a str>,
    /// `YYYY-MM-DD`
    pub since: Option<&'a str>,
    pub limit: usize,
}

/// Commit authors over at most [`COMMIT_SCAN`] commits.
pub fn contributors(c: &Client, path: &str, o: &Contributors) -> Result<Value> {
    let (include, exclude) = match (o.range, o.git_ref) {
        (Some(range), _) => {
            let (from, to) = range
                .split_once("..")
                .filter(|(f, t)| !f.is_empty() && !t.is_empty() && !t.starts_with('.'))
                .ok_or_else(|| usage("--range looks like main..feature"))?;
            (to.to_string(), Some(from))
        }
        (None, Some(r)) => (r.to_string(), None),
        (None, None) => (main_branch(c, path)?, None),
    };
    let mut q = vec![("include", include.as_str())];
    if let Some(x) = exclude {
        q.push(("exclude", x));
    }
    let (commits, more) = scan(c, &format!("{path}/commits"), &q, COMMIT_SCAN)?;
    let mut by: HashMap<String, (Value, u64)> = HashMap::new();
    for commit in &commits {
        let day = commit["date"]
            .as_str()
            .and_then(|d| d.get(..10))
            .unwrap_or_default();
        if o.since.is_some_and(|s| day < s) {
            continue;
        }
        let a = &commit["author"];
        let key = a["user"]["uuid"]
            .as_str()
            .or(a["raw"].as_str())
            .unwrap_or("unknown");
        by.entry(key.to_string())
            .or_insert_with(|| (a.clone(), 0))
            .1 += 1;
    }
    let mut list: Vec<(String, (Value, u64))> = by.into_iter().collect();
    list.sort_by(|(ka, (_, a)), (kb, (_, b))| b.cmp(a).then_with(|| ka.cmp(kb)));
    let values: Vec<Value> = list
        .into_iter()
        .take(o.limit)
        .map(|(_, (author, n))| json!({"author": author, "commits": n}))
        .collect();
    if more {
        crate::output::notes::warn(
            "scan_limit",
            format!("counted the {COMMIT_SCAN} most recent commits only"),
        );
    }
    Ok(values.into())
}

// --- search ------------------------------------------------------------------

/// `--in` on `pr search`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Field {
    Both,
    Title,
    Description,
}

impl Field {
    fn names(self) -> &'static [&'static str] {
        match self {
            Field::Both => &["title", "description"],
            Field::Title => &["title"],
            Field::Description => &["description"],
        }
    }
}

/// Pull requests whose title or description (per `field`) contains `text`.
pub fn search_prs(
    c: &Client,
    path: &str,
    text: &str,
    field: Field,
    state: Option<&str>,
    p: &Paging,
) -> Result<Listed> {
    if text.trim().is_empty() {
        return Err(usage("pass the text to look for"));
    }
    let terms: Vec<String> = field
        .names()
        .iter()
        .map(|f| format!("{f} ~ {}", quote(text)))
        .collect();
    let filter = format!("({})", terms.join(" OR "));
    let states = super::states(state);
    let mut q: Vec<(&str, &str)> = states.iter().map(|s| ("state", s.as_str())).collect();
    q.push(("sort", "-updated_on"));
    q.push(("q", filter.as_str()));
    list(c, &format!("{path}/pullrequests"), &q, p)
}

/// `pr search --mine`: scans like `pr list --mine`, then matches the text.
pub fn search_my_prs(
    c: &Client,
    o: &lifecycle::Mine,
    text: &str,
    field: Field,
) -> Result<lifecycle::Found> {
    let all = lifecycle::Mine {
        limit: usize::MAX,
        ..*o
    };
    let mut found = lifecycle::mine(c, &all)?;
    let needle = text.to_lowercase();
    found.items.retain(|pr| {
        field.names().iter().any(|f| {
            pr[*f]
                .as_str()
                .is_some_and(|v| v.to_lowercase().contains(&needle))
        })
    });
    found.more |= found.items.len() > o.limit;
    found.items.truncate(o.limit);
    Ok(found)
}
