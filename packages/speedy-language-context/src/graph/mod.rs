//! Symbol graph types: nodes (symbols) + edges (relations).
//!
//! The store layer (see `store.rs`) persists these into SQLite.

pub mod store;

pub use store::GraphStore;

use std::fmt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub id: i64,
    pub file: String,
    pub kind: SymbolKind,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub is_public: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Interface,
    Class,
    Type,
    Constant,
    Module,
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Interface => "interface",
            SymbolKind::Class => "class",
            SymbolKind::Type => "type",
            SymbolKind::Constant => "constant",
            SymbolKind::Module => "module",
        };
        f.write_str(s)
    }
}

impl SymbolKind {
    pub fn from_db(s: &str) -> SymbolKind {
        match s {
            "function" => SymbolKind::Function,
            "method" => SymbolKind::Method,
            "struct" => SymbolKind::Struct,
            "enum" => SymbolKind::Enum,
            "trait" => SymbolKind::Trait,
            "impl" => SymbolKind::Impl,
            "interface" => SymbolKind::Interface,
            "class" => SymbolKind::Class,
            "type" => SymbolKind::Type,
            "constant" => SymbolKind::Constant,
            "module" => SymbolKind::Module,
            other => {
                tracing::warn!(kind = %other, "unknown SymbolKind in DB, defaulting to Function");
                SymbolKind::Function
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    pub src_id: i64,
    pub dst_id: i64,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Calls,
    Imports,
    Implements,
    References,
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            EdgeKind::Calls => "calls",
            EdgeKind::Imports => "imports",
            EdgeKind::Implements => "implements",
            EdgeKind::References => "references",
        };
        f.write_str(s)
    }
}

impl EdgeKind {
    pub fn from_db(s: &str) -> EdgeKind {
        match s {
            "calls" => EdgeKind::Calls,
            "imports" => EdgeKind::Imports,
            "implements" => EdgeKind::Implements,
            "references" => EdgeKind::References,
            _ => EdgeKind::References,
        }
    }
}
