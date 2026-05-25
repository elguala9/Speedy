use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Once};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub id: String,
    pub file_path: String,
    pub line: usize,
    pub text: String,
    pub hash: String,
    pub embedding: Vec<f32>,
    pub last_modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub path: String,
    pub line: usize,
    pub text: String,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub hash: String,
    pub last_modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub root: String,
    pub file_count: usize,
    pub chunk_count: usize,
    pub last_indexed: String,
    pub summary: Option<String>,
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
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

#[async_trait]
pub trait VectorStore: Send + Sync {
    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<()>;
    async fn remove_chunks_for_file(&self, file_path: &str) -> Result<()>;
    async fn similarity_search(
        &self,
        embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<SearchResult>>;
    async fn get_all_file_paths(&self) -> Result<Vec<String>>;
    async fn count_chunks(&self) -> Result<usize>;
    async fn get_last_hash(&self, file_path: &str) -> Result<Option<String>>;
    /// Returns hash and last_modified for a file in one query — used for mtime pre-filter.
    async fn get_file_meta(&self, file_path: &str) -> Result<Option<FileMeta>>;
    async fn ensure_tables(&self) -> Result<()>;
    /// Read a key from the `metadata` table.
    async fn get_metadata(&self, key: &str) -> Result<Option<String>>;
    /// Upsert a key into the `metadata` table.
    async fn set_metadata(&self, key: &str, value: &str) -> Result<()>;
    /// Drop every chunk in the store. Used by `reembed` after a model change.
    async fn clear_all_chunks(&self) -> Result<()>;
    /// Load the most recent hash+mtime for every file in one query.
    /// Used by `index_directory` to avoid per-file DB round-trips.
    async fn get_all_file_meta(&self) -> Result<HashMap<String, FileMeta>>;
    /// Remove old chunks for each file and insert the new ones, all in a
    /// single SQLite transaction. More efficient than N separate calls when
    /// indexing a batch of files.
    async fn replace_file_chunks_batch(
        &self,
        replacements: &[(String, Vec<ChunkRecord>)],
    ) -> Result<()>;
}

pub struct SqliteVectorStore {
    conn: Mutex<Connection>,
}

impl SqliteVectorStore {
    pub async fn new(path: &str) -> Result<Arc<Self>> {
        // Register sqlite-vec globally so every connection opened after this
        // automatically has the vec0 virtual-table module available.
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn test_sqlite_roundtrip() {
        let dir = std::env::temp_dir().join("speedy_test_db_vec");
        let _ = std::fs::remove_dir_all(&dir);

        let store = SqliteVectorStore::new(dir.to_str().unwrap())
            .await
            .expect("create store");

        let records = vec![ChunkRecord {
            id: "test-1".to_string(),
            file_path: "src/main.rs".to_string(),
            line: 42,
            text: "fn main() { println!(\"hello\"); }".to_string(),
            hash: "abc123".to_string(),
            embedding: vec![1.0, 0.0, 0.0],
            last_modified: "2024-01-01".to_string(),
        }];

        store.insert_chunks(&records).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 1);

        let results = store
            .similarity_search(&[0.99, 0.01, 0.01], 5)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "src/main.rs");
        assert_eq!(results[0].line, 42);
        assert!(results[0].score > 0.99);

        store.remove_chunks_for_file("src/main.rs").await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_metadata_roundtrip() {
        let dir = std::env::temp_dir().join("speedy_test_metadata_vec");
        let _ = std::fs::remove_dir_all(&dir);
        let store = SqliteVectorStore::new(dir.to_str().unwrap()).await.unwrap();

        assert!(store.get_metadata("embedding_model").await.unwrap().is_none());
        store
            .set_metadata("embedding_model", "nomic-embed-text")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_metadata("embedding_model")
                .await
                .unwrap()
                .as_deref(),
            Some("nomic-embed-text"),
        );
        store
            .set_metadata("embedding_model", "all-minilm")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_metadata("embedding_model")
                .await
                .unwrap()
                .as_deref(),
            Some("all-minilm"),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_clear_all_chunks_empties_store() {
        let dir = std::env::temp_dir().join("speedy_test_clear_vec");
        let _ = std::fs::remove_dir_all(&dir);
        let store = SqliteVectorStore::new(dir.to_str().unwrap()).await.unwrap();

        let records = vec![
            ChunkRecord {
                id: "c1".into(),
                file_path: "a.rs".into(),
                line: 1,
                text: "x".into(),
                hash: "h".into(),
                embedding: vec![1.0, 0.0],
                last_modified: "n".into(),
            },
            ChunkRecord {
                id: "c2".into(),
                file_path: "b.rs".into(),
                line: 2,
                text: "y".into(),
                hash: "h".into(),
                embedding: vec![0.0, 1.0],
                last_modified: "n".into(),
            },
        ];
        store.insert_chunks(&records).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 2);

        store.clear_all_chunks().await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 0);
        assert!(store
            .similarity_search(&[1.0, 0.0], 5)
            .await
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_migration_from_old_blob_schema() {
        // Simulate a DB created by an old version of speedy that stored embeddings
        // as a BLOB column directly inside `chunks`.  SqliteVectorStore::new() must
        // detect the old column, drop the stale tables, and recreate the new schema.
        let dir = tempfile::TempDir::new().unwrap();
        let speedy_dir = dir.path().join(".speedy");
        std::fs::create_dir_all(&speedy_dir).unwrap();
        let db_path = speedy_dir.join("sac.sqlite");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE chunks (
                    rowid INTEGER PRIMARY KEY,
                    id TEXT NOT NULL UNIQUE,
                    file_path TEXT NOT NULL,
                    text TEXT NOT NULL,
                    hash TEXT NOT NULL,
                    embedding BLOB NOT NULL,
                    last_modified TEXT NOT NULL
                );
                INSERT INTO chunks(id, file_path, text, hash, embedding, last_modified)
                VALUES ('old-1', 'old.rs', 'old content', 'h1', X'00000000', '2024-01-01');",
            )
            .unwrap();
        }

        let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
            .await
            .expect("store should open and migrate successfully");

        assert_eq!(store.count_chunks().await.unwrap(), 0, "old data should be dropped");

        // New schema must be fully functional after migration
        let records = vec![ChunkRecord {
            id: "new-1".to_string(),
            file_path: "new.rs".to_string(),
            line: 1,
            text: "new content".to_string(),
            hash: "h2".to_string(),
            embedding: vec![1.0, 0.0],
            last_modified: "2024-01-02".to_string(),
        }];
        store.insert_chunks(&records).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_ensure_tables_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
            .await
            .unwrap();
        store.ensure_tables().await.unwrap();
        store.ensure_tables().await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_insert_chunks_after_clear_accepts_different_dimension() {
        // After clear_all_chunks() drops vec_chunks, a subsequent insert with a
        // different embedding dimension should succeed (vec_chunks is recreated).
        let dir = tempfile::TempDir::new().unwrap();
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
            .await
            .unwrap();

        let r1 = vec![ChunkRecord {
            id: "d1".into(),
            file_path: "f.rs".into(),
            line: 1,
            text: "hello".into(),
            hash: "h".into(),
            embedding: vec![1.0, 0.0, 0.0],
            last_modified: "t".into(),
        }];
        store.insert_chunks(&r1).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 1);

        store.clear_all_chunks().await.unwrap();

        let r2 = vec![ChunkRecord {
            id: "d2".into(),
            file_path: "g.rs".into(),
            line: 1,
            text: "world".into(),
            hash: "h2".into(),
            embedding: vec![0.0, 1.0],
            last_modified: "t".into(),
        }];
        store.insert_chunks(&r2).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_sqlite_persists() {
        let dir = std::env::temp_dir().join("speedy_test_persist_vec");
        let _ = std::fs::remove_dir_all(&dir);

        let records = vec![ChunkRecord {
            id: "p-1".to_string(),
            file_path: "lib.rs".to_string(),
            line: 1,
            text: "pub fn foo() -> i32 { 42 }".to_string(),
            hash: "def456".to_string(),
            embedding: vec![0.0, 0.0, 1.0],
            last_modified: "2024-06-15".to_string(),
        }];

        {
            let store = SqliteVectorStore::new(dir.to_str().unwrap()).await.unwrap();
            store.insert_chunks(&records).await.unwrap();
        }

        {
            let store = SqliteVectorStore::new(dir.to_str().unwrap()).await.unwrap();
            assert_eq!(store.count_chunks().await.unwrap(), 1);
            let results = store
                .similarity_search(&[0.0, 0.0, 0.99], 5)
                .await
                .unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].path, "lib.rs");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_remove_chunks_for_file_leaves_others_intact() {
        // Verifies that removing chunks for one file does not affect chunks from another file.
        let dir = tempfile::TempDir::new().unwrap();
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
            .await
            .unwrap();

        let chunks = vec![
            ChunkRecord {
                id: "a-1".to_string(),
                file_path: "alpha.rs".to_string(),
                line: 1,
                text: "fn alpha() {}".to_string(),
                hash: "ha1".to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                last_modified: "2024-01-01".to_string(),
            },
            ChunkRecord {
                id: "b-1".to_string(),
                file_path: "beta.rs".to_string(),
                line: 1,
                text: "fn beta() {}".to_string(),
                hash: "hb1".to_string(),
                embedding: vec![0.0, 1.0, 0.0],
                last_modified: "2024-01-01".to_string(),
            },
        ];
        store.insert_chunks(&chunks).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 2);

        store.remove_chunks_for_file("alpha.rs").await.unwrap();

        assert_eq!(store.count_chunks().await.unwrap(), 1, "only beta.rs chunk should remain");
        let paths = store.get_all_file_paths().await.unwrap();
        assert_eq!(paths, vec!["beta.rs"], "only beta.rs should be in file paths");
    }

    #[tokio::test]
    async fn test_get_last_hash_returns_none_for_unknown_file() {
        // Verifies that get_last_hash returns None for a file that was never inserted.
        let dir = tempfile::TempDir::new().unwrap();
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
            .await
            .unwrap();

        let result = store.get_last_hash("nonexistent.rs").await.unwrap();
        assert!(result.is_none(), "hash for unknown file should be None");
    }

    #[tokio::test]
    async fn test_get_all_file_paths_after_multi_insert() {
        // Verifies that get_all_file_paths returns all distinct file paths after inserting chunks for 3 files.
        use std::collections::HashSet;

        let dir = tempfile::TempDir::new().unwrap();
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
            .await
            .unwrap();

        let chunks = vec![
            ChunkRecord {
                id: "f1-1".to_string(),
                file_path: "file1.rs".to_string(),
                line: 1,
                text: "fn one() {}".to_string(),
                hash: "h1".to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                last_modified: "2024-01-01".to_string(),
            },
            ChunkRecord {
                id: "f2-1".to_string(),
                file_path: "file2.rs".to_string(),
                line: 1,
                text: "fn two() {}".to_string(),
                hash: "h2".to_string(),
                embedding: vec![0.0, 1.0, 0.0],
                last_modified: "2024-01-01".to_string(),
            },
            ChunkRecord {
                id: "f3-1".to_string(),
                file_path: "file3.rs".to_string(),
                line: 1,
                text: "fn three() {}".to_string(),
                hash: "h3".to_string(),
                embedding: vec![0.0, 0.0, 1.0],
                last_modified: "2024-01-01".to_string(),
            },
        ];
        store.insert_chunks(&chunks).await.unwrap();

        let paths: HashSet<String> = store.get_all_file_paths().await.unwrap().into_iter().collect();
        assert_eq!(paths.len(), 3);
        assert!(paths.contains("file1.rs"));
        assert!(paths.contains("file2.rs"));
        assert!(paths.contains("file3.rs"));
    }

    // ── Mock-based tests ──────────────────────────────────────────────────────────
    // [MOCK] The MockVectorStore below replaces SQLite with an in-memory store,
    // letting us test VectorStore contract behaviour without any I/O.
    mod mocked {
        use super::*;
        use std::collections::HashMap;
        use std::sync::Mutex;
        use anyhow::anyhow;
        use async_trait::async_trait;

        /// [MOCK] In-memory VectorStore for unit testing.
        /// Tracks insert/remove calls and supports controlled error injection.
        struct MockVectorStore {
            chunks: Mutex<Vec<ChunkRecord>>,
            hashes: Mutex<HashMap<String, String>>,
            metadata: Mutex<HashMap<String, String>>,
            fail_insert: Mutex<bool>,
        }

        #[allow(dead_code)]
        impl MockVectorStore {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    chunks: Mutex::new(vec![]),
                    hashes: Mutex::new(HashMap::new()),
                    metadata: Mutex::new(HashMap::new()),
                    fail_insert: Mutex::new(false),
                })
            }

            fn set_fail_insert(&self, fail: bool) {
                *self.fail_insert.lock().unwrap() = fail;
            }

            fn chunk_count_for_file(&self, file_path: &str) -> usize {
                self.chunks
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.file_path == file_path)
                    .count()
            }

            fn all_chunks(&self) -> Vec<ChunkRecord> {
                self.chunks.lock().unwrap().clone()
            }
        }

        #[async_trait]
        impl VectorStore for MockVectorStore {
            async fn ensure_tables(&self) -> Result<()> {
                Ok(())
            }

            async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<()> {
                if *self.fail_insert.lock().unwrap() {
                    return Err(anyhow!("mock insert error"));
                }
                let mut store = self.chunks.lock().unwrap();
                let mut hashes = self.hashes.lock().unwrap();
                for c in chunks {
                    store.push(c.clone());
                    hashes.insert(c.file_path.clone(), c.hash.clone());
                }
                Ok(())
            }

            async fn remove_chunks_for_file(&self, file_path: &str) -> Result<()> {
                self.chunks.lock().unwrap().retain(|c| c.file_path != file_path);
                self.hashes.lock().unwrap().remove(file_path);
                Ok(())
            }

            async fn similarity_search(&self, _embedding: &[f32], top_k: usize) -> Result<Vec<SearchResult>> {
                let results: Vec<SearchResult> = self
                    .chunks
                    .lock()
                    .unwrap()
                    .iter()
                    .take(top_k)
                    .map(|c| SearchResult {
                        path: c.file_path.clone(),
                        line: c.line,
                        text: c.text.clone(),
                        score: 1.0,
                    })
                    .collect();
                Ok(results)
            }

            async fn get_all_file_paths(&self) -> Result<Vec<String>> {
                let mut seen = std::collections::HashSet::new();
                let paths: Vec<String> = self
                    .chunks
                    .lock()
                    .unwrap()
                    .iter()
                    .filter_map(|c| {
                        if seen.insert(c.file_path.clone()) {
                            Some(c.file_path.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                Ok(paths)
            }

            async fn count_chunks(&self) -> Result<usize> {
                Ok(self.chunks.lock().unwrap().len())
            }

            async fn get_last_hash(&self, file_path: &str) -> Result<Option<String>> {
                Ok(self.hashes.lock().unwrap().get(file_path).cloned())
            }

            async fn get_file_meta(&self, file_path: &str) -> Result<Option<FileMeta>> {
                Ok(self.chunks.lock().unwrap()
                    .iter()
                    .find(|c| c.file_path == file_path)
                    .map(|c| FileMeta {
                        hash: c.hash.clone(),
                        last_modified: c.last_modified.clone(),
                    }))
            }

            async fn get_metadata(&self, key: &str) -> Result<Option<String>> {
                Ok(self.metadata.lock().unwrap().get(key).cloned())
            }

            async fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
                self.metadata.lock().unwrap().insert(key.to_string(), value.to_string());
                Ok(())
            }

            async fn clear_all_chunks(&self) -> Result<()> {
                self.chunks.lock().unwrap().clear();
                self.hashes.lock().unwrap().clear();
                Ok(())
            }

            async fn get_all_file_meta(&self) -> Result<HashMap<String, FileMeta>> {
                let chunks = self.chunks.lock().unwrap();
                let mut map = HashMap::new();
                for c in chunks.iter() {
                    map.entry(c.file_path.clone()).or_insert(FileMeta {
                        hash: c.hash.clone(),
                        last_modified: c.last_modified.clone(),
                    });
                }
                Ok(map)
            }

            async fn replace_file_chunks_batch(
                &self,
                replacements: &[(String, Vec<ChunkRecord>)],
            ) -> Result<()> {
                let mut chunks = self.chunks.lock().unwrap();
                let mut hashes = self.hashes.lock().unwrap();
                for (file_path, new_chunks) in replacements {
                    chunks.retain(|c| &c.file_path != file_path);
                    hashes.remove(file_path.as_str());
                    for c in new_chunks {
                        chunks.push(c.clone());
                        hashes.insert(c.file_path.clone(), c.hash.clone());
                    }
                }
                Ok(())
            }
        }

        fn make_chunk(id: &str, file: &str) -> ChunkRecord {
            ChunkRecord {
                id: id.to_string(),
                file_path: file.to_string(),
                line: 1,
                text: format!("fn {}() {{}}", id),
                hash: format!("hash_{}", id),
                embedding: vec![0.1, 0.2, 0.3],
                last_modified: "2024-01-01".to_string(),
            }
        }

        #[tokio::test]
        async fn test_mock_insert_and_count() {
            // Verifies that inserting chunks increases the count correctly.
            let store = MockVectorStore::new();
            store.insert_chunks(&[make_chunk("c1", "a.rs"), make_chunk("c2", "b.rs")]).await.unwrap();
            assert_eq!(store.count_chunks().await.unwrap(), 2);
        }

        #[tokio::test]
        async fn test_mock_remove_clears_chunks_and_hash() {
            // Verifies that remove_chunks_for_file clears both chunks and the cached hash.
            let store = MockVectorStore::new();
            store.insert_chunks(&[make_chunk("c1", "a.rs")]).await.unwrap();
            assert_eq!(store.get_last_hash("a.rs").await.unwrap(), Some("hash_c1".to_string()));

            store.remove_chunks_for_file("a.rs").await.unwrap();

            assert_eq!(store.count_chunks().await.unwrap(), 0);
            assert!(store.get_last_hash("a.rs").await.unwrap().is_none());
        }

        #[tokio::test]
        async fn test_mock_insert_error_propagates() {
            // Verifies that a failing insert returns an error and leaves the store empty.
            let store = MockVectorStore::new();
            store.set_fail_insert(true);
            let result = store.insert_chunks(&[make_chunk("c1", "a.rs")]).await;
            assert!(result.is_err(), "expected error from mock insert");
            assert_eq!(store.count_chunks().await.unwrap(), 0);
        }

        #[tokio::test]
        async fn test_mock_clear_all_resets_state() {
            // Verifies that clear_all_chunks removes all chunks and resets hashes.
            let store = MockVectorStore::new();
            store.insert_chunks(&[make_chunk("c1", "a.rs"), make_chunk("c2", "b.rs")]).await.unwrap();
            assert_eq!(store.count_chunks().await.unwrap(), 2);

            store.clear_all_chunks().await.unwrap();

            assert_eq!(store.count_chunks().await.unwrap(), 0);
            assert!(store.get_last_hash("a.rs").await.unwrap().is_none());
        }

        #[tokio::test]
        async fn test_mock_get_last_hash_tracks_inserts() {
            // Verifies that get_last_hash returns the hash set during the last insert for a file.
            let store = MockVectorStore::new();
            store.insert_chunks(&[make_chunk("c1", "src/lib.rs")]).await.unwrap();
            assert_eq!(
                store.get_last_hash("src/lib.rs").await.unwrap(),
                Some("hash_c1".to_string())
            );
        }

        #[tokio::test]
        async fn test_mock_metadata_roundtrip() {
            // Verifies that metadata written with set_metadata is returned by get_metadata.
            let store = MockVectorStore::new();
            assert!(store.get_metadata("model").await.unwrap().is_none());
            store.set_metadata("model", "test-model").await.unwrap();
            assert_eq!(store.get_metadata("model").await.unwrap(), Some("test-model".to_string()));
        }

        #[tokio::test]
        async fn test_mock_similarity_search_respects_top_k() {
            // Verifies that similarity_search returns at most top_k results.
            let store = MockVectorStore::new();
            let chunks: Vec<ChunkRecord> = (0..5)
                .map(|i| make_chunk(&format!("c{}", i), &format!("f{}.rs", i)))
                .collect();
            store.insert_chunks(&chunks).await.unwrap();

            let results = store.similarity_search(&[0.1, 0.2, 0.3], 3).await.unwrap();
            assert_eq!(results.len(), 3, "top_k=3 should return exactly 3 results");
        }
    }
}
