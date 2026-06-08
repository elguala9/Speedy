use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn find_root(start: &Path) -> PathBuf {
    start.to_path_buf()
}

pub fn db_path(root: &Path) -> PathBuf {
    // Shares the per-workspace `.speedy/` dir with the other contexts.
    // `speedy_subdir` is idempotent, so a `root` that already points at a
    // `.speedy/` dir does not produce a nested `.speedy/.speedy/`.
    speedy_core::daemon_util::speedy_subdir(root).join("index.db")
}

pub fn extensions_path(root: &Path) -> PathBuf {
    root.join(".speedyextensions")
}

/// Relative path from root, always using forward slashes.
pub fn normalize_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Reads `.speedyextensions` from `root`. Creates the file with defaults if absent.
/// Returns a set of lowercase extensions without leading dot (e.g. `"md"`, `"rs"`).
pub fn load_or_create_extensions(root: &Path) -> Result<HashSet<String>> {
    let path = extensions_path(root);
    if !path.exists() {
        std::fs::write(&path, DEFAULT_EXTENSIONS)?;
        eprintln!(
            "[speedy-text] created default .speedyextensions at {}",
            path.display()
        );
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(parse_extensions(&content))
}

fn parse_extensions(content: &str) -> HashSet<String> {
    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.trim_start_matches('.').to_lowercase())
        .collect()
}

const DEFAULT_EXTENSIONS: &str = "\
# speedy-text: file extensions to index.
# One extension per line, without the leading dot.
# Lines starting with # are comments.
#
# Text / documentation
md
txt
rst
adoc
#
# Config
toml
yaml
yml
ini
cfg
conf
env
#
# Database / query
sql
";
