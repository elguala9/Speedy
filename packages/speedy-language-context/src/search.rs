//! Lightweight BM25-ish search over the symbol graph.
//!
//! No embeddings — keeps the binary independent of Ollama/HTTP. Good enough
//! for jump-to-symbol style lookups by free-form query.

use anyhow::Result;

use crate::graph::GraphStore;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub id: i64,
    pub file: String,
    pub symbol_name: String,
    pub kind: String,
    pub signature: String,
    pub score: f32,
    pub start_line: u32,
}

pub fn search(store: &GraphStore, query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
    let q_terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if q_terms.is_empty() {
        return Ok(Vec::new());
    }
    let q_full = query.to_lowercase();

    let symbols = store.get_all_symbols()?;
    let mut scored: Vec<SearchResult> = symbols
        .into_iter()
        .filter_map(|s| {
            let name_l = s.name.to_lowercase();
            let sig_l = s.signature.to_lowercase();

            let mut score = 0.0f32;
            // Exact name match — biggest single boost.
            if name_l == q_full {
                score += 50.0;
            } else if name_l.contains(&q_full) {
                score += 15.0;
            }

            // Per-term partial matches.
            for term in &q_terms {
                if name_l == *term {
                    score += 20.0;
                } else if name_l.contains(term) {
                    score += 6.0;
                }
                if sig_l.contains(term) {
                    score += 2.0;
                }
            }

            // Tiny boost for public symbols — they're usually what callers want.
            if s.is_public {
                score += 0.5;
            }

            if score <= 0.0 {
                return None;
            }
            Some(SearchResult {
                id: s.id,
                file: s.file,
                symbol_name: s.name,
                kind: s.kind.to_string(),
                signature: s.signature,
                score,
                start_line: s.start_line,
            })
        })
        .collect();

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    Ok(scored)
}
