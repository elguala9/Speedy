use speedy_core::config::Config;
use crate::db::{ChunkRecord, ProjectSummary, SearchResult, SqliteVectorStore, VectorStore};
use crate::embed::{self, EmbeddingProvider};
use crate::hash;
use crate::ignore::FileFilter;
use crate::document;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::path::Path;
use tracing::error;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStats {
    pub files: usize,
    pub chunks: usize,
    pub removed: usize,
    pub duration_ms: u64,
}

pub struct Indexer {
    pub db: Arc<dyn VectorStore>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub root: String,
    pub model: String,
    embed_cache: Mutex<HashMap<String, Vec<f32>>>,
}

const METADATA_MODEL_KEY: &str = "embedding_model";

impl Indexer {
    pub async fn new(config: &Config) -> Result<Self> {
        let root = std::env::current_dir()
            .context("failed to get current directory")?
            .to_string_lossy()
            .to_string();
        let db: Arc<dyn VectorStore> = SqliteVectorStore::new(&root).await
            .context("failed to initialize vector database")?;

        // Auto-create .speedyignore if missing: merge .gitignore (if present) + default patterns
        let speedyignore = Path::new(&root).join(".speedyignore");
        if !speedyignore.exists() {
            let mut content = String::new();
            let gitignore = Path::new(&root).join(".gitignore");
            if gitignore.exists() {
                if let Ok(text) = std::fs::read_to_string(&gitignore) {
                    content.push_str(&text);
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                }
            }
            content.push_str(speedy_core::default_ignores::RAW);
            let _ = std::fs::write(&speedyignore, content);
        }

        let embedder = embed::create_provider(config)?;

        // Compatibility check: warn if the DB was built with a different model
        // than what's configured now. Old chunks won't be in the same vector
        // space as new query embeddings, so similarity scores become garbage.
        // We warn but don't refuse — the user might be mid-transition and want
        // to run `reembed` next. Indexing only writes the marker once.
        let chunk_count = db.count_chunks().await.unwrap_or(0);
        match db.get_metadata(METADATA_MODEL_KEY).await? {
            Some(stored) if stored != config.model => {
                tracing::warn!(
                    "Embedding model mismatch: DB built with '{stored}' but configured model is '{}'. \
                     Run `speedy reembed` to rebuild the index, or revert SPEEDY_MODEL.",
                    config.model
                );
            }
            None if chunk_count > 0 => {
                tracing::warn!(
                    "DB contains {chunk_count} chunks but has no recorded model. \
                     Assuming current model '{}' — run `speedy reembed` if wrong.",
                    config.model
                );
                db.set_metadata(METADATA_MODEL_KEY, &config.model).await?;
            }
            None => {
                db.set_metadata(METADATA_MODEL_KEY, &config.model).await?;
            }
            _ => {}
        }

        Ok(Self {
            db,
            embedder,
            root,
            model: config.model.clone(),
            embed_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Drop every chunk and re-index the entire workspace with the current
    /// embedding provider. Use after switching `SPEEDY_MODEL`. Persists the
    /// new model name in the DB metadata table on success.
    pub async fn reembed(&self) -> Result<IndexStats> {
        self.db.clear_all_chunks().await
            .context("failed to clear existing chunks before reembed")?;
        let stats = self.index_directory(&self.root).await?;
        self.db.set_metadata(METADATA_MODEL_KEY, &self.model).await
            .context("failed to record model name after reembed")?;
        Ok(stats)
    }

    pub async fn index_directory(&self, path: &str) -> Result<IndexStats> {
        let start = Instant::now();
        let filter = FileFilter::new(path);
        let files: Vec<String> = filter.filtered_files().into_iter()
            .filter(|f| !FileFilter::is_binary(Path::new(f)))
            .collect();
        let total = files.len();
        let mut total_chunks = 0;

        let pb = indicatif::ProgressBar::new(total as u64);
        pb.set_style(indicatif::ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files ({per_sec}) {msg}")
            .unwrap()
            .progress_chars("##-"));

        for file_path in &files {
            let short = if file_path.len() > 50 {
                format!("...{}", &file_path[file_path.len()-47..])
            } else {
                file_path.clone()
            };
            pb.set_message(short);
            match self.index_file(file_path).await {
                Ok(chunks) => total_chunks += chunks,
                Err(e) => error!("Failed: {file_path}: {e}"),
            }
            pb.inc(1);
        }

        pb.finish_and_clear();

        Ok(IndexStats {
            files: total,
            chunks: total_chunks,
            removed: 0,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    pub async fn index_file(&self, file_path: &str) -> Result<usize> {
        let path = Path::new(file_path);
        if !path.exists() {
            self.db.remove_chunks_for_file(file_path).await?;
            return Ok(0);
        }

        let content = match fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => return Ok(0),
        };
        let file_hash = hash::hash_file(path).await
            .context(format!("failed to hash file: {file_path}"))?;
        let metadata = fs::metadata(path).await
            .context(format!("failed to read metadata for: {file_path}"))?;
        let last_modified = metadata
            .modified()
            .ok()
            .map(|t| {
                let dt: chrono::DateTime<Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_default();

        let chunks = document::Document::chunk_file(&content, 1000, 200);
        let mut records = Vec::with_capacity(chunks.len());

        for chunk in &chunks {
            let embedding = {
                let chunk_hash = crate::hash::hash_bytes(chunk.text.as_bytes());
                let cached = {
                    let cache = self.embed_cache.lock().await;
                    cache.get(&chunk_hash).cloned()
                };
                if let Some(emb) = cached {
                    emb
                } else {
                    let emb = self.embedder.embed(&chunk.text).await
                        .context(format!("failed to embed chunk at line {}", chunk.line))?;
                    let mut cache = self.embed_cache.lock().await;
                    cache.insert(chunk_hash, emb.clone());
                    emb
                }
            };
            records.push(ChunkRecord {
                id: Uuid::new_v4().to_string(),
                file_path: file_path.to_string(),
                line: chunk.line,
                text: chunk.text.clone(),
                hash: file_hash.clone(),
                embedding,
                last_modified: last_modified.clone(),
            });
        }

        self.db.remove_chunks_for_file(file_path).await
            .context(format!("failed to remove old chunks for: {file_path}"))?;
        self.db.insert_chunks(&records).await
            .context("failed to insert chunks into database")?;

        Ok(records.len())
    }

    pub async fn query(&self, query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
        let embedding = self.embedder.embed(query).await?;
        self.db.similarity_search(&embedding, top_k).await
    }

    pub async fn project_context(&self) -> Result<ProjectSummary> {
        let file_count = self.db.get_all_file_paths().await?.len();
        let chunk_count = self.db.count_chunks().await?;
        Ok(ProjectSummary {
            root: self.root.clone(),
            file_count,
            chunk_count,
            last_indexed: Utc::now().to_rfc3339(),
            summary: None,
        })
    }

    #[cfg(test)]
    pub async fn new_with_parts(
        db: Arc<dyn crate::db::VectorStore>,
        embedder: Arc<dyn EmbeddingProvider>,
        root: String,
        model: String,
    ) -> Self {
        Self {
            db,
            embedder,
            root,
            model,
            embed_cache: Mutex::new(HashMap::new()),
        }
    }

    pub async fn sync_all(&self) -> Result<IndexStats> {
        let start = Instant::now();


        let filter = FileFilter::new(&self.root);
        let current_files: std::collections::HashSet<String> =
            filter.filtered_files().into_iter().collect();

        let db_files: std::collections::HashSet<String> =
            self.db.get_all_file_paths().await?.into_iter().collect();

        let mut added = 0;
        let mut removed = 0;

        for file in &current_files {
            let p = Path::new(file);
            if FileFilter::is_binary(p) {
                continue;
            }
            let chunks = self.index_file(file).await?;
            added += chunks;
        }

        for file in db_files.difference(&current_files) {
            self.db.remove_chunks_for_file(file).await?;
            removed += 1;
        }

        Ok(IndexStats {
            files: added,
            chunks: added,
            removed,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteVectorStore;
    use crate::embed::tests::StubProvider;
    use tempfile::TempDir;

    async fn make_indexer(dir: &TempDir) -> (Indexer, Arc<StubProvider>) {
        let db: Arc<dyn crate::db::VectorStore> =
            SqliteVectorStore::new(dir.path().to_str().unwrap())
                .await
                .unwrap();
        let stub = StubProvider::new();
        let embedder: Arc<dyn EmbeddingProvider> = stub.clone();
        let idx = Indexer::new_with_parts(
            db,
            embedder,
            dir.path().to_str().unwrap().to_string(),
            "test-model".to_string(),
        )
        .await;
        (idx, stub)
    }

    #[tokio::test]
    async fn test_index_file_nonexistent_removes_stale_chunks() {
        let dir = TempDir::new().unwrap();
        let (idx, _stub) = make_indexer(&dir).await;

        let fake = vec![crate::db::ChunkRecord {
            id: "x1".into(),
            file_path: "/gone.rs".into(),
            line: 1,
            text: "old".into(),
            hash: "h".into(),
            embedding: vec![0.1, 0.2, 0.3],
            last_modified: "t".into(),
        }];
        idx.db.insert_chunks(&fake).await.unwrap();
        assert_eq!(idx.db.count_chunks().await.unwrap(), 1);

        let n = idx.index_file("/gone.rs").await.unwrap();
        assert_eq!(n, 0, "nonexistent file produces zero chunks");
        assert_eq!(idx.db.count_chunks().await.unwrap(), 0, "stale chunks must be removed");
    }

    #[tokio::test]
    async fn test_index_file_produces_chunks_and_calls_embedder() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("hello.rs");
        std::fs::write(&file, b"pub fn hello() -> i32 { 42 }\n").unwrap();

        let (idx, stub) = make_indexer(&dir).await;
        let n = idx.index_file(file.to_str().unwrap()).await.unwrap();
        assert!(n > 0, "should produce at least one chunk");
        assert_eq!(idx.db.count_chunks().await.unwrap(), n);
        assert!(
            !stub.calls.lock().unwrap().is_empty(),
            "embedder should have been called"
        );
    }

    #[tokio::test]
    async fn test_index_file_uses_embed_cache_for_identical_content() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("cached.rs");
        std::fs::write(&file, b"pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();

        let (idx, stub) = make_indexer(&dir).await;

        let n1 = idx.index_file(file.to_str().unwrap()).await.unwrap();
        let calls_after_first = stub.calls.lock().unwrap().len();
        assert_eq!(calls_after_first, n1, "one embed call per chunk on first index");

        // Second call: same content → embed_cache hit → no new embed calls
        let n2 = idx.index_file(file.to_str().unwrap()).await.unwrap();
        let calls_after_second = stub.calls.lock().unwrap().len();
        assert_eq!(n1, n2, "same file produces same chunk count");
        assert_eq!(
            calls_after_first, calls_after_second,
            "no new embed calls when content is in cache"
        );
    }

    #[tokio::test]
    async fn test_index_file_embeds_again_on_content_change() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("changing.rs");
        std::fs::write(&file, b"fn v1() {}").unwrap();

        let (idx, stub) = make_indexer(&dir).await;
        idx.index_file(file.to_str().unwrap()).await.unwrap();
        let calls_v1 = stub.calls.lock().unwrap().len();

        std::fs::write(&file, b"fn v2() { println!(\"changed\"); }\nfn extra() {}").unwrap();
        idx.index_file(file.to_str().unwrap()).await.unwrap();
        let calls_v2 = stub.calls.lock().unwrap().len();

        assert!(calls_v2 > calls_v1, "changed content must trigger new embed calls");
    }

    #[tokio::test]
    async fn test_reembed_clears_and_reindexes_with_model_metadata() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("reembed.rs");
        std::fs::write(&file, b"pub fn target() {}\n").unwrap();

        let (idx, _) = make_indexer(&dir).await;
        idx.index_file(file.to_str().unwrap()).await.unwrap();
        assert!(idx.db.count_chunks().await.unwrap() > 0);

        idx.reembed().await.unwrap();

        assert_eq!(
            idx.db.get_metadata("embedding_model").await.unwrap().as_deref(),
            Some("test-model"),
            "reembed must record the model name"
        );
        assert!(
            idx.db.count_chunks().await.unwrap() > 0,
            "reembed must re-index the workspace"
        );
    }

    #[tokio::test]
    async fn test_metadata_key_set_on_empty_fresh_store() {
        let dir = TempDir::new().unwrap();
        let db: Arc<dyn crate::db::VectorStore> =
            SqliteVectorStore::new(dir.path().to_str().unwrap())
                .await
                .unwrap();
        assert!(
            db.get_metadata("embedding_model").await.unwrap().is_none(),
            "fresh DB has no model recorded"
        );
        db.set_metadata("embedding_model", "my-model").await.unwrap();
        assert_eq!(
            db.get_metadata("embedding_model").await.unwrap().as_deref(),
            Some("my-model")
        );
    }

    #[tokio::test]
    async fn test_project_context_reflects_indexed_content() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("ctx.rs");
        std::fs::write(&file, b"fn bar() {}\nfn baz() {}\n").unwrap();

        let (idx, _) = make_indexer(&dir).await;
        idx.index_file(file.to_str().unwrap()).await.unwrap();

        let ctx = idx.project_context().await.unwrap();
        assert!(ctx.chunk_count > 0, "project context should reflect indexed chunks");
        assert_eq!(ctx.file_count, 1);
    }

    // ── Mock-based tests ──────────────────────────────────────────────────────────
    // [MOCK] The MockVectorStore below replaces SQLite with an in-memory store,
    // letting us test Indexer behaviour without any SQLite I/O.
    mod mocked {
        use super::*;
        use std::collections::HashMap;
        use std::sync::Mutex;
        use async_trait::async_trait;

        /// [MOCK] In-memory VectorStore for Indexer unit testing.
        struct MockVectorStore {
            chunks: Mutex<Vec<crate::db::ChunkRecord>>,
            hashes: Mutex<HashMap<String, String>>,
            metadata: Mutex<HashMap<String, String>>,
        }

        impl MockVectorStore {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    chunks: Mutex::new(vec![]),
                    hashes: Mutex::new(HashMap::new()),
                    metadata: Mutex::new(HashMap::new()),
                })
            }

            fn chunk_count_for_file(&self, file_path: &str) -> usize {
                self.chunks
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.file_path == file_path)
                    .count()
            }
        }

        #[async_trait]
        impl crate::db::VectorStore for MockVectorStore {
            async fn ensure_tables(&self) -> anyhow::Result<()> {
                Ok(())
            }

            async fn insert_chunks(&self, chunks: &[crate::db::ChunkRecord]) -> anyhow::Result<()> {
                let mut store = self.chunks.lock().unwrap();
                let mut hashes = self.hashes.lock().unwrap();
                for c in chunks {
                    store.push(c.clone());
                    hashes.insert(c.file_path.clone(), c.hash.clone());
                }
                Ok(())
            }

            async fn remove_chunks_for_file(&self, file_path: &str) -> anyhow::Result<()> {
                self.chunks.lock().unwrap().retain(|c| c.file_path != file_path);
                self.hashes.lock().unwrap().remove(file_path);
                Ok(())
            }

            async fn similarity_search(
                &self,
                _embedding: &[f32],
                top_k: usize,
            ) -> anyhow::Result<Vec<crate::db::SearchResult>> {
                let results: Vec<crate::db::SearchResult> = self
                    .chunks
                    .lock()
                    .unwrap()
                    .iter()
                    .take(top_k)
                    .map(|c| crate::db::SearchResult {
                        path: c.file_path.clone(),
                        line: c.line,
                        text: c.text.clone(),
                        score: 1.0,
                    })
                    .collect();
                Ok(results)
            }

            async fn get_all_file_paths(&self) -> anyhow::Result<Vec<String>> {
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

            async fn count_chunks(&self) -> anyhow::Result<usize> {
                Ok(self.chunks.lock().unwrap().len())
            }

            async fn get_last_hash(&self, file_path: &str) -> anyhow::Result<Option<String>> {
                Ok(self.hashes.lock().unwrap().get(file_path).cloned())
            }

            async fn get_metadata(&self, key: &str) -> anyhow::Result<Option<String>> {
                Ok(self.metadata.lock().unwrap().get(key).cloned())
            }

            async fn set_metadata(&self, key: &str, value: &str) -> anyhow::Result<()> {
                self.metadata
                    .lock()
                    .unwrap()
                    .insert(key.to_string(), value.to_string());
                Ok(())
            }

            async fn clear_all_chunks(&self) -> anyhow::Result<()> {
                self.chunks.lock().unwrap().clear();
                self.hashes.lock().unwrap().clear();
                Ok(())
            }
        }

        async fn make_mock_indexer(
            root: &str,
        ) -> (Indexer, Arc<MockVectorStore>, Arc<StubProvider>) {
            let mock_db = MockVectorStore::new();
            let db: Arc<dyn crate::db::VectorStore> = mock_db.clone();
            let stub = StubProvider::new();
            let embedder: Arc<dyn EmbeddingProvider> = stub.clone();
            let idx = Indexer::new_with_parts(
                db,
                embedder,
                root.to_string(),
                "test-model".to_string(),
            )
            .await;
            (idx, mock_db, stub)
        }

        #[tokio::test]
        async fn test_mock_index_file_calls_embed_then_stores_chunk() {
            // Verifies that indexing a real file calls the embedder and stores at least one chunk.
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("mock_embed.rs");
            std::fs::write(&file, b"pub fn greet() -> &'static str { \"hello\" }\n").unwrap();

            let (idx, mock_db, stub) = make_mock_indexer(dir.path().to_str().unwrap()).await;
            let n = idx.index_file(file.to_str().unwrap()).await.unwrap();

            assert!(n > 0, "should produce at least one chunk");
            assert_eq!(mock_db.count_chunks().await.unwrap(), n);
            assert!(
                !stub.calls.lock().unwrap().is_empty(),
                "embedder must have been called at least once"
            );
        }

        #[tokio::test]
        async fn test_mock_index_file_skips_embed_when_hash_unchanged() {
            // Verifies that indexing the same file twice only calls the embedder once (embed_cache hit).
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("cache_test.rs");
            std::fs::write(&file, b"pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();

            let (idx, _mock_db, stub) = make_mock_indexer(dir.path().to_str().unwrap()).await;

            let n1 = idx.index_file(file.to_str().unwrap()).await.unwrap();
            let calls_after_first = stub.calls.lock().unwrap().len();
            assert_eq!(calls_after_first, n1, "one embed call per chunk on first index");

            // Second call: same content → embed_cache hit → no new embed calls
            let n2 = idx.index_file(file.to_str().unwrap()).await.unwrap();
            let calls_after_second = stub.calls.lock().unwrap().len();
            assert_eq!(n1, n2, "same file should produce same chunk count");
            assert_eq!(
                calls_after_first, calls_after_second,
                "no new embed calls when content is in cache"
            );
        }

        #[tokio::test]
        async fn test_mock_index_file_remove_stale_when_file_deleted() {
            // Verifies that indexing a deleted file removes its chunks from the store.
            let dir = TempDir::new().unwrap();
            let file = dir.path().join("gone.rs");
            std::fs::write(&file, b"pub fn gone() {}\n").unwrap();

            let (idx, mock_db, _stub) = make_mock_indexer(dir.path().to_str().unwrap()).await;
            let n = idx.index_file(file.to_str().unwrap()).await.unwrap();
            assert!(n > 0, "file should have been indexed");

            // Delete the file and re-index
            std::fs::remove_file(&file).unwrap();
            let n2 = idx.index_file(file.to_str().unwrap()).await.unwrap();

            assert_eq!(n2, 0, "deleted file produces zero chunks");
            assert_eq!(
                mock_db.chunk_count_for_file(file.to_str().unwrap()),
                0,
                "stale chunks for deleted file must be removed"
            );
        }

        #[tokio::test]
        async fn test_mock_sync_all_removes_deleted_files() {
            // Verifies that sync_all removes chunks for files that no longer exist on disk.
            let dir = TempDir::new().unwrap();
            let file_a = dir.path().join("stay.rs");
            let file_b = dir.path().join("leave.rs");
            std::fs::write(&file_a, b"pub fn stay() {}\n").unwrap();
            std::fs::write(&file_b, b"pub fn leave() {}\n").unwrap();

            let (idx, mock_db, _stub) = make_mock_indexer(dir.path().to_str().unwrap()).await;

            // Index both files
            idx.index_file(file_a.to_str().unwrap()).await.unwrap();
            idx.index_file(file_b.to_str().unwrap()).await.unwrap();
            assert!(mock_db.count_chunks().await.unwrap() > 0);

            // Delete one file then sync_all
            std::fs::remove_file(&file_b).unwrap();
            idx.sync_all().await.unwrap();

            assert_eq!(
                mock_db.chunk_count_for_file(file_b.to_str().unwrap()),
                0,
                "chunks for deleted file must be removed after sync_all"
            );
            assert!(
                mock_db.chunk_count_for_file(file_a.to_str().unwrap()) > 0,
                "chunks for remaining file must still be present"
            );
        }
    }
}
