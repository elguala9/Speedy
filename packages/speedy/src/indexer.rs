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
    embed_cache: Mutex<HashMap<String, Vec<f32>>>,
}

impl Indexer {
    pub async fn new(config: &Config) -> Result<Self> {
        let root = std::env::current_dir()
            .context("failed to get current directory")?
            .to_string_lossy()
            .to_string();
        let db: Arc<dyn VectorStore> = SqliteVectorStore::new(&root).await
            .context("failed to initialize vector database")?;
        let embedder = embed::create_provider(config);
        Ok(Self {
            db,
            embedder,
            root,
            embed_cache: Mutex::new(HashMap::new()),
        })
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
