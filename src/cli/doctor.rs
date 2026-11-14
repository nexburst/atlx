//! `atlx doctor`: config, profile, permission and connectivity checks.

use super::{Globals, Out};
use crate::config::{self, Config};
use crate::{bitbucket, confluence, jira};
use serde_json::{Value, json};

fn check_entry(
    status: config::CheckStatus,
    detail: impl Into<String>,
    extra: &[(&str, Value)],
) -> (config::CheckStatus, Value) {
    let mut obj = serde_json::Map::new();
    obj.insert("status".into(), json!(status));
    for (k, v) in extra {
        if !v.is_null() {
            obj.insert((*k).into(), v.clone());
        }
    }
    obj.insert("detail".into(), Value::String(detail.into()));
    (status, Value::Object(obj))
}

fn check_jira(cfg: &Config) -> (config::CheckStatus, Value) {
    let site = cfg.site.as_deref().map(config::normalize_site);
    let site_val = site.as_deref().map_or(Value::Null, |s| json!(s));
    let client = match cfg.jira() {
        Ok(c) => c,
        Err(e) => {
            return check_entry(
                config::CheckStatus::Error,
                format!("{e:#}"),
                &[("site", site_val)],
            );
        }
    };
    match jira::myself(&client) {
        Ok(me) => {
            let name = me["displayName"].as_str().unwrap_or("unknown");
            let detail = match site.as_deref() {
                Some(s) => format!("authenticated as {name} ({s})"),
                None => format!("authenticated as {name}"),
            };
            check_entry(
                config::CheckStatus::Ok,
                detail,
                &[
                    ("site", site_val),
                    ("name", json!(name)),
                    ("email", me["emailAddress"].clone()),
                    ("account_id", me["accountId"].clone()),
                ],
            )
        }
        Err(e) => check_entry(
            config::CheckStatus::Error,
            format!("{e:#}"),
            &[("site", site_val)],
        ),
    }
}

fn check_confluence(cfg: &Config) -> (config::CheckStatus, Value) {
    let site = cfg.site.as_deref().map(config::normalize_site);
    let site_val = site.as_deref().map_or(Value::Null, |s| json!(s));
    let client = match cfg.confluence() {
        Ok(c) => c,
        Err(e) => {
            return check_entry(
                config::CheckStatus::Error,
                format!("{e:#}"),
                &[("site", site_val)],
            );
        }
    };
    match confluence::spaces(&client, None, None, None, 1, None) {
        Ok(spaces) => {
            let count = spaces["results"].as_array().map_or(0, Vec::len);
            let detail = match site.as_deref() {
                Some(s) => format!("reachable ({s}/wiki)"),
                None => "reachable".into(),
            };
            check_entry(
                config::CheckStatus::Ok,
                detail,
                &[("site", site_val), ("spaces_sampled", json!(count))],
            )
        }
        Err(e) => check_entry(
            config::CheckStatus::Error,
            format!("{e:#}"),
            &[("site", site_val)],
        ),
    }
}

fn check_bitbucket(cfg: &Config) -> (config::CheckStatus, Value) {
    if !cfg.bitbucket_configured() {
        return check_entry(
            config::CheckStatus::NotConfigured,
            "no Bitbucket workspace configured (run `atlx setup bitbucket` or set ATLX_BB_WORKSPACE)",
            &[],
        );
    }
    let ws = cfg.bitbucket.workspace.as_deref().unwrap_or_default();
    let ws_val = json!(ws);
    let (client, _) = match cfg.bitbucket() {
        Ok(pair) => pair,
        Err(e) => {
            return check_entry(
                config::CheckStatus::Error,
                format!("{e:#}"),
                &[("workspace", ws_val)],
            );
        }
    };
    let q = bitbucket::RepoQuery {
        query: None,
        project: None,
        language: None,
        sort: "-updated_on",
        role: None,
        limit: 1,
    };
    match bitbucket::repo_query(&client, ws, &q) {
        Ok(repos) => {
            let count = repos.as_array().map_or(0, Vec::len);
            check_entry(
                config::CheckStatus::Ok,
                format!("authenticated (workspace {ws})"),
                &[("workspace", ws_val), ("repos_sampled", json!(count))],
            )
        }
        Err(e) => check_entry(
            config::CheckStatus::Error,
            format!("{e:#}"),
            &[("workspace", ws_val)],
        ),
    }
}

/// Which profile is active and where its tokens come from (never the values).
fn check_profile(cfg: &Config) -> Value {
    let (src, bb_src) = (cfg.token_source(), cfg.bitbucket_token_source());
    let status = if src == "none" {
        config::CheckStatus::NotConfigured
    } else {
        config::CheckStatus::Ok
    };
    check_entry(
        status,
        format!("profile `{}`, token from {src}", cfg.profile),
        &[
            ("name", json!(cfg.profile)),
            ("token_source", json!(src)),
            ("bitbucket_token_source", json!(bb_src)),
        ],
    )
    .1
}

pub(crate) fn doctor_passed(statuses: &[config::CheckStatus], strict: bool) -> bool {
    statuses.iter().all(|s| match s {
        config::CheckStatus::Ok => true,
        config::CheckStatus::Warn | config::CheckStatus::NotConfigured => !strict,
        config::CheckStatus::Error => false,
    })
}

pub(crate) fn doctor(g: &Globals, strict: bool) -> Out {
    let mut cfg_inspection = match config::path() {
        Ok(p) => config::inspect_file(&p),
        Err(e) => config::FileInspection {
            status: config::CheckStatus::Error,
            path: String::new(),
            exists: false,
            mode: None,
            detail: format!("{e:#}"),
        },
    };

    let cfg = match g.config() {
        Ok(c) => Some(c),
        Err(e) => {
            cfg_inspection.status = config::CheckStatus::Error;
            cfg_inspection.detail = format!("{e:#}");
            None
        }
    };

    let ((j_status, j_val), (cf_status, cf_val), (bb_status, bb_val)) = match cfg.as_ref() {
        Some(cfg) => std::thread::scope(|s| {
            let j = s.spawn(|| check_jira(cfg));
            let cf = s.spawn(|| check_confluence(cfg));
            let bb = s.spawn(|| check_bitbucket(cfg));
            let panic_err = |name: &str| {
                check_entry(
                    config::CheckStatus::Error,
                    format!("{name} check panicked"),
                    &[],
                )
            };
            (
                j.join().unwrap_or_else(|_| panic_err("jira")),
                cf.join().unwrap_or_else(|_| panic_err("confluence")),
                bb.join().unwrap_or_else(|_| panic_err("bitbucket")),
            )
        }),
        None => (
            check_entry(
                config::CheckStatus::Error,
                "skipped: config failed to load",
                &[],
            ),
            check_entry(
                config::CheckStatus::Error,
                "skipped: config failed to load",
                &[],
            ),
            check_entry(
                config::CheckStatus::NotConfigured,
                "skipped: config failed to load",
                &[],
            ),
        ),
    };

    let profile_val = match cfg.as_ref() {
        Some(cfg) => check_profile(cfg),
        None => {
            check_entry(
                config::CheckStatus::Error,
                "skipped: config failed to load",
                &[],
            )
            .1
        }
    };

    let ok = doctor_passed(
        &[cfg_inspection.status, j_status, cf_status, bb_status],
        strict,
    );
    let report = json!({
        "ok": ok,
        "strict": strict,
        "checks": {
            "config": cfg_inspection,
            "profile": profile_val,
            "jira": j_val,
            "confluence": cf_val,
            "bitbucket": bb_val,
        }
    });

    if ok {
        Out::Data(report)
    } else {
        Out::Exit(report, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_exit_policy() {
        use config::CheckStatus::{Error, NotConfigured, Ok, Warn};

        assert!(doctor_passed(&[Ok, Ok, Ok, NotConfigured], false));
        assert!(doctor_passed(&[Warn, Ok, Ok, Ok], false));
        assert!(!doctor_passed(&[Ok, Error, Ok, Ok], false));

        assert!(doctor_passed(&[Ok, Ok, Ok, Ok], true));
        assert!(!doctor_passed(&[Warn, Ok, Ok, Ok], true));
        assert!(!doctor_passed(&[Ok, Ok, Ok, NotConfigured], true));
        assert!(!doctor_passed(&[Ok, Error, Ok, Ok], true));
    }
}
