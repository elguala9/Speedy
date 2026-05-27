use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    Cased,
    IsolatedSpecial,
    Isolated,
}

impl SearchType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchType::Cased => "cased",
            SearchType::IsolatedSpecial => "isolated_special",
            SearchType::Isolated => "isolated",
        }
    }

}

#[derive(Debug, Clone)]
pub struct Token {
    pub symbol: String,
    pub search_type: SearchType,
    pub line_no: u32,
    pub col_start: u32,
    pub col_end: u32,
}

/// Tokenizes text content into `Token` entries.
///
/// Token chars: `[a-zA-Z0-9_-]`. Each contiguous run is one token.
/// Sub-tokens are emitted as `isolated_special` when `_` or `-` appear inside.
/// Type is the narrowest that applies based on surrounding chars.
pub fn tokenize(content: &str) -> Vec<Token> {
    let mut tokens = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_no = (line_idx as u32) + 1;
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;

        while i < len {
            if !is_token_byte(bytes[i]) {
                i += 1;
                continue;
            }

            let start = i;
            while i < len && is_token_byte(bytes[i]) {
                i += 1;
            }
            let end = i;

            let symbol = &line[start..end];
            let prev_char = if start > 0 { line[..start].chars().last() } else { None };
            let next_char = if end < len { line[end..].chars().next() } else { None };
            let st = classify(prev_char, next_char);

            tokens.push(Token {
                symbol: symbol.to_string(),
                search_type: st,
                line_no,
                col_start: start as u32,
                col_end: end as u32,
            });

            // Emit sub-components for tokens containing _ or -
            if symbol.bytes().any(|b| b == b'_' || b == b'-') {
                emit_sub_tokens(symbol, start, line_no, &mut tokens);
            }
        }
    }

    tokens
}

fn emit_sub_tokens(symbol: &str, sym_start: usize, line_no: u32, out: &mut Vec<Token>) {
    let bytes = symbol.as_bytes();
    let len = bytes.len();
    let mut j = 0usize;

    while j < len {
        // skip separator chars
        while j < len && (bytes[j] == b'_' || bytes[j] == b'-') {
            j += 1;
        }
        let part_start = j;
        // find end of part
        while j < len && bytes[j] != b'_' && bytes[j] != b'-' {
            j += 1;
        }
        let part_end = j;
        if part_end > part_start {
            out.push(Token {
                symbol: symbol[part_start..part_end].to_string(),
                search_type: SearchType::IsolatedSpecial,
                line_no,
                col_start: (sym_start + part_start) as u32,
                col_end: (sym_start + part_end) as u32,
            });
        }
    }
}

#[inline]
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn classify(prev: Option<char>, next: Option<char>) -> SearchType {
    // Adjacent chars are never alphanumeric (they would have been included in the
    // [a-zA-Z0-9_-]+ token), so the only distinction is space vs. other non-alnum.
    if is_space_boundary(prev) && is_space_boundary(next) {
        SearchType::Isolated
    } else {
        SearchType::IsolatedSpecial
    }
}

#[inline]
fn is_space_boundary(c: Option<char>) -> bool {
    matches!(c, None | Some(' ') | Some('\t') | Some('\n') | Some('\r'))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn types_for(content: &str, sym: &str) -> Vec<SearchType> {
        tokenize(content)
            .into_iter()
            .filter(|t| t.symbol == sym)
            .map(|t| t.search_type)
            .collect()
    }

    #[test]
    fn isolated_space() {
        let t = types_for("foo Dummy bar", "Dummy");
        assert!(t.contains(&SearchType::Isolated));
    }

    #[test]
    fn isolated_special_dollar() {
        let t = types_for("$Dummy$", "Dummy");
        assert!(t.contains(&SearchType::IsolatedSpecial));
    }

    #[test]
    fn isolated_special_mixed_boundary() {
        // "bar" has space on the left, "." on the right → both non-alnum → isolated_special
        let t = types_for("foo bar.baz", "bar");
        assert!(t.contains(&SearchType::IsolatedSpecial));
    }

    #[test]
    fn sub_tokens_underscore() {
        let tokens = tokenize("my_func");
        let syms: Vec<_> = tokens.iter().map(|t| t.symbol.as_str()).collect();
        assert!(syms.contains(&"my_func"));
        assert!(syms.contains(&"my"));
        assert!(syms.contains(&"func"));
    }
}
