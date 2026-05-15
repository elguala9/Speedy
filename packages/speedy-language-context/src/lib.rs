//! speedy-language-context — local symbol graph + MCP server.
//!
//! Layered like this:
//!
//! ```text
//! cli ──► indexer ──► parser ──► tree-sitter
//!  │           │
//!  │           └──► graph (SQLite)
//!  └──► mcp (JSON-RPC over stdio)
//! ```

pub mod cli;
pub mod daemon_bridge;
pub mod features;
pub mod graph;
pub mod impact;
pub mod indexer;
pub mod mcp;
pub mod memory;
pub mod parser;
pub mod search;
pub mod skeleton;
