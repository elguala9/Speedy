use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use super::EmbeddingProvider;

pub struct CascadeEmbeddingProvider {
    providers: Vec<Arc<dyn EmbeddingProvider>>,
}

impl CascadeEmbeddingProvider {
    pub fn new(providers: Vec<Arc<dyn EmbeddingProvider>>) -> Self {
        Self { providers }
    }
}

#[async_trait]
impl EmbeddingProvider for CascadeEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut last_err = anyhow::anyhow!("CascadeEmbeddingProvider: no providers configured");
        for provider in &self.providers {
            match provider.embed(text).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    tracing::warn!("cascade: provider failed, trying next: {e}");
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }
}
