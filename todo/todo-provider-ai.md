# TODO: AI Provider Abstraction

## Obiettivo
Astrarre il provider AI di embedding in modo che Speedy possa funzionare con qualsiasi backend (locale o remoto), non solo Ollama. Il comportamento di default rimane identico all'attuale.

---

## 1. Principio di merge della configurazione

**Vale per ogni campo scalare del config.**

La risoluzione di ogni singola chiave segue questa cascata:

```
env var  >  workspace config.speedy.json  >  utente config.speedy.json  >  default hard-coded
```

**Eccezione — `ignore_patterns` (lista):** override totale. Se il workspace definisce la lista, quella dell'utente viene ignorata completamente. Se il workspace non la definisce, si usa quella dell'utente. Se nessuno la definisce, si usa il default hard-coded. Non si fa union.

---

## 2. Formato del file di configurazione: `config.speedy.json`

File JSON che affianca il TOML esistente (il TOML rimane supportato).

### Posizioni

1. `<workspace>/.speedy/config.speedy.json`  ← workspace, priorità alta
2. `~/.speedy/config.speedy.json`             ← utente, fallback

Su Windows: `~` = `%USERPROFILE%`. Usare il crate `dirs` per la risoluzione cross-platform.

**`config.speedy.json` va aggiunto al `.gitignore` del progetto** — può contenere API key.

### Struttura generale

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

Il file può contenere anche solo alcuni campi — i restanti vengono risolti dalla cascata sopra.

### Esempi provider

**Ollama (default, embedding nativo)**
```json
{ "provider": { "type": "ollama", "model": "all-minilm" } }
```

**OpenAI (embedding nativo)**
```json
{ "provider": { "type": "openai", "model": "text-embedding-3-small", "api_key": "sk-..." } }
```

**Gemini (embedding nativo)**
```json
{ "provider": { "type": "gemini", "model": "text-embedding-004", "api_key": "AIza..." } }
```

**Anthropic / Claude (generativo usato come proxy embedding)**
```json
{ "provider": { "type": "anthropic", "model": "claude-3-haiku-20240307", "api_key": "sk-ant-...", "dims": 384 } }
```

**DeepSeek (generativo usato come proxy embedding)**
```json
{ "provider": { "type": "deepseek", "model": "deepseek-chat", "api_key": "..." } }
```

**Qualsiasi endpoint OpenAI-compatible**
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

**Processo esterno (ex agent_command)**
```json
{ "provider": { "type": "agent", "command": "my-embed-script" } }
```

---

## 3. Design Rust: massima astrazione

### Principio
Non enumerare i provider in una enum — usare una struttura generica. Il `type` è una stringa aperta; il codice sa come adattare il transport in base ad essa.

### 3.1 Struttura config provider (in `speedy-core`)

```rust
pub struct ProviderConfig {
    /// "ollama", "openai", "gemini", "anthropic", "deepseek",
    /// "openai-compatible", "agent", o qualsiasi stringa custom.
    pub provider_type: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    /// Per type="agent": comando da eseguire.
    pub command: Option<String>,
    /// Dimensione del vettore per GenerativeEmbeddingProvider. Default: 384.
    pub dims: Option<usize>,
    /// Headers HTTP aggiuntivi (proxy aziendali, auth custom).
    pub extra_headers: Option<HashMap<String, String>>,
}
```

### 3.2 Due famiglie di provider

**A) Provider con embedding API nativa** — usano `HttpEmbeddingProvider`:

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
    ApiKeyHeader(String, String),  // (header_name, value) — es. Azure usa "api-key"
    QueryParam(String, String),    // (param_name, value) — es. Gemini usa ?key=...
}
```

**B) Provider generativi usati come proxy embedding** — usano `GenerativeEmbeddingProvider`:

Inviano il testo al modello con un prompt strutturato che chiede di restituire un vettore JSON di float. La risposta viene parsata ed estratta.

```rust
struct GenerativeEmbeddingProvider {
    endpoint: String,
    auth: AuthScheme,
    model: String,
    dims: usize,  // dimensione del vettore richiesta, es. 384 o 1536
    client: reqwest::Client,
}
```

Il prompt usato è deterministic e fisso, es.:
```
Return ONLY a JSON array of {dims} floats representing the semantic embedding of this text. No explanation.
Text: {input}
```

**C) Provider agent (processo esterno)** — `AgentEmbeddingProvider` già esistente, integrato come tipo nel JSON.

### 3.3 Trait `EmbeddingProvider` (già in `embed.rs`)

Invariato — già astratto correttamente. Tutte e tre le famiglie lo implementano.

### 3.4 Factory `create_provider()`

Mappa `provider_type` → implementazione concreta:

| type                | famiglia    | base_url        | auth                        |
|---------------------|-------------|-----------------|------------------------------|
| `ollama`            | HTTP native | localhost:11434 | None                         |
| `openai`            | HTTP native | api.openai.com  | BearerToken                  |
| `openai-compatible` | HTTP native | da config       | BearerToken (opzionale)      |
| `gemini`            | HTTP native | generativelanguage.googleapis.com | QueryParam(`key`) |
| `azure-openai`      | HTTP native | da config       | ApiKeyHeader(`api-key`)      |
| `anthropic`         | Generativo  | api.anthropic.com | ApiKeyHeader(`x-api-key`)  |
| `deepseek`          | Generativo  | api.deepseek.com | BearerToken                 |
| `agent`             | Processo    | —               | —                            |
| qualsiasi altro     | HTTP native | da config (obbligatorio) | BearerToken          |

### 3.5 Validazione config all'avvio

Tutti i controlli avvengono in `create_provider()` prima di restituire il provider, non a runtime durante una query.

```rust
fn requires_api_key(provider_type: &str) -> bool {
    !matches!(provider_type, "ollama" | "agent")
}
```

Errori espliciti da restituire:

| Condizione                                           | Messaggio                                                      |
|------------------------------------------------------|----------------------------------------------------------------|
| provider remoto + `api_key` mancante                 | `"Provider '{type}' requires an API key. Set 'api_key' in config.speedy.json or SPEEDY_API_KEY env var."` |
| `type: "agent"` + `command` mancante                 | `"Provider 'agent' requires a 'command' field."` |
| `type: "openai-compatible"` o sconosciuto + `base_url` mancante | `"Provider '{type}' requires a 'base_url' field."` |

### 3.6 Comportamento GenerativeEmbeddingProvider su risposta non parsabile

Se il modello generativo non restituisce un JSON array valido di float:
- **non fare retry** — il fallimento è probabilmente deterministico (modello sbagliato, prompt non seguito)
- restituire errore con messaggio: `"Generative provider '{type}' returned unparseable embedding. Check that the model supports instruction-following."`
- il chunk non viene indicizzato (stessa semantica di un errore HTTP)

---

## 4. Logica di merge config (in `speedy-core/src/config.rs`)

```
fn load_config() -> Config:
    1. Carica JSON workspace (.speedy/config.speedy.json)
    2. Carica JSON utente (~/.speedy/config.speedy.json)
    3. Carica TOML workspace (speedy.toml / .speedy/config.toml)  [compat]
    4. Carica TOML utente [compat]
    5. Per ogni campo scalare: primo non-None in ordine:
       env var → workspace JSON → utente JSON → workspace TOML → utente TOML → default
    6. Per ignore_patterns (lista): primo non-None in ordine (stesso), nessuna union
```

Mapping TOML flat → JSON nested per compatibilità:
- `model` → `provider.model`
- `ollama_url` → `provider.base_url` (quando `provider_type = "ollama"`)
- `provider_type` → `provider.type`
- `agent_command` → `provider.command` (quando `provider_type = "agent"`)

**TOML utente:** il TOML utente (`~/.speedy/config.toml`) non esiste oggi. Non va aggiunto — il JSON utente copre già il caso. La cascata si semplifica a:
```
env var → workspace JSON → utente JSON → workspace TOML → default
```

`Config::from_env()` — usato per background task dove il CWD è incidentale — skippa **tutti** i file (JSON e TOML). Solo: default + env var.

---

## 5. Variabili d'ambiente

| Env var              | Campo corrispondente              |
|----------------------|-----------------------------------|
| `SPEEDY_PROVIDER`    | `provider.type`                   |
| `SPEEDY_MODEL`       | `provider.model`                  |
| `SPEEDY_BASE_URL`    | `provider.base_url`               |
| `SPEEDY_API_KEY`     | `provider.api_key`                |
| `SPEEDY_AGENT_COMMAND` | `provider.command`              |
| `SPEEDY_OLLAMA_URL`  | alias legacy → `provider.base_url` |

---

## 6. Passi implementativi

- [x] Aggiungere `serde_json` e `dirs` a `speedy-core/Cargo.toml` (già presenti)
- [x] Creare `packages/speedy-core/src/provider_config.rs` con `ProviderConfig` e logica merge
- [x] Aggiornare `config.rs`: cascata completa (JSON + TOML + env) con regola override per `ignore_patterns`
- [x] Aggiornare `config.rs`: mapping TOML flat → `ProviderConfig` nested per retrocompatibilità
- [x] Refactoring `embed.rs`: `HttpEmbeddingProvider` generico + `AuthScheme` + `GenerativeEmbeddingProvider`
- [x] Integrare `AgentEmbeddingProvider` esistente come `type: "agent"` nella factory
- [x] Implementare factory `create_provider()` con tabella di mapping completa
- [x] Aggiungere validazione all'avvio: api_key mancante, command mancante per agent, base_url mancante per tipo sconosciuto/openai-compatible
- [x] Rinominare `SPEEDY_OLLAMA_URL` → `SPEEDY_BASE_URL` (mantenere alias legacy)
- [ ] Aggiungere `.speedy/config.speedy.json` al `.gitignore` di default generato da `speedy init` (comando non ancora implementato)
- [x] Aggiornare tests in `embed.rs` per i nuovi transport
- [x] Aggiornare `README.md` con tabella provider e esempi `config.speedy.json`
- [x] Aggiornare `CONFIG.md` con campo `dims` negli esempi provider generativi

---

## 7. Note

- Il DB vettoriale salva già il nome del modello. Se cambia provider/modello, l'avviso esistente funziona già — nessuna modifica necessaria.
- I test E2E skippano se Ollama non è disponibile: aggiungere skip per provider remoti se `SPEEDY_API_KEY` non è settata.
- Azure OpenAI usa `api-key` header — gestito da `AuthScheme::ApiKeyHeader`.
- Gemini usa `?key=` in query string — gestito da `AuthScheme::QueryParam`.
- I provider generativi (Anthropic, DeepSeek) producono vettori meno affidabili degli embedding nativi: documentare questo limite nel README.
- `dims` per `GenerativeEmbeddingProvider` deve essere configurabile (default 384 per coerenza con `all-minilm`).
