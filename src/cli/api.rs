//! `atlx api`: call any REST endpoint directly.

use super::util::text_arg;
use crate::config::Config;
use crate::{Product, bitbucket};
use anyhow::{Context, Result};
use clap::Args;
use serde_json::Value;

// The command's help text lives on the `Cmd::Api` variant.
#[derive(Args)]
pub(crate) struct ApiArgs {
    product: Product,
    path: String,
    #[arg(short = 'X', long, default_value = "GET")]
    method: String,
    /// JSON body; `-` reads stdin, `@file` reads a file
    #[arg(short, long)]
    data: Option<String>,
    /// Query parameter, repeatable
    #[arg(short, long = "query", value_name = "KEY=VALUE")]
    query: Vec<String>,
    /// Confirm a DELETE (other methods don't need it)
    #[arg(short = 'y', long)]
    yes: bool,
}

pub(crate) fn run(cfg: &Config, a: ApiArgs) -> Result<Value> {
    api(cfg, a.product, &a.path, &a.method, a.data, &a.query)
}

fn api(
    cfg: &Config,
    product: Product,
    path: &str,
    method: &str,
    data: Option<String>,
    query: &[String],
) -> Result<Value> {
    let mut path = if path.starts_with('/') || path.starts_with("http") {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let c = match product {
        Product::Jira => cfg.jira()?,
        Product::Confluence => cfg.confluence()?,
        Product::Bitbucket => {
            let (c, ws) = cfg.bitbucket()?;
            if path.contains("{workspace}") {
                path = path.replace("{workspace}", bitbucket::workspace_or(None, ws.as_deref())?);
            }
            c
        }
    };
    let body = match data {
        None => None,
        Some(d) => {
            let text = match d.strip_prefix('@') {
                Some(file) => {
                    std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?
                }
                None => text_arg(d)?,
            };
            Some(serde_json::from_str::<Value>(&text).context("--data is not valid JSON")?)
        }
    };
    let query: Vec<(&str, &str)> = query
        .iter()
        .map(|kv| {
            kv.split_once('=')
                .with_context(|| format!("query '{kv}' must be KEY=VALUE"))
        })
        .collect::<Result<_>>()?;
    c.request(&method.to_ascii_uppercase(), &path, &query, body.as_ref())
}
