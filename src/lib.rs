pub mod bitbucket;
pub mod cache;
pub mod cli;
pub mod client;
pub mod config;
pub mod confluence;
mod files;
pub mod jira;
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
