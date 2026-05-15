//! Per-language symbol extraction with tree-sitter.
//!
//! Each grammar crate exposes a `LANGUAGE` constant (`LanguageFn`).
//!
//! Walk strategy: once a *function/method* symbol is extracted we stop
//! recursing (avoids noise from inner closures or nested helpers).
//! For *class/struct/trait/impl* nodes we DO recurse so that methods
//! declared inside are picked up.

use std::collections::HashSet;
use std::path::Path;
use tree_sitter::{Node, Parser, Tree, TreeCursor};

use crate::graph::{EdgeKind, SymbolKind};

use super::{ParsedEdgeRef, ParsedSymbol};

/// Dispatch to the right per-language extractor based on extension.
pub fn parse_file(path: &Path, source: &[u8]) -> Vec<ParsedSymbol> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "rs" => parse_with(tree_sitter_rust::LANGUAGE.into(), source, Lang::Rust),
        "js" | "jsx" | "mjs" | "cjs" => parse_with(
            tree_sitter_javascript::LANGUAGE.into(),
            source,
            Lang::JavaScript,
        ),
        "ts" => parse_with(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            source,
            Lang::TypeScript,
        ),
        "tsx" => parse_with(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            source,
            Lang::TypeScript,
        ),
        "py" | "pyi" => parse_with(tree_sitter_python::LANGUAGE.into(), source, Lang::Python),
        "go" => parse_with(tree_sitter_go::LANGUAGE.into(), source, Lang::Go),
        "c" | "h" => parse_with(tree_sitter_c::LANGUAGE.into(), source, Lang::C),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "h++" => {
            parse_with(tree_sitter_cpp::LANGUAGE.into(), source, Lang::Cpp)
        }
        "java" => parse_with(tree_sitter_java::LANGUAGE.into(), source, Lang::Java),
        "cs" => parse_with(tree_sitter_c_sharp::LANGUAGE.into(), source, Lang::CSharp),
        "rb" | "rake" => parse_with(tree_sitter_ruby::LANGUAGE.into(), source, Lang::Ruby),
        "php" | "php5" | "php7" | "php8" => {
            parse_with(tree_sitter_php::LANGUAGE_PHP.into(), source, Lang::Php)
        }
        "swift" => parse_with(tree_sitter_swift::LANGUAGE.into(), source, Lang::Swift),
        // "kt" | "kts" — tree-sitter-kotlin not yet compatible with tree-sitter 0.23
        "scala" | "sc" => parse_with(tree_sitter_scala::LANGUAGE.into(), source, Lang::Scala),
        _ => Vec::new(),
    }
}

/// Extract call-site edges from a source file using the already-parsed symbols
/// for containment resolution. Only same-file edges are returned.
pub fn parse_edges(path: &Path, source: &[u8], symbols: &[ParsedSymbol]) -> Vec<ParsedEdgeRef> {
    if symbols.is_empty() {
        return Vec::new();
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let (grammar, lang) = match ext.as_str() {
        "rs" => (tree_sitter_rust::LANGUAGE.into(), Lang::Rust),
        "js" | "jsx" | "mjs" | "cjs" => {
            (tree_sitter_javascript::LANGUAGE.into(), Lang::JavaScript)
        }
        "ts" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::TypeScript,
        ),
        "tsx" => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::TypeScript,
        ),
        "py" | "pyi" => (tree_sitter_python::LANGUAGE.into(), Lang::Python),
        "go" => (tree_sitter_go::LANGUAGE.into(), Lang::Go),
        "c" | "h" => (tree_sitter_c::LANGUAGE.into(), Lang::C),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "h++" => {
            (tree_sitter_cpp::LANGUAGE.into(), Lang::Cpp)
        }
        "java" => (tree_sitter_java::LANGUAGE.into(), Lang::Java),
        "cs" => (tree_sitter_c_sharp::LANGUAGE.into(), Lang::CSharp),
        "rb" | "rake" => (tree_sitter_ruby::LANGUAGE.into(), Lang::Ruby),
        "php" | "php5" | "php7" | "php8" => {
            (tree_sitter_php::LANGUAGE_PHP.into(), Lang::Php)
        }
        "swift" => (tree_sitter_swift::LANGUAGE.into(), Lang::Swift),
        // "kt" | "kts" — tree-sitter-kotlin not yet compatible with tree-sitter 0.23
        "scala" | "sc" => (tree_sitter_scala::LANGUAGE.into(), Lang::Scala),
        _ => return Vec::new(),
    };
    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return Vec::new();
    }
    let tree: Tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let known_names: HashSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    let fn_ranges: Vec<(u32, u32, &str)> = symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        .map(|s| (s.start_line, s.end_line, s.name.as_str()))
        .collect();

    let mut out = Vec::new();
    collect_call_edges(tree.root_node(), source, lang, &fn_ranges, &known_names, &mut out);
    out
}

fn find_containing_fn<'a>(line: u32, fn_ranges: &'a [(u32, u32, &'a str)]) -> Option<&'a str> {
    fn_ranges
        .iter()
        .filter(|(start, end, _)| line >= *start && line <= *end)
        .min_by_key(|(start, end, _)| end - start)
        .map(|(_, _, name)| *name)
}

fn collect_call_edges<'a>(
    node: Node<'_>,
    source: &[u8],
    lang: Lang,
    fn_ranges: &'a [(u32, u32, &'a str)],
    known_names: &HashSet<&str>,
    out: &mut Vec<ParsedEdgeRef>,
) {
    let call_kind = match lang {
        Lang::Python => "call",
        Lang::Java => "method_invocation",
        Lang::CSharp => "invocation_expression",
        Lang::Ruby => "method_call",
        Lang::Php => "function_call_expression",
        _ => "call_expression",
    };
    if node.kind() == call_kind {
        if let Some(callee) = extract_callee_name(node, source, lang) {
            if known_names.contains(callee.as_str()) {
                let line = node.start_position().row as u32;
                if let Some(src) = find_containing_fn(line, fn_ranges) {
                    if src != callee {
                        out.push(ParsedEdgeRef {
                            src_name: src.to_string(),
                            dst_name: callee,
                            kind: EdgeKind::Calls,
                        });
                    }
                }
            }
        }
    }
    let mut cursor: TreeCursor<'_> = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_call_edges(cursor.node(), source, lang, fn_ranges, known_names, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn extract_callee_name(node: Node<'_>, source: &[u8], lang: Lang) -> Option<String> {
    match lang {
        Lang::Java => {
            // method_invocation: name field = method name
            node.child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
        }
        _ => {
            let fn_child = node.child_by_field_name("function")?;
            match fn_child.kind() {
                "identifier" => Some(node_text(fn_child, source).to_string()),
                // Rust: foo::bar()
                "scoped_identifier" => fn_child
                    .child_by_field_name("name")
                    .map(|n| node_text(n, source).to_string()),
                // Rust: self.foo() or value.foo()
                "field_expression" => fn_child
                    .child_by_field_name("field")
                    .map(|n| node_text(n, source).to_string()),
                // JS/TS: obj.foo()
                "member_expression" => fn_child
                    .child_by_field_name("property")
                    .map(|n| node_text(n, source).to_string()),
                // Python: obj.foo()
                "attribute" => fn_child
                    .child_by_field_name("attribute")
                    .map(|n| node_text(n, source).to_string()),
                // Go: pkg.Foo()
                "selector_expression" => fn_child
                    .child_by_field_name("field")
                    .map(|n| node_text(n, source).to_string()),
                _ => None,
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Lang {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    C,
    Cpp,
    Java,
    CSharp,
    Ruby,
    Php,
    Swift,
    Scala,
}

fn parse_with(grammar: tree_sitter::Language, source: &[u8], lang: Lang) -> Vec<ParsedSymbol> {
    let mut parser = Parser::new();
    if parser.set_language(&grammar).is_err() {
        return Vec::new();
    }
    let tree: Tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let root = tree.root_node();
    let mut out = Vec::new();
    walk(root, source, lang, &mut out);
    out
}

fn walk(node: Node<'_>, source: &[u8], lang: Lang, out: &mut Vec<ParsedSymbol>) {
    let extracted = extract_symbol(node, source, lang);
    let is_fn = extracted
        .as_ref()
        .map(|s| is_function_kind(&s.kind))
        .unwrap_or(false);
    if let Some(sym) = extracted {
        out.push(sym);
    }

    if is_fn {
        return;
    }

    let mut cursor: TreeCursor<'_> = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk(cursor.node(), source, lang, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn is_function_kind(kind: &crate::graph::SymbolKind) -> bool {
    matches!(
        kind,
        crate::graph::SymbolKind::Function | crate::graph::SymbolKind::Method
    )
}

fn extract_symbol(node: Node<'_>, source: &[u8], lang: Lang) -> Option<ParsedSymbol> {
    match lang {
        Lang::Rust => extract_rust(node, source),
        Lang::JavaScript => extract_js(node, source, false),
        Lang::TypeScript => extract_js(node, source, true),
        Lang::Python => extract_python(node, source),
        Lang::Go => extract_go(node, source),
        Lang::C => extract_c(node, source),
        Lang::Cpp => extract_cpp(node, source),
        Lang::Java => extract_java(node, source),
        Lang::CSharp => extract_csharp(node, source),
        Lang::Ruby => extract_ruby(node, source),
        Lang::Php => extract_php(node, source),
        Lang::Swift => extract_swift(node, source),
        Lang::Scala => extract_scala(node, source),
    }
}

fn node_text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    let range = node.byte_range();
    let bytes = source.get(range).unwrap_or(&[]);
    std::str::from_utf8(bytes).unwrap_or("")
}

fn child_text_with_kind<'a>(node: Node<'_>, source: &'a [u8], kinds: &[&str]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            return Some(node_text(child, source).to_string());
        }
    }
    None
}

/// Returns true if any of the first tokens on the declaration's first line
/// match the given keyword (e.g. "public", "private", "protected").
fn first_line_has_token(node: Node<'_>, source: &[u8], token: &str) -> bool {
    let text = node_text(node, source);
    text.lines()
        .next()
        .map(|line| line.split_whitespace().take(8).any(|t| t == token))
        .unwrap_or(false)
}

fn build_signature(node: Node<'_>, source: &[u8]) -> String {
    let text = node_text(node, source);
    let max_lines = 3.min(text.lines().count().max(1));
    let mut sig = String::new();
    for (i, line) in text.lines().enumerate() {
        if i >= max_lines {
            break;
        }
        if i > 0 {
            sig.push('\n');
        }
        sig.push_str(line.trim_end());
    }
    if sig.len() > 200 {
        sig.truncate(200);
    }
    sig
}

fn make_symbol(
    kind: SymbolKind,
    name: String,
    node: Node<'_>,
    source: &[u8],
    is_public: bool,
) -> ParsedSymbol {
    ParsedSymbol {
        kind,
        name,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        signature: build_signature(node, source),
        is_public,
    }
}

// ─── Rust ────────────────────────────────────────────────────────────

fn extract_rust(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_item" => SymbolKind::Function,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "impl_item" => SymbolKind::Impl,
        "type_alias" | "type_item" => SymbolKind::Type,
        "const_item" => SymbolKind::Constant,
        "mod_item" => SymbolKind::Module,
        _ => return None,
    };

    let is_public = child_text_with_kind(node, source, &["visibility_modifier"])
        .map(|s| s.trim().starts_with("pub"))
        .unwrap_or(false);

    let name = if let Some(name_node) = node.child_by_field_name("name") {
        node_text(name_node, source).to_string()
    } else if kind == SymbolKind::Impl {
        node.child_by_field_name("type")
            .map(|n| node_text(n, source).to_string())
            .unwrap_or_else(|| "impl".to_string())
    } else {
        child_text_with_kind(node, source, &["identifier", "type_identifier"]).unwrap_or_default()
    };

    if name.is_empty() && kind != SymbolKind::Impl {
        return None;
    }
    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── JavaScript / TypeScript ──────────────────────────────────────────

fn extract_js(node: Node<'_>, source: &[u8], is_ts: bool) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_declaration" => SymbolKind::Function,
        "method_definition" => SymbolKind::Method,
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" if is_ts => SymbolKind::Interface,
        "type_alias_declaration" if is_ts => SymbolKind::Type,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .or_else(|| child_text_with_kind(node, source, &["identifier", "property_identifier"]))
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = if is_ts {
        node.parent()
            .map(|p| node_text(p, source).trim_start().starts_with("export"))
            .unwrap_or(false)
            || node_text(node, source).trim_start().starts_with("export")
    } else {
        true
    };

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── Python ───────────────────────────────────────────────────────────

fn extract_python(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_definition" => SymbolKind::Function,
        "class_definition" => SymbolKind::Class,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = !name.starts_with('_');

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── Go ───────────────────────────────────────────────────────────────

fn extract_go(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_declaration" => SymbolKind::Function,
        "method_declaration" => SymbolKind::Method,
        "type_declaration" => SymbolKind::Type,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .or_else(|| child_text_with_kind(node, source, &["identifier", "type_identifier"]))
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── C ────────────────────────────────────────────────────────────────

fn extract_c(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    match node.kind() {
        "function_definition" => {
            let name = c_function_name(node, source)?;
            Some(make_symbol(SymbolKind::Function, name, node, source, true))
        }
        "struct_specifier" | "union_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .filter(|n| !n.is_empty())?;
            Some(make_symbol(SymbolKind::Struct, name, node, source, true))
        }
        "enum_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .filter(|n| !n.is_empty())?;
            Some(make_symbol(SymbolKind::Enum, name, node, source, true))
        }
        _ => None,
    }
}

fn c_function_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let decl = node.child_by_field_name("declarator")?;
    c_declarator_ident(decl, source)
}

fn c_declarator_ident(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(node_text(node, source).to_string()),
        "function_declarator" | "pointer_declarator" | "parenthesized_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|n| c_declarator_ident(n, source)),
        _ => None,
    }
}

// ─── C++ ──────────────────────────────────────────────────────────────

fn extract_cpp(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    match node.kind() {
        "function_definition" => {
            let name = c_function_name(node, source)?;
            Some(make_symbol(SymbolKind::Function, name, node, source, true))
        }
        "struct_specifier" | "union_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .filter(|n| !n.is_empty())?;
            Some(make_symbol(SymbolKind::Struct, name, node, source, true))
        }
        "enum_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .filter(|n| !n.is_empty())?;
            Some(make_symbol(SymbolKind::Enum, name, node, source, true))
        }
        "class_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .filter(|n| !n.is_empty())?;
            Some(make_symbol(SymbolKind::Class, name, node, source, true))
        }
        "namespace_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .filter(|n| !n.is_empty())?;
            Some(make_symbol(SymbolKind::Module, name, node, source, true))
        }
        _ => None,
    }
}

// ─── Java ─────────────────────────────────────────────────────────────

fn extract_java(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "method_declaration" => SymbolKind::Method,
        "constructor_declaration" => SymbolKind::Function,
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = first_line_has_token(node, source, "public");

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── C# ───────────────────────────────────────────────────────────────

fn extract_csharp(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "method_declaration" => SymbolKind::Method,
        "constructor_declaration" => SymbolKind::Function,
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "struct_declaration" => SymbolKind::Struct,
        "enum_declaration" => SymbolKind::Enum,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = first_line_has_token(node, source, "public");

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── Ruby ─────────────────────────────────────────────────────────────

fn extract_ruby(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    match node.kind() {
        "method" | "singleton_method" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                return None;
            }
            let is_public = !name.starts_with('_');
            Some(make_symbol(SymbolKind::Method, name, node, source, is_public))
        }
        "class" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                return None;
            }
            Some(make_symbol(SymbolKind::Class, name, node, source, true))
        }
        "module" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                return None;
            }
            Some(make_symbol(SymbolKind::Module, name, node, source, true))
        }
        _ => None,
    }
}

// ─── PHP ──────────────────────────────────────────────────────────────

fn extract_php(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_definition" => SymbolKind::Function,
        "method_declaration" => SymbolKind::Method,
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "trait_declaration" => SymbolKind::Trait,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = node.kind() != "method_declaration"
        || !child_text_with_kind(node, source, &["visibility_modifier"])
            .map(|m| m == "private" || m == "protected")
            .unwrap_or(false);

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── Swift ────────────────────────────────────────────────────────────

fn extract_swift(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_declaration" => SymbolKind::Function,
        "class_declaration" => SymbolKind::Class,
        "struct_declaration" => SymbolKind::Struct,
        "protocol_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .or_else(|| child_text_with_kind(node, source, &["simple_identifier", "type_identifier"]))
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = first_line_has_token(node, source, "public")
        || first_line_has_token(node, source, "open");

    Some(make_symbol(kind, name, node, source, is_public))
}

// ─── Scala ────────────────────────────────────────────────────────────

fn extract_scala(node: Node<'_>, source: &[u8]) -> Option<ParsedSymbol> {
    let kind = match node.kind() {
        "function_definition" | "function_declaration" => SymbolKind::Function,
        "class_definition" => SymbolKind::Class,
        "trait_definition" => SymbolKind::Trait,
        "object_definition" => SymbolKind::Struct,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).to_string())
        .or_else(|| child_text_with_kind(node, source, &["identifier"]))
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let is_public = !first_line_has_token(node, source, "private")
        && !first_line_has_token(node, source, "protected");

    Some(make_symbol(kind, name, node, source, is_public))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_rust_function() {
        let src = b"pub fn hello() -> u32 { 1 }";
        let syms = parse_file(&PathBuf::from("t.rs"), src);
        assert!(syms.iter().any(|s| s.name == "hello" && s.is_public));
    }

    #[test]
    fn parse_python_class() {
        let src = b"class Foo:\n    def bar(self):\n        pass\n";
        let syms = parse_file(&PathBuf::from("t.py"), src);
        assert!(syms.iter().any(|s| s.name == "Foo"));
    }

    #[test]
    fn unknown_extension_returns_empty() {
        let syms = parse_file(&PathBuf::from("t.unknown"), b"whatever");
        assert!(syms.is_empty());
    }

    #[test]
    fn parse_typescript_function_and_interface() {
        let src = b"export function greet(name: string): string { return name; }\nexport interface Greeter { greet(name: string): string; }";
        let syms = parse_file(&PathBuf::from("t.ts"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "greet" && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "greet function missing"
        );
        assert!(
            syms.iter().any(|s| s.name == "Greeter"
                && matches!(s.kind, crate::graph::SymbolKind::Interface)),
            "Greeter interface missing"
        );
    }

    #[test]
    fn parse_go_function_and_method() {
        let src = b"package main\nfunc Hello() string { return \"hi\" }\ntype Greeter struct{}\nfunc (g Greeter) Greet() string { return \"\" }";
        let syms = parse_file(&PathBuf::from("t.go"), src);
        assert!(
            syms.iter().any(|s| s.name == "Hello"
                && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "Hello function missing"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "Greet" && matches!(s.kind, crate::graph::SymbolKind::Method)),
            "Greet method missing"
        );
    }

    #[test]
    fn parse_javascript_function_and_class() {
        let src = b"function add(a, b) { return a + b; }\nclass Calculator { add(a, b) { return a + b; } }";
        let syms = parse_file(&PathBuf::from("t.js"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "add" && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "add function missing"
        );
        assert!(
            syms.iter().any(|s| s.name == "Calculator"
                && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "Calculator class missing"
        );
    }

    #[test]
    fn parse_python_method_in_class() {
        let src =
            b"class Animal:\n    def speak(self):\n        pass\n    def _private(self):\n        pass\n";
        let syms = parse_file(&PathBuf::from("t.py"), src);
        assert!(syms.iter().any(|s| s.name == "Animal"), "Animal class missing");
        assert!(
            syms.iter().any(|s| s.name == "speak" && s.is_public),
            "speak method missing or not public"
        );
        assert!(
            syms.iter().any(|s| s.name == "_private" && !s.is_public),
            "_private should not be public"
        );
    }

    #[test]
    fn parse_rust_impl_block() {
        let src = b"struct Foo;\nimpl Foo { pub fn new() -> Self { Foo } fn helper(&self) {} }";
        let syms = parse_file(&PathBuf::from("t.rs"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "Foo" && matches!(s.kind, crate::graph::SymbolKind::Struct)),
            "Foo struct missing"
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "new" && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "new fn missing"
        );
    }

    #[test]
    fn parse_edges_rust_same_file_call() {
        let src = b"fn helper() -> u32 { 1 }\nfn main_fn() -> u32 { helper() }";
        let path = PathBuf::from("t.rs");
        let syms = parse_file(&path, src);
        let edges = parse_edges(&path, src, &syms);
        assert!(
            edges
                .iter()
                .any(|e| e.src_name == "main_fn" && e.dst_name == "helper"),
            "expected main_fn -> helper edge, got: {:?}",
            edges
                .iter()
                .map(|e| (&e.src_name, &e.dst_name))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_c_function_and_struct() {
        let src = b"int add(int a, int b) { return a + b; }\nstruct Point { int x; int y; };";
        let syms = parse_file(&PathBuf::from("t.c"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "add" && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "add function missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
        assert!(
            syms.iter()
                .any(|s| s.name == "Point" && matches!(s.kind, crate::graph::SymbolKind::Struct)),
            "Point struct missing"
        );
    }

    #[test]
    fn parse_cpp_class_and_function() {
        let src = b"class Foo { public: void bar() {} };\nvoid standalone() {}";
        let syms = parse_file(&PathBuf::from("t.cpp"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "Foo" && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "Foo class missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
        assert!(
            syms.iter().any(|s| s.name == "standalone"
                && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "standalone function missing"
        );
    }

    #[test]
    fn parse_java_class_and_method() {
        let src =
            b"public class Hello {\n    public void greet() {}\n    private void secret() {}\n}";
        let syms = parse_file(&PathBuf::from("t.java"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "Hello" && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "Hello class missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
        assert!(
            syms.iter().any(|s| s.name == "greet"
                && matches!(s.kind, crate::graph::SymbolKind::Method)
                && s.is_public),
            "greet method missing or not public"
        );
    }

    #[test]
    fn parse_csharp_class_and_method() {
        let src = b"public class MyClass {\n    public void DoWork() {}\n    private int secret;\n}";
        let syms = parse_file(&PathBuf::from("t.cs"), src);
        assert!(
            syms.iter().any(|s| s.name == "MyClass"
                && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "MyClass missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
        assert!(
            syms.iter().any(|s| s.name == "DoWork"
                && matches!(s.kind, crate::graph::SymbolKind::Method)),
            "DoWork method missing"
        );
    }

    #[test]
    fn parse_ruby_class_and_method() {
        let src = b"class Animal\n  def speak\n    'hello'\n  end\nend\n";
        let syms = parse_file(&PathBuf::from("t.rb"), src);
        assert!(
            syms.iter()
                .any(|s| s.name == "Animal" && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "Animal class missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
        assert!(
            syms.iter().any(|s| s.name == "speak"
                && matches!(s.kind, crate::graph::SymbolKind::Method)),
            "speak method missing"
        );
    }

    #[test]
    fn parse_php_function_and_class() {
        let src = b"<?php\nfunction hello() { return 'hi'; }\nclass Greeter {\n    public function greet() {}\n}\n";
        let syms = parse_file(&PathBuf::from("t.php"), src);
        assert!(
            syms.iter().any(|s| s.name == "hello"
                && matches!(s.kind, crate::graph::SymbolKind::Function)),
            "hello function missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
        assert!(
            syms.iter().any(|s| s.name == "Greeter"
                && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "Greeter class missing"
        );
    }

    #[test]
    fn parse_scala_class_and_function() {
        let src = b"class Greeter(name: String) {\n  def greet(): String = s\"Hello, $name\"\n}\nobject Main {\n  def main(args: Array[String]): Unit = {}\n}";
        let syms = parse_file(&PathBuf::from("t.scala"), src);
        assert!(
            syms.iter().any(|s| s.name == "Greeter"
                && matches!(s.kind, crate::graph::SymbolKind::Class)),
            "Greeter class missing; got: {:?}",
            syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
        );
    }

}
