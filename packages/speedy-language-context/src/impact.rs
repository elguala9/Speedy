//! Reverse-impact analysis: who references a given set of symbols, transitively?

use anyhow::Result;
use std::collections::HashSet;

use crate::graph::{GraphStore, Symbol};

pub fn find_impact(store: &GraphStore, symbol_ids: &[i64], max_depth: u32) -> Result<Vec<Symbol>> {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut out: Vec<Symbol> = Vec::new();
    for &sid in symbol_ids {
        let referencing = store.find_referencing_symbols(sid, max_depth)?;
        for s in referencing {
            if seen.insert(s.id) {
                out.push(s);
            }
        }
    }
    Ok(out)
}
