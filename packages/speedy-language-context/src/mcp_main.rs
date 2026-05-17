use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

use speedy_language_context::mcp;

#[derive(Parser)]
#[command(name = "speedy-language-context-mcp", about = "MCP server for Speedy code intelligence")]
struct Cli {
    /// Workspace root (defaults to current directory)
    #[arg(long, short = 'w')]
    workspace: Option<PathBuf>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let root = match cli.workspace {
        Some(p) => p.canonicalize().or_else(|_| Ok::<_, anyhow::Error>(p))?,
        None => {
            let p = std::env::current_dir().context("getting current dir")?;
            p.canonicalize().or_else(|_| Ok::<PathBuf, anyhow::Error>(p))?
        }
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(mcp::run_server(root))
}
