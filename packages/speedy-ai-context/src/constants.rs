use std::time::Duration;

/// Maximum file size (in bytes) the indexer will load into memory.
/// Files larger than this are skipped — they're almost always generated
/// artifacts, vendored bundles, or data dumps with no semantic value, and
/// reading them whole has crashed the indexer with OOM on workspaces that
/// contain multi-GB Dart `.dart_tool/` files or similar build caches.
///
/// 5 MiB is chosen empirically: covers all reasonable source files, rejects
/// minified bundles and build outputs that slipped past the ignore filter.
pub const MAX_INDEXABLE_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// Hard cap on per-process embed cache entries.
/// With ~6 KB embedding vectors, bounds the cache at roughly 60 MB of RAM —
/// enough to avoid re-embedding duplicate chunks across an indexing pass
/// while preventing unbounded growth on workspaces with tens of thousands
/// of unique chunks.
pub const EMBED_CACHE_MAX_ENTRIES: usize = 10_000;

/// How often to emit a progress log line during bulk indexing.
pub const PROGRESS_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// Files at or above this size are memory-mapped instead of heap-copied
/// during indexing. For files >= 64 KiB, mmap avoids a heap allocation.
pub const MMAP_THRESHOLD: u64 = 64 * 1024;

/// Maximum input characters per chunk sent to Ollama.
/// all-minilm and similar small models have a ~256 token context;
/// truncating to 500 chars avoids "input length exceeds context length" errors.
pub const OLLAMA_MAX_INPUT_CHARS: usize = 500;

/// HTTP timeout (seconds) for single-text embed calls (Ollama legacy provider).
pub const HTTP_TIMEOUT_SINGLE_SECS: u64 = 60;

/// HTTP timeout (seconds) for batch embed calls and generative providers.
pub const HTTP_TIMEOUT_BATCH_SECS: u64 = 120;
