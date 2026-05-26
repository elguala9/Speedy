# Speedy — AI Provider Configuration

## File locations

Speedy looks for `config.speedy.json` in this order:

1. `<workspace>/.speedy/config.speedy.json` — project configuration
2. `~/.speedy/config.speedy.json` — global user configuration

If a field is absent in the workspace file, it is inherited from the user file. If missing there too, the default is used.

> **Warning:** add `.speedy/config.speedy.json` to your `.gitignore` — the file may contain API keys.

---

## Full structure

```json
{
  "provider": {
    "type": "ollama",
    "base_url": "http://localhost:11434",
    "model": "all-minilm",
    "api_key": "",
    "command": "",
    "dims": 384
  },
  "max_chunk_size": 1000,
  "chunk_overlap": 200,
  "top_k": 5,
  "watch_delay_ms": 500,
  "ignore_patterns": ["target/", ".git/", "node_modules/"]
}
```

All fields are optional. The default is Ollama on localhost.

| Provider field | Type | Default | Description |
|---|---|---|---|
| `type` | string | `"ollama"` | Provider type |
| `base_url` | string | *(provider default)* | Base URL of the endpoint |
| `model` | string | `"all-minilm"` | Embedding model |
| `api_key` | string | *(empty)* | API key (not needed for Ollama/agent) |
| `command` | string | *(empty)* | Command for `type: "agent"` |
| `dims` | number | `384` | Vector dimension for generative providers (Anthropic, DeepSeek) |

---

## Providers

### Ollama (default)

No API key required. Requires Ollama running locally.

```json
{
  "provider": {
    "type": "ollama",
    "base_url": "http://localhost:11434",
    "model": "all-minilm"
  }
}
```

Default: `all-minilm`. Other models: `nomic-embed-text`, `mxbai-embed-large`, `qwen3-embedding:0.6b`

---

### OpenAI

```json
{
  "provider": {
    "type": "openai",
    "model": "text-embedding-3-small",
    "api_key": "sk-..."
  }
}
```

Available models: `text-embedding-3-small`, `text-embedding-3-large`, `text-embedding-ada-002`

---

### Gemini

```json
{
  "provider": {
    "type": "gemini",
    "model": "text-embedding-004",
    "api_key": "AIza..."
  }
}
```

---

### Any OpenAI-compatible endpoint

For local providers (LM Studio, vLLM, Ollama with OpenAI API, etc.) or custom services.

```json
{
  "provider": {
    "type": "openai-compatible",
    "base_url": "http://localhost:1234/v1",
    "model": "my-model",
    "api_key": "optional"
  }
}
```

---

### Azure OpenAI

```json
{
  "provider": {
    "type": "azure-openai",
    "base_url": "https://<resource>.openai.azure.com/openai/deployments/<deployment>",
    "model": "text-embedding-3-small",
    "api_key": "..."
  }
}
```

---

### Anthropic / Claude *(proxy embedding)*

Claude does not have a native embedding API. Speedy uses the generative model to produce vectors via a structured prompt. It works but is slower and less precise than a native embedding.

```json
{
  "provider": {
    "type": "anthropic",
    "model": "claude-3-haiku-20240307",
    "api_key": "sk-ant-...",
    "dims": 384
  }
}
```

`dims` controls the vector dimension requested from the model (default: 384). Must be consistent between index and query.

---

### DeepSeek *(proxy embedding)*

Like Anthropic: uses the chat model as a proxy. See note above.

```json
{
  "provider": {
    "type": "deepseek",
    "model": "deepseek-chat",
    "api_key": "...",
    "dims": 384
  }
}
```

---

### External process (agent)

Speedy launches the specified command and reads the embedding vector from stdout (JSON float array format).

```json
{
  "provider": {
    "type": "agent",
    "command": "python3 my_embedder.py"
  }
}
```

The process receives the text to embed on stdin and must return a JSON array on stdout, e.g. `[0.12, -0.34, ...]`.

---

## Environment variables

Environment variables override any value in the configuration file.

| Variable | Description |
|---|---|
| `SPEEDY_PROVIDER` | Provider type (`ollama`, `openai`…) |
| `SPEEDY_MODEL` | Model to use |
| `SPEEDY_BASE_URL` | Base URL of the endpoint |
| `SPEEDY_API_KEY` | API key |
| `SPEEDY_AGENT_COMMAND` | Command for `agent` provider |
| `SPEEDY_OLLAMA_URL` | Legacy alias for `SPEEDY_BASE_URL` |
| `SPEEDY_TOP_K` | Number of results per query |

---

## Typical setup: global user config + per-project override

**`~/.speedy/config.speedy.json`** — personal default configuration:
```json
{
  "provider": {
    "type": "openai",
    "model": "text-embedding-3-small",
    "api_key": "sk-..."
  },
  "top_k": 10
}
```

**`<workspace>/.speedy/config.speedy.json`** — override for a specific project (uses local Ollama, different model):
```json
{
  "provider": {
    "type": "ollama",
    "model": "nomic-embed-text"
  }
}
```

In this case `top_k: 10` is inherited from the user file because the workspace does not specify it.
