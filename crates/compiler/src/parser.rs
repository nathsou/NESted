//! Recursive-descent parser with statement- and item-level error recovery.
use crate::ast::*;
use crate::diag::{Diagnostic, Diagnostics};
use crate::lexer::{Tok, Token};
use crate::source::{FileId, Span};

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    #[allow(dead_code)]
    file: FileId,
    no_struct: bool,
    pub diags: Diagnostics,
}

type PResult<T> = Result<T, Diagnostic>;

const BINARY: &[(&str, u8)] = &[
    ("||", 1),
    ("&&", 2),
    ("==", 3),
    ("!=", 3),
    ("<", 3),
    ("<=", 3),
    (">", 3),
    (">=", 3),
    ("|", 4),
    ("^", 5),
    ("&", 6),
    ("<<", 7),
    (">>", 7),
    ("+", 8),
    ("-", 8),
    ("*", 9),
    ("/", 9),
    ("%", 9),
];

const ASSIGN: &[&str] = &[
    "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
];

pub fn parse_module(file: FileId, tokens: &[Token]) -> (Module, Diagnostics) {
    let mut p = Parser {
        tokens,
        pos: 0,
        file,
        no_struct: false,
        diags: Diagnostics::default(),
    };
    let mut items = Vec::new();
    while !p.at_eof() {
        let start = p.pos;
        match p.item() {
            Ok(item) => items.push(item),
            Err(d) => {
                p.diags.push(d);
                p.recover_item(start);
            }
        }
    }
    (Module { file, items }, p.diags)
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.tokens[(self.pos + n).min(self.tokens.len() - 1)].tok
    }
    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }
    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn bump(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == k)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn error<T>(&self, message: impl Into<String>) -> PResult<T> {
        Err(Diagnostic::error(self.span(), message))
    }
    fn describe(&self) -> String {
        match self.peek() {
            Tok::Eof => "end of file".into(),
            _ => format!("'{}'", self.tokens[self.pos].text),
        }
    }
    fn expect(&mut self, p: &str) -> PResult<Span> {
        if self.is(p) {
            Ok(self.bump().span)
        } else {
            self.error(format!("Expected '{p}', found {}", self.describe()))
        }
    }
    fn expect_kw(&mut self, k: &str) -> PResult<Span> {
        if self.is_kw(k) {
            Ok(self.bump().span)
        } else {
            self.error(format!("Expected '{k}', found {}", self.describe()))
        }
    }
    fn ident(&mut self) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) if !crate::lexer::KEYWORDS.contains(&name.as_str()) => {
                let span = self.bump().span;
                Ok(Ident { name, span })
            }
            _ => self.error(format!("Expected a name, found {}", self.describe())),
        }
    }
    fn recover_item(&mut self, start: usize) {
        if self.pos == start {
            self.bump();
        }
        let mut depth = 0i32;
        while !self.at_eof() {
            if depth == 0 {
                if let Tok::Ident(w) = self.peek() {
                    if matches!(
                        w.as_str(),
                        "fn" | "const" | "var" | "struct" | "enum" | "type" | "use" | "pub" | "cartridge"
                    ) {
                        return;
                    }
                }
                if self.is("@") {
                    return;
                }
            }
            if self.is("{") {
                depth += 1;
            } else if self.is("}") {
                depth -= 1;
                if depth <= 0 {
                    self.bump();
                    if depth < 0 {
                        depth = 0;
                    }
                    continue;
                }
            }
            self.bump();
        }
    }

    // ------------------------------------------------------------------ items

    fn attrs(&mut self) -> PResult<Vec<Attr>> {
        let mut attrs = Vec::new();
        while self.is("@") {
            let start = self.bump().span;
            let name = self.ident()?;
            let mut args = Vec::new();
            if self.eat("(") {
                while !self.is(")") {
                    args.push(self.expr()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
            }
            attrs.push(Attr {
                name,
                args,
                span: start.to(self.prev_span()),
            });
        }
        Ok(attrs)
    }

    fn item(&mut self) -> PResult<Item> {
        let start = self.span();
        let attrs = self.attrs()?;
        let public = self.eat_kw("pub");
        let kind = match self.peek() {
            Tok::Ident(w) => match w.as_str() {
                "use" => {
                    self.bump();
                    let tree = self.use_tree()?;
                    self.expect(";")?;
                    ItemKind::Use(tree)
                }
                "cartridge" => {
                    self.bump();
                    self.expect("{")?;
                    let mut fields = Vec::new();
                    while !self.is("}") && !self.at_eof() {
                        let name = self.ident()?;
                        if !self.eat("=") {
                            self.expect(":")?;
                        }
                        let value = self.expr()?;
                        fields.push((name, value));
                        if !self.eat(";") && !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                    ItemKind::Cartridge(fields)
                }
                "const" => {
                    self.bump();
                    let name = self.ident()?;
                    let ty = if self.eat(":") {
                        Some(self.type_expr()?)
                    } else {
                        None
                    };
                    self.expect("=")?;
                    let value = self.expr()?;
                    self.expect(";")?;
                    ItemKind::Const { name, ty, value }
                }
                "var" => {
                    self.bump();
                    let name = self.ident()?;
                    self.expect(":")?;
                    let ty = self.type_expr()?;
                    let value = if self.eat("=") {
                        Some(self.expr()?)
                    } else {
                        None
                    };
                    self.expect(";")?;
                    ItemKind::Var { name, ty, value }
                }
                "fn" => {
                    self.bump();
                    ItemKind::Fn(self.function()?)
                }
                "struct" => {
                    self.bump();
                    let name = self.ident()?;
                    self.expect("{")?;
                    let mut fields = Vec::new();
                    while !self.is("}") && !self.at_eof() {
                        let field = self.ident()?;
                        self.expect(":")?;
                        fields.push((field, self.type_expr()?));
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                    ItemKind::Struct { name, fields }
                }
                "enum" => {
                    self.bump();
                    let name = self.ident()?;
                    let repr = if self.eat(":") {
                        Some(self.type_expr()?)
                    } else {
                        None
                    };
                    self.expect("{")?;
                    let mut variants = Vec::new();
                    while !self.is("}") && !self.at_eof() {
                        let v = self.ident()?;
                        let value = if self.eat("=") {
                            Some(self.expr()?)
                        } else {
                            None
                        };
                        variants.push((v, value));
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                    ItemKind::Enum {
                        name,
                        repr,
                        variants,
                    }
                }
                "type" => {
                    self.bump();
                    let name = self.ident()?;
                    self.expect("=")?;
                    let ty = self.type_expr()?;
                    self.expect(";")?;
                    ItemKind::Type { name, ty }
                }
                _ => return self.error(format!("Expected an item, found {}", self.describe())),
            },
            _ => return self.error(format!("Expected an item, found {}", self.describe())),
        };
        Ok(Item {
            kind,
            attrs,
            public,
            span: start.to(self.prev_span()),
        })
    }

    fn use_tree(&mut self) -> PResult<UseTree> {
        let first = self.ident()?;
        let mut segments = vec![first];
        let mut names = Vec::new();
        let mut glob = false;
        while self.eat("::") {
            if self.eat("{") {
                while !self.is("}") {
                    let n = self.ident()?;
                    let alias = if self.eat_kw("as") {
                        Some(self.ident()?)
                    } else {
                        None
                    };
                    names.push((n, alias));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("}")?;
                break;
            }
            if self.eat("*") {
                glob = true;
                break;
            }
            segments.push(self.ident()?);
        }
        let alias = if self.eat_kw("as") {
            Some(self.ident()?)
        } else {
            None
        };
        let span = segments[0].span.to(segments.last().unwrap().span);
        Ok(UseTree {
            path: Path { segments, span },
            names,
            alias,
            glob,
        })
    }

    fn function(&mut self) -> PResult<FnDecl> {
        let name = self.ident()?;
        self.expect("(")?;
        let mut params = Vec::new();
        while !self.is(")") {
            let mutable = self.eat_kw("var");
            let pname = self.ident()?;
            self.expect(":")?;
            let ty = self.type_expr()?;
            params.push(Param {
                name: pname,
                ty,
                mutable,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let result = if self.eat("->") {
            Some(self.type_expr()?)
        } else {
            None
        };
        let body = self.block()?;
        Ok(FnDecl {
            name,
            params,
            result,
            body,
        })
    }

    fn path(&mut self) -> PResult<Path> {
        let first = self.ident()?;
        let mut segments = vec![first];
        while self.is("::") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            segments.push(self.ident()?);
        }
        let span = segments[0].span.to(segments.last().unwrap().span);
        Ok(Path { segments, span })
    }

    pub fn type_expr(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        let base = if self.eat("&") {
            self.expect("[")?;
            let elem = self.type_expr()?;
            self.expect("]")?;
            TypeExpr::Slice(Box::new(elem), start.to(self.prev_span()))
        } else if self.eat("[") {
            let elem = self.type_expr()?;
            self.expect(";")?;
            let len = if self.eat("_") {
                None
            } else {
                Some(Box::new(self.expr()?))
            };
            self.expect("]")?;
            TypeExpr::Array(Box::new(elem), len, start.to(self.prev_span()))
        } else {
            TypeExpr::Path(self.path()?)
        };
        if self.eat_kw("in") {
            let lo = self.binary(4)?;
            let inclusive = if self.eat("..=") {
                true
            } else {
                self.expect("..")?;
                false
            };
            let hi = self.binary(4)?;
            return Ok(TypeExpr::Range(
                Box::new(base),
                Box::new(lo),
                Box::new(hi),
                inclusive,
                start.to(self.prev_span()),
            ));
        }
        Ok(base)
    }

    // ------------------------------------------------------------- statements

    pub fn block(&mut self) -> PResult<Block> {
        let start = self.expect("{")?;
        let mut stmts = Vec::new();
        while !self.is("}") && !self.at_eof() {
            let before = self.pos;
            match self.stmt() {
                Ok(s) => stmts.push(s),
                Err(d) => {
                    self.diags.push(d);
                    self.recover_stmt(before);
                }
            }
        }
        let end = self.expect("}")?;
        Ok(Block {
            stmts,
            span: start.to(end),
        })
    }

    fn recover_stmt(&mut self, before: usize) {
        if self.pos == before {
            self.bump();
        }
        let mut depth = 0i32;
        while !self.at_eof() {
            if self.is("{") {
                depth += 1;
            } else if self.is("}") {
                if depth == 0 {
                    return;
                }
                depth -= 1;
                if depth == 0 {
                    self.bump();
                    return;
                }
            } else if self.is(";") && depth == 0 {
                self.bump();
                return;
            }
            self.bump();
        }
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let kind = if self.is_kw("let") || self.is_kw("var") {
            let mutable = self.bump().text == "var";
            let name = self.ident()?;
            let ty = if self.eat(":") {
                Some(self.type_expr()?)
            } else {
                None
            };
            self.expect("=")?;
            let value = self.expr()?;
            self.expect(";")?;
            StmtKind::Let {
                name,
                mutable,
                ty,
                value,
            }
        } else if self.eat_kw("if") {
            self.if_stmt()?
        } else if self.eat_kw("while") {
            let cond = self.expr_no_struct()?;
            let body = self.block()?;
            StmtKind::While { cond, body }
        } else if self.eat_kw("for") {
            let var = self.ident()?;
            self.expect_kw("in")?;
            let paren = self.eat("(");
            let start_e = self.expr_no_struct_prec(4)?;
            let inclusive = if self.eat("..=") {
                true
            } else {
                self.expect("..")?;
                false
            };
            let end = self.expr_no_struct_prec(4)?;
            let mut reverse = false;
            if paren {
                self.expect(")")?;
                self.expect(".")?;
                let m = self.ident()?;
                if m.name != "rev" {
                    return Err(Diagnostic::error(m.span, "Expected '.rev()'"));
                }
                self.expect("(")?;
                self.expect(")")?;
                reverse = true;
            }
            let body = self.block()?;
            StmtKind::For {
                var,
                start: start_e,
                end,
                inclusive,
                reverse,
                body,
            }
        } else if self.eat_kw("loop") {
            StmtKind::Loop(self.block()?)
        } else if self.eat_kw("break") {
            self.expect(";")?;
            StmtKind::Break
        } else if self.eat_kw("continue") {
            self.expect(";")?;
            StmtKind::Continue
        } else if self.eat_kw("return") {
            let value = if self.is(";") {
                None
            } else {
                Some(self.expr()?)
            };
            self.expect(";")?;
            StmtKind::Return(value)
        } else if self.is("{") {
            StmtKind::Block(self.block()?)
        } else {
            let e = self.expr()?;
            if let Some(op) = ASSIGN.iter().find(|op| self.is(op)) {
                self.bump();
                let value = self.expr()?;
                self.expect(";")?;
                StmtKind::Assign {
                    target: e,
                    op,
                    value,
                }
            } else {
                let block_like = matches!(e.kind, ExprKind::Match(..))
                    || matches!(e.kind, ExprKind::Asm(..));
                if !(block_like && !self.is(";")) {
                    self.expect(";")?;
                }
                StmtKind::Expr(e)
            }
        };
        Ok(Stmt {
            kind,
            span: start.to(self.prev_span()),
        })
    }

    fn if_stmt(&mut self) -> PResult<StmtKind> {
        let cond = self.expr_no_struct()?;
        let then = self.block()?;
        let otherwise = if self.eat_kw("else") {
            if self.is_kw("if") {
                let start = self.bump().span;
                let kind = self.if_stmt()?;
                let span = start.to(self.prev_span());
                Some(Block {
                    stmts: vec![Stmt { kind, span }],
                    span,
                })
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        Ok(StmtKind::If {
            cond,
            then,
            otherwise,
        })
    }

    // ------------------------------------------------------------ expressions

    pub fn expr(&mut self) -> PResult<Expr> {
        self.binary(1)
    }

    fn expr_no_struct(&mut self) -> PResult<Expr> {
        self.expr_no_struct_prec(1)
    }

    fn expr_no_struct_prec(&mut self, prec: u8) -> PResult<Expr> {
        let old = std::mem::replace(&mut self.no_struct, true);
        let r = self.binary(prec);
        self.no_struct = old;
        r
    }

    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let Some((op, prec)) = BINARY
                .iter()
                .find(|(op, _)| self.is(op))
                .map(|(op, p)| (*op, *p))
            else {
                break;
            };
            if prec < min {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            if prec == 3 && !self.parenthesized(lhs.span) {
                if let ExprKind::Binary(o, ..) = &lhs.kind {
                    if BINARY.iter().any(|(b, p)| b == o && *p == 3) {
                        return Err(Diagnostic::error(
                            span,
                            "Comparisons cannot be chained; use parentheses",
                        ));
                    }
                }
            }
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
                span,
            };
        }
        Ok(lhs)
    }

    /// The expression at `span` is wrapped in parentheses.
    fn parenthesized(&self, span: Span) -> bool {
        self.tokens
            .binary_search_by_key(&span.start, |t| t.span.start)
            .ok()
            .is_some_and(|i| matches!(self.tokens[i].tok, Tok::Punct("(")))
    }

    fn unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        for op in ["-", "!", "~"] {
            if self.eat(op) {
                let e = self.unary()?;
                // Fold negative literals so `-128` fits i8.
                if op == "-" {
                    if let ExprKind::Int(n) = e.kind {
                        return Ok(Expr {
                            kind: ExprKind::Int(-n),
                            span: start.to(e.span),
                        });
                    }
                }
                let span = start.to(e.span);
                return Ok(Expr {
                    kind: ExprKind::Unary(op, Box::new(e)),
                    span,
                });
            }
        }
        let mut e = self.postfix()?;
        while self.eat_kw("as") {
            let ty = self.type_expr_no_range()?;
            let span = e.span.to(ty.span());
            e = Expr {
                kind: ExprKind::Cast(Box::new(e), ty),
                span,
            };
        }
        Ok(e)
    }

    fn type_expr_no_range(&mut self) -> PResult<TypeExpr> {
        let start = self.span();
        if self.eat("[") {
            let elem = self.type_expr()?;
            self.expect(";")?;
            let len = Some(Box::new(self.expr()?));
            self.expect("]")?;
            return Ok(TypeExpr::Array(Box::new(elem), len, start.to(self.prev_span())));
        }
        Ok(TypeExpr::Path(self.path()?))
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat("(") {
                let mut args = Vec::new();
                while !self.is(")") {
                    let a = self.expr()?;
                    if self.is("..") || self.is("..=") {
                        let inclusive = self.bump().text == "..=";
                        let b = self.expr()?;
                        let span = a.span.to(b.span);
                        args.push(Expr {
                            kind: ExprKind::Range(Box::new(a), Box::new(b), inclusive),
                            span,
                        });
                    } else {
                        args.push(a);
                    }
                    if !self.eat(",") {
                        break;
                    }
                }
                let end = self.expect(")")?;
                let span = e.span.to(end);
                e = Expr {
                    kind: ExprKind::Call(Box::new(e), args),
                    span,
                };
            } else if self.eat("[") {
                let raw = self.eat_kw("raw");
                let index = self.expr()?;
                let end = self.expect("]")?;
                let span = e.span.to(end);
                e = Expr {
                    kind: ExprKind::Index(Box::new(e), Box::new(index), raw),
                    span,
                };
            } else if self.is(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                self.bump();
                let field = self.ident()?;
                let span = e.span.to(field.span);
                e = Expr {
                    kind: ExprKind::Field(Box::new(e), field),
                    span,
                };
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let tok = self.peek().clone();
        let simple = |kind| {
            Ok(Expr {
                kind,
                span: start,
            })
        };
        match tok {
            Tok::Int(n) => {
                self.bump();
                simple(ExprKind::Int(n))
            }
            Tok::Str(s) => {
                self.bump();
                simple(ExprKind::Str(s))
            }
            Tok::Char(c) => {
                self.bump();
                simple(ExprKind::Char(c))
            }
            Tok::Punct("(") => {
                self.bump();
                let old = std::mem::replace(&mut self.no_struct, false);
                let e = self.expr();
                self.no_struct = old;
                let mut e = e?;
                let end = self.expect(")")?;
                e.span = start.to(end);
                Ok(e)
            }
            Tok::Punct("[") => {
                self.bump();
                let mut items = Vec::new();
                if self.eat("]") {
                    return Ok(Expr {
                        kind: ExprKind::Array(items),
                        span: start.to(self.prev_span()),
                    });
                }
                let first = self.expr()?;
                if self.eat(";") {
                    let count = self.expr()?;
                    let end = self.expect("]")?;
                    return Ok(Expr {
                        kind: ExprKind::Repeat(Box::new(first), Box::new(count)),
                        span: start.to(end),
                    });
                }
                items.push(first);
                while self.eat(",") {
                    if self.is("]") {
                        break;
                    }
                    items.push(self.expr()?);
                }
                let end = self.expect("]")?;
                Ok(Expr {
                    kind: ExprKind::Array(items),
                    span: start.to(end),
                })
            }
            Tok::Ident(w) => match w.as_str() {
                "true" | "false" => {
                    self.bump();
                    simple(ExprKind::Bool(w == "true"))
                }
                "match" => {
                    self.bump();
                    self.match_expr(start)
                }
                "if" => {
                    self.bump();
                    let cond = self.expr_no_struct()?;
                    self.expect("{")?;
                    let a = self.expr()?;
                    self.expect("}")?;
                    self.expect_kw("else")?;
                    let b = if self.is_kw("if") {
                        self.primary()?
                    } else {
                        self.expect("{")?;
                        let b = self.expr()?;
                        self.expect("}")?;
                        b
                    };
                    Ok(Expr {
                        kind: ExprKind::If(Box::new(cond), Box::new(a), Box::new(b)),
                        span: start.to(self.prev_span()),
                    })
                }
                "asm" => {
                    self.bump();
                    let mut inputs = Vec::new();
                    if self.eat("(") {
                        while !self.is(")") {
                            let register = self.ident()?;
                            self.expect("=")?;
                            let value = self.expr()?;
                            inputs.push(AsmInput { register, value });
                            if !self.eat(",") {
                                break;
                            }
                        }
                        self.expect(")")?;
                    }
                    match self.peek().clone() {
                        Tok::Asm(body) => {
                            let span = self.bump().span;
                            Ok(Expr {
                                kind: ExprKind::Asm(inputs, body, span),
                                span: start.to(span),
                            })
                        }
                        _ => self.error("Expected an asm body '{ ... }'"),
                    }
                }
                _ => {
                    let path = self.path()?;
                    if self.is("{") && !self.no_struct && starts_upper(&path) {
                        return self.struct_lit(path);
                    }
                    let span = path.span;
                    Ok(Expr {
                        kind: ExprKind::Path(path),
                        span,
                    })
                }
            },
            _ => self.error(format!("Expected an expression, found {}", self.describe())),
        }
    }

    fn struct_lit(&mut self, path: Path) -> PResult<Expr> {
        self.expect("{")?;
        let mut fields = Vec::new();
        while !self.is("}") {
            let name = self.ident()?;
            let value = if self.eat(":") {
                self.expr()?
            } else {
                Expr {
                    kind: ExprKind::Path(Path {
                        segments: vec![name.clone()],
                        span: name.span,
                    }),
                    span: name.span,
                }
            };
            fields.push((name, value));
            if !self.eat(",") {
                break;
            }
        }
        let end = self.expect("}")?;
        let span = path.span.to(end);
        Ok(Expr {
            kind: ExprKind::StructLit(path, fields),
            span,
        })
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        if self.is("_") {
            return Ok(Pattern::Wildcard(self.bump().span));
        }
        let a = self.binary(5)?;
        if self.is("..=") || self.is("..") {
            let inclusive = self.bump().text == "..=";
            let b = self.binary(5)?;
            return Ok(Pattern::Range(a, b, inclusive));
        }
        Ok(Pattern::Value(a))
    }

    fn match_expr(&mut self, start: Span) -> PResult<Expr> {
        let scrutinee = self.expr_no_struct()?;
        self.expect("{")?;
        let mut arms = Vec::new();
        while !self.is("}") && !self.at_eof() {
            let arm_start = self.span();
            let mut patterns = vec![self.pattern()?];
            while self.eat("|") {
                patterns.push(self.pattern()?);
            }
            let guard = if self.eat_kw("if") {
                Some(self.expr()?)
            } else {
                None
            };
            self.expect("=>")?;
            let body = if self.is("{") {
                ArmBody::Block(self.block()?)
            } else {
                ArmBody::Expr(self.expr()?)
            };
            let block = matches!(body, ArmBody::Block(_));
            arms.push(Arm {
                patterns,
                guard,
                body,
                span: arm_start.to(self.prev_span()),
            });
            if !self.eat(",") && !block {
                break;
            }
        }
        let end = self.expect("}")?;
        Ok(Expr {
            kind: ExprKind::Match(Box::new(scrutinee), arms),
            span: start.to(end),
        })
    }
}

fn starts_upper(p: &Path) -> bool {
    p.last()
        .name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
}

pub fn parse(file: FileId, src: &str) -> (Module, Vec<Token>, Vec<crate::lexer::Comment>, Diagnostics) {
    let mut diags = Diagnostics::default();
    let lexed = crate::lexer::lex(file, src, &mut diags);
    let (module, d) = parse_module(file, &lexed.tokens);
    diags.extend(d);
    (module, lexed.tokens, lexed.comments, diags)
}
