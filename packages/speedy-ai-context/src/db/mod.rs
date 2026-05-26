use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod sqlite;
#[cfg(test)]
mod tests;

pub use sqlite::SqliteVectorStore;

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
    async fn get_file_meta(&self, file_path: &str) -> Result<Option<FileMeta>>;
    async fn ensure_tables(&self) -> Result<()>;
    async fn get_metadata(&self, key: &str) -> Result<Option<String>>;
    async fn set_metadata(&self, key: &str, value: &str) -> Result<()>;
    async fn clear_all_chunks(&self) -> Result<()>;
    async fn get_all_file_meta(&self) -> Result<HashMap<String, FileMeta>>;
    async fn replace_file_chunks_batch(
        &self,
        replacements: &[(String, Vec<ChunkRecord>)],
    ) -> Result<()>;
    async fn text_search(&self, pattern: &str, top_k: usize) -> Result<Vec<SearchResult>>;
}
