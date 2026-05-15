//! Free-form notes that the AI can save and retrieve via the MCP server.
//! Backed by SQLite + FTS5 in the shared `.speedy/slc.db`.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryEntry {
    pub id: i64,
    pub text: String,
    pub created_at: String,
}

pub struct Memory {
    conn: Mutex<Connection>,
}

impl Memory {
    pub fn open(workspace_root: &Path) -> Result<Self> {
        let speedy_dir = workspace_root.join(".speedy");
        if !speedy_dir.exists() {
            std::fs::create_dir_all(&speedy_dir)
                .with_context(|| format!("creating .speedy/ at {}", speedy_dir.display()))?;
        }
        let db_path = speedy_dir.join("slc.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")?;
        // Schema is created by GraphStore::open as well; safe to re-run.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS slc_observations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS slc_observations_fts USING fts5(
                text, content=slc_observations, content_rowid=id
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn save(&self, text: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO slc_observations (text, created_at) VALUES (?1, ?2)",
            params![text, now],
        )?;
        let id = conn.last_insert_rowid();
        // Mirror into FTS index. We use the contentless-link form via INSERT
        // INTO ... (rowid, text). External-content tables would auto-sync via
        // triggers, but we keep it explicit so the schema stays simple.
        conn.execute(
            "INSERT INTO slc_observations_fts (rowid, text) VALUES (?1, ?2)",
            params![id, text],
        )?;
        Ok(id)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT o.id, o.text, o.created_at
             FROM slc_observations o
             JOIN slc_observations_fts fts ON fts.rowid = o.id
             WHERE slc_observations_fts MATCH ?1
             ORDER BY o.id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit as i64], |row| {
            Ok(MemoryEntry {
                id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}
