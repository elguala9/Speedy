use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

use crate::tokenize::{SearchType, Token};

#[derive(Debug, Serialize)]
pub struct Occurrence {
    pub file: String,
    pub line: u32,
    pub col_start: u32,
    pub col_end: u32,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub files: i64,
    pub occurrences: i64,
    pub unique_symbols: i64,
    pub db_size_bytes: u64,
    pub last_index_at: Option<i64>,
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS occurrences (
            symbol      TEXT    NOT NULL,
            search_type TEXT    NOT NULL,
            file_path   TEXT    NOT NULL,
            file_ext    TEXT    NOT NULL,
            line_no     INTEGER NOT NULL,
            col_start   INTEGER NOT NULL,
            col_end     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sym_type     ON occurrences(symbol, search_type);
        CREATE INDEX IF NOT EXISTS idx_sym_type_ext ON occurrences(symbol, search_type, file_ext);
        CREATE TABLE IF NOT EXISTS indexed_files (
            file_path  TEXT PRIMARY KEY,
            file_ext   TEXT NOT NULL,
            hash       TEXT NOT NULL,
            indexed_at INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', '1');
        "#,
    )?;
    Ok(())
}

pub fn insert_occurrences(conn: &Connection, file_path: &str, file_ext: &str, tokens: &[Token]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO occurrences(symbol, search_type, file_path, file_ext, line_no, col_start, col_end) \
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
    )?;
    for t in tokens {
        stmt.execute(params![
            t.symbol,
            t.search_type.as_str(),
            file_path,
            file_ext,
            t.line_no,
            t.col_start,
            t.col_end
        ])?;
    }
    Ok(())
}

pub fn delete_file(conn: &Connection, file_path: &str) -> Result<()> {
    conn.execute("DELETE FROM occurrences WHERE file_path = ?1", params![file_path])?;
    conn.execute("DELETE FROM indexed_files WHERE file_path = ?1", params![file_path])?;
    Ok(())
}

pub fn upsert_indexed_file(conn: &Connection, file_path: &str, file_ext: &str, hash: &str) -> Result<()> {
    let now = now_secs();
    conn.execute(
        "INSERT OR REPLACE INTO indexed_files(file_path, file_ext, hash, indexed_at) VALUES (?1,?2,?3,?4)",
        params![file_path, file_ext, hash, now],
    )?;
    Ok(())
}

pub fn get_file_hash(conn: &Connection, file_path: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached("SELECT hash FROM indexed_files WHERE file_path = ?1")?;
    let mut rows = stmt.query(params![file_path])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn query_occurrences(
    conn: &Connection,
    symbol: &str,
    search_type: &SearchType,
    ext_filter: Option<&str>,
    ignore_case: bool,
) -> Result<Vec<Occurrence>> {
    // search_type expansion: narrowest stored type implies wider query types
    let types_in = match search_type {
        SearchType::Cased => "'cased','isolated_special','isolated'",
        SearchType::IsolatedSpecial => "'isolated_special','isolated'",
        SearchType::Isolated => "'isolated'",
    };

    let sym_expr = if ignore_case {
        "LOWER(symbol) = LOWER(?1)"
    } else {
        "symbol = ?1"
    };

    let sql = if ext_filter.is_some() {
        format!(
            "SELECT file_path, line_no, col_start, col_end \
             FROM occurrences \
             WHERE {sym_expr} AND search_type IN ({types_in}) AND file_ext = ?2 \
             ORDER BY file_path, line_no, col_start"
        )
    } else {
        format!(
            "SELECT file_path, line_no, col_start, col_end \
             FROM occurrences \
             WHERE {sym_expr} AND search_type IN ({types_in}) \
             ORDER BY file_path, line_no, col_start"
        )
    };

    let mut stmt = conn.prepare(&sql)?;

    let rows = if let Some(ext) = ext_filter {
        stmt.query_map(params![symbol, ext], |row| {
            Ok(Occurrence {
                file: row.get(0)?,
                line: row.get(1)?,
                col_start: row.get(2)?,
                col_end: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(params![symbol], |row| {
            Ok(Occurrence {
                file: row.get(0)?,
                line: row.get(1)?,
                col_start: row.get(2)?,
                col_end: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(rows)
}

pub fn get_status(conn: &Connection, db_path: &Path) -> Result<Status> {
    let files: i64 = conn.query_row("SELECT COUNT(*) FROM indexed_files", [], |r| r.get(0))?;
    let occurrences: i64 = conn.query_row("SELECT COUNT(*) FROM occurrences", [], |r| r.get(0))?;
    let unique_symbols: i64 =
        conn.query_row("SELECT COUNT(DISTINCT symbol) FROM occurrences", [], |r| r.get(0))?;
    let db_size_bytes = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    let last_index_at: Option<i64> = conn
        .query_row("SELECT value FROM meta WHERE key = 'last_index_at'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|v| v.parse().ok());

    Ok(Status {
        files,
        occurrences,
        unique_symbols,
        db_size_bytes,
        last_index_at,
    })
}

pub fn all_indexed_paths(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT file_path FROM indexed_files")?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}

pub fn clear_all(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM occurrences; DELETE FROM indexed_files;")?;
    Ok(())
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES (?1,?2)",
        params![key, value],
    )?;
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
