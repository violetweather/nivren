use std::collections::HashSet;

const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

pub fn valid(source: &str) -> bool {
    parse(source).is_ok()
}

pub fn compact(source: &str) -> Result<String, String> {
    let value = parse(source)?;
    let mut output = String::new();
    write_json(&value, &mut output, None, 0);
    Ok(output)
}

pub fn pretty(source: &str) -> Result<String, String> {
    let value = parse(source)?;
    let mut output = String::new();
    write_json(&value, &mut output, Some(2), 0);
    output.push('\n');
    Ok(output)
}

fn parse(source: &str) -> Result<Json, String> {
    if source.len() > MAX_INPUT {
        return Err("JSON input exceeds 16 MiB limit".into());
    }
    let mut parser = Parser {
        chars: source.chars().collect(),
        at: 0,
    };
    let value = parser.value(0)?;
    parser.whitespace();
    if parser.at != parser.chars.len() {
        return Err(parser.error("unexpected trailing JSON data"));
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("JSON nesting limit exceeded"));
        }
        self.whitespace();
        match self.peek() {
            Some('n') => {
                self.keyword("null")?;
                Ok(Json::Null)
            }
            Some('t') => {
                self.keyword("true")?;
                Ok(Json::Bool(true))
            }
            Some('f') => {
                self.keyword("false")?;
                Ok(Json::Bool(false))
            }
            Some('"') => self.string().map(Json::String),
            Some('[') => self.array(depth + 1),
            Some('{') => self.object(depth + 1),
            Some('-' | '0'..='9') => self.number().map(Json::Number),
            Some(_) => Err(self.error("expected JSON value")),
            None => Err(self.error("unexpected end of JSON input")),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1;
        self.whitespace();
        let mut values = vec![];
        if self.take(']') {
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value(depth)?);
            self.whitespace();
            if self.take(']') {
                break;
            }
            self.expect(',', "expected ',' or ']' in array")?;
        }
        Ok(Json::Array(values))
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1;
        self.whitespace();
        let mut values = vec![];
        let mut keys = HashSet::new();
        if self.take('}') {
            return Ok(Json::Object(values));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            if !keys.insert(key.clone()) {
                return Err(self.error("duplicate JSON object key"));
            }
            self.whitespace();
            self.expect(':', "expected ':' after object key")?;
            values.push((key, self.value(depth)?));
            self.whitespace();
            if self.take('}') {
                break;
            }
            self.expect(',', "expected ',' or '}' in object")?;
        }
        Ok(Json::Object(values))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"', "expected JSON string")?;
        let mut value = String::new();
        loop {
            let character = self
                .next()
                .ok_or_else(|| self.error("unterminated JSON string"))?;
            match character {
                '"' => return Ok(value),
                '\\' => {
                    let escaped = self
                        .next()
                        .ok_or_else(|| self.error("unterminated JSON escape"))?;
                    match escaped {
                        '"' | '\\' | '/' => value.push(escaped),
                        'b' => value.push('\u{0008}'),
                        'f' => value.push('\u{000c}'),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        'u' => value.push(self.unicode_escape()?),
                        _ => return Err(self.error("invalid JSON escape")),
                    }
                }
                c if c <= '\u{001f}' => return Err(self.error("control character in JSON string")),
                c => value.push(c),
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            if self.next() != Some('\\') || self.next() != Some('u') {
                return Err(self.error("high surrogate requires a low surrogate"));
            }
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(self.error("invalid low surrogate"));
            }
            0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(self.error("unexpected low surrogate"));
        } else {
            u32::from(first)
        };
        char::from_u32(scalar).ok_or_else(|| self.error("invalid Unicode scalar"))
    }

    fn hex_quad(&mut self) -> Result<u16, String> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = self
                .next()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| self.error("invalid Unicode escape"))?;
            value = value * 16 + u16::try_from(digit).unwrap();
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<String, String> {
        let start = self.at;
        self.take('-');
        if self.take('0') {
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.error("leading zero in JSON number"));
            }
        } else {
            self.digits("expected integer digits")?;
        }
        if self.take('.') {
            self.digits("expected fraction digits")?;
        }
        if self.peek().is_some_and(|c| matches!(c, 'e' | 'E')) {
            self.at += 1;
            if self.peek().is_some_and(|c| matches!(c, '+' | '-')) {
                self.at += 1;
            }
            self.digits("expected exponent digits")?;
        }
        Ok(self.chars[start..self.at].iter().collect())
    }

    fn digits(&mut self, message: &str) -> Result<(), String> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        if self.at == start {
            Err(self.error(message))
        } else {
            Ok(())
        }
    }
    fn keyword(&mut self, keyword: &str) -> Result<(), String> {
        for expected in keyword.chars() {
            self.expect(expected, "invalid JSON keyword")?;
        }
        Ok(())
    }
    fn whitespace(&mut self) {
        while self
            .peek()
            .is_some_and(|character| matches!(character, ' ' | '\t' | '\r' | '\n'))
        {
            self.at += 1;
        }
    }
    fn expect(&mut self, expected: char, message: &str) -> Result<(), String> {
        if self.take(expected) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }
    fn take(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn next(&mut self) -> Option<char> {
        let value = self.peek()?;
        self.at += 1;
        Some(value)
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn error(&self, message: &str) -> String {
        format!("{message} at character {}", self.at + 1)
    }
}

fn write_json(value: &Json, output: &mut String, indent: Option<usize>, depth: usize) {
    match value {
        Json::Null => output.push_str("null"),
        Json::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Json::Number(value) => output.push_str(value),
        Json::String(value) => write_string(value, output),
        Json::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                separator(output, indent, depth + 1, index);
                write_json(value, output, indent, depth + 1);
            }
            close(output, indent, depth, values.is_empty());
            output.push(']');
        }
        Json::Object(values) => {
            output.push('{');
            for (index, (key, value)) in values.iter().enumerate() {
                separator(output, indent, depth + 1, index);
                write_string(key, output);
                output.push(':');
                if indent.is_some() {
                    output.push(' ');
                }
                write_json(value, output, indent, depth + 1);
            }
            close(output, indent, depth, values.is_empty());
            output.push('}');
        }
    }
}

fn separator(output: &mut String, indent: Option<usize>, depth: usize, index: usize) {
    if index > 0 {
        output.push(',');
    }
    if let Some(width) = indent {
        output.push('\n');
        output.push_str(&" ".repeat(width * depth));
    }
}
fn close(output: &mut String, indent: Option<usize>, depth: usize, empty: bool) {
    if !empty {
        if let Some(width) = indent {
            output.push('\n');
            output.push_str(&" ".repeat(width * depth));
        }
    }
}
fn write_string(value: &str, output: &mut String) {
    output.push('"');
    for c in value.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c <= '\u{001f}' => output.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => output.push(c),
        }
    }
    output.push('"');
}
