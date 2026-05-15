# Speedy — Configurazione Provider AI

## Posizioni del file

Speedy cerca `config.speedy.json` in quest'ordine:

1. `<workspace>/.speedy/config.speedy.json` — configurazione del progetto
2. `~/.speedy/config.speedy.json` — configurazione globale utente

Se un campo è assente nel file workspace, viene ereditato dal file utente. Se manca anche lì, si usa il default.

> **Attenzione:** aggiungi `.speedy/config.speedy.json` al tuo `.gitignore` — il file può contenere API key.

---

## Struttura completa

```json
{
  "provider": {
    "type": "ollama",
    "base_url": "http://localhost:11434",
    "model": "all-minilm:l6-v2",
    "api_key": "",
    "command": ""
  },
  "max_chunk_size": 1000,
  "chunk_overlap": 200,
  "top_k": 5,
  "watch_delay_ms": 500,
  "ignore_patterns": ["target/", ".git/", "node_modules/"]
}
```

Tutti i campi sono opzionali. Il default è Ollama su localhost.

---

## Provider

### Ollama (default)

Nessuna API key richiesta. Richiede Ollama in esecuzione localmente.

```json
{
  "provider": {
    "type": "ollama",
    "base_url": "http://localhost:11434",
    "model": "all-minilm:l6-v2"
  }
}
```

Modelli consigliati: `all-minilm:l6-v2`, `nomic-embed-text`, `mxbai-embed-large`

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

Modelli disponibili: `text-embedding-3-small`, `text-embedding-3-large`, `text-embedding-ada-002`

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

### Qualsiasi endpoint OpenAI-compatible

Per provider locali (LM Studio, vLLM, Ollama con API OpenAI, ecc.) o servizi custom.

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

Claude non ha un'API di embedding nativa. Speedy usa il modello generativo per produrre vettori tramite prompt strutturato. Funziona ma è più lento e meno preciso di un embedding nativo.

```json
{
  "provider": {
    "type": "anthropic",
    "model": "claude-3-haiku-20240307",
    "api_key": "sk-ant-..."
  }
}
```

---

### DeepSeek *(proxy embedding)*

Come Anthropic: usa il modello chat come proxy. Vedi nota sopra.

```json
{
  "provider": {
    "type": "deepseek",
    "model": "deepseek-chat",
    "api_key": "..."
  }
}
```

---

### Processo esterno (agent)

Speedy lancia il comando specificato e legge il vettore embedding dallo stdout (formato JSON array di float).

```json
{
  "provider": {
    "type": "agent",
    "command": "python3 my_embedder.py"
  }
}
```

Il processo riceve il testo da embeddare su stdin e deve restituire su stdout un array JSON, es. `[0.12, -0.34, ...]`.

---

## Variabili d'ambiente

Le env var sovrascrivono qualsiasi valore nel file di configurazione.

| Variabile            | Descrizione                          |
|----------------------|--------------------------------------|
| `SPEEDY_PROVIDER`    | Tipo di provider (`ollama`, `openai`…) |
| `SPEEDY_MODEL`       | Modello da usare                     |
| `SPEEDY_BASE_URL`    | URL base dell'endpoint               |
| `SPEEDY_API_KEY`     | API key                              |
| `SPEEDY_AGENT_COMMAND` | Comando per provider `agent`       |
| `SPEEDY_OLLAMA_URL`  | Alias legacy per `SPEEDY_BASE_URL`   |
| `SPEEDY_TOP_K`       | Numero di risultati per query        |

---

## Configurazione tipica: utente globale + override per progetto

**`~/.speedy/config.speedy.json`** — configurazione personale di default:
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

**`<workspace>/.speedy/config.speedy.json`** — override per un progetto specifico (usa Ollama locale, diverso modello):
```json
{
  "provider": {
    "type": "ollama",
    "model": "nomic-embed-text"
  }
}
```

In questo caso `top_k: 10` viene ereditato dal file utente perché il workspace non lo specifica.
