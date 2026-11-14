//! Cross-product search (atlx extra; twg's equivalent `rovo search` needs Rovo):
//! fan out to Jira, Confluence and Bitbucket in parallel.

use crate::client::Client;
use crate::config::Config;
use crate::{Product, bitbucket, confluence, jira};
use anyhow::Result;
use serde_json::{Map, Value, json};

fn settle(r: Result<Value>) -> Value {
    r.unwrap_or_else(|e| json!({"error": format!("{e:#}")}))
}

pub fn run(cfg: &Config, query: &str, limit: usize, only: &[Product]) -> Value {
    run_with(
        query,
        limit,
        only,
        cfg.bitbucket_configured(),
        || cfg.jira(),
        || cfg.confluence(),
        || cfg.bitbucket(),
    )
}

#[doc(hidden)]
pub fn run_with(
    query: &str,
    limit: usize,
    only: &[Product],
    bitbucket_configured: bool,
    jira_client: impl Fn() -> Result<Client> + Sync,
    confluence_client: impl Fn() -> Result<Client> + Sync,
    bitbucket_client: impl Fn() -> Result<(Client, Option<String>)> + Sync,
) -> Value {
    let explicit = |p: Product| only.contains(&p);
    let want = |p: Product| only.is_empty() || explicit(p);

    let (j, cf, bb) = std::thread::scope(|s| {
        let j = want(Product::Jira).then(|| {
            s.spawn(|| {
                let c = jira_client()?;
                Ok(jira::search(&c, query, limit, &[], None)?["issues"].take())
            })
        });
        let cf = want(Product::Confluence).then(|| {
            s.spawn(|| {
                let c = confluence_client()?;
                let cql = confluence::text_cql(query, None, None)?;
                Ok(confluence::search_query(&c, &cql, limit, true, None)?["results"].take())
            })
        });
        // Bitbucket is optional: only search it when configured or asked for.
        let bb = (want(Product::Bitbucket)
            && (explicit(Product::Bitbucket) || bitbucket_configured))
            .then(|| {
                s.spawn(|| {
                    let (c, ws) = bitbucket_client()?;
                    let ws = bitbucket::workspace_or(None, ws.as_deref())?;
                    let q = bitbucket::RepoQuery {
                        query: Some(query),
                        project: None,
                        language: None,
                        sort: "-updated_on",
                        role: None,
                        limit,
                    };
                    bitbucket::repo_query(&c, ws, &q)
                })
            });
        let join = |h: Option<std::thread::ScopedJoinHandle<'_, Result<Value>>>| {
            h.map(|h| {
                settle(
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("search thread panicked"))),
                )
            })
        };
        (join(j), join(cf), join(bb))
    });

    let mut out = Map::new();
    out.insert("query".into(), query.into());
    for (name, v) in [("jira", j), ("confluence", cf), ("bitbucket", bb)] {
        if let Some(v) = v {
            out.insert(name.into(), v);
        }
    }
    Value::Object(out)
}
