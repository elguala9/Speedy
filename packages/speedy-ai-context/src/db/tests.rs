use super::*;

#[tokio::test]
async fn test_sqlite_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();

    let store = SqliteVectorStore::new(dir.path().to_str().unwrap())
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
}

#[tokio::test]
async fn test_metadata_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

    assert!(store.get_metadata("embedding_model").await.unwrap().is_none());
    store.set_metadata("embedding_model", "nomic-embed-text").await.unwrap();
    assert_eq!(
        store.get_metadata("embedding_model").await.unwrap().as_deref(),
        Some("nomic-embed-text"),
    );
    store.set_metadata("embedding_model", "all-minilm").await.unwrap();
    assert_eq!(
        store.get_metadata("embedding_model").await.unwrap().as_deref(),
        Some("all-minilm"),
    );
}

#[tokio::test]
async fn test_clear_all_chunks_empties_store() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

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
    assert!(store.similarity_search(&[1.0, 0.0], 5).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_migration_from_old_blob_schema() {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = speedy_core::daemon_util::workspace_data_dir(dir.path());
    let db_path = data_dir.join("sac.sqlite");

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
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();
    store.ensure_tables().await.unwrap();
    store.ensure_tables().await.unwrap();
    assert_eq!(store.count_chunks().await.unwrap(), 0);
}

#[tokio::test]
async fn test_insert_chunks_after_clear_accepts_different_dimension() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

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
    let dir = tempfile::TempDir::new().unwrap();

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
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();
        store.insert_chunks(&records).await.unwrap();
    }

    {
        let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 1);
        let results = store.similarity_search(&[0.0, 0.0, 0.99], 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "lib.rs");
    }
}

#[tokio::test]
async fn test_remove_chunks_for_file_leaves_others_intact() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

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
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

    let result = store.get_last_hash("nonexistent.rs").await.unwrap();
    assert!(result.is_none(), "hash for unknown file should be None");
}

#[tokio::test]
async fn test_get_all_file_paths_after_multi_insert() {
    use std::collections::HashSet;

    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

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

#[tokio::test]
async fn test_text_search_finds_keyword() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

    let chunks = vec![
        ChunkRecord {
            id: "s1".to_string(),
            file_path: "auth.rs".to_string(),
            line: 10,
            text: "fn authenticate(user: &str) -> bool { true }".to_string(),
            hash: "h1".to_string(),
            embedding: vec![1.0, 0.0],
            last_modified: "2024-01-01".to_string(),
        },
        ChunkRecord {
            id: "s2".to_string(),
            file_path: "main.rs".to_string(),
            line: 1,
            text: "fn main() { println!(\"hello world\"); }".to_string(),
            hash: "h2".to_string(),
            embedding: vec![0.0, 1.0],
            last_modified: "2024-01-01".to_string(),
        },
    ];
    store.insert_chunks(&chunks).await.unwrap();

    let results = store.text_search("authenticate", 10).await.unwrap();
    assert_eq!(results.len(), 1, "should find exactly one match");
    assert_eq!(results[0].path, "auth.rs");
    assert_eq!(results[0].line, 10);
}

#[tokio::test]
async fn test_text_search_no_results() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

    let chunks = vec![ChunkRecord {
        id: "s1".to_string(),
        file_path: "lib.rs".to_string(),
        line: 1,
        text: "pub fn add(a: i32, b: i32) -> i32 { a + b }".to_string(),
        hash: "h1".to_string(),
        embedding: vec![1.0, 0.0],
        last_modified: "2024-01-01".to_string(),
    }];
    store.insert_chunks(&chunks).await.unwrap();

    let results = store.text_search("nonexistent_symbol_xyz", 10).await.unwrap();
    assert!(results.is_empty(), "should find no matches");
}

#[tokio::test]
async fn test_text_search_respects_top_k() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = SqliteVectorStore::new(dir.path().to_str().unwrap()).await.unwrap();

    let chunks: Vec<ChunkRecord> = (0..5)
        .map(|i| ChunkRecord {
            id: format!("t{i}"),
            file_path: format!("f{i}.rs"),
            line: i,
            text: format!("fn handler_{i}() {{ }}"),
            hash: format!("h{i}"),
            embedding: vec![1.0, 0.0],
            last_modified: "2024-01-01".to_string(),
        })
        .collect();
    store.insert_chunks(&chunks).await.unwrap();

    let results = store.text_search("handler", 3).await.unwrap();
    assert_eq!(results.len(), 3, "top_k=3 should limit to 3 results");
}

mod mocked {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use anyhow::anyhow;
    use async_trait::async_trait;

    struct MockVectorStore {
        chunks: Mutex<Vec<ChunkRecord>>,
        hashes: Mutex<HashMap<String, String>>,
        metadata: Mutex<HashMap<String, String>>,
        fail_insert: Mutex<bool>,
    }

    #[allow(dead_code)]
    impl MockVectorStore {
        fn new() -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
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
        async fn ensure_tables(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn insert_chunks(&self, chunks: &[ChunkRecord]) -> anyhow::Result<()> {
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

        async fn remove_chunks_for_file(&self, file_path: &str) -> anyhow::Result<()> {
            self.chunks.lock().unwrap().retain(|c| c.file_path != file_path);
            self.hashes.lock().unwrap().remove(file_path);
            Ok(())
        }

        async fn similarity_search(&self, _embedding: &[f32], top_k: usize) -> anyhow::Result<Vec<SearchResult>> {
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

        async fn get_file_meta(&self, file_path: &str) -> anyhow::Result<Option<FileMeta>> {
            Ok(self.chunks.lock().unwrap()
                .iter()
                .find(|c| c.file_path == file_path)
                .map(|c| FileMeta {
                    hash: c.hash.clone(),
                    last_modified: c.last_modified.clone(),
                }))
        }

        async fn get_metadata(&self, key: &str) -> anyhow::Result<Option<String>> {
            Ok(self.metadata.lock().unwrap().get(key).cloned())
        }

        async fn set_metadata(&self, key: &str, value: &str) -> anyhow::Result<()> {
            self.metadata.lock().unwrap().insert(key.to_string(), value.to_string());
            Ok(())
        }

        async fn clear_all_chunks(&self) -> anyhow::Result<()> {
            self.chunks.lock().unwrap().clear();
            self.hashes.lock().unwrap().clear();
            Ok(())
        }

        async fn get_all_file_meta(&self) -> anyhow::Result<HashMap<String, FileMeta>> {
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
        ) -> anyhow::Result<()> {
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

        async fn text_search(&self, pattern: &str, top_k: usize) -> anyhow::Result<Vec<SearchResult>> {
            let results: Vec<SearchResult> = self
                .chunks
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.text.contains(pattern))
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
        let store = MockVectorStore::new();
        store.insert_chunks(&[make_chunk("c1", "a.rs"), make_chunk("c2", "b.rs")]).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_mock_remove_clears_chunks_and_hash() {
        let store = MockVectorStore::new();
        store.insert_chunks(&[make_chunk("c1", "a.rs")]).await.unwrap();
        assert_eq!(store.get_last_hash("a.rs").await.unwrap(), Some("hash_c1".to_string()));

        store.remove_chunks_for_file("a.rs").await.unwrap();

        assert_eq!(store.count_chunks().await.unwrap(), 0);
        assert!(store.get_last_hash("a.rs").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_mock_insert_error_propagates() {
        let store = MockVectorStore::new();
        store.set_fail_insert(true);
        let result = store.insert_chunks(&[make_chunk("c1", "a.rs")]).await;
        assert!(result.is_err(), "expected error from mock insert");
        assert_eq!(store.count_chunks().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_mock_clear_all_resets_state() {
        let store = MockVectorStore::new();
        store.insert_chunks(&[make_chunk("c1", "a.rs"), make_chunk("c2", "b.rs")]).await.unwrap();
        assert_eq!(store.count_chunks().await.unwrap(), 2);

        store.clear_all_chunks().await.unwrap();

        assert_eq!(store.count_chunks().await.unwrap(), 0);
        assert!(store.get_last_hash("a.rs").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_mock_get_last_hash_tracks_inserts() {
        let store = MockVectorStore::new();
        store.insert_chunks(&[make_chunk("c1", "src/lib.rs")]).await.unwrap();
        assert_eq!(
            store.get_last_hash("src/lib.rs").await.unwrap(),
            Some("hash_c1".to_string())
        );
    }

    #[tokio::test]
    async fn test_mock_metadata_roundtrip() {
        let store = MockVectorStore::new();
        assert!(store.get_metadata("model").await.unwrap().is_none());
        store.set_metadata("model", "test-model").await.unwrap();
        assert_eq!(store.get_metadata("model").await.unwrap(), Some("test-model".to_string()));
    }

    #[tokio::test]
    async fn test_mock_similarity_search_respects_top_k() {
        let store = MockVectorStore::new();
        let chunks: Vec<ChunkRecord> = (0..5)
            .map(|i| make_chunk(&format!("c{}", i), &format!("f{}.rs", i)))
            .collect();
        store.insert_chunks(&chunks).await.unwrap();

        let results = store.similarity_search(&[0.1, 0.2, 0.3], 3).await.unwrap();
        assert_eq!(results.len(), 3, "top_k=3 should return exactly 3 results");
    }
}
