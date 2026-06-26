//! Command-line surface.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "plaud",
    version,
    about = "CLI client for Plaud recordings via the Plaud MCP server"
)]
pub struct Cli {
    /// Emit normalized JSON (stable schema; jq-friendly).
    #[arg(long, global = true)]
    pub json: bool,

    /// Emit newline-delimited JSON (one record per line) where applicable.
    #[arg(long, global = true)]
    pub ndjson: bool,

    /// Emit the raw, unprocessed Plaud tool payload (debugging).
    #[arg(long, global = true)]
    pub raw: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Resolved output mode, precedence raw > ndjson > json > human.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    Json,
    Ndjson,
    Raw,
}

impl OutputMode {
    /// Resolve flags to a mode. Precedence: raw > ndjson > json > human.
    pub fn resolve(raw: bool, ndjson: bool, json: bool) -> OutputMode {
        if raw {
            OutputMode::Raw
        } else if ndjson {
            OutputMode::Ndjson
        } else if json {
            OutputMode::Json
        } else {
            OutputMode::Human
        }
    }
}

impl Cli {
    pub fn output_mode(&self) -> OutputMode {
        OutputMode::resolve(self.raw, self.ndjson, self.json)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    /// One Markdown file per recording, with YAML frontmatter.
    Md,
    /// One JSON file per recording (normalized record).
    Json,
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
    /// Pull recordings into the local store (transcript + notes + actions).
    Sync(SyncArgs),
    /// Full-text search across synced transcripts, notes, and titles.
    Search(SearchArgs),
    /// Export synced records to files (Markdown or JSON).
    Export(ExportArgs),
    /// Print the stable JSON schema emitted by --json.
    Schema,
}

#[derive(Args)]
pub struct ListArgs {
    /// Keyword search over titles.
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
    /// Results per page (Plaud requires >= 10).
    #[arg(long = "page-size")]
    pub page_size: Option<u32>,
}

#[derive(Args)]
pub struct SyncArgs {
    /// Only sync recordings uploaded on/after this date (YYYY-MM-DD).
    #[arg(long)]
    pub since: Option<String>,
    /// Only sync recordings newer than the last sync cursor.
    #[arg(long = "since-last")]
    pub since_last: bool,
    /// Re-sync records already in the store.
    #[arg(long)]
    pub force: bool,
    /// Stop after syncing at most N recordings.
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args)]
pub struct SearchArgs {
    /// Text to search for (case-insensitive).
    pub query: String,
    /// Lines of transcript context to show around each match.
    #[arg(long, default_value_t = 1)]
    pub context: usize,
}

#[derive(Args)]
pub struct ExportArgs {
    /// Output directory (created if missing).
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ExportFormat::Md)]
    pub format: ExportFormat,
    /// Export only these recording ids (default: all synced records).
    pub ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn output_mode_precedence() {
        // raw wins over everything.
        assert_eq!(OutputMode::resolve(true, true, true), OutputMode::Raw);
        // ndjson beats json.
        assert_eq!(OutputMode::resolve(false, true, true), OutputMode::Ndjson);
        assert_eq!(OutputMode::resolve(false, false, true), OutputMode::Json);
        assert_eq!(OutputMode::resolve(false, false, false), OutputMode::Human);
    }

    #[test]
    fn cli_definition_is_valid() {
        // Catches clap config errors (duplicate args, bad defaults) at test time.
        Cli::command().debug_assert();
    }
}
