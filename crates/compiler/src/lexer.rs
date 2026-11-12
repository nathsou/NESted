//! Tokens. Comments are kept as trivia for the formatter.
use crate::diag::Diagnostics;
use crate::source::{FileId, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Str(Vec<u8>),
    Char(u8),
    /// Raw text of an `asm { ... }` body, without the braces.
    Asm(String),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// The token's original spelling.
    pub text: String,
    /// A blank line precedes this token.
    pub blank_before: bool,
}

#[derive(Clone, Debug)]
pub struct Comment {
    pub span: Span,
    pub text: String,
    /// The comment follows code on the same line.
    pub trailing: bool,
}

pub const KEYWORDS: &[&str] = &[
    "fn",
    "let",
    "var",
    "const",
    "if",
    "else",
    "while",
    "for",
    "in",
    "loop",
    "break",
    "continue",
    "return",
    "match",
    "struct",
    "enum",
    "type",
    "use",
    "pub",
    "true",
    "false",
    "as",
    "raw",
    "asm",
    "cartridge",
];

const PUNCT: &[&str] = &[
    "<<=", ">>=", "..=", "::", "->", "=>", "==", "!=", "<=", ">=", "<<", ">>", "&&", "||", "+=",
    "-=", "*=", "/=", "%=", "&=", "|=", "^=", "..", "(", ")", "[", "]", "{", "}", ",", ";", ":",
    ".", "=", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "~", "!", "@", "_", "?", "#",
];

pub struct Lexed {
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
}

pub fn lex(file: FileId, src: &str, diags: &mut Diagnostics) -> Lexed {
    let bytes = src.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut comments = Vec::new();
    let mut i = 0;
    let mut newlines = 0usize;
    let mut line_has_code = false;
    // `asm` header state: Some(paren depth) after an `asm` keyword.
    let mut asm_state: Option<i32> = None;
    let span = |s: usize, e: usize| Span::new(file, s, e);
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            newlines += 1;
            line_has_code = false;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let blank_before = newlines >= 2;
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let s = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            comments.push(Comment {
                span: span(s, i),
                text: src[s..i].trim_end().to_owned(),
                trailing: line_has_code,
            });
            if !line_has_code {
                newlines = 0;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let s = i;
            let mut depth = 0;
            while i < bytes.len() {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                diags.error(span(s, i), "Unterminated block comment");
            }
            comments.push(Comment {
                span: span(s, i),
                text: src[s..i].to_owned(),
                trailing: line_has_code,
            });
            continue;
        }
        newlines = 0;
        line_has_code = true;
        let start = i;
        let push = |tokens: &mut Vec<Token>, tok: Tok, end: usize| {
            tokens.push(Token {
                tok,
                span: span(start, end),
                text: src[start..end].to_owned(),
                blank_before,
            });
        };
        if c.is_ascii_alphabetic() || (c == b'_' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')) {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            if word == "asm" {
                asm_state = Some(0);
            }
            push(&mut tokens, Tok::Ident(word.to_owned()), i);
            continue;
        }
        if c.is_ascii_digit() {
            let (radix, digits_start) = match (c, bytes.get(i + 1)) {
                (b'0', Some(b'x' | b'X')) => (16, i + 2),
                (b'0', Some(b'b' | b'B')) => (2, i + 2),
                _ => (10, i),
            };
            i = digits_start;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let digits: String = src[digits_start..i].chars().filter(|c| *c != '_').collect();
            match i64::from_str_radix(&digits, radix) {
                Ok(n) if n <= 0xffff_ffff => push(&mut tokens, Tok::Int(n), i),
                _ => {
                    diags.error(span(start, i), "Invalid integer literal");
                    push(&mut tokens, Tok::Int(0), i);
                }
            }
            continue;
        }
        if c == b'"' {
            i += 1;
            let mut value = Vec::new();
            let mut closed = false;
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => {
                        i += 1;
                        closed = true;
                        break;
                    }
                    b'\\' => {
                        let (b, n) = escape(&bytes[i..]);
                        match b {
                            Some(b) => value.push(b),
                            None => diags.error(span(i, i + n), "Invalid escape"),
                        }
                        i += n;
                    }
                    b => {
                        if b >= 0x80 {
                            diags.error(span(i, i + 1), "Strings must be ASCII");
                        }
                        value.push(b);
                        i += 1;
                    }
                }
            }
            if !closed {
                diags.error(span(start, i), "Unterminated string");
            }
            push(&mut tokens, Tok::Str(value), i);
            continue;
        }
        if c == b'\'' {
            i += 1;
            let (value, n) = if bytes.get(i) == Some(&b'\\') {
                escape(&bytes[i..])
            } else {
                (bytes.get(i).copied().filter(|b| *b < 0x80), 1)
            };
            i += n;
            if bytes.get(i) == Some(&b'\'') {
                i += 1;
            } else {
                diags.error(span(start, i), "Unterminated character literal");
            }
            if value.is_none() {
                diags.error(span(start, i), "Invalid character literal");
            }
            push(&mut tokens, Tok::Char(value.unwrap_or(0)), i);
            continue;
        }
        if c == b'{' && asm_state == Some(0) {
            asm_state = None;
            // Raw assembly body up to the matching brace; `;` comments may
            // contain anything.
            i += 1;
            let body_start = i;
            let mut depth = 1;
            while i < bytes.len() {
                match bytes[i] {
                    b';' => {
                        while i < bytes.len() && bytes[i] != b'\n' {
                            i += 1;
                        }
                        continue;
                    }
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                diags.error(span(start, i), "Unterminated asm block");
            }
            let body = src[body_start..i.min(src.len())].to_owned();
            i = (i + 1).min(bytes.len());
            push(&mut tokens, Tok::Asm(body), i);
            continue;
        }
        match PUNCT.iter().find(|p| src[i..].starts_with(**p)) {
            Some(p) => {
                i += p.len();
                if let Some(depth) = &mut asm_state {
                    match *p {
                        "(" => *depth += 1,
                        ")" => *depth -= 1,
                        _ if *depth == 0 => asm_state = None,
                        _ => {}
                    }
                }
                push(&mut tokens, Tok::Punct(p), i);
            }
            None => {
                let len = src[i..].chars().next().map_or(1, char::len_utf8);
                diags.error(span(i, i + len), "Unexpected character");
                i += len;
            }
        }
    }
    tokens.push(Token {
        tok: Tok::Eof,
        span: span(src.len(), src.len()),
        text: String::new(),
        blank_before: false,
    });
    Lexed { tokens, comments }
}

fn escape(b: &[u8]) -> (Option<u8>, usize) {
    match b.get(1) {
        Some(b'n') => (Some(b'\n'), 2),
        Some(b't') => (Some(b'\t'), 2),
        Some(b'0') => (Some(0), 2),
        Some(b'\\') => (Some(b'\\'), 2),
        Some(b'"') => (Some(b'"'), 2),
        Some(b'\'') => (Some(b'\''), 2),
        Some(b'x') => {
            let hex = b.get(2..4).and_then(|h| std::str::from_utf8(h).ok());
            match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => (Some(v), 4),
                None => (None, 2),
            }
        }
        _ => (None, 2),
    }
}
