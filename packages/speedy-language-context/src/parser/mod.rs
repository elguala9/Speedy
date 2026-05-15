//! Tree-sitter based symbol extraction.
//!
//! Each supported language has its own walk routine in `tree_sitter_parser`.

pub mod tree_sitter_parser;

pub use tree_sitter_parser::{parse_edges, parse_file};

#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub kind: crate::graph::SymbolKind,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub is_public: bool,
}

/// A resolved edge reference extracted from source: caller → callee.
/// Both names are resolved to known symbol names within the same file.
#[derive(Debug, Clone)]
pub struct ParsedEdgeRef {
    pub src_name: String,
    pub dst_name: String,
    pub kind: crate::graph::EdgeKind,
}
