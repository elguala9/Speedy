/// Minimal hand-rolled tokenizer for a simple expression language.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Eq,
    Comma,
    Eof,
}

#[derive(Debug)]
pub struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let done = tok == Token::Eof;
            tokens.push(tok);
            if done { break; }
        }
        tokens
    }

    fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        if self.pos >= self.src.len() {
            return Token::Eof;
        }
        let ch = self.current();
        match ch {
            '+' => { self.advance(); Token::Plus }
            '-' => { self.advance(); Token::Minus }
            '*' => { self.advance(); Token::Star }
            '/' => { self.advance(); Token::Slash }
            '(' => { self.advance(); Token::LParen }
            ')' => { self.advance(); Token::RParen }
            '=' => { self.advance(); Token::Eq }
            ',' => { self.advance(); Token::Comma }
            '0'..='9' | '.' => self.read_number(),
            'a'..='z' | 'A'..='Z' | '_' => self.read_ident(),
            other => panic!("unexpected char: {other:?}"),
        }
    }

    fn read_number(&mut self) -> Token {
        let start = self.pos;
        while self.pos < self.src.len() && matches!(self.current(), '0'..='9' | '.') {
            self.advance();
        }
        let n: f64 = self.src[start..self.pos].parse().expect("invalid number");
        Token::Number(n)
    }

    fn read_ident(&mut self) -> Token {
        let start = self.pos;
        while self.pos < self.src.len()
            && matches!(self.current(), 'a'..='z' | 'A'..='Z' | '_' | '0'..='9')
        {
            self.advance();
        }
        Token::Ident(self.src[start..self.pos].to_string())
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.src.len() && self.current().is_ascii_whitespace() {
            self.advance();
        }
    }

    fn current(&self) -> char {
        self.src[self.pos..].chars().next().unwrap()
    }

    fn advance(&mut self) {
        self.pos += self.current().len_utf8();
    }
}
