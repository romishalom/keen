//! Lexer (spec section 2).

use crate::error::{Diag, R};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Kw(&'static str),
    Int { text: String, suffix: Option<String> },
    Float { text: String, suffix: Option<String> },
    /// Raw source text between the quotes (escapes preserved).
    Char(String),
    Str(String),
    /// Contents of a raw string, without delimiters.
    RawStr(String),
    /// Label such as `'outer` (stored without the quote).
    Label(String),
    /// `<stdio.h>` (including the angle brackets); only produced after `use`.
    Header(String),
    P(&'static str),
    /// Only produced by `lex_with_comments` (used by the formatter).
    Comment(String),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
    /// Byte range in the (LF-normalized) source text (`lo..hi`).
    pub lo: usize,
    pub hi: usize,
}

pub const KEYWORDS: &[&str] = &[
    "alignof", "as", "atomic", "break", "const", "continue", "do", "else", "enum", "extern",
    "false", "fn", "for", "goto", "if", "impl", "in", "let", "loop", "match", "mod", "mut", "pub",
    "restrict", "return", "Self", "sizeof", "static", "static_assert", "struct", "true", "type",
    "typeof", "union", "use", "volatile", "while",
];

const PUNCTS: &[&str] = &[
    "<<=", ">>=", "..=", "...", "->", "=>", "::", "..", "<<", ">>", "<=", ">=", "==", "!=", "&&",
    "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "+", "-", "*", "/", "%", "&", "|", "^",
    "~", "!", "=", "<", ">", ".", ",", ";", ":", "(", ")", "[", "]", "{", "}", "#",
];

const SUFFIXES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
    "f64",
];

struct Lexer {
    ch: Vec<char>,
    /// Byte offset of every char index (plus one past the end).
    byte: Vec<usize>,
    i: usize,
    line: u32,
    col: u32,
    keep_comments: bool,
}

impl Lexer {
    fn peek(&self, n: usize) -> char {
        *self.ch.get(self.i + n).unwrap_or(&'\0')
    }

    fn eof(&self) -> bool {
        self.i >= self.ch.len()
    }

    fn bump(&mut self) -> char {
        let c = self.peek(0);
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn err<T>(&self, msg: &str) -> R<T> {
        Err(Diag::new("E0101", msg, self.line, self.col))
    }

    fn skip_trivia(&mut self) -> R<()> {
        loop {
            let c = self.peek(0);
            if self.eof() {
                return Ok(());
            }
            if c.is_whitespace() {
                self.bump();
            } else if self.keep_comments && c == '/' && matches!(self.peek(1), '/' | '*') {
                return Ok(());
            } else if c == '/' && self.peek(1) == '/' {
                while !self.eof() && self.peek(0) != '\n' {
                    self.bump();
                }
            } else if c == '/' && self.peek(1) == '*' {
                let (l, cl) = (self.line, self.col);
                self.bump();
                self.bump();
                loop {
                    if self.eof() {
                        return Err(Diag::new("E0101", "unterminated block comment", l, cl));
                    }
                    if self.peek(0) == '*' && self.peek(1) == '/' {
                        self.bump();
                        self.bump();
                        break;
                    }
                    self.bump();
                }
            } else {
                return Ok(());
            }
        }
    }

    fn comment(&mut self) -> R<Tok> {
        let (l, cl) = (self.line, self.col);
        let mut s = String::new();
        if self.peek(1) == '/' {
            while !self.eof() && self.peek(0) != '\n' {
                s.push(self.bump());
            }
            while s.ends_with('\r') {
                s.pop();
            }
            return Ok(Tok::Comment(s));
        }
        s.push(self.bump());
        s.push(self.bump());
        loop {
            if self.eof() {
                return Err(Diag::new("E0101", "unterminated block comment", l, cl));
            }
            if self.peek(0) == '*' && self.peek(1) == '/' {
                s.push(self.bump());
                s.push(self.bump());
                return Ok(Tok::Comment(s));
            }
            s.push(self.bump());
        }
    }

    fn ident_or_raw(&mut self) -> R<Tok> {
        let c = self.peek(0);
        if c == 'r'
            && (self.peek(1) == '"'
                || (self.peek(1) == '#' && (self.peek(2) == '"' || self.peek(2) == '#')))
        {
            return self.raw_string();
        }
        let mut s = String::new();
        while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == '_' {
            s.push(self.bump());
        }
        if s == "_" {
            return Ok(Tok::P("_"));
        }
        if let Some(k) = KEYWORDS.iter().find(|k| **k == s) {
            return Ok(Tok::Kw(k));
        }
        Ok(Tok::Ident(s))
    }

    fn raw_string(&mut self) -> R<Tok> {
        self.bump(); // r
        let mut hashes = 0;
        while self.peek(0) == '#' {
            self.bump();
            hashes += 1;
        }
        if self.peek(0) != '"' {
            return self.err("malformed raw string literal");
        }
        self.bump();
        let mut s = String::new();
        loop {
            if self.eof() {
                return self.err("unterminated raw string literal");
            }
            if self.peek(0) == '"' && (0..hashes).all(|k| self.peek(1 + k) == '#') {
                self.bump();
                for _ in 0..hashes {
                    self.bump();
                }
                return Ok(Tok::RawStr(s));
            }
            s.push(self.bump());
        }
    }

    fn suffix(&mut self) -> R<Option<String>> {
        if self.peek(0).is_ascii_alphabetic() {
            let mut s = String::new();
            while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == '_' {
                s.push(self.bump());
            }
            if SUFFIXES.contains(&s.as_str()) {
                Ok(Some(s))
            } else {
                self.err(&format!("invalid literal suffix `{}`", s))
            }
        } else {
            Ok(None)
        }
    }

    fn digits(&mut self, pred: fn(char) -> bool) -> String {
        let mut s = String::new();
        while pred(self.peek(0)) || self.peek(0) == '_' {
            let c = self.bump();
            if c != '_' {
                s.push(c);
            }
        }
        s
    }

    fn number(&mut self) -> R<Tok> {
        let c1 = self.peek(1);
        if self.peek(0) == '0' && matches!(c1, 'x' | 'X' | 'b' | 'B' | 'o' | 'O') {
            self.bump();
            self.bump();
            let (radix, digits) = match c1 {
                'x' | 'X' => (16, self.digits(|c| c.is_ascii_hexdigit())),
                'b' | 'B' => (2, self.digits(|c| c == '0' || c == '1')),
                _ => (8, self.digits(|c| ('0'..='7').contains(&c))),
            };
            if digits.is_empty() {
                return self.err("numeric literal has no digits");
            }
            let suffix = self.suffix()?;
            let text = match radix {
                16 => format!("0x{}", digits),
                2 => match u128::from_str_radix(&digits, 2) {
                    Ok(v) => format!("0x{:X}", v),
                    Err(_) => return self.err("binary literal too large"),
                },
                _ => {
                    let t = digits.trim_start_matches('0');
                    if t.is_empty() {
                        "0".to_string()
                    } else {
                        format!("0{}", t)
                    }
                }
            };
            return Ok(Tok::Int { text, suffix });
        }
        let mut text = self.digits(|c| c.is_ascii_digit());
        let mut is_float = false;
        if self.peek(0) == '.' && self.peek(1).is_ascii_digit() {
            is_float = true;
            text.push(self.bump());
            text.push_str(&self.digits(|c| c.is_ascii_digit()));
        }
        if matches!(self.peek(0), 'e' | 'E')
            && (self.peek(1).is_ascii_digit()
                || (matches!(self.peek(1), '+' | '-') && self.peek(2).is_ascii_digit()))
        {
            is_float = true;
            text.push(self.bump());
            if matches!(self.peek(0), '+' | '-') {
                text.push(self.bump());
            }
            text.push_str(&self.digits(|c| c.is_ascii_digit()));
        }
        let suffix = self.suffix()?;
        let float_suffix = matches!(suffix.as_deref(), Some("f32") | Some("f64"));
        if is_float || float_suffix {
            Ok(Tok::Float { text, suffix })
        } else {
            // Decimal literals with leading zeros would be octal in C.
            let t = text.trim_start_matches('0');
            let text = if t.is_empty() { "0".to_string() } else { t.to_string() };
            Ok(Tok::Int { text, suffix })
        }
    }

    fn string(&mut self) -> R<Tok> {
        let (l, c) = (self.line, self.col);
        self.bump();
        let mut s = String::new();
        loop {
            if self.eof() || self.peek(0) == '\n' {
                return Err(Diag::new("E0101", "unterminated string literal", l, c));
            }
            let ch = self.bump();
            if ch == '\\' {
                s.push(ch);
                if self.eof() {
                    return Err(Diag::new("E0101", "unterminated string literal", l, c));
                }
                s.push(self.bump());
            } else if ch == '"' {
                return Ok(Tok::Str(s));
            } else {
                s.push(ch);
            }
        }
    }

    /// `'a'` is a character literal; `'name` (no closing quote) is a label.
    fn quote(&mut self) -> R<Tok> {
        self.bump();
        let mut s = String::new();
        if self.peek(0) == '\\' {
            s.push(self.bump());
            s.push(self.bump());
            while !self.eof() && self.peek(0) != '\'' && s.len() < 16 {
                s.push(self.bump());
            }
            if self.peek(0) != '\'' {
                return self.err("unterminated character literal");
            }
            self.bump();
            return Ok(Tok::Char(s));
        }
        if self.peek(1) == '\'' && self.peek(0) != '\'' {
            s.push(self.bump());
            self.bump();
            return Ok(Tok::Char(s));
        }
        if self.peek(0).is_ascii_alphabetic() || self.peek(0) == '_' {
            while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == '_' {
                s.push(self.bump());
            }
            return Ok(Tok::Label(s));
        }
        self.err("malformed character literal or label")
    }

    fn header(&mut self) -> R<Tok> {
        let mut s = String::new();
        s.push(self.bump()); // <
        loop {
            if self.eof() || self.peek(0) == '\n' {
                return self.err("unterminated header name (missing `>`)");
            }
            let c = self.bump();
            s.push(c);
            if c == '>' {
                return Ok(Tok::Header(s));
            }
        }
    }

    fn punct(&mut self) -> R<Tok> {
        for p in PUNCTS {
            let pc: Vec<char> = p.chars().collect();
            if pc.iter().enumerate().all(|(k, c)| self.peek(k) == *c) {
                for _ in 0..pc.len() {
                    self.bump();
                }
                return Ok(Tok::P(p));
            }
        }
        let c = self.peek(0);
        self.err(&format!("unexpected character `{}`", c))
    }
}

pub fn lex(src: &str) -> R<Vec<Token>> {
    lex_impl(src, false)
}

/// Like [`lex`], but comments are returned as `Tok::Comment` tokens.
pub fn lex_with_comments(src: &str) -> R<Vec<Token>> {
    lex_impl(src, true)
}

fn lex_impl(src: &str, keep_comments: bool) -> R<Vec<Token>> {
    // CRLF and LF sources must mean the same thing (raw strings, `c!` blocks, line counts), so the
    // text is normalized here. Token byte offsets refer to this normalized text; the formatter
    // normalizes before lexing, so its offsets always match its own copy.
    let normalized;
    let src = if src.contains('\r') {
        normalized = src.replace("\r\n", "\n");
        normalized.as_str()
    } else {
        src
    };
    let mut byte: Vec<usize> = src.char_indices().map(|(b, _)| b).collect();
    byte.push(src.len());
    let mut lx = Lexer { ch: src.chars().collect(), byte, i: 0, line: 1, col: 1, keep_comments };
    let mut out: Vec<Token> = Vec::new();
    loop {
        lx.skip_trivia()?;
        if lx.eof() {
            let b = lx.byte[lx.i.min(lx.byte.len() - 1)];
            out.push(Token { tok: Tok::Eof, line: lx.line, col: lx.col, lo: b, hi: b });
            return Ok(out);
        }
        let (line, col, start) = (lx.line, lx.col, lx.i);
        let c = lx.peek(0);
        let after_use = matches!(
            out.iter().rev().find(|t| !matches!(t.tok, Tok::Comment(_))),
            Some(Token { tok: Tok::Kw("use"), .. })
        );
        let tok = if keep_comments && c == '/' && matches!(lx.peek(1), '/' | '*') {
            lx.comment()?
        } else if after_use && c == '<' {
            lx.header()?
        } else if c.is_ascii_alphabetic() || c == '_' {
            lx.ident_or_raw()?
        } else if c.is_ascii_digit() {
            lx.number()?
        } else if c == '"' {
            lx.string()?
        } else if c == '\'' {
            lx.quote()?
        } else {
            lx.punct()?
        };
        out.push(Token { tok, line, col, lo: lx.byte[start], hi: lx.byte[lx.i] });
    }
}
