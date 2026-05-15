//! SQLite-backed store for the symbol graph.
//!
//! Schema lives in `.speedy/slc.db` (shared with `memory.rs`).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use super::{EdgeKind, Symbol, SymbolKind};
use crate::parser::ParsedSymbol;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS slc_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    mtime INTEGER NOT NULL,
    content_hash TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS slc_symbols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES slc_files(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    signature TEXT NOT NULL,
    is_public INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS slc_edges (
    src_id INTEGER NOT NULL REFERENCES slc_symbols(id) ON DELETE CASCADE,
    dst_id INTEGER NOT NULL REFERENCES slc_symbols(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    PRIMARY KEY (src_id, dst_id, kind)
);

CREATE INDEX IF NOT EXISTS idx_symbols_file ON slc_symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON slc_symbols(name);
CREATE INDEX IF NOT EXISTS idx_edges_src ON slc_edges(src_id);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON slc_edges(dst_id);

CREATE TABLE IF NOT EXISTS slc_observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS slc_observations_fts USING fts5(
    text, content=slc_observations, content_rowid=id
);

CREATE TABLE IF NOT EXISTS slc_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub struct GraphStore {
    conn: Mutex<Connection>,
}

impl GraphStore {
    pub fn open(workspace_root: &Path) -> Result<Self> {
        let speedy_dir = workspace_root.join(".speedy");
        if !speedy_dir.exists() {
            std::fs::create_dir_all(&speedy_dir)
                .with_context(|| format!("creating .speedy/ at {}", speedy_dir.display()))?;
        }
        let db_path = speedy_dir.join("slc.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("opening {}", db_path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;",
        )?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn upsert_file(&self, path: &str, mtime: i64, hash: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO slc_files (path, mtime, content_hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET mtime = excluded.mtime, content_hash = excluded.content_hash",
            params![path, mtime, hash],
        )?;
        let id = conn.query_row(
            "SELECT id FROM slc_files WHERE path = ?1",
            params![path],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(id)
    }

    pub fn delete_file_symbols(&self, file_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM slc_symbols WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }

    pub fn insert_symbol(&self, file_id: i64, sym: &ParsedSymbol) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO slc_symbols (file_id, kind, name, start_line, end_line, signature, is_public)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                file_id,
                sym.kind.to_string(),
                sym.name,
                sym.start_line,
                sym.end_line,
                sym.signature,
                sym.is_public as i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn insert_edge(&self, src: i64, dst: i64, kind: EdgeKind) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO slc_edges (src_id, dst_id, kind) VALUES (?1, ?2, ?3)",
            params![src, dst, kind.to_string()],
        )?;
        Ok(())
    }

    pub fn get_symbols_for_file(&self, path: &str) -> Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, f.path, s.kind, s.name, s.start_line, s.end_line, s.signature, s.is_public
             FROM slc_symbols s
             JOIN slc_files f ON f.id = s.file_id
             WHERE f.path = ?1
             ORDER BY s.start_line",
        )?;
        let rows = stmt.query_map(params![path], row_to_symbol)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_all_symbols(&self) -> Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, f.path, s.kind, s.name, s.start_line, s.end_line, s.signature, s.is_public
             FROM slc_symbols s
             JOIN slc_files f ON f.id = s.file_id
             ORDER BY f.path, s.start_line",
        )?;
        let rows = stmt.query_map([], row_to_symbol)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_symbol_by_id(&self, id: i64) -> Result<Option<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, f.path, s.kind, s.name, s.start_line, s.end_line, s.signature, s.is_public
             FROM slc_symbols s
             JOIN slc_files f ON f.id = s.file_id
             WHERE s.id = ?1",
        )?;
        let res = stmt
            .query_row(params![id], row_to_symbol)
            .optional()?;
        Ok(res)
    }

    /// BFS the edge graph outwards from `sym_id`, treating any `slc_edges` row
    /// with `dst_id = sym_id` as "X references sym_id". Returns the deduplicated
    /// set of ancestor symbols up to `depth` hops away.
    pub fn find_referencing_symbols(&self, sym_id: i64, depth: u32) -> Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut seen: HashSet<i64> = HashSet::new();
        let mut frontier: Vec<i64> = vec![sym_id];
        let mut results: Vec<Symbol> = Vec::new();

        for _ in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next: Vec<i64> = Vec::new();
            for current in frontier.drain(..) {
                let mut stmt = conn.prepare(
                    "SELECT s.id, f.path, s.kind, s.name, s.start_line, s.end_line, s.signature, s.is_public
                     FROM slc_edges e
                     JOIN slc_symbols s ON s.id = e.src_id
                     JOIN slc_files f ON f.id = s.file_id
                     WHERE e.dst_id = ?1",
                )?;
                let rows = stmt.query_map(params![current], row_to_symbol)?;
                for r in rows {
                    let sym = r?;
                    if seen.insert(sym.id) {
                        next.push(sym.id);
                        results.push(sym);
                    }
                }
            }
            frontier = next;
        }
        Ok(results)
    }

    pub fn file_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM slc_files", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn symbol_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM slc_symbols", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn edge_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM slc_edges", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn get_file_hash(&self, path: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let res = conn
            .query_row(
                "SELECT content_hash FROM slc_files WHERE path = ?1",
                params![path],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(res)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let res = conn
            .query_row(
                "SELECT value FROM slc_meta WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(res)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO slc_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn row_to_symbol(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    let kind: String = row.get(2)?;
    let is_public: i64 = row.get(7)?;
    Ok(Symbol {
        id: row.get(0)?,
        file: row.get(1)?,
        kind: SymbolKind::from_db(&kind),
        name: row.get(3)?,
        start_line: row.get::<_, i64>(4)? as u32,
        end_line: row.get::<_, i64>(5)? as u32,
        signature: row.get(6)?,
        is_public: is_public != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_open_and_counts() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        assert_eq!(store.file_count().unwrap(), 0);
        assert_eq!(store.symbol_count().unwrap(), 0);
        assert_eq!(store.edge_count().unwrap(), 0);
    }

    #[test]
    fn test_upsert_file() {
        let dir = tempdir().unwrap();
        let store = GraphStore::open(dir.path()).unwrap();
        let id1 = store.upsert_file("foo.rs", 100, "h1").unwrap();
        let id2 = store.upsert_file("foo.rs", 200, "h2").unwrap();
        assert_eq!(id1, id2);
        assert_eq!(store.get_file_hash("foo.rs").unwrap().as_deref(), Some("h2"));
    }
}
