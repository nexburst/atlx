// Product modules still call the envelope's transition shims
// (`request`, `meta`); they drop the calls, then these `allow`s go.
#[allow(deprecated)]
pub mod bitbucket;
pub mod cache;
pub mod cli;
pub mod client;
pub mod config;
#[allow(deprecated)]
pub mod confluence;
mod files;
#[allow(deprecated)]
pub mod jira;
#[allow(deprecated)]
pub mod jsm;
pub mod markup;
pub mod output;
pub mod paging;
pub mod schema;
pub mod search;
pub mod time;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Product {
    Jira,
    Confluence,
    Bitbucket,
}
