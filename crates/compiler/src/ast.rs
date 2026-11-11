//! Syntax tree produced by the parser.
use crate::source::{FileId, Span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Path {
    pub segments: Vec<Ident>,
    pub span: Span,
}

impl Path {
    pub fn last(&self) -> &Ident {
        self.segments.last().unwrap()
    }
    pub fn is_single(&self) -> bool {
        self.segments.len() == 1
    }
    pub fn text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::")
    }
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub name: Ident,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeExpr {
    Path(Path),
    /// `[T; N]`; `None` length is `_`, inferred from the initializer.
    Array(Box<TypeExpr>, Option<Box<Expr>>, Span),
    /// `&[T]`: a read-only view of constant ROM data.
    Slice(Box<TypeExpr>, Span),
    /// `u8 in 0..128` or `u8 in 1..=6`.
    Range(Box<TypeExpr>, Box<Expr>, Box<Expr>, bool, Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Path(p) => p.span,
            TypeExpr::Array(_, _, s) | TypeExpr::Slice(_, s) | TypeExpr::Range(.., s) => *s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Module {
    pub file: FileId,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: ItemKind,
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub span: Span,
}

impl Item {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name.name == name)
    }
    pub fn name(&self) -> Option<&Ident> {
        match &self.kind {
            ItemKind::Const { name, .. }
            | ItemKind::Var { name, .. }
            | ItemKind::Struct { name, .. }
            | ItemKind::Enum { name, .. }
            | ItemKind::Type { name, .. } => Some(name),
            ItemKind::Fn(f) => Some(&f.name),
            ItemKind::Use(..) | ItemKind::Cartridge(..) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct UseTree {
    /// Module path, such as `std::video`.
    pub path: Path,
    /// `use a::{b, c as d}` imports; empty imports the module itself.
    pub names: Vec<(Ident, Option<Ident>)>,
    /// `use a::b as c`.
    pub alias: Option<Ident>,
    /// `use a::*`.
    pub glob: bool,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Use(UseTree),
    Cartridge(Vec<(Ident, Expr)>),
    Const {
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    Var {
        name: Ident,
        ty: TypeExpr,
        value: Option<Expr>,
    },
    Fn(FnDecl),
    Struct {
        name: Ident,
        fields: Vec<(Ident, TypeExpr)>,
    },
    Enum {
        name: Ident,
        repr: Option<TypeExpr>,
        variants: Vec<(Ident, Option<Expr>)>,
    },
    Type {
        name: Ident,
        ty: TypeExpr,
    },
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
    pub mutable: bool,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub result: Option<TypeExpr>,
    pub body: Block,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Let {
        name: Ident,
        mutable: bool,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    Assign {
        target: Expr,
        op: &'static str,
        value: Expr,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then: Block,
        otherwise: Option<Block>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    For {
        var: Ident,
        start: Expr,
        end: Expr,
        inclusive: bool,
        reverse: bool,
        body: Block,
    },
    Loop(Block),
    Break,
    Continue,
    Return(Option<Expr>),
    Block(Block),
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Wildcard(Span),
    /// A constant expression: literal, `const`, or enum variant path.
    Value(Expr),
    Range(Expr, Expr, bool),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(s) => *s,
            Pattern::Value(e) => e.span,
            Pattern::Range(a, b, _) => a.span.to(b.span),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ArmBody {
    Expr(Expr),
    Block(Block),
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: ArmBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct AsmInput {
    pub register: Ident,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Bool(bool),
    Str(Vec<u8>),
    Char(u8),
    Path(Path),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>, bool),
    Field(Box<Expr>, Ident),
    Cast(Box<Expr>, TypeExpr),
    StructLit(Path, Vec<(Ident, Expr)>),
    Array(Vec<Expr>),
    Repeat(Box<Expr>, Box<Expr>),
    Match(Box<Expr>, Vec<Arm>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Asm(Vec<AsmInput>, String, Span),
    /// `a..b` in argument position, used by `asset("f", a..b)`.
    Range(Box<Expr>, Box<Expr>, bool),
    Error,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}
