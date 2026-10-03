use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
    pub warning: bool,
}
impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            warning: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    U8,
    I8,
    U16,
    I16,
    Bool,
    Void,
    Str,
    Array(Box<Ty>, usize),
}
impl Ty {
    pub fn size(&self) -> usize {
        match self {
            Self::U16 | Self::I16 => 2,
            Self::Array(t, n) => t.size() * n,
            Self::Void => 0,
            _ => 1,
        }
    }
    pub fn wide(&self) -> bool {
        self.size() == 2 && !matches!(self, Self::Array(..))
    }
    pub fn signed(&self) -> bool {
        matches!(self, Self::I8 | Self::I16)
    }
    pub fn numeric(&self) -> bool {
        matches!(self, Self::U8 | Self::I8 | Self::U16 | Self::I16)
    }
    pub fn mask(&self) -> i64 {
        if self.wide() {
            65535
        } else {
            255
        }
    }
    pub fn wrap(&self, n: i64) -> i64 {
        let n = n & self.mask();
        if self.signed() {
            let sign = if self.wide() { 32768 } else { 128 };
            if n & sign != 0 {
                n - (sign * 2)
            } else {
                n
            }
        } else {
            n
        }
    }
}
impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Array(t, n) => write!(f, "[{t}; {n}]"),
            _ => write!(
                f,
                "{}",
                match self {
                    Self::U8 => "u8",
                    Self::I8 => "i8",
                    Self::U16 => "u16",
                    Self::I16 => "i16",
                    Self::Bool => "bool",
                    Self::Void => "void",
                    Self::Str => "str",
                    _ => unreachable!(),
                }
            ),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub span: Span,
    pub string: bool,
}
pub fn lex(source: &str) -> Result<Vec<Token>, Vec<Diagnostic>> {
    if source.len() > 2_000_000 {
        return Err(vec![Diagnostic::error(
            Span::default(),
            "Source exceeds 2 MB",
        )]);
    }
    let bytes = source.as_bytes();
    let mut p = 0;
    let mut out = Vec::new();
    let mut errors = Vec::new();
    while p < bytes.len() {
        if bytes[p].is_ascii_whitespace() {
            p += 1;
            continue;
        }
        if source[p..].starts_with("//") {
            while p < bytes.len() && bytes[p] != b'\n' {
                p += 1;
            }
            continue;
        }
        if source[p..].starts_with("/*") {
            let start = p;
            p += 2;
            let mut nesting = 1;
            while p < bytes.len() && nesting > 0 {
                if source[p..].starts_with("/*") {
                    nesting += 1;
                    p += 2;
                } else if source[p..].starts_with("*/") {
                    nesting -= 1;
                    p += 2;
                } else {
                    p += source[p..].chars().next().unwrap().len_utf8();
                }
            }
            if nesting != 0 {
                errors.push(Diagnostic::error(
                    Span { start, end: p },
                    "Unterminated block comment",
                ));
            }
            continue;
        }
        let start = p;
        if bytes[p] == b'"' || bytes[p] == b'\'' {
            let quote = bytes[p];
            p += 1;
            let mut s = String::new();
            let mut closed = false;
            while p < bytes.len() {
                let c = source[p..].chars().next().unwrap();
                p += c.len_utf8();
                if c as u32 == quote as u32 {
                    closed = true;
                    break;
                }
                if c == '\\' {
                    if p >= bytes.len() {
                        break;
                    }
                    let e = bytes[p];
                    p += 1;
                    s.push(match e {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'0' => '\0',
                        b'\\' => '\\',
                        b'"' => '"',
                        b'\'' => '\'',
                        _ => {
                            errors.push(Diagnostic::error(
                                Span {
                                    start: p - 2,
                                    end: p,
                                },
                                "Unsupported escape",
                            ));
                            e as char
                        }
                    });
                } else {
                    s.push(c);
                }
            }
            if !closed {
                errors.push(Diagnostic::error(
                    Span { start, end: p },
                    "Unterminated literal",
                ));
            }
            if quote == b'\'' {
                if s.len() != 1 {
                    errors.push(Diagnostic::error(
                        Span { start, end: p },
                        "Character literals must contain one ASCII byte",
                    ));
                }
                out.push(Token {
                    text: s.as_bytes().first().copied().unwrap_or(0).to_string(),
                    span: Span { start, end: p },
                    string: false,
                });
            } else {
                out.push(Token {
                    text: s,
                    span: Span { start, end: p },
                    string: true,
                });
            }
            continue;
        }
        if bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_' {
            p += 1;
            while p < bytes.len() && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_') {
                p += 1;
            }
        } else {
            let mut found = false;
            for op in [
                "<<=", ">>=", "->", "..", "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "+=",
                "-=", "*=", "/=", "%=", "&=", "|=", "^=",
            ] {
                if source[p..].starts_with(op) {
                    p += op.len();
                    found = true;
                    break;
                }
            }
            if !found {
                let c = source[p..].chars().next().unwrap();
                p += c.len_utf8();
                if !"{}()[]:;,+-*/%&|^~!<>=@.#$".contains(c) {
                    errors.push(Diagnostic::error(
                        Span { start, end: p },
                        format!("Unexpected character '{c}'"),
                    ));
                }
            }
        }
        out.push(Token {
            text: source[start..p].into(),
            span: Span { start, end: p },
            string: false,
        });
        if errors.len() >= 32 {
            break;
        }
    }
    out.push(Token {
        text: "<eof>".into(),
        span: Span { start: p, end: p },
        string: false,
    });
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}
#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Ty,
}
#[derive(Clone, Debug)]
pub enum ExprKind {
    Number(i64),
    String(String),
    Name(String),
    Index(Box<Expr>, Box<Expr>, bool),
    Call(String, Vec<Expr>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Asm(Vec<(String, Expr)>, String),
}
#[derive(Clone, Debug)]
pub enum Stmt {
    Local {
        name: String,
        ty: Option<Ty>,
        value: Expr,
        mutable: bool,
        span: Span,
    },
    Assign {
        target: Expr,
        op: String,
        value: Expr,
    },
    Expr(Expr),
    If {
        condition: Expr,
        yes: Vec<Stmt>,
        no: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        name: String,
        start: Expr,
        end: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    Loop(Vec<Stmt>),
    Break(Span),
    Continue(Span),
    Return(Option<Expr>, Span),
}
#[derive(Clone, Debug)]
pub enum Initial {
    Scalar(Expr),
    List(Vec<Expr>),
    Repeat(Expr, usize),
    Asset(String),
}
#[derive(Clone, Debug)]
pub struct Global {
    pub name: String,
    pub ty: Ty,
    pub initial: Initial,
    pub constant: bool,
    pub attrs: BTreeMap<String, String>,
    pub span: Span,
    pub data: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<(String, Ty, Span)>,
    pub result: Ty,
    pub body: Vec<Stmt>,
    pub attrs: BTreeMap<String, String>,
    pub span: Span,
    pub name_span: Span,
    pub locals: BTreeMap<String, Ty>,
}
#[derive(Clone, Debug)]
pub struct Program {
    pub config: BTreeMap<String, String>,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    pub tokens: Vec<Token>,
}
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
    pub scope: Option<Span>,
    pub kind: u8,
    pub mutable: bool,
}
#[derive(Clone, Debug)]
pub struct Analysis {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: Vec<Symbol>,
    pub tokens: Vec<Token>,
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    p: usize,
    depth: usize,
}
type PResult<T> = Result<T, Diagnostic>;
impl Parser<'_> {
    fn current(&self) -> &Token {
        &self.tokens[self.p]
    }
    fn is(&self, s: &str) -> bool {
        !self.current().string && self.current().text == s
    }
    fn take(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn need(&mut self, s: &str) -> PResult<Token> {
        if self.is(s) {
            let t = self.current().clone();
            self.p += 1;
            Ok(t)
        } else {
            Err(Diagnostic::error(
                self.current().span,
                format!("Expected '{s}', found '{}'", self.current().text),
            ))
        }
    }
    fn ident(&mut self) -> PResult<Token> {
        let t = self.current().clone();
        if !t.string
            && t.text
                .as_bytes()
                .first()
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            self.p += 1;
            Ok(t)
        } else {
            Err(Diagnostic::error(t.span, "Expected identifier"))
        }
    }
    fn number(&mut self) -> PResult<usize> {
        let t = self.current().clone();
        self.p += 1;
        parse_number(&t.text)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| Diagnostic::error(t.span, "Expected nonnegative integer"))
    }
    fn ty(&mut self) -> PResult<Ty> {
        if self.take("[") {
            let t = self.ty()?;
            self.need(";")?;
            let n = self.number()?;
            self.need("]")?;
            if n == 0 || n > 16384 {
                return Err(Diagnostic::error(
                    self.current().span,
                    "Array length must be 1..16384",
                ));
            }
            return Ok(Ty::Array(Box::new(t), n));
        }
        let t = self.ident()?;
        match t.text.as_str() {
            "u8" => Ok(Ty::U8),
            "i8" => Ok(Ty::I8),
            "u16" => Ok(Ty::U16),
            "i16" => Ok(Ty::I16),
            "bool" => Ok(Ty::Bool),
            "void" => Ok(Ty::Void),
            _ => Err(Diagnostic::error(
                t.span,
                format!("Unknown type '{}'", t.text),
            )),
        }
    }
    fn attrs(&mut self) -> PResult<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        while self.take("@") {
            let name = self.ident()?.text;
            let val = if self.take("(") {
                let t = self.current().clone();
                self.p += 1;
                self.need(")")?;
                t.text
            } else {
                String::new()
            };
            out.insert(name, val);
        }
        Ok(out)
    }
    fn expression(&mut self, min: u8) -> PResult<Expr> {
        self.depth += 1;
        if self.depth > 96 {
            return Err(Diagnostic::error(
                self.current().span,
                "Expression nesting exceeds 96",
            ));
        }
        let t = self.current().clone();
        let start = t.span.start;
        let mut e = if ["-", "!", "~", "+"].iter().any(|s| self.is(s)) {
            self.p += 1;
            let x = self.expression(12)?;
            Expr {
                span: Span {
                    start,
                    end: x.span.end,
                },
                kind: ExprKind::Unary(t.text, Box::new(x)),
                ty: Ty::Void,
            }
        } else if self.take("(") {
            let e = self.expression(0)?;
            self.need(")")?;
            e
        } else if self.is("asm") {
            self.p += 1;
            let mut inputs = Vec::new();
            if self.take("(") {
                if !self.is(")") {
                    loop {
                        let r = self.ident()?;
                        if !["a", "x", "y"].contains(&r.text.as_str()) {
                            return Err(Diagnostic::error(
                                r.span,
                                "Assembly input register must be a, x, or y",
                            ));
                        }
                        self.need("=")?;
                        inputs.push((r.text, self.expression(0)?));
                        if !self.take(",") {
                            break;
                        }
                    }
                }
                self.need(")")?;
            }
            let open = self.need("{")?;
            let mut nest = 1;
            let body_start = open.span.end;
            let mut body_end = body_start;
            while !self.is("<eof>") && nest > 0 {
                let q = self.current().clone();
                if !q.string && q.text == "{" {
                    nest += 1;
                }
                if !q.string && q.text == "}" {
                    nest -= 1;
                    if nest == 0 {
                        body_end = q.span.start;
                    }
                }
                self.p += 1;
            }
            if nest != 0 {
                return Err(Diagnostic::error(open.span, "Unterminated assembly block"));
            }
            Expr {
                span: Span {
                    start,
                    end: self.tokens[self.p - 1].span.end,
                },
                kind: ExprKind::Asm(inputs, self.source[body_start..body_end].to_owned()),
                ty: Ty::U8,
            }
        } else if t.string {
            self.p += 1;
            Expr {
                kind: ExprKind::String(t.text),
                span: t.span,
                ty: Ty::Str,
            }
        } else if let Some(n) = parse_number(&t.text) {
            self.p += 1;
            Expr {
                kind: ExprKind::Number(n),
                span: t.span,
                ty: Ty::Void,
            }
        } else {
            let id = self.ident()?;
            let kind = match id.text.as_str() {
                "true" => ExprKind::Number(1),
                "false" => ExprKind::Number(0),
                _ => ExprKind::Name(id.text),
            };
            let ty = if ["true", "false"].contains(&t.text.as_str()) {
                Ty::Bool
            } else {
                Ty::Void
            };
            Expr {
                kind,
                span: id.span,
                ty,
            }
        };
        loop {
            if self.is("(") {
                if let ExprKind::Name(name) = e.kind {
                    self.p += 1;
                    let mut args = Vec::new();
                    if !self.is(")") {
                        loop {
                            args.push(self.expression(0)?);
                            if args.len() > 32 {
                                return Err(Diagnostic::error(e.span, "At most 32 call arguments"));
                            }
                            if !self.take(",") {
                                break;
                            }
                        }
                    }
                    let end = self.need(")")?.span.end;
                    e = Expr {
                        kind: ExprKind::Call(name, args),
                        span: Span { start, end },
                        ty: Ty::Void,
                    };
                    continue;
                } else {
                    return Err(Diagnostic::error(
                        e.span,
                        "Only named functions can be called",
                    ));
                }
            }
            if self.take("[") {
                let raw = self.take("raw");
                let i = self.expression(0)?;
                let end = self.need("]")?.span.end;
                e = Expr {
                    kind: ExprKind::Index(Box::new(e), Box::new(i), raw),
                    span: Span { start, end },
                    ty: Ty::Void,
                };
                continue;
            }
            let op = self.current().text.clone();
            let precedence = match op.as_str() {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                _ => 0,
            };
            if precedence == 0 || precedence < min {
                break;
            }
            self.p += 1;
            let rhs = self.expression(precedence + 1)?;
            let end = rhs.span.end;
            e = Expr {
                kind: ExprKind::Binary(op, Box::new(e), Box::new(rhs)),
                span: Span { start, end },
                ty: Ty::Void,
            };
        }
        self.depth -= 1;
        Ok(e)
    }
    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.need("{")?;
        self.depth += 1;
        if self.depth > 96 {
            return Err(Diagnostic::error(
                self.current().span,
                "Block nesting exceeds 96",
            ));
        }
        let mut body = Vec::new();
        while !self.is("}") && !self.is("<eof>") {
            body.push(self.statement()?);
        }
        self.need("}")?;
        self.depth -= 1;
        Ok(body)
    }
    fn statement(&mut self) -> PResult<Stmt> {
        if self.is("let") || self.is("var") {
            let mutable = self.take("var");
            if !mutable {
                self.need("let")?;
            }
            let id = self.ident()?;
            let ty = if self.take(":") {
                Some(self.ty()?)
            } else {
                None
            };
            self.need("=")?;
            let value = self.expression(0)?;
            self.need(";")?;
            return Ok(Stmt::Local {
                name: id.text,
                ty,
                value,
                mutable,
                span: id.span,
            });
        }
        if self.take("if") {
            let condition = self.expression(0)?;
            let yes = self.block()?;
            let no = if self.take("else") {
                if self.is("if") {
                    vec![self.statement()?]
                } else {
                    self.block()?
                }
            } else {
                Vec::new()
            };
            return Ok(Stmt::If { condition, yes, no });
        }
        if self.take("while") {
            let condition = self.expression(0)?;
            let body = self.block()?;
            return Ok(Stmt::While { condition, body });
        }
        if self.take("for") {
            let id = self.ident()?;
            self.need("in")?;
            let start = self.expression(0)?;
            self.need("..")?;
            let end = self.expression(0)?;
            let body = self.block()?;
            return Ok(Stmt::For {
                name: id.text,
                start,
                end,
                body,
                span: id.span,
            });
        }
        if self.take("loop") {
            return Ok(Stmt::Loop(self.block()?));
        }
        for (name, kind) in [("break", 0), ("continue", 1)] {
            if self.take(name) {
                let s = self.tokens[self.p - 1].span;
                self.need(";")?;
                return Ok(if kind == 0 {
                    Stmt::Break(s)
                } else {
                    Stmt::Continue(s)
                });
            }
        }
        if self.take("return") {
            let s = self.tokens[self.p - 1].span;
            let e = if self.is(";") {
                None
            } else {
                Some(self.expression(0)?)
            };
            self.need(";")?;
            return Ok(Stmt::Return(e, s));
        }
        let target = self.expression(0)?;
        let op = self.current().text.clone();
        if [
            "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
        ]
        .contains(&op.as_str())
        {
            self.p += 1;
            let value = self.expression(0)?;
            self.need(";")?;
            Ok(Stmt::Assign { target, op, value })
        } else {
            if !matches!(target.kind, ExprKind::Asm(..)) {
                self.need(";")?;
            } else {
                self.take(";");
            }
            Ok(Stmt::Expr(target))
        }
    }
    fn program(mut self) -> PResult<Program> {
        let mut config = BTreeMap::new();
        let mut globals = Vec::new();
        let mut functions = Vec::new();
        while !self.is("<eof>") {
            let attrs = self.attrs()?;
            if self.take("cartridge") {
                if self.current().string {
                    config.insert("title".into(), self.current().text.clone());
                    self.p += 1;
                }
                self.need("{")?;
                while !self.is("}") {
                    let k = self.ident()?.text;
                    self.need("=")?;
                    let v = self.current().text.clone();
                    self.p += 1;
                    self.need(";")?;
                    if config.insert(k.clone(), v).is_some() {
                        return Err(Diagnostic::error(
                            self.current().span,
                            format!("Duplicate cartridge setting '{k}'"),
                        ));
                    }
                }
                self.need("}")?;
                continue;
            }
            if self.is("var") || self.is("const") {
                let constant = self.take("const");
                if !constant {
                    self.need("var")?;
                }
                let id = self.ident()?;
                self.need(":")?;
                let ty = self.ty()?;
                self.need("=")?;
                let initial = if self.take("[") {
                    let first = self.expression(0)?;
                    if self.take(";") {
                        let n = self.number()?;
                        self.need("]")?;
                        Initial::Repeat(first, n)
                    } else {
                        let mut v = vec![first];
                        while self.take(",") {
                            if self.is("]") {
                                break;
                            }
                            v.push(self.expression(0)?);
                        }
                        self.need("]")?;
                        Initial::List(v)
                    }
                } else if self.take("asset") {
                    self.need("(")?;
                    let t = self.current().clone();
                    if !t.string {
                        return Err(Diagnostic::error(t.span, "asset requires a literal path"));
                    }
                    self.p += 1;
                    self.need(")")?;
                    Initial::Asset(t.text)
                } else {
                    Initial::Scalar(self.expression(0)?)
                };
                self.need(";")?;
                globals.push(Global {
                    name: id.text,
                    ty,
                    initial,
                    constant,
                    attrs,
                    span: id.span,
                    data: Vec::new(),
                });
                continue;
            }
            if self.take("fn") {
                let id = self.ident()?;
                self.need("(")?;
                let mut params = Vec::new();
                if !self.is(")") {
                    loop {
                        let p = self.ident()?;
                        self.need(":")?;
                        let t = self.ty()?;
                        params.push((p.text, t, p.span));
                        if !self.take(",") {
                            break;
                        }
                    }
                }
                self.need(")")?;
                let result = if self.take("->") {
                    self.ty()?
                } else {
                    Ty::Void
                };
                let body = self.block()?;
                let span = Span {
                    start: id.span.start,
                    end: self.tokens[self.p - 1].span.end,
                };
                functions.push(Function {
                    name: id.text,
                    params,
                    result,
                    body,
                    attrs,
                    span,
                    name_span: id.span,
                    locals: BTreeMap::new(),
                });
                continue;
            }
            return Err(Diagnostic::error(
                self.current().span,
                "Expected cartridge, var, const, or fn",
            ));
        }
        Ok(Program {
            config,
            globals,
            functions,
            tokens: self.tokens,
        })
    }
}
pub fn parse_number(s: &str) -> Option<i64> {
    let s = s.replace('_', "");
    if let Some(s) = s.strip_prefix("0x") {
        i64::from_str_radix(s, 16).ok()
    } else if let Some(s) = s.strip_prefix("0b") {
        i64::from_str_radix(s, 2).ok()
    } else {
        s.parse().ok()
    }
}
pub fn parse(source: &str) -> Result<Program, Vec<Diagnostic>> {
    let tokens = lex(source)?;
    Parser {
        source,
        tokens,
        p: 0,
        depth: 0,
    }
    .program()
    .map_err(|e| vec![e])
}

#[derive(Clone)]
pub struct Builtin {
    pub name: &'static str,
    pub params: Vec<Ty>,
    pub result: Ty,
    pub doc: &'static str,
}
pub fn builtins() -> Vec<Builtin> {
    use Ty::*;
    vec![
    Builtin{name:"buttons",params:vec![],result:U8,doc:"Held controller buttons: A=1, B=2, Select=4, Start=8, Up=16, Down=32, Left=64, Right=128."},
    Builtin{name:"pressed",params:vec![],result:U8,doc:"Buttons newly pressed in this published frame."},
    Builtin{name:"tile",params:vec![U8,U8,U8],result:Void,doc:"Write a nametable tile. Direct during initialization; queued during gameplay (40 writes/frame)."},
    Builtin{name:"text",params:vec![U8,U8,Str],result:Void,doc:"Draw a literal ASCII string using the cartridge font."},
    Builtin{name:"sprite",params:vec![U8,U8,U8,U8,U8],result:Void,doc:"Set an OAM shadow entry: slot, x, y, tile, attributes. DMA occurs on publication."},
    Builtin{name:"hide_sprites",params:vec![],result:Void,doc:"Hide all 64 shadow OAM entries."},
    Builtin{name:"palette",params:vec![U8,U8],result:Void,doc:"Initialize one PPU palette entry; call while rendering is disabled."},
    Builtin{name:"render",params:vec![Bool],result:Void,doc:"Explicitly disable/enable rendering and direct nametable writes. Used for scene transitions."},
    Builtin{name:"screen",params:vec![Str],result:Void,doc:"Copy a literal 1024-byte nametable asset while rendering is disabled."},
    Builtin{name:"screen_bank",params:vec![U8,U16],result:Void,doc:"Explicitly load a 1024-byte nametable from a switched PRG bank and offset."},
    Builtin{name:"tone",params:vec![U8,U16,U8],result:Void,doc:"Set pulse channel 0/1 or triangle channel 2: channel, timer period, volume."},
    Builtin{name:"noise",params:vec![U8,U8],result:Void,doc:"Play a noise-channel instrument: period (0..15), volume (0..15)."},
    Builtin{name:"silence",params:vec![U8],result:Void,doc:"Silence channel 0..3."},
    Builtin{name:"rand",params:vec![],result:U8,doc:"Advance a deterministic 16-bit LFSR and return a byte."},
    Builtin{name:"peek",params:vec![U16],result:U8,doc:"Explicit volatile CPU bus read; register side effects apply."},
    Builtin{name:"poke",params:vec![U16,U8],result:Void,doc:"Explicit volatile CPU bus write, preserving bus access order."},
    Builtin{name:"bank",params:vec![U8],result:Void,doc:"Explicitly select the switchable PRG bank (MMC1/UxROM/MMC3). Code remains fixed."},
    Builtin{name:"bank_peek",params:vec![U8,U16],result:U8,doc:"Select a PRG bank and read a byte at its window-relative offset."},
]
}

pub fn analyze(source: &str) -> Analysis {
    let (mut program, syntax_errors) = match parse(source) {
        Ok(p) => (p, Vec::new()),
        Err(d) => {
            if let Some(p) = recover_program(source) {
                (p, d)
            } else {
                return Analysis {
                    program: None,
                    diagnostics: d,
                    symbols: Vec::new(),
                    tokens: lex(source).unwrap_or_default(),
                };
            }
        }
    };
    let mut a = Checker {
        globals: BTreeMap::new(),
        functions: BTreeMap::new(),
        constants: BTreeMap::new(),
        diagnostics: syntax_errors,
        symbols: Vec::new(),
        bounds: BTreeMap::new(),
        shared: program
            .globals
            .iter()
            .filter(|g| g.attrs.contains_key("shared"))
            .map(|g| g.name.clone())
            .collect(),
        loops: 0,
        result: Ty::Void,
        scope: None,
        all_locals: BTreeMap::new(),
    };
    for b in builtins() {
        a.functions.insert(b.name.into(), (b.params, b.result));
    }
    for g in &program.globals {
        if g.name.starts_with("__") || builtins().iter().any(|b| b.name == g.name) {
            a.error(g.span, "Name is reserved by the compiler/runtime");
        }
        if g.attrs.contains_key("zp") && g.attrs.contains_key("ram") {
            a.error(g.span, "Choose @zp or @ram, not both");
        }
        if g.attrs.contains_key("bank") && (!g.constant || !matches!(g.ty, Ty::Array(..))) {
            a.error(g.span, "@bank requires a constant array");
        }
        if a.globals
            .insert(g.name.clone(), (g.ty.clone(), !g.constant))
            .is_some()
        {
            a.error(g.span, format!("Duplicate global '{}'", g.name));
        }
        if g.ty == Ty::Void || g.ty == Ty::Str {
            a.error(g.span, "Storage requires a fixed-size value type");
        }
        a.symbols.push(Symbol {
            name: g.name.clone(),
            ty: g.ty.clone(),
            span: g.span,
            scope: None,
            kind: if g.constant { 14 } else { 13 },
            mutable: !g.constant,
        });
        if g.constant {
            if let Initial::Scalar(e) = &g.initial {
                if let Some(n) = eval_const(e, &a.constants) {
                    a.constants.insert(g.name.clone(), g.ty.wrap(n));
                }
            }
        }
        for attr in g.attrs.keys() {
            if !["zp", "ram", "bank", "shared"].contains(&attr.as_str()) {
                a.error(g.span, format!("Unknown storage attribute '@{attr}'"));
            }
        }
    }
    for f in &program.functions {
        if f.name.starts_with("__") || a.globals.contains_key(&f.name) {
            a.error(
                f.name_span,
                "Function name collides with reserved or global storage",
            );
        }
        if a.functions
            .insert(
                f.name.clone(),
                (
                    f.params.iter().map(|p| p.1.clone()).collect(),
                    f.result.clone(),
                ),
            )
            .is_some()
        {
            a.error(
                f.name_span,
                format!("Duplicate or reserved function '{}'", f.name),
            );
        }
        a.symbols.push(Symbol {
            name: f.name.clone(),
            ty: f.result.clone(),
            span: f.name_span,
            scope: None,
            kind: 12,
            mutable: false,
        });
        for attr in f.attrs.keys() {
            if ![
                "nmi", "irq", "reset", "export", "inline", "noinline", "cold", "budget",
            ]
            .contains(&attr.as_str())
            {
                a.error(f.name_span, format!("Unknown function attribute '@{attr}'"));
            }
        }
    }
    for g in &mut program.globals {
        let t = if let Ty::Array(t, _) = &g.ty {
            t.as_ref().clone()
        } else {
            g.ty.clone()
        };
        let mut env = BTreeMap::new();
        let mut check_init = |e: &mut Expr| {
            a.expression(e, Some(&t), &env);
            if eval_const(e, &a.constants).is_none() {
                a.error(
                    e.span,
                    "Global initialization must be compile-time constant",
                );
            }
        };
        match &mut g.initial {
            Initial::Scalar(e) => {
                check_init(e);
                if matches!(g.ty, Ty::Array(..)) {
                    a.error(
                        e.span,
                        "Array requires a list, repetition, or asset initializer",
                    );
                }
            }
            Initial::List(v) => {
                for e in v.iter_mut() {
                    check_init(e);
                }
                if !matches!(g.ty,Ty::Array(_,n) if n==v.len()) {
                    a.error(g.span, "Initializer count does not match array length");
                }
            }
            Initial::Repeat(e, n) => {
                check_init(e);
                if !matches!(g.ty,Ty::Array(_,len) if len==*n) {
                    a.error(g.span, "Repetition count does not match array length");
                }
            }
            Initial::Asset(_) => {
                if !g.constant || !matches!(g.ty,Ty::Array(ref t,_) if **t==Ty::U8) {
                    a.error(g.span, "Assets require const [u8; N] storage");
                }
            }
        }
        env.clear();
    }
    for f in &mut program.functions {
        let mut env = BTreeMap::new();
        a.bounds.clear();
        a.all_locals.clear();
        a.scope = Some(f.span);
        a.result = f.result.clone();
        a.loops = 0;
        if !f.result.numeric() && !matches!(f.result, Ty::Void | Ty::Bool) {
            a.error(
                f.name_span,
                "Functions return scalar fixed-width values or void",
            );
        }
        for (name, t, s) in &f.params {
            if a.globals.contains_key(name)
                || name.starts_with("__")
                || builtins().iter().any(|b| b.name == name)
            {
                a.error(*s, "Parameter name shadows global or reserved storage");
            }
            if !t.numeric() && *t != Ty::Bool {
                a.error(*s, "Parameters require scalar fixed-width types");
            }
            if env.insert(name.clone(), (t.clone(), true)).is_some() {
                a.error(*s, "Duplicate parameter");
            }
            a.all_locals.insert(name.clone(), t.clone());
            a.symbols.push(Symbol {
                name: name.clone(),
                ty: t.clone(),
                span: *s,
                scope: Some(f.span),
                kind: 13,
                mutable: true,
            });
        }
        a.block(&mut f.body, &mut env);
        f.locals = a.all_locals.clone();
        if f.result != Ty::Void && !terminates(&f.body) {
            a.error(
                f.name_span,
                "Value-returning function must return on every path",
            );
        }
    }
    let tokens = lex(source).unwrap_or_else(|_| program.tokens.clone());
    Analysis {
        program: Some(program),
        diagnostics: a.diagnostics,
        symbols: a.symbols,
        tokens,
    }
}
struct Checker {
    globals: BTreeMap<String, (Ty, bool)>,
    functions: BTreeMap<String, (Vec<Ty>, Ty)>,
    constants: BTreeMap<String, i64>,
    diagnostics: Vec<Diagnostic>,
    symbols: Vec<Symbol>,
    bounds: BTreeMap<String, (i64, i64)>,
    shared: BTreeSet<String>,
    loops: usize,
    result: Ty,
    scope: Option<Span>,
    all_locals: BTreeMap<String, Ty>,
}
impl Checker {
    fn error(&mut self, s: Span, m: impl Into<String>) {
        if self.diagnostics.len() < 64 {
            self.diagnostics.push(Diagnostic::error(s, m));
        }
    }
    fn hint(&self, e: &Expr, env: &BTreeMap<String, (Ty, bool)>) -> Option<Ty> {
        match &e.kind {
            ExprKind::Name(n) => env.get(n).or(self.globals.get(n)).map(|v| v.0.clone()),
            ExprKind::Call(n, _) => {
                self.functions
                    .get(n)
                    .map(|v| v.1.clone())
                    .or(match n.as_str() {
                        "u8" => Some(Ty::U8),
                        "i8" => Some(Ty::I8),
                        "u16" => Some(Ty::U16),
                        "i16" => Some(Ty::I16),
                        "bool" => Some(Ty::Bool),
                        _ => None,
                    })
            }
            ExprKind::Binary(_, l, r) => self.hint(l, env).or_else(|| self.hint(r, env)),
            ExprKind::Index(b, _, _) => {
                if let Some(Ty::Array(t, _)) = self.hint(b, env) {
                    Some(*t)
                } else {
                    None
                }
            }
            ExprKind::Unary(_, e) => self.hint(e, env),
            _ => None,
        }
    }
    fn expression(
        &mut self,
        e: &mut Expr,
        expected: Option<&Ty>,
        env: &BTreeMap<String, (Ty, bool)>,
    ) -> Ty {
        let t = match &mut e.kind {
            ExprKind::Number(n) => {
                let t = if e.ty == Ty::Bool {
                    Ty::Bool
                } else {
                    expected
                        .filter(|t| t.numeric() || **t == Ty::Bool)
                        .cloned()
                        .unwrap_or(if *n > 255 { Ty::U16 } else { Ty::U8 })
                };
                let (lo, hi) = if t.signed() {
                    if t.wide() {
                        (-32768, 32767)
                    } else {
                        (-128, 127)
                    }
                } else {
                    (0, t.mask())
                };
                if *n < lo || *n > hi {
                    self.error(
                        e.span,
                        format!("Literal {n} does not fit {t}; use an explicit cast"),
                    );
                }
                t
            }
            ExprKind::String(_) => Ty::Str,
            ExprKind::Name(n) => match env.get(n).or(self.globals.get(n)) {
                Some((t, _)) => t.clone(),
                None => {
                    self.error(e.span, format!("Unknown name '{n}'"));
                    Ty::U8
                }
            },
            ExprKind::Index(base, index, raw) => {
                let b = self.expression(base, None, env);
                let i = self.expression(index, None, env);
                if !i.numeric() {
                    self.error(index.span, "Array index must be an integer");
                }
                if let Ty::Array(t, n) = b {
                    if let Some(v) = eval_const(index, &self.constants) {
                        if v < 0 || v as usize >= n {
                            self.error(
                                index.span,
                                format!("Index {v} is outside array length {n}"),
                            );
                        }
                    } else if !*raw {
                        let (lo, hi) = self.range(index);
                        if lo < 0 || hi >= n as i64 {
                            self.error(index.span,format!("Index is not proven within 0..{n}; guard it or use [raw index]"));
                        }
                    }
                    *t
                } else {
                    self.error(base.span, "Indexing requires an array");
                    Ty::U8
                }
            }
            ExprKind::Call(name, args) => {
                if ["u8", "i8", "u16", "i16", "bool"].contains(&name.as_str()) {
                    if args.len() != 1 {
                        self.error(e.span, "A cast takes one argument");
                    }
                    for x in args.iter_mut() {
                        self.expression(x, None, env);
                    }
                    match name.as_str() {
                        "u8" => Ty::U8,
                        "i8" => Ty::I8,
                        "u16" => Ty::U16,
                        "i16" => Ty::I16,
                        _ => Ty::Bool,
                    }
                } else if let Some((params, result)) = self.functions.get(name).cloned() {
                    if params.len() != args.len() {
                        self.error(
                            e.span,
                            format!(
                                "{name} expects {} arguments, received {}",
                                params.len(),
                                args.len()
                            ),
                        );
                    }
                    for (i, x) in args.iter_mut().enumerate() {
                        self.expression(x, params.get(i), env);
                    }
                    if ["text", "screen"].contains(&name.as_str())
                        && args
                            .last()
                            .is_some_and(|x| !matches!(x.kind, ExprKind::String(_)))
                    {
                        self.error(e.span, "Text/asset argument must be a literal string");
                    }
                    if !builtins().iter().any(|b| b.name == name) {
                        for global in self.globals.keys() {
                            self.bounds.remove(global);
                        }
                    }
                    result
                } else {
                    self.error(e.span, format!("Unknown function '{name}'"));
                    for x in args {
                        self.expression(x, None, env);
                    }
                    Ty::U8
                }
            }
            ExprKind::Unary(op, x) => {
                let t = self.expression(x, if op == "!" { Some(&Ty::Bool) } else { expected }, env);
                if op == "!" {
                    Ty::Bool
                } else {
                    if !t.numeric() {
                        self.error(e.span, "Arithmetic unary operator requires an integer");
                    }
                    t
                }
            }
            ExprKind::Binary(op, l, r) => {
                let boolean = op == "&&" || op == "||";
                let compare = ["==", "!=", "<", ">", "<=", ">="].contains(&op.as_str());
                let hint = if boolean {
                    Some(Ty::Bool)
                } else {
                    self.hint(l, env).or_else(|| self.hint(r, env)).or_else(|| {
                        if compare {
                            None
                        } else {
                            expected.cloned()
                        }
                    })
                };
                let lt = self.expression(l, hint.as_ref(), env);
                let rt = self.expression(r, Some(&lt), env);
                if lt != rt {
                    self.error(
                        e.span,
                        format!(
                            "Operands have different widths/types: {lt} and {rt}; cast explicitly"
                        ),
                    );
                }
                if !compare && !boolean && !lt.numeric() {
                    self.error(e.span, "Arithmetic requires integers");
                }
                if op == "/" || op == "%" {
                    match eval_const(r, &self.constants) {
                        Some(0) => self.error(r.span, "Division by zero"),
                        None => {
                            self.error(r.span, "Division requires a nonzero compile-time divisor")
                        }
                        _ => {}
                    }
                }
                if boolean || compare {
                    Ty::Bool
                } else {
                    lt
                }
            }
            ExprKind::Asm(inputs, _) => {
                let mut seen = BTreeSet::new();
                for (r, x) in inputs {
                    if !seen.insert(r.clone()) {
                        self.error(x.span, "Duplicate assembly register input");
                    }
                    self.expression(x, Some(&Ty::U8), env);
                }
                self.bounds.clear();
                Ty::U8
            }
        };
        if let Some(expected) = expected {
            if *expected != t && *expected != Ty::Void {
                self.error(
                    e.span,
                    format!("Expected {expected}, found {t}; use an explicit cast"),
                );
            }
        }
        e.ty = t.clone();
        t
    }
    fn range(&self, e: &Expr) -> (i64, i64) {
        if let Some(n) = eval_const(e, &self.constants) {
            return (n, n);
        }
        let full = || {
            if e.ty.signed() {
                if e.ty.wide() {
                    (-32768, 32767)
                } else {
                    (-128, 127)
                }
            } else {
                (0, e.ty.mask())
            }
        };
        let fit = |lo: i64, hi: i64| {
            let (a, b) = full();
            if lo >= a && hi <= b {
                (lo, hi)
            } else {
                (a, b)
            }
        };
        match &e.kind {
            ExprKind::Name(n) => {
                if self.shared.contains(n) {
                    full()
                } else {
                    self.bounds.get(n).copied().unwrap_or_else(full)
                }
            }
            ExprKind::Call(n, v)
                if ["u8", "i8", "u16", "i16"].contains(&n.as_str()) && v.len() == 1 =>
            {
                let (a, b) = self.range(&v[0]);
                fit(a, b)
            }
            ExprKind::Unary(op, x) if op == "-" => {
                let (a, b) = self.range(x);
                fit(-b, -a)
            }
            ExprKind::Binary(op, l, r) => {
                let (a, b) = self.range(l);
                let (c, d) = self.range(r);
                match op.as_str() {
                    "+" => fit(a + c, b + d),
                    "-" => fit(a - d, b - c),
                    "*" if a >= 0 && c >= 0 => fit(a * c, b * d),
                    "&" if a >= 0 && c >= 0 => (0, b.min(d)),
                    "|" | "^" if a >= 0 && c >= 0 => {
                        let max = b.max(d) as u64;
                        let cover = (max + 1).next_power_of_two() - 1;
                        fit(0, cover as i64)
                    }
                    "<<" if c == d && (0..16).contains(&c) && a >= 0 => fit(a << c, b << c),
                    ">>" if c == d && (0..16).contains(&c) => fit(a >> c, b >> c),
                    "%" if c == d && c > 0 && a >= 0 => (0, c - 1),
                    "/" if c == d && c > 0 && a >= 0 => fit(a / c, b / c),
                    _ => full(),
                }
            }
            _ => full(),
        }
    }
    fn refine(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Binary(op, l, r) if op == "&&" => {
                self.refine(l);
                let mut calls = BTreeSet::new();
                let mut asm = false;
                crate::optimizer::calls_expr(r, &mut calls, &mut asm);
                if asm {
                    self.bounds.clear();
                } else if calls.iter().any(|n| {
                    self.functions.contains_key(n) && !builtins().iter().any(|b| b.name == n)
                }) {
                    for n in self.globals.keys() {
                        self.bounds.remove(n);
                    }
                }
                self.refine(r);
            }
            ExprKind::Binary(op, l, r) => {
                if let (ExprKind::Name(n), Some(v)) = (&l.kind, eval_const(r, &self.constants)) {
                    let range = self.bounds.entry(n.clone()).or_insert(if l.ty.signed() {
                        (-32768, 32767)
                    } else {
                        (0, l.ty.mask())
                    });
                    match op.as_str() {
                        "<" => range.1 = range.1.min(v - 1),
                        "<=" => range.1 = range.1.min(v),
                        ">" => range.0 = range.0.max(v + 1),
                        ">=" => range.0 = range.0.max(v),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    fn local(
        &mut self,
        name: &str,
        t: Ty,
        mutable: bool,
        span: Span,
        env: &mut BTreeMap<String, (Ty, bool)>,
    ) {
        if name.starts_with("__") || builtins().iter().any(|b| b.name == name) {
            self.error(span, "Local name is reserved");
        }
        if self.all_locals.contains_key(name) || self.globals.contains_key(name) {
            self.error(
                span,
                format!("'{name}' already exists; local shadowing is deliberately disallowed"),
            );
        }
        env.insert(name.into(), (t.clone(), mutable));
        self.all_locals.insert(name.into(), t.clone());
        self.symbols.push(Symbol {
            name: name.into(),
            ty: t,
            span,
            scope: self.scope,
            kind: 13,
            mutable,
        });
    }
    fn block(&mut self, body: &mut [Stmt], env: &mut BTreeMap<String, (Ty, bool)>) {
        for s in body {
            match s {
                Stmt::Local {
                    name,
                    ty,
                    value,
                    mutable,
                    span,
                } => {
                    let t = self.expression(value, ty.as_ref(), env);
                    if !t.numeric() && t != Ty::Bool {
                        self.error(*span, "Local bindings require scalar values");
                    }
                    *ty = Some(t.clone());
                    let bounds = self.range(value);
                    self.local(name, t, *mutable, *span, env);
                    self.bounds.insert(name.clone(), bounds);
                }
                Stmt::Assign { target, value, .. } => {
                    let t = self.expression(target, None, env);
                    let n = match &target.kind {
                        ExprKind::Name(n) => Some(n),
                        ExprKind::Index(b, _, _) => {
                            if let ExprKind::Name(n) = &b.kind {
                                Some(n)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some(n) = n {
                        if !env.get(n).or(self.globals.get(n)).is_some_and(|v| v.1) {
                            self.error(target.span, "Cannot assign to immutable storage");
                        }
                        self.bounds.remove(n);
                    } else {
                        self.error(
                            target.span,
                            "Assignment requires a variable or array element",
                        );
                    }
                    self.expression(value, Some(&t), env);
                }
                Stmt::Expr(e) => {
                    self.expression(e, None, env);
                }
                Stmt::If { condition, yes, no } => {
                    self.expression(condition, Some(&Ty::Bool), env);
                    let old = self.bounds.clone();
                    self.refine(condition);
                    self.block(yes, &mut env.clone());
                    let yes_bounds = self.bounds.clone();
                    self.bounds = old;
                    self.block(no, &mut env.clone());
                    let no_bounds = self.bounds.clone();
                    self.bounds = yes_bounds
                        .into_iter()
                        .filter_map(|(n, (lo, hi))| {
                            no_bounds
                                .get(&n)
                                .map(|(a, b)| (n.clone(), (lo.min(*a), hi.max(*b))))
                        })
                        .collect();
                }
                Stmt::While { condition, body } => {
                    let old = self.bounds.clone();
                    let mut changed = assigned_names(body);
                    changed.extend(self.globals.keys().cloned());
                    for n in &changed {
                        self.bounds.remove(n);
                    }
                    self.expression(condition, Some(&Ty::Bool), env);
                    self.refine(condition);
                    self.loops += 1;
                    self.block(body, &mut env.clone());
                    self.loops -= 1;
                    self.bounds = old
                        .into_iter()
                        .filter(|(n, _)| !changed.contains(n))
                        .collect();
                }
                Stmt::For {
                    name,
                    start,
                    end,
                    body,
                    span,
                } => {
                    let end_hint = eval_const(end, &self.constants).unwrap_or(255);
                    let t = if end_hint > 255 { Ty::U16 } else { Ty::U8 };
                    self.expression(start, Some(&t), env);
                    self.expression(end, Some(&t), env);
                    self.local(name, t, false, *span, env);
                    let old = self.bounds.clone();
                    let mut changed = assigned_names(body);
                    changed.extend(self.globals.keys().cloned());
                    for n in &changed {
                        self.bounds.remove(n);
                    }
                    if let (Some(lo), Some(hi)) = (
                        eval_const(start, &self.constants),
                        eval_const(end, &self.constants),
                    ) {
                        self.bounds.insert(name.clone(), (lo, hi - 1));
                    }
                    self.loops += 1;
                    self.block(body, &mut env.clone());
                    self.loops -= 1;
                    self.bounds = old
                        .into_iter()
                        .filter(|(n, _)| !changed.contains(n))
                        .collect();
                    env.remove(name);
                }
                Stmt::Loop(body) => {
                    let old = self.bounds.clone();
                    let mut changed = assigned_names(body);
                    changed.extend(self.globals.keys().cloned());
                    for n in &changed {
                        self.bounds.remove(n);
                    }
                    self.loops += 1;
                    self.block(body, &mut env.clone());
                    self.loops -= 1;
                    self.bounds = old
                        .into_iter()
                        .filter(|(n, _)| !changed.contains(n))
                        .collect();
                }
                Stmt::Break(s) | Stmt::Continue(s) => {
                    if self.loops == 0 {
                        self.error(*s, "Loop control outside a loop");
                    }
                }
                Stmt::Return(e, s) => {
                    if let Some(e) = e {
                        let result = self.result.clone();
                        self.expression(e, Some(&result), env);
                        if result == Ty::Void {
                            self.error(*s, "Void function cannot return a value");
                        }
                    } else if self.result != Ty::Void {
                        self.error(*s, "Return requires a value");
                    }
                }
            }
        }
    }
}
pub fn eval_const(e: &Expr, constants: &BTreeMap<String, i64>) -> Option<i64> {
    let n = match &e.kind {
        ExprKind::Number(n) => *n,
        ExprKind::Name(n) => *constants.get(n)?,
        ExprKind::Unary(op, x) => {
            let n = eval_const(x, constants)?;
            match op.as_str() {
                "-" => n.wrapping_neg(),
                "+" => n,
                "~" => !n,
                "!" => i64::from(n == 0),
                _ => return None,
            }
        }
        ExprKind::Binary(op, l, r) => {
            let a = eval_const(l, constants)?;
            if op == "&&" && a == 0 {
                return Some(0);
            }
            if op == "||" && a != 0 {
                return Some(1);
            }
            let b = eval_const(r, constants)?;
            match op.as_str() {
                "+" => a.wrapping_add(b),
                "-" => a.wrapping_sub(b),
                "*" => a.wrapping_mul(b),
                "/" => {
                    if b == 0 {
                        return None;
                    } else {
                        a.wrapping_div(b)
                    }
                }
                "%" => {
                    if b == 0 {
                        return None;
                    } else {
                        a.wrapping_rem(b)
                    }
                }
                "&" => a & b,
                "|" => a | b,
                "^" => a ^ b,
                "<<" => {
                    if b < 0 || b >= if l.ty.wide() { 16 } else { 8 } {
                        0
                    } else {
                        a.wrapping_shl(b as u32)
                    }
                }
                ">>" => {
                    if b < 0 || b >= if l.ty.wide() { 16 } else { 8 } {
                        if a < 0 {
                            -1
                        } else {
                            0
                        }
                    } else {
                        a >> b
                    }
                }
                "==" => i64::from(a == b),
                "!=" => i64::from(a != b),
                "<" => i64::from(a < b),
                ">" => i64::from(a > b),
                "<=" => i64::from(a <= b),
                ">=" => i64::from(a >= b),
                "&&" => i64::from(a != 0 && b != 0),
                "||" => i64::from(a != 0 || b != 0),
                _ => return None,
            }
        }
        ExprKind::Call(n, args) if ["u8", "i8", "u16", "i16", "bool"].contains(&n.as_str()) => {
            let x = eval_const(args.first()?, constants)?;
            if n == "bool" {
                i64::from(x != 0)
            } else {
                x
            }
        }
        _ => return None,
    };
    Some(if e.ty == Ty::Bool {
        i64::from(n != 0)
    } else if e.ty.numeric() {
        e.ty.wrap(n)
    } else {
        n
    })
}
pub fn terminates(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Return(..) => true,
        Stmt::If { yes, no, .. } => terminates(yes) && terminates(no),
        _ => false,
    })
}

fn assigned_names(body: &[Stmt]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in body {
        match s {
            Stmt::Assign {
                target:
                    Expr {
                        kind: ExprKind::Name(n),
                        ..
                    },
                ..
            } => {
                out.insert(n.clone());
            }
            Stmt::If { yes, no, .. } => {
                out.extend(assigned_names(yes));
                out.extend(assigned_names(no));
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Loop(body) => {
                out.extend(assigned_names(body))
            }
            _ => {}
        }
    }
    out
}

// Retain declarations and completed statements while a user types an unfinished one.
// Recovery never suppresses the original diagnostic and therefore cannot emit a ROM.
fn recover_program(source: &str) -> Option<Program> {
    let tokens = lex(source).ok()?;
    for end in tokens
        .iter()
        .rev()
        .filter(|t| [";", "{", "}"].contains(&t.text.as_str()))
        .take(128)
        .map(|t| t.span.end)
    {
        let prefix = &source[..end];
        let mut stack = Vec::new();
        for t in tokens.iter().take_while(|t| t.span.end <= end) {
            match t.text.as_str() {
                "{" => stack.push("}"),
                "(" => stack.push(")"),
                "[" => stack.push("]"),
                "}" | ")" | "]" => {
                    stack.pop();
                }
                _ => {}
            }
        }
        let mut repaired = prefix.to_owned();
        for close in stack.iter().rev() {
            repaired.push_str(close);
        }
        if let Ok(mut p) = parse(&repaired) {
            if let Some(f) = p.functions.last_mut() {
                if f.span.end >= end {
                    f.span.end = source.len();
                }
            }
            return Some(p);
        }
    }
    None
}
