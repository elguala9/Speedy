pub mod tokenizer;

#[cfg(test)]
mod tests {
    use super::tokenizer::{Token, Tokenizer};

    #[test]
    fn tokenizes_arithmetic() {
        let mut t = Tokenizer::new("x = 3.14 + foo(2, y)");
        let tokens = t.tokenize();
        assert_eq!(tokens[0], Token::Ident("x".into()));
        assert_eq!(tokens[1], Token::Eq);
        assert_eq!(tokens[2], Token::Number(3.14));
        assert_eq!(tokens[3], Token::Plus);
        assert_eq!(tokens[4], Token::Ident("foo".into()));
    }
}
