//! Bitbucket Cloud REST 2.0: repositories, branches, commits, pull requests,
//! pipelines, deployments and workspaces.
//!
//! API functions take a repository as its REST path
//! (`/repositories/{workspace}/{slug}`, see [`repo_path`]) and return the
//! REST objects; the command layer projects them onto the records in
//! [`records`]. List functions return one page of results and the cursor
//! for the next (see [`list`]).

use crate::client::{ApiError, Client, ErrorKind};
use crate::markup::quote;
use crate::paging::{self, Cursor, Page, Position};
use crate::time;
use anyhow::{Result, anyhow};
use clap::ValueEnum;
use serde_json::{Value, json};

pub mod deployments;
pub mod lifecycle;
pub mod pipelines;
pub mod records;
pub mod repos;

pub(crate) fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

fn usage(msg: impl Into<String>) -> anyhow::Error {
    ApiError::usage(msg).into()
}

/// The REST path of a repository given as `workspace/slug`, a Bitbucket URL,
/// or a bare slug in `workspace` (else `default_ws`).
pub fn repo_path(
    repo: Option<&str>,
    workspace: Option<&str>,
    default_ws: Option<&str>,
) -> Result<String> {
    let repo = repo.ok_or_else(|| {
        anyhow::Error::from(
            ApiError::usage("no repository: pass --repo workspace/slug").with_hint(
                "inside a clone of a bitbucket.org repository, or with `bitbucket.repo` in .atlx.toml, --repo can be left out",
            ),
        )
    })?;
    let trimmed = repo
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("bitbucket.org/")
        .trim_end_matches('/');
    let mut parts = trimmed.split('/');
    let (ws, slug) = match (parts.next(), parts.next()) {
        (Some(ws), Some(slug)) if !ws.is_empty() && !slug.is_empty() => (ws, slug),
        (Some(slug), None) if !slug.is_empty() => (workspace_or(workspace, default_ws)?, slug),
        _ => {
            return Err(usage(format!(
                "invalid --repo '{repo}': use workspace/slug"
            )));
        }
    };
    Ok(format!(
        "/repositories/{}/{}",
        enc(ws),
        enc(slug.trim_end_matches(".git"))
    ))
}

/// The workspace a repository value points at: its own `workspace/` part or
/// URL, else `workspace`, else the default workspace.
pub fn workspace_of(
    repo: &str,
    workspace: Option<&str>,
    default_ws: Option<&str>,
) -> Result<String> {
    let path = repo_path(Some(repo), workspace, default_ws)?;
    let (ws, _) = split_path(&path);
    Ok(ws)
}

/// `(workspace, slug)` of a `/repositories/{ws}/{slug}` path, decoded.
pub fn split_path(path: &str) -> (String, String) {
    let mut parts = path.trim_start_matches("/repositories/").splitn(2, '/');
    let decode = |s: &str| urlencoding::decode(s).map_or(s.to_string(), |d| d.into_owned());
    let ws = decode(parts.next().unwrap_or_default());
    let slug = decode(parts.next().unwrap_or_default());
    (ws, slug)
}

/// `workspace/slug` from a `/repositories/{ws}/{slug}` path.
pub fn full_name(path: &str) -> String {
    let (ws, slug) = split_path(path);
    format!("{ws}/{slug}")
}

pub fn workspace_or<'a>(ws: Option<&'a str>, default_ws: Option<&'a str>) -> Result<&'a str> {
    ws.or(default_ws).ok_or_else(|| {
        ApiError::config("no Bitbucket workspace: pass --workspace")
            .with_hint("save a default with `atlx auth bitbucket --workspace NAME`, or set ATLX_BB_WORKSPACE")
            .into()
    })
}

// --- paging ------------------------------------------------------------------

/// Records per REST page (most Bitbucket list endpoints allow 50 at most).
pub const PAGELEN: usize = 50;

/// How many REST pages a client-side filter reads per call before it stops
/// and hands back a cursor.
pub const SCAN_PAGES: usize = 20;

/// `--limit` and `--cursor` of a list command.
#[derive(Debug, Clone, Default)]
pub struct Paging {
    pub limit: usize,
    pub cursor: Option<String>,
}

impl Paging {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            cursor: None,
        }
    }
}

/// One page of records and the cursor that continues it (`None` when there
/// is no more).
pub type Listed = (Vec<Value>, Option<String>);

/// A REST page position, as atlx encodes it in a cursor token:
/// `<pagelen>:<page>`, so later pages keep the page size of the first.
fn token(pagelen: usize, page: &str) -> Position {
    Position::Token(format!("{pagelen}:{page}"))
}

/// The page size and page of a position; `None` page for the first.
fn at(pos: &Position, wanted: usize) -> Result<(usize, Option<String>)> {
    match pos {
        Position::Start => Ok((wanted.clamp(1, PAGELEN), None)),
        Position::Page(p) => Ok((PAGELEN, Some(p.to_string()))),
        Position::Token(t) => {
            let (len, page) = t
                .split_once(':')
                .and_then(|(l, p)| Some((l.parse::<usize>().ok()?, p)))
                .filter(|(l, p)| (1..=PAGELEN).contains(l) && !p.is_empty())
                .ok_or_else(|| usage("--cursor is not a cursor from a Bitbucket result"))?;
            Ok((len, Some(page.to_string())))
        }
        Position::Offset(_) => Err(usage("--cursor is not a cursor from a Bitbucket result")),
    }
}

/// The `page` parameter of a REST `next` link.
fn next_page(v: &Value) -> Option<String> {
    let next = v["next"].as_str()?;
    let query = next.split_once('?')?.1;
    query.split('&').find_map(|kv| {
        let (k, val) = kv.split_once('=')?;
        (k == "page").then(|| {
            urlencoding::decode(val)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| val.to_string())
        })
    })
}

/// Fetch one REST page of `path` at `pos`.
fn fetch_page(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    pos: &Position,
    wanted: usize,
) -> Result<(Vec<Value>, Option<Position>)> {
    let (len, page) = at(pos, wanted)?;
    let len_s = len.to_string();
    let mut q = query.to_vec();
    q.push(("pagelen", &len_s));
    if let Some(p) = &page {
        q.push(("page", p));
    }
    let v = c.get(path, &q)?;
    let items = v["values"].as_array().cloned().unwrap_or_default();
    let next = next_page(&v).map(|p| token(len, &p));
    Ok((items, next))
}

/// `--limit` records of a REST collection from `--cursor` on.
pub fn list(c: &Client, path: &str, query: &[(&str, &str)], p: &Paging) -> Result<Listed> {
    paging::collect(p.limit, p.cursor.as_deref(), |pos, wanted| {
        let (items, next) = fetch_page(c, path, query, pos, wanted)?;
        Ok(Page { items, next })
    })
}

/// [`list`] with a client-side filter. Reads full pages, keeps what `keep`
/// accepts, and stops after [`SCAN_PAGES`] pages with a cursor, so a filter
/// that matches nothing costs a bounded number of requests.
pub fn list_where(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    p: &Paging,
    keep: impl FnMut(&mut Value) -> Result<bool>,
) -> Result<Listed> {
    list_where_capped(c, path, query, p, SCAN_PAGES, keep)
}

/// [`list_where`] reading at most `max_pages` REST pages per call (each
/// item may cost `keep` a request of its own).
pub fn list_where_capped(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    p: &Paging,
    max_pages: usize,
    mut keep: impl FnMut(&mut Value) -> Result<bool>,
) -> Result<Listed> {
    let limit = p.limit.clamp(1, paging::HARD_CAP);
    let mut cur = match p.cursor.as_deref() {
        Some(s) => Cursor::decode(s)?,
        None => Cursor::start(),
    };
    let mut out = Vec::new();
    let mut pages = 0;
    loop {
        let (items, next) = fetch_page(c, path, query, &cur.position, PAGELEN)?;
        let mut kept = Vec::new();
        for mut item in items {
            if keep(&mut item)? {
                kept.push(item);
            }
        }
        let available = kept.len().saturating_sub(cur.skip);
        let need = limit - out.len();
        let take = available.min(need);
        let used = cur.skip + take;
        out.extend(kept.into_iter().skip(cur.skip).take(take));
        if take < available {
            let resume = Cursor {
                position: cur.position,
                skip: used,
            };
            return Ok((out, Some(resume.encode())));
        }
        let Some(next) = next else {
            return Ok((out, None));
        };
        pages += 1;
        if out.len() >= limit || pages >= max_pages {
            return Ok((out, Some(Cursor::at(next).encode())));
        }
        cur = Cursor::at(next);
    }
}

/// Up to `cap` items of a collection, for scans that need all of them
/// (counts, lookups). Also says whether more were left.
pub(crate) fn scan(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    cap: usize,
) -> Result<(Vec<Value>, bool)> {
    let mut out = Vec::new();
    let mut pos = Position::Start;
    loop {
        let (items, next) = fetch_page(c, path, query, &pos, PAGELEN)?;
        out.extend(items);
        match next {
            Some(n) if out.len() < cap => pos = n,
            next => {
                let more = next.is_some() || out.len() > cap;
                out.truncate(cap);
                return Ok((out, more));
            }
        }
    }
}

/// [`scan`] without the flag.
pub(crate) fn scan_all(
    c: &Client,
    path: &str,
    query: &[(&str, &str)],
    cap: usize,
) -> Result<Vec<Value>> {
    Ok(scan(c, path, query, cap)?.0)
}

fn and(filters: Vec<String>) -> Option<String> {
    (!filters.is_empty()).then(|| filters.join(" AND "))
}

// --- repositories ------------------------------------------------------------

/// Kept for the cross-product search and the doctor: repositories whose
/// name contains `query`.
pub struct RepoQuery<'a> {
    pub query: Option<&'a str>,
    pub project: Option<&'a str>,
    pub language: Option<&'a str>,
    pub sort: &'a str,
    pub role: Option<&'a str>,
    pub limit: usize,
}

/// Repositories of a workspace as an array (see [`repo_list`]).
pub fn repo_query(c: &Client, ws: &str, o: &RepoQuery) -> Result<Value> {
    let l = RepoList {
        name: o.query,
        project: o.project,
        language: o.language,
        sort: Some(o.sort),
        role: o.role,
        bbql: None,
    };
    Ok(repo_list(c, ws, &l, &Paging::new(o.limit))?.0.into())
}

/// Filters for `bitbucket repo list`.
#[derive(Debug, Default)]
pub struct RepoList<'a> {
    /// Name contains.
    pub name: Option<&'a str>,
    pub project: Option<&'a str>,
    pub language: Option<&'a str>,
    pub sort: Option<&'a str>,
    /// owner, admin, contributor, member.
    pub role: Option<&'a str>,
    /// A BBQL filter, combined with the others by AND.
    pub bbql: Option<&'a str>,
}

pub fn repo_list(c: &Client, ws: &str, o: &RepoList, p: &Paging) -> Result<Listed> {
    let mut f = Vec::new();
    if let Some(q) = o.name {
        f.push(format!("name ~ {}", quote(q)));
    }
    if let Some(p) = o.project {
        f.push(format!("project.key = {}", quote(p)));
    }
    if let Some(l) = o.language {
        f.push(format!("language = {}", quote(&l.to_lowercase())));
    }
    if let Some(q) = o.bbql {
        f.push(format!("({q})"));
    }
    let filter = and(f);
    let mut q = vec![("sort", o.sort.unwrap_or("-updated_on"))];
    if let Some(f) = &filter {
        q.push(("q", f));
    }
    if let Some(r) = o.role {
        q.push(("role", r));
    }
    list(c, &format!("/repositories/{}", enc(ws)), &q, p)
}

/// The REST repository.
pub fn repo_get(c: &Client, path: &str) -> Result<Value> {
    c.get(path, &[])
}

pub(crate) fn repo_src_path(repo_path: &str, git_ref: &str, file_path: &str) -> String {
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

/// Options for `bitbucket repo branch list`.
#[derive(Default)]
pub struct BranchQuery<'a> {
    /// Name contains.
    pub name: Option<&'a str>,
    /// Head commit author: display name (part of it) or account ID.
    pub author: Option<&'a str>,
    /// Sort field, e.g. `name` or `-target.date`.
    pub sort: Option<&'a str>,
    pub bbql: Option<&'a str>,
}

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

/// Branches; the author filter runs client side.
pub fn branch_list(c: &Client, path: &str, o: &BranchQuery, p: &Paging) -> Result<Listed> {
    let mut f = Vec::new();
    if let Some(q) = o.name {
        f.push(format!("name ~ {}", quote(q)));
    }
    if let Some(q) = o.bbql {
        f.push(format!("({q})"));
    }
    let filter = and(f);
    let mut params = Vec::new();
    if let Some(f) = &filter {
        params.push(("q", f.as_str()));
    }
    params.push(("sort", o.sort.unwrap_or("-target.date")));
    let url = format!("{path}/refs/branches");
    match o.author {
        None => list(c, &url, &params, p),
        Some(a) => list_where(c, &url, &params, p, |b| Ok(branch_by_author(b, a))),
    }
}

// --- pull requests -----------------------------------------------------------

/// Filters for `bitbucket pr list`.
#[derive(Default, Clone, Copy)]
pub struct PrQuery<'a> {
    pub state: Option<&'a str>,
    pub author: Option<&'a str>,
    pub source: Option<&'a str>,
    pub dest: Option<&'a str>,
    /// Title or description contains.
    pub text: Option<&'a str>,
    pub created_since: Option<&'a str>,
    pub created_before: Option<&'a str>,
    pub updated_since: Option<&'a str>,
    pub updated_before: Option<&'a str>,
    /// `{uuid}`, account ID, or exact nickname.
    pub reviewer: Option<&'a str>,
    /// A BBQL filter, combined with the others by AND.
    pub bbql: Option<&'a str>,
}

/// The BBQL filter for a pull request list (state is a separate parameter).
pub(crate) fn pr_filter(o: &PrQuery) -> Result<Option<String>> {
    let mut f = Vec::new();
    if let Some(a) = o.author {
        f.push(if a.starts_with('{') {
            format!("author.uuid = {}", quote(a))
        } else if lifecycle::is_account_id(a) {
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
        } else if lifecycle::is_account_id(r) {
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
    if let Some(q) = o.bbql {
        f.push(format!("({q})"));
    }
    Ok(and(f))
}

/// The state parameter: one state, or every state for `ALL`.
fn states(state: Option<&str>) -> Vec<String> {
    match state.map(str::to_ascii_uppercase).as_deref() {
        None => vec!["OPEN".into()],
        Some("ALL") => ["OPEN", "MERGED", "DECLINED", "SUPERSEDED"]
            .map(String::from)
            .to_vec(),
        Some(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
    }
}

/// Pull requests of a repository, newest update first.
pub fn pr_list(c: &Client, path: &str, o: &PrQuery, p: &Paging) -> Result<Listed> {
    let states = states(o.state);
    let filter = pr_filter(o)?;
    let mut q: Vec<(&str, &str)> = states.iter().map(|s| ("state", s.as_str())).collect();
    q.push(("sort", "-updated_on"));
    if let Some(f) = &filter {
        q.push(("q", f));
    }
    list(c, &format!("{path}/pullrequests"), &q, p)
}

/// One pull request.
pub fn pr_get(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.get(&format!("{path}/pullrequests/{id}"), &[])
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

/// A pull request number, or a Bitbucket pull request URL.
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
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| usage(format!("'{s}' is not a pull request number or URL")))
}

/// The repository of a pull request URL (`workspace/slug`), if it is one.
pub fn pr_url_repo(s: &str) -> Option<String> {
    let s = s.trim();
    let i = s.find("/pull-requests/")?;
    let head = s[..i]
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("bitbucket.org/");
    let mut parts = head.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(ws), Some(slug), None) if !ws.is_empty() && !slug.is_empty() => {
            Some(format!("{ws}/{slug}"))
        }
        _ => None,
    }
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

/// The request body for a new comment.
pub fn comment_body(n: &NewComment) -> Result<Value> {
    if n.text.trim().is_empty() {
        return Err(usage("the comment body can't be empty"));
    }
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

pub fn pr_comment(c: &Client, path: &str, id: u64, n: NewComment) -> Result<Value> {
    let req = comment_body(&n)?;
    c.post(&format!("{path}/pullrequests/{id}/comments"), &req)
}

fn comment_path(path: &str, id: u64, cid: u64) -> String {
    format!("{path}/pullrequests/{id}/comments/{cid}")
}

/// Comments in creation order, without deleted ones.
pub fn pr_comment_list(c: &Client, path: &str, id: u64, p: &Paging) -> Result<Listed> {
    list_where(
        c,
        &format!("{path}/pullrequests/{id}/comments"),
        &[("sort", "created_on")],
        p,
        |cm| Ok(cm["deleted"] != true),
    )
}

pub fn pr_comment_get(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.get(&comment_path(path, id, cid), &[])
}

/// Replaces the comment text.
pub fn pr_comment_update(c: &Client, path: &str, id: u64, cid: u64, text: &str) -> Result<Value> {
    if text.trim().is_empty() {
        return Err(usage("the comment body can't be empty"));
    }
    c.put(
        &comment_path(path, id, cid),
        &json!({"content": {"raw": text}}),
    )
}

/// Bitbucket answers 204, so the result names what was removed.
pub fn pr_comment_delete(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.delete(&comment_path(path, id, cid), &[])?;
    Ok(json!({"id": cid, "deleted": true}))
}

/// Resolves the thread, then reads the comment back.
pub fn pr_comment_resolve(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.request(
        "POST",
        &format!("{}/resolve", comment_path(path, id, cid)),
        &[],
        None,
    )?;
    pr_comment_get(c, path, id, cid)
}

/// Reopens the thread (204), then reads the comment back.
pub fn pr_comment_reopen(c: &Client, path: &str, id: u64, cid: u64) -> Result<Value> {
    c.delete(&format!("{}/resolve", comment_path(path, id, cid)), &[])?;
    pr_comment_get(c, path, id, cid)
}

fn task_path(path: &str, id: u64, tid: u64) -> String {
    format!("{path}/pullrequests/{id}/tasks/{tid}")
}

pub fn pr_task_create(
    c: &Client,
    path: &str,
    id: u64,
    text: &str,
    comment: Option<u64>,
) -> Result<Value> {
    if text.trim().is_empty() {
        return Err(usage("the task body can't be empty"));
    }
    let mut req = json!({"content": {"raw": text}});
    if let Some(cid) = comment {
        req["comment"] = json!({"id": cid});
    }
    c.post(&format!("{path}/pullrequests/{id}/tasks"), &req)
}

pub fn pr_task_get(c: &Client, path: &str, id: u64, tid: u64) -> Result<Value> {
    c.get(&task_path(path, id, tid), &[])
}

/// A change to a task.
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
        if text.trim().is_empty() {
            return Err(usage("the task body can't be empty"));
        }
        req["content"] = json!({"raw": text});
    }
    if let Some(r) = t.resolved {
        req["state"] = if r { "RESOLVED" } else { "UNRESOLVED" }.into();
    }
    if req.as_object().is_some_and(|o| o.is_empty()) {
        return Err(usage("nothing to change: pass --body"));
    }
    Ok(req)
}

pub fn pr_task_update(c: &Client, path: &str, id: u64, tid: u64, t: &TaskChange) -> Result<Value> {
    let req = task_body(t)?;
    c.put(&task_path(path, id, tid), &req)
}

/// Bitbucket answers 204.
pub fn pr_task_delete(c: &Client, path: &str, id: u64, tid: u64) -> Result<Value> {
    c.delete(&task_path(path, id, tid), &[])?;
    Ok(json!({"id": tid, "deleted": true}))
}

pub fn pr_tasks(c: &Client, path: &str, id: u64, p: &Paging) -> Result<Listed> {
    list(c, &format!("{path}/pullrequests/{id}/tasks"), &[], p)
}

fn is_conflict(file: &Value) -> bool {
    matches!(
        file["status"].as_str(),
        Some("merge conflict" | "local deleted" | "remote deleted")
    )
}

/// Files changed, with lines added and removed; `conflicts` keeps only
/// the files with merge conflicts.
pub fn pr_diffstat(c: &Client, path: &str, id: u64, conflicts: bool, p: &Paging) -> Result<Listed> {
    let url = format!("{path}/pullrequests/{id}/diffstat");
    if conflicts {
        list_where(c, &url, &[], p, |f| Ok(is_conflict(f)))
    } else {
        list(c, &url, &[], p)
    }
}

pub fn pr_commits(c: &Client, path: &str, id: u64, p: &Paging) -> Result<Listed> {
    list(c, &format!("{path}/pullrequests/{id}/commits"), &[], p)
}

fn raw_text(c: &Client, path: &str, query: &[(&str, &str)]) -> Result<String> {
    let b = c.get_bytes(path, query)?;
    Ok(String::from_utf8_lossy(&b.data).into_owned())
}

/// The unified diff, optionally of one file.
pub fn pr_diff(c: &Client, path: &str, id: u64, file: Option<&str>) -> Result<String> {
    let q: Vec<(&str, &str)> = file.iter().map(|f| ("path", *f)).collect();
    raw_text(c, &format!("{path}/pullrequests/{id}/diff"), &q)
}

/// The mbox-style patch (for `git am`).
pub fn pr_patch(c: &Client, path: &str, id: u64) -> Result<String> {
    raw_text(c, &format!("{path}/pullrequests/{id}/patch"), &[])
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

/// What `bitbucket pr diff` prints.
#[derive(Debug, Default)]
pub struct DiffOpts<'a> {
    pub path: Option<&'a str>,
    /// The mbox patch instead of the diff.
    pub patch: bool,
    /// Only the lines containing this, with the numbers a comment needs.
    pub grep: Option<&'a str>,
}

/// `{pr, path, diff, matches}`: the diff (or patch) text, or with `grep`
/// the matching lines instead.
pub fn pr_diff_view(c: &Client, path: &str, id: u64, o: &DiffOpts) -> Result<Value> {
    if o.grep.is_some_and(str::is_empty) {
        return Err(usage("--grep must not be empty"));
    }
    let text = if o.patch {
        pr_patch(c, path, id)?
    } else {
        pr_diff(c, path, id, o.path)?
    };
    Ok(match o.grep {
        Some(needle) => json!({
            "pr": id, "path": o.path, "diff": null,
            "matches": diff_lines(&text, needle, o.path),
        }),
        None => json!({"pr": id, "path": o.path, "diff": text, "matches": null}),
    })
}

/// The kind of an activity entry (`comment`, `approval`, `update`, ...).
pub fn activity_kind(item: &Value) -> &str {
    for kind in ["comment", "approval", "update", "changes_requested"] {
        if item[kind].is_object() {
            return kind;
        }
    }
    if let Some(obj) = item.as_object() {
        for (k, v) in obj {
            if k != "pull_request" && v.is_object() {
                return k.as_str();
            }
        }
    }
    "unknown"
}

pub fn pr_activity(
    c: &Client,
    path: &str,
    id: u64,
    kind: Option<&str>,
    p: &Paging,
) -> Result<Listed> {
    let url = format!("{path}/pullrequests/{id}/activity");
    match kind {
        None => list(c, &url, &[], p),
        Some(want) => list_where(c, &url, &[], p, |item| {
            Ok(want.eq_ignore_ascii_case(activity_kind(item)))
        }),
    }
}

/// The participant record Bitbucket returns, with the pull request number.
fn review(mut v: Value, id: u64) -> Value {
    if v.is_object() {
        v["pr"] = id.into();
    }
    v
}

pub fn pr_approve(c: &Client, path: &str, id: u64) -> Result<Value> {
    let v = c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/approve"),
        &[],
        None,
    )?;
    Ok(review(v, id))
}

pub fn pr_unapprove(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.delete(&format!("{path}/pullrequests/{id}/approve"), &[])?;
    Ok(json!({"pr": id, "approved": false, "state": null}))
}

pub fn pr_request_changes(c: &Client, path: &str, id: u64) -> Result<Value> {
    let v = c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/request-changes"),
        &[],
        None,
    )?;
    Ok(review(v, id))
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
    /// Merge only if the source branch head is still this commit.
    pub if_head: Option<&'a str>,
}

pub(crate) fn pr_merge_body(o: &PrMerge) -> Value {
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

/// Merge without waiting (see [`lifecycle::merge`]).
pub fn pr_merge(c: &Client, path: &str, id: u64, o: &PrMerge) -> Result<Value> {
    lifecycle::merge(c, path, id, o, None)
}

pub fn pr_decline(c: &Client, path: &str, id: u64) -> Result<Value> {
    c.request(
        "POST",
        &format!("{path}/pullrequests/{id}/decline"),
        &[],
        None,
    )
}

// --- code search -------------------------------------------------------------

fn repo_slug(repo: &str) -> &str {
    let repo = repo.trim().trim_end_matches('/');
    let last = repo.rsplit('/').next().unwrap_or(repo);
    last.trim_end_matches(".git")
}

/// The search term and the filters. The modifiers `repo:`, `project:`,
/// `path:`, `ext:`, `lang:` in the query become filters; text in quotes is
/// never read as one.
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

/// Code search across a workspace (`POST /workspaces/{ws}/codesearch`, a
/// read). The cursor carries the search's request id and its own cursor.
pub fn search_code(
    c: &Client,
    ws: &str,
    query: &str,
    repo: Option<&str>,
    p: &Paging,
) -> Result<Listed> {
    let (term, filters) = code_search_query(query, repo);
    if term.is_empty() || term.chars().count() > 1024 {
        return Err(usage("search needs 1 to 1024 characters of text"));
    }
    let mut search = json!({"search_term": term});
    if let Some(f) = filters {
        search["filters"] = f;
    }
    let path = format!("/workspaces/{}/codesearch", enc(ws));
    let limit = p.limit.clamp(1, paging::HARD_CAP);
    let mut next: Option<(String, String)> = match p.cursor.as_deref() {
        None => None,
        Some(s) => match Cursor::decode(s)?.position {
            Position::Token(t) => {
                let (id, cur) = t
                    .split_once('|')
                    .ok_or_else(|| usage("--cursor is not a cursor from a code search"))?;
                Some((id.to_string(), cur.to_string()))
            }
            _ => return Err(usage("--cursor is not a cursor from a code search")),
        },
    };
    let mut hits = Vec::new();
    loop {
        let mut body = json!({
            "query": search,
            "config": {"max_results": (limit - hits.len()).min(100)},
        });
        if let Some((id, cursor)) = &next {
            body["request_id"] = id.clone().into();
            body["pagination"] = json!({"next_cursor": cursor});
        }
        let v = c.post_read(&path, &body)?;
        let files = v["matched_files"]
            .as_array()
            .ok_or_else(|| anyhow!("Bitbucket returned no matched_files list"))?;
        hits.extend(files.iter().take(limit - hits.len()).cloned());
        let page = &v["pagination"];
        if page["has_more"] != true {
            return Ok((hits, None));
        }
        let (Some(id), Some(cursor)) = (v["id"].as_str(), page["next_cursor"].as_str()) else {
            return Err(anyhow!(
                "Bitbucket said there are more results but gave no cursor for them"
            ));
        };
        let following = (id.to_string(), cursor.to_string());
        if next.as_ref() == Some(&following) {
            return Err(anyhow!(
                "Bitbucket returned the same code search page twice"
            ));
        }
        if hits.len() >= limit {
            let t = Position::Token(format!("{}|{}", following.0, following.1));
            return Ok((hits, Some(Cursor::at(t).encode())));
        }
        next = Some(following);
    }
}

// --- pipelines ---------------------------------------------------------------

pub(crate) fn state_of(p: &Value) -> (String, Option<String>) {
    let state = p["state"]["name"].as_str().unwrap_or_default().to_string();
    let result = p["state"]["result"]["name"]
        .as_str()
        .or(p["state"]["stage"]["name"].as_str())
        .map(String::from);
    (state, result)
}

#[derive(Default, Clone, Copy)]
pub struct PipelineQuery<'a> {
    pub state: Option<&'a str>,
    /// SUCCESSFUL (or PASSED), FAILED, ERROR, STOPPED.
    pub result: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub trigger: Option<&'a str>,
    /// The target selector's pattern, e.g. a custom pipeline's name.
    pub pattern: Option<&'a str>,
}

impl PipelineQuery<'_> {
    fn filters(&self) -> bool {
        self.state.is_some()
            || self.result.is_some()
            || self.trigger.is_some()
            || self.pattern.is_some()
    }

    fn keep(&self, p: &Value) -> bool {
        let eq = |a: Option<&str>, b: Option<&str>| match (a, b) {
            (None, _) => true,
            (Some(want), Some(got)) => want.eq_ignore_ascii_case(got),
            (Some(_), None) => false,
        };
        let (state, result) = state_of(p);
        let result_ok = eq(self.result, result.as_deref())
            || (self
                .result
                .is_some_and(|s| s.eq_ignore_ascii_case("PASSED"))
                && result.as_deref() == Some("SUCCESSFUL"));
        eq(self.state, Some(&state))
            && result_ok
            && eq(self.pattern, p["target"]["selector"]["pattern"].as_str())
            && eq(
                self.trigger,
                p["trigger"]["name"]
                    .as_str()
                    .map(|t| t.trim_end_matches("_TRIGGER")),
            )
    }
}

/// Pipeline runs, newest first. State, result, trigger and pattern are
/// filtered client side.
pub fn pipeline_list(c: &Client, path: &str, o: &PipelineQuery, p: &Paging) -> Result<Listed> {
    let mut q = vec![("sort", "-created_on")];
    if let Some(b) = o.branch {
        q.push(("target.branch", b));
    }
    let url = format!("{path}/pipelines");
    if o.filters() {
        list_where(c, &url, &q, p, |v| Ok(o.keep(v)))
    } else {
        list(c, &url, &q, p)
    }
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

/// One pipeline run.
pub fn pipeline_get(c: &Client, path: &str, pipeline: &str) -> Result<Value> {
    c.get(
        &format!("{path}/pipelines/{}", enc(&pipeline_ref(pipeline))),
        &[],
    )
}

/// A reason an API call came back as the wrong kind of thing.
pub(crate) fn not_found(message: impl Into<String>) -> anyhow::Error {
    ApiError::new(ErrorKind::NotFound, message).into()
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
        let none = repo_path(None, Some("acme"), None).unwrap_err();
        assert_eq!(ApiError::find(&none).unwrap().kind, ErrorKind::Usage);
        assert_eq!(full_name("/repositories/ac%20me/api"), "ac me/api");
    }

    #[test]
    fn refs() {
        assert_eq!(pr_number("42").unwrap(), 42);
        assert_eq!(
            pr_number("https://bitbucket.org/acme/api/pull-requests/930/diff").unwrap(),
            930
        );
        assert!(pr_number("abc").is_err());
        assert!(pr_number("0").is_err());
        assert_eq!(
            pr_url_repo("https://bitbucket.org/acme/api/pull-requests/930/diff").as_deref(),
            Some("acme/api")
        );
        assert_eq!(pr_url_repo("930"), None);
        assert_eq!(
            pipeline_ref("https://bitbucket.org/acme/api/pipelines/results/1234"),
            "1234"
        );
        assert_eq!(pipeline_ref("{abc-def}"), "{abc-def}");
    }

    #[test]
    fn pr_filters() {
        let f = pr_filter(&PrQuery {
            author: Some("jane"),
            source: Some("feat/x"),
            created_since: Some("2026-01-01"),
            bbql: Some("comment_count > 2"),
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            f,
            r#"(author.nickname = "jane" OR author.display_name ~ "jane") AND source.branch.name = "feat/x" AND created_on >= 2026-01-01 AND (comment_count > 2)"#
        );
        assert_eq!(pr_filter(&PrQuery::default()).unwrap(), None);
        assert_eq!(states(Some("all")).len(), 4);
        assert_eq!(states(None), ["OPEN"]);
    }

    #[test]
    fn cursors_keep_the_page_size() {
        assert_eq!(at(&Position::Start, 10).unwrap(), (10, None));
        assert_eq!(at(&Position::Start, 500).unwrap(), (50, None));
        assert_eq!(
            at(&Position::Token("25:3".into()), 1).unwrap(),
            (25, Some("3".into()))
        );
        assert_eq!(
            at(&Position::Token("25:a:b".into()), 1).unwrap(),
            (25, Some("a:b".into()))
        );
        assert!(at(&Position::Token("x".into()), 1).is_err());
        assert!(at(&Position::Offset(3), 1).is_err());
        let v = json!({"next": "https://api.bitbucket.org/2.0/x?pagelen=10&page=Zm9v%3D"});
        assert_eq!(next_page(&v).as_deref(), Some("Zm9v="));
        assert_eq!(next_page(&json!({})), None);
    }

    #[test]
    fn src_paths() {
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
}
