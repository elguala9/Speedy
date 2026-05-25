use super::*;

pub struct StubProvider {
    pub calls: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl EmbeddingProvider for StubProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.calls.lock().unwrap().push(text.to_string());
        Ok(vec![0.1, 0.2, 0.3])
    }
}

impl StubProvider {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }
}

pub struct MockEmbeddingProvider {
    pub dims: usize,
    pub calls: std::sync::Mutex<Vec<String>>,
    fail_next: std::sync::atomic::AtomicBool,
    latency_ms: std::sync::atomic::AtomicU64,
}

impl MockEmbeddingProvider {
    pub fn new(dims: usize) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            dims,
            calls: std::sync::Mutex::new(Vec::new()),
            fail_next: std::sync::atomic::AtomicBool::new(false),
            latency_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }

    pub fn set_fail_next(&self, fail: bool) {
        self.fail_next.store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_latency_ms(&self, ms: u64) {
        self.latency_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
    }

    fn hash_to_vec(text: &str, dims: usize) -> Vec<f32> {
        let mut seed: u64 = 14695981039346656037;
        for b in text.bytes() {
            seed ^= b as u64;
            seed = seed.wrapping_mul(1099511628211);
        }
        (0..dims)
            .map(|i| {
                let h = seed.wrapping_add(i as u64).wrapping_mul(2654435761);
                (h as f32 / u64::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for MockEmbeddingProvider {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
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

#[tokio::test]
async fn test_cascade_uses_first_provider_when_healthy() {
    let p1 = MockEmbeddingProvider::new(4);
    let p2 = MockEmbeddingProvider::new(4);
    let cascade = std::sync::Arc::new(CascadeEmbeddingProvider::new(vec![
        p1.clone() as std::sync::Arc<dyn EmbeddingProvider>,
        p2.clone() as std::sync::Arc<dyn EmbeddingProvider>,
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
    let cascade = std::sync::Arc::new(CascadeEmbeddingProvider::new(vec![
        p1.clone() as std::sync::Arc<dyn EmbeddingProvider>,
        p2.clone() as std::sync::Arc<dyn EmbeddingProvider>,
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
        p1 as std::sync::Arc<dyn EmbeddingProvider>,
        p2 as std::sync::Arc<dyn EmbeddingProvider>,
    ]);
    assert!(cascade.embed("fail").await.is_err(), "all providers failed → cascade should error");
}

#[tokio::test]
async fn test_cascade_empty_providers_errors() {
    let cascade = CascadeEmbeddingProvider::new(vec![]);
    assert!(cascade.embed("x").await.is_err(), "no providers → must error");
}
