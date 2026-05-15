use std::sync::OnceLock;

pub const RAW: &str = include_str!("../assets/default_ignores.txt");

static BINARY_EXTS: OnceLock<Vec<&'static str>> = OnceLock::new();

/// All non-comment, non-empty patterns (for FileFilter and .speedyignore generation).
pub fn patterns() -> Vec<&'static str> {
    RAW.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Directory names only (e.g. "node_modules" from "node_modules/").
/// Used by the daemon for WATCH_IGNORE_DIRS and workspace scan skip lists.
pub fn watch_dirs() -> Vec<&'static str> {
    RAW.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && l.ends_with('/') && !l.contains('*'))
        .map(|l| l.trim_end_matches('/'))
        .collect()
}

/// File extensions that are considered binary (e.g. "exe" from "*.exe").
/// Cached after first call.
pub fn binary_extensions() -> &'static [&'static str] {
    BINARY_EXTS.get_or_init(|| {
        RAW.lines()
            .map(str::trim)
            .filter(|l| l.starts_with("*.") && l.len() > 2)
            .map(|l| &l[2..])
            .collect()
    })
}
