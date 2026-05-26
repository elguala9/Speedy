use speedy_core::config::Config;
use crate::constants::{EMBED_CACHE_MAX_ENTRIES, MMAP_THRESHOLD, PROGRESS_LOG_INTERVAL};
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
    pub index_concurrency: usize,
}

struct PreparedFile {
    file_path: String,
    chunks: Vec<crate::document::Chunk>,
    chunk_hashes: Vec<String>,
    file_hash: String,
    last_modified: String,
}

const METADATA_MODEL_KEY: &str = "embedding_model";

// ---------------------------------------------------------------------------
// IndexerBuilder — dependency-injection entry point
// ---------------------------------------------------------------------------

pub struct IndexerBuilder {
    db: Option<Arc<dyn VectorStore>>,
    embedder: Option<Arc<dyn EmbeddingProvider>>,
    root: Option<String>,
    model: String,
    index_concurrency: usize,
}

impl IndexerBuilder {
    pub fn new() -> Self {
        Self {
            db: None,
            embedder: None,
            root: None,
            model: "all-minilm".to_string(),
            index_concurrency: 4,
        }
    }

    pub fn db(mut self, db: Arc<dyn VectorStore>) -> Self {
        self.db = Some(db);
        self
    }

    pub fn embedder(mut self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn root(mut self, root: impl Into<String>) -> Self {
        self.root = Some(root.into());
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn concurrency(mut self, n: usize) -> Self {
        self.index_concurrency = n;
        self
    }

    pub async fn build(self) -> Result<Indexer> {
        let root = match self.root {
            Some(r) => r,
            None => std::env::current_dir()
                .context("failed to get current directory")?
                .to_string_lossy()
                .to_string(),
        };
        let db: Arc<dyn VectorStore> = match self.db {
            Some(db) => db,
            None => SqliteVectorStore::new(&root).await
                .context("failed to initialize vector database")?,
        };
        let embedder = self.embedder
            .ok_or_else(|| anyhow::anyhow!("embedder is required — call .embedder() on the builder"))?;
        Ok(Indexer {
            db,
            embedder,
            root,
            model: self.model,
            embed_cache: Mutex::new(HashMap::new()),
            index_concurrency: self.index_concurrency,
        })
    }
}

fn write_index_progress(root: &str, processed: usize, total: usize) {
    let _ = std::fs::write(
        Path::new(root).join(".speedy").join("index-progress.json"),
        format!("{{\"processed\":{processed},\"total\":{total}}}"),
    );
}

fn clear_index_progress(root: &str) {
    let _ = std::fs::remove_file(
        Path::new(root).join(".speedy").join("index-progress.json"),
    );
}

fn log_progress(
    root: &str,
    start: &Instant,
    done: usize,
    total: usize,
    chunks: usize,
    failures: usize,
    last_log: &mut Instant,
) {
    let elapsed = start.elapsed();
    let rate = done as f64 / elapsed.as_secs_f64().max(0.001);
    let remaining = total.saturating_sub(done);
    let eta_s = if rate > 0.0 { (remaining as f64 / rate) as u64 } else { 0 };
    tracing::info!(
        target: "ai-context",
        processed = done,
        total,
        chunks,
        failures,
        elapsed_s = elapsed.as_secs(),
        files_per_sec = format!("{rate:.2}"),
        eta_s,
        "index_directory progress"
    );
    write_index_progress(root, done, total);
    *last_log = Instant::now();
}

impl Indexer {
    pub async fn new(config: &Config) -> Result<Self> {
        let root = std::env::current_dir()
            .context("failed to get current directory")?
            .to_string_lossy()
            .to_string();

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

        let indexer = IndexerBuilder::new()
            .root(root)
            .model(config.model.clone())
            .concurrency(config.index_concurrency)
            .embedder(embedder)
            .build()
            .await?;

        // Compatibility check: warn if the DB was built with a different model
        // than what's configured now. Old chunks won't be in the same vector
        // space as new query embeddings, so similarity scores become garbage.
        // We warn but don't refuse — the user might be mid-transition and want
        // to run `reembed` next. Indexing only writes the marker once.
        let chunk_count = indexer.db.count_chunks().await.unwrap_or(0);
        match indexer.db.get_metadata(METADATA_MODEL_KEY).await? {
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
                indexer.db.set_metadata(METADATA_MODEL_KEY, &config.model).await?;
            }
            None => {
                indexer.db.set_metadata(METADATA_MODEL_KEY, &config.model).await?;
            }
            _ => {}
        }

        Ok(indexer)
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

    /// Read, hash-check, and chunk a file without performing any embedding or
    /// DB writes. Returns `None` if the file is unchanged (mtime/hash match)
    /// or should be skipped (oversized, binary, non-UTF-8).
    async fn prepare_file(
        &self,
        file_path: &str,
        file_meta_cache: &HashMap<String, crate::db::FileMeta>,
    ) -> Result<Option<PreparedFile>> {
        let path = Path::new(file_path);
        if !path.exists() {
            return Ok(None);
        }

        let metadata = fs::metadata(path).await
            .context(format!("failed to read metadata for: {file_path}"))?;

        if metadata.len() > crate::MAX_INDEXABLE_FILE_SIZE {
            tracing::warn!(
                file = %file_path,
                size = metadata.len(),
                limit = crate::MAX_INDEXABLE_FILE_SIZE,
                "skipping oversized file"
            );
            return Ok(None);
        }

        let last_modified = metadata.modified().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        }).unwrap_or_default();

        // Mtime pre-filter: skip file read if mtime unchanged.
        if !last_modified.is_empty() {
            if let Some(m) = file_meta_cache.get(file_path) {
                if m.last_modified == last_modified {
                    return Ok(None);
                }
            }
        }

        // Move the CPU-heavy work (read + hash + chunk) off the async reactor.
        // For files >= 64 KB we memory-map instead of heap-copying the bytes.
        let stored_meta = file_meta_cache.get(file_path).cloned();
        let file_path_owned = file_path.to_string();
        let size = metadata.len();

        let blocking_result = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<(String, Vec<crate::document::Chunk>, Vec<String>)>> {
            let (file_hash, chunks) = if size >= MMAP_THRESHOLD {
                let file = std::fs::File::open(&file_path_owned)?;
                // SAFETY: the file is only read; no writer is assumed during indexing.
                let mmap = unsafe { memmap2::Mmap::map(&file)? };
                drop(file);
                let s = match std::str::from_utf8(&mmap) {
                    Ok(s) => s,
                    Err(_) => {
                        tracing::debug!(file = %file_path_owned, "skipping non-UTF-8 file");
                        return Ok(None);
                    }
                };
                let fh = hash::hash_bytes(s.as_bytes());
                if let Some(ref m) = stored_meta {
                    if m.hash == fh { return Ok(None); }
                }
                let ch = document::Document::chunk_file(s, 1000, 200);
                (fh, ch)
            } else {
                let bytes = match std::fs::read(&file_path_owned) {
                    Ok(b) => b,
                    Err(_) => return Ok(None),
                };
                let s = match String::from_utf8(bytes) {
                    Ok(s) => s,
                    Err(_) => {
                        tracing::debug!(file = %file_path_owned, "skipping non-UTF-8 file");
                        return Ok(None);
                    }
                };
                let fh = hash::hash_bytes(s.as_bytes());
                if let Some(ref m) = stored_meta {
                    if m.hash == fh { return Ok(None); }
                }
                let ch = document::Document::chunk_file(&s, 1000, 200);
                (fh, ch)
            };
            let chunk_hashes = chunks.iter()
                .map(|c| hash::hash_bytes(c.text.as_bytes()))
                .collect::<Vec<_>>();
            Ok(Some((file_hash, chunks, chunk_hashes)))
        })
        .await
        .context("file processing task panicked")?
        .context(format!("failed to process: {file_path}"))?;

        let Some((file_hash, chunks, chunk_hashes)) = blocking_result else {
            return Ok(None);
        };

        Ok(Some(PreparedFile {
            file_path: file_path.to_string(),
            chunks,
            chunk_hashes,
            file_hash,
            last_modified,
        }))
    }

    pub async fn index_directory(&self, path: &str) -> Result<IndexStats> {
        let start = Instant::now();

        let t_walk = Instant::now();
        let filter = FileFilter::new(path);
        let files: Vec<String> = filter.filtered_files().into_iter()
            .filter(|f| !FileFilter::is_binary(Path::new(f)))
            .collect();
        let walk_ms = t_walk.elapsed().as_millis() as u64;
        let total = files.len();

        // Load all stored file meta in one DB query — eliminates per-file round-trips.
        let file_meta_cache = self.db.get_all_file_meta().await
            .unwrap_or_default();

        tracing::info!(
            target: "ai-context",
            root = %path,
            files = total,
            cached_files = file_meta_cache.len(),
            walk_ms,
            "index_directory: file scan done, starting per-file indexing"
        );

        let concurrency = self.index_concurrency;
        let mut total_chunks = 0usize;
        let mut failures = 0usize;
        let mut done = 0usize;
        let mut last_progress_log = Instant::now();

        write_index_progress(path, 0, total);

        let pb = indicatif::ProgressBar::new(total as u64);
        pb.set_style(indicatif::ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files ({per_sec}) {msg}")
            .unwrap()
            .progress_chars("##-"));

        for batch in files.chunks(concurrency) {
            // Phase 1: read + chunk all files in this batch concurrently (no embed yet).
            let prep_futs: Vec<_> = batch.iter()
                .map(|f| self.prepare_file(f, &file_meta_cache))
                .collect();
            let prep_results = futures::future::join_all(prep_futs).await;

            let mut prepared: Vec<PreparedFile> = Vec::new();
            for (file_path, result) in batch.iter().zip(prep_results) {
                match result {
                    Ok(Some(p)) => prepared.push(p),
                    Ok(None) => {}
                    Err(e) => {
                        error!("Failed to prepare {file_path}: {e:#}");
                        tracing::error!(
                            target: "ai-context",
                            file = %file_path,
                            error = %format!("{:#}", e),
                            "file prepare failed"
                        );
                        failures += 1;
                    }
                }
                done += 1;
                pb.inc(1);
            }

            if prepared.is_empty() {
                if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
                    log_progress(path, &start, done, total, total_chunks, failures, &mut last_progress_log);
                }
                continue;
            }

            // Phase 2: collect embeddings needed — split into cache hits and misses.
            // Pre-extracting hits before the embed call guards against eviction
            // during Phase 3 overwriting a hash we already resolved.
            let mut hit_map: HashMap<String, Vec<f32>> = HashMap::new();
            let mut miss_hashes: Vec<String> = Vec::new();
            let mut miss_texts: Vec<String> = Vec::new();
            let mut miss_set: std::collections::HashSet<String> = std::collections::HashSet::new();
            {
                let cache = self.embed_cache.lock().await;
                for prep in &prepared {
                    for (ci, hash) in prep.chunk_hashes.iter().enumerate() {
                        if hit_map.contains_key(hash.as_str()) {
                            continue;
                        }
                        if let Some(emb) = cache.get(hash) {
                            hit_map.insert(hash.clone(), emb.clone());
                        } else if miss_set.insert(hash.clone()) {
                            miss_hashes.push(hash.clone());
                            miss_texts.push(prep.chunks[ci].text.clone());
                        }
                    }
                }
            }

            // Phase 3: single embed_batch for ALL uncached chunks across the batch.
            let new_map: HashMap<String, Vec<f32>> = if !miss_texts.is_empty() {
                let texts: Vec<&str> = miss_texts.iter().map(|s| s.as_str()).collect();
                let batch_embs = match self.embedder.embed_batch(&texts).await {
                    Ok(b) => b,
                    Err(e) => {
                        error!("embed_batch failed for batch ({} files): {e:#}", prepared.len());
                        failures += prepared.len();
                        done += 0;
                        if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
                            log_progress(path, &start, done, total, total_chunks, failures, &mut last_progress_log);
                        }
                        continue;
                    }
                };
                if batch_embs.len() != miss_hashes.len() {
                    error!(
                        "embed_batch returned {} vectors for {} texts — skipping batch",
                        batch_embs.len(), miss_hashes.len()
                    );
                    failures += prepared.len();
                    continue;
                }
                let mut cache = self.embed_cache.lock().await;
                let mut map = HashMap::with_capacity(miss_hashes.len());
                for (hash, emb) in miss_hashes.iter().zip(batch_embs) {
                    if cache.len() >= EMBED_CACHE_MAX_ENTRIES {
                        if let Some(victim) = cache.keys().next().cloned() {
                            cache.remove(&victim);
                        }
                    }
                    cache.insert(hash.clone(), emb.clone());
                    map.insert(hash.clone(), emb);
                }
                map
            } else {
                HashMap::new()
            };

            // Merge hits + new embeddings into a single lookup table.
            let all_embeddings: HashMap<String, Vec<f32>> = hit_map.into_iter()
                .chain(new_map)
                .collect();

            // Phase 4: build ChunkRecords and write all files in one DB transaction.
            let mut replacements: Vec<(String, Vec<crate::db::ChunkRecord>)> =
                Vec::with_capacity(prepared.len());
            for prep in &prepared {
                let records: Vec<crate::db::ChunkRecord> = prep.chunks.iter()
                    .zip(prep.chunk_hashes.iter())
                    .enumerate()
                    .map(|(ci, (chunk, hash))| crate::db::ChunkRecord {
                        id: format!("{}:{}", prep.file_path, ci),
                        file_path: prep.file_path.clone(),
                        line: chunk.line,
                        text: chunk.text.clone(),
                        hash: prep.file_hash.clone(),
                        embedding: all_embeddings.get(hash.as_str())
                            .cloned()
                            .expect("every chunk must have an embedding"),
                        last_modified: prep.last_modified.clone(),
                    })
                    .collect();
                total_chunks += records.len();
                replacements.push((prep.file_path.clone(), records));
            }

            if let Err(e) = self.db.replace_file_chunks_batch(&replacements).await {
                error!("DB batch write failed: {e:#}");
                failures += replacements.len();
            } else {
                if let Some(last) = replacements.last() {
                    let short = if last.0.len() > 50 {
                        format!("...{}", &last.0[last.0.len()-47..])
                    } else {
                        last.0.clone()
                    };
                    pb.set_message(short);
                }
            }

            if last_progress_log.elapsed() >= PROGRESS_LOG_INTERVAL {
                log_progress(path, &start, done, total, total_chunks, failures, &mut last_progress_log);
            }
        }

        pb.finish_and_clear();
        clear_index_progress(path);

        let duration_ms = start.elapsed().as_millis() as u64;
        tracing::info!(
            target: "ai-context",
            root = %path,
            files = total,
            chunks = total_chunks,
            failures,
            walk_ms,
            duration_ms,
            files_per_sec = format!("{:.2}", total as f64 / start.elapsed().as_secs_f64().max(0.001)),
            "index_directory complete"
        );

        Ok(IndexStats {
            files: total,
            chunks: total_chunks,
            removed: 0,
            duration_ms,
        })
    }

    pub async fn index_file(&self, file_path: &str) -> Result<usize> {
        let t_total = Instant::now();
        let path = Path::new(file_path);
        if !path.exists() {
            self.db.remove_chunks_for_file(file_path).await?;
            return Ok(0);
        }

        let metadata = fs::metadata(path).await
            .context(format!("failed to read metadata for: {file_path}"))?;

        // Size guard: refuse to load multi-MB/GB files into memory. Without
        // this, a single oversized file (generated Dart, build artifact,
        // data dump) makes `read_to_string` allocate the whole file at once
        // and abort the process with OOM. See lib.rs for the rationale.
        if metadata.len() > crate::MAX_INDEXABLE_FILE_SIZE {
            tracing::warn!(
                file = %file_path,
                size = metadata.len(),
                limit = crate::MAX_INDEXABLE_FILE_SIZE,
                "skipping oversized file"
            );
            self.db.remove_chunks_for_file(file_path).await?;
            return Ok(0);
        }

        // Compute mtime before any file read — used for the fast-path skip.
        let last_modified = metadata.modified().ok().map(|t| {
            let dt: chrono::DateTime<Utc> = t.into();
            dt.to_rfc3339()
        }).unwrap_or_default();

        // One DB query gives us both stored hash and stored mtime.
        let stored_meta = self.db.get_file_meta(file_path).await.unwrap_or(None);

        // Mtime pre-filter: skip file read entirely if mtime is unchanged.
        if !last_modified.is_empty() {
            if let Some(ref m) = stored_meta {
                if m.last_modified == last_modified {
                    return Ok(0);
                }
            }
        }

        let t_read = Instant::now();
        let bytes = match fs::read(path).await {
            Ok(b) => b,
            Err(_) => return Ok(0),
        };
        // Skip files that aren't valid UTF-8 — they're binary blobs that
        // slipped past the extension filter (no point chunking them).
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                tracing::debug!(file = %file_path, "skipping non-UTF-8 file");
                return Ok(0);
            }
        };
        let read_ms = t_read.elapsed().as_millis() as u64;
        let bytes_len = content.len();
        let file_hash = hash::hash_bytes(content.as_bytes());

        // Hash fallback: mtime changed (e.g. git checkout) but content is identical.
        if let Some(ref m) = stored_meta {
            if m.hash == file_hash {
                return Ok(0);
            }
        }

        let t_chunk = Instant::now();
        let chunks = document::Document::chunk_file(&content, 1000, 200);
        let chunk_ms = t_chunk.elapsed().as_millis() as u64;

        let mut embed_total_ms: u64 = 0;
        let mut embed_calls: usize = 0;
        let mut embed_cache_hits: usize = 0;
        let mut embed_max_ms: u64 = 0;

        // Precompute chunk hashes; check cache for each chunk.
        let chunk_hashes: Vec<String> = chunks.iter()
            .map(|c| hash::hash_bytes(c.text.as_bytes()))
            .collect();
        let mut embeddings: Vec<Option<Vec<f32>>> = vec![None; chunks.len()];
        let mut uncached_indices: Vec<usize> = Vec::new();
        {
            let cache = self.embed_cache.lock().await;
            for (i, ch) in chunk_hashes.iter().enumerate() {
                if let Some(emb) = cache.get(ch) {
                    embed_cache_hits += 1;
                    embeddings[i] = Some(emb.clone());
                } else {
                    uncached_indices.push(i);
                }
            }
        }

        // Batch-embed all uncached chunks in a single provider call.
        if !uncached_indices.is_empty() {
            let texts: Vec<&str> = uncached_indices.iter()
                .map(|&i| chunks[i].text.as_str())
                .collect();
            let t_embed = Instant::now();
            let batch = self.embedder.embed_batch(&texts).await
                .context(format!("failed to embed chunks in {file_path}"))?;
            let embed_ms = t_embed.elapsed().as_millis() as u64;
            embed_total_ms += embed_ms;
            embed_calls += 1;
            if embed_ms > embed_max_ms { embed_max_ms = embed_ms; }
            if embed_ms > 2000 {
                tracing::warn!(
                    target: "ai-context",
                    file = %file_path,
                    chunks = texts.len(),
                    embed_ms,
                    "slow embed batch (>2s)"
                );
            }
            let mut cache = self.embed_cache.lock().await;
            for (pos, &chunk_idx) in uncached_indices.iter().enumerate() {
                let emb = batch[pos].clone();
                let ch = &chunk_hashes[chunk_idx];
                if cache.len() >= EMBED_CACHE_MAX_ENTRIES {
                    if let Some(victim) = cache.keys().next().cloned() {
                        cache.remove(&victim);
                    }
                }
                cache.insert(ch.clone(), emb.clone());
                embeddings[chunk_idx] = Some(emb);
            }
        }

        let mut records = Vec::with_capacity(chunks.len());
        for (idx, (chunk, emb)) in chunks.iter().zip(embeddings).enumerate() {
            records.push(ChunkRecord {
                id: format!("{}:{}", file_path, idx),
                file_path: file_path.to_string(),
                line: chunk.line,
                text: chunk.text.clone(),
                hash: file_hash.clone(),
                embedding: emb.expect("every chunk must have an embedding"),
                last_modified: last_modified.clone(),
            });
        }

        let t_db = Instant::now();
        self.db.remove_chunks_for_file(file_path).await
            .context(format!("failed to remove old chunks for: {file_path}"))?;
        self.db.insert_chunks(&records).await
            .context("failed to insert chunks into database")?;
        let db_ms = t_db.elapsed().as_millis() as u64;

        let total_ms = t_total.elapsed().as_millis() as u64;
        tracing::info!(
            target: "ai-context",
            file = %file_path,
            bytes = bytes_len,
            chunks = chunks.len(),
            embed_calls,
            embed_cache_hits,
            embed_total_ms,
            embed_avg_ms = if embed_calls > 0 { embed_total_ms / embed_calls as u64 } else { 0 },
            embed_max_ms,
            read_ms,
            chunk_ms,
            db_ms,
            total_ms,
            "file indexed"
        );

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
            index_concurrency: 4,
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

    /// Regression test for the OOM bug: oversized files must be skipped
    /// without ever being read into memory. We write a file just over the
    /// size limit and check that index_file returns Ok(0) and emits no
    /// embed calls.
    #[tokio::test]
    async fn test_index_file_skips_oversized_file() {
        let dir = TempDir::new().unwrap();
        let huge = dir.path().join("huge.rs");
        // 5 MiB + 1 byte — just over the cap.
        let size = (crate::MAX_INDEXABLE_FILE_SIZE as usize) + 1;
        std::fs::write(&huge, vec![b'x'; size]).unwrap();

        let (idx, stub) = make_indexer(&dir).await;
        let n = idx.index_file(huge.to_str().unwrap()).await.unwrap();
        assert_eq!(n, 0, "oversized file must produce zero chunks");
        assert!(
            stub.calls.lock().unwrap().is_empty(),
            "oversized file must not trigger any embed call"
        );
    }

    /// Regression test for the chunker infinite-loop bug: a file with a
    /// `\n\n` separator just past the overlap boundary used to make
    /// `chunk_file` spin forever or allocate billions of windows. With the
    /// forward-progress invariant in place it must terminate quickly.
    #[test]
    fn test_chunk_file_forward_progress_on_adversarial_overlap() {
        // 1 KiB of garbage, a `\n\n` at index 199-200, then more garbage.
        // chunk_size=1000, overlap=200 (the production defaults).
        let mut content = String::with_capacity(3000);
        content.push_str(&"x".repeat(199));
        content.push_str("\n\n");
        content.push_str(&"y".repeat(2000));
        let chunks = crate::document::Document::chunk_file(&content, 1000, 200);
        // Must terminate; with the bug it spun forever.
        assert!(
            chunks.len() < 20,
            "expected modest chunk count, got {}",
            chunks.len()
        );
        // And cover the whole input.
        let last = chunks.last().unwrap();
        assert!(last.text.ends_with('y'), "last chunk should reach end of content");
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

        // Second call: same content → file-level hash-skip → 0 chunks returned, no new embed calls
        let n2 = idx.index_file(file.to_str().unwrap()).await.unwrap();
        let calls_after_second = stub.calls.lock().unwrap().len();
        assert_eq!(n2, 0, "unchanged file is skipped (hash-skip fast path)");
        assert_eq!(
            calls_after_first, calls_after_second,
            "no new embed calls when file hash is unchanged"
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

            async fn get_file_meta(&self, file_path: &str) -> anyhow::Result<Option<crate::db::FileMeta>> {
                Ok(self.chunks.lock().unwrap()
                    .iter()
                    .find(|c| c.file_path == file_path)
                    .map(|c| crate::db::FileMeta {
                        hash: c.hash.clone(),
                        last_modified: c.last_modified.clone(),
                    }))
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

            async fn get_all_file_meta(&self) -> anyhow::Result<HashMap<String, crate::db::FileMeta>> {
                let chunks = self.chunks.lock().unwrap();
                let mut map = HashMap::new();
                for c in chunks.iter() {
                    map.entry(c.file_path.clone()).or_insert(crate::db::FileMeta {
                        hash: c.hash.clone(),
                        last_modified: c.last_modified.clone(),
                    });
                }
                Ok(map)
            }

            async fn replace_file_chunks_batch(
                &self,
                replacements: &[(String, Vec<crate::db::ChunkRecord>)],
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

            async fn text_search(
                &self,
                pattern: &str,
                top_k: usize,
            ) -> anyhow::Result<Vec<crate::db::SearchResult>> {
                let results: Vec<crate::db::SearchResult> = self
                    .chunks
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| c.text.contains(pattern))
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

            // Second call: same content → file-level hash-skip → 0 returned, no new embed calls
            let n2 = idx.index_file(file.to_str().unwrap()).await.unwrap();
            let calls_after_second = stub.calls.lock().unwrap().len();
            assert_eq!(n2, 0, "unchanged file is skipped (hash-skip fast path)");
            assert_eq!(
                calls_after_first, calls_after_second,
                "no new embed calls when file hash is unchanged"
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
