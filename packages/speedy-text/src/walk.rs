use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ignore::build_walker;

/// Returns (absolute_path, lowercase_ext_without_dot) for every file under `root`
/// whose extension is in `allowed_exts`, respecting gitignore + .speedyignore rules.
/// `.speedy/` (where the index DB lives) and `.git/` directories are always
/// skipped — the former so the indexer never walks its own database or loops.
pub fn walk(root: &Path, allowed_exts: &HashSet<String>) -> Result<Vec<(PathBuf, String)>> {
    let walker = build_walker(root)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            name != ".speedy" && name != ".git"
        })
        .build();

    let mut results = Vec::new();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[speedy-text] walk error: {}", e);
                continue;
            }
        };

        if entry.file_type().map(|ft| !ft.is_file()).unwrap_or(true) {
            continue;
        }

        let path = entry.into_path();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if !allowed_exts.contains(ext.as_str()) {
            continue;
        }

        results.push((path, ext));
    }

    Ok(results)
}
