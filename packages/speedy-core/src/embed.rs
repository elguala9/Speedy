use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embedding: Vec<f32>,
}

impl OllamaProvider {
    pub fn new(model: &str, base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
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
        let resp = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await?
            .json::<OllamaEmbedResponse>()
            .await?;

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

pub fn create_provider(config: &Config) -> Arc<dyn EmbeddingProvider> {
    match config.provider_type.as_str() {
        "ollama" => Arc::new(OllamaProvider::new(&config.model, &config.ollama_url)),
        "agent" => Arc::new(AgentProvider::new(&config.agent_command)),
        other => panic!(
            "Unknown provider type '{}'. Use 'ollama' or 'agent'.",
            other
        ),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub struct StubProvider {
        pub calls: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl EmbeddingProvider for StubProvider {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.calls.lock().unwrap().push(text.to_string());
            Ok(vec![0.1, 0.2, 0.3])
        }
    }

    impl StubProvider {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
            })
        }
    }

    #[tokio::test]
    async fn test_stub_provider() {
        let p = StubProvider::new();
        let v = p.embed("hello").await.unwrap();
        assert_eq!(v, vec![0.1, 0.2, 0.3]);
        assert_eq!(p.calls.lock().unwrap()[0], "hello");
    }

    #[test]
    fn test_ollama_constructor() {
        let p = OllamaProvider::new("my-model", "http://my-url:9999");
        assert_eq!(p.model, "my-model");
        assert_eq!(p.base_url, "http://my-url:9999");
    }

    #[test]
    fn test_config_defaults_ollama() {
        let config = crate::config::Config::default();
        assert_eq!(config.provider_type, "ollama");
        assert_eq!(config.model, "all-minilm:l6-v2");
        assert_eq!(config.ollama_url, "http://localhost:11434");
    }

    #[test]
    fn test_create_provider_ollama() {
        let mut config = crate::config::Config::default();
        config.model = "nomic-embed-text".to_string();
        config.ollama_url = "http://10.0.0.1:11434".to_string();
        config.provider_type = "ollama".to_string();

        let _p = create_provider(&config);
    }

    #[test]
    fn test_create_provider_agent() {
        let mut config = crate::config::Config::default();
        config.provider_type = "agent".to_string();
        config.agent_command = "echo".to_string();

        let _p = create_provider(&config);
    }

    #[test]
    #[should_panic(expected = "Unknown provider type")]
    fn test_create_provider_invalid() {
        let mut config = crate::config::Config::default();
        config.provider_type = "invalid".to_string();
        let _p = create_provider(&config);
    }

    #[tokio::test]
    async fn test_stub_provider_tracks_multiple_calls() {
        let p = StubProvider::new();
        p.embed("first").await.unwrap();
        p.embed("second").await.unwrap();
        let calls = p.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], "first");
        assert_eq!(calls[1], "second");
    }

    #[test]
    fn test_ollama_constructor_url_trailing_slash() {
        let p = OllamaProvider::new("m", "http://host:11434/");
        assert_eq!(p.base_url, "http://host:11434/");
    }

    #[test]
    fn test_agent_provider_errors_with_empty_command() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let provider = AgentProvider::new("");
        let result = rt.block_on(provider.embed("test"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no command configured"));
    }
}
