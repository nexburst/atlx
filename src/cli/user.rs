//! `atlx user`: Atlassian users.

use crate::client::{ApiError, Client};
use crate::jira;
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

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
        /// One short line per user in text output; JSON is unchanged
        #[arg(long)]
        compact: bool,
        #[arg(long, short = 'n', default_value_t = 20)]
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
            compact,
            limit,
        } => {
            let users = |request: Value, data: Value| {
                let n = data.as_array().map_or(0, Vec::len);
                envelope("user-search.query", "user-search")
                    .request(request)
                    .meta("resultCount", n)
                    .data(data)
            };
            if let Some(id) = account_id {
                let request = json!({"site": jira::site(c), "accountId": id, "compact": compact});
                return Ok(users(request, jira::user_by_account(c, &id)?));
            }
            // Jira's user search matches display names and email addresses
            // with the same `query` parameter.
            let name = name
                .or(query_flag)
                .or_else(|| (!query.is_empty()).then(|| query.join(" ")));
            let request = json!({
                "site": jira::site(c),
                "name": name,
                "email": email,
                "compact": compact,
                "includeSlack": false,
                "limit": limit,
            });
            let Some(text) = name.or(email) else {
                return Err(ApiError::usage("pass a name, --name, --email or --account-id").into());
            };
            Ok(users(request, jira::user_search(c, &text, limit)?))
        }
    }
}
