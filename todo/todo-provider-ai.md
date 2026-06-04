# TODO: AI Provider Abstraction

## Goal
Abstract the AI embedding provider so that Speedy can work with any backend (local or remote), not just Ollama. The default behavior remains identical to the current one.

---

## 1. Configuration merge principle

**Applies to every scalar config field.**

The resolution of each individual key follows this cascade:

```
env var  >  workspace config.speedy.json  >  user config.speedy.json  >  hard-coded default
```

**Exception — `ignore_patterns` (list):** total override. If the workspace defines the list, the user's one is ignored completely. If the workspace does not define it, the user's one is used. If neither defines it, the hard-coded default is used. No union is performed.

---

## 2. Configuration file format: `config.speedy.json`

JSON file that sits alongside the existing TOML (the TOML remains supported).

### Locations

1. `<workspace>/.speedy/config.speedy.json`  ← workspace, high priority
2. `~/.speedy/config.speedy.json`             ← user, fallback

On Windows: `~` = `%USERPROFILE%`. Use the `dirs` crate for cross-platform resolution.

**`config.speedy.json` must be added to the project's `.gitignore`** — it may contain API keys.

### General structure

```json
{
  "provider": {
    "type": "ollama",
    "base_url": "http://localhost:11434",
    "model": "all-minilm:l6-v2"
  },
  "max_chunk_size": 1000,
  "chunk_overlap": 200,
  "top_k": 10,
  "watch_delay_ms": 500,
  "ignore_patterns": ["target/", ".git/"]
}
```

The file may contain only some of the fields — the remaining ones are resolved through the cascade above.

### Provider examples

**Ollama (default, native embedding)**
```json
{ "provider": { "type": "ollama", "model": "all-minilm" } }
```

**OpenAI (native embedding)**
```json
{ "provider": { "type": "openai", "model": "text-embedding-3-small", "api_key": "sk-..." } }
```

**Gemini (native embedding)**
```json
{ "provider": { "type": "gemini", "model": "text-embedding-004", "api_key": "AIza..." } }
```

**Anthropic / Claude (generative used as an embedding proxy)**
```json
{ "provider": { "type": "anthropic", "model": "claude-3-haiku-20240307", "api_key": "sk-ant-...", "dims": 384 } }
```

**DeepSeek (generative used as an embedding proxy)**
```json
{ "provider": { "type": "deepseek", "model": "deepseek-chat", "api_key": "..." } }
```

**Any OpenAI-compatible endpoint**
```json
{
  "provider": {
    "type": "openai-compatible",
    "base_url": "https://my-endpoint.com/v1",
    "model": "my-model",
    "api_key": "..."
  }
}
```

**External process (formerly agent_command)**
```json
{ "provider": { "type": "agent", "command": "my-embed-script" } }
```

---

## 3. Rust design: maximum abstraction

### Principle
Do not enumerate the providers in an enum — use a generic structure. The `type` is an open string; the code knows how to adapt the transport based on it.

### 3.1 Provider config structure (in `speedy-core`)

```rust
pub struct ProviderConfig {
    /// "ollama", "openai", "gemini", "anthropic", "deepseek",
    /// "openai-compatible", "agent", or any custom string.
    pub provider_type: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    /// For type="agent": command to execute.
    pub command: Option<String>,
    /// Vector dimension for GenerativeEmbeddingProvider. Default: 384.
    pub dims: Option<usize>,
    /// Additional HTTP headers (corporate proxies, custom auth).
    pub extra_headers: Option<HashMap<String, String>>,
}
```

### 3.2 Two provider families

**A) Providers with a native embedding API** — use `HttpEmbeddingProvider`:

```rust
struct HttpEmbeddingProvider {
    endpoint: String,
    auth: AuthScheme,
    request_builder: Box<dyn Fn(&str, &str) -> serde_json::Value>,
    response_parser: Box<dyn Fn(serde_json::Value) -> Vec<f32>>,
    client: reqwest::Client,
}

enum AuthScheme {
    None,
    BearerToken(String),
    ApiKeyHeader(String, String),  // (header_name, value) — e.g. Azure uses "api-key"
    QueryParam(String, String),    // (param_name, value) — e.g. Gemini uses ?key=...
}
```

**B) Generative providers used as an embedding proxy** — use `GenerativeEmbeddingProvider`:

They send the text to the model with a structured prompt that asks it to return a JSON array of floats. The response is parsed and extracted.

```rust
struct GenerativeEmbeddingProvider {
    endpoint: String,
    auth: AuthScheme,
    model: String,
    dims: usize,  // requested vector dimension, e.g. 384 or 1536
    client: reqwest::Client,
}
```

The prompt used is deterministic and fixed, e.g.:
```
Return ONLY a JSON array of {dims} floats representing the semantic embedding of this text. No explanation.
Text: {input}
```

**C) Agent provider (external process)** — `AgentEmbeddingProvider` already existing, integrated as a type in the JSON.

### 3.3 `EmbeddingProvider` trait (already in `embed.rs`)

Unchanged — already abstracted correctly. All three families implement it.

### 3.4 `create_provider()` factory

Maps `provider_type` → concrete implementation:

| type                | family      | base_url        | auth                        |
|---------------------|-------------|-----------------|------------------------------|
| `ollama`            | HTTP native | localhost:11434 | None                         |
| `openai`            | HTTP native | api.openai.com  | BearerToken                  |
| `openai-compatible` | HTTP native | from config     | BearerToken (optional)       |
| `gemini`            | HTTP native | generativelanguage.googleapis.com | QueryParam(`key`) |
| `azure-openai`      | HTTP native | from config     | ApiKeyHeader(`api-key`)      |
| `anthropic`         | Generative  | api.anthropic.com | ApiKeyHeader(`x-api-key`)  |
| `deepseek`          | Generative  | api.deepseek.com | BearerToken                 |
| `agent`             | Process     | —               | —                            |
| any other           | HTTP native | from config (mandatory) | BearerToken          |

### 3.5 Config validation at startup

All the checks happen in `create_provider()` before returning the provider, not at runtime during a query.

```rust
fn requires_api_key(provider_type: &str) -> bool {
    !matches!(provider_type, "ollama" | "agent")
}
```

Explicit errors to return:

| Condition                                            | Message                                                       |
|------------------------------------------------------|----------------------------------------------------------------|
| remote provider + missing `api_key`                  | `"Provider '{type}' requires an API key. Set 'api_key' in config.speedy.json or SPEEDY_API_KEY env var."` |
| `type: "agent"` + missing `command`                  | `"Provider 'agent' requires a 'command' field."` |
| `type: "openai-compatible"` or unknown + missing `base_url` | `"Provider '{type}' requires a 'base_url' field."` |

### 3.6 GenerativeEmbeddingProvider behavior on an unparseable response

If the generative model does not return a valid JSON array of floats:
- **do not retry** — the failure is probably deterministic (wrong model, prompt not followed)
- return an error with the message: `"Generative provider '{type}' returned unparseable embedding. Check that the model supports instruction-following."`
- the chunk is not indexed (same semantics as an HTTP error)

---

## 4. Config merge logic (in `speedy-core/src/config.rs`)

```
fn load_config() -> Config:
    1. Load workspace JSON (.speedy/config.speedy.json)
    2. Load user JSON (~/.speedy/config.speedy.json)
    3. Load workspace TOML (speedy.toml / .speedy/config.toml)  [compat]
    4. Load user TOML [compat]
    5. For each scalar field: first non-None in order:
       env var → workspace JSON → user JSON → workspace TOML → user TOML → default
    6. For ignore_patterns (list): first non-None in order (same), no union
```

Flat TOML → nested JSON mapping for compatibility:
- `model` → `provider.model`
- `ollama_url` → `provider.base_url` (when `provider_type = "ollama"`)
- `provider_type` → `provider.type`
- `agent_command` → `provider.command` (when `provider_type = "agent"`)

**User TOML:** the user TOML (`~/.speedy/config.toml`) does not exist today. It should not be added — the user JSON already covers the case. The cascade simplifies to:
```
env var → workspace JSON → user JSON → workspace TOML → default
```

`Config::from_env()` — used for background tasks where the CWD is incidental — skips **all** files (JSON and TOML). Only: default + env var.

---

## 5. Environment variables

| Env var              | Corresponding field               |
|----------------------|-----------------------------------|
| `SPEEDY_PROVIDER`    | `provider.type`                   |
| `SPEEDY_MODEL`       | `provider.model`                  |
| `SPEEDY_BASE_URL`    | `provider.base_url`               |
| `SPEEDY_API_KEY`     | `provider.api_key`                |
| `SPEEDY_AGENT_COMMAND` | `provider.command`              |
| `SPEEDY_OLLAMA_URL`  | legacy alias → `provider.base_url` |

---

## 6. Implementation steps

- [x] Add `serde_json` and `dirs` to `speedy-core/Cargo.toml` (already present)
- [x] Create `packages/speedy-core/src/provider_config.rs` with `ProviderConfig` and merge logic
- [x] Update `config.rs`: full cascade (JSON + TOML + env) with override rule for `ignore_patterns`
- [x] Update `config.rs`: flat TOML → nested `ProviderConfig` mapping for backward compatibility
- [x] Refactor `embed.rs`: generic `HttpEmbeddingProvider` + `AuthScheme` + `GenerativeEmbeddingProvider`
- [x] Integrate the existing `AgentEmbeddingProvider` as `type: "agent"` in the factory
- [x] Implement the `create_provider()` factory with the complete mapping table
- [x] Add startup validation: missing api_key, missing command for agent, missing base_url for unknown/openai-compatible type
- [x] Rename `SPEEDY_OLLAMA_URL` → `SPEEDY_BASE_URL` (keep the legacy alias)
- [ ] Add `.speedy/config.speedy.json` to the default `.gitignore` generated by `speedy init` (command not yet implemented)
- [x] Update the tests in `embed.rs` for the new transports
- [x] Update `README.md` with the provider table and `config.speedy.json` examples
- [x] Update `CONFIG.md` with the `dims` field in the generative provider examples

---

## 7. Notes

- The vector DB already stores the model name. If the provider/model changes, the existing warning already works — no change needed.
- The E2E tests skip if Ollama is not available: add a skip for remote providers if `SPEEDY_API_KEY` is not set.
- Azure OpenAI uses the `api-key` header — handled by `AuthScheme::ApiKeyHeader`.
- Gemini uses `?key=` in the query string — handled by `AuthScheme::QueryParam`.
- Generative providers (Anthropic, DeepSeek) produce less reliable vectors than native embeddings: document this limitation in the README.
- `dims` for `GenerativeEmbeddingProvider` must be configurable (default 384 for consistency with `all-minilm`).
