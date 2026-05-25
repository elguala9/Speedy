use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

use crate::constants::{HTTP_TIMEOUT_BATCH_SECS, OLLAMA_MAX_INPUT_CHARS};
use super::EmbeddingProvider;

pub enum AuthScheme {
    None,
    BearerToken(String),
    ApiKeyHeader { header: String, value: String },
    QueryParam { param: String, value: String },
}

pub(super) fn apply_auth(auth: &AuthScheme, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match auth {
        AuthScheme::None => builder,
        AuthScheme::BearerToken(token) => {
            builder.header("Authorization", format!("Bearer {}", token))
        }
        AuthScheme::ApiKeyHeader { header, value } => {
            builder.header(header.as_str(), value.as_str())
        }
        AuthScheme::QueryParam { param, value } => {
            builder.query(&[(param.as_str(), value.as_str())])
        }
    }
}

pub(crate) enum HttpProtocol {
    Ollama,
    OpenAI,
    Gemini,
}

pub struct HttpEmbeddingProvider {
    base_url: String,
    model: String,
    auth: AuthScheme,
    protocol: HttpProtocol,
    client: reqwest::Client,
}

impl HttpEmbeddingProvider {
    pub(crate) fn new(base_url: &str, model: &str, auth: AuthScheme, protocol: HttpProtocol) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_BATCH_SECS))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            auth,
            protocol,
            client,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for HttpEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        match &self.protocol {
            HttpProtocol::Ollama => {
                #[derive(serde::Serialize)]
                struct OllamaReq<'a> { model: &'a str, prompt: &'a str }
                #[derive(Deserialize)]
                struct OllamaResp { embedding: Vec<f32> }

                let url = format!("{}/api/embeddings", self.base_url);
                let builder = self.client.post(&url).json(&OllamaReq {
                    model: &self.model,
                    prompt: text,
                });
                let resp = apply_auth(&self.auth, builder).send().await?.json::<OllamaResp>().await?;
                Ok(resp.embedding)
            }
            HttpProtocol::OpenAI => {
                #[derive(serde::Serialize)]
                struct OpenAIReq<'a> { model: &'a str, input: &'a str }
                #[derive(Deserialize)]
                struct OpenAIData { embedding: Vec<f32> }
                #[derive(Deserialize)]
                struct OpenAIResp { data: Vec<OpenAIData> }

                let url = format!("{}/v1/embeddings", self.base_url);
                let builder = self.client.post(&url).json(&OpenAIReq {
                    model: &self.model,
                    input: text,
                });
                let resp = apply_auth(&self.auth, builder).send().await?.json::<OpenAIResp>().await?;
                let embedding = resp.data.into_iter().next()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI response had no embedding data"))?
                    .embedding;
                Ok(embedding)
            }
            HttpProtocol::Gemini => {
                let url = format!(
                    "{}/v1beta/models/{}:embedContent",
                    self.base_url, self.model
                );
                #[derive(serde::Serialize)]
                struct GeminiPart<'a> { text: &'a str }
                #[derive(serde::Serialize)]
                struct GeminiContent<'a> { parts: Vec<GeminiPart<'a>> }
                #[derive(serde::Serialize)]
                struct GeminiReq<'a> { content: GeminiContent<'a> }
                #[derive(Deserialize)]
                struct GeminiValues { values: Vec<f32> }
                #[derive(Deserialize)]
                struct GeminiResp { embedding: GeminiValues }

                let body = GeminiReq {
                    content: GeminiContent {
                        parts: vec![GeminiPart { text }],
                    },
                };
                let builder = self.client.post(&url).json(&body);
                let resp = apply_auth(&self.auth, builder).send().await?.json::<GeminiResp>().await?;
                Ok(resp.embedding.values)
            }
        }
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        match &self.protocol {
            HttpProtocol::Ollama => {
                #[derive(serde::Serialize)]
                struct OllamaBatchReq<'a> { model: &'a str, input: &'a [&'a str] }
                #[derive(Deserialize)]
                struct OllamaBatchResp { embeddings: Vec<Vec<f32>> }

                let truncated: Vec<&str> = texts.iter()
                    .map(|t| {
                        if t.len() <= OLLAMA_MAX_INPUT_CHARS { return *t; }
                        let mut end = OLLAMA_MAX_INPUT_CHARS;
                        while !t.is_char_boundary(end) { end -= 1; }
                        &t[..end]
                    })
                    .collect();

                let url = format!("{}/api/embed", self.base_url);
                let builder = self.client.post(&url).json(&OllamaBatchReq {
                    model: &self.model,
                    input: &truncated,
                });
                let http_resp = apply_auth(&self.auth, builder).send().await?;
                let body = http_resp.text().await?;
                let resp = serde_json::from_str::<OllamaBatchResp>(&body).map_err(|e| {
                    tracing::error!(target: "ai-context", ollama_response = %body, "ollama /api/embed returned unexpected body");
                    e
                })?;
                Ok(resp.embeddings)
            }
            HttpProtocol::OpenAI => {
                #[derive(serde::Serialize)]
                struct OpenAIBatchReq<'a> { model: &'a str, input: &'a [&'a str] }
                #[derive(Deserialize)]
                struct OpenAIData { embedding: Vec<f32> }
                #[derive(Deserialize)]
                struct OpenAIBatchResp { data: Vec<OpenAIData> }

                let url = format!("{}/v1/embeddings", self.base_url);
                let builder = self.client.post(&url).json(&OpenAIBatchReq {
                    model: &self.model,
                    input: texts,
                });
                let resp = apply_auth(&self.auth, builder).send().await?.json::<OpenAIBatchResp>().await?;
                Ok(resp.data.into_iter().map(|d| d.embedding).collect())
            }
            HttpProtocol::Gemini => {
                let mut out = Vec::with_capacity(texts.len());
                for t in texts {
                    out.push(self.embed(t).await?);
                }
                Ok(out)
            }
        }
    }
}
