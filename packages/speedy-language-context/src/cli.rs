//! CLI surface — thin clap derive over the library modules.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "speedy-language-context",
    version,
    about = "Local code intelligence — symbol graph + MCP"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(
        global = true,
        short = 'p',
        long = "path",
        help = "Workspace root (default: current dir)"
    )]
    pub workspace_path: Option<PathBuf>,

    #[arg(global = true, long, help = "Output in JSON format")]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Full-index the workspace")]
    Index,

    #[command(about = "Incrementally update index for specific files")]
    Update { files: Vec<PathBuf> },

    #[command(about = "Show index status")]
    Status,

    #[command(about = "Start MCP server on stdio")]
    Serve,

    #[command(about = "Print skeleton for files")]
    Skeleton {
        files: Vec<String>,
        #[arg(long, default_value = "standard")]
        detail: String,
    },

    #[command(about = "Search the symbol graph")]
    Search {
        query: String,
        #[arg(short = 'k', long, default_value = "10")]
        top_k: usize,
    },
}
