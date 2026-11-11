//! Typed, name-resolved program: the output of semantic analysis and the
//! input of compile-time evaluation and MIR construction.
use crate::source::Span;
use crate::types::{Type, TypeTable};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GlobalId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FnId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(pub u32);

/// Compile-time value. Scalars (integers, bools, enums, ranges) are `Int`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    /// Struct fields in declaration order, or array elements.
    Aggregate(Rc<Vec<Value>>),
    /// A read-only view of a constant global.
    Slice(GlobalId, u32, u32),
    Void,
}

impl Value {
    pub fn int(&self) -> i64 {
        match self {
            Value::Int(n) => *n,
            _ => 0,
        }
    }
    pub fn items(&self) -> &[Value] {
        match self {
            Value::Aggregate(v) => v,
            _ => &[],
        }
    }
    pub fn aggregate(v: Vec<Value>) -> Self {
        Value::Aggregate(Rc::new(v))
    }
    /// Little-endian encoding of a value of type `t` in declaration layout.
    pub fn encode(&self, types: &TypeTable, t: &Type, out: &mut Vec<u8>) {
        match t {
            Type::Array(e, _) => {
                for v in self.items() {
                    v.encode(types, e, out);
                }
            }
            Type::Struct(s) => {
                for (v, (_, ft)) in self.items().iter().zip(&types.structs[s.0 as usize].fields) {
                    v.encode(types, ft, out);
                }
            }
            _ => {
                let n = self.int();
                out.push(n as u8);
                if types.size(t) == 2 {
                    out.push((n >> 8) as u8);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Auto,
    ZeroPage,
    Ram,
    At(u16),
}

#[derive(Clone, Debug)]
pub enum GlobalKind {
    Const,
    Var,
    /// Memory-mapped hardware: every access is volatile.
    Io(u16),
}

#[derive(Clone, Debug)]
pub struct Global {
    pub name: String,
    /// Module-qualified name, such as `std::video::oam`.
    pub path: String,
    pub ty: Type,
    pub kind: GlobalKind,
    pub value: Value,
    pub placement: Placement,
    pub bank: Option<u32>,
    /// Accessed from more than one interrupt context.
    pub shared: bool,
    pub span: Span,
    pub module: usize,
    pub doc: String,
}

impl Global {
    pub fn is_const(&self) -> bool {
        matches!(self.kind, GlobalKind::Const)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Context {
    Main,
    Nmi,
    Irq,
}

#[derive(Clone, Debug, Default)]
pub struct FnAttrs {
    pub inline: bool,
    pub noinline: bool,
    pub interrupt: Option<Context>,
    pub reset: bool,
    pub recursive: Option<u32>,
    pub budget: Option<u32>,
    pub bank: Option<u32>,
    pub export: bool,
    pub cold: bool,
}

#[derive(Clone, Debug)]
pub struct LocalDecl {
    pub name: String,
    pub ty: Type,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub path: String,
    pub params: Vec<LocalId>,
    pub result: Type,
    pub locals: Vec<LocalDecl>,
    pub body: Vec<Stmt>,
    pub attrs: FnAttrs,
    pub span: Span,
    pub module: usize,
    pub doc: String,
    /// Body contains inline assembly or volatile accesses.
    pub has_asm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LogicAnd,
    LogicOr,
}

impl BinOp {
    pub fn from_str(s: &str) -> Option<BinOp> {
        Some(match s {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            "%" => BinOp::Rem,
            "&" => BinOp::And,
            "|" => BinOp::Or,
            "^" => BinOp::Xor,
            "<<" => BinOp::Shl,
            ">>" => BinOp::Shr,
            "==" => BinOp::Eq,
            "!=" => BinOp::Ne,
            "<" => BinOp::Lt,
            "<=" => BinOp::Le,
            ">" => BinOp::Gt,
            ">=" => BinOp::Ge,
            "&&" => BinOp::LogicAnd,
            "||" => BinOp::LogicOr,
            _ => return None,
        })
    }
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::LogicAnd => "&&",
            BinOp::LogicOr => "||",
        }
    }
    pub fn is_compare(self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }
    pub fn is_logic(self) -> bool {
        matches!(self, BinOp::LogicAnd | BinOp::LogicOr)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intrinsic {
    Lo,
    Hi,
    Peek,
    Poke,
    SliceLen,
}

#[derive(Clone, Debug)]
pub struct AsmBlock {
    /// Assembly text with `{name}` placeholders already resolved to operands.
    pub body: String,
    pub inputs: Vec<(char, Expr)>,
    /// Placeholder references: (text, what it names).
    pub refs: Vec<AsmRef>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum AsmRef {
    Local(LocalId),
    Global(GlobalId),
    Fn(FnId),
    Const(i64),
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Const(Value),
    Local(LocalId),
    Global(GlobalId),
    Unary(UnOp, Box<Expr>),
    /// Operands of arithmetic and comparisons share a machine type.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Wrapping conversion of a scalar to `ty`.
    Cast(Box<Expr>),
    /// Array element; the bool records an unchecked (`raw`) index.
    Index(Box<Expr>, Box<Expr>, bool),
    Field(Box<Expr>, usize),
    Struct(Vec<Expr>),
    Array(Vec<Expr>),
    Repeat(Box<Expr>, u32),
    Call(FnId, Vec<Expr>),
    Intrinsic(Intrinsic, Vec<Expr>),
    /// Value-producing match; arms in order, `None` patterns is the default.
    Match(Box<Expr>, Vec<Arm<Expr>>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Asm(Box<AsmBlock>),
    /// View of a whole constant array as a slice.
    SliceOf(GlobalId),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Type, span: Span) -> Self {
        Self { kind, ty, span }
    }
    pub fn constant(&self) -> Option<i64> {
        match &self.kind {
            ExprKind::Const(Value::Int(n)) => Some(*n),
            _ => None,
        }
    }
}

/// A pattern alternative, as an inclusive range of scalar values.
pub type Pattern = (i64, i64);

#[derive(Clone, Debug)]
pub struct Arm<B> {
    /// Empty means wildcard.
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: B,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let(LocalId, Expr),
    Assign(Expr, Option<BinOp>, Expr, Span),
    Expr(Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    Loop(Vec<Stmt>),
    For {
        var: LocalId,
        start: Expr,
        /// Exclusive end (inclusive ranges are normalized with a flag).
        end: Expr,
        inclusive: bool,
        reverse: bool,
        body: Vec<Stmt>,
    },
    Match(Expr, Vec<Arm<Vec<Stmt>>>),
    Break(Span),
    Continue(Span),
    Return(Option<Expr>, Span),
    Block(Vec<Stmt>),
}

#[derive(Clone, Debug, Default)]
pub struct Cartridge {
    pub title: String,
    pub mapper: String,
    pub mirroring: String,
    /// Concatenated CHR data, padded by the linker.
    pub chr: Vec<u8>,
    pub chr_ram: bool,
    pub prg_kib: Option<u32>,
    pub battery: bool,
    pub span: Span,
    pub fields: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub types: TypeTable,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    pub cartridge: Cartridge,
    /// Module paths by index.
    pub modules: Vec<String>,
}

impl Program {
    pub fn global(&self, id: GlobalId) -> &Global {
        &self.globals[id.0 as usize]
    }
    pub fn function(&self, id: FnId) -> &Function {
        &self.functions[id.0 as usize]
    }
}
