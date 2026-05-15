pub mod preprocessor {
    pub fn normalize_whitespace(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn truncate(text: &str, max_chars: usize) -> String {
        text.chars().take(max_chars).collect()
    }
}

pub mod tokenizer {
    pub fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    pub fn estimate_cost(text: &str, cost_per_token: f64) -> f64 {
        count_tokens(text) as f64 * cost_per_token
    }
}

pub mod chunking {
    pub fn by_paragraphs(text: &str) -> Vec<String> {
        text.split("\n\n")
            .filter(|p| !p.trim().is_empty())
            .map(|p| p.to_string())
            .collect()
    }

    pub fn by_sentences(text: &str) -> Vec<String> {
        let mut sentences = Vec::new();
        let mut current = String::new();
        for ch in text.chars() {
            current.push(ch);
            if ch == '.' || ch == '!' || ch == '?' {
                sentences.push(current.trim().to_string());
                current.clear();
            }
        }
        if !current.trim().is_empty() {
            sentences.push(current.trim().to_string());
        }
        sentences
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(preprocessor::normalize_whitespace("  a   b  c "), "a b c");
        assert_eq!(preprocessor::normalize_whitespace("hello"), "hello");
        assert_eq!(preprocessor::normalize_whitespace(""), "");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(preprocessor::truncate("hello world", 5), "hello");
        assert_eq!(preprocessor::truncate("hi", 5), "hi");
    }

    #[test]
    fn test_count_tokens() {
        assert_eq!(tokenizer::count_tokens("hello world"), 2);
        assert_eq!(tokenizer::count_tokens(""), 0);
        assert_eq!(tokenizer::count_tokens("a b c d e"), 5);
    }

    #[test]
    fn test_by_paragraphs() {
        let text = "p1\n\np2\n\np3";
        let paragraphs = chunking::by_paragraphs(text);
        assert_eq!(paragraphs.len(), 3);
    }

    #[test]
    fn test_by_sentences() {
        let text = "Hello world. How are you? Fine!";
        let sentences = chunking::by_sentences(text);
        assert_eq!(sentences.len(), 3);
        assert_eq!(sentences[0], "Hello world.");
    }

    #[test]
    fn test_normalize_whitespace_tabs_newlines() {
        assert_eq!(preprocessor::normalize_whitespace("a\tb\nc"), "a b c");
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(preprocessor::truncate("", 10), "");
        assert_eq!(preprocessor::truncate("hello", 0), "");
    }

    #[test]
    fn test_truncate_unicode() {
        assert_eq!(preprocessor::truncate("héllo wörld", 6), "héllo ");
    }

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(tokenizer::count_tokens("   "), 0);
    }

    #[test]
    fn test_estimate_cost() {
        let cost = tokenizer::estimate_cost("hello world", 0.5);
        assert!((cost - 1.0).abs() < 1e-6);
        assert_eq!(tokenizer::estimate_cost("", 1.0), 0.0);
    }

    #[test]
    fn test_by_paragraphs_empty() {
        let p = chunking::by_paragraphs("");
        assert!(p.is_empty());
    }

    #[test]
    fn test_by_paragraphs_whitespace_only() {
        let p = chunking::by_paragraphs("  \n\n  \n\n  ");
        assert!(p.is_empty());
    }

    #[test]
    fn test_by_paragraphs_trailing_newlines() {
        let p = chunking::by_paragraphs("p1\n\np2\n\n\n\n");
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn test_by_sentences_empty() {
        let s = chunking::by_sentences("");
        assert!(s.is_empty());
    }

    #[test]
    fn test_by_sentences_no_punctuation() {
        let s = chunking::by_sentences("hello world");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0], "hello world");
    }

    #[test]
    fn test_by_sentences_trailing_text() {
        let s = chunking::by_sentences("Hello. World");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], "Hello.");
        assert_eq!(s[1], "World");
    }
}
