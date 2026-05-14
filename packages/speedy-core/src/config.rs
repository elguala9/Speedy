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
        }
    }
}

fn default_model() -> String { "all-minilm:l6-v2".to_string() }
fn default_max_chunk_size() -> usize { 1000 }
fn default_chunk_overlap() -> usize { 200 }
fn default_top_k() -> usize { 5 }
fn default_ollama_url() -> String { "http://localhost:11434".to_string() }
fn default_provider_type() -> String { "ollama".to_string() }
fn default_watch_delay_ms() -> u64 { 500 }
fn default_ignore_patterns() -> Vec<String> {
    vec![
        "target/".to_string(),
        ".git/".to_string(),
        "node_modules/".to_string(),
        ".speedy/".to_string(),
    ]
}

impl Config {
    /// Build a `Config` from `speedy.toml` / `.speedy/config.toml` (cwd or
    /// project subdir), then overlay env-var overrides. Use this for normal
    /// runs where the user might have a config file. Returns `Default` if no
    /// file is found.
    pub fn load() -> Self {
        let mut config = Self::from_file().unwrap_or_default();
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
        if let Ok(val) = std::env::var("SPEEDY_MODEL") {
            self.model = val;
        }
        if let Ok(val) = std::env::var("SPEEDY_OLLAMA_URL") {
            self.ollama_url = val;
        }
        if let Ok(val) = std::env::var("SPEEDY_PROVIDER") {
            self.provider_type = val;
        }
        if let Ok(val) = std::env::var("SPEEDY_AGENT_COMMAND") {
            self.agent_command = val;
        }
        if let Ok(val) = std::env::var("SPEEDY_TOP_K") {
            if let Ok(k) = val.parse() {
                self.top_k = k;
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
    }

    #[test]
    fn test_default_values() {
        let config = Config::default();
        assert_eq!(config.model, "all-minilm:l6-v2");
        assert_eq!(config.max_chunk_size, 1000);
        assert_eq!(config.chunk_overlap, 200);
        assert_eq!(config.top_k, 5);
        assert_eq!(config.ollama_url, "http://localhost:11434");
        assert_eq!(config.provider_type, "ollama");
        assert_eq!(config.agent_command, "");
        assert_eq!(config.watch_delay_ms, 500);
        assert_eq!(config.ignore_patterns, vec!["target/", ".git/", "node_modules/", ".speedy/"]);
    }

    #[test]
    fn test_merge_env_model() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_MODEL", "nomic-embed-text");
        config.merge_env();
        assert_eq!(config.model, "nomic-embed-text");
    }

    #[test]
    fn test_merge_env_ollama_url() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_OLLAMA_URL", "http://10.0.0.1:11434");
        config.merge_env();
        assert_eq!(config.ollama_url, "http://10.0.0.1:11434");
    }

    #[test]
    fn test_merge_env_provider() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_PROVIDER", "agent");
        config.merge_env();
        assert_eq!(config.provider_type, "agent");
    }

    #[test]
    fn test_merge_env_agent_command() {
        let _lock = ENV_LOCK.lock().unwrap();
        clear_all_env();
        let mut config = Config::default();
        std::env::set_var("SPEEDY_AGENT_COMMAND", "my-agent");
        config.merge_env();
        assert_eq!(config.agent_command, "my-agent");
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
        assert_eq!(config.model, "all-minilm:l6-v2");
        assert_eq!(config.ollama_url, "http://localhost:11434");
    }
}
