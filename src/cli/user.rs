//! `atlx jira user`: Atlassian accounts, looked up through Jira.
//!
//! The same commands are mounted at the top-level `atlx user` until the
//! command line drops that group (it is owned by `cli/mod.rs`).

use super::jira::{Many, Paging, listed};
use crate::client::Client;
use crate::jira;
use crate::schema::{CommandMeta, Effect};
use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

#[derive(Subcommand)]
pub(crate) enum UserCmd {
    /// Accounts whose name or email matches
    Search {
        /// Name or email text
        #[arg(required = true)]
        text: Vec<String>,
        #[command(flatten)]
        paging: Paging,
    },
    /// Accounts by `me`, account ID, identity ARI or exact email
    Get {
        /// Users (`-` reads them from stdin)
        #[arg(required = true)]
        users: Vec<String>,
        #[command(flatten)]
        many: Many,
    },
}

/// Transitional: the top-level `atlx user` copies of `jira user`.
pub(crate) const META: &[CommandMeta] = &[
    CommandMeta::new("user search", Effect::Read)
        .record(&jira::records::USER)
        .examples(&["atlx user search alice"]),
    CommandMeta::new("user get", Effect::Read)
        .record(&jira::records::USER)
        .examples(&["atlx user get me"]),
];

pub(crate) fn user_cmd(c: &Client, cmd: UserCmd) -> Result<Value> {
    match cmd {
        UserCmd::Search { text, paging } => Ok(listed(jira::user_search(
            c,
            &text.join(" "),
            paging.limit(),
            paging.cursor(),
        )?)),
        UserCmd::Get { users, many } => many.run(users, |u| jira::user_get(c, u)),
    }
}
