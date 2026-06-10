//! `plaud` — a CLI client for Plaud recordings over the Plaud MCP server.

mod cli;
mod commands;
mod config;
mod mcp;
mod oauth;
mod output;

use clap::Parser;

#[tokio::main]
async fn main() {
    if let Err(err) = commands::dispatch(cli::Cli::parse()).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
