use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

use crate::constants::HTTP_TIMEOUT_SINGLE_SECS;
use super::EmbeddingProvider;

pub struct OllamaProvider {
    pub(crate) model: String,
    pub(crate) base_url: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embedding: Vec<f32>,
}

impl OllamaProvider {
    pub fn new(model: &str, base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SINGLE_SECS))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            model: model.to_string(),
            base_url: base_url.to_string(),
            client,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        #[derive(serde::Serialize)]
        struct Request {
            model: String,
            prompt: String,
        }

        let request = Request {
            model: self.model.clone(),
            prompt: text.to_string(),
        };

        let url = format!("{}/api/embeddings", self.base_url.trim_end_matches('/'));
        let t_total = std::time::Instant::now();
        let t_send = std::time::Instant::now();
        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await?;
        let send_ms = t_send.elapsed().as_millis() as u64;
        let t_decode = std::time::Instant::now();
        let resp = response.json::<OllamaEmbedResponse>().await?;
        let decode_ms = t_decode.elapsed().as_millis() as u64;
        let total_ms = t_total.elapsed().as_millis() as u64;
        tracing::debug!(
            target: "ai-context",
            model = %self.model,
            text_len = text.len(),
            send_ms,
            decode_ms,
            total_ms,
            "ollama embed call"
        );

        Ok(resp.embedding)
    }
}

pub struct AgentProvider {
    command: String,
}

impl AgentProvider {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.to_string(),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for AgentProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if self.command.is_empty() {
            anyhow::bail!("AgentProvider: no command configured. Set SPEEDY_AGENT_COMMAND or use provider=ollama");
        }

        let output = tokio::process::Command::new(&self.command)
            .arg(text)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "AgentProvider: command '{}' failed: {}",
                self.command,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let stdout = String::from_utf8(output.stdout)?;
        let embedding: Vec<f32> = serde_json::from_str(stdout.trim())?;
        Ok(embedding)
    }
}
