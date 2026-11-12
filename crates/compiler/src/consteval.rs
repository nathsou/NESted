//! Compile-time interpreter over the typed program. It evaluates constant
//! initializers (including calls to ordinary functions) and doubles as the
//! reference semantics that compiled code is tested against.
use crate::check::Checker;
use crate::diag::Diagnostic;
use crate::hir::*;
use crate::source::Span;
use crate::types::{Type, TypeTable};
use std::rc::Rc;

/// Something that can supply checked function bodies and constant values.
pub trait Env {
    fn types(&self) -> &TypeTable;
    fn function(&mut self, f: FnId) -> Option<Rc<Function>>;
    fn global(&mut self, g: GlobalId) -> Result<Value, String>;
}

impl Env for Checker<'_> {
    fn types(&self) -> &TypeTable {
        &self.types
    }
    fn function(&mut self, f: FnId) -> Option<Rc<Function>> {
        self.body(f)
    }
    fn global(&mut self, g: GlobalId) -> Result<Value, String> {
        self.ensure(crate::check::Def::Global(g));
        let gl = &self.globals[g.0 as usize];
        if !gl.is_const() {
            return Err(format!(
                "'{}' is a variable, not a compile-time constant",
                gl.name
            ));
        }
        Ok(gl.value.clone())
    }
}

/// A finished program, for running reference executions in tests.
pub struct ProgramEnv<'a> {
    pub program: &'a Program,
    pub globals: Vec<Value>,
}

impl<'a> ProgramEnv<'a> {
    pub fn new(program: &'a Program) -> Self {
        Self {
            program,
            globals: program.globals.iter().map(|g| g.value.clone()).collect(),
        }
    }
}

impl Env for ProgramEnv<'_> {
    fn types(&self) -> &TypeTable {
        &self.program.types
    }
    fn function(&mut self, f: FnId) -> Option<Rc<Function>> {
        Some(Rc::new(self.program.functions[f.0 as usize].clone()))
    }
    fn global(&mut self, g: GlobalId) -> Result<Value, String> {
        Ok(self.globals[g.0 as usize].clone())
    }
}

enum Flow {
    Normal,
    Break,
    Continue,
    Return(Value),
}

type R<T> = Result<T, Diagnostic>;

pub struct Interp<'e, E: Env> {
    pub env: &'e mut E,
    frames: Vec<Vec<Value>>,
    fuel: u64,
    depth: u32,
    /// Writes to globals are allowed (reference execution of a whole program).
    pub mutable_globals: bool,
    pub global_writes: Vec<(GlobalId, Value)>,
}

impl<'e, E: Env> Interp<'e, E> {
    pub fn new(env: &'e mut E) -> Self {
        Self {
            env,
            frames: vec![Vec::new()],
            fuel: 50_000_000,
            depth: 0,
            mutable_globals: false,
            global_writes: Vec::new(),
        }
    }

    fn tick(&mut self, span: Span) -> R<()> {
        if self.fuel == 0 {
            return Err(Diagnostic::error(
                span,
                "Compile-time evaluation ran too long (50 million steps)",
            ));
        }
        self.fuel -= 1;
        Ok(())
    }

    fn wrap(&self, t: &Type, v: i64) -> i64 {
        if *t == Type::Int {
            v
        } else {
            self.env.types().wrap(t, v)
        }
    }

    fn read_global(&mut self, g: GlobalId, span: Span) -> R<Value> {
        if let Some((_, v)) = self.global_writes.iter().rev().find(|(x, _)| *x == g) {
            return Ok(v.clone());
        }
        self.env.global(g).map_err(|m| Diagnostic::error(span, m))
    }

    pub fn call(&mut self, f: FnId, args: Vec<Value>, span: Span) -> R<Value> {
        let Some(func) = self.env.function(f) else {
            return Err(Diagnostic::error(span, "This function is not available at compile time (it may depend on the constant being defined)"));
        };
        if self.depth > 200 {
            return Err(Diagnostic::error(
                span,
                "Compile-time recursion is too deep",
            ));
        }
        if func.has_asm {
            return Err(Diagnostic::error(
                span,
                format!(
                    "'{}' uses hardware access or assembly and cannot run at compile time",
                    func.name
                ),
            ));
        }
        let mut frame = vec![Value::Void; func.locals.len()];
        for (p, v) in func.params.iter().zip(args) {
            frame[p.0 as usize] = v;
        }
        self.frames.push(frame);
        self.depth += 1;
        let r = self.block(&func.body);
        self.depth -= 1;
        self.frames.pop();
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Void),
        }
    }

    pub fn expr(&mut self, e: &Expr) -> R<Value> {
        self.tick(e.span)?;
        Ok(match &e.kind {
            ExprKind::Const(v) => v.clone(),
            ExprKind::Local(id) => self.frames.last().unwrap()[id.0 as usize].clone(),
            ExprKind::Global(g) => self.read_global(*g, e.span)?,
            ExprKind::SliceOf(g) => {
                let n = match &self.env.types().size(&Type::U8) {
                    _ => match self.read_global(*g, e.span)? {
                        Value::Aggregate(v) => v.len() as u32,
                        _ => 0,
                    },
                };
                Value::Slice(*g, 0, n)
            }
            ExprKind::Unary(op, x) => {
                let v = self.expr(x)?.int();
                Value::Int(match op {
                    UnOp::Neg => self.wrap(&e.ty, -v),
                    UnOp::Not => (v == 0) as i64,
                    UnOp::BitNot => self.wrap(&e.ty, !v),
                })
            }
            ExprKind::Binary(op, a, b) => {
                let x = self.expr(a)?.int();
                if *op == BinOp::LogicAnd && x == 0 {
                    return Ok(Value::Int(0));
                }
                if *op == BinOp::LogicOr && x != 0 {
                    return Ok(Value::Int(1));
                }
                let y = self.expr(b)?.int();
                Value::Int(
                    binary(self.env.types(), *op, x, y, &a.ty).ok_or_else(|| {
                        Diagnostic::error(e.span, "Division by zero at compile time")
                    })?,
                )
            }
            ExprKind::Cast(x) => {
                let v = self.expr(x)?.int();
                Value::Int(self.wrap(&e.ty, v))
            }
            ExprKind::Index(b, i, _) => {
                let i = self.expr(i)?.int();
                match self.expr(b)? {
                    Value::Aggregate(v) => v.get(i as usize).cloned().ok_or_else(|| {
                        Diagnostic::error(
                            e.span,
                            format!("Index {i} is out of bounds for length {}", v.len()),
                        )
                    })?,
                    Value::Slice(g, start, len) => {
                        if i < 0 || i >= len as i64 {
                            return Err(Diagnostic::error(
                                e.span,
                                format!("Index {i} is out of bounds for slice length {len}"),
                            ));
                        }
                        let data = self.read_global(g, e.span)?;
                        data.items()[(start as i64 + i) as usize].clone()
                    }
                    _ => return Err(Diagnostic::error(e.span, "Cannot index this value")),
                }
            }
            ExprKind::Field(b, f) => self
                .expr(b)?
                .items()
                .get(*f)
                .cloned()
                .unwrap_or(Value::Void),
            ExprKind::Struct(fields) | ExprKind::Array(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for f in fields {
                    out.push(self.expr(f)?);
                }
                Value::aggregate(out)
            }
            ExprKind::Repeat(x, n) => Value::aggregate(vec![self.expr(x)?; *n as usize]),
            ExprKind::Call(f, args) => {
                let mut values = Vec::new();
                for a in args {
                    values.push(self.expr(a)?);
                }
                self.call(*f, values, e.span)?
            }
            ExprKind::Intrinsic(i, args) => match i {
                Intrinsic::Lo => Value::Int(self.expr(&args[0])?.int() & 255),
                Intrinsic::Hi => Value::Int((self.expr(&args[0])?.int() >> 8) & 255),
                Intrinsic::SliceLen => match self.expr(&args[0])? {
                    Value::Slice(_, _, n) => Value::Int(n as i64),
                    _ => Value::Int(0),
                },
                Intrinsic::Addr => {
                    return Err(Diagnostic::error(
                        e.span,
                        "Addresses are only known after linking",
                    ))
                }
                Intrinsic::Peek | Intrinsic::Poke => {
                    return Err(Diagnostic::error(
                        e.span,
                        "Hardware access cannot run at compile time",
                    ))
                }
            },
            ExprKind::Match(x, arms) => {
                let v = self.expr(x)?.int();
                for arm in arms {
                    if self.arm_matches(arm, v)? {
                        return self.expr(&arm.body);
                    }
                }
                return Err(Diagnostic::error(e.span, "No match arm applies"));
            }
            ExprKind::If(c, a, b) => {
                if self.expr(c)?.int() != 0 {
                    self.expr(a)?
                } else {
                    self.expr(b)?
                }
            }
            ExprKind::Asm(_) => {
                return Err(Diagnostic::error(
                    e.span,
                    "Assembly cannot run at compile time",
                ))
            }
        })
    }

    fn arm_matches<B>(&mut self, arm: &Arm<B>, v: i64) -> R<bool> {
        let hit =
            arm.patterns.is_empty() || arm.patterns.iter().any(|(lo, hi)| *lo <= v && v <= *hi);
        if !hit {
            return Ok(false);
        }
        match &arm.guard {
            Some(g) => Ok(self.expr(g)?.int() != 0),
            None => Ok(true),
        }
    }

    fn block(&mut self, body: &[Stmt]) -> R<Flow> {
        for s in body {
            match self.stmt(s)? {
                Flow::Normal => {}
                f => return Ok(f),
            }
        }
        Ok(Flow::Normal)
    }

    fn assign(&mut self, place: &Expr, value: Value) -> R<()> {
        // Collect the access path from the root variable.
        let mut path = Vec::new();
        let mut cur = place;
        loop {
            match &cur.kind {
                ExprKind::Index(b, i, _) => {
                    let i = self.expr(i)?.int();
                    path.push((i as usize, cur.span));
                    cur = b;
                }
                ExprKind::Field(b, f) => {
                    path.push((*f, cur.span));
                    cur = b;
                }
                _ => break,
            }
        }
        path.reverse();
        let mut root = match &cur.kind {
            ExprKind::Local(id) => std::mem::replace(
                &mut self.frames.last_mut().unwrap()[id.0 as usize],
                Value::Void,
            ),
            ExprKind::Global(g) if self.mutable_globals => self.read_global(*g, cur.span)?,
            ExprKind::Global(_) => {
                return Err(Diagnostic::error(
                    cur.span,
                    "Globals cannot be modified at compile time",
                ))
            }
            _ => return Err(Diagnostic::error(cur.span, "Cannot assign here")),
        };
        let mut slot = &mut root;
        for (i, span) in &path {
            let Value::Aggregate(v) = slot else {
                return Err(Diagnostic::error(*span, "Cannot assign here"));
            };
            let v = Rc::make_mut(v);
            let len = v.len();
            slot = v.get_mut(*i).ok_or_else(|| {
                Diagnostic::error(
                    *span,
                    format!("Index {i} is out of bounds for length {len}"),
                )
            })?;
        }
        *slot = value;
        match &cur.kind {
            ExprKind::Local(id) => self.frames.last_mut().unwrap()[id.0 as usize] = root,
            ExprKind::Global(g) => self.global_writes.push((*g, root)),
            _ => {}
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> R<Flow> {
        match s {
            Stmt::Let(id, e) => {
                let v = self.expr(e)?;
                self.frames.last_mut().unwrap()[id.0 as usize] = v;
            }
            Stmt::Assign(place, op, value, span) => {
                self.tick(*span)?;
                let v = self.expr(value)?;
                let v = match op {
                    None => v,
                    Some(op) => {
                        let old = self.expr(place)?.int();
                        let t = match place.ty {
                            Type::Range(_) => self.env.types().base(&place.ty),
                            ref t => t.clone(),
                        };
                        let r =
                            binary(self.env.types(), *op, old, v.int(), &t).ok_or_else(|| {
                                Diagnostic::error(*span, "Division by zero at compile time")
                            })?;
                        Value::Int(r)
                    }
                };
                self.assign(place, v)?;
            }
            Stmt::Expr(e) => {
                self.expr(e)?;
            }
            Stmt::If(c, a, b) => {
                return if self.expr(c)?.int() != 0 {
                    self.block(a)
                } else {
                    self.block(b)
                };
            }
            Stmt::While(c, body) => {
                while self.expr(c)?.int() != 0 {
                    match self.block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            Stmt::Loop(body) => loop {
                self.tick(Span::default())?;
                match self.block(body)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    _ => {}
                }
            },
            Stmt::For {
                var,
                start,
                end,
                inclusive,
                reverse,
                body,
            } => {
                let a = self.expr(start)?.int();
                let b = self.expr(end)?.int() - !inclusive as i64;
                let values: Box<dyn Iterator<Item = i64>> = if *reverse {
                    Box::new((a..=b).rev())
                } else {
                    Box::new(a..=b)
                };
                for i in values {
                    self.frames.last_mut().unwrap()[var.0 as usize] = Value::Int(i);
                    match self.block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
            }
            Stmt::Match(x, arms) => {
                let v = self.expr(x)?.int();
                for arm in arms {
                    if self.arm_matches(arm, v)? {
                        return self.block(&arm.body);
                    }
                }
            }
            Stmt::Break(_) => return Ok(Flow::Break),
            Stmt::Continue(_) => return Ok(Flow::Continue),
            Stmt::Return(v, _) => {
                let v = match v {
                    Some(e) => self.expr(e)?,
                    None => Value::Void,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Block(b) => return self.block(b),
        }
        Ok(Flow::Normal)
    }
}

/// NESted arithmetic on values of operand type `t`; `None` on division by zero.
pub fn binary(types: &TypeTable, op: BinOp, a: i64, b: i64, t: &Type) -> Option<i64> {
    let base = match t {
        Type::Int => Type::Int,
        t => types.base(t),
    };
    let signed = matches!(base, Type::I8 | Type::I16 | Type::Int);
    let bits: i64 = match base {
        Type::U8 | Type::I8 => 8,
        Type::U16 | Type::I16 => 16,
        _ => 63,
    };
    let wrap = |v: i64| {
        if base == Type::Int {
            v
        } else {
            types.wrap(&base, v)
        }
    };
    let unsigned = |v: i64| if bits >= 63 { v } else { v & ((1 << bits) - 1) };
    Some(match op {
        BinOp::Add => wrap(a.wrapping_add(b)),
        BinOp::Sub => wrap(a.wrapping_sub(b)),
        BinOp::Mul => wrap(a.wrapping_mul(b)),
        BinOp::Div => {
            if b == 0 {
                return None;
            }
            if signed {
                wrap(a.wrapping_div(b))
            } else {
                wrap(unsigned(a) / unsigned(b))
            }
        }
        BinOp::Rem => {
            if b == 0 {
                return None;
            }
            if signed {
                wrap(a.wrapping_rem(b))
            } else {
                wrap(unsigned(a) % unsigned(b))
            }
        }
        BinOp::And => wrap(a & b),
        BinOp::Or => wrap(a | b),
        BinOp::Xor => wrap(a ^ b),
        BinOp::Shl => {
            if b >= bits {
                0
            } else {
                wrap(a << b)
            }
        }
        BinOp::Shr => {
            if signed {
                wrap(a >> b.min(63))
            } else if b >= bits {
                0
            } else {
                wrap(unsigned(a) >> b)
            }
        }
        BinOp::Eq => (a == b) as i64,
        BinOp::Ne => (a != b) as i64,
        BinOp::Lt => (a < b) as i64,
        BinOp::Le => (a <= b) as i64,
        BinOp::Gt => (a > b) as i64,
        BinOp::Ge => (a >= b) as i64,
        BinOp::LogicAnd => (a != 0 && b != 0) as i64,
        BinOp::LogicOr => (a != 0 || b != 0) as i64,
    })
}
