use anyhow::Result;
use async_trait::async_trait;

pub mod cascade;
pub mod factory;
pub mod generative;
pub mod http;
pub mod legacy;
#[cfg(test)]
pub mod tests;

pub use cascade::CascadeEmbeddingProvider;
pub use factory::create_provider;
pub use generative::GenerativeEmbeddingProvider;
pub(crate) use generative::GenerativeProtocol;
pub use http::{AuthScheme, HttpEmbeddingProvider};
pub(crate) use http::HttpProtocol;
pub use legacy::{AgentProvider, OllamaProvider};

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }
}
