//! `speedy` worker crate. Exposed both as the `speedy` binary and as a
//! library so that benches (and any future embedders) can call the internal
//! indexing / chunking / db primitives without re-implementing them.

pub mod cli;
pub mod db;
pub mod document;
pub mod embed;
pub mod file;
pub mod hash;
pub mod hooks;
pub mod ignore;
pub mod indexer;
pub mod text;

/// Upper bound (in bytes) for files the indexer will load into memory.
/// Files larger than this are skipped — they're almost always generated
/// artifacts, vendored bundles, or data dumps with no semantic value, and
/// reading them whole has crashed the indexer with OOM on workspaces that
/// contain multi-GB Dart `.dart_tool/` files or similar build caches.
///
/// 5 MiB is chosen empirically: covers all reasonable source files, rejects
/// minified bundles and build outputs that slipped past the ignore filter.
pub const MAX_INDEXABLE_FILE_SIZE: u64 = 5 * 1024 * 1024;
