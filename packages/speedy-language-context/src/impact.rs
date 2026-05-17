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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, SymbolKind};
    use crate::parser::ParsedSymbol;
    use tempfile::tempdir;

    fn sym(name: &str) -> ParsedSymbol {
        ParsedSymbol {
            kind: SymbolKind::Function,
            name: name.to_string(),
            start_line: 0,
            end_line: 5,
            signature: format!("fn {}()", name),
            is_public: true,
        }
    }

    fn setup(dir: &tempfile::TempDir) -> (GraphStore, i64, i64, i64) {
        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file("t.rs", 0, "h").unwrap();
        let a = store.insert_symbol(fid, &sym("a")).unwrap();
        let b = store.insert_symbol(fid, &sym("b")).unwrap();
        let c = store.insert_symbol(fid, &sym("c")).unwrap();
        (store, a, b, c)
    }

    #[test]
    fn test_find_impact_empty_symbol_list() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        assert!(find_impact(&store, &[], 10).unwrap().is_empty());
    }

    #[test]
    fn test_find_impact_root_node_no_callers() {
        let dir = tempdir().unwrap();
        let (store, a, b, _) = setup(&dir);
        store.insert_edge(a, b, EdgeKind::Calls).unwrap();
        // a has no callers
        let impact = find_impact(&store, &[a], 10).unwrap();
        assert!(impact.is_empty(), "root node has no impact upstream");
    }

    #[test]
    fn test_find_impact_direct_caller() {
        let dir = tempdir().unwrap();
        let (store, a, b, _) = setup(&dir);
        store.insert_edge(a, b, EdgeKind::Calls).unwrap();
        let impact = find_impact(&store, &[b], 10).unwrap();
        assert_eq!(impact.len(), 1);
        assert_eq!(impact[0].name, "a");
    }

    #[test]
    fn test_find_impact_transitive_chain() {
        // a -> b -> c.  Impact of c = [b, a]
        let dir = tempdir().unwrap();
        let (store, a, b, c) = setup(&dir);
        store.insert_edge(a, b, EdgeKind::Calls).unwrap();
        store.insert_edge(b, c, EdgeKind::Calls).unwrap();

        let impact = find_impact(&store, &[c], 10).unwrap();
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"b"), "b directly calls c");
        assert!(names.contains(&"a"), "a transitively calls c");
    }

    #[test]
    fn test_find_impact_cycle_terminates() {
        // a -> b -> c -> a. Should terminate without panic.
        let dir = tempdir().unwrap();
        let (store, a, b, c) = setup(&dir);
        store.insert_edge(a, b, EdgeKind::Calls).unwrap();
        store.insert_edge(b, c, EdgeKind::Calls).unwrap();
        store.insert_edge(c, a, EdgeKind::Calls).unwrap();

        let impact = find_impact(&store, &[c], 100).unwrap();
        assert!(impact.len() <= 3, "cycle BFS must terminate, got {} results", impact.len());
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"b"), "b directly calls c");
    }

    #[test]
    fn test_find_impact_depth_1_only_direct() {
        // a -> b -> c.  depth=1 → only b, not a
        let dir = tempdir().unwrap();
        let (store, a, b, c) = setup(&dir);
        store.insert_edge(a, b, EdgeKind::Calls).unwrap();
        store.insert_edge(b, c, EdgeKind::Calls).unwrap();

        let impact = find_impact(&store, &[c], 1).unwrap();
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"b"), "b is a direct caller");
        assert!(!names.contains(&"a"), "a is beyond depth=1");
    }

    #[test]
    fn test_find_impact_multiple_symbols_deduplicates_callers() {
        // a -> b and a -> c.  Impact of [b, c] should include a exactly once.
        let dir = tempdir().unwrap();
        let (store, a, b, c) = setup(&dir);
        store.insert_edge(a, b, EdgeKind::Calls).unwrap();
        store.insert_edge(a, c, EdgeKind::Calls).unwrap();

        let impact = find_impact(&store, &[b, c], 10).unwrap();
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        let count_a = names.iter().filter(|&&n| n == "a").count();
        assert_eq!(count_a, 1, "a should appear exactly once despite calling both b and c");
    }

    #[test]
    fn test_find_impact_wide_fan_out() {
        // A is called by B1..B5; impact of [A] at depth 1 should return exactly 5 symbols.
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file("t.rs", 0, "h").unwrap();
        let a = store.insert_symbol(fid, &sym("a")).unwrap();
        let mut callers = Vec::new();
        for i in 1..=5 {
            let b = store.insert_symbol(fid, &sym(&format!("b{}", i))).unwrap();
            store.insert_edge(b, a, EdgeKind::Calls).unwrap();
            callers.push(b);
        }
        let impact = find_impact(&store, &[a], 1).unwrap();
        assert_eq!(impact.len(), 5, "should have exactly 5 callers");
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        for i in 1..=5 {
            assert!(names.contains(&format!("b{}", i).as_str()), "b{} should be in impact", i);
        }
    }

    #[test]
    fn test_find_impact_diamond_pattern() {
        // Graph: B calls A, C calls A, D calls B, D calls C
        // Impact of A at depth 5 → {B, C, D} with no duplicates even though D is reachable twice.
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file("t.rs", 0, "h").unwrap();
        let a = store.insert_symbol(fid, &sym("a")).unwrap();
        let b = store.insert_symbol(fid, &sym("b")).unwrap();
        let c = store.insert_symbol(fid, &sym("c")).unwrap();
        let d = store.insert_symbol(fid, &sym("d")).unwrap();
        store.insert_edge(b, a, EdgeKind::Calls).unwrap(); // b calls a
        store.insert_edge(c, a, EdgeKind::Calls).unwrap(); // c calls a
        store.insert_edge(d, b, EdgeKind::Calls).unwrap(); // d calls b
        store.insert_edge(d, c, EdgeKind::Calls).unwrap(); // d calls c

        let impact = find_impact(&store, &[a], 5).unwrap();
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"b"), "b should be in impact");
        assert!(names.contains(&"c"), "c should be in impact");
        assert!(names.contains(&"d"), "d should be in impact");
        let count_d = names.iter().filter(|&&n| n == "d").count();
        assert_eq!(count_d, 1, "d should appear exactly once despite being reachable via both b and c");
        assert_eq!(impact.len(), 3, "should have exactly 3 distinct symbols");
    }

    #[test]
    fn test_find_impact_max_depth_zero_returns_empty() {
        // Depth 0 means no traversal; no callers should be returned.
        let dir = tempdir().unwrap();
        let (store, a, b, _c) = setup(&dir);
        store.insert_edge(b, a, EdgeKind::Calls).unwrap();
        let impact = find_impact(&store, &[a], 0).unwrap();
        assert!(impact.is_empty(), "depth 0 should return no callers");
    }

    #[test]
    fn test_find_impact_symbol_not_in_store_returns_empty() {
        // A symbol id that does not exist in the store should return Ok(empty) without panicking.
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let impact = find_impact(&store, &[99999], 5).unwrap();
        assert!(impact.is_empty(), "nonexistent symbol should yield empty impact");
    }

    #[test]
    fn test_find_impact_two_roots_union_of_callers() {
        // Symbol A has callers X and Z; symbol B has caller Y.
        // find_impact([A, B], 3) should return X, Z, and Y.
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let fid = store.upsert_file("t.rs", 0, "h").unwrap();
        let a = store.insert_symbol(fid, &sym("a")).unwrap();
        let b = store.insert_symbol(fid, &sym("b")).unwrap();
        let x = store.insert_symbol(fid, &sym("x")).unwrap();
        let y = store.insert_symbol(fid, &sym("y")).unwrap();
        let z = store.insert_symbol(fid, &sym("z")).unwrap();
        store.insert_edge(x, a, EdgeKind::Calls).unwrap(); // x calls a
        store.insert_edge(z, a, EdgeKind::Calls).unwrap(); // z calls a
        store.insert_edge(y, b, EdgeKind::Calls).unwrap(); // y calls b

        let impact = find_impact(&store, &[a, b], 3).unwrap();
        let names: Vec<&str> = impact.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"x"), "x should be in impact");
        assert!(names.contains(&"y"), "y should be in impact");
        assert!(names.contains(&"z"), "z should be in impact");
        assert_eq!(impact.len(), 3, "should have exactly 3 distinct callers");
    }
}
