//! `atlx user`: Atlassian users.

use crate::client::Client;
use crate::jira;
use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

#[derive(Subcommand)]
pub(crate) enum UserCmd {
    /// Search Jira users by name or email
    Search {
        #[arg(required = true)]
        query: Vec<String>,
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
}

pub(crate) fn user_cmd(c: &Client, cmd: UserCmd) -> Result<Value> {
    match cmd {
        UserCmd::Search { query, limit } => jira::user_search(c, &query.join(" "), limit),
    }
}
