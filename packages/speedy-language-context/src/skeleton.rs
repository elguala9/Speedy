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
