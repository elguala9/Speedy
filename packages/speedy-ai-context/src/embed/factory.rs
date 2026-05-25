use std::sync::Arc;
use speedy_core::config::Config;

use super::{
    AgentProvider, AuthScheme, EmbeddingProvider,
    GenerativeEmbeddingProvider, GenerativeProtocol, HttpEmbeddingProvider, HttpProtocol,
};

pub fn create_provider(config: &Config) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
    let provider_type = config.provider.provider_type.as_deref()
        .unwrap_or(&config.provider_type);
    let model = config.provider.model.as_deref()
        .unwrap_or(&config.model);
    let base_url = config.provider.base_url.as_deref()
        .unwrap_or(&config.ollama_url);
    let command = config.provider.command.as_deref()
        .unwrap_or(&config.agent_command);
    let api_key = config.provider.api_key.as_deref();
    let dims = config.provider.dims.unwrap_or(384);

    match provider_type {
        "ollama" => {
            Ok(Arc::new(HttpEmbeddingProvider::new(
                base_url,
                model,
                AuthScheme::None,
                HttpProtocol::Ollama,
            )))
        }
        "openai" => {
            let key = require_api_key(provider_type, api_key)?;
            Ok(Arc::new(HttpEmbeddingProvider::new(
                "https://api.openai.com",
                model,
                AuthScheme::BearerToken(key),
                HttpProtocol::OpenAI,
            )))
        }
        "openai-compatible" => {
            let bu = config.provider.base_url.as_deref()
                .ok_or_else(|| anyhow::anyhow!(
                    "Provider 'openai-compatible' requires a 'base_url' field."
                ))?;
            let auth = api_key
                .map(|k| AuthScheme::BearerToken(k.to_string()))
                .unwrap_or(AuthScheme::None);
            Ok(Arc::new(HttpEmbeddingProvider::new(bu, model, auth, HttpProtocol::OpenAI)))
        }
        "azure-openai" => {
            let bu = config.provider.base_url.as_deref()
                .ok_or_else(|| anyhow::anyhow!(
                    "Provider 'azure-openai' requires a 'base_url' field."
                ))?;
            let key = require_api_key(provider_type, api_key)?;
            Ok(Arc::new(HttpEmbeddingProvider::new(
                bu,
                model,
                AuthScheme::ApiKeyHeader {
                    header: "api-key".to_string(),
                    value: key,
                },
                HttpProtocol::OpenAI,
            )))
        }
        "gemini" => {
            let key = require_api_key(provider_type, api_key)?;
            Ok(Arc::new(HttpEmbeddingProvider::new(
                "https://generativelanguage.googleapis.com",
                model,
                AuthScheme::QueryParam {
                    param: "key".to_string(),
                    value: key,
                },
                HttpProtocol::Gemini,
            )))
        }
        "anthropic" => {
            let key = require_api_key(provider_type, api_key)?;
            Ok(Arc::new(GenerativeEmbeddingProvider::new(
                "https://api.anthropic.com/v1/messages",
                model,
                AuthScheme::ApiKeyHeader {
                    header: "x-api-key".to_string(),
                    value: key,
                },
                dims,
                GenerativeProtocol::Anthropic,
            )))
        }
        "deepseek" => {
            let key = require_api_key(provider_type, api_key)?;
            Ok(Arc::new(GenerativeEmbeddingProvider::new(
                "https://api.deepseek.com/chat/completions",
                model,
                AuthScheme::BearerToken(key),
                dims,
                GenerativeProtocol::OpenAIChat,
            )))
        }
        "agent" => {
            Ok(Arc::new(AgentProvider::new(command)))
        }
        unknown => {
            if let Some(bu) = config.provider.base_url.as_deref() {
                let auth = api_key
                    .map(|k| AuthScheme::BearerToken(k.to_string()))
                    .unwrap_or(AuthScheme::None);
                Ok(Arc::new(HttpEmbeddingProvider::new(bu, model, auth, HttpProtocol::OpenAI)))
            } else {
                anyhow::bail!(
                    "Provider '{}' requires a 'base_url' field.",
                    unknown
                )
            }
        }
    }
}

fn require_api_key(provider_type: &str, api_key: Option<&str>) -> anyhow::Result<String> {
    api_key.map(|k| k.to_string()).ok_or_else(|| anyhow::anyhow!(
        "Provider '{}' requires an API key. Set 'api_key' in .speedy/config.speedy.json or SPEEDY_API_KEY env var.",
        provider_type
    ))
}
