use crate::provider_config::{JsonConfig, ProviderConfig, merge_provider};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_chunk_size")]
    pub max_chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    #[serde(default = "default_provider_type")]
    pub provider_type: String,
    #[serde(default)]
    pub agent_command: String,
    #[serde(default = "default_watch_delay_ms")]
    pub watch_delay_ms: u64,
    #[serde(default = "default_ignore_patterns")]
    pub ignore_patterns: Vec<String>,
    /// Structured provider config (populated from JSON config cascade + env vars).
    /// This is skipped during TOML deserialization; fields in the TOML are mapped
    /// via the flat fields above for backward compatibility.
    #[serde(skip)]
    pub provider: ProviderConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: default_model(),
            max_chunk_size: default_max_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            top_k: default_top_k(),
            ollama_url: default_ollama_url(),
            provider_type: default_provider_type(),
            agent_command: String::new(),
            watch_delay_ms: default_watch_delay_ms(),
            ignore_patterns: default_ignore_patterns(),
            provider: ProviderConfig::default(),
        }
    }
}

fn default_model() -> String { "nomic-embed-text".to_string() }
fn default_max_chunk_size() -> usize { 1000 }
fn default_chunk_overlap() -> usize { 200 }
fn default_top_k() -> usize { 5 }
fn default_ollama_url() -> String { "http://localhost:11434".to_string() }
fn default_provider_type() -> String { "ollama".to_string() }
fn default_watch_delay_ms() -> u64 { 500 }
fn default_ignore_patterns() -> Vec<String> {
    crate::default_ignores::patterns()
        .into_iter()
        .map(String::from)
        .collect()
}

impl Config {
    /// Build a `Config` from the full cascade:
    /// 1. TOML workspace file (speedy.toml / .speedy/config.toml)
    /// 2. User JSON (~/.speedy/config.speedy.json) — low priority
    /// 3. Workspace JSON (.speedy/config.speedy.json) — higher priority
    /// 4. Env vars — highest priority
    ///
    /// Returns `Default` as base if no TOML is found.
    pub fn load() -> Self {
        // --- 1. Start from TOML workspace config (or defaults) ---
        let mut config = Self::from_file().unwrap_or_default();

        // --- 2. Load JSON configs ---
        let user_json = load_user_json_config();
        let workspace_json = load_workspace_json_config();

        // --- 3. Merge scalar fields from JSON configs ---
        // Priority: workspace JSON > user JSON > TOML (already applied via defaults)
        // For scalar numeric/string fields, only overwrite if TOML didn't set a non-default
        // value is complex to detect, so we apply user JSON first then workspace JSON on top.

        // Apply user JSON scalars (lower priority)
        if let Some(ref ujson) = user_json {
            if let Some(v) = ujson.max_chunk_size { config.max_chunk_size = v; }
            if let Some(v) = ujson.chunk_overlap { config.chunk_overlap = v; }
            if let Some(v) = ujson.top_k { config.top_k = v; }
            if let Some(v) = ujson.watch_delay_ms { config.watch_delay_ms = v; }
            if let Some(ref v) = ujson.ignore_patterns { config.ignore_patterns = v.clone(); }
        }

        // Apply workspace JSON scalars (higher priority — overwrites user JSON)
        if let Some(ref wjson) = workspace_json {
            if let Some(v) = wjson.max_chunk_size { config.max_chunk_size = v; }
            if let Some(v) = wjson.chunk_overlap { config.chunk_overlap = v; }
            if let Some(v) = wjson.top_k { config.top_k = v; }
            if let Some(v) = wjson.watch_delay_ms { config.watch_delay_ms = v; }
            if let Some(ref v) = wjson.ignore_patterns { config.ignore_patterns = v.clone(); }
        }

        // --- 4. Build provider config from cascade ---
        // Start empty, then merge: user JSON -> workspace JSON -> env vars
        let mut provider = ProviderConfig::default();

        if let Some(ujson) = user_json {
            if let Some(p) = ujson.provider {
                merge_provider(&mut provider, p);
            }
        }
        if let Some(wjson) = workspace_json {
            if let Some(p) = wjson.provider {
                merge_provider(&mut provider, p);
            }
        }

        config.provider = provider;

        // --- 5. Apply env vars (highest priority) ---
        config.merge_env();

        config
    }

    fn from_file() -> Option<Self> {
        let candidates = [
            std::path::Path::new("speedy.toml"),
            std::path::Path::new(".speedy/config.toml"),
        ];
        for path in &candidates {
            if path.exists() {
                let content = std::fs::read_to_string(path).ok()?;
                if let Ok(cfg) = toml::from_str(&content) {
                    return Some(cfg);
                }
            }
        }
        None
    }

    fn merge_env(&mut self) {
        // Legacy flat fields (maintain backward compat)
        if let Ok(val) = std::env::var("SPEEDY_MODEL") {
            self.model = val.clone();
            self.provider.model = Some(val);
        }
        if let Ok(val) = std::env::var("SPEEDY_PROVIDER") {
            self.provider_type = val.clone();
            self.provider.provider_type = Some(val);
        }
        if let Ok(val) = std::env::var("SPEEDY_AGENT_COMMAND") {
            self.agent_command = val.clone();
            self.provider.command = Some(val);
        }
        if let Ok(val) = std::env::var("SPEEDY_TOP_K") {
            if let Ok(k) = val.parse() {
                self.top_k = k;
            }
        }

        // New env vars
        if let Ok(val) = std::env::var("SPEEDY_BASE_URL") {
            self.ollama_url = val.clone();
            self.provider.base_url = Some(val);
        }
        if let Ok(val) = std::env::var("SPEEDY_API_KEY") {
            self.provider.api_key = Some(val);
        }

        // Legacy alias: SPEEDY_OLLAMA_URL → base_url (lower priority than SPEEDY_BASE_URL)
        if let Ok(val) = std::env::var("SPEEDY_OLLAMA_URL") {
            self.ollama_url = val.clone();
            // Only set provider.base_url if SPEEDY_BASE_URL was not already set
            if std::env::var("SPEEDY_BASE_URL").is_err() {
                self.provider.base_url = Some(val);
            }
        }
    }

    /// Build a `Config` from `Default` + env-var overrides only — **skips the
    /// config file lookup**. Use this from contexts where reading the cwd's
    /// `speedy.toml` is wrong (e.g. background tasks where cwd is incidental,
    /// or pure-env-driven helpers). Most callers want [`Config::load`].
    pub fn from_env() -> Self {
        let mut config = Config::default();
        config.merge_env();
        config
    }
}

fn load_workspace_json_config() -> Option<JsonConfig> {
    let path = std::path::Path::new(".speedy/config.speedy.json");
    if path.exists() {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    } else {
        None
    }
}

fn load_user_json_config() -> Option<JsonConfig> {
    let home = dirs::home_dir()?;
    let path = home.join(".speedy").join("config.speedy.json");
    if path.exists() {
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_all_env() {
        std::env::remove_var("SPEEDY_MODEL");
        std::env::remove_var("SPEEDY_OLLAMA_URL");
        std::env::remove_var("SPEEDY_PROVIDER");
        std::env::remove_var("SPEEDY_AGENT_COMMAND");
        std::env::remove_var("SPEEDY_TOP_K");
        std::env::remove_var("SPEEDY_BASE_URL");
        std::env::remove_var("SPEEDY_API_KEY");
    }

    #[test]
    fn test_default_values() {
        let config = Config::default();
        assert_eq!(config.model, "nomic-embed-text");
        assert_eq!(config.max_chunk_size, 1000);
        assert_eq!(config.chunk_overlap, 200);
        assert_eq!(config.top_k, 5);
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.provider_type, "ollama");
        assert_eq!(config.agent_command, "");
        assert_eq!(config.watch_delay_ms, 500);
        assert!(config.ignore_patterns.iter().any(|p| p == "target/"));
        assert!(config.ignore_patterns.iter().any(|p| p == "node_modules/"));
        assert!(config.ignore_patterns.iter().any(|p| p == ".git/"));
        // provider defaults to all-None
        assert!(config.provider.provider_type.is_none());
        assert!(config.provider.api_key.is_none());
    }

    #[test]
    fn test_merge_env_model() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_MODEL", "nomic-embed-text");
        config.merge_env();
        assert_eq!(config.model, "nomic-embed-text");
        assert_eq!(config.provider.model.as_deref(), Some("nomic-embed-text"));
    }

    #[test]
    fn test_merge_env_ollama_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_OLLAMA_URL", "http://10.0.0.1:11434");
        config.merge_env();
        assert_eq!(config.ollama_url, "http://10.0.0.1:11434");
        assert_eq!(config.provider.base_url.as_deref(), Some("http://10.0.0.1:11434"));
    }

    #[test]
    fn test_merge_env_base_url_takes_priority_over_ollama_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_BASE_URL", "https://my-server.com");
        std::env::set_var("SPEEDY_OLLAMA_URL", "http://10.0.0.1:11434");
        config.merge_env();
        // SPEEDY_BASE_URL wins for provider.base_url
        assert_eq!(config.provider.base_url.as_deref(), Some("https://my-server.com"));
        // ollama_url gets the last written (SPEEDY_OLLAMA_URL overwrites legacy flat field)
        // but provider.base_url stays as SPEEDY_BASE_URL
    }

    #[test]
    fn test_merge_env_provider() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_PROVIDER", "agent");
        config.merge_env();
        assert_eq!(config.provider_type, "agent");
        assert_eq!(config.provider.provider_type.as_deref(), Some("agent"));
    }

    #[test]
    fn test_merge_env_agent_command() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_AGENT_COMMAND", "my-agent");
        config.merge_env();
        assert_eq!(config.agent_command, "my-agent");
        assert_eq!(config.provider.command.as_deref(), Some("my-agent"));
    }

    #[test]
    fn test_merge_env_api_key() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_API_KEY", "sk-test-key");
        config.merge_env();
        assert_eq!(config.provider.api_key.as_deref(), Some("sk-test-key"));
    }

    #[test]
    fn test_merge_env_top_k() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_TOP_K", "42");
        config.merge_env();
        assert_eq!(config.top_k, 42);
    }

    #[test]
    fn test_merge_env_top_k_invalid() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        std::env::set_var("SPEEDY_TOP_K", "not-a-number");
        let mut config = Config::default();
        config.merge_env();
        assert_eq!(config.top_k, 5);
    }

    #[test]
    fn test_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        std::env::set_var("SPEEDY_MODEL", "test-model");
        let config = Config::from_env();
        assert_eq!(config.model, "test-model");
    }

    #[test]
    fn test_from_env_no_vars() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let config = Config::from_env();
        assert_eq!(config.model, "nomic-embed-text");
        assert_eq!(config.ollama_url, "http://localhost:11434");
    }
}
