use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

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

#[derive(Debug, Clone)]
struct CachedEntry {
    file_path: String,
    line: usize,
    text: String,
    hash: String,
    embedding: Vec<f32>,
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &val in v {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

fn blob_to_vec(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
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
}

pub struct SqliteVectorStore {
    conn: Mutex<Connection>,
    cache: RwLock<Vec<CachedEntry>>,
}

impl SqliteVectorStore {
    pub async fn new(path: &str) -> Result<Arc<Self>> {
        let db_dir = Path::new(path).join(".speedy");
        std::fs::create_dir_all(&db_dir)
            .context(format!("failed to create .speedy directory in {path}"))?;
        let db_path = db_dir.join("vectors.db");
        let conn = Connection::open(&db_path)
            .context(format!("failed to open database at {}", db_path.display()))?;

        let store = Arc::new(Self {
            conn: Mutex::new(conn),
            cache: RwLock::new(Vec::new()),
        });

        store.ensure_tables().await?;
        store.load_cache().await?;
        Ok(store)
    }

    async fn load_cache(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT file_path, line, text, hash, embedding FROM chunks",
        ).context("failed to prepare cache load query")?;
        let entries = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(4)?;
                Ok(CachedEntry {
                    file_path: row.get(0)?,
                    line: row.get::<_, i64>(1)? as usize,
                    text: row.get(2)?,
                    hash: row.get(3)?,
                    embedding: blob_to_vec(&blob),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut cache = self.cache.write().await;
        *cache = entries;
        Ok(())
    }
}

#[async_trait]
impl VectorStore for SqliteVectorStore {
    async fn ensure_tables(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                file_path TEXT NOT NULL,
                line INTEGER NOT NULL,
                text TEXT NOT NULL,
                hash TEXT NOT NULL,
                embedding BLOB NOT NULL,
                last_modified TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_file_path ON chunks(file_path);
            CREATE INDEX IF NOT EXISTS idx_hash ON chunks(hash);
            ",
        )?;
        Ok(())
    }

    async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> Result<()> {
        let cache_entries: Vec<CachedEntry> = {
            let conn = self.conn.lock().await;
            let tx = conn.unchecked_transaction()?;
            let mut entries = Vec::with_capacity(chunks.len());
            for chunk in chunks {
                let blob = vec_to_blob(&chunk.embedding);
                tx.execute(
                    "INSERT OR REPLACE INTO chunks (id, file_path, line, text, hash, embedding, last_modified)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        chunk.id,
                        chunk.file_path,
                        chunk.line as i64,
                        chunk.text,
                        chunk.hash,
                        blob,
                        chunk.last_modified,
                    ],
                )?;
                entries.push(CachedEntry {
                    file_path: chunk.file_path.clone(),
                    line: chunk.line,
                    text: chunk.text.clone(),
                    hash: chunk.hash.clone(),
                    embedding: chunk.embedding.clone(),
                });
            }
            tx.commit()?;
            entries
        };

        let mut cache = self.cache.write().await;
        cache.extend(cache_entries);
        Ok(())
    }

    async fn remove_chunks_for_file(&self, file_path: &str) -> Result<()> {
        {
            let conn = self.conn.lock().await;
            conn.execute(
                "DELETE FROM chunks WHERE file_path = ?1",
                params![file_path],
            )?;
        }

        let mut cache = self.cache.write().await;
        cache.retain(|e| e.file_path != file_path);
        Ok(())
    }

    async fn similarity_search(
        &self,
        embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let cache = self.cache.read().await;

        let mut scored: Vec<(f64, &CachedEntry)> = cache
            .iter()
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (score, e)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        Ok(scored
            .into_iter()
            .map(|(score, e)| SearchResult {
                path: e.file_path.clone(),
                line: e.line,
                text: e.text.clone(),
                score,
            })
            .collect())
    }

    async fn get_all_file_paths(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT file_path FROM chunks ORDER BY file_path",
        )?;
        let paths = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(paths)
    }

    async fn count_chunks(&self) -> Result<usize> {
        let cache = self.cache.read().await;
        Ok(cache.len())
    }

    async fn get_last_hash(&self, file_path: &str) -> Result<Option<String>> {
        let cache = self.cache.read().await;
        Ok(cache
            .iter()
            .find(|e| e.file_path == file_path)
            .map(|e| e.hash.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_vec_to_blob_roundtrip() {
        let v = vec![1.0, -2.5, 3.14];
        let blob = vec_to_blob(&v);
        let back = blob_to_vec(&blob);
        assert_eq!(v, back);
    }

    #[test]
    fn test_blob_to_vec_empty() {
        assert!(blob_to_vec(&[]).is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_roundtrip() {
        let dir = std::env::temp_dir().join("speedy_test_db");
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

        store
            .remove_chunks_for_file("src/main.rs")
            .await
            .unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_sqlite_persists() {
        let dir = std::env::temp_dir().join("speedy_test_persist");
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
            let store = SqliteVectorStore::new(dir.to_str().unwrap())
                .await
                .unwrap();
            store.insert_chunks(&records).await.unwrap();
        }

        {
            let store = SqliteVectorStore::new(dir.to_str().unwrap())
                .await
                .unwrap();
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
