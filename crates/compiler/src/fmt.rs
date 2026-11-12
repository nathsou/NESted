//! Source formatter. It works from tokens and comments, so it formats files
//! with errors as well; only an unterminated literal leaves the text as is.
use crate::diag::Diagnostics;
use crate::lexer::{lex, Comment, Tok, Token};
use crate::source::FileId;

#[derive(Clone, Debug)]
pub struct Style {
    pub indent: String,
    pub width: usize,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            indent: "    ".into(),
            width: 100,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Brace {
    Block,
    /// struct, enum and cartridge bodies: one entry per line.
    Decl,
    Match,
    /// Struct literal: inline.
    Literal,
    /// `use a::{b, c}`: inline.
    Group,
    /// A block written on one line stays on one line.
    Inline,
}

pub fn format(src: &str, style: &Style) -> String {
    let mut diags = Diagnostics::default();
    let lexed = lex(FileId(0), src, &mut diags);
    if diags.items.iter().any(|d| d.message.starts_with("Unterminated")) {
        return src.to_owned();
    }
    let mut f = Formatter {
        style,
        toks: &lexed.tokens,
        comments: &lexed.comments,
        next_comment: 0,
        out: String::new(),
        line: String::new(),
        indent: 0,
        braces: Vec::new(),
        parens: Vec::new(),
        pending_match: Vec::new(),
        pending_decl: false,
        header: false,
        expr_if: None,
        src,
    };
    f.run();
    let mut out = f.out;
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

struct Formatter<'a> {
    style: &'a Style,
    toks: &'a [Token],
    comments: &'a [Comment],
    next_comment: usize,
    out: String,
    line: String,
    indent: usize,
    braces: Vec<Brace>,
    /// Open parens/brackets: true for `[` array literals that may wrap.
    parens: Vec<char>,
    /// Paren depth at which a `match` awaits its `{`.
    pending_match: Vec<usize>,
    pending_decl: bool,
    /// Inside an if/while/for/match header (struct literals not allowed).
    header: bool,
    /// Paren depth of an `if` used as an expression.
    expr_if: Option<usize>,
    src: &'a str,
}

fn is_word(t: &Tok) -> bool {
    matches!(t, Tok::Ident(_) | Tok::Int(_) | Tok::Str(_) | Tok::Char(_))
}

impl Formatter<'_> {
    fn newline(&mut self) {
        let l = self.line.trim_end();
        if !l.is_empty() {
            self.out.push_str(&self.style.indent.repeat(self.indent));
            self.out.push_str(l);
        }
        if !(l.is_empty() && self.out.ends_with("\n\n")) && !(l.is_empty() && self.out.is_empty()) {
            self.out.push('\n');
        }
        self.line.clear();
    }
    fn blank(&mut self) {
        if !self.line.trim().is_empty() {
            self.newline();
        }
        if !self.out.is_empty() && !self.out.ends_with("\n\n") && !self.out.ends_with("{\n") {
            self.out.push('\n');
        }
    }
    fn push(&mut self, s: &str) {
        self.line.push_str(s);
    }
    fn space(&mut self) {
        if !self.line.is_empty() && !self.line.ends_with(' ') {
            self.line.push(' ');
        }
    }
    fn emit_comments_before(&mut self, pos: u32) {
        while self.next_comment < self.comments.len() && self.comments[self.next_comment].span.start < pos {
            let c = &self.comments[self.next_comment];
            self.next_comment += 1;
            if c.trailing && !self.line.trim().is_empty() {
                self.space();
                self.push(&c.text);
                self.newline();
            } else {
                if !self.line.trim().is_empty() {
                    self.newline();
                }
                // A blank line before a comment block is kept.
                let before = &self.src[..c.span.start as usize];
                let gap = before.trim_end_matches([' ', '\t']).ends_with("\n\n") || before.trim_end_matches([' ', '\t']).ends_with("\r\n\r\n");
                if gap {
                    self.blank();
                }
                for (i, l) in c.text.lines().enumerate() {
                    if i > 0 {
                        self.newline();
                    }
                    self.push(if i == 0 { l } else { l.trim_end() });
                }
                self.newline();
            }
        }
    }

    fn run(&mut self) {
        let n = self.toks.len();
        let mut prev: Option<&Tok> = None;
        let mut prev_text = String::new();
        let mut i = 0;
        while i < n {
            let t = &self.toks[i];
            if t.tok == Tok::Eof {
                self.emit_comments_before(u32::MAX);
                break;
            }
            self.emit_comments_before(t.span.start);
            if t.blank_before && self.line.trim().is_empty() && !self.out.is_empty() {
                self.blank();
            }
            let text = t.text.as_str();
            let top = self.braces.last().copied();
            match &t.tok {
                Tok::Asm(body) => {
                    self.space();
                    self.push("{");
                    self.newline();
                    self.indent += 1;
                    for l in body.lines() {
                        let l = l.trim();
                        if l.is_empty() {
                            continue;
                        }
                        let label = l.split(';').next().unwrap_or("").trim_end().ends_with(':');
                        if label {
                            self.indent -= 1;
                            self.push(l);
                            self.newline();
                            self.indent += 1;
                        } else {
                            self.push(l);
                            self.newline();
                        }
                    }
                    self.indent -= 1;
                    self.push("}");
                    let next = self.toks.get(i + 1).map(|t| &t.tok);
                    if !matches!(next, Some(Tok::Punct(";" | "," | ")"))) {
                        self.newline();
                    }
                }
                Tok::Punct("{") => {
                    let close = self.matching_brace(i);
                    let one_line = close.is_some_and(|c| {
                        let inner = &self.src[t.span.end as usize..self.toks[c].span.start as usize];
                        !inner.contains('\n') && !inner.contains('{') && !inner.trim().is_empty() && inner.len() < 64
                            && (prev_text == "=>" || self.expr_if == Some(self.parens.len()))
                    });
                    let kind = if self.pending_decl {
                        self.pending_decl = false;
                        Brace::Decl
                    } else if self.pending_match.last() == Some(&self.parens.len()) {
                        self.pending_match.pop();
                        self.header = false;
                        Brace::Match
                    } else if prev_text == "::" {
                        Brace::Group
                    } else if !self.header
                        && matches!(prev, Some(Tok::Ident(w)) if w.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
                    {
                        Brace::Literal
                    } else if one_line {
                        self.header = false;
                        Brace::Inline
                    } else {
                        self.header = false;
                        Brace::Block
                    };
                    match kind {
                        Brace::Literal | Brace::Group | Brace::Inline => {
                            self.space();
                            self.push("{");
                            // An empty literal stays `{}`.
                            if !matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Punct("}"))) {
                                self.push(" ");
                            }
                        }
                        _ => {
                            self.space();
                            if matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Punct("}"))) && kind == Brace::Block {
                                // Empty block: `{}`.
                                self.push("{}");
                                i += 2;
                                prev = Some(&self.toks[i - 1].tok);
                                prev_text = "}".into();
                                let next = self.toks.get(i).map(|t| &t.tok);
                                if !matches!(next, Some(Tok::Punct(";" | "," | ")")) | Some(Tok::Ident(_))) || matches!(next, Some(Tok::Ident(w)) if w != "else") {
                                    if !matches!(next, Some(Tok::Ident(w)) if w == "else") && !matches!(next, Some(Tok::Punct(_))) {
                                        self.newline();
                                    }
                                }
                                continue;
                            }
                            self.push("{");
                            self.newline();
                            self.indent += 1;
                        }
                    }
                    self.braces.push(kind);
                }
                Tok::Punct("}") => {
                    let kind = self.braces.pop().unwrap_or(Brace::Block);
                    match kind {
                        Brace::Literal | Brace::Group | Brace::Inline => {
                            if self.line.ends_with(' ') && !self.line.ends_with("{ ") {
                                self.line.pop();
                            }
                            if self.line.ends_with("{ ") {
                                self.line.pop();
                            } else {
                                self.push(" ");
                            }
                            self.push("}");
                            if kind == Brace::Inline {
                                let next = self.toks.get(i + 1).map(|t| &t.tok);
                                if !matches!(next, Some(Tok::Ident(w)) if w == "else") && self.expr_if == Some(self.parens.len()) {
                                    self.expr_if = None;
                                }
                                let keep = matches!(next, Some(Tok::Punct(";" | ")" | "," | "}")) | Some(Tok::Eof))
                                    || matches!(next, Some(Tok::Ident(w)) if w == "else");
                                if !keep && matches!(self.braces.last(), Some(Brace::Block | Brace::Match | Brace::Inline) | None) {
                                    self.newline();
                                }
                            }
                        }
                        _ => {
                            if !self.line.trim().is_empty() {
                                // Last entry without a trailing separator.
                                if matches!(kind, Brace::Decl) && !self.line.trim_end().ends_with(',') && !self.line.trim_end().ends_with(';') {
                                    let in_cartridge = self.line.contains('=');
                                    self.push(if in_cartridge { ";" } else { "," });
                                }
                                if matches!(kind, Brace::Match) && !self.line.trim_end().ends_with(',') && !self.line.trim_end().ends_with('}') {
                                    self.push(",");
                                }
                                self.newline();
                            }
                            self.indent = self.indent.saturating_sub(1);
                            self.push("}");
                            let next = self.toks.get(i + 1).map(|t| &t.tok);
                            let keep = matches!(next, Some(Tok::Punct(";" | ")" | ",")))
                                || matches!(next, Some(Tok::Ident(w)) if w == "else");
                            if !keep {
                                if matches!(self.braces.last(), Some(Brace::Match)) {
                                    // Block arm: no comma needed.
                                }
                                self.newline();
                            }
                        }
                    }
                }
                Tok::Punct(";") if top == Some(Brace::Inline) && self.parens.is_empty() => {
                    self.push("; ");
                }
                Tok::Punct(";") => {
                    self.push(";");
                    if self.parens.is_empty() || !matches!(self.parens.last(), Some('[')) {
                        self.newline();
                    }
                }
                Tok::Punct(",") => {
                    self.push(",");
                    if self.parens.last() == Some(&'M') {
                        let next_line = self.toks.get(i + 1).is_some_and(|n| self.src[t.span.end as usize..n.span.start as usize].contains('\n'));
                        if next_line {
                            self.newline();
                        } else {
                            self.push(" ");
                        }
                        prev = Some(&t.tok);
                        prev_text = ",".into();
                        i += 1;
                        continue;
                    }
                    match (self.parens.last(), top) {
                        (None, Some(Brace::Decl | Brace::Match)) => self.newline(),
                        _ => self.push(" "),
                    }
                }
                Tok::Punct(p @ ("(" | "[")) => {
                    let after_word = prev.is_some_and(is_word) || matches!(prev, Some(Tok::Punct(")" | "]")));
                    let after_keyword = matches!(prev, Some(Tok::Ident(w)) if matches!(w.as_str(), "if" | "while" | "return" | "in" | "match" | "let" | "var"));
                    if !after_word || after_keyword {
                        if !(prev_text == "(" || prev_text == "[" || prev_text == "!" || prev_text == "~" || prev_text == "@" || prev_text == "&") || after_keyword {
                            self.space();
                        }
                    }
                    self.push(p);
                    // Multi-line array literals keep one row per source line.
                    let literal = *p == "[" && !after_word;
                    let close = self.matching(i);
                    let multiline = literal && close.is_some_and(|c| self.src[t.span.end as usize..self.toks[c].span.start as usize].contains('\n'));
                    if multiline {
                        self.parens.push('M');
                        self.newline();
                        self.indent += 1;
                    } else {
                        self.parens.push(p.chars().next().unwrap());
                    }
                }
                Tok::Punct(p @ (")" | "]")) => {
                    if self.line.ends_with(' ') {
                        self.line.pop();
                    }
                    if self.parens.last() == Some(&'M') {
                        if !self.line.trim().is_empty() {
                            if !self.line.trim_end().ends_with(',') {
                                self.push(",");
                            }
                            self.newline();
                        }
                        self.indent = self.indent.saturating_sub(1);
                    }
                    self.push(p);
                    self.parens.pop();
                }
                Tok::Punct(p) => {
                    let p = *p;
                    let unary = matches!(p, "-" | "!" | "~" | "&")
                        && (!(prev.is_some_and(is_word) || matches!(prev, Some(Tok::Punct(")" | "]" | "}"))))
                            || matches!(prev, Some(Tok::Ident(w)) if crate::lexer::KEYWORDS.contains(&w.as_str()) && w != "true" && w != "false"));
                    match p {
                        "." | "::" => {
                            if self.line.ends_with(' ') && p == "." && prev_text != "," {
                                self.line.pop();
                            }
                            self.push(p);
                        }
                        ":" => {
                            self.push(": ");
                        }
                        "@" => {
                            self.space();
                            self.push("@");
                        }
                        ".." | "..=" => self.push(p),
                        _ if unary => {
                            self.space();
                            if prev_text == "(" || prev_text == "[" {
                                if self.line.ends_with(' ') {
                                    self.line.pop();
                                }
                            }
                            self.push(p);
                        }
                        "_" => {
                            self.space();
                            self.push("_");
                        }
                        _ => {
                            self.space();
                            self.push(p);
                            self.push(" ");
                        }
                    }
                }
                Tok::Ident(w) => {
                    let glued = matches!(prev_text.as_str(), "." | "::" | "@" | "(" | "[" | "!" | "~" | ".." | "..=")
                        || (prev_text == "-" && self.line.ends_with('-'))
                        || (prev_text == "&" && self.line.ends_with('&'));
                    if !glued {
                        self.space();
                    }
                    self.push(text);
                    match w.as_str() {
                        "struct" | "enum" | "cartridge" => self.pending_decl = true,
                        "match" => {
                            self.pending_match.push(self.parens.len());
                            self.header = true;
                        }
                        "if" => {
                            self.header = true;
                            if matches!(prev_text.as_str(), "=" | "(" | "," | "=>" | "return" | "else") && (prev_text != "else" || self.expr_if.is_some()) {
                                self.expr_if = Some(self.parens.len());
                            }
                        }
                        "while" | "for" => self.header = true,
                        _ => {}
                    }
                    // An attribute without arguments ends its line when it
                    // stands alone (`@inline` before `fn`).
                    if prev_text == "@" && !matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::Punct("("))) {
                        let next_line = self.toks.get(i + 1).is_some_and(|n| self.src[t.span.end as usize..n.span.start as usize].contains('\n'));
                        if next_line {
                            self.newline();
                        }
                    }
                }
                _ => {
                    let glued = matches!(prev_text.as_str(), "(" | "[" | "!" | "~" | ".." | "..=")
                        || (prev_text == "-" && self.line.ends_with('-'));
                    if !glued {
                        self.space();
                    }
                    self.push(text);
                }
            }
            // Attribute argument lists end their line too.
            if text == ")" && self.parens.is_empty() {
                if let Some(open) = self.find_attr_start(i) {
                    let next_line = self.toks.get(i + 1).is_some_and(|n| self.src[t.span.end as usize..n.span.start as usize].contains('\n'));
                    if open && next_line {
                        self.newline();
                    }
                }
            }
            prev = Some(&t.tok);
            prev_text = text.to_owned();
            if matches!(t.tok, Tok::Asm(_)) {
                prev_text = "}".into();
            }
            i += 1;

        }
        if !self.line.trim().is_empty() {
            self.newline();
        }
    }

    fn matching_brace(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for k in open..self.toks.len() {
            match &self.toks[k].tok {
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
        }
        None
    }

    fn matching(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for k in open..self.toks.len() {
            match &self.toks[k].tok {
                Tok::Punct("(" | "[") => depth += 1,
                Tok::Punct(")" | "]") => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(k);
                    }
                }
                Tok::Eof => return None,
                _ => {}
            }
        }
        None
    }

    /// For a `)` closing `@name(...)`, whether it closes an attribute.
    fn find_attr_start(&self, close: usize) -> Option<bool> {
        let mut depth = 0i32;
        let mut k = close;
        loop {
            match &self.toks[k].tok {
                Tok::Punct(")") => depth += 1,
                Tok::Punct("(") => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            if k == 0 {
                return None;
            }
            k -= 1;
        }
        if k >= 2 && matches!(self.toks[k - 2].tok, Tok::Punct("@")) {
            Some(true)
        } else {
            Some(false)
        }
    }
}
