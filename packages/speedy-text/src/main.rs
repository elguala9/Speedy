use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use speedy_text::replace::{ReplaceArgs, run_replace};
use speedy_text::tokenize::SearchType;
use speedy_text::{config, db, indexer, query};

#[derive(Parser)]
#[command(name = "speedy-text-context", about = "Index and query text symbol occurrences in a repo")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Delete the index and re-index everything from scratch
    Index {
        path: PathBuf,
    },
    /// Incremental update: add/update modified files, remove deleted files
    Sync {
        path: PathBuf,
    },
    /// Re-index a single file (used by the daemon file watcher)
    Update {
        /// Workspace root
        path: PathBuf,
        /// File that changed (absolute or relative to root)
        file: PathBuf,
    },
    /// Query the index for a symbol
    Query {
        path: PathBuf,
        symbol: String,
        /// Search type (default: cased — finds cased, isolated_special and isolated)
        #[arg(long, value_enum)]
        r#type: Option<SearchTypeArg>,
        /// Filter by file extension (e.g. md)
        #[arg(long)]
        ext: Option<String>,
        /// Case-insensitive search
        #[arg(long)]
        ignore_case: bool,
    },
    /// Replace occurrences of a symbol and re-index the changed files
    Replace {
        path: PathBuf,
        symbol: String,
        replacement: String,
        /// Search type (default: cased)
        #[arg(long, value_enum)]
        r#type: Option<SearchTypeArg>,
        /// Filter by file extension (e.g. md)
        #[arg(long)]
        ext: Option<String>,
        /// Case-insensitive match (replacement is written literally)
        #[arg(long)]
        ignore_case: bool,
        /// Skip sub-token matches inside a larger token (e.g. 'Dummy' in 'FooDummyBar')
        #[arg(long)]
        whole_token_only: bool,
        /// Bypass the safety limit on large replaces
        #[arg(long)]
        force: bool,
        /// Restrict the replace to these root-relative paths (repeatable)
        #[arg(long)]
        files: Vec<String>,
        /// Preview changes without writing files or updating the index
        #[arg(long)]
        dry_run: bool,
    },
    /// Show index statistics
    Status {
        path: PathBuf,
    },
}

#[derive(Clone, ValueEnum)]
enum SearchTypeArg {
    Cased,
    Isolated,
    #[value(name = "isolated_special")]
    IsolatedSpecial,
}

impl From<SearchTypeArg> for SearchType {
    fn from(a: SearchTypeArg) -> Self {
        match a {
            SearchTypeArg::Cased => SearchType::Cased,
            SearchTypeArg::Isolated => SearchType::Isolated,
            SearchTypeArg::IsolatedSpecial => SearchType::IsolatedSpecial,
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("[speedy-text] error: {:?}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path } => {
            let root = config::find_root(&path);
            let db_path = config::db_path(&root);
            let mut conn = db::open(&db_path)
                .with_context(|| format!("cannot open DB at {}", db_path.display()))?;
            db::migrate(&conn)?;
            let (indexed, total) = indexer::index(&mut conn, &root)?;
            println!(
                "Indexed {indexed}/{total} files into {}",
                db_path.display()
            );
        }

        Commands::Sync { path } => {
            let root = config::find_root(&path);
            let db_path = config::db_path(&root);
            let mut conn = db::open(&db_path)
                .with_context(|| format!("cannot open DB at {}", db_path.display()))?;
            db::migrate(&conn)?;
            let (updated, removed) = indexer::sync(&mut conn, &root)?;
            println!("Synced: {updated} updated, {removed} removed");
        }

        Commands::Update { path, file } => {
            let root = config::find_root(&path);
            let db_path = config::db_path(&root);
            let mut conn = db::open(&db_path)
                .with_context(|| format!("cannot open DB at {}", db_path.display()))?;
            db::migrate(&conn)?;
            indexer::update_file(&mut conn, &root, &file)?;
        }

        Commands::Query {
            path,
            symbol,
            r#type,
            ext,
            ignore_case,
        } => {
            let root = config::find_root(&path);
            let db_path = config::db_path(&root);
            let conn = db::open(&db_path)
                .with_context(|| format!("cannot open DB at {}", db_path.display()))?;
            db::migrate(&conn)?;
            let search_type: SearchType = r#type.map(Into::into).unwrap_or(SearchType::Cased);
            query::run_query(&conn, &symbol, &search_type, ext.as_deref(), ignore_case)?;
        }

        Commands::Replace { path, symbol, replacement, r#type, ext, ignore_case, whole_token_only, force, files, dry_run } => {
            let root = config::find_root(&path);
            let db_path = config::db_path(&root);
            let mut conn = db::open(&db_path)
                .with_context(|| format!("cannot open DB at {}", db_path.display()))?;
            db::migrate(&conn)?;
            let search_type: SearchType = r#type.map(Into::into).unwrap_or(SearchType::Cased);
            let result = run_replace(&mut conn, &root, &ReplaceArgs {
                symbol: &symbol,
                replacement: &replacement,
                search_type,
                ext_filter: ext.as_deref(),
                ignore_case,
                dry_run,
                whole_token_only,
                force,
                files: if files.is_empty() { None } else { Some(&files) },
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        Commands::Status { path } => {
            let root = config::find_root(&path);
            let db_path = config::db_path(&root);
            let conn = db::open(&db_path)
                .with_context(|| format!("cannot open DB at {}", db_path.display()))?;
            db::migrate(&conn)?;
            let status = db::get_status(&conn, &db_path)?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
    }

    Ok(())
}
