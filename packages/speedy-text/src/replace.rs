use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::config;
use crate::db;
use crate::indexer;
use crate::tokenize::SearchType;

/// Default ceiling on how many occurrences a single non-dry-run replace may touch
/// before it refuses to proceed. Override per call with `force`.
pub const REPLACE_SAFETY_LIMIT: usize = 500;

#[derive(Debug, Serialize)]
pub struct ReplaceResult {
    pub symbol: String,
    pub search_type: String,
    pub replacement: String,
    pub files_changed: usize,
    pub occurrences_replaced: usize,
    /// Sub-token spans skipped because of `whole_token_only` (e.g. `Dummy` inside `FooDummyBar`).
    pub sub_tokens_skipped: usize,
    pub dry_run: bool,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Serialize)]
pub struct SkippedFile {
    pub file: String,
    pub reason: String,
}

pub struct ReplaceArgs<'a> {
    pub symbol: &'a str,
    pub replacement: &'a str,
    pub search_type: SearchType,
    pub ext_filter: Option<&'a str>,
    pub ignore_case: bool,
    pub dry_run: bool,
    /// Skip spans that are part of a larger token (camelCase or `_`/`-` sub-tokens).
    pub whole_token_only: bool,
    /// Bypass `REPLACE_SAFETY_LIMIT` for intentionally large replaces.
    pub force: bool,
    /// Restrict the replace to these files (root-relative or absolute paths).
    /// `None` (or empty) means every indexed file.
    pub files: Option<&'a [String]>,
}

/// A file ready to be written: its absolute path and the fully rewritten content,
/// together with how many spans were applied.
struct Plan {
    abs_path: PathBuf,
    new_content: String,
    span_count: usize,
}

/// Replace all occurrences of `args.symbol` in the indexed files, honouring the same
/// search-type expansion as `db::query_occurrences`.
///
/// Algorithm:
/// 1. For every file, read content and verify each span against the on-disk text
///    (anti-stale check); optionally drop sub-token spans (`whole_token_only`).
/// 2. Rewrite each file in memory applying spans end → start so byte offsets stay valid.
/// 3. Enforce the safety limit on the total occurrence count (unless `force`/`dry_run`).
/// 4. Write atomically (sibling `.tmp` file + rename) and re-index via `update_file`.
pub fn run_replace(conn: &mut Connection, root: &Path, args: &ReplaceArgs) -> Result<ReplaceResult> {
    let occurrences = db::query_occurrences(conn, args.symbol, &args.search_type, args.ext_filter, args.ignore_case)?;

    // Optional path allow-list: normalise the caller-supplied paths the same way the
    // index stores them (root-relative, forward slashes) so comparison is exact.
    let allowed_files: Option<std::collections::HashSet<String>> = match args.files {
        Some(fs) if !fs.is_empty() => Some(
            fs.iter()
                .map(|f| config::normalize_path(root, Path::new(f)))
                .collect(),
        ),
        _ => None,
    };

    // Group by normalized file path (BTreeMap → deterministic ordering).
    let mut by_file: std::collections::BTreeMap<String, Vec<db::Occurrence>> = Default::default();
    for occ in occurrences {
        if let Some(ref allow) = allowed_files {
            if !allow.contains(&occ.file) {
                continue;
            }
        }
        by_file.entry(occ.file.clone()).or_default().push(occ);
    }

    let mut skipped: Vec<SkippedFile> = Vec::new();
    let mut sub_tokens_skipped = 0usize;
    let mut plans: Vec<Plan> = Vec::new();

    // ── Phase 1: plan every file (read, stale-check, whole-token filter, rewrite in memory) ──
    for (norm_path, spans) in by_file {
        let abs_path = denormalize_path(root, &norm_path);

        let content_bytes = match std::fs::read(&abs_path) {
            Ok(b) => b,
            Err(e) => {
                skipped.push(SkippedFile { file: norm_path, reason: format!("cannot read: {e}") });
                continue;
            }
        };

        let content = match std::str::from_utf8(&content_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                skipped.push(SkippedFile { file: norm_path, reason: "non-UTF-8 content".to_string() });
                continue;
            }
        };

        // Use split('\n') so join('\n') faithfully reconstructs the original layout.
        let lines: Vec<&str> = content.split('\n').collect();

        // Anti-stale check + whole-token filter in one pass. Any out-of-bounds or
        // mismatching span means the index is stale → skip the whole file.
        let mut stale_reason: Option<String> = None;
        let mut applicable: Vec<&db::Occurrence> = Vec::new();
        'stale: for span in &spans {
            let line_idx = (span.line as usize).saturating_sub(1);
            if line_idx >= lines.len() {
                stale_reason = Some(format!(
                    "stale-index: line {} out of bounds (file has {} lines; run sync)",
                    span.line,
                    lines.len()
                ));
                break 'stale;
            }
            let line_bytes = lines[line_idx].as_bytes();
            let start = span.col_start as usize;
            let end = span.col_end as usize;
            if start > end || end > line_bytes.len() {
                stale_reason = Some(format!(
                    "stale-index: span [{start},{end}) out of bounds in line {} (run sync)",
                    span.line
                ));
                break 'stale;
            }
            let span_text = match std::str::from_utf8(&line_bytes[start..end]) {
                Ok(s) => s,
                Err(_) => {
                    stale_reason = Some(format!(
                        "stale-index: span [{start},{end}) in line {} is not valid UTF-8 (run sync)",
                        span.line
                    ));
                    break 'stale;
                }
            };
            let matches = if args.ignore_case {
                span_text.eq_ignore_ascii_case(args.symbol)
            } else {
                span_text == args.symbol
            };
            if !matches {
                stale_reason = Some(format!(
                    "stale-index: expected {:?} at line {}:[{start},{end}), found {:?} (run sync)",
                    args.symbol, span.line, span_text
                ));
                break 'stale;
            }

            // whole_token_only: a sub-token has a token byte ([a-zA-Z0-9_-]) immediately
            // before its start or after its end (covers camelCase and `_`/`-` splits).
            if args.whole_token_only {
                let prev_is_token = start > 0 && is_token_byte(line_bytes[start - 1]);
                let next_is_token = end < line_bytes.len() && is_token_byte(line_bytes[end]);
                if prev_is_token || next_is_token {
                    sub_tokens_skipped += 1;
                    continue;
                }
            }

            applicable.push(span);
        }

        if let Some(reason) = stale_reason {
            skipped.push(SkippedFile { file: norm_path, reason });
            continue;
        }

        if applicable.is_empty() {
            continue; // every span filtered out by whole_token_only → nothing to do
        }

        // Apply end → start so applying span N doesn't shift the offsets of span N-1.
        applicable.sort_by(|a, b| b.line.cmp(&a.line).then(b.col_start.cmp(&a.col_start)));
        let span_count = applicable.len();

        let mut lines_owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        let mut abort_reason: Option<String> = None;

        for span in &applicable {
            let line_idx = (span.line as usize).saturating_sub(1);
            let line = &lines_owned[line_idx];
            let start = span.col_start as usize;
            let end = span.col_end as usize;

            if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                abort_reason = Some(format!(
                    "span [{start},{end}) is not on a UTF-8 char boundary in line {}",
                    span.line
                ));
                break;
            }

            let mut new_line = line[..start].to_string();
            new_line.push_str(args.replacement);
            new_line.push_str(&line[end..]);
            lines_owned[line_idx] = new_line;
        }

        if let Some(reason) = abort_reason {
            skipped.push(SkippedFile { file: norm_path, reason });
            continue;
        }

        plans.push(Plan {
            abs_path,
            new_content: lines_owned.join("\n"),
            span_count,
        });
    }

    // ── Phase 2: safety limit on the total occurrence count ──
    let total: usize = plans.iter().map(|p| p.span_count).sum();
    if !args.dry_run && !args.force && total > REPLACE_SAFETY_LIMIT {
        anyhow::bail!(
            "refusing to replace {total} occurrences across {} files (safety limit {}); \
             re-run with force=true to override, or dry_run=true to preview",
            plans.len(),
            REPLACE_SAFETY_LIMIT
        );
    }

    // ── Phase 3: write + re-index ──
    let mut files_changed = 0usize;
    let mut occurrences_replaced = 0usize;

    for plan in &plans {
        if !args.dry_run {
            // Atomic write: write to a sibling temp file, then rename.
            let tmp_path = {
                let mut p = plan.abs_path.clone();
                let mut name = p.file_name().unwrap_or_default().to_os_string();
                name.push(".__speedy_tmp__");
                p.set_file_name(name);
                p
            };
            std::fs::write(&tmp_path, plan.new_content.as_bytes())
                .with_context(|| format!("write temp file {}", tmp_path.display()))?;
            std::fs::rename(&tmp_path, &plan.abs_path)
                .with_context(|| format!("rename {} → {}", tmp_path.display(), plan.abs_path.display()))?;

            // Re-index the modified file so the DB and HashRegistry are immediately consistent.
            indexer::update_file(conn, root, &plan.abs_path)?;
        }

        files_changed += 1;
        occurrences_replaced += plan.span_count;
    }

    Ok(ReplaceResult {
        symbol: args.symbol.to_string(),
        search_type: args.search_type.as_str().to_string(),
        replacement: args.replacement.to_string(),
        files_changed,
        occurrences_replaced,
        sub_tokens_skipped,
        dry_run: args.dry_run,
        skipped,
    })
}

/// Token char as defined by `tokenize`: `[a-zA-Z0-9_-]`.
#[inline]
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Reconstructs an OS-native absolute path from a forward-slash normalised relative path.
fn denormalize_path(root: &Path, norm: &str) -> std::path::PathBuf {
    let mut p = root.to_path_buf();
    for component in norm.split('/') {
        if !component.is_empty() {
            p.push(component);
        }
    }
    p
}
