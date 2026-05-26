use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Once};
use tokio::sync::Mutex;

use super::{ChunkRecord, FileMeta, SearchResult, VectorStore};

pub(super) fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &val in v {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

static SQLITE_VEC_LOADED: Once = Once::new();

fn load_sqlite_vec_extension() {
    SQLITE_VEC_LOADED.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

fn create_vec_table(conn: &Connection, dim: usize) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks \
         USING vec0(embedding float[{dim}] distance_metric=cosine);"
    ))?;
    Ok(())
}

pub struct SqliteVectorStore {
    conn: Mutex<Connection>,
}

impl SqliteVectorStore {
    pub async fn new(path: &str) -> Result<Arc<Self>> {
        load_sqlite_vec_extension();

        let db_dir = Path::new(path).join(".speedy");
        std::fs::create_dir_all(&db_dir)
            .context(format!("failed to create .speedy directory in {path}"))?;
        let db_path = db_dir.join("sac.sqlite");
        let conn = Connection::open(&db_path)
            .context(format!("failed to open database at {}", db_path.display()))?;

        // WAL mode: reduces per-commit fsync overhead significantly for bulk
        // writes; synchronous=NORMAL is safe for an index (not financial data).
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-32000;
             PRAGMA temp_store=MEMORY;"
        ).context("failed to set SQLite WAL pragmas")?;

        let store = Arc::new(Self {
            conn: Mutex::new(conn),
        });

        store.ensure_tables().await?;
        Ok(store)
    }
}

#[async_trait]
impl VectorStore for SqliteVectorStore {
    async fn ensure_tables(&self) -> Result<()> {
        let conn = self.conn.lock().await;

        // Migrate away from the old schema that stored embeddings as BLOBs in `chunks`.
        let has_old_embedding_col: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('chunks') WHERE name = 'embedding'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if has_old_embedding_col {
            conn.execute_batch(
                "DROP TABLE IF EXISTS chunks;
                 DROP TABLE IF EXISTS metadata;
                 DROP TABLE IF EXISTS vec_chunks;",
            )?;
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chunks (
                rowid INTEGER PRIMARY KEY,
                id TEXT NOT NULL UNIQUE,
                file_path TEXT NOT NULL,
                line INTEGER NOT NULL,
                text TEXT NOT NULL,
                hash TEXT NOT NULL,
                last_modified TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_file_path ON chunks(file_path);
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;

        // Rebuild FTS index once on first open (e.g. after migration or first install).
        let fts_ready: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM metadata WHERE key = 'fts_initialized' AND value = '1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !fts_ready {
            conn.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts
                 USING fts5(text, content='chunks', content_rowid='rowid');
                 INSERT INTO chunks_fts(chunks_fts) VALUES('rebuild');",
            )?;
            conn.execute(
                "INSERT INTO metadata(key, value) VALUES('fts_initialized', '1')
                 ON CONFLICT(key) DO UPDATE SET value = '1'",
                [],
            )?;
        }

        Ok(())
    }

    async fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT value FROM metadata WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        Ok(rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?)
    }

    async fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    async fn clear_all_chunks(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM chunks", [])?;
        // Drop vec_chunks so it is recreated with the correct dimension on the
        // next insert (relevant when the embedding model changes).
        conn.execute_batch("DROP TABLE IF EXISTS vec_chunks;")?;
        // Drop FTS index; it will be rebuilt on next ensure_tables call.
        conn.execute_batch("DROP TABLE IF EXISTS chunks_fts;")?;
        conn.execute(
            "DELETE FROM metadata WHERE key = 'fts_initialized'",
            [],
        )?;
        Ok(())
    }

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let dim = chunks[0].embedding.len();
        let conn = self.conn.lock().await;

        // Ensure vec_chunks exists with the right dimension before the transaction.
        create_vec_table(&conn, dim)?;

        let tx = conn.unchecked_transaction()?;
        for chunk in chunks {
            // ON CONFLICT DO UPDATE preserves the rowid, which is the FK into vec_chunks.
            tx.execute(
                "INSERT INTO chunks(id, file_path, line, text, hash, last_modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     file_path     = excluded.file_path,
                     line          = excluded.line,
                     text          = excluded.text,
                     hash          = excluded.hash,
                     last_modified = excluded.last_modified",
                params![
                    chunk.id,
                    chunk.file_path,
                    chunk.line as i64,
                    chunk.text,
                    chunk.hash,
                    chunk.last_modified,
                ],
            )?;

            let rowid: i64 = tx.query_row(
                "SELECT rowid FROM chunks WHERE id = ?1",
                params![chunk.id],
                |row| row.get(0),
            )?;

            let blob = vec_to_blob(&chunk.embedding);
            // Remove any stale vec entry for this rowid before re-inserting.
            tx.execute("DELETE FROM vec_chunks WHERE rowid = ?1", params![rowid])?;
            tx.execute(
                "INSERT INTO vec_chunks(rowid, embedding) VALUES (?1, ?2)",
                params![rowid, blob],
            )?;
        }
        tx.commit()?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts
             USING fts5(text, content='chunks', content_rowid='rowid');
             INSERT INTO chunks_fts(chunks_fts) VALUES('rebuild');",
        )?;
        Ok(())
    }

    async fn remove_chunks_for_file(&self, file_path: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        // Resolve rowids from chunks before deleting them.
        // Silently ignore "no such table" when vec_chunks hasn't been created yet.
        let _ = conn.execute(
            "DELETE FROM vec_chunks WHERE rowid IN (
                SELECT rowid FROM chunks WHERE file_path = ?1
            )",
            params![file_path],
        );
        conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![file_path])?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts
             USING fts5(text, content='chunks', content_rowid='rowid');
             INSERT INTO chunks_fts(chunks_fts) VALUES('rebuild');",
        )?;
        Ok(())
    }

    async fn similarity_search(
        &self,
        embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let conn = self.conn.lock().await;

        let vec_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='vec_chunks'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !vec_exists {
            return Ok(vec![]);
        }

        let blob = vec_to_blob(embedding);
        // vec0 requires the k constraint directly on the vec0 table scan, not on
        // the outer JOIN. Use a subquery so the LIMIT is pushed into the ANN search.
        let mut stmt = conn.prepare(
            "SELECT c.file_path, c.line, c.text, sub.distance
             FROM (
                 SELECT rowid, distance
                 FROM vec_chunks
                 WHERE embedding MATCH ?1
                 AND k = ?2
             ) sub
             JOIN chunks c ON c.rowid = sub.rowid
             ORDER BY sub.distance",
        )?;

        let results = stmt
            .query_map(params![blob, top_k as i64], |row| {
                let dist: f64 = row.get(3)?;
                Ok(SearchResult {
                    path: row.get(0)?,
                    line: row.get::<_, i64>(1)? as usize,
                    text: row.get(2)?,
                    // cosine distance = 1 − cosine_similarity → invert to recover similarity
                    score: 1.0 - dist,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(results)
    }

    async fn get_all_file_paths(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let mut stmt =
            conn.prepare("SELECT DISTINCT file_path FROM chunks ORDER BY file_path")?;
        let paths = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(paths)
    }

    async fn count_chunks(&self) -> Result<usize> {
        let conn = self.conn.lock().await;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    async fn get_last_hash(&self, file_path: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().await;
        let mut stmt =
            conn.prepare("SELECT hash FROM chunks WHERE file_path = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![file_path])?;
        Ok(rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?)
    }

    async fn get_file_meta(&self, file_path: &str) -> Result<Option<FileMeta>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT hash, last_modified FROM chunks WHERE file_path = ?1 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![file_path])?;
        Ok(rows.next()?.map(|r| -> rusqlite::Result<FileMeta> {
            Ok(FileMeta {
                hash: r.get::<_, String>(0)?,
                last_modified: r.get::<_, String>(1)?,
            })
        }).transpose()?)
    }

    async fn get_all_file_meta(&self) -> Result<HashMap<String, FileMeta>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT file_path, hash, last_modified FROM chunks GROUP BY file_path",
        )?;
        let map = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    FileMeta {
                        hash: row.get::<_, String>(1)?,
                        last_modified: row.get::<_, String>(2)?,
                    },
                ))
            })?
            .collect::<std::result::Result<HashMap<_, _>, _>>()?;
        Ok(map)
    }

    async fn replace_file_chunks_batch(
        &self,
        replacements: &[(String, Vec<ChunkRecord>)],
    ) -> Result<()> {
        if replacements.is_empty() {
            return Ok(());
        }
        let first = replacements.iter()
            .flat_map(|(_, chunks)| chunks.iter())
            .next();
        let Some(first) = first else { return Ok(()); };

        let dim = first.embedding.len();
        let conn = self.conn.lock().await;
        create_vec_table(&conn, dim)?;

        let tx = conn.unchecked_transaction()?;
        for (file_path, chunks) in replacements {
            let _ = tx.execute(
                "DELETE FROM vec_chunks WHERE rowid IN \
                 (SELECT rowid FROM chunks WHERE file_path = ?1)",
                params![file_path],
            );
            tx.execute("DELETE FROM chunks WHERE file_path = ?1", params![file_path])?;
            for chunk in chunks {
                tx.execute(
                    "INSERT INTO chunks(id, file_path, line, text, hash, last_modified)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         file_path=excluded.file_path, line=excluded.line,
                         text=excluded.text, hash=excluded.hash,
                         last_modified=excluded.last_modified",
                    params![
                        chunk.id, chunk.file_path, chunk.line as i64,
                        chunk.text, chunk.hash, chunk.last_modified,
                    ],
                )?;
                let rowid: i64 = tx.query_row(
                    "SELECT rowid FROM chunks WHERE id = ?1",
                    params![chunk.id],
                    |row| row.get(0),
                )?;
                let blob = vec_to_blob(&chunk.embedding);
                tx.execute("DELETE FROM vec_chunks WHERE rowid = ?1", params![rowid])?;
                tx.execute(
                    "INSERT INTO vec_chunks(rowid, embedding) VALUES (?1, ?2)",
                    params![rowid, blob],
                )?;
            }
        }
        tx.commit()?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts
             USING fts5(text, content='chunks', content_rowid='rowid');
             INSERT INTO chunks_fts(chunks_fts) VALUES('rebuild');",
        )?;
        Ok(())
    }

    async fn text_search(&self, pattern: &str, top_k: usize) -> Result<Vec<SearchResult>> {
        let conn = self.conn.lock().await;

        let fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='chunks_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !fts_exists {
            return Ok(vec![]);
        }

        let mut stmt = conn.prepare(
            "SELECT chunks.file_path, chunks.line, chunks.text, chunks_fts.rank
             FROM chunks_fts
             JOIN chunks ON chunks.rowid = chunks_fts.rowid
             WHERE chunks_fts MATCH ?1
             ORDER BY chunks_fts.rank
             LIMIT ?2",
        )?;

        let results = stmt
            .query_map(params![pattern, top_k as i64], |row| {
                let rank: f64 = row.get(3)?;
                Ok(SearchResult {
                    path: row.get(0)?,
                    line: row.get::<_, i64>(1)? as usize,
                    text: row.get(2)?,
                    // FTS5 rank is negative (more negative = more relevant); invert for consistency
                    score: -rank,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::vec_to_blob;

    #[test]
    fn test_vec_to_blob_roundtrip() {
        let v = vec![1.0_f32, -2.5, 3.14];
        let blob = vec_to_blob(&v);
        let back: Vec<f32> = blob
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(v, back);
    }
}
