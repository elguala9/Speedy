//! `speedy` worker crate. Exposed both as the `speedy` binary and as a
//! library so that benches (and any future embedders) can call the internal
//! indexing / chunking / db primitives without re-implementing them.

pub mod cli;
pub mod db;
pub mod document;
pub mod embed;
pub mod file;
pub mod hash;
pub mod ignore;
pub mod indexer;
pub mod text;
