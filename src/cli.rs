//! Command-line surface.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "plaud",
    version,
    about = "CLI client for Plaud recordings via the Plaud MCP server"
)]
pub struct Cli {
    /// Emit raw JSON (jq-friendly) instead of formatted output.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Sign in to Plaud via browser OAuth.
    Login,
    /// Sign out and remove the stored tokens.
    Logout,
    /// Show the current Plaud account.
    Whoami,
    /// List recordings.
    List(ListArgs),
    /// Show full details for a single recording.
    Get {
        /// Recording id.
        id: String,
    },
    /// Show the AI note (summary, action items, key topics).
    Note {
        /// Recording id.
        id: String,
    },
    /// Show the transcript (timestamps + speaker labels).
    Transcript {
        /// Recording id.
        id: String,
    },
    /// Download a recording's audio via its 24h presigned URL.
    Download {
        /// Recording id.
        id: String,
        /// Output path (default: <id>.mp3).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[derive(Args)]
pub struct ListArgs {
    /// Keyword search.
    #[arg(short, long)]
    pub query: Option<String>,
    /// Earliest date, YYYY-MM-DD.
    #[arg(long)]
    pub from: Option<String>,
    /// Latest date, YYYY-MM-DD.
    #[arg(long)]
    pub to: Option<String>,
    /// Page number (1-based).
    #[arg(long)]
    pub page: Option<u32>,
    /// Results per page.
    #[arg(long = "page-size")]
    pub page_size: Option<u32>,
}
