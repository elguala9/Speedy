use std::collections::HashMap;

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub command: Option<String>,
    pub dims: Option<usize>,
    pub extra_headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct JsonConfig {
    pub provider: Option<ProviderConfig>,
    pub max_chunk_size: Option<usize>,
    pub chunk_overlap: Option<usize>,
    pub top_k: Option<usize>,
    pub watch_delay_ms: Option<u64>,
    pub ignore_patterns: Option<Vec<String>>,
    pub index_concurrency: Option<usize>,
}

pub fn merge_provider(base: &mut ProviderConfig, overlay: ProviderConfig) {
    if overlay.provider_type.is_some() {
        base.provider_type = overlay.provider_type;
    }
    if overlay.base_url.is_some() {
        base.base_url = overlay.base_url;
    }
    if overlay.model.is_some() {
        base.model = overlay.model;
    }
    if overlay.api_key.is_some() {
        base.api_key = overlay.api_key;
    }
    if overlay.command.is_some() {
        base.command = overlay.command;
    }
    if overlay.dims.is_some() {
        base.dims = overlay.dims;
    }
    if overlay.extra_headers.is_some() {
        base.extra_headers = overlay.extra_headers;
    }
}
