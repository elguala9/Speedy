use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
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
    async fn ensure_tables(&self) -> Result<()>;
    /// Read a key from the `metadata` table.
    async fn get_metadata(&self, key: &str) -> Result<Option<String>>;
    /// Upsert a key into the `metadata` table.
    async fn set_metadata(&self, key: &str, value: &str) -> Result<()>;
    /// Drop every chunk in the store. Used by `reembed` after a model change.
    async fn clear_all_chunks(&self) -> Result<()>;
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
}
