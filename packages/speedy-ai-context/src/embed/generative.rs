use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

use crate::constants::HTTP_TIMEOUT_BATCH_SECS;
use super::{AuthScheme, EmbeddingProvider};
use super::http::apply_auth;

pub(crate) enum GenerativeProtocol {
    Anthropic,
    OpenAIChat,
}

pub struct GenerativeEmbeddingProvider {
    endpoint: String,
    model: String,
    auth: AuthScheme,
    dims: usize,
    protocol: GenerativeProtocol,
    client: reqwest::Client,
}

impl GenerativeEmbeddingProvider {
    pub(crate) fn new(
        endpoint: &str,
        model: &str,
        auth: AuthScheme,
        dims: usize,
        protocol: GenerativeProtocol,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_BATCH_SECS))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            auth,
            dims,
            protocol,
            client,
        }
    }

    fn build_prompt(&self, text: &str) -> String {
        format!(
            "Return ONLY a JSON array of {} floats representing the semantic embedding of this text. No explanation.\nText: {}",
            self.dims, text
        )
    }

    fn parse_embedding(&self, raw: &str, provider_type: &str) -> Result<Vec<f32>> {
        let trimmed = raw.trim();
        if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
            let json_slice = &trimmed[start..=end];
            if let Ok(floats) = serde_json::from_str::<Vec<f32>>(json_slice) {
                return Ok(floats);
            }
        }
        anyhow::bail!(
            "Generative provider '{}' returned unparseable embedding. Check that the model supports instruction-following.",
            provider_type
        )
    }
}

#[async_trait]
impl EmbeddingProvider for GenerativeEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let prompt = self.build_prompt(text);

        match &self.protocol {
            GenerativeProtocol::Anthropic => {
                #[derive(serde::Serialize)]
                struct AnthropicMsg<'a> { role: &'a str, content: &'a str }
                #[derive(serde::Serialize)]
                struct AnthropicReq<'a> {
                    model: &'a str,
                    max_tokens: u32,
                    messages: Vec<AnthropicMsg<'a>>,
                }
                #[derive(Deserialize)]
                struct ContentBlock { text: String }
                #[derive(Deserialize)]
                struct AnthropicResp { content: Vec<ContentBlock> }

                let body = AnthropicReq {
                    model: &self.model,
                    max_tokens: 4096,
                    messages: vec![AnthropicMsg { role: "user", content: &prompt }],
                };
                let builder = self.client.post(&self.endpoint)
                    .header("anthropic-version", "2023-06-01")
                    .json(&body);
                let builder = apply_auth(&self.auth, builder);
                let resp = builder.send().await?.json::<AnthropicResp>().await?;
                let raw = resp.content.into_iter().next()
                    .ok_or_else(|| anyhow::anyhow!("Anthropic response had no content"))?
                    .text;
                self.parse_embedding(&raw, "anthropic")
            }
            GenerativeProtocol::OpenAIChat => {
                #[derive(serde::Serialize)]
                struct ChatMsg<'a> { role: &'a str, content: &'a str }
                #[derive(serde::Serialize)]
                struct ChatReq<'a> { model: &'a str, messages: Vec<ChatMsg<'a>> }
                #[derive(Deserialize)]
                struct ChatMsgResp { content: String }
                #[derive(Deserialize)]
                struct ChatChoice { message: ChatMsgResp }
                #[derive(Deserialize)]
                struct ChatResp { choices: Vec<ChatChoice> }

                let body = ChatReq {
                    model: &self.model,
                    messages: vec![ChatMsg { role: "user", content: &prompt }],
                };
                let builder = self.client.post(&self.endpoint).json(&body);
                let builder = apply_auth(&self.auth, builder);
                let resp = builder.send().await?.json::<ChatResp>().await?;
                let raw = resp.choices.into_iter().next()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI chat response had no choices"))?
                    .message.content;
                self.parse_embedding(&raw, "deepseek/openai-chat")
            }
        }
    }
}
