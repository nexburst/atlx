//! `atlx search`: search Jira, Confluence and Bitbucket at once.

use super::Globals;
use crate::{Product, search};
use anyhow::Result;
use clap::Args;
use serde_json::Value;

// The command's help text lives on the `Cmd::Search` variant.
#[derive(Args)]
pub(crate) struct SearchArgs {
    query: String,
    /// Max results per product
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// Only search these products (comma-separated)
    #[arg(long, value_enum, value_delimiter = ',')]
    only: Vec<Product>,
}

pub(crate) fn run(g: &Globals, a: SearchArgs) -> Result<Value> {
    Ok(search::run(&g.config()?, &a.query, a.limit, &a.only))
}
