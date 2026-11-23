//! `atlx user`: Atlassian users.

use crate::client::{ApiError, Client};
use crate::jira::{self, bulk::Who};
use crate::output::envelope::envelope;
use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};

/// How positional identifiers are read.
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum InputType {
    Auto,
    Email,
    AccountId,
}

pub(crate) const META: &[crate::schema::CommandMeta] = &[];

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
    /// One user's profile (yours by default)
    Get {
        /// Account ID or identity ARI
        #[arg(conflicts_with = "account_id")]
        account: Option<String>,
        /// Account ID or identity ARI
        #[arg(long)]
        account_id: Option<String>,
    },
    /// Look up many users by account ID or email
    BulkLookup {
        /// Account IDs, identity ARIs or emails
        identifiers: Vec<String>,
        /// An account ID or ARI; repeatable
        #[arg(long)]
        account_id: Vec<String>,
        /// An exact email; repeatable
        #[arg(long)]
        email: Vec<String>,
        /// How to read the positional values
        #[arg(long, value_enum, default_value = "auto")]
        input_type: InputType,
        /// Exit with an error unless every user is found
        #[arg(long)]
        strict: bool,
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
        UserCmd::Get {
            account,
            account_id,
        } => {
            let id = account_id.or(account);
            Ok(envelope("user.get", "user")
                .meta("resultCount", 1)
                .request(json!({"site": jira::site(c), "accountId": id}))
                .data(jira::bulk::user(c, id.as_deref())?))
        }
        UserCmd::BulkLookup {
            identifiers,
            account_id,
            email,
            input_type,
            strict,
        } => {
            let given: Vec<&String> = identifiers
                .iter()
                .chain(&account_id)
                .chain(&email)
                .collect();
            let request = json!({
                "site": null,
                "identifiers": given,
                "inputType": match input_type {
                    InputType::Auto => "auto",
                    InputType::Email => "email",
                    InputType::AccountId => "account-id",
                },
            });
            let who: Vec<Who> = identifiers
                .into_iter()
                .map(|s| match input_type {
                    InputType::Email => Who::Email(s),
                    InputType::Auto if s.contains('@') => Who::Email(s),
                    _ => Who::Account(s),
                })
                .chain(account_id.into_iter().map(Who::Account))
                .chain(email.into_iter().map(Who::Email))
                .collect();
            let data = jira::bulk::lookup(c, &who)?;
            let items = data["items"].as_array().map_or(&[][..], Vec::as_slice);
            let missed = items.iter().filter(|i| i["ok"] != true).count();
            // As twg: print the result either way, and fail only when nobody
            // was found, or anyone wasn't under --strict.
            if missed > 0 && (strict || missed == items.len()) {
                crate::cli::util::mark_failed();
            }
            let found = items.len() - missed;
            // Not `data()` with the batch: user lookups have no acquisition.
            let mut out = envelope("user.bulk-lookup", "user")
                .request(request)
                .meta("resultCount", found)
                .data(Value::Null);
            out["data"] = data;
            Ok(out)
        }
    }
}
