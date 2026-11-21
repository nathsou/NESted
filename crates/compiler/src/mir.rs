//! Mid-level IR: per-context function instances with scalar variables,
//! structure-of-arrays memory objects, and basic blocks of expression trees.
use crate::hir::{BinOp, Context, FnId, GlobalId, Placement, UnOp};
use crate::source::Span;
use std::fmt::Write;

/// Machine scalar types after scalarization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sc {
    U8,
    I8,
    U16,
    I16,
}

impl Sc {
    pub fn wide(self) -> bool {
        matches!(self, Sc::U16 | Sc::I16)
    }
    pub fn signed(self) -> bool {
        matches!(self, Sc::I8 | Sc::I16)
    }
    pub fn size(self) -> usize {
        if self.wide() {
            2
        } else {
            1
        }
    }
    pub fn wrap(self, v: i64) -> i64 {
        match self {
            Sc::U8 => v & 0xff,
            Sc::I8 => v as i8 as i64,
            Sc::U16 => v & 0xffff,
            Sc::I16 => v as i16 as i64,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Sc::U8 => "u8",
            Sc::I8 => "i8",
            Sc::U16 => "u16",
            Sc::I16 => "i16",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum ObjKind {
    Ram(Placement),
    Rom,
    Io(u16),
    /// An array local to a function instance's frame.
    Frame(InstId),
}

/// One scalar or array of scalars in memory. Wide arrays are stored as
/// separate low-byte and high-byte tables.
#[derive(Clone, Debug)]
pub struct Obj {
    pub name: String,
    pub sc: Sc,
    pub len: Option<u32>,
    pub kind: ObjKind,
    pub init: Vec<i64>,
    pub bank: Option<u32>,
    pub shared: bool,
    pub global: Option<GlobalId>,
    pub span: Span,
}

impl Obj {
    pub fn bytes(&self) -> usize {
        self.sc.size() * self.len.unwrap_or(1) as usize
    }
    pub fn volatile(&self) -> bool {
        matches!(self.kind, ObjKind::Io(_)) || self.shared
    }
}

#[derive(Clone, Debug)]
pub struct Var {
    pub name: String,
    pub sc: Sc,
    /// Used as a zero-page pointer (must be allocated in zero page).
    pub pointer: bool,
    pub temp: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ex {
    Const(i64, Sc),
    /// Address of an object's first byte (or of its high-byte table).
    Addr(ObjId, bool),
    Var(VarId, Sc),
    /// Memory object element; index counts elements.
    Load(ObjId, Option<Box<Ex>>, Sc),
    /// Byte at `ptr + index` (index u8).
    Deref(Box<Ex>, Box<Ex>, Sc),
    Peek(Box<Ex>),
    Un(UnOp, Box<Ex>, Sc),
    /// Operands of type `Sc`; comparisons yield U8 0/1.
    Bin(BinOp, Box<Ex>, Box<Ex>, Sc),
    /// Conversion between scalar types (wrapping / extension).
    Cast(Box<Ex>, Sc, Sc),
}

impl Ex {
    pub fn sc(&self) -> Sc {
        match self {
            Ex::Const(_, s)
            | Ex::Var(_, s)
            | Ex::Load(_, _, s)
            | Ex::Deref(_, _, s)
            | Ex::Un(_, _, s) => *s,
            Ex::Addr(..) => Sc::U16,
            Ex::Peek(_) => Sc::U8,
            Ex::Bin(op, _, _, s) => {
                if op.is_compare() || op.is_logic() {
                    Sc::U8
                } else {
                    *s
                }
            }
            Ex::Cast(_, _, to) => *to,
        }
    }
    pub fn konst(&self) -> Option<i64> {
        match self {
            Ex::Const(n, _) => Some(*n),
            _ => None,
        }
    }
    pub fn simple(&self) -> bool {
        matches!(self, Ex::Const(..) | Ex::Var(..))
    }
    /// Visit every sub-expression.
    pub fn walk(&self, f: &mut impl FnMut(&Ex)) {
        f(self);
        match self {
            Ex::Load(_, Some(i), _) => i.walk(f),
            Ex::Deref(p, i, _) => {
                p.walk(f);
                i.walk(f);
            }
            Ex::Peek(a) | Ex::Un(_, a, _) | Ex::Cast(a, _, _) => a.walk(f),
            Ex::Bin(_, a, b, _) => {
                a.walk(f);
                b.walk(f);
            }
            _ => {}
        }
    }
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Ex)) {
        match self {
            Ex::Load(_, Some(i), _) => i.walk_mut(f),
            Ex::Deref(p, i, _) => {
                p.walk_mut(f);
                i.walk_mut(f);
            }
            Ex::Peek(a) | Ex::Un(_, a, _) | Ex::Cast(a, _, _) => a.walk_mut(f),
            Ex::Bin(_, a, b, _) => {
                a.walk_mut(f);
                b.walk_mut(f);
            }
            _ => {}
        }
        f(self);
    }
    pub fn reads_var(&self, v: VarId) -> bool {
        let mut hit = false;
        self.walk(&mut |e| {
            if let Ex::Var(x, _) = e {
                if *x == v {
                    hit = true;
                }
            }
        });
        hit
    }
    pub fn reads_memory(&self) -> bool {
        let mut hit = false;
        self.walk(&mut |e| {
            if matches!(e, Ex::Load(..) | Ex::Deref(..) | Ex::Peek(_)) {
                hit = true;
            }
        });
        hit
    }
    pub fn volatile(&self, objs: &[Obj]) -> bool {
        let mut hit = false;
        self.walk(&mut |e| match e {
            Ex::Peek(_) => hit = true,
            Ex::Load(o, _, _) if objs[o.0 as usize].volatile() => hit = true,
            _ => {}
        });
        hit
    }
    pub fn size(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |_| n += 1);
        n
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Var(VarId),
    Mem(ObjId, Option<Ex>),
    Deref(Ex, Ex),
    Poke(Ex),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AsmOperand {
    Var(VarId),
    Obj(ObjId),
    Fn(InstId),
    Const(i64),
}

#[derive(Clone, Debug)]
pub enum St {
    Assign(Place, Ex),
    Call {
        target: InstId,
        args: Vec<Ex>,
        rets: Vec<Place>,
        /// Variables saved across a recursive self-call.
        saved: Vec<VarId>,
        span: Span,
    },
    Asm {
        body: String,
        operands: Vec<AsmOperand>,
        inputs: Vec<(char, Ex)>,
        output: Option<Place>,
        span: Span,
    },
    /// Source position marker for debug info.
    Line(Span),
}

#[derive(Clone, Debug)]
pub enum Term {
    Goto(BlockId),
    /// Branch on `cond != 0`.
    Branch(Ex, BlockId, BlockId),
    Return,
    /// Not yet terminated (during construction).
    Open,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<St>,
    pub term: Term,
    /// Loop nesting depth, for allocation weights.
    pub depth: u32,
}

#[derive(Clone, Debug)]
pub struct Inst {
    pub name: String,
    pub func: FnId,
    pub ctx: Context,
    pub vars: Vec<Var>,
    pub params: Vec<VarId>,
    /// Scalar result leaves; one byte comes back in A, a word in A/X.
    pub rets: Vec<VarId>,
    pub blocks: Vec<Block>,
    pub entry: BlockId,
    pub inline: bool,
    pub noinline: bool,
    pub recursive: Option<u32>,
    pub interrupt: Option<Context>,
    pub reset: bool,
    pub budget: Option<u32>,
    pub bank: Option<u32>,
    pub span: Span,
    pub has_asm: bool,
}

impl Inst {
    pub fn var(&mut self, name: impl Into<String>, sc: Sc, temp: bool) -> VarId {
        let id = VarId(self.vars.len() as u32);
        self.vars.push(Var {
            name: name.into(),
            sc,
            pointer: false,
            temp,
        });
        id
    }
    pub fn block(&mut self, depth: u32) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(Block {
            stmts: Vec::new(),
            term: Term::Open,
            depth,
        });
        id
    }
    pub fn successors(&self, b: BlockId) -> Vec<BlockId> {
        match &self.blocks[b.0 as usize].term {
            Term::Goto(t) => vec![*t],
            Term::Branch(_, t, f) => vec![*t, *f],
            _ => vec![],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mir {
    pub objs: Vec<Obj>,
    pub insts: Vec<Inst>,
    /// Instance of each (function, context) pair.
    pub inst_of: std::collections::BTreeMap<(FnId, Context), InstId>,
    /// Objects of each global's scalar leaves.
    pub global_objs: std::collections::BTreeMap<GlobalId, Vec<ObjId>>,
}

impl Mir {
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for (i, o) in self.objs.iter().enumerate() {
            let _ = writeln!(
                out,
                "obj{i} {} : {}{} {:?}{}",
                o.name,
                o.sc.name(),
                o.len.map(|n| format!("[{n}]")).unwrap_or_default(),
                o.kind,
                o.bank.map(|b| format!(" bank {b}")).unwrap_or_default()
            );
        }
        for inst in &self.insts {
            out.push('\n');
            let _ = writeln!(
                out,
                "fn {} [{:?}] ({}) -> ({})",
                inst.name,
                inst.ctx,
                inst.params
                    .iter()
                    .map(|v| self.var_name(inst, *v))
                    .collect::<Vec<_>>()
                    .join(", "),
                inst.rets
                    .iter()
                    .map(|v| self.var_name(inst, *v))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for (b, block) in inst.blocks.iter().enumerate() {
                let _ = writeln!(out, "  b{b}:");
                for s in &block.stmts {
                    match s {
                        St::Line(_) => {}
                        _ => {
                            let _ = writeln!(out, "    {}", self.stmt(inst, s));
                        }
                    }
                }
                let t = match &block.term {
                    Term::Goto(t) => format!("goto b{}", t.0),
                    Term::Branch(c, t, f) => {
                        format!("if {} then b{} else b{}", self.ex(inst, c), t.0, f.0)
                    }
                    Term::Return => "return".into(),
                    Term::Open => "<open>".into(),
                };
                let _ = writeln!(out, "    {t}");
            }
        }
        out
    }
    fn var_name(&self, inst: &Inst, v: VarId) -> String {
        format!("{}%{}", inst.vars[v.0 as usize].name, v.0)
    }
    pub fn ex(&self, inst: &Inst, e: &Ex) -> String {
        match e {
            Ex::Const(n, s) => format!("{n}{}", s.name()),
            Ex::Addr(o, hi) => format!(
                "&{}{}",
                self.objs[o.0 as usize].name,
                if *hi { ".hi" } else { "" }
            ),
            Ex::Var(v, _) => self.var_name(inst, *v),
            Ex::Load(o, None, _) => self.objs[o.0 as usize].name.clone(),
            Ex::Load(o, Some(i), _) => {
                format!("{}[{}]", self.objs[o.0 as usize].name, self.ex(inst, i))
            }
            Ex::Deref(p, i, _) => format!("*({} + {})", self.ex(inst, p), self.ex(inst, i)),
            Ex::Peek(a) => format!("peek({})", self.ex(inst, a)),
            Ex::Un(op, a, _) => format!(
                "{}{}",
                match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                    UnOp::BitNot => "~",
                },
                self.ex(inst, a)
            ),
            Ex::Bin(op, a, b, s) => format!(
                "({} {}{} {})",
                self.ex(inst, a),
                op.symbol(),
                s.name(),
                self.ex(inst, b)
            ),
            Ex::Cast(a, _, to) => format!("{}({})", to.name(), self.ex(inst, a)),
        }
    }
    fn place(&self, inst: &Inst, p: &Place) -> String {
        match p {
            Place::Var(v) => self.var_name(inst, *v),
            Place::Mem(o, None) => self.objs[o.0 as usize].name.clone(),
            Place::Mem(o, Some(i)) => {
                format!("{}[{}]", self.objs[o.0 as usize].name, self.ex(inst, i))
            }
            Place::Deref(p, i) => format!("*({} + {})", self.ex(inst, p), self.ex(inst, i)),
            Place::Poke(a) => format!("poke({})", self.ex(inst, a)),
        }
    }
    pub fn stmt(&self, inst: &Inst, s: &St) -> String {
        match s {
            St::Assign(p, e) => format!("{} = {}", self.place(inst, p), self.ex(inst, e)),
            St::Call {
                target,
                args,
                rets,
                saved,
                ..
            } => format!(
                "{}call {}({}){}",
                if rets.is_empty() {
                    String::new()
                } else {
                    format!(
                        "{} = ",
                        rets.iter()
                            .map(|r| self.place(inst, r))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
                self.insts[target.0 as usize].name,
                args.iter()
                    .map(|a| self.ex(inst, a))
                    .collect::<Vec<_>>()
                    .join(", "),
                if saved.is_empty() {
                    String::new()
                } else {
                    format!(
                        " saving {}",
                        saved
                            .iter()
                            .map(|v| self.var_name(inst, *v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            ),
            St::Asm { body, .. } => format!("asm {{ {} }}", body.trim().replace('\n', "; ")),
            St::Line(_) => String::new(),
        }
    }
}
