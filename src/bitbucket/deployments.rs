//! Deployments, their environments and environment variables.

use super::{Listed, Paging, enc, list, pipelines::hide_secured, scan, scan_all};
use crate::client::{ApiError, Client, ErrorKind};
use crate::paging::{self, Cursor, Position};
use crate::time::epoch_millis;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};

/// How many deployments a filtered list or a comparison reads. REST can
/// neither sort nor filter them, so that happens here.
pub const SCAN: usize = 5000;

fn bare(uuid: &str) -> &str {
    uuid.trim().trim_matches(['{', '}'])
}

fn braced(uuid: &str) -> String {
    format!("{{{}}}", bare(uuid))
}

fn looks_like_uuid(s: &str) -> bool {
    let s = bare(s);
    s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    ApiError::usage(message).into()
}

pub fn environments(c: &Client, path: &str, p: &Paging) -> Result<Listed> {
    list(c, &format!("{path}/environments"), &[], p)
}

/// An environment UUID from a UUID, name or slug.
pub fn environment(c: &Client, path: &str, r: &str) -> Result<String> {
    if looks_like_uuid(r) {
        return Ok(braced(r));
    }
    let all = scan_all(c, &format!("{path}/environments"), &[], SCAN)?;
    let found: Vec<&str> = all
        .iter()
        .filter(|e| {
            ["name", "slug"].iter().any(|k| {
                e[*k]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case(r.trim()))
            })
        })
        .filter_map(|e| e["uuid"].as_str())
        .collect();
    match found.as_slice() {
        [id] => Ok(id.to_string()),
        _ => Err(
            ApiError::usage(format!("'{r}' matches {} environments", found.len()))
                .with_hint("list them with `atlx bitbucket environment list`")
                .into(),
        ),
    }
}

/// Environment variables, without the values of secured ones.
pub fn variables(c: &Client, path: &str, env: &str, p: &Paging) -> Result<Listed> {
    let id = environment(c, path, env)?;
    let url = format!(
        "{path}/deployments_config/environments/{}/variables",
        enc(&id)
    );
    let (items, next) = list(c, &url, &[], p)?;
    let mut items = Value::Array(items);
    hide_secured(&mut items);
    let Value::Array(items) = items else {
        unreachable!("an array stays an array")
    };
    Ok((items, next))
}

fn in_environment(d: &Value, env: &str) -> bool {
    d["environment"]["uuid"]
        .as_str()
        .is_some_and(|u| bare(u) == bare(env))
}

fn started(d: &Value) -> Option<i64> {
    d["state"]["started_on"]
        .as_str()
        .and_then(|s| epoch_millis(s).ok())
}

fn outcome(d: &Value) -> &str {
    d["state"]["status"]["name"]
        .as_str()
        .or(d["state"]["name"].as_str())
        .unwrap_or_default()
}

fn commit_of(d: &Value) -> Option<&str> {
    ["release", "deployable"]
        .iter()
        .find_map(|k| d[*k]["commit"]["hash"].as_str())
}

/// Filters for `deployment list`; any of them makes atlx read up to
/// [`SCAN`] deployments, filter and sort them by start time.
#[derive(Debug, Default)]
pub struct Filter<'a> {
    pub environment: Option<&'a str>,
    pub statuses: &'a [String],
    pub pipeline: Option<&'a str>,
    pub after: Option<&'a str>,
    pub before: Option<&'a str>,
    /// Oldest first.
    pub ascending: bool,
}

impl Filter<'_> {
    fn any(&self) -> bool {
        self.environment.is_some()
            || !self.statuses.is_empty()
            || self.pipeline.is_some()
            || self.after.is_some()
            || self.before.is_some()
            || self.ascending
    }
}

/// Deployments. Without filters, in REST order; with any, newest started
/// first (or oldest with `ascending`), with an offset cursor into the
/// filtered list.
pub fn deployment_list(c: &Client, path: &str, o: &Filter, p: &Paging) -> Result<Listed> {
    let url = format!("{path}/deployments");
    if !o.any() {
        return list(c, &url, &[], p);
    }
    let after = o.after.map(epoch_millis).transpose()?;
    let before = o.before.map(epoch_millis).transpose()?;
    let env = o.environment.map(|e| environment(c, path, e)).transpose()?;
    let (all, unseen) = scan(c, &url, &[], SCAN)?;
    let mut found: Vec<Value> = all
        .into_iter()
        .filter(|d| {
            let at = started(d);
            env.as_deref().is_none_or(|e| in_environment(d, e))
                && (o.statuses.is_empty()
                    || o.statuses
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case(outcome(d))))
                && o.pipeline.is_none_or(|p| {
                    d["release"]["pipeline"]["uuid"]
                        .as_str()
                        .is_some_and(|u| bare(u) == bare(p))
                })
                && after.is_none_or(|t| at.is_some_and(|a| a >= t))
                && before.is_none_or(|t| at.is_some_and(|a| a <= t))
        })
        .collect();
    found.sort_by_key(started);
    if !o.ascending {
        found.reverse();
    }
    if unseen {
        crate::output::notes::warn(
            "scan_limit",
            format!("only the first {SCAN} deployments were read"),
        );
    }
    let start = match p.cursor.as_deref() {
        None => 0,
        Some(s) => match Cursor::decode(s)?.position {
            Position::Offset(n) => usize::try_from(n).unwrap_or(usize::MAX),
            _ => return Err(usage("--cursor is not a cursor from this deployment list")),
        },
    };
    let limit = p.limit.clamp(1, paging::HARD_CAP);
    let end = start.saturating_add(limit).min(found.len());
    let page = found
        .get(start..end)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    let next = (end < found.len()).then(|| Cursor::at(Position::Offset(end as u64)).encode());
    Ok((page, next))
}

/// One deployment; with `env`, it must be in that environment.
pub fn get(c: &Client, path: &str, uuid: &str, env: Option<&str>) -> Result<Value> {
    let d = c.get(&format!("{path}/deployments/{}", enc(&braced(uuid))), &[])?;
    if let Some(env) = env {
        let env = environment(c, path, env)?;
        if !in_environment(&d, &env) {
            return Err(ApiError::new(
                ErrorKind::NotFound,
                format!("deployment {uuid} is not in that environment"),
            )
            .into());
        }
    }
    Ok(d)
}

/// A commit as a deployment comparison lists it.
fn short_commit(v: &Value) -> Value {
    json!({
        "hash": v["hash"],
        "date": v["date"],
        "author": v["author"]["user"]["display_name"].as_str().or(v["author"]["raw"].as_str()),
        "title": v["message"].as_str().and_then(|m| m.lines().next()),
    })
}

/// What a deployment added and removed compared with its environment's
/// previous successful one: `{deployment, environment, previous, added,
/// removed}`.
pub fn commits(
    c: &Client,
    path: &str,
    uuid: &str,
    env: Option<&str>,
    limit: usize,
) -> Result<Value> {
    let target = get(c, path, uuid, env)?;
    let env = target["environment"]["uuid"]
        .as_str()
        .ok_or_else(|| anyhow!("that deployment has no environment"))?
        .to_string();
    let at = started(&target).ok_or_else(|| anyhow!("that deployment has no start time"))?;
    let new = commit_of(&target).ok_or_else(|| anyhow!("that deployment has no commit"))?;
    let (all, unseen) = scan(c, &format!("{path}/deployments"), &[], SCAN)?;
    if unseen {
        return Err(anyhow!(
            "the repository has more than {SCAN} deployments, so the previous one can't be found reliably"
        ));
    }
    let previous = all
        .iter()
        .filter(|d| in_environment(d, &env) && outcome(d) == "SUCCESSFUL")
        .filter(|d| started(d).is_some_and(|t| t < at))
        .max_by_key(|d| started(d));
    let Some(previous) = previous else {
        return Ok(json!({
            "deployment": target["uuid"],
            "environment": env,
            "previous": null,
            "added": [],
            "removed": [],
        }));
    };
    let old =
        commit_of(previous).ok_or_else(|| anyhow!("the previous deployment has no commit"))?;
    let url = format!("{path}/commits");
    let between = |include: &str, exclude: &str| {
        scan(
            c,
            &url,
            &[("include", include), ("exclude", exclude)],
            limit,
        )
    };
    let (added, more_added) = between(new, old)?;
    let (removed, more_removed) = between(old, new)?;
    if more_added || more_removed {
        crate::output::notes::warn(
            "more_commits",
            format!("more than {limit} commits changed; raise --limit"),
        );
    }
    Ok(json!({
        "deployment": target["uuid"],
        "environment": env,
        "previous": previous["uuid"],
        "added": added.iter().map(short_commit).collect::<Vec<_>>(),
        "removed": removed.iter().map(short_commit).collect::<Vec<_>>(),
    }))
}
