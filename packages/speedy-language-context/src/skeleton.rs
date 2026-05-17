//! Compact textual skeletons of files for LLM consumption.

use anyhow::Result;
use std::fmt::Write;
use std::path::Path;
use std::str::FromStr;

use crate::graph::{GraphStore, Symbol};

#[derive(Debug, Clone, Copy)]
pub enum DetailLevel {
    Minimal,
    Standard,
    Detailed,
}

impl FromStr for DetailLevel {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "minimal" => Ok(DetailLevel::Minimal),
            "standard" => Ok(DetailLevel::Standard),
            "detailed" => Ok(DetailLevel::Detailed),
            other => Err(anyhow::anyhow!("unknown detail level: {other}")),
        }
    }
}

/// Generate skeleton text for the given workspace-relative file paths.
/// `workspace_root` is used to build absolute paths for `--detailed` body reads.
pub fn get_skeleton(
    store: &GraphStore,
    workspace_root: &Path,
    files: &[&str],
    detail: DetailLevel,
) -> Result<String> {
    let mut out = String::new();
    for file in files {
        let syms = store.get_symbols_for_file(file)?;
        if syms.is_empty() {
            writeln!(out, "── {file} (no indexed symbols) ──")?;
            continue;
        }
        writeln!(out, "── {file} ──")?;
        let abs_path = workspace_root.join(file);
        for sym in &syms {
            render_symbol(&mut out, sym, detail, &abs_path)?;
        }
        writeln!(out)?;
    }
    Ok(out)
}

fn render_symbol(
    out: &mut String,
    sym: &Symbol,
    detail: DetailLevel,
    abs_path: &Path,
) -> std::fmt::Result {
    match detail {
        DetailLevel::Minimal => {
            if !sym.is_public {
                return Ok(());
            }
            writeln!(out, "[{}] {} — {}", sym.kind, sym.name, oneline(&sym.signature))
        }
        DetailLevel::Standard => writeln!(
            out,
            "[{}] {} (line {}-{}) — {}",
            sym.kind,
            sym.name,
            sym.start_line + 1,
            sym.end_line + 1,
            oneline(&sym.signature),
        ),
        DetailLevel::Detailed => {
            writeln!(
                out,
                "[{}] {} (line {}-{}) — {}",
                sym.kind,
                sym.name,
                sym.start_line + 1,
                sym.end_line + 1,
                oneline(&sym.signature),
            )?;
            let lines = sym.end_line.saturating_sub(sym.start_line) + 1;
            if lines <= 30 {
                if let Ok(body) = read_lines(abs_path, sym.start_line, sym.end_line) {
                    for line in body.lines() {
                        writeln!(out, "    {line}")?;
                    }
                }
            }
            Ok(())
        }
    }
}

fn oneline(s: &str) -> String {
    s.replace('\n', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_lines(abs_path: &Path, start: u32, end: u32) -> std::io::Result<String> {
    let content = std::fs::read_to_string(abs_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let s = start as usize;
    let e = (end as usize + 1).min(lines.len());
    if s >= lines.len() {
        return Ok(String::new());
    }
    Ok(lines[s..e].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GraphStore, SymbolKind};
    use crate::parser::ParsedSymbol;
    use tempfile::tempdir;

    fn sym(name: &str, start: u32, end: u32, is_public: bool, kind: SymbolKind) -> ParsedSymbol {
        ParsedSymbol {
            kind,
            name: name.to_string(),
            start_line: start,
            end_line: end,
            signature: format!("fn {}()", name),
            is_public,
        }
    }

    // ── DetailLevel parsing ──────────────────────────────────────────────────

    #[test]
    fn test_detail_level_from_str_all_variants() {
        assert!(matches!("minimal".parse::<DetailLevel>().unwrap(), DetailLevel::Minimal));
        assert!(matches!("standard".parse::<DetailLevel>().unwrap(), DetailLevel::Standard));
        assert!(matches!("detailed".parse::<DetailLevel>().unwrap(), DetailLevel::Detailed));
    }

    #[test]
    fn test_detail_level_from_str_case_insensitive() {
        assert!(matches!("MINIMAL".parse::<DetailLevel>().unwrap(), DetailLevel::Minimal));
        assert!(matches!("Standard".parse::<DetailLevel>().unwrap(), DetailLevel::Standard));
        assert!(matches!("DETAILED".parse::<DetailLevel>().unwrap(), DetailLevel::Detailed));
    }

    #[test]
    fn test_detail_level_from_str_invalid_errors() {
        assert!("unknown".parse::<DetailLevel>().is_err());
        assert!("".parse::<DetailLevel>().is_err());
        assert!("max".parse::<DetailLevel>().is_err());
    }

    // ── get_skeleton ─────────────────────────────────────────────────────────

    fn setup_store(dir: &tempfile::TempDir, file: &str) -> GraphStore {
        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file(file, 0, "hash").unwrap();
        store
            .insert_symbol(fid, &sym("public_fn", 0, 5, true, SymbolKind::Function))
            .unwrap();
        store
            .insert_symbol(fid, &sym("private_fn", 10, 15, false, SymbolKind::Function))
            .unwrap();
        store
            .insert_symbol(fid, &sym("MyStruct", 20, 25, true, SymbolKind::Struct))
            .unwrap();
        store
    }

    #[test]
    fn test_get_skeleton_no_symbols_returns_placeholder() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let out = get_skeleton(&store, dir.path(), &["missing.rs"], DetailLevel::Standard).unwrap();
        assert!(
            out.contains("no indexed symbols"),
            "expected placeholder for unindexed file: {out}"
        );
    }

    #[test]
    fn test_get_skeleton_minimal_only_shows_public() {
        let dir = tempdir().unwrap();
        let store = setup_store(&dir, "src/lib.rs");
        let out = get_skeleton(&store, dir.path(), &["src/lib.rs"], DetailLevel::Minimal).unwrap();
        assert!(out.contains("public_fn"), "public symbol must appear in minimal");
        assert!(!out.contains("private_fn"), "private symbol must be hidden in minimal");
        assert!(out.contains("MyStruct"), "public struct must appear");
    }

    #[test]
    fn test_get_skeleton_standard_shows_all_with_line_numbers() {
        let dir = tempdir().unwrap();
        let store = setup_store(&dir, "src/lib.rs");
        let out = get_skeleton(&store, dir.path(), &["src/lib.rs"], DetailLevel::Standard).unwrap();
        assert!(out.contains("public_fn"), "public function must appear");
        assert!(out.contains("private_fn"), "private function must appear in standard");
        assert!(out.contains("line"), "standard output should contain line references");
    }

    #[test]
    fn test_get_skeleton_multiple_files() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let fa = store.upsert_file("a.rs", 0, "h").unwrap();
        let fb = store.upsert_file("b.rs", 0, "h").unwrap();
        store
            .insert_symbol(fa, &sym("fn_a", 0, 3, true, SymbolKind::Function))
            .unwrap();
        store
            .insert_symbol(fb, &sym("fn_b", 0, 3, true, SymbolKind::Function))
            .unwrap();

        let out =
            get_skeleton(&store, dir.path(), &["a.rs", "b.rs"], DetailLevel::Standard).unwrap();
        assert!(out.contains("a.rs"), "first file header must appear");
        assert!(out.contains("b.rs"), "second file header must appear");
        assert!(out.contains("fn_a"));
        assert!(out.contains("fn_b"));
    }

    #[test]
    fn test_get_skeleton_detailed_reads_actual_file_body() {
        let dir = tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("main.rs"),
            b"pub fn hello() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();

        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file("src/main.rs", 0, "h").unwrap();
        store
            .insert_symbol(fid, &sym("hello", 0, 2, true, SymbolKind::Function))
            .unwrap();

        let out =
            get_skeleton(&store, dir.path(), &["src/main.rs"], DetailLevel::Detailed).unwrap();
        assert!(out.contains("hello"), "function name must appear");
        assert!(
            out.contains("println"),
            "body lines should be included in detailed mode"
        );
    }

    #[test]
    fn test_get_skeleton_empty_file_list_returns_empty_string() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let out = get_skeleton(&store, dir.path(), &[], DetailLevel::Standard).unwrap();
        assert!(out.is_empty());
    }

    // ── render_symbol / oneline unit tests (no external I/O) ─────────────────
    // These tests directly call render_symbol and oneline without touching
    // GraphStore or files on disk — effectively unit/mock-level isolation.

    #[test]
    fn test_render_symbol_minimal_hides_private() {
        let sym = Symbol {
            id: 1,
            file: "t.rs".to_string(),
            kind: SymbolKind::Function,
            name: "secret".to_string(),
            start_line: 0,
            end_line: 5,
            signature: "fn secret()".to_string(),
            is_public: false,
        };
        let mut out = String::new();
        render_symbol(&mut out, &sym, DetailLevel::Minimal, std::path::Path::new("/nonexistent/file.rs")).unwrap();
        assert!(out.is_empty(), "private symbol should produce no output in Minimal mode");
    }

    #[test]
    fn test_render_symbol_minimal_shows_public() {
        let sym = Symbol {
            id: 2,
            file: "t.rs".to_string(),
            kind: SymbolKind::Function,
            name: "greet".to_string(),
            start_line: 0,
            end_line: 5,
            signature: "pub fn greet()".to_string(),
            is_public: true,
        };
        let mut out = String::new();
        render_symbol(&mut out, &sym, DetailLevel::Minimal, std::path::Path::new("/nonexistent/file.rs")).unwrap();
        assert!(out.contains("[function] greet"), "output should contain kind and name");
        assert!(out.contains("pub fn greet()"), "output should contain the signature");
        assert!(out.contains("—"), "output should contain the em-dash separator");
    }

    #[test]
    fn test_render_symbol_standard_shows_private() {
        // Private symbol with start_line=4, end_line=6 → displayed as "line 5-7" (1-based).
        let sym = Symbol {
            id: 3,
            file: "t.rs".to_string(),
            kind: SymbolKind::Function,
            name: "internal".to_string(),
            start_line: 4,
            end_line: 6,
            signature: "fn internal()".to_string(),
            is_public: false,
        };
        let mut out = String::new();
        render_symbol(&mut out, &sym, DetailLevel::Standard, std::path::Path::new("/nonexistent/file.rs")).unwrap();
        assert!(out.contains("internal"), "name should appear in Standard mode");
        assert!(out.contains("line 5-7"), "line numbers should be 1-based");
        assert!(out.contains("fn internal()"), "signature should appear");
    }

    #[test]
    fn test_render_symbol_detailed_skips_body_when_too_long() {
        // end_line - start_line + 1 = 31 > 30 → body read is skipped.
        // Also the path does not exist, so even if lines <= 30, read_lines would fail silently.
        let sym = Symbol {
            id: 4,
            file: "t.rs".to_string(),
            kind: SymbolKind::Function,
            name: "huge_fn".to_string(),
            start_line: 0,
            end_line: 30,
            signature: "fn huge_fn()".to_string(),
            is_public: true,
        };
        let mut out = String::new();
        render_symbol(&mut out, &sym, DetailLevel::Detailed, std::path::Path::new("/nonexistent/file.rs")).unwrap();
        // Header line should be present
        assert!(out.contains("huge_fn"), "function name should appear in header");
        assert!(out.contains("line 1-31"), "line numbers should be 1-based");
        // No body lines (indented with 4 spaces) because lines > 30 threshold
        assert!(!out.contains("    "), "no indented body lines expected when symbol is too long");
    }

    #[test]
    fn test_oneline_normalizes_whitespace() {
        // oneline replaces newlines with spaces and normalizes whitespace.
        assert_eq!(oneline("hello\nworld"), "hello world");
        assert_eq!(oneline("single"), "single");
        assert_eq!(oneline(""), "");
        assert_eq!(oneline("\n"), "");
    }

    #[test]
    fn test_get_skeleton_standard_line_numbers_are_one_based() {
        // Symbol with start_line=9, end_line=14 → output should contain "(line 10-15)".
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file("src/lib.rs", 0, "hash").unwrap();
        store
            .insert_symbol(
                fid,
                &sym("indexed_fn", 9, 14, true, SymbolKind::Function),
            )
            .unwrap();
        let out = get_skeleton(&store, dir.path(), &["src/lib.rs"], DetailLevel::Standard).unwrap();
        assert!(out.contains("indexed_fn"), "function name should appear");
        assert!(out.contains("(line 10-15)"), "line numbers should be 1-based (10-15)");
    }
}
