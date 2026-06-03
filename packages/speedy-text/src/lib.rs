//! speedy-text — index and query text symbol occurrences in a repo.
//!
//! Layered like this:
//!
//! ```text
//! cli ──► indexer ──► tokenize ──► walk
//!  │           │
//!  │           └──► db (SQLite)
//!  └──► mcp (JSON-RPC over stdio)
//! ```

pub mod config;
pub mod db;
pub mod ignore;
pub mod indexer;
pub mod mcp;
pub mod query;
pub mod replace;
pub mod tokenize;
pub mod walk;
