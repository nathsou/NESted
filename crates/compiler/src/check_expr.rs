//! Expression and statement checking: types, coercions, index proofs and
//! match exhaustiveness.
use crate::ast::{self, ArmBody, ExprKind as AK, StmtKind};
use crate::check::{Checker, Def};
use crate::diag::Diagnostic;
use crate::hir::*;
use crate::source::Span;
use crate::types::*;
use std::collections::BTreeSet;

/// What a fact is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Local(LocalId),
    Global(GlobalId),
    /// Kill marker for every global (a call or assembly ran).
    AllGlobals,
}

/// Lexical environment of the expression being checked.
pub struct Scope {
    pub module: usize,
    pub fn_id: Option<FnId>,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<Vec<(String, LocalId)>>,
    /// Proven inclusive bounds of variables. An entry with lo > hi kills
    /// earlier facts about its key (the variable was assigned).
    pub facts: Vec<(Key, i64, i64)>,
    /// `index < len(slice)` facts for immutable locals.
    pub slice_facts: Vec<(LocalId, LocalId)>,
    /// Immutable locals bound to `len(slice)`.
    pub len_alias: Vec<(LocalId, LocalId)>,
    pub loops: u32,
    pub result: Type,
    pub has_asm: bool,
}

impl Scope {
    pub fn constant(module: usize) -> Self {
        Self {
            module,
            fn_id: None,
            locals: Vec::new(),
            blocks: vec![Vec::new()],
            facts: Vec::new(),
            slice_facts: Vec::new(),
            len_alias: Vec::new(),
            loops: 0,
            result: Type::Void,
            has_asm: false,
        }
    }
    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.blocks
            .iter()
            .rev()
            .flat_map(|b| b.iter().rev())
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }
    fn declare(&mut self, name: &str, ty: Type, mutable: bool, span: Span) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(LocalDecl {
            name: name.to_owned(),
            ty,
            mutable,
            span,
        });
        self.blocks.last_mut().unwrap().push((name.to_owned(), id));
        id
    }
    fn immutable(&self, id: LocalId) -> bool {
        !self.locals[id.0 as usize].mutable
    }
}

fn n_of(k: &AK) -> &i64 {
    match k {
        AK::Int(n) => n,
        _ => &0,
    }
}

fn err(span: Span) -> Expr {
    Expr::new(ExprKind::Const(Value::Void), Type::Error, span)
}

fn int(n: i64, ty: Type, span: Span) -> Expr {
    Expr::new(ExprKind::Const(Value::Int(n)), ty, span)
}

impl Checker<'_> {
    // ------------------------------------------------------------ helpers

    pub(crate) fn ty_name(&self, t: &Type) -> String {
        self.types.name(t)
    }

    pub(crate) fn default_type(&mut self, e: &Expr) -> Type {
        match &e.ty {
            Type::Int => {
                let n = e.constant().unwrap_or(0);
                if (0..=255).contains(&n) {
                    Type::U8
                } else if (-128..0).contains(&n) {
                    Type::I8
                } else if (0..=65535).contains(&n) {
                    Type::U16
                } else if (-32768..0).contains(&n) {
                    Type::I16
                } else {
                    self.diags.error(e.span, "Integer does not fit any type");
                    Type::Error
                }
            }
            Type::Array(elem, n) if **elem == Type::Int => {
                let mut lo = 0;
                let mut hi = 0;
                if let ExprKind::Const(v) = &e.kind {
                    for x in v.items() {
                        lo = lo.min(x.int());
                        hi = hi.max(x.int());
                    }
                }
                if let ExprKind::Array(items) = &e.kind {
                    for x in items {
                        let v = x.constant().unwrap_or(0);
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
                let t = if lo >= 0 && hi <= 255 {
                    Type::U8
                } else if lo >= -128 && hi <= 127 {
                    Type::I8
                } else if lo >= 0 {
                    Type::U16
                } else {
                    Type::I16
                };
                Type::Array(Box::new(t), *n)
            }
            Type::Never => Type::Void,
            t => t.clone(),
        }
    }

    /// Evaluate a checked constant expression.
    pub(crate) fn eval(&mut self, e: &Expr) -> Option<Value> {
        let mut interp = crate::consteval::Interp::new(self);
        match interp.expr(e) {
            Ok(v) => Some(v),
            Err(d) => {
                self.diags.push(d);
                None
            }
        }
    }

    pub(crate) fn const_int(&mut self, m: usize, e: &ast::Expr) -> Option<i64> {
        let mut scope = Scope::constant(m);
        let x = self.expr(&mut scope, e, None);
        if x.ty.is_error() {
            return None;
        }
        if !self.types.is_scalar(&x.ty) {
            self.diags.error(e.span, "Expected an integer constant");
            return None;
        }
        if let Some(n) = x.constant() {
            return Some(n);
        }
        self.eval(&x).map(|v| v.int())
    }

    // --------------------------------------------------------- intervals

    /// Sound inclusive bounds of a scalar expression's value.
    pub(crate) fn interval(&self, s: &Scope, e: &Expr) -> (i64, i64) {
        let full = self.types.bounds(&e.ty);
        let base = self.types.base(&e.ty);
        let (blo, bhi) = self.types.bounds(&base);
        let fits = |lo: i64, hi: i64| {
            if lo >= blo && hi <= bhi {
                (lo, hi)
            } else {
                full
            }
        };
        match &e.kind {
            ExprKind::Const(Value::Int(n)) => (*n, *n),
            ExprKind::Local(id) => self.fact_bounds(s, Key::Local(*id), full),
            ExprKind::Global(g) if self.trackable(*g) => self.fact_bounds(s, Key::Global(*g), full),
            ExprKind::Cast(x) => {
                let (lo, hi) = self.interval(s, x);
                if lo >= blo && hi <= bhi {
                    (lo.max(full.0), hi.min(full.1))
                } else {
                    full
                }
            }
            ExprKind::Binary(op, a, b) => {
                let (alo, ahi) = self.interval(s, a);
                let (clo, chi) = self.interval(s, b);
                let signed = self.types.signed(&e.ty);
                match op {
                    BinOp::And if !signed => {
                        let m = if clo == chi && clo >= 0 {
                            clo
                        } else if alo == ahi && alo >= 0 {
                            alo
                        } else {
                            ahi.max(chi)
                        };
                        if alo >= 0 && clo >= 0 {
                            (0, m.min(ahi).min(chi))
                        } else {
                            full
                        }
                    }
                    BinOp::Rem if !signed && clo == chi && clo > 0 => (0, (clo - 1).min(ahi)),
                    BinOp::Shr if !signed && clo == chi => (alo >> clo, ahi >> clo),
                    BinOp::Div if !signed && clo == chi && clo > 0 => (alo / clo, ahi / clo),
                    BinOp::Add => fits(alo + clo, ahi + chi),
                    BinOp::Sub => fits(alo - chi, ahi - clo),
                    BinOp::Mul if alo >= 0 && clo >= 0 => fits(alo * clo, ahi * chi),
                    BinOp::Shl if alo >= 0 && clo == chi && clo < 16 => {
                        fits(alo << clo, ahi << clo)
                    }
                    BinOp::Or if !signed && alo >= 0 && clo >= 0 => {
                        let bits = 64 - (ahi | chi).leading_zeros();
                        fits(alo.max(clo), (1i64 << bits) - 1)
                    }
                    _ if op.is_compare() || op.is_logic() => (0, 1),
                    _ => full,
                }
            }
            ExprKind::If(_, a, b) => {
                let (alo, ahi) = self.interval(s, a);
                let (blo2, bhi2) = self.interval(s, b);
                (alo.min(blo2), ahi.max(bhi2))
            }
            ExprKind::Match(_, arms) => {
                let mut lo = i64::MAX;
                let mut hi = i64::MIN;
                for a in arms {
                    let (l, h) = self.interval(s, &a.body);
                    lo = lo.min(l);
                    hi = hi.max(h);
                }
                if arms.is_empty() {
                    full
                } else {
                    (lo, hi)
                }
            }
            ExprKind::Intrinsic(Intrinsic::Lo | Intrinsic::Hi, _) => (0, 255),
            _ => full,
        }
    }

    /// Globals whose value only this thread changes.
    fn trackable(&self, g: GlobalId) -> bool {
        let gl = &self.globals[g.0 as usize];
        matches!(gl.kind, GlobalKind::Var) && !gl.shared
    }

    fn fact_bounds(&self, s: &Scope, key: Key, full: (i64, i64)) -> (i64, i64) {
        let mut r = full;
        for (k, lo, hi) in s.facts.iter().rev() {
            let global = matches!(key, Key::Global(_));
            if *k == key || (global && *k == Key::AllGlobals) {
                if lo > hi {
                    break;
                }
                if *k == key {
                    r = (r.0.max(*lo), r.1.min(*hi));
                }
            }
        }
        if r.0 > r.1 {
            full
        } else {
            r
        }
    }

    fn kill(&self, s: &mut Scope, key: Key) {
        s.facts.push((key, 1, 0));
    }

    /// Before a loop body: forget facts about anything the loop assigns.
    fn kill_loop(&mut self, s: &mut Scope, body: &[ast::Stmt], cond: Option<&ast::Expr>) {
        let mut names = Vec::new();
        let mut calls = false;
        for st in body {
            assigned(st, &mut names, &mut calls);
        }
        if let Some(c) = cond {
            calls |= has_call(c);
        }
        for n in names {
            match n {
                Some(n) => {
                    if let Some(id) = s.lookup(&n) {
                        self.kill(s, Key::Local(id));
                    } else {
                        self.kill(s, Key::AllGlobals);
                    }
                }
                None => self.kill(s, Key::AllGlobals),
            }
        }
        if calls {
            self.kill(s, Key::AllGlobals);
        }
    }

    /// Facts about a variable after `place = value`.
    fn assigned_fact(&mut self, s: &mut Scope, place: &Expr, value: Option<&Expr>) {
        let key = match &place.kind {
            ExprKind::Local(id) => Key::Local(*id),
            ExprKind::Global(g) => Key::Global(*g),
            _ => return,
        };
        self.kill(s, key);
        if let Some(v) = value {
            let (lo, hi) = self.interval(s, v);
            let full = self.types.bounds(&place.ty);
            if (lo, hi) != full && lo <= hi {
                s.facts.push((key, lo, hi));
            }
        }
    }

    // ----------------------------------------------------------- coercion

    pub(crate) fn coerce(&mut self, s: &mut Scope, e: Expr, target: &Type) -> Expr {
        if &e.ty == target || e.ty.is_error() || target.is_error() || e.ty == Type::Never {
            return e;
        }
        let span = e.span;
        let mismatch = |c: &mut Self, e: &Expr| {
            c.diags.error(
                span,
                format!("Expected {}, found {}", c.ty_name(target), c.ty_name(&e.ty)),
            );
            err(span)
        };
        match (&e.ty, target) {
            (Type::Int, t) if self.types.is_integer(t) => {
                let n = e.constant().unwrap_or(0);
                let (lo, hi) = self.types.bounds(t);
                // Unsigned targets also accept the matching negative
                // spelling only through explicit casts.
                if n < lo || n > hi {
                    self.diags.error(
                        span,
                        format!("{n} does not fit {} ({lo}..={hi})", self.ty_name(t)),
                    );
                    return err(span);
                }
                int(n, t.clone(), span)
            }
            (from, to) if self.types.is_integer(from) && self.types.is_integer(to) => {
                let fb = self.types.base(from);
                let tb = self.types.base(to);
                if fb != tb {
                    self.diags.error(
                        span,
                        format!(
                            "Expected {}, found {}; convert explicitly with {}(...)",
                            self.ty_name(to),
                            self.ty_name(from),
                            self.ty_name(&tb)
                        ),
                    );
                    return err(span);
                }
                if let Type::Range(_) = to {
                    let (lo, hi) = self.interval(s, &e);
                    let (tlo, thi) = self.types.bounds(to);
                    if lo < tlo || hi > thi {
                        self.diags.push(Diagnostic::error(
                            span,
                            format!(
                                "Cannot prove this {} value is within {} ({tlo}..={thi}); its bounds are {lo}..={hi}. Guard it, mask it, or convert unchecked with 'as {}'",
                                self.ty_name(from),
                                self.ty_name(to),
                                self.ty_name(to)
                            ),
                        ));
                        return err(span);
                    }
                }
                if let Some(n) = e.constant() {
                    return int(n, to.clone(), span);
                }
                Expr::new(ExprKind::Cast(Box::new(e)), to.clone(), span)
            }
            (Type::Array(from, n), Type::Array(to, m)) if n == m => {
                let to = (**to).clone();
                let from = (**from).clone();
                let kind = match e.kind {
                    ExprKind::Array(items) => {
                        ExprKind::Array(items.into_iter().map(|x| self.coerce(s, x, &to)).collect())
                    }
                    ExprKind::Repeat(x, n) => {
                        ExprKind::Repeat(Box::new(self.coerce(s, *x, &to)), n)
                    }
                    ExprKind::Const(v)
                        if from == Type::Int || self.types.base(&from) == self.types.base(&to) =>
                    {
                        let (lo, hi) = self.types.bounds(&to);
                        if v.items().iter().any(|x| x.int() < lo || x.int() > hi) {
                            self.diags
                                .error(span, format!("A value does not fit {}", self.ty_name(&to)));
                            return err(span);
                        }
                        ExprKind::Const(v)
                    }
                    kind => {
                        let e = Expr::new(kind, Type::Array(Box::new(from), *n), span);
                        return mismatch(self, &e);
                    }
                };
                Expr::new(kind, target.clone(), span)
            }
            (Type::Array(from, _), Type::Slice(to)) => {
                let fb = self.types.base(from);
                let tb = self.types.base(to);
                let compatible = **from == Type::Int
                    || from == to
                    || (fb == tb && !matches!(**to, Type::Range(_) | Type::Enum(_)));
                if !compatible {
                    return mismatch(self, &e);
                }
                let g = match &e.kind {
                    ExprKind::Global(g) if self.globals[g.0 as usize].is_const() => *g,
                    ExprKind::Const(v) => self.intern_bytes(v.clone(), (**to).clone(), span),
                    _ => {
                        self.diags.error(
                            span,
                            "Slices view constant ROM arrays; this array is not constant",
                        );
                        return err(span);
                    }
                };
                Expr::new(ExprKind::SliceOf(g), target.clone(), span)
            }
            _ => mismatch(self, &e),
        }
    }

    /// A hidden constant global holding literal data (strings, assets).
    fn intern_bytes(&mut self, v: Value, elem: Type, span: Span) -> GlobalId {
        let bytes: Vec<u8> = v.items().iter().map(|x| x.int() as u8).collect();
        if let Some(g) = self.strings.get(&bytes) {
            return *g;
        }
        let id = GlobalId(self.globals.len() as u32);
        let n = bytes.len() as u32;
        self.globals.push(Global {
            name: format!("__lit{}", id.0),
            path: format!("__lit{}", id.0),
            ty: Type::Array(Box::new(elem), n),
            kind: GlobalKind::Const,
            value: v,
            placement: Placement::Auto,
            bank: None,
            shared: false,
            span,
            module: 0,
            doc: String::new(),
        });
        self.strings.insert(bytes, id);
        id
    }

    /// Unify two operand types for arithmetic or comparison.
    fn unify(&mut self, s: &mut Scope, a: Expr, b: Expr, span: Span) -> (Expr, Expr, Type) {
        if a.ty.is_error() || b.ty.is_error() {
            return (a, b, Type::Error);
        }
        let decay = |c: &Self, t: &Type| match t {
            Type::Range(_) => c.types.base(t),
            t => t.clone(),
        };
        let ta = decay(self, &a.ty);
        let tb = decay(self, &b.ty);
        if ta == tb {
            let a = self.coerce(s, a, &ta);
            let b = self.coerce(s, b, &tb);
            return (a, b, ta);
        }
        if ta == Type::Int && self.types.is_integer(&tb) {
            let a = self.coerce(s, a, &tb);
            let b = self.coerce(s, b, &tb);
            return (a, b, tb);
        }
        if tb == Type::Int && self.types.is_integer(&ta) {
            let a = self.coerce(s, a, &ta);
            let b = self.coerce(s, b, &ta);
            return (a, b, ta);
        }
        self.diags.error(
            span,
            format!(
                "Mismatched operand types {} and {}; convert one explicitly",
                self.ty_name(&a.ty),
                self.ty_name(&b.ty)
            ),
        );
        (a, b, Type::Error)
    }

    fn fold(&self, op: BinOp, a: i64, b: i64, t: &Type) -> Option<i64> {
        let signed = self.types.signed(t) || *t == Type::Int;
        let bits = if *t == Type::Int {
            63
        } else {
            self.types.size(t) as u32 * 8
        };
        let v = match op {
            BinOp::Add => a + b,
            BinOp::Sub => a - b,
            BinOp::Mul => a.wrapping_mul(b),
            BinOp::Div => {
                if b == 0 {
                    return None;
                }
                if signed {
                    a / b
                } else {
                    (a as u64 / b as u64) as i64
                }
            }
            BinOp::Rem => {
                if b == 0 {
                    return None;
                }
                if signed {
                    a % b
                } else {
                    (a as u64 % b as u64) as i64
                }
            }
            BinOp::And => a & b,
            BinOp::Or => a | b,
            BinOp::Xor => a ^ b,
            BinOp::Shl => {
                if b >= bits as i64 {
                    0
                } else {
                    a << b
                }
            }
            BinOp::Shr => {
                if signed {
                    a >> b.min(63)
                } else if b >= bits as i64 {
                    0
                } else {
                    let m = if bits >= 63 { a } else { a & ((1 << bits) - 1) };
                    m >> b
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
        };
        Some(if *t == Type::Int {
            v
        } else {
            self.types.wrap(t, v)
        })
    }

    // -------------------------------------------------------- expressions

    pub(crate) fn expr(&mut self, s: &mut Scope, e: &ast::Expr, expected: Option<&Type>) -> Expr {
        let span = e.span;
        match &e.kind {
            AK::Error => err(span),
            AK::Int(_) | AK::Char(_) => {
                let n = match &e.kind {
                    AK::Char(c) => *c as i64,
                    _ => *n_of(&e.kind),
                };
                // Literals take a machine type from context, as in `~0` for u8.
                if let Some(t) = expected.filter(|t| t.is_machine_int()) {
                    let (lo, hi) = self.types.bounds(t);
                    if n >= lo && n <= hi {
                        return int(n, t.clone(), span);
                    }
                }
                int(n, Type::Int, span)
            }
            AK::Bool(b) => int(*b as i64, Type::Bool, span),
            AK::Str(bytes) => {
                let v = Value::aggregate(bytes.iter().map(|b| Value::Int(*b as i64)).collect());
                if bytes.is_empty() {
                    self.diags.error(span, "Empty strings are not supported");
                    return err(span);
                }
                Expr::new(
                    ExprKind::Const(v),
                    Type::Array(Box::new(Type::U8), bytes.len() as u32),
                    span,
                )
            }
            AK::Path(p) => self.path_expr(s, p, expected),
            AK::Unary(op, x) => {
                let x = self.expr(s, x, expected);
                if x.ty.is_error() {
                    return x;
                }
                match *op {
                    "!" => {
                        let x = self.coerce(s, x, &Type::Bool);
                        if let Some(n) = x.constant() {
                            return int((n == 0) as i64, Type::Bool, span);
                        }
                        Expr::new(ExprKind::Unary(UnOp::Not, Box::new(x)), Type::Bool, span)
                    }
                    op => {
                        if !self.types.is_integer(&x.ty) {
                            self.diags.error(
                                span,
                                format!("'{op}' needs an integer, found {}", self.ty_name(&x.ty)),
                            );
                            return err(span);
                        }
                        let t = match &x.ty {
                            Type::Range(_) => self.types.base(&x.ty),
                            t => t.clone(),
                        };
                        let x = self.coerce(s, x, &t);
                        if let Some(n) = x.constant() {
                            let v = if op == "-" { -n } else { !n };
                            return int(
                                if t == Type::Int {
                                    v
                                } else {
                                    self.types.wrap(&t, v)
                                },
                                t,
                                span,
                            );
                        }
                        let u = if op == "-" { UnOp::Neg } else { UnOp::BitNot };
                        Expr::new(ExprKind::Unary(u, Box::new(x)), t, span)
                    }
                }
            }
            AK::Binary(op, a, b) => self.binary(s, op, a, b, expected, span),
            AK::Call(callee, args) => self.call(s, callee, args, expected, span),
            AK::Index(base, index, raw) => {
                let b = self.expr(s, base, None);
                let i = self.expr(s, index, None);
                self.index(s, b, i, *raw, span)
            }
            AK::Field(base, field) => {
                let b = self.expr(s, base, None);
                match &b.ty {
                    Type::Struct(sid) => {
                        let sid = *sid;
                        let def = &self.types.structs[sid.0 as usize];
                        match def.fields.iter().position(|f| f.0 == field.name) {
                            Some(i) => {
                                let t = def.fields[i].1.clone();
                                self.refs.push((field.span, Def::Field(sid, i as u32)));
                                if let ExprKind::Const(v) = &b.kind {
                                    if let Some(x) = v.items().get(i) {
                                        return Expr::new(ExprKind::Const(x.clone()), t, span);
                                    }
                                }
                                Expr::new(ExprKind::Field(Box::new(b), i), t, span)
                            }
                            None => {
                                let name = def.name.clone();
                                self.diags.error(
                                    field.span,
                                    format!("'{name}' has no field '{}'", field.name),
                                );
                                err(span)
                            }
                        }
                    }
                    Type::Error => b,
                    t => {
                        let t = self.ty_name(t);
                        self.diags.error(field.span, format!("{t} has no fields"));
                        err(span)
                    }
                }
            }
            AK::Cast(x, t) => {
                let target = self.resolve_type(s.module, t);
                let x = self.expr(s, x, None);
                self.cast(s, x, target, span, true)
            }
            AK::StructLit(path, fields) => {
                let Some(def) = self.resolve_path(s.module, path, true) else {
                    return err(span);
                };
                let Def::Struct(sid) = def else {
                    self.diags
                        .error(path.span, format!("'{}' is not a struct", path.text()));
                    return err(span);
                };
                self.ensure(def);
                let defs = self.types.structs[sid.0 as usize].fields.clone();
                let mut values: Vec<Option<Expr>> = vec![None; defs.len()];
                for (name, value) in fields {
                    match defs.iter().position(|f| f.0 == name.name) {
                        Some(i) => {
                            self.refs.push((name.span, Def::Field(sid, i as u32)));
                            let v = self.expr(s, value, Some(&defs[i].1));
                            let v = self.coerce(s, v, &defs[i].1);
                            if values[i].replace(v).is_some() {
                                self.diags
                                    .error(name.span, format!("Field '{}' is repeated", name.name));
                            }
                        }
                        None => self
                            .diags
                            .error(name.span, format!("No field '{}'", name.name)),
                    }
                }
                let missing: Vec<String> = defs
                    .iter()
                    .zip(&values)
                    .filter(|(_, v)| v.is_none())
                    .map(|(f, _)| f.0.clone())
                    .collect();
                if !missing.is_empty() {
                    self.diags
                        .error(span, format!("Missing fields: {}", missing.join(", ")));
                    return err(span);
                }
                let values: Vec<Expr> = values.into_iter().map(Option::unwrap).collect();
                let ty = Type::Struct(sid);
                if values.iter().all(|v| matches!(v.kind, ExprKind::Const(_))) {
                    let v = Value::aggregate(
                        values
                            .iter()
                            .map(|v| match &v.kind {
                                ExprKind::Const(x) => x.clone(),
                                _ => unreachable!(),
                            })
                            .collect(),
                    );
                    return Expr::new(ExprKind::Const(v), ty, span);
                }
                Expr::new(ExprKind::Struct(values), ty, span)
            }
            AK::Array(items) => {
                if items.is_empty() {
                    self.diags.error(span, "Empty arrays are not supported");
                    return err(span);
                }
                let elem_expected = match expected {
                    Some(Type::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let mut out = Vec::new();
                for i in items {
                    out.push(self.expr(s, i, elem_expected.as_ref()));
                }
                let elem = match &elem_expected {
                    Some(t) => t.clone(),
                    None => {
                        let first = out
                            .iter()
                            .find(|x| x.ty != Type::Int)
                            .map(|x| x.ty.clone())
                            .unwrap_or(Type::Int);
                        first
                    }
                };
                let out: Vec<Expr> = out.into_iter().map(|x| self.coerce(s, x, &elem)).collect();
                let n = out.len() as u32;
                let ty = Type::Array(Box::new(elem), n);
                if out.iter().all(|v| matches!(v.kind, ExprKind::Const(_))) {
                    let v = Value::aggregate(
                        out.into_iter()
                            .map(|v| match v.kind {
                                ExprKind::Const(x) => x,
                                _ => unreachable!(),
                            })
                            .collect(),
                    );
                    return Expr::new(ExprKind::Const(v), ty, span);
                }
                Expr::new(ExprKind::Array(out), ty, span)
            }
            AK::Repeat(x, count) => {
                let elem_expected = match expected {
                    Some(Type::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let x = self.expr(s, x, elem_expected.as_ref());
                let elem = elem_expected.unwrap_or_else(|| x.ty.clone());
                let x = self.coerce(s, x, &elem);
                let Some(n) = self.const_int(s.module, count) else {
                    return err(span);
                };
                if !(1..=65535).contains(&n) {
                    self.diags
                        .error(count.span, "Repeat count must be between 1 and 65535");
                    return err(span);
                }
                let ty = Type::Array(Box::new(elem), n as u32);
                if let ExprKind::Const(v) = &x.kind {
                    return Expr::new(
                        ExprKind::Const(Value::aggregate(vec![v.clone(); n as usize])),
                        ty,
                        span,
                    );
                }
                Expr::new(ExprKind::Repeat(Box::new(x), n as u32), ty, span)
            }
            AK::Match(scrutinee, arms) => {
                let (x, arms_out) = self.match_arms(s, scrutinee, arms, span, expected);
                match arms_out {
                    Some(arms) => {
                        let ty = self.unify_arms(s, arms, expected, span);
                        match ty {
                            Ok((arms, t)) => Expr::new(ExprKind::Match(Box::new(x), arms), t, span),
                            Err(()) => err(span),
                        }
                    }
                    None => err(span),
                }
            }
            AK::If(c, a, b) => {
                let c = self.expr(s, c, Some(&Type::Bool));
                let c = self.coerce(s, c, &Type::Bool);
                let mark = (s.facts.len(), s.slice_facts.len());
                let target = expected.filter(|t| **t != Type::Void).cloned();
                self.facts_from(s, &c, true);
                let mut a = self.expr(s, a, expected);
                if let Some(t) = &target {
                    a = self.coerce(s, a, t);
                }
                s.facts.truncate(mark.0);
                s.slice_facts.truncate(mark.1);
                self.facts_from(s, &c, false);
                let mut b = self.expr(s, b, expected);
                if let Some(t) = &target {
                    b = self.coerce(s, b, t);
                }
                s.facts.truncate(mark.0);
                s.slice_facts.truncate(mark.1);
                let (a, b, t) = if let Some(t) = expected.filter(|t| **t != Type::Void) {
                    let a = self.coerce(s, a, t);
                    let b = self.coerce(s, b, t);
                    (a, b, t.clone())
                } else if a.ty == b.ty && !self.types.is_integer(&a.ty) {
                    let t = a.ty.clone();
                    (a, b, t)
                } else {
                    self.unify(s, a, b, span)
                };
                if let Some(n) = c.constant() {
                    return if n != 0 { a } else { b };
                }
                Expr::new(ExprKind::If(Box::new(c), Box::new(a), Box::new(b)), t, span)
            }
            AK::Asm(inputs, body, body_span) => self.asm(s, inputs, body, *body_span, span),
            AK::Range(..) => {
                self.diags.error(
                    span,
                    "A range is only valid in 'for' loops, patterns and asset()",
                );
                err(span)
            }
        }
    }

    fn path_expr(&mut self, s: &mut Scope, p: &ast::Path, expected: Option<&Type>) -> Expr {
        let span = p.span;
        if p.is_single() {
            if let Some(id) = s.lookup(&p.segments[0].name) {
                if let Some(f) = s.fn_id {
                    self.refs.push((span, Def::Local(f, id)));
                }
                let t = s.locals[id.0 as usize].ty.clone();
                return Expr::new(ExprKind::Local(id), t, span);
            }
            // A bare variant name where the enum is expected.
            if let Some(Type::Enum(e)) = expected {
                let found = self.types.enums[e.0 as usize]
                    .variants
                    .iter()
                    .position(|v| v.0 == p.segments[0].name);
                if self.lookup(s.module, &p.segments[0].name).is_none() {
                    if let Some(i) = found {
                        self.refs.push((span, Def::Variant(*e, i as u32)));
                        let v = self.types.enums[e.0 as usize].variants[i].1;
                        return int(v, Type::Enum(*e), span);
                    }
                }
            }
        }
        let Some(def) = self.resolve_path(s.module, p, true) else {
            return err(span);
        };
        match def {
            Def::Global(g) => {
                self.ensure(def);
                let gl = &self.globals[g.0 as usize];
                let t = gl.ty.clone();
                if gl.is_const() && self.types.is_scalar(&t) {
                    if let Value::Int(n) = gl.value {
                        return int(n, t, span);
                    }
                }
                if gl.is_const() {
                    if let (Type::Struct(_), Value::Aggregate(_)) = (&t, &gl.value) {
                        return Expr::new(ExprKind::Const(gl.value.clone()), t, span);
                    }
                }
                Expr::new(ExprKind::Global(g), t, span)
            }
            Def::Variant(e, i) => {
                let v = self.types.enums[e.0 as usize].variants[i as usize].1;
                int(v, Type::Enum(e), span)
            }
            Def::Fn(_) => {
                self.diags
                    .error(span, "Functions are not values; call it with (...)");
                err(span)
            }
            _ => {
                self.diags
                    .error(span, format!("'{}' is not a value", p.text()));
                err(span)
            }
        }
    }

    fn binary(
        &mut self,
        s: &mut Scope,
        op: &str,
        a: &ast::Expr,
        b: &ast::Expr,
        expected: Option<&Type>,
        span: Span,
    ) -> Expr {
        let op = BinOp::parse(op).unwrap();
        if op.is_logic() {
            let x = self.expr(s, a, Some(&Type::Bool));
            let x = self.coerce(s, x, &Type::Bool);
            // `a && b`: facts of `a` hold while checking `b`.
            let mark = (s.facts.len(), s.slice_facts.len());
            if op == BinOp::LogicAnd {
                self.facts_from(s, &x, true);
            } else {
                self.facts_from(s, &x, false);
            }
            let y = self.expr(s, b, Some(&Type::Bool));
            s.facts.truncate(mark.0);
            s.slice_facts.truncate(mark.1);
            let y = self.coerce(s, y, &Type::Bool);
            if let (Some(p), Some(q)) = (x.constant(), y.constant()) {
                return int(self.fold(op, p, q, &Type::Bool).unwrap(), Type::Bool, span);
            }
            return Expr::new(
                ExprKind::Binary(op, Box::new(x), Box::new(y)),
                Type::Bool,
                span,
            );
        }
        let arith_expected = if op.is_compare() { None } else { expected };
        let x = self.expr(s, a, arith_expected);
        let hint = if x.ty != Type::Int && self.types.is_integer(&x.ty) {
            Some(match &x.ty {
                Type::Range(_) => self.types.base(&x.ty),
                t => t.clone(),
            })
        } else if op.is_compare() && x.ty != Type::Int {
            Some(x.ty.clone())
        } else {
            arith_expected.cloned()
        };
        let y = if matches!(op, BinOp::Shl | BinOp::Shr) {
            self.expr(s, b, None)
        } else {
            self.expr(s, b, hint.as_ref())
        };
        if x.ty.is_error() || y.ty.is_error() {
            return err(span);
        }
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            if !self.types.is_integer(&x.ty) || !self.types.is_integer(&y.ty) {
                self.diags.error(span, "Shifts need integer operands");
                return err(span);
            }
            let t = match &x.ty {
                Type::Range(_) => self.types.base(&x.ty),
                Type::Int if expected.is_some_and(|t| self.types.is_integer(t)) => {
                    match expected.unwrap() {
                        Type::Range(_) => self.types.base(expected.unwrap()),
                        t => t.clone(),
                    }
                }
                t => t.clone(),
            };
            let x = self.coerce(s, x, &t);
            let y = match &y.ty {
                Type::Int => {
                    let n = y.constant().unwrap_or(0);
                    if n < 0 {
                        self.diags.error(y.span, "Negative shift");
                    }
                    int(n.clamp(0, 255), Type::U8, y.span)
                }
                t if self.types.size(t) == 1 => {
                    let t = self.types.base(t);
                    if t == Type::U8 {
                        y
                    } else {
                        Expr::new(ExprKind::Cast(Box::new(y)), Type::U8, span)
                    }
                }
                _ => Expr::new(ExprKind::Cast(Box::new(y)), Type::U8, span),
            };
            if let (Some(p), Some(q)) = (x.constant(), y.constant()) {
                return int(self.fold(op, p, q, &t).unwrap(), t, span);
            }
            return Expr::new(ExprKind::Binary(op, Box::new(x), Box::new(y)), t, span);
        }
        // Equality on enums and bools; ordering on enums.
        if op.is_compare() && x.ty == y.ty && matches!(x.ty, Type::Enum(_) | Type::Bool) {
            if x.ty == Type::Bool && !matches!(op, BinOp::Eq | BinOp::Ne) {
                self.diags.error(span, "Booleans support only == and !=");
            }
            if let (Some(p), Some(q)) = (x.constant(), y.constant()) {
                return int(self.fold(op, p, q, &Type::Int).unwrap(), Type::Bool, span);
            }
            return Expr::new(
                ExprKind::Binary(op, Box::new(x), Box::new(y)),
                Type::Bool,
                span,
            );
        }
        if x.ty == Type::Bool
            && y.ty == Type::Bool
            && matches!(op, BinOp::And | BinOp::Or | BinOp::Xor)
        {
            if let (Some(p), Some(q)) = (x.constant(), y.constant()) {
                return int(self.fold(op, p, q, &Type::U8).unwrap(), Type::Bool, span);
            }
            return Expr::new(
                ExprKind::Binary(op, Box::new(x), Box::new(y)),
                Type::Bool,
                span,
            );
        }
        for v in [&x, &y] {
            if !self.types.is_integer(&v.ty) {
                let t = self.ty_name(&v.ty);
                self.diags.error(
                    v.span,
                    format!("'{}' needs integer operands, found {t}", op.symbol()),
                );
                return err(span);
            }
        }
        let (x, y, t) = self.unify(s, x, y, span);
        if t.is_error() {
            return err(span);
        }
        if matches!(op, BinOp::Div | BinOp::Rem) && y.constant() == Some(0) {
            self.diags.error(y.span, "Division by zero");
            return err(span);
        }
        let rt = if op.is_compare() {
            Type::Bool
        } else {
            t.clone()
        };
        if let (Some(p), Some(q)) = (x.constant(), y.constant()) {
            if let Some(v) = self.fold(op, p, q, &t) {
                return int(v, rt, span);
            }
        }
        Expr::new(ExprKind::Binary(op, Box::new(x), Box::new(y)), rt, span)
    }

    fn cast(&mut self, s: &mut Scope, x: Expr, target: Type, span: Span, unchecked: bool) -> Expr {
        if x.ty.is_error() || target.is_error() {
            return err(span);
        }
        if !self.types.is_scalar(&x.ty) || !self.types.is_scalar(&target) {
            self.diags.error(
                span,
                format!(
                    "Cannot convert {} to {}",
                    self.ty_name(&x.ty),
                    self.ty_name(&target)
                ),
            );
            return err(span);
        }
        if target == Type::Bool {
            if x.ty == Type::Bool {
                return x;
            }
            let t = if x.ty == Type::Int {
                Type::U16
            } else {
                self.types.base(&x.ty)
            };
            if let Some(n) = x.constant() {
                return int((n != 0) as i64, Type::Bool, span);
            }
            let x = self.coerce(s, x, &t);
            let zero = int(0, t, span);
            return Expr::new(
                ExprKind::Binary(BinOp::Ne, Box::new(x), Box::new(zero)),
                Type::Bool,
                span,
            );
        }
        if !unchecked {
            if let Type::Range(_) = target {
                let x = if x.ty == Type::Int {
                    x
                } else if self.types.base(&x.ty) != self.types.base(&target) {
                    let b = self.types.base(&target);
                    self.cast(s, x, b, span, true)
                } else {
                    x
                };
                return self.coerce(s, x, &target);
            }
        }
        if let Some(n) = x.constant() {
            let v = self.types.wrap(&target, n);
            if let Type::Enum(e) = &target {
                if !self.types.enums[e.0 as usize]
                    .variants
                    .iter()
                    .any(|v2| v2.1 == v)
                {
                    self.diags.error(
                        span,
                        format!("{v} is not a value of {}", self.ty_name(&target)),
                    );
                }
            }
            return int(v, target, span);
        }
        if x.ty == target {
            return x;
        }
        let x = if x.ty == Type::Int {
            let t = self.default_type(&x);
            self.coerce(s, x, &t)
        } else {
            x
        };
        Expr::new(ExprKind::Cast(Box::new(x)), target, span)
    }

    fn index(&mut self, s: &mut Scope, b: Expr, i: Expr, raw: bool, span: Span) -> Expr {
        if b.ty.is_error() || i.ty.is_error() {
            return err(span);
        }
        let (elem, len) = match &b.ty {
            Type::Array(e, n) => ((**e).clone(), Some(*n)),
            Type::Slice(e) => ((**e).clone(), None),
            t => {
                let t = self.ty_name(t);
                self.diags.error(b.span, format!("Cannot index {t}"));
                return err(span);
            }
        };
        if !self.types.is_scalar(&i.ty) || i.ty == Type::Bool {
            self.diags.error(i.span, "Index must be an integer or enum");
            return err(span);
        }
        let i = if i.ty == Type::Int {
            let n = i.constant().unwrap_or(0);
            let t = if (0..=255).contains(&n) {
                Type::U8
            } else {
                Type::U16
            };
            if n < 0 {
                self.diags.error(i.span, "Negative index");
                return err(span);
            }
            self.coerce(s, i, &t)
        } else if self.types.signed(&i.ty) {
            self.diags.error(
                i.span,
                format!("Index must be unsigned; found {}", self.ty_name(&i.ty)),
            );
            return err(span);
        } else {
            i
        };
        let (lo, hi) = self.interval(s, &i);
        match len {
            Some(n) => {
                if let Some(k) = i.constant() {
                    if k >= n as i64 {
                        self.diags
                            .error(i.span, format!("Index {k} is out of bounds for length {n}"));
                        return err(span);
                    }
                } else if !raw && (lo < 0 || hi >= n as i64) {
                    self.diags.error(
                        i.span,
                        format!(
                            "Cannot prove index is below {n}; its bounds are {lo}..={hi}. Use a guard, a mask, a range type, or 'raw' to index unchecked"
                        ),
                    );
                }
            }
            None => {
                let proven = match (&i.kind, &b.kind) {
                    (ExprKind::Local(ix), ExprKind::Local(sl)) => {
                        s.slice_facts.contains(&(*ix, *sl))
                    }
                    _ => false,
                };
                if !raw && !proven {
                    self.diags.error(
                        i.span,
                        "Cannot prove index is below the slice length; loop over 0..len(slice), guard with 'if i < len(slice)', or use 'raw'",
                    );
                }
            }
        }
        if let (ExprKind::Const(v), Some(k)) = (&b.kind, i.constant()) {
            if let Some(x) = v.items().get(k as usize) {
                return Expr::new(ExprKind::Const(x.clone()), elem, span);
            }
        }
        Expr::new(ExprKind::Index(Box::new(b), Box::new(i), raw), elem, span)
    }

    fn call(
        &mut self,
        s: &mut Scope,
        callee: &ast::Expr,
        args: &[ast::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Expr {
        let AK::Path(path) = &callee.kind else {
            self.diags
                .error(callee.span, "Only named functions can be called");
            return err(span);
        };
        if path.is_single() && s.lookup(&path.segments[0].name).is_some() {
            self.diags.error(callee.span, "Locals cannot be called");
            return err(span);
        }
        if path.is_single() && path.segments[0].name == "defined" {
            // Resolved without reporting errors.
            let ok = match args.first().map(|a| &a.kind) {
                Some(AK::Path(p)) => {
                    let before = self.refs.len();
                    let r = self.resolve_path(s.module, p, false).is_some();
                    if !r {
                        self.refs.truncate(before);
                    }
                    r
                }
                _ => {
                    self.diags.error(span, "defined() takes a path");
                    false
                }
            };
            return int(ok as i64, Type::Bool, span);
        }
        let Some(def) = self.resolve_path(s.module, path, true) else {
            for a in args {
                self.expr(s, a, None);
            }
            return err(span);
        };
        let arity = |c: &mut Self, n: usize| {
            if args.len() != n {
                c.diags.error(
                    span,
                    format!("Expected {n} argument(s), found {}", args.len()),
                );
                false
            } else {
                true
            }
        };
        match def {
            Def::Builtin(name) => match name {
                "u8" | "i8" | "u16" | "i16" | "bool" => {
                    if !arity(self, 1) {
                        return err(span);
                    }
                    let t = match name {
                        "u8" => Type::U8,
                        "i8" => Type::I8,
                        "u16" => Type::U16,
                        "i16" => Type::I16,
                        _ => Type::Bool,
                    };
                    let x = self.expr(s, &args[0], None);
                    self.cast(s, x, t, span, true)
                }
                "lo" | "hi" => {
                    if !arity(self, 1) {
                        return err(span);
                    }
                    let x = self.expr(s, &args[0], Some(&Type::U16));
                    let t = if x.ty == Type::I16 {
                        Type::I16
                    } else {
                        Type::U16
                    };
                    let x = if self.types.size(&x.ty) == 1 && x.ty != Type::Int {
                        self.cast(s, x, Type::U16, span, true)
                    } else {
                        self.coerce(s, x, &t)
                    };
                    if let Some(n) = x.constant() {
                        let v = if name == "lo" {
                            n & 255
                        } else {
                            (n >> 8) & 255
                        };
                        return int(v, Type::U8, span);
                    }
                    let i = if name == "lo" {
                        Intrinsic::Lo
                    } else {
                        Intrinsic::Hi
                    };
                    Expr::new(ExprKind::Intrinsic(i, vec![x]), Type::U8, span)
                }
                "len" => {
                    if !arity(self, 1) {
                        return err(span);
                    }
                    let x = self.expr(s, &args[0], None);
                    match &x.ty {
                        Type::Array(_, n) => int(*n as i64, Type::Int, span),
                        Type::Slice(_) => {
                            if let ExprKind::SliceOf(g) = &x.kind {
                                if let Type::Array(_, n) = &self.globals[g.0 as usize].ty {
                                    return int(*n as i64, Type::Int, span);
                                }
                            }
                            Expr::new(
                                ExprKind::Intrinsic(Intrinsic::SliceLen, vec![x]),
                                Type::U16,
                                span,
                            )
                        }
                        Type::Error => err(span),
                        t => {
                            let t = self.ty_name(t);
                            self.diags
                                .error(span, format!("len() needs an array or slice, found {t}"));
                            err(span)
                        }
                    }
                }
                "peek" => {
                    if !arity(self, 1) {
                        return err(span);
                    }
                    s.has_asm = true;
                    let a = self.expr(s, &args[0], Some(&Type::U16));
                    let a = self.coerce(s, a, &Type::U16);
                    Expr::new(
                        ExprKind::Intrinsic(Intrinsic::Peek, vec![a]),
                        Type::U8,
                        span,
                    )
                }
                "poke" => {
                    if !arity(self, 2) {
                        return err(span);
                    }
                    s.has_asm = true;
                    let a = self.expr(s, &args[0], Some(&Type::U16));
                    let a = self.coerce(s, a, &Type::U16);
                    let v = self.expr(s, &args[1], Some(&Type::U8));
                    let v = if self.types.size(&v.ty) == 1 && v.ty != Type::Int && v.ty != Type::U8
                    {
                        self.cast(s, v, Type::U8, span, true)
                    } else {
                        self.coerce(s, v, &Type::U8)
                    };
                    Expr::new(
                        ExprKind::Intrinsic(Intrinsic::Poke, vec![a, v]),
                        Type::Void,
                        span,
                    )
                }
                "asset" => self.asset(s, args, expected, span),
                "addr" => {
                    if !arity(self, 1) {
                        return err(span);
                    }
                    let x = self.expr(s, &args[0], None);
                    match (&x.kind, &x.ty) {
                        (_, Type::Slice(_)) | (ExprKind::Global(_), _) => Expr::new(
                            ExprKind::Intrinsic(Intrinsic::Addr, vec![x]),
                            Type::U16,
                            span,
                        ),
                        _ if x.ty.is_error() => err(span),
                        _ => {
                            self.diags.error(span, "addr() takes a global or a slice");
                            err(span)
                        }
                    }
                }
                _ => {
                    self.diags.error(span, format!("'{name}' cannot be called"));
                    err(span)
                }
            },
            Def::Range(r) => {
                self.ensure(def);
                if !arity(self, 1) {
                    return err(span);
                }
                let x = self.expr(s, &args[0], None);
                self.cast(s, x, Type::Range(r), span, false)
            }
            Def::Enum(_) => {
                self.diags
                    .error(span, "Convert integers to enums with 'x as Enum'");
                err(span)
            }
            Def::Fn(f) => {
                let (params, result) = self.signature(f);
                if args.len() != params.len() {
                    self.diags.error(
                        span,
                        format!(
                            "'{}' takes {} argument(s), found {}",
                            path.text(),
                            params.len(),
                            args.len()
                        ),
                    );
                    return err(span);
                }
                let mut out = Vec::new();
                for (a, (_, t, _)) in args.iter().zip(&params) {
                    let x = self.expr(s, a, Some(t));
                    out.push(self.coerce(s, x, t));
                }
                self.kill(s, Key::AllGlobals);
                Expr::new(ExprKind::Call(f, out), result, span)
            }
            _ => {
                self.diags
                    .error(callee.span, format!("'{}' is not a function", path.text()));
                err(span)
            }
        }
    }

    fn asset(
        &mut self,
        s: &mut Scope,
        args: &[ast::Expr],
        expected: Option<&Type>,
        span: Span,
    ) -> Expr {
        let path = match args.first().map(|a| &a.kind) {
            Some(AK::Str(p)) => String::from_utf8_lossy(p).to_string(),
            _ => {
                self.diags.error(span, "asset() needs a path string");
                return err(span);
            }
        };
        let Some(bytes) = self.asset_bytes(&path) else {
            self.diags
                .error(args[0].span, format!("Missing asset '{path}'"));
            return err(span);
        };
        let (start, end) = match args.get(1) {
            None => (0, bytes.len() as i64),
            Some(ast::Expr {
                kind: AK::Range(a, b, inclusive),
                ..
            }) => {
                let a = self.const_int(s.module, a).unwrap_or(0);
                let b = self.const_int(s.module, b).unwrap_or(0) + *inclusive as i64;
                (a, b)
            }
            Some(e) => {
                // A start offset; the length comes from the expected type.
                let a = self.const_int(s.module, e).unwrap_or(0);
                let n = match expected {
                    Some(t) => self.types.size(t) as i64,
                    None => bytes.len() as i64 - a,
                };
                (a, a + n)
            }
        };
        if start < 0 || end > bytes.len() as i64 || start >= end {
            self.diags.error(
                span,
                format!(
                    "Range {start}..{end} is outside asset '{path}' ({} bytes)",
                    bytes.len()
                ),
            );
            return err(span);
        }
        let data = &bytes[start as usize..end as usize];
        // Decode to the expected element type when it is wider than a byte.
        if let Some(t) = expected {
            if let Type::Array(elem, n) = t {
                let size = self.types.size(elem);
                if self.types.is_scalar(elem) && size == 2 && data.len() == *n as usize * 2 {
                    let signed = self.types.signed(elem);
                    let v: Vec<Value> = data
                        .chunks(2)
                        .map(|c| {
                            let w = c[0] as i64 | (c[1] as i64) << 8;
                            Value::Int(if signed { w as i16 as i64 } else { w })
                        })
                        .collect();
                    return Expr::new(ExprKind::Const(Value::aggregate(v)), t.clone(), span);
                }
                if self.types.is_scalar(elem) && size == 1 && data.len() == *n as usize {
                    let signed = self.types.signed(elem);
                    let v: Vec<Value> = data
                        .iter()
                        .map(|b| Value::Int(if signed { *b as i8 as i64 } else { *b as i64 }))
                        .collect();
                    return Expr::new(ExprKind::Const(Value::aggregate(v)), t.clone(), span);
                }
                if data.len() != self.types.size(t) {
                    self.diags.error(
                        span,
                        format!(
                            "Asset range has {} bytes; {} needs {}",
                            data.len(),
                            self.ty_name(t),
                            self.types.size(t)
                        ),
                    );
                    return err(span);
                }
            }
        }
        let v = Value::aggregate(data.iter().map(|b| Value::Int(*b as i64)).collect());
        Expr::new(
            ExprKind::Const(v),
            Type::Array(Box::new(Type::U8), data.len() as u32),
            span,
        )
    }

    fn asm(
        &mut self,
        s: &mut Scope,
        inputs: &[ast::AsmInput],
        body: &str,
        body_span: Span,
        span: Span,
    ) -> Expr {
        s.has_asm = true;
        let mut out_inputs = Vec::new();
        for i in inputs {
            let r = match i.register.name.as_str() {
                "a" => 'a',
                "x" => 'x',
                "y" => 'y',
                _ => {
                    self.diags
                        .error(i.register.span, "Assembly inputs bind a, x or y");
                    continue;
                }
            };
            let v = self.expr(s, &i.value, Some(&Type::U8));
            let v = if v.ty == Type::Int {
                self.coerce(s, v, &Type::U8)
            } else if self.types.size(&v.ty) == 1 {
                if v.ty == Type::U8 {
                    v
                } else {
                    Expr::new(ExprKind::Cast(Box::new(v)), Type::U8, span)
                }
            } else {
                self.diags.error(v.span, "Assembly inputs are bytes");
                v
            };
            out_inputs.push((r, v));
        }
        // Resolve `{name}` placeholders.
        let mut text = String::new();
        let mut refs = Vec::new();
        let mut rest = body;
        let mut offset = body_span.start as usize + 1;
        while let Some(open) = rest.find('{') {
            text.push_str(&rest[..open]);
            let Some(close) = rest[open..].find('}') else {
                text.push_str(&rest[open..]);
                rest = "";
                break;
            };
            let name = rest[open + 1..open + close].trim();
            let nspan = Span::new(body_span.file, offset + open + 1, offset + open + close);
            let segments: Vec<ast::Ident> = name
                .split("::")
                .map(|n| ast::Ident {
                    name: n.trim().to_owned(),
                    span: nspan,
                })
                .collect();
            let r = if segments.len() == 1 && s.lookup(name).is_some() {
                let id = s.lookup(name).unwrap();
                if let Some(f) = s.fn_id {
                    self.refs.push((nspan, Def::Local(f, id)));
                }
                Some(AsmRef::Local(id))
            } else {
                let path = ast::Path {
                    segments,
                    span: nspan,
                };
                match self.resolve_path(s.module, &path, true) {
                    Some(d @ Def::Global(g)) => {
                        self.ensure(d);
                        let gl = &self.globals[g.0 as usize];
                        match (&gl.kind, &gl.value) {
                            (GlobalKind::Const, Value::Int(n)) if self.types.is_scalar(&gl.ty) => {
                                Some(AsmRef::Const(*n))
                            }
                            _ => Some(AsmRef::Global(g)),
                        }
                    }
                    Some(Def::Fn(f)) => Some(AsmRef::Fn(f)),
                    Some(Def::Variant(e, i)) => Some(AsmRef::Const(
                        self.types.enums[e.0 as usize].variants[i as usize].1,
                    )),
                    Some(_) => {
                        self.diags
                            .error(nspan, format!("'{name}' cannot be used in assembly"));
                        None
                    }
                    None => None,
                }
            };
            if let Some(r) = r {
                text.push_str(&format!("{{{}}}", refs.len()));
                refs.push(r);
            }
            offset += open + close + 1;
            rest = &rest[open + close + 1..];
        }
        text.push_str(rest);
        self.kill(s, Key::AllGlobals);
        for r in &refs {
            if let AsmRef::Local(id) = r {
                self.kill(s, Key::Local(*id));
            }
        }
        if let Err((line, message)) = crate::asm::validate(&text) {
            self.diags
                .error(body_span, format!("Assembly line {line}: {message}"));
        }
        Expr::new(
            ExprKind::Asm(Box::new(AsmBlock {
                body: text,
                inputs: out_inputs,
                refs,
                span,
            })),
            Type::U8,
            span,
        )
    }

    // ---------------------------------------------------------------- match

    fn pattern(&mut self, s: &mut Scope, p: &ast::Pattern, t: &Type) -> Option<Pattern> {
        let value = |c: &mut Self, s: &mut Scope, e: &ast::Expr| -> Option<i64> {
            let x = c.expr(s, e, Some(t));
            if x.ty.is_error() {
                return None;
            }
            let x = c.coerce(s, x, t);
            match x.constant() {
                Some(n) => Some(n),
                None if x.ty.is_error() => None,
                None => {
                    c.diags.error(e.span, "Patterns must be constants");
                    None
                }
            }
        };
        match p {
            ast::Pattern::Wildcard(_) => None,
            ast::Pattern::Value(e) => value(self, s, e).map(|n| (n, n)),
            ast::Pattern::Range(a, b, inclusive) => {
                let lo = value(self, s, a)?;
                let hi = value(self, s, b)? - (!inclusive) as i64;
                if lo > hi {
                    self.diags.error(a.span.to(b.span), "Empty pattern range");
                    return None;
                }
                Some((lo, hi))
            }
        }
    }

    /// Checks arms; bodies are checked by `body` with each arm's facts in scope.
    fn match_arms(
        &mut self,
        s: &mut Scope,
        scrutinee: &ast::Expr,
        arms: &[ast::Arm],
        span: Span,
        expected: Option<&Type>,
    ) -> (Expr, Option<Vec<Arm<Expr>>>) {
        let x = self.expr(s, scrutinee, None);
        if x.ty.is_error() {
            return (x, None);
        }
        let x = if x.ty == Type::Int {
            let t = self.default_type(&x);
            self.coerce(s, x, &t)
        } else {
            x
        };
        if !self.types.is_scalar(&x.ty) {
            let t = self.ty_name(&x.ty);
            self.diags.error(x.span, format!("Cannot match on {t}"));
            return (x, None);
        }
        let t = x.ty.clone();
        let mut out = Vec::new();
        let mut covered: Vec<(i64, i64)> = Vec::new();
        let mut wildcard = false;
        for arm in arms {
            let mut pats = Vec::new();
            let mut any = false;
            for p in &arm.patterns {
                match (p, self.pattern(s, p, &t)) {
                    (ast::Pattern::Wildcard(_), _) => any = true,
                    (_, Some(r)) => pats.push(r),
                    _ => {}
                }
            }
            if any {
                pats.clear();
            }
            if wildcard || (!pats.is_empty() && pats.iter().all(|p| covers(&covered, *p))) {
                self.diags.warning(arm.span, "Unreachable match arm");
            }
            let guard = arm.guard.as_ref().map(|g| {
                let g = self.expr(s, g, Some(&Type::Bool));
                self.coerce(s, g, &Type::Bool)
            });
            if guard.is_none() {
                if any {
                    wildcard = true;
                }
                covered.extend(pats.iter().copied());
            }
            let mark = (s.facts.len(), s.slice_facts.len());
            if let ExprKind::Local(id) = &x.kind {
                if !pats.is_empty() {
                    let lo = pats.iter().map(|p| p.0).min().unwrap();
                    let hi = pats.iter().map(|p| p.1).max().unwrap();
                    s.facts.push((Key::Local(*id), lo, hi));
                }
            }
            let body = match &arm.body {
                ArmBody::Expr(e) => self.expr(s, e, expected),
                ArmBody::Block(b) => {
                    self.diags
                        .error(b.span, "Use block arms only in statement matches");
                    err(b.span)
                }
            };
            s.facts.truncate(mark.0);
            s.slice_facts.truncate(mark.1);
            out.push(Arm {
                patterns: pats,
                guard,
                body,
                span: arm.span,
            });
        }
        if !wildcard {
            self.check_exhaustive(&t, &covered, span);
        }
        (x, Some(out))
    }

    fn check_exhaustive(&mut self, t: &Type, covered: &[(i64, i64)], span: Span) {
        let missing: Vec<String> = match t {
            Type::Enum(e) => self.types.enums[e.0 as usize]
                .variants
                .iter()
                .filter(|(_, v)| !covers(covered, (*v, *v)))
                .map(|(n, _)| n.clone())
                .collect(),
            _ => {
                let (lo, hi) = self.types.bounds(t);
                match first_gap(covered, lo, hi) {
                    Some(g) => vec![g.to_string()],
                    None => vec![],
                }
            }
        };
        if !missing.is_empty() {
            self.diags.error(
                span,
                format!(
                    "Match does not cover {}; add the missing cases or '_'",
                    missing.join(", ")
                ),
            );
        }
    }

    fn unify_arms(
        &mut self,
        s: &mut Scope,
        arms: Vec<Arm<Expr>>,
        expected: Option<&Type>,
        span: Span,
    ) -> Result<(Vec<Arm<Expr>>, Type), ()> {
        let t = match expected.filter(|t| **t != Type::Void) {
            Some(t) => t.clone(),
            None => {
                let concrete = arms
                    .iter()
                    .find(|a| a.body.ty != Type::Int && a.body.ty != Type::Never);
                match concrete {
                    Some(a) => match &a.body.ty {
                        Type::Range(_) if arms.iter().any(|b| b.body.ty != a.body.ty) => {
                            self.types.base(&a.body.ty)
                        }
                        t => t.clone(),
                    },
                    None => {
                        let first = arms.first().map(|a| a.body.clone());
                        match first {
                            Some(e) => {
                                let mut t = self.default_type(&e);
                                for a in &arms {
                                    if self.default_type(&a.body) == Type::U16 {
                                        t = Type::U16;
                                    }
                                }
                                t
                            }
                            None => {
                                self.diags.error(span, "A value match needs arms");
                                return Err(());
                            }
                        }
                    }
                }
            }
        };
        let arms = arms
            .into_iter()
            .map(|mut a| {
                a.body = self.coerce(s, a.body, &t);
                a
            })
            .collect();
        Ok((arms, t))
    }

    // ------------------------------------------------------------- facts

    /// Record facts implied by `cond` being `truth`.
    fn facts_from(&mut self, s: &mut Scope, cond: &Expr, truth: bool) {
        match &cond.kind {
            ExprKind::Unary(UnOp::Not, x) => self.facts_from(s, x, !truth),
            ExprKind::Binary(BinOp::LogicAnd, a, b) if truth => {
                self.facts_from(s, a, true);
                self.facts_from(s, b, true);
            }
            ExprKind::Binary(BinOp::LogicOr, a, b) if !truth => {
                self.facts_from(s, a, false);
                self.facts_from(s, b, false);
            }
            ExprKind::Binary(op, a, b) if op.is_compare() => {
                let var = |e: &Expr| {
                    // A range viewed as its base type is the same variable.
                    let e = match &e.kind {
                        ExprKind::Cast(x) if self.types.base(&x.ty) == self.types.base(&e.ty) => {
                            x.as_ref()
                        }
                        _ => e,
                    };
                    match &e.kind {
                        ExprKind::Local(id) => Some(Key::Local(*id)),
                        ExprKind::Global(g) if self.trackable(*g) => Some(Key::Global(*g)),
                        _ => None,
                    }
                };
                let (local, k, op) = match (var(a), var(b)) {
                    (Some(id), _) => (Some(id), b, *op),
                    (_, Some(id)) => (
                        Some(id),
                        a,
                        match op {
                            BinOp::Lt => BinOp::Gt,
                            BinOp::Le => BinOp::Ge,
                            BinOp::Gt => BinOp::Lt,
                            BinOp::Ge => BinOp::Le,
                            o => *o,
                        },
                    ),
                    _ => (None, b, *op),
                };
                let Some(key) = local else { return };
                let op = if truth {
                    op
                } else {
                    match op {
                        BinOp::Lt => BinOp::Ge,
                        BinOp::Le => BinOp::Gt,
                        BinOp::Gt => BinOp::Le,
                        BinOp::Ge => BinOp::Lt,
                        BinOp::Eq => BinOp::Ne,
                        BinOp::Ne => BinOp::Eq,
                        o => o,
                    }
                };
                if let Key::Local(id) = key {
                    if s.immutable(id) {
                        if let ExprKind::Intrinsic(Intrinsic::SliceLen, args) = &k.kind {
                            if let (ExprKind::Local(sl), BinOp::Lt) = (&args[0].kind, op) {
                                s.slice_facts.push((id, *sl));
                            }
                            return;
                        }
                        if let ExprKind::Local(n) = &k.kind {
                            if let Some((_, sl)) = s.len_alias.iter().find(|(a, _)| a == n).copied()
                            {
                                if op == BinOp::Lt {
                                    s.slice_facts.push((id, sl));
                                }
                            }
                        }
                    }
                }
                let (klo, khi) = self.interval(s, k);
                let current = {
                    let e = if var(a).is_some() { a } else { b };
                    self.interval(s, e)
                };
                let fact = match op {
                    BinOp::Ne if klo == khi && klo == current.0 => Some((klo + 1, i64::MAX)),
                    BinOp::Ne if klo == khi && klo == current.1 => Some((i64::MIN, khi - 1)),
                    BinOp::Lt => Some((i64::MIN, khi - 1)),
                    BinOp::Le => Some((i64::MIN, khi)),
                    BinOp::Gt => Some((klo + 1, i64::MAX)),
                    BinOp::Ge => Some((klo, i64::MAX)),
                    BinOp::Eq => Some((klo, khi)),
                    _ => None,
                };
                if let Some((lo, hi)) = fact {
                    s.facts.push((key, lo, hi));
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------- statements

    pub(crate) fn check_body(
        &mut self,
        m: usize,
        id: FnId,
        decl: &ast::FnDecl,
        params: Vec<(String, Type, bool)>,
        result: Type,
        attrs: FnAttrs,
        item: &ast::Item,
    ) -> Function {
        let mut s = Scope::constant(m);
        s.fn_id = Some(id);
        s.result = result.clone();
        let mut ids = Vec::new();
        for (p, (name, t, mutable)) in decl.params.iter().zip(params) {
            if s.blocks[0].iter().any(|(n, _)| *n == name) {
                self.diags
                    .error(p.name.span, format!("Parameter '{name}' is repeated"));
            }
            let l = s.declare(&name, t, mutable, p.name.span);
            self.defs.insert(Def::Local(id, l), p.name.span);
            ids.push(l);
        }
        let body = self.block(&mut s, &decl.body.stmts);
        if result != Type::Void && !result.is_error() && !terminates(&body) {
            self.diags.error(
                decl.body.span,
                format!(
                    "'{}' can reach its end without returning a {}",
                    decl.name.name,
                    self.ty_name(&result)
                ),
            );
        }
        Function {
            name: decl.name.name.clone(),
            path: self.qualify(m, &decl.name.name),
            params: ids,
            result,
            locals: s.locals,
            body,
            attrs,
            span: decl.name.span,
            module: m,
            doc: {
                let _ = item;
                String::new()
            },
            has_asm: s.has_asm,
        }
    }

    /// Check a block and report `probe` evaluated at its end.
    fn block_probe(
        &mut self,
        s: &mut Scope,
        stmts: &[ast::Stmt],
        probe: &dyn Fn(&Self, &Scope) -> Vec<(i64, i64)>,
    ) -> (Vec<Stmt>, Vec<(i64, i64)>) {
        s.blocks.push(Vec::new());
        let mark = (s.facts.len(), s.slice_facts.len());
        let mut out = Vec::new();
        for st in stmts {
            if let Some(x) = self.stmt(s, st) {
                out.push(x);
            }
        }
        let bounds = probe(self, s);
        s.facts.truncate(mark.0);
        s.slice_facts.truncate(mark.1);
        s.blocks.pop();
        (out, bounds)
    }

    fn block(&mut self, s: &mut Scope, stmts: &[ast::Stmt]) -> Vec<Stmt> {
        s.blocks.push(Vec::new());
        let mark = (s.facts.len(), s.slice_facts.len());
        let mut out = Vec::new();
        let mut dead = false;
        for st in stmts {
            if dead {
                self.diags.warning(st.span, "Unreachable statement");
                dead = false;
            }
            let x = self.stmt(s, st);
            if let Some(x) = x {
                if terminates(std::slice::from_ref(&x)) && !matches!(x, Stmt::Loop(_)) {
                    dead = true;
                }
                out.push(x);
            }
        }
        s.facts.truncate(mark.0);
        s.slice_facts.truncate(mark.1);
        s.blocks.pop();
        // What the block assigned is unknown afterwards.
        self.kill_loop(s, stmts, None);
        out
    }

    fn stmt(&mut self, s: &mut Scope, st: &ast::Stmt) -> Option<Stmt> {
        let span = st.span;
        Some(match &st.kind {
            StmtKind::Let {
                name,
                mutable,
                ty,
                value,
            } => {
                let (t, e) = match ty {
                    Some(t) => {
                        let (t, pre) = self.resolve_decl_type(s.module, t, None);
                        let e = match pre {
                            Some(e) => e,
                            None => {
                                let e = self.expr(s, value, Some(&t));
                                self.coerce(s, e, &t)
                            }
                        };
                        (t, e)
                    }
                    None => {
                        let e = self.expr(s, value, None);
                        let t = self.default_type(&e);
                        self.hints.push((name.span, t.clone()));
                        let e = self.coerce(s, e, &t);
                        (t, e)
                    }
                };
                if t == Type::Void {
                    self.diags.error(value.span, "This expression has no value");
                }
                if matches!(t, Type::Slice(_)) && *mutable {
                    self.diags.error(name.span, "Slice bindings are immutable");
                }
                let id = s.declare(&name.name, t, *mutable, name.span);
                if let Some(f) = s.fn_id {
                    self.defs.insert(Def::Local(f, id), name.span);
                }
                {
                    let (lo, hi) = self.interval(s, &e);
                    let full = self.types.bounds(&s.locals[id.0 as usize].ty);
                    if (lo, hi) != full
                        && lo <= hi
                        && self.types.is_scalar(&s.locals[id.0 as usize].ty)
                    {
                        s.facts.push((Key::Local(id), lo, hi));
                    }
                }
                if !*mutable {
                    if let ExprKind::Intrinsic(Intrinsic::SliceLen, args) = &e.kind {
                        if let ExprKind::Local(sl) = &args[0].kind {
                            s.len_alias.push((id, *sl));
                        }
                    }
                }
                Stmt::Let(id, e)
            }
            StmtKind::Assign { target, op, value } => {
                let t = self.expr(s, target, None);
                self.check_place(s, &t);
                let bin = if *op == "=" {
                    None
                } else {
                    BinOp::parse(&op[..op.len() - 1])
                };
                let v = match bin {
                    Some(BinOp::Shl | BinOp::Shr) => {
                        let v = self.expr(s, value, Some(&Type::U8));
                        if v.ty == Type::Int {
                            self.coerce(s, v, &Type::U8)
                        } else if self.types.size(&v.ty) == 1 {
                            v
                        } else {
                            Expr::new(ExprKind::Cast(Box::new(v)), Type::U8, span)
                        }
                    }
                    Some(b) => {
                        let base = match &t.ty {
                            Type::Range(_) => self.types.base(&t.ty),
                            x => x.clone(),
                        };
                        if t.ty == Type::Bool && matches!(b, BinOp::And | BinOp::Or | BinOp::Xor) {
                            let v = self.expr(s, value, Some(&Type::Bool));
                            self.coerce(s, v, &Type::Bool)
                        } else {
                            if !self.types.is_integer(&t.ty) && !t.ty.is_error() {
                                self.diags
                                    .error(span, format!("'{op}' needs an integer target"));
                            }
                            let v = self.expr(s, value, Some(&base));
                            if matches!(t.ty, Type::Range(_)) {
                                let v2 = self.coerce(s, v.clone(), &base);
                                let probe = Expr::new(
                                    ExprKind::Binary(
                                        b,
                                        Box::new(Expr::new(
                                            ExprKind::Cast(Box::new(t.clone())),
                                            base.clone(),
                                            span,
                                        )),
                                        Box::new(v2),
                                    ),
                                    base.clone(),
                                    span,
                                );
                                let (lo, hi) = self.interval(s, &probe);
                                let (tlo, thi) = self.types.bounds(&t.ty);
                                if lo < tlo || hi > thi {
                                    self.diags.error(
                                        span,
                                        format!("Cannot prove the result stays within {} ({tlo}..={thi}); its bounds are {lo}..={hi}", self.ty_name(&t.ty)),
                                    );
                                }
                            }
                            if matches!(b, BinOp::Div | BinOp::Rem) && v.constant() == Some(0) {
                                self.diags.error(v.span, "Division by zero");
                            }
                            self.coerce(s, v, &base)
                        }
                    }
                    None => {
                        let v = self.expr(s, value, Some(&t.ty));
                        let ty = t.ty.clone();
                        self.coerce(s, v, &ty)
                    }
                };
                if bin.is_none() {
                    self.assigned_fact(s, &t, Some(&v));
                } else {
                    let new = bin.map(|b| {
                        Expr::new(
                            ExprKind::Binary(b, Box::new(t.clone()), Box::new(v.clone())),
                            t.ty.clone(),
                            span,
                        )
                    });
                    // Compound assignment: bounds from the old value, when they cannot wrap.
                    let fact = new.as_ref().map(|n| self.interval(s, n));
                    self.assigned_fact(s, &t, None);
                    if let (Some((lo, hi)), ExprKind::Local(_) | ExprKind::Global(_)) =
                        (fact, &t.kind)
                    {
                        let full = self.types.bounds(&t.ty);
                        if (lo, hi) != full
                            && lo <= hi
                            && matches!(
                                bin,
                                Some(BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Shr)
                            )
                        {
                            let key = match &t.kind {
                                ExprKind::Local(id) => Key::Local(*id),
                                ExprKind::Global(g) => Key::Global(*g),
                                _ => unreachable!(),
                            };
                            s.facts.push((key, lo, hi));
                        }
                    }
                }
                Stmt::Assign(t, bin, v, span)
            }
            StmtKind::Expr(e) => {
                if let AK::Match(scrutinee, arms) = &e.kind {
                    if arms.iter().any(|a| matches!(a.body, ArmBody::Block(_))) || arms.is_empty() {
                        return Some(self.match_stmt(s, scrutinee, arms, e.span));
                    }
                }
                let x = self.expr(s, e, Some(&Type::Void));
                match &x.kind {
                    ExprKind::Call(..)
                    | ExprKind::Asm(..)
                    | ExprKind::Intrinsic(..)
                    | ExprKind::Match(..) => {}
                    _ if x.ty.is_error() => {}
                    _ => self.diags.warning(e.span, "This expression has no effect"),
                }
                Stmt::Expr(x)
            }
            StmtKind::If {
                cond,
                then,
                otherwise,
            } => {
                let c = self.expr(s, cond, Some(&Type::Bool));
                let c = self.coerce(s, c, &Type::Bool);
                // Compile-time conditions (such as `defined(...)`) skip the
                // other branch entirely, so it may name missing items.
                match c.constant() {
                    Some(0) => {
                        return Some(Stmt::Block(match otherwise {
                            Some(b) => self.block(s, &b.stmts),
                            None => Vec::new(),
                        }));
                    }
                    Some(_) => return Some(Stmt::Block(self.block(s, &then.stmts))),
                    None => {}
                }
                // Variables whose bounds are merged where the branches join.
                let mut keys: Vec<(Key, Type)> = Vec::new();
                {
                    let mut names = Vec::new();
                    let mut calls = false;
                    for st in then
                        .stmts
                        .iter()
                        .chain(otherwise.iter().flat_map(|b| b.stmts.iter()))
                    {
                        assigned(st, &mut names, &mut calls);
                    }
                    for n in names.into_iter().flatten() {
                        if let Some(id) = s.lookup(&n) {
                            let t = s.locals[id.0 as usize].ty.clone();
                            if self.types.is_scalar(&t)
                                && !keys.iter().any(|(k, _)| *k == Key::Local(id))
                            {
                                keys.push((Key::Local(id), t));
                            }
                        }
                    }
                    let mut vars = Vec::new();
                    cond_vars(&c, &mut vars);
                    for e in vars {
                        let key = match &e.kind {
                            ExprKind::Local(id) => Key::Local(*id),
                            ExprKind::Global(g) if self.trackable(*g) => Key::Global(*g),
                            _ => continue,
                        };
                        if !keys.iter().any(|(k, _)| *k == key) {
                            keys.push((key, e.ty.clone()));
                        }
                    }
                }
                let probe = |c: &Self, s: &Scope| -> Vec<(i64, i64)> {
                    keys.iter()
                        .map(|(k, t)| c.fact_bounds(s, *k, c.types.bounds(t)))
                        .collect()
                };
                let mark = (s.facts.len(), s.slice_facts.len());
                self.facts_from(s, &c, true);
                let (yes, then_bounds) = self.block_probe(s, &then.stmts, &probe);
                s.facts.truncate(mark.0);
                s.slice_facts.truncate(mark.1);
                self.facts_from(s, &c, false);
                let (no, else_bounds) = match otherwise {
                    Some(b) => self.block_probe(s, &b.stmts, &probe),
                    None => (Vec::new(), probe(self, s)),
                };
                s.facts.truncate(mark.0);
                s.slice_facts.truncate(mark.1);
                let yes_exits = terminates(&yes);
                let no_exits = otherwise.is_some() && terminates(&no);
                // Forget what either branch assigned, then merge the bounds.
                self.kill_loop(s, &then.stmts, None);
                if let Some(b) = otherwise {
                    self.kill_loop(s, &b.stmts, None);
                }
                for (i, (k, t)) in keys.iter().enumerate() {
                    let (a, b) = (then_bounds[i], else_bounds[i]);
                    let merged = match (yes_exits, no_exits) {
                        (true, false) => b,
                        (false, true) => a,
                        (true, true) => continue,
                        _ => (a.0.min(b.0), a.1.max(b.1)),
                    };
                    if merged != self.types.bounds(t) {
                        s.facts.push((*k, merged.0, merged.1));
                    }
                }
                // Slice facts from an early exit.
                if yes_exits && !no_exits {
                    let before = s.facts.len();
                    self.facts_from(s, &c, false);
                    s.facts.truncate(before);
                } else if no_exits && !yes_exits {
                    let before = s.facts.len();
                    self.facts_from(s, &c, true);
                    s.facts.truncate(before);
                }
                if let Some(n) = c.constant() {
                    return Some(Stmt::Block(if n != 0 { yes } else { no }));
                }
                Stmt::If(c, yes, no)
            }
            StmtKind::While { cond, body } => {
                self.kill_loop(s, &body.stmts, Some(cond));
                let c = self.expr(s, cond, Some(&Type::Bool));
                let c = self.coerce(s, c, &Type::Bool);
                let mark = (s.facts.len(), s.slice_facts.len());
                self.facts_from(s, &c, true);
                s.loops += 1;
                let b = self.block(s, &body.stmts);
                s.loops -= 1;
                s.facts.truncate(mark.0);
                s.slice_facts.truncate(mark.1);
                Stmt::While(c, b)
            }
            StmtKind::Loop(body) => {
                self.kill_loop(s, &body.stmts, None);
                s.loops += 1;
                let b = self.block(s, &body.stmts);
                s.loops -= 1;
                Stmt::Loop(b)
            }
            StmtKind::For {
                var,
                start,
                end,
                inclusive,
                reverse,
                body,
            } => {
                let a = self.expr(s, start, None);
                let b = self.expr(s, end, None);
                let t = match (&a.ty, &b.ty) {
                    (Type::Int, Type::Int) => {
                        let lo = a.constant().unwrap_or(0).min(b.constant().unwrap_or(0));
                        let hi = a
                            .constant()
                            .unwrap_or(0)
                            .max(b.constant().unwrap_or(0) - !inclusive as i64);
                        if lo >= 0 && hi <= 255 {
                            Type::U8
                        } else if lo >= -128 && hi <= 127 {
                            Type::I8
                        } else if lo >= 0 && hi <= 65535 {
                            Type::U16
                        } else {
                            Type::I16
                        }
                    }
                    (Type::Int, t) | (t, Type::Int) => t.clone(),
                    (x, y) if x == y => x.clone(),
                    (x, y) if self.types.base(x) == self.types.base(y) => self.types.base(x),
                    (x, y) => {
                        let (x, y) = (self.ty_name(x), self.ty_name(y));
                        self.diags.error(
                            span,
                            format!("Loop bounds have different types {x} and {y}"),
                        );
                        Type::Error
                    }
                };
                if !self.types.is_integer(&t) && !matches!(t, Type::Enum(_) | Type::Error) {
                    let n = self.ty_name(&t);
                    self.diags
                        .error(span, format!("Loop bounds must be integers, found {n}"));
                }
                let a = self.coerce(s, a, &t);
                // An exclusive end may be one past the type's maximum.
                let b = match (b.constant(), &b.ty) {
                    (Some(n), Type::Int) if !inclusive && n == self.types.bounds(&t).1 + 1 => {
                        int(n, Type::Int, b.span)
                    }
                    _ => self.coerce(s, b, &t),
                };
                s.blocks.push(Vec::new());
                let id = s.declare(&var.name, t.clone(), false, var.span);
                if let Some(f) = s.fn_id {
                    self.defs.insert(Def::Local(f, id), var.span);
                }
                self.kill_loop(s, &body.stmts, None);
                let mark = (s.facts.len(), s.slice_facts.len());
                let (alo, _) = self.interval(s, &a);
                let (_, bhi) = self.interval(s, &b);
                let (_, ahi) = self.interval(s, &a);
                let (blo, _) = self.interval(s, &b);
                let _ = (ahi, blo);
                s.facts.push((Key::Local(id), alo, bhi - !inclusive as i64));
                if let ExprKind::Intrinsic(Intrinsic::SliceLen, args) = &b.kind {
                    if let (ExprKind::Local(sl), false) = (&args[0].kind, *inclusive) {
                        s.slice_facts.push((id, *sl));
                    }
                }
                if let (ExprKind::Local(n), false) = (&b.kind, *inclusive) {
                    if let Some((_, sl)) = s.len_alias.iter().find(|(a, _)| a == n).copied() {
                        s.slice_facts.push((id, sl));
                    }
                }
                s.loops += 1;
                let body = self.block(s, &body.stmts);
                s.loops -= 1;
                s.facts.truncate(mark.0);
                s.slice_facts.truncate(mark.1);
                s.blocks.pop();
                Stmt::For {
                    var: id,
                    start: a,
                    end: b,
                    inclusive: *inclusive,
                    reverse: *reverse,
                    body,
                }
            }
            StmtKind::Break => {
                if s.loops == 0 {
                    self.diags.error(span, "'break' outside a loop");
                }
                Stmt::Break(span)
            }
            StmtKind::Continue => {
                if s.loops == 0 {
                    self.diags.error(span, "'continue' outside a loop");
                }
                Stmt::Continue(span)
            }
            StmtKind::Return(v) => {
                let r = s.result.clone();
                let v = match v {
                    Some(v) => {
                        if r == Type::Void {
                            self.diags.error(v.span, "This function returns nothing");
                        }
                        let e = self.expr(s, v, Some(&r));
                        Some(self.coerce(s, e, &r))
                    }
                    None => {
                        if r != Type::Void && !r.is_error() {
                            let t = self.ty_name(&r);
                            self.diags
                                .error(span, format!("Expected a {t} return value"));
                        }
                        None
                    }
                };
                Stmt::Return(v, span)
            }
            StmtKind::Block(b) => Stmt::Block(self.block(s, &b.stmts)),
        })
    }

    fn match_stmt(
        &mut self,
        s: &mut Scope,
        scrutinee: &ast::Expr,
        arms: &[ast::Arm],
        span: Span,
    ) -> Stmt {
        let x = self.expr(s, scrutinee, None);
        if x.ty.is_error() {
            return Stmt::Block(Vec::new());
        }
        let x = if x.ty == Type::Int {
            let t = self.default_type(&x);
            self.coerce(s, x, &t)
        } else {
            x
        };
        if !self.types.is_scalar(&x.ty) {
            let t = self.ty_name(&x.ty);
            self.diags.error(x.span, format!("Cannot match on {t}"));
            return Stmt::Block(Vec::new());
        }
        let t = x.ty.clone();
        let mut out = Vec::new();
        let mut covered = Vec::new();
        let mut wildcard = false;
        for arm in arms {
            let mut pats = Vec::new();
            let mut any = false;
            for p in &arm.patterns {
                match (p, self.pattern(s, p, &t)) {
                    (ast::Pattern::Wildcard(_), _) => any = true,
                    (_, Some(r)) => pats.push(r),
                    _ => {}
                }
            }
            if any {
                pats.clear();
            }
            if wildcard || (!pats.is_empty() && pats.iter().all(|p| covers(&covered, *p))) {
                self.diags.warning(arm.span, "Unreachable match arm");
            }
            let guard = arm.guard.as_ref().map(|g| {
                let g = self.expr(s, g, Some(&Type::Bool));
                self.coerce(s, g, &Type::Bool)
            });
            if guard.is_none() {
                if any {
                    wildcard = true;
                }
                covered.extend(pats.iter().copied());
            }
            let mark = (s.facts.len(), s.slice_facts.len());
            if let ExprKind::Local(id) = &x.kind {
                if !pats.is_empty() {
                    let lo = pats.iter().map(|p| p.0).min().unwrap();
                    let hi = pats.iter().map(|p| p.1).max().unwrap();
                    s.facts.push((Key::Local(*id), lo, hi));
                }
            }
            let body = match &arm.body {
                ArmBody::Block(b) => self.block(s, &b.stmts),
                ArmBody::Expr(e) => {
                    let e = self.expr(s, e, Some(&Type::Void));
                    vec![Stmt::Expr(e)]
                }
            };
            s.facts.truncate(mark.0);
            s.slice_facts.truncate(mark.1);
            out.push(Arm {
                patterns: pats,
                guard,
                body,
                span: arm.span,
            });
        }
        if !wildcard {
            self.check_exhaustive(&t, &covered, span);
        }
        Stmt::Match(x, out)
    }

    fn check_place(&mut self, s: &mut Scope, e: &Expr) {
        match &e.kind {
            ExprKind::Local(id) => {
                if !s.locals[id.0 as usize].mutable {
                    let name = s.locals[id.0 as usize].name.clone();
                    self.diags.push(
                        Diagnostic::error(
                            e.span,
                            format!("'{name}' is immutable; declare it with 'var'"),
                        )
                        .note(s.locals[id.0 as usize].span, "declared here"),
                    );
                }
            }
            ExprKind::Global(g) => {
                if self.globals[g.0 as usize].is_const() {
                    self.diags.error(e.span, "Constants cannot be assigned");
                }
            }
            ExprKind::Index(b, _, _) | ExprKind::Field(b, _) => {
                if matches!(b.ty, Type::Slice(_)) {
                    self.diags.error(e.span, "Slices are read-only");
                } else {
                    self.check_place(s, b);
                }
            }
            ExprKind::Const(_) if e.ty.is_error() => {}
            _ => self
                .diags
                .error(e.span, "This expression cannot be assigned"),
        }
    }
}

/// Every pattern value in `p` lies within the union of `covered`.
fn covers(covered: &[(i64, i64)], p: (i64, i64)) -> bool {
    first_gap(covered, p.0, p.1).is_none()
}

fn first_gap(covered: &[(i64, i64)], lo: i64, hi: i64) -> Option<i64> {
    let mut v: Vec<(i64, i64)> = covered.to_vec();
    v.sort();
    let mut next = lo;
    for (a, b) in v {
        if a > next {
            break;
        }
        next = next.max(b + 1);
        if next > hi {
            return None;
        }
    }
    if next > hi {
        None
    } else {
        Some(next)
    }
}

/// The statements never complete normally.
pub fn terminates(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Return(..)) => true,
        Some(Stmt::Loop(b)) => !breaks(b),
        Some(Stmt::If(_, a, b)) => terminates(a) && terminates(b),
        Some(Stmt::Block(b)) => terminates(b),
        Some(Stmt::Match(_, arms)) => {
            !arms.is_empty()
                && arms.iter().all(|a| terminates(&a.body))
                && arms
                    .iter()
                    .any(|a| a.patterns.is_empty() && a.guard.is_none())
                || {
                    // Exhaustive without a wildcard: every arm terminates.
                    !arms.is_empty() && arms.iter().all(|a| terminates(&a.body))
                }
        }
        _ => false,
    }
}

fn breaks(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Break(_) => true,
        Stmt::If(_, a, b) => breaks(a) || breaks(b),
        Stmt::Block(b) => breaks(b),
        Stmt::Match(_, arms) => arms.iter().any(|a| breaks(&a.body)),
        _ => false,
    })
}

#[allow(dead_code)]
fn unused(_: BTreeSet<u8>) {}

/// Variables assigned in a statement (None: a non-local target).
fn assigned(st: &ast::Stmt, out: &mut Vec<Option<String>>, calls: &mut bool) {
    let root = |e: &ast::Expr| -> Option<String> {
        let mut e = e;
        loop {
            match &e.kind {
                AK::Index(b, _, _) | AK::Field(b, _) => e = b,
                AK::Path(p) if p.is_single() => return Some(p.segments[0].name.clone()),
                _ => return None,
            }
        }
    };
    let block = |b: &ast::Block, out: &mut Vec<Option<String>>, calls: &mut bool| {
        for s in &b.stmts {
            assigned(s, out, calls);
        }
    };
    match &st.kind {
        StmtKind::Let { value, .. } => *calls |= has_call(value),
        StmtKind::Assign { target, value, .. } => {
            // Element writes do not change scalar facts.
            if matches!(target.kind, AK::Path(_)) {
                out.push(root(target));
            }
            *calls |= has_call(value) || has_call(target);
        }
        StmtKind::Expr(e) => {
            *calls |= has_call(e);
            if let AK::Match(_, arms) = &e.kind {
                for a in arms {
                    if let ast::ArmBody::Block(b) = &a.body {
                        block(b, out, calls);
                    }
                }
            }
        }
        StmtKind::If {
            cond,
            then,
            otherwise,
        } => {
            *calls |= has_call(cond);
            block(then, out, calls);
            if let Some(o) = otherwise {
                block(o, out, calls);
            }
        }
        StmtKind::While { cond, body } => {
            *calls |= has_call(cond);
            block(body, out, calls);
        }
        StmtKind::For {
            body, start, end, ..
        } => {
            *calls |= has_call(start) || has_call(end);
            block(body, out, calls);
        }
        StmtKind::Loop(b) | StmtKind::Block(b) => block(b, out, calls),
        StmtKind::Return(Some(e)) => *calls |= has_call(e),
        _ => {}
    }
}

fn has_call(e: &ast::Expr) -> bool {
    match &e.kind {
        AK::Call(_, _) | AK::Asm(..) => true,
        AK::Unary(_, a) | AK::Cast(a, _) | AK::Field(a, _) => has_call(a),
        AK::Binary(_, a, b) | AK::Index(a, b, _) | AK::Repeat(a, b) | AK::Range(a, b, _) => {
            has_call(a) || has_call(b)
        }
        AK::StructLit(_, f) => f.iter().any(|(_, v)| has_call(v)),
        AK::Array(v) => v.iter().any(has_call),
        AK::Match(x, arms) => {
            has_call(x)
                || arms.iter().any(|a| match &a.body {
                    ast::ArmBody::Expr(e) => has_call(e),
                    ast::ArmBody::Block(_) => true,
                })
        }
        AK::If(c, a, b) => has_call(c) || has_call(a) || has_call(b),
        _ => false,
    }
}

/// Variables compared in a condition.
fn cond_vars<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match &e.kind {
        ExprKind::Unary(UnOp::Not, x) => cond_vars(x, out),
        ExprKind::Binary(BinOp::LogicAnd | BinOp::LogicOr, a, b) => {
            cond_vars(a, out);
            cond_vars(b, out);
        }
        ExprKind::Binary(op, a, b) if op.is_compare() => {
            for x in [a, b] {
                let x = match &x.kind {
                    ExprKind::Cast(inner) => inner.as_ref(),
                    _ => x.as_ref(),
                };
                if matches!(x.kind, ExprKind::Local(_) | ExprKind::Global(_)) {
                    out.push(x);
                }
            }
        }
        _ => {}
    }
}
