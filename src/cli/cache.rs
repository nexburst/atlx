//! `atlx cache path|clear`: the metadata cache (see [`crate::cache`]).

use crate::cache::{self, Cache, Table};
use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

#[derive(Subcommand)]
pub(crate) enum CacheCmd {
    /// Show where the metadata cache lives, whether it is on, and which
    /// tables it may hold
    Path,
    /// Delete every cached table (they are fetched again when needed)
    Clear,
}

pub(crate) fn run(cmd: CacheCmd) -> Result<Value> {
    let c = Cache::global();
    Ok(match cmd {
        CacheCmd::Path => json!({
            "path": cache::root().map(|p| p.display().to_string()),
            "enabled": cache::enabled(),
            "ttlSeconds": cache::TTL.as_secs(),
            "tables": Table::ALL.iter().map(|t| t.name()).collect::<Vec<_>>(),
        }),
        CacheCmd::Clear => {
            // Clear even when lookups are off (`--no-cache`): that flag only
            // stops reads and writes.
            let root = cache::root();
            let removed = match &root {
                Some(r) if c.root().is_none() => Cache::at(r, cache::TTL).clear()?,
                _ => c.clear()?,
            };
            json!({"path": root.map(|p| p.display().to_string()), "removed": removed})
        }
    })
}
