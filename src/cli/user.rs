//! `atlx user`: Atlassian users.

use crate::client::{ApiError, Client};
use crate::jira;
use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

#[derive(Subcommand)]
pub(crate) enum UserCmd {
    /// Search Jira users by name or email
    Search {
        /// Name or email text
        #[arg(value_name = "name")]
        query: Vec<String>,
        /// Full or partial display name
        #[arg(long)]
        name: Option<String>,
        /// Same as --name
        #[arg(long = "query")]
        query_flag: Option<String>,
        /// Full email address or its start
        #[arg(long)]
        email: Option<String>,
        /// Look up one user by account ID or identity ARI
        #[arg(long, conflicts_with_all = ["query", "name", "query_flag", "email"])]
        account_id: Option<String>,
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
    },
}

pub(crate) fn user_cmd(c: &Client, cmd: UserCmd) -> Result<Value> {
    match cmd {
        UserCmd::Search {
            query,
            name,
            query_flag,
            email,
            account_id,
            limit,
        } => {
            if let Some(id) = account_id {
                return jira::user_by_account(c, &id);
            }
            // Jira's user search matches display names and email addresses
            // with the same `query` parameter.
            let text = name
                .or(query_flag)
                .or(email)
                .or_else(|| (!query.is_empty()).then(|| query.join(" ")));
            let Some(text) = text else {
                return Err(ApiError::usage("pass a name, --name, --email or --account-id").into());
            };
            jira::user_search(c, &text, limit)
        }
    }
}
