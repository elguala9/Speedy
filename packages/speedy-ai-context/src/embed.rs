use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use speedy_core::config::Config;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed multiple texts in one call. Default impl calls `embed` serially.
    /// Providers that support batch APIs (Ollama `/api/embed`, OpenAI) override this.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// OllamaProvider — kept for backward compatibility (used in tests, wrappers)
// ---------------------------------------------------------------------------

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
        // debug-level: the per-file summary in indexer.rs is the primary
        // signal. Set RUST_LOG=ai_context=debug to see one row per HTTP call.
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

// ---------------------------------------------------------------------------
// AgentProvider — kept for backward compatibility
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AuthScheme
// ---------------------------------------------------------------------------

pub enum AuthScheme {
    None,
    BearerToken(String),
    ApiKeyHeader { header: String, value: String },
    QueryParam { param: String, value: String },
}

// ---------------------------------------------------------------------------
// HttpEmbeddingProvider — generic HTTP embedding (Ollama, OpenAI, Gemini, …)
// ---------------------------------------------------------------------------

pub(crate) enum HttpProtocol {
    Ollama,  // POST /api/embeddings, {"model":"...","prompt":"..."}
    OpenAI,  // POST /v1/embeddings, {"model":"...","input":"..."}
    Gemini,  // POST /v1beta/models/{model}:embedContent
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
            .timeout(Duration::from_secs(120))
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

    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
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
                let resp = self.apply_auth(builder).send().await?.json::<OllamaResp>().await?;
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
                let resp = self.apply_auth(builder).send().await?.json::<OpenAIResp>().await?;
                let embedding = resp.data.into_iter().next()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI response had no embedding data"))?
                    .embedding;
                Ok(embedding)
            }
            HttpProtocol::Gemini => {
                // Model goes in the URL path for Gemini
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
                let resp = self.apply_auth(builder).send().await?.json::<GeminiResp>().await?;
                Ok(resp.embedding.values)
            }
        }
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        match &self.protocol {
            HttpProtocol::Ollama => {
                // Ollama ≥0.3 supports /api/embed with an input array.
                #[derive(serde::Serialize)]
                struct OllamaBatchReq<'a> { model: &'a str, input: &'a [&'a str] }
                #[derive(Deserialize)]
                struct OllamaBatchResp { embeddings: Vec<Vec<f32>> }

                // all-minilm and similar small models have a ~256 token context.
                // Truncate to 500 chars to avoid "input length exceeds context length" errors.
                const MAX_INPUT_CHARS: usize = 500;
                let truncated: Vec<&str> = texts.iter()
                    .map(|t| {
                        if t.len() <= MAX_INPUT_CHARS { return *t; }
                        let mut end = MAX_INPUT_CHARS;
                        while !t.is_char_boundary(end) { end -= 1; }
                        &t[..end]
                    })
                    .collect();

                let url = format!("{}/api/embed", self.base_url);
                let builder = self.client.post(&url).json(&OllamaBatchReq {
                    model: &self.model,
                    input: &truncated,
                });
                let http_resp = self.apply_auth(builder).send().await?;
                let body = http_resp.text().await?;
                let resp = serde_json::from_str::<OllamaBatchResp>(&body).map_err(|e| {
                    tracing::error!(target: "ai-context", ollama_response = %body, "ollama /api/embed returned unexpected body");
                    e
                })?;
                Ok(resp.embeddings)
            }
            HttpProtocol::OpenAI => {
                // OpenAI /v1/embeddings accepts an array for input.
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
                let resp = self.apply_auth(builder).send().await?.json::<OpenAIBatchResp>().await?;
                Ok(resp.data.into_iter().map(|d| d.embedding).collect())
            }
            HttpProtocol::Gemini => {
                // Gemini has no batch embed endpoint — fall back to serial.
                let mut out = Vec::with_capacity(texts.len());
                for t in texts {
                    out.push(self.embed(t).await?);
                }
                Ok(out)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GenerativeEmbeddingProvider — uses a chat/messages API to produce embeddings
// ---------------------------------------------------------------------------

pub(crate) enum GenerativeProtocol {
    Anthropic,   // POST /v1/messages, x-api-key header
    OpenAIChat,  // POST /chat/completions, bearer auth
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
            .timeout(Duration::from_secs(120))
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

    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
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

    fn build_prompt(&self, text: &str) -> String {
        format!(
            "Return ONLY a JSON array of {} floats representing the semantic embedding of this text. No explanation.\nText: {}",
            self.dims, text
        )
    }

    fn parse_embedding(&self, raw: &str, provider_type: &str) -> Result<Vec<f32>> {
        // Try to extract a JSON array from the response text
        let trimmed = raw.trim();
        // Find the first '[' and last ']'
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
                let builder = self.apply_auth(builder);
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
                let builder = self.apply_auth(builder);
                let resp = builder.send().await?.json::<ChatResp>().await?;
                let raw = resp.choices.into_iter().next()
                    .ok_or_else(|| anyhow::anyhow!("OpenAI chat response had no choices"))?
                    .message.content;
                self.parse_embedding(&raw, "deepseek/openai-chat")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub fn create_provider(config: &Config) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
    // Resolve with cascade: provider struct fields take priority, flat fields are fallback
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
            // If base_url is provided, treat as openai-compatible
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

// ---------------------------------------------------------------------------
// CascadeEmbeddingProvider — tries providers in order, falls back on error
// ---------------------------------------------------------------------------

/// Tries each provider in order; falls back to the next one when a call fails.
/// Returns an error only when all providers have been exhausted.
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

fn require_api_key(provider_type: &str, api_key: Option<&str>) -> anyhow::Result<String> {
    api_key.map(|k| k.to_string()).ok_or_else(|| anyhow::anyhow!(
        "Provider '{}' requires an API key. Set 'api_key' in .speedy/config.speedy.json or SPEEDY_API_KEY env var.",
        provider_type
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    /// Deterministic mock provider for unit tests.
    ///
    /// Vectors are stable across calls: same input always produces the same output.
    /// Supports optional error injection via `set_fail_next` and latency injection
    /// via `set_latency_ms`.
    pub struct MockEmbeddingProvider {
        pub dims: usize,
        pub calls: std::sync::Mutex<Vec<String>>,
        fail_next: std::sync::atomic::AtomicBool,
        latency_ms: std::sync::atomic::AtomicU64,
    }

    impl MockEmbeddingProvider {
        pub fn new(dims: usize) -> Arc<Self> {
            Arc::new(Self {
                dims,
                calls: std::sync::Mutex::new(Vec::new()),
                fail_next: std::sync::atomic::AtomicBool::new(false),
                latency_ms: std::sync::atomic::AtomicU64::new(0),
            })
        }

        /// Cause the next `embed` call to return an error. Resets after one call.
        pub fn set_fail_next(&self, fail: bool) {
            self.fail_next.store(fail, std::sync::atomic::Ordering::SeqCst);
        }

        /// Add artificial latency (ms) to each `embed` call.
        pub fn set_latency_ms(&self, ms: u64) {
            self.latency_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
        }

        fn hash_to_vec(text: &str, dims: usize) -> Vec<f32> {
            // FNV-1a hash seeded per dimension for stable, unique vectors per input
            let mut seed: u64 = 14695981039346656037;
            for b in text.bytes() {
                seed ^= b as u64;
                seed = seed.wrapping_mul(1099511628211);
            }
            (0..dims)
                .map(|i| {
                    let h = seed.wrapping_add(i as u64).wrapping_mul(2654435761);
                    // Map to (-1.0, 1.0)
                    (h as f32 / u64::MAX as f32) * 2.0 - 1.0
                })
                .collect()
        }
    }

    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let ms = self.latency_ms.load(std::sync::atomic::Ordering::Relaxed);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
            if self.fail_next.swap(false, std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("MockEmbeddingProvider: injected failure");
            }
            self.calls.lock().unwrap().push(text.to_string());
            Ok(Self::hash_to_vec(text, self.dims))
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
        let config = speedy_core::config::Config::default();
        assert_eq!(config.provider_type, "ollama");
        assert_eq!(config.model, "all-minilm");
        assert_eq!(config.ollama_url, "http://localhost:11434");
    }

    #[test]
    fn test_create_provider_ollama() {
        let mut config = speedy_core::config::Config::default();
        config.model = "nomic-embed-text".to_string();
        config.ollama_url = "http://10.0.0.1:11434".to_string();
        config.provider_type = "ollama".to_string();

        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_provider_agent() {
        let mut config = speedy_core::config::Config::default();
        config.provider_type = "agent".to_string();
        config.agent_command = "echo".to_string();

        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_provider_unknown_no_base_url() {
        let mut config = speedy_core::config::Config::default();
        config.provider_type = "invalid".to_string();
        // config.provider is all-None (default), so fallback to flat fields
        // flat field base_url is ollama_url (http://localhost:11434) but provider.base_url is None
        // The factory checks config.provider.base_url specifically for unknown types
        let result = create_provider(&config);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("base_url"));
    }

    #[test]
    fn test_create_provider_openai_requires_api_key() {
        let mut config = speedy_core::config::Config::default();
        config.provider_type = "openai".to_string();
        let result = create_provider(&config);
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("API key"));
    }

    #[test]
    fn test_create_provider_openai_with_api_key() {
        let mut config = speedy_core::config::Config::default();
        config.provider.api_key = Some("sk-test-key".to_string());
        config.provider.provider_type = Some("openai".to_string());
        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_provider_unknown_with_base_url() {
        let mut config = speedy_core::config::Config::default();
        config.provider.provider_type = Some("my-custom".to_string());
        config.provider.base_url = Some("https://my-endpoint.com".to_string());
        let result = create_provider(&config);
        assert!(result.is_ok());
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

    #[test]
    fn test_create_provider_gemini_requires_api_key() {
        let mut config = speedy_core::config::Config::default();
        config.provider.provider_type = Some("gemini".to_string());
        let result = create_provider(&config);
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("API key"));
    }

    #[test]
    fn test_create_provider_gemini_with_api_key() {
        let mut config = speedy_core::config::Config::default();
        config.provider.provider_type = Some("gemini".to_string());
        config.provider.api_key = Some("AIza-test-key".to_string());
        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_provider_anthropic_with_api_key() {
        let mut config = speedy_core::config::Config::default();
        config.provider.provider_type = Some("anthropic".to_string());
        config.provider.api_key = Some("sk-ant-test".to_string());
        let result = create_provider(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_provider_openai_compatible_requires_base_url() {
        let mut config = speedy_core::config::Config::default();
        config.provider.provider_type = Some("openai-compatible".to_string());
        let result = create_provider(&config);
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("base_url"));
    }

    #[test]
    fn test_create_provider_azure_requires_base_url_and_api_key() {
        let mut config = speedy_core::config::Config::default();
        config.provider.provider_type = Some("azure-openai".to_string());
        let result = create_provider(&config);
        assert!(result.is_err());

        // With base_url but no api_key
        let mut config2 = speedy_core::config::Config::default();
        config2.provider.provider_type = Some("azure-openai".to_string());
        config2.provider.base_url = Some("https://my.openai.azure.com".to_string());
        let result2 = create_provider(&config2);
        assert!(result2.is_err());
        assert!(result2.err().unwrap().to_string().contains("API key"));
    }

    #[tokio::test]
    async fn test_mock_provider_deterministic() {
        let p = MockEmbeddingProvider::new(8);
        let v1 = p.embed("hello").await.unwrap();
        let v2 = p.embed("hello").await.unwrap();
        assert_eq!(v1, v2, "same input must produce same vector");
        assert_eq!(v1.len(), 8);
    }

    #[tokio::test]
    async fn test_mock_provider_different_inputs() {
        let p = MockEmbeddingProvider::new(8);
        let v1 = p.embed("hello").await.unwrap();
        let v2 = p.embed("world").await.unwrap();
        assert_ne!(v1, v2, "different inputs must produce different vectors");
    }

    #[tokio::test]
    async fn test_mock_provider_vectors_bounded() {
        let p = MockEmbeddingProvider::new(16);
        let v = p.embed("test text").await.unwrap();
        for f in &v {
            assert!(
                *f >= -1.0 && *f <= 1.0,
                "vector components must be in [-1.0, 1.0], got {}",
                f
            );
        }
    }

    #[tokio::test]
    async fn test_mock_provider_fail_next() {
        let p = MockEmbeddingProvider::new(8);
        p.set_fail_next(true);
        assert!(p.embed("a").await.is_err(), "should fail on first call");
        assert!(p.embed("b").await.is_ok(), "should succeed after reset");
    }

    #[tokio::test]
    async fn test_mock_provider_tracks_calls() {
        let p = MockEmbeddingProvider::new(4);
        p.embed("x").await.unwrap();
        p.embed("y").await.unwrap();
        let calls = p.calls.lock().unwrap();
        assert_eq!(calls.as_slice(), &["x", "y"]);
    }

    #[tokio::test]
    async fn test_mock_provider_fail_does_not_record_call() {
        let p = MockEmbeddingProvider::new(4);
        p.set_fail_next(true);
        let _ = p.embed("fail-me").await;
        assert!(p.calls.lock().unwrap().is_empty(), "failed call must not be recorded");
    }

    #[tokio::test]
    async fn test_mock_provider_latency_is_measurable() {
        let p = MockEmbeddingProvider::new(4);
        p.set_latency_ms(50);
        let start = std::time::Instant::now();
        p.embed("slow").await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() >= 40,
            "expected ≥40ms latency, got {}ms",
            elapsed.as_millis()
        );
    }

    // ── CascadeEmbeddingProvider ──────────────────────────────────────────

    #[tokio::test]
    async fn test_cascade_uses_first_provider_when_healthy() {
        let p1 = MockEmbeddingProvider::new(4);
        let p2 = MockEmbeddingProvider::new(4);
        let cascade = Arc::new(CascadeEmbeddingProvider::new(vec![
            p1.clone() as Arc<dyn EmbeddingProvider>,
            p2.clone() as Arc<dyn EmbeddingProvider>,
        ]));
        cascade.embed("hello").await.unwrap();
        assert_eq!(p1.calls.lock().unwrap().len(), 1, "first provider should have been called");
        assert!(p2.calls.lock().unwrap().is_empty(), "second provider must not be called if first succeeds");
    }

    #[tokio::test]
    async fn test_cascade_falls_back_to_second_when_first_fails() {
        let p1 = MockEmbeddingProvider::new(4);
        let p2 = MockEmbeddingProvider::new(4);
        p1.set_fail_next(true);
        let cascade = Arc::new(CascadeEmbeddingProvider::new(vec![
            p1.clone() as Arc<dyn EmbeddingProvider>,
            p2.clone() as Arc<dyn EmbeddingProvider>,
        ]));
        let v = cascade.embed("hello").await.unwrap();
        assert_eq!(v.len(), 4, "result should come from second provider");
        assert!(p1.calls.lock().unwrap().is_empty(), "failed p1 should not record the call");
        assert_eq!(p2.calls.lock().unwrap().len(), 1, "second provider should have been called");
    }

    #[tokio::test]
    async fn test_cascade_all_fail_returns_error() {
        let p1 = MockEmbeddingProvider::new(4);
        let p2 = MockEmbeddingProvider::new(4);
        p1.set_fail_next(true);
        p2.set_fail_next(true);
        let cascade = CascadeEmbeddingProvider::new(vec![
            p1 as Arc<dyn EmbeddingProvider>,
            p2 as Arc<dyn EmbeddingProvider>,
        ]);
        assert!(cascade.embed("fail").await.is_err(), "all providers failed → cascade should error");
    }

    #[tokio::test]
    async fn test_cascade_empty_providers_errors() {
        let cascade = CascadeEmbeddingProvider::new(vec![]);
        assert!(cascade.embed("x").await.is_err(), "no providers → must error");
    }
}
