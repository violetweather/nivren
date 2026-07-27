use crate::error::NivError;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Semicolon,
    Colon,
    Arrow,
    Question,
    QuestionQuestion,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    BangEqual,
    Equal,
    EqualEqual,
    FatArrow,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Identifier(String),
    Int(i64),
    Float(f64),
    String(String),
    Let,
    Var,
    Fun,
    Return,
    If,
    Else,
    While,
    For,
    In,
    True,
    False,
    Null,
    Print,
    Record,
    Enum,
    Protocol,
    Adopt,
    ForType,
    Match,
    Import,
    Export,
    Through,
    Start,
    Wait,
    Together,
    Race,
    Needs,
    Using,
    And,
    Or,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub column: usize,
}

pub fn scan(source: &str) -> Result<Vec<Token>, Vec<NivError>> {
    Lexer::new(source).scan_tokens()
}

struct Lexer {
    chars: Vec<char>,
    start: usize,
    current: usize,
    line: usize,
    column: usize,
    start_column: usize,
    tokens: Vec<Token>,
    errors: Vec<NivError>,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            start: 0,
            current: 0,
            line: 1,
            column: 1,
            start_column: 1,
            tokens: vec![],
            errors: vec![],
        }
    }

    fn scan_tokens(mut self) -> Result<Vec<Token>, Vec<NivError>> {
        while !self.is_at_end() {
            self.start = self.current;
            self.start_column = self.column;
            self.scan_token();
        }
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            line: self.line,
            column: self.column,
        });
        if self.errors.is_empty() {
            Ok(self.tokens)
        } else {
            Err(self.errors)
        }
    }

    fn scan_token(&mut self) {
        let c = self.advance();
        match c {
            '(' => self.add(TokenKind::LeftParen),
            ')' => self.add(TokenKind::RightParen),
            '{' => self.add(TokenKind::LeftBrace),
            '}' => self.add(TokenKind::RightBrace),
            '[' => self.add(TokenKind::LeftBracket),
            ']' => self.add(TokenKind::RightBracket),
            ',' => self.add(TokenKind::Comma),
            ';' => self.add(TokenKind::Semicolon),
            ':' => self.add(TokenKind::Colon),
            '.' => self.add(TokenKind::Dot),
            '?' => {
                let kind = if self.matches('?') {
                    TokenKind::QuestionQuestion
                } else {
                    TokenKind::Question
                };
                self.add(kind);
            }
            '+' => self.add(TokenKind::Plus),
            '-' => self.add(TokenKind::Minus),
            '*' => self.add(TokenKind::Star),
            '%' => self.add(TokenKind::Percent),
            '!' => {
                let kind = if self.matches('=') {
                    TokenKind::BangEqual
                } else {
                    TokenKind::Bang
                };
                self.add(kind);
            }
            '=' => {
                let kind = if self.matches('=') {
                    TokenKind::EqualEqual
                } else if self.matches('>') {
                    TokenKind::FatArrow
                } else {
                    TokenKind::Equal
                };
                self.add(kind);
            }
            '<' => {
                let kind = if self.matches('=') {
                    TokenKind::LessEqual
                } else {
                    TokenKind::Less
                };
                self.add(kind);
            }
            '>' => {
                let kind = if self.matches('=') {
                    TokenKind::GreaterEqual
                } else {
                    TokenKind::Greater
                };
                self.add(kind);
            }
            '/' if self.matches('/') => {
                while self.peek() != '\n' && !self.is_at_end() {
                    self.advance();
                }
            }
            '/' if self.matches('*') => self.block_comment(),
            '/' => self.add(TokenKind::Slash),
            ' ' | '\r' | '\t' => {}
            '\n' => {
                self.line += 1;
                self.column = 1;
            }
            '"' => self.string(),
            c if c.is_ascii_digit() => self.number(),
            c if is_identifier_start(c) => self.identifier(),
            _ => self.errors.push(NivError::new(
                format!("unexpected character '{c}'"),
                self.line,
                self.start_column,
            )),
        }
    }

    fn block_comment(&mut self) {
        let mut depth = 1;
        while depth > 0 && !self.is_at_end() {
            if self.peek() == '/' && self.peek_next() == '*' {
                self.advance();
                self.advance();
                depth += 1;
            } else if self.peek() == '*' && self.peek_next() == '/' {
                self.advance();
                self.advance();
                depth -= 1;
            } else {
                let c = self.advance();
                if c == '\n' {
                    self.line += 1;
                    self.column = 1;
                }
            }
        }
        if depth > 0 {
            self.errors.push(NivError::new(
                "unterminated block comment",
                self.line,
                self.column,
            ));
        }
    }

    fn string(&mut self) {
        let mut value = String::new();
        while !self.is_at_end() && self.peek() != '"' {
            let c = self.advance();
            if c == '\n' {
                self.line += 1;
                self.column = 1;
                value.push('\n');
            } else if c == '\\' {
                if self.is_at_end() {
                    break;
                }
                let escaped = self.advance();
                value.push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            } else {
                value.push(c);
            }
        }
        if self.is_at_end() {
            self.errors.push(NivError::new(
                "unterminated string",
                self.line,
                self.start_column,
            ));
            return;
        }
        self.advance();
        self.add(TokenKind::String(value));
    }

    fn number(&mut self) {
        while self.peek().is_ascii_digit() {
            self.advance();
        }
        if self.peek() == '.' && self.peek_next().is_ascii_digit() {
            self.advance();
            while self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let text: String = self.chars[self.start..self.current].iter().collect();
        let kind: Result<TokenKind, ()> = if text.contains('.') {
            text.parse().map(TokenKind::Float).map_err(|_| ())
        } else {
            text.parse().map(TokenKind::Int).map_err(|_| ())
        };
        match kind {
            Ok(number) => self.add(number),
            Err(_) => self.errors.push(NivError::new(
                "invalid number",
                self.line,
                self.start_column,
            )),
        }
    }

    fn identifier(&mut self) {
        while is_identifier_continue(self.peek()) {
            self.advance();
        }
        let text: String = self.chars[self.start..self.current].iter().collect();
        let kind = match text.as_str() {
            "keep" => TokenKind::Let,
            "change" => TokenKind::Var,
            "define" => TokenKind::Fun,
            "give" => TokenKind::Return,
            "when" => TokenKind::If,
            "otherwise" => TokenKind::Else,
            "repeat" => TokenKind::While,
            "each" => TokenKind::For,
            "within" => TokenKind::In,
            "yes" => TokenKind::True,
            "no" => TokenKind::False,
            "none" => TokenKind::Null,
            "show" => TokenKind::Print,
            "shape" => TokenKind::Record,
            "choice" => TokenKind::Enum,
            "protocol" => TokenKind::Protocol,
            "adopt" => TokenKind::Adopt,
            "for" => TokenKind::ForType,
            "choose" => TokenKind::Match,
            "use" => TokenKind::Import,
            "expose" => TokenKind::Export,
            "through" => TokenKind::Through,
            "start" => TokenKind::Start,
            "wait" => TokenKind::Wait,
            "together" => TokenKind::Together,
            "race" => TokenKind::Race,
            "needs" => TokenKind::Needs,
            "using" => TokenKind::Using,
            "gives" => TokenKind::Arrow,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            _ => TokenKind::Identifier(text),
        };
        self.add(kind);
    }

    fn add(&mut self, kind: TokenKind) {
        self.tokens.push(Token {
            kind,
            line: self.line,
            column: self.start_column,
        });
    }
    fn is_at_end(&self) -> bool {
        self.current >= self.chars.len()
    }
    fn advance(&mut self) -> char {
        let c = self.chars[self.current];
        self.current += 1;
        self.column += 1;
        c
    }
    fn matches(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.chars[self.current] != expected {
            false
        } else {
            self.current += 1;
            self.column += 1;
            true
        }
    }
    fn peek(&self) -> char {
        if self.is_at_end() {
            '\0'
        } else {
            self.chars[self.current]
        }
    }
    fn peek_next(&self) -> char {
        if self.current + 1 >= self.chars.len() {
            '\0'
        } else {
            self.chars[self.current + 1]
        }
    }
}

fn is_identifier_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}
fn is_identifier_continue(c: char) -> bool {
    is_identifier_start(c) || c.is_ascii_digit()
}
