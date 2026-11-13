//! Lowering from the typed program to MIR: scalarization of structs and
//! slices, structure-of-arrays memory, per-context function instances and
//! control flow graphs.
use crate::diag::Diagnostics;
use crate::hir::{self, *};
use crate::mir::*;
use crate::source::Span;
use crate::types::Type;
use std::collections::BTreeMap;

pub fn sc_of(types: &crate::types::TypeTable, t: &Type) -> Sc {
    match types.base(t) {
        Type::I8 => Sc::I8,
        Type::U16 | Type::Int => Sc::U16,
        Type::I16 => Sc::I16,
        _ => Sc::U8,
    }
}

/// Scalar leaf types of a value: struct fields flattened, slices as
/// (pointer, length), arrays by their innermost element.
fn leaves(types: &crate::types::TypeTable, t: &Type) -> Vec<(String, Sc)> {
    match t {
        Type::Array(e, _) => leaves(types, e),
        Type::Slice(_) => vec![(".ptr".into(), Sc::U16), (".len".into(), Sc::U16)],
        Type::Void | Type::Never | Type::Error => vec![],
        t => types
            .leaves(t)
            .into_iter()
            .map(|(n, lt)| (n, sc_of(types, &lt)))
            .collect(),
    }
}

/// Flattened element count of a (possibly nested) array type.
fn flat_len(t: &Type) -> Option<u32> {
    match t {
        Type::Array(e, n) => Some(n * flat_len(e).unwrap_or(1)),
        _ => None,
    }
}

fn raw_index(e: &hir::Expr) -> &bool {
    match &e.kind {
        ExprKind::Index(_, _, raw) => raw,
        _ => &false,
    }
}

fn innermost(t: &Type) -> &Type {
    match t {
        Type::Array(e, _) => innermost(e),
        t => t,
    }
}

fn flat_values(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Aggregate(items) => {
            for i in items.iter() {
                flat_values(i, out);
            }
        }
        v => out.push(v.clone()),
    }
}

#[derive(Clone, Debug)]
enum LocalRepr {
    Scalars(Vec<VarId>),
    Array(Vec<ObjId>),
}

#[derive(Clone, Copy, Debug)]
enum Root {
    Global(GlobalId),
    Local(LocalId),
}

#[derive(Clone, Debug)]
struct PlaceRef {
    root: Root,
    index: Option<Ex>,
    leaf: usize,
    count: usize,
    ty: Type,
}

pub fn build(p: &Program, diags: &mut Diagnostics) -> Mir {
    let mut mir = Mir::default();
    // Memory objects for every global leaf.
    for (gi, g) in p.globals.iter().enumerate() {
        let gid = GlobalId(gi as u32);
        if g.ty.is_error() {
            continue;
        }
        let len = flat_len(&g.ty);
        let ls = leaves(&p.types, innermost(&g.ty));
        // Flatten initial values: elements, each a tuple of leaves.
        let mut flat = Vec::new();
        flat_values(&g.value, &mut flat);
        let n_leaves = ls.len().max(1);
        let mut ids = Vec::new();
        for (k, (suffix, sc)) in ls.iter().enumerate() {
            let init: Vec<i64> = flat
                .iter()
                .skip(k)
                .step_by(n_leaves)
                .map(|v| v.int())
                .collect();
            let kind = match &g.kind {
                GlobalKind::Const => ObjKind::Rom,
                GlobalKind::Var => ObjKind::Ram(g.placement),
                GlobalKind::Io(a) => ObjKind::Io(*a),
            };
            let id = ObjId(mir.objs.len() as u32);
            mir.objs.push(Obj {
                name: format!("{}{suffix}", g.path),
                sc: *sc,
                len,
                kind,
                init,
                bank: g.bank,
                shared: g.shared,
                global: Some(gid),
                span: g.span,
            });
            ids.push(id);
        }
        mir.global_objs.insert(gid, ids);
    }
    // Instances reachable from entry points.
    let mut queue: Vec<(FnId, Context)> = Vec::new();
    for (i, f) in p.functions.iter().enumerate() {
        let id = FnId(i as u32);
        if f.result.is_error() && f.body.is_empty() {
            continue;
        }
        let ctx = if f.attrs.reset {
            Some(Context::Main)
        } else if let Some(c) = f.attrs.interrupt {
            Some(c)
        } else if f.attrs.export {
            Some(Context::Main)
        } else {
            None
        };
        if let Some(ctx) = ctx {
            instance(&mut mir, p, id, ctx, &mut queue);
        }
    }
    while let Some((f, ctx)) = queue.pop() {
        let inst_id = mir.inst_of[&(f, ctx)];
        let func = &p.functions[f.0 as usize];
        let mut b = Builder {
            p,
            mir: &mut mir,
            inst: Inst {
                name: if ctx == Context::Main {
                    func.path.clone()
                } else {
                    format!("{}@{:?}", func.path, ctx).to_lowercase()
                },
                func: f,
                ctx,
                vars: Vec::new(),
                params: Vec::new(),
                rets: Vec::new(),
                blocks: Vec::new(),
                entry: BlockId(0),
                inline: func.attrs.inline,
                noinline: func.attrs.noinline || func.attrs.budget.is_some(),
                recursive: func.attrs.recursive,
                interrupt: func.attrs.interrupt,
                reset: func.attrs.reset,
                budget: func.attrs.budget,
                bank: func.attrs.bank,
                span: func.span,
                has_asm: func.has_asm,
            },
            locals: Vec::new(),
            cur: BlockId(0),
            loops: Vec::new(),
            depth: 0,
            exit: BlockId(0),
            queue: &mut queue,
            diags,
            inst_id,
        };
        b.function(func);
        let inst = b.inst;
        mir.insts[inst_id.0 as usize] = inst;
    }
    mir
}

fn instance(
    mir: &mut Mir,
    p: &Program,
    f: FnId,
    ctx: Context,
    queue: &mut Vec<(FnId, Context)>,
) -> InstId {
    if let Some(i) = mir.inst_of.get(&(f, ctx)) {
        return *i;
    }
    let id = InstId(mir.insts.len() as u32);
    mir.insts.push(Inst {
        name: p.functions[f.0 as usize].path.clone(),
        func: f,
        ctx,
        vars: Vec::new(),
        params: Vec::new(),
        rets: Vec::new(),
        blocks: Vec::new(),
        entry: BlockId(0),
        inline: false,
        noinline: false,
        recursive: None,
        interrupt: None,
        reset: false,
        budget: None,
        bank: None,
        span: Span::default(),
        has_asm: false,
    });
    mir.inst_of.insert((f, ctx), id);
    queue.push((f, ctx));
    id
}

struct Builder<'a, 'm> {
    p: &'a Program,
    mir: &'m mut Mir,
    inst: Inst,
    locals: Vec<LocalRepr>,
    cur: BlockId,
    loops: Vec<(BlockId, BlockId)>,
    depth: u32,
    exit: BlockId,
    queue: &'m mut Vec<(FnId, Context)>,
    diags: &'m mut Diagnostics,
    inst_id: InstId,
}

/// Expression may change state or read hardware.
fn effectful(e: &hir::Expr) -> bool {
    match &e.kind {
        ExprKind::Call(..) | ExprKind::Asm(_) => true,
        ExprKind::Intrinsic(Intrinsic::Peek | Intrinsic::Poke, _) => true,
        ExprKind::Const(_) | ExprKind::Local(_) | ExprKind::Global(_) | ExprKind::SliceOf(_) => {
            false
        }
        ExprKind::Unary(_, a)
        | ExprKind::Cast(a)
        | ExprKind::Field(a, _)
        | ExprKind::Repeat(a, _) => effectful(a),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => effectful(a) || effectful(b),
        ExprKind::Struct(v) | ExprKind::Array(v) | ExprKind::Intrinsic(_, v) => {
            v.iter().any(effectful)
        }
        ExprKind::Match(x, arms) => {
            effectful(x)
                || arms
                    .iter()
                    .any(|a| effectful(&a.body) || a.guard.as_ref().is_some_and(effectful))
        }
        ExprKind::If(c, a, b) => effectful(c) || effectful(a) || effectful(b),
    }
}

impl Builder<'_, '_> {
    fn types(&self) -> &crate::types::TypeTable {
        &self.p.types
    }
    fn sc(&self, t: &Type) -> Sc {
        sc_of(&self.p.types, t)
    }
    fn emit(&mut self, s: St) {
        let b = &mut self.inst.blocks[self.cur.0 as usize];
        if matches!(b.term, Term::Open) {
            b.stmts.push(s);
        }
    }
    fn term(&mut self, t: Term) {
        let b = &mut self.inst.blocks[self.cur.0 as usize];
        if matches!(b.term, Term::Open) {
            b.term = t;
        }
    }
    fn new_block(&mut self) -> BlockId {
        self.inst.block(self.depth)
    }
    fn switch(&mut self, b: BlockId) {
        self.cur = b;
    }
    fn temp(&mut self, sc: Sc) -> VarId {
        let n = self.inst.vars.len();
        self.inst.var(format!("t{n}"), sc, true)
    }
    fn materialize(&mut self, e: Ex) -> Ex {
        if matches!(e, Ex::Const(..) | Ex::Addr(..)) {
            return e;
        }
        let sc = e.sc();
        let t = self.temp(sc);
        self.emit(St::Assign(Place::Var(t), e));
        Ex::Var(t, sc)
    }

    fn function(&mut self, f: &Function) {
        let entry = self.new_block();
        self.inst.entry = entry;
        self.exit = self.new_block();
        self.cur = entry;
        for l in &f.locals {
            let repr = match &l.ty {
                Type::Array(..) => {
                    let len = flat_len(&l.ty);
                    let mut objs = Vec::new();
                    for (suffix, sc) in leaves(self.types(), innermost(&l.ty)) {
                        let id = ObjId(self.mir.objs.len() as u32);
                        self.mir.objs.push(Obj {
                            name: format!("{}.{}{suffix}", self.inst.name, l.name),
                            sc,
                            len,
                            kind: ObjKind::Frame(self.inst_id),
                            init: Vec::new(),
                            bank: None,
                            shared: false,
                            global: None,
                            span: l.span,
                        });
                        objs.push(id);
                    }
                    LocalRepr::Array(objs)
                }
                t => {
                    let ls = leaves(&self.p.types, t);
                    let vars = ls
                        .into_iter()
                        .map(|(suffix, sc)| self.inst.var(format!("{}{suffix}", l.name), sc, false))
                        .collect();
                    LocalRepr::Scalars(vars)
                }
            };
            self.locals.push(repr);
        }
        for p in &f.params {
            if let LocalRepr::Scalars(v) = &self.locals[p.0 as usize] {
                self.inst.params.extend(v.iter().copied());
            }
        }
        for (suffix, sc) in leaves(self.types(), &f.result) {
            let v = self.inst.var(format!("ret{suffix}"), sc, false);
            self.inst.rets.push(v);
        }
        // Slice parameters are dereferenced through zero page.
        self.block(&f.body);
        self.term(Term::Goto(self.exit));
        let exit = self.exit;
        self.inst.blocks[exit.0 as usize].term = Term::Return;
    }

    // ------------------------------------------------------------ places

    fn place_ref(&mut self, e: &hir::Expr) -> Option<PlaceRef> {
        match &e.kind {
            ExprKind::Global(g) => Some(PlaceRef {
                root: Root::Global(*g),
                index: None,
                leaf: 0,
                count: leaves(self.types(), innermost(&e.ty)).len(),
                ty: e.ty.clone(),
            }),
            ExprKind::Local(l) => {
                if matches!(e.ty, Type::Slice(_)) {
                    return None;
                }
                Some(PlaceRef {
                    root: Root::Local(*l),
                    index: None,
                    leaf: 0,
                    count: leaves(self.types(), innermost(&e.ty)).len(),
                    ty: e.ty.clone(),
                })
            }
            ExprKind::Field(b, f) => {
                let mut pr = self.place_ref(b)?;
                let Type::Struct(sid) = &pr.ty else {
                    return None;
                };
                let fields = &self.p.types.structs[sid.0 as usize].fields;
                let offset: usize = fields[..*f]
                    .iter()
                    .map(|(_, t)| leaves(self.types(), t).len())
                    .sum();
                let ft = fields[*f].1.clone();
                pr.leaf += offset;
                pr.count = leaves(self.types(), &ft).len();
                pr.ty = ft;
                Some(pr)
            }
            ExprKind::Index(b, i, _) => {
                if matches!(b.ty, Type::Slice(_)) {
                    return None;
                }
                let mut pr = self.place_ref(b)?;
                let Type::Array(elem, n) = pr.ty.clone() else {
                    return None;
                };
                let inner = flat_len(&elem).unwrap_or(1);
                let total = flat_len(&pr.ty).unwrap_or(1)
                    * match pr.index {
                        Some(_) => 1,
                        None => 1,
                    };
                let _ = total;
                let root_len = match pr.root {
                    Root::Global(g) => flat_len(&self.p.globals[g.0 as usize].ty).unwrap_or(1),
                    Root::Local(_) => {
                        // Local arrays: total length of the root type.
                        n * inner
                    }
                };
                let wide = root_len > 256;
                let isc = if wide { Sc::U16 } else { Sc::U8 };
                // `a[raw i + k]` addresses element i + k without wrapping, so
                // the constant folds into the address.
                if let (ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub), x, k), true, false, 1) =
                    (&i.kind, *raw_index(e), wide, inner)
                {
                    if let (Some(k), Sc::U8) = (k.constant(), self.sc(&x.ty)) {
                        if pr.index.is_none() && (0..256).contains(&k) {
                            let base = self.scalar(x);
                            let wide_x = self.convert(base, Sc::U16);
                            let off = if *op == BinOp::Add { k } else { -k };
                            let idx = Ex::Bin(
                                BinOp::Add,
                                Box::new(wide_x),
                                Box::new(Ex::Const(off & 0xffff, Sc::U16)),
                                Sc::U16,
                            );
                            pr.index = Some(idx);
                            pr.ty = *elem;
                            return Some(pr);
                        }
                    }
                }
                let mut idx = self.scalar(i);
                idx = self.convert(idx, isc);
                if inner > 1 {
                    idx = fold_bin(BinOp::Mul, idx, Ex::Const(inner as i64, isc), isc);
                }
                let index = match pr.index.take() {
                    None => idx,
                    Some(outer) => fold_bin(BinOp::Add, outer, idx, isc),
                };
                pr.index = Some(index);
                pr.ty = *elem;
                Some(pr)
            }
            _ => None,
        }
    }

    fn convert(&self, e: Ex, to: Sc) -> Ex {
        let from = e.sc();
        if from == to {
            return e;
        }
        if let Ex::Const(n, _) = e {
            return Ex::Const(to.wrap(n), to);
        }
        Ex::Cast(Box::new(e), from, to)
    }

    fn leaf_objs(&self, root: Root) -> Vec<ObjId> {
        match root {
            Root::Global(g) => self.mir.global_objs.get(&g).cloned().unwrap_or_default(),
            Root::Local(l) => match &self.locals[l.0 as usize] {
                LocalRepr::Array(o) => o.clone(),
                LocalRepr::Scalars(_) => Vec::new(),
            },
        }
    }

    /// Simplify an index so it can be repeated across leaves.
    fn stable_index(&mut self, pr: &mut PlaceRef) {
        if pr.count > 1 {
            if let Some(i) = pr.index.take() {
                let i = if i.simple() { i } else { self.materialize(i) };
                pr.index = Some(i);
            }
        }
    }

    fn read_place(&mut self, mut pr: PlaceRef) -> Vec<Ex> {
        self.stable_index(&mut pr);
        let mut out = Vec::new();
        match pr.root {
            Root::Local(l) if matches!(self.locals[l.0 as usize], LocalRepr::Scalars(_)) => {
                let LocalRepr::Scalars(vars) = &self.locals[l.0 as usize] else {
                    unreachable!()
                };
                for v in &vars[pr.leaf..pr.leaf + pr.count] {
                    out.push(Ex::Var(*v, self.inst.vars[v.0 as usize].sc));
                }
            }
            root => {
                let objs = self.leaf_objs(root);
                for o in &objs[pr.leaf..(pr.leaf + pr.count).min(objs.len())] {
                    let obj = &self.mir.objs[o.0 as usize];
                    let sc = obj.sc;
                    // Constant ROM scalars fold.
                    if obj.len.is_none() && obj.kind == ObjKind::Rom {
                        out.push(Ex::Const(obj.init.first().copied().unwrap_or(0), sc));
                        continue;
                    }
                    if let (ObjKind::Rom, Some(Ex::Const(k, _)), None) =
                        (&obj.kind, &pr.index, obj.bank)
                    {
                        if let Some(v) = obj.init.get(*k as usize) {
                            out.push(Ex::Const(*v, sc));
                            continue;
                        }
                    }
                    out.push(Ex::Load(*o, pr.index.clone().map(Box::new), sc));
                }
            }
        }
        out
    }

    fn write_place(&mut self, mut pr: PlaceRef, values: Vec<Ex>) {
        self.stable_index(&mut pr);
        match pr.root {
            Root::Local(l) if matches!(self.locals[l.0 as usize], LocalRepr::Scalars(_)) => {
                let LocalRepr::Scalars(vars) = self.locals[l.0 as usize].clone() else {
                    unreachable!()
                };
                for (v, e) in vars[pr.leaf..pr.leaf + pr.count].iter().zip(values) {
                    self.emit(St::Assign(Place::Var(*v), e));
                }
            }
            root => {
                let objs = self.leaf_objs(root);
                for (o, e) in objs[pr.leaf..pr.leaf + pr.count].iter().zip(values) {
                    self.emit(St::Assign(Place::Mem(*o, pr.index.clone()), e));
                }
            }
        }
    }

    // ------------------------------------------------------- expressions

    fn scalar(&mut self, e: &hir::Expr) -> Ex {
        let mut v = self.expr(e);
        if v.is_empty() {
            return Ex::Const(0, Sc::U8);
        }
        v.swap_remove(0)
    }

    /// Lower an expression to its scalar leaves.
    fn expr(&mut self, e: &hir::Expr) -> Vec<Ex> {
        let types = &self.p.types;
        match &e.kind {
            ExprKind::Const(v) => {
                if let Type::Slice(_) = e.ty {
                    return vec![Ex::Const(0, Sc::U16), Ex::Const(0, Sc::U16)];
                }
                let mut flat = Vec::new();
                flat_values(v, &mut flat);
                let ls = leaves(types, &e.ty);
                flat.iter()
                    .zip(ls)
                    .map(|(v, (_, sc))| Ex::Const(sc.wrap(v.int()), sc))
                    .collect()
            }
            ExprKind::Local(l) => match &self.locals[l.0 as usize] {
                LocalRepr::Scalars(vars) => vars
                    .iter()
                    .map(|v| Ex::Var(*v, self.inst.vars[v.0 as usize].sc))
                    .collect(),
                LocalRepr::Array(_) => {
                    self.diags
                        .error(e.span, "Whole arrays cannot be copied; index them");
                    vec![]
                }
            },
            ExprKind::Global(_) | ExprKind::Field(..) | ExprKind::Index(..) => {
                if let ExprKind::Index(b, i, _) = &e.kind {
                    if let Type::Slice(_) = b.ty {
                        return vec![self.slice_index(b, i)];
                    }
                }
                if let ExprKind::Field(b, f) = &e.kind {
                    if self.place_ref(b).is_none() {
                        // Field of an rvalue struct.
                        let all = self.expr(b);
                        let Type::Struct(sid) = &b.ty else {
                            return vec![];
                        };
                        let fields = &self.p.types.structs[sid.0 as usize].fields;
                        let offset: usize = fields[..*f]
                            .iter()
                            .map(|(_, t)| leaves(&self.p.types, t).len())
                            .sum();
                        let count = leaves(&self.p.types, &fields[*f].1).len();
                        return all.into_iter().skip(offset).take(count).collect();
                    }
                }
                if matches!(e.ty, Type::Array(..)) {
                    self.diags.error(e.span, "Whole arrays cannot be used as values; index them or view a constant as a slice");
                    return vec![];
                }
                match self.place_ref(e) {
                    Some(pr) => self.read_place(pr),
                    None => vec![],
                }
            }
            ExprKind::SliceOf(g) => {
                let objs = &self.mir.global_objs[g];
                let len = self.mir.objs[objs[0].0 as usize].len.unwrap_or(1);
                vec![Ex::Addr(objs[0], false), Ex::Const(len as i64, Sc::U16)]
            }
            ExprKind::Unary(op, x) => {
                let sc = self.sc(&e.ty);
                let a = self.scalar(x);
                vec![match op {
                    UnOp::Not => Ex::Bin(
                        BinOp::Eq,
                        Box::new(a),
                        Box::new(Ex::Const(0, Sc::U8)),
                        Sc::U8,
                    ),
                    op => Ex::Un(*op, Box::new(a), sc),
                }]
            }
            ExprKind::Binary(op, a, b) => {
                if op.is_logic() {
                    let t = self.temp(Sc::U8);
                    let x = self.scalar(a);
                    self.emit(St::Assign(Place::Var(t), x));
                    let rhs = self.new_block();
                    let done = self.new_block();
                    let cond = Ex::Var(t, Sc::U8);
                    if *op == BinOp::LogicAnd {
                        self.term(Term::Branch(cond, rhs, done));
                    } else {
                        self.term(Term::Branch(cond, done, rhs));
                    }
                    self.switch(rhs);
                    let y = self.scalar(b);
                    self.emit(St::Assign(Place::Var(t), y));
                    self.term(Term::Goto(done));
                    self.switch(done);
                    return vec![Ex::Var(t, Sc::U8)];
                }
                let sc = self.sc(&a.ty);
                let mut x = self.scalar(a);
                if effectful(b) && !matches!(x, Ex::Const(..)) {
                    x = self.materialize(x);
                }
                let y = self.scalar(b);
                vec![fold_bin(*op, x, y, sc)]
            }
            ExprKind::Cast(x) => {
                if matches!(e.ty, Type::Slice(_)) {
                    return self.expr(x);
                }
                let to = self.sc(&e.ty);
                let v = self.scalar(x);
                vec![self.convert(v, to)]
            }
            ExprKind::Struct(fields) | ExprKind::Array(fields) => {
                let mut out = Vec::new();
                let n = fields.len();
                for (i, f) in fields.iter().enumerate() {
                    let mut v = self.expr(f);
                    if fields[i + 1..n].iter().any(effectful) {
                        v = v
                            .into_iter()
                            .map(|x| if x.simple() { x } else { self.materialize(x) })
                            .collect();
                    }
                    out.extend(v);
                }
                out
            }
            ExprKind::Repeat(x, _) => self.expr(x),
            ExprKind::Call(f, args) => self.call(*f, args, &e.ty, e.span),
            ExprKind::Intrinsic(i, args) => match i {
                Intrinsic::Lo => {
                    let a = self.scalar(&args[0]);
                    vec![self.convert(a, Sc::U8)]
                }
                Intrinsic::Hi => {
                    let a = self.scalar(&args[0]);
                    let sc = a.sc();
                    let shifted = fold_bin(BinOp::Shr, a, Ex::Const(8, Sc::U8), Sc::U16.max(sc));
                    vec![self.convert(shifted, Sc::U8)]
                }
                Intrinsic::Peek => {
                    let a = self.scalar(&args[0]);
                    vec![Ex::Peek(Box::new(a))]
                }
                Intrinsic::Poke => {
                    let mut a = self.scalar(&args[0]);
                    if effectful(&args[1]) && !a.simple() {
                        a = self.materialize(a);
                    }
                    let v = self.scalar(&args[1]);
                    self.emit(St::Assign(Place::Poke(a), v));
                    vec![]
                }
                Intrinsic::Addr => match &args[0].kind {
                    ExprKind::Global(g) => {
                        match self.mir.global_objs.get(g).and_then(|o| o.first()) {
                            Some(o) => vec![Ex::Addr(*o, false)],
                            None => vec![Ex::Const(0, Sc::U16)],
                        }
                    }
                    _ => {
                        let v = self.expr(&args[0]);
                        vec![v.into_iter().next().unwrap_or(Ex::Const(0, Sc::U16))]
                    }
                },
                Intrinsic::SliceLen => {
                    let v = self.expr(&args[0]);
                    vec![v.get(1).cloned().unwrap_or(Ex::Const(0, Sc::U16))]
                }
            },
            ExprKind::Match(x, arms) => {
                let ls = leaves(types, &e.ty);
                let temps: Vec<VarId> = ls.iter().map(|(_, sc)| self.temp(*sc)).collect();
                let scrut = self.scalar(x);
                let scrut = if scrut.simple() {
                    scrut
                } else {
                    self.materialize(scrut)
                };
                let done = self.new_block();
                let full = self.types().bounds(&x.ty);
                self.dispatch(&scrut, arms, full, |b, arm: &hir::Arm<hir::Expr>| {
                    let v = b.expr(&arm.body);
                    for (t, x) in temps.iter().zip(v) {
                        b.emit(St::Assign(Place::Var(*t), x));
                    }
                    b.term(Term::Goto(done));
                });
                self.switch(done);
                temps
                    .iter()
                    .zip(ls)
                    .map(|(t, (_, sc))| Ex::Var(*t, sc))
                    .collect()
            }
            ExprKind::If(c, a, b) => {
                let ls = leaves(types, &e.ty);
                let temps: Vec<VarId> = ls.iter().map(|(_, sc)| self.temp(*sc)).collect();
                let yes = self.new_block();
                let no = self.new_block();
                let done = self.new_block();
                self.branch(c, yes, no);
                for (blk, x) in [(yes, a), (no, b)] {
                    self.switch(blk);
                    let v = self.expr(x);
                    for (t, x) in temps.iter().zip(v) {
                        self.emit(St::Assign(Place::Var(*t), x));
                    }
                    self.term(Term::Goto(done));
                }
                self.switch(done);
                temps
                    .iter()
                    .zip(ls)
                    .map(|(t, (_, sc))| Ex::Var(*t, sc))
                    .collect()
            }
            ExprKind::Asm(block) => {
                let t = self.temp(Sc::U8);
                self.asm(block, Some(Place::Var(t)));
                vec![Ex::Var(t, Sc::U8)]
            }
        }
    }

    fn slice_index(&mut self, s: &hir::Expr, i: &hir::Expr) -> Ex {
        let sc = match &s.ty {
            Type::Slice(t) => self.sc(t),
            _ => Sc::U8,
        };
        let parts = self.expr(s);
        let idx = self.scalar(i);
        match parts.first() {
            Some(Ex::Addr(o, false)) => {
                let small = self.mir.objs[o.0 as usize].len.unwrap_or(1) <= 256;
                let idx = if small {
                    self.convert(idx, Sc::U8)
                } else {
                    self.convert(idx, Sc::U16)
                };
                Ex::Load(*o, Some(Box::new(idx)), sc)
            }
            Some(Ex::Var(v, _)) => {
                self.inst.vars[v.0 as usize].pointer = true;
                Ex::Deref(Box::new(Ex::Var(*v, Sc::U16)), Box::new(idx), sc)
            }
            Some(other) => {
                let p = self.materialize(other.clone());
                if let Ex::Var(v, _) = p {
                    self.inst.vars[v.0 as usize].pointer = true;
                }
                Ex::Deref(Box::new(p), Box::new(idx), sc)
            }
            None => Ex::Const(0, sc),
        }
    }

    fn target(&mut self, f: FnId) -> InstId {
        instance(self.mir, self.p, f, self.inst.ctx, self.queue)
    }

    fn call(&mut self, f: FnId, args: &[hir::Expr], result: &Type, span: Span) -> Vec<Ex> {
        let mut values = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let mut v = self.expr(a);
            if args[i + 1..].iter().any(effectful) {
                v = v
                    .into_iter()
                    .map(|x| if x.simple() { x } else { self.materialize(x) })
                    .collect();
            }
            values.extend(v);
        }
        let target = self.target(f);
        let ls = leaves(self.types(), result);
        let temps: Vec<VarId> = ls.iter().map(|(_, sc)| self.temp(*sc)).collect();
        self.emit(St::Call {
            target,
            args: values,
            rets: temps.iter().map(|t| Place::Var(*t)).collect(),
            saved: Vec::new(),
            span,
        });
        temps
            .iter()
            .zip(ls)
            .map(|(t, (_, sc))| Ex::Var(*t, sc))
            .collect()
    }

    fn asm(&mut self, block: &AsmBlock, output: Option<Place>) {
        let mut operands = Vec::new();
        for r in &block.refs {
            operands.push(match r {
                AsmRef::Local(l) => match &self.locals[l.0 as usize] {
                    LocalRepr::Scalars(v) if !v.is_empty() => AsmOperand::Var(v[0]),
                    LocalRepr::Array(o) if !o.is_empty() => AsmOperand::Obj(o[0]),
                    _ => AsmOperand::Const(0),
                },
                AsmRef::Global(g) => match self.mir.global_objs.get(g).and_then(|o| o.first()) {
                    Some(o) => AsmOperand::Obj(*o),
                    None => AsmOperand::Const(0),
                },
                AsmRef::Fn(f) => AsmOperand::Fn(self.target(*f)),
                AsmRef::Const(n) => AsmOperand::Const(*n),
            });
        }
        let mut inputs = Vec::new();
        for (r, e) in &block.inputs {
            let v = self.scalar(e);
            inputs.push((*r, v));
        }
        // Inputs are evaluated before any register is loaded.
        let inputs = inputs
            .into_iter()
            .map(|(r, v)| {
                (
                    r,
                    if matches!(v, Ex::Const(..) | Ex::Var(..)) {
                        v
                    } else {
                        self.materialize(v)
                    },
                )
            })
            .collect();
        self.inst.has_asm = true;
        self.emit(St::Asm {
            body: block.body.clone(),
            operands,
            inputs,
            output,
            span: block.span,
        });
    }

    // ------------------------------------------------------ control flow

    fn branch(&mut self, e: &hir::Expr, t: BlockId, f: BlockId) {
        match &e.kind {
            ExprKind::Const(Value::Int(n)) => self.term(Term::Goto(if *n != 0 { t } else { f })),
            ExprKind::Unary(UnOp::Not, x) => self.branch(x, f, t),
            ExprKind::Binary(BinOp::LogicAnd, a, b) => {
                let mid = self.new_block();
                self.branch(a, mid, f);
                self.switch(mid);
                self.branch(b, t, f);
            }
            ExprKind::Binary(BinOp::LogicOr, a, b) => {
                let mid = self.new_block();
                self.branch(a, t, mid);
                self.switch(mid);
                self.branch(b, t, f);
            }
            _ => {
                let c = self.scalar(e);
                match c {
                    Ex::Const(n, _) => self.term(Term::Goto(if n != 0 { t } else { f })),
                    c => self.term(Term::Branch(c, t, f)),
                }
            }
        }
    }

    /// Branch to each arm's body by pattern; `body` lowers an arm.
    fn dispatch<B>(
        &mut self,
        scrut: &Ex,
        arms: &[hir::Arm<B>],
        full: (i64, i64),
        mut body: impl FnMut(&mut Self, &hir::Arm<B>),
    ) {
        let sc = scrut.sc();
        let n = arms.len();
        for (k, arm) in arms.iter().enumerate() {
            let next = self.new_block();
            let target = self.new_block();
            let last = k + 1 == n;
            if arm.patterns.is_empty() || (last && arm.guard.is_none()) {
                // Wildcard, or the final arm of an exhaustive match.
                self.term(Term::Goto(target));
            } else {
                for (i, (lo, hi)) in arm.patterns.iter().enumerate() {
                    let fail = if i + 1 == arm.patterns.len() {
                        next
                    } else {
                        self.new_block()
                    };
                    let c = |op, v: i64| {
                        Ex::Bin(
                            op,
                            Box::new(scrut.clone()),
                            Box::new(Ex::Const(sc.wrap(v), sc)),
                            sc,
                        )
                    };
                    if lo == hi {
                        self.term(Term::Branch(c(BinOp::Eq, *lo), target, fail));
                    } else if *lo <= full.0 {
                        self.term(Term::Branch(c(BinOp::Le, *hi), target, fail));
                    } else if *hi >= full.1 {
                        self.term(Term::Branch(c(BinOp::Ge, *lo), target, fail));
                    } else {
                        let mid = self.new_block();
                        self.term(Term::Branch(c(BinOp::Ge, *lo), mid, fail));
                        self.switch(mid);
                        self.term(Term::Branch(c(BinOp::Le, *hi), target, fail));
                    }
                    self.switch(fail);
                }
            }
            self.switch(target);
            if let Some(g) = &arm.guard {
                let ok = self.new_block();
                self.branch(g, ok, next);
                self.switch(ok);
            }
            body(self, arm);
            self.switch(next);
        }
    }

    fn block(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let(id, e) => {
                if let LocalRepr::Array(objs) = self.locals[id.0 as usize].clone() {
                    // Initialize a frame array element by element.
                    let values = match &e.kind {
                        ExprKind::Const(v) => {
                            let mut flat = Vec::new();
                            flat_values(v, &mut flat);
                            flat
                        }
                        _ => {
                            self.diags
                                .error(e.span, "Local arrays need a constant initializer");
                            return;
                        }
                    };
                    let n = objs.len().max(1);
                    let len = values.len() / n;
                    if len > 64 {
                        self.diags.warning(e.span, "Large local arrays are initialized element by element; prefer a global");
                    }
                    for i in 0..len {
                        for (k, o) in objs.iter().enumerate() {
                            let sc = self.mir.objs[o.0 as usize].sc;
                            let isc = if self.mir.objs[o.0 as usize].len.unwrap_or(0) > 256 {
                                Sc::U16
                            } else {
                                Sc::U8
                            };
                            let v = values[i * n + k].int();
                            self.emit(St::Assign(
                                Place::Mem(*o, Some(Ex::Const(i as i64, isc))),
                                Ex::Const(sc.wrap(v), sc),
                            ));
                        }
                    }
                    return;
                }
                self.emit(St::Line(e.span));
                let v = self.expr(e);
                let LocalRepr::Scalars(vars) = self.locals[id.0 as usize].clone() else {
                    return;
                };
                for (var, x) in vars.iter().zip(v) {
                    self.emit(St::Assign(Place::Var(*var), x));
                }
            }
            Stmt::Assign(target, op, value, span) => {
                self.emit(St::Line(*span));
                if let ExprKind::Index(b, i, _) = &target.kind {
                    if let Type::Slice(_) = b.ty {
                        let _ = i;
                        return;
                    }
                }
                let Some(mut pr) = self.place_ref(target) else {
                    self.diags.error(*span, "Cannot assign to this expression");
                    return;
                };
                if let Some(op) = op {
                    // Index evaluated once.
                    if let Some(i) = pr.index.take() {
                        let i = if i.simple() { i } else { self.materialize(i) };
                        pr.index = Some(i);
                    }
                    let old = self.read_place(pr.clone());
                    let sc = old.first().map(Ex::sc).unwrap_or(Sc::U8);
                    let v = self.scalar(value);
                    let v = if matches!(op, BinOp::Shl | BinOp::Shr) {
                        v
                    } else {
                        self.convert(v, sc)
                    };
                    let new = fold_bin(
                        *op,
                        old.into_iter().next().unwrap_or(Ex::Const(0, sc)),
                        v,
                        sc,
                    );
                    self.write_place(pr, vec![new]);
                } else {
                    let v = self.expr(value);
                    if effectful(value) || pr.count > 1 {
                        // Values computed before the place's index is used.
                        let v: Vec<Ex> = if pr.count > 1 {
                            v.into_iter()
                                .map(|x| if x.simple() { x } else { self.materialize(x) })
                                .collect()
                        } else {
                            v
                        };
                        self.write_place(pr, v);
                    } else {
                        self.write_place(pr, v);
                    }
                }
            }
            Stmt::Expr(e) => {
                self.emit(St::Line(e.span));
                match &e.kind {
                    ExprKind::Asm(block) => self.asm(block, None),
                    ExprKind::Call(f, args) => {
                        let mut values = Vec::new();
                        for (i, a) in args.iter().enumerate() {
                            let mut v = self.expr(a);
                            if args[i + 1..].iter().any(effectful) {
                                v = v
                                    .into_iter()
                                    .map(|x| if x.simple() { x } else { self.materialize(x) })
                                    .collect();
                            }
                            values.extend(v);
                        }
                        let target = self.target(*f);
                        self.emit(St::Call {
                            target,
                            args: values,
                            rets: Vec::new(),
                            saved: Vec::new(),
                            span: e.span,
                        });
                    }
                    _ => {
                        let v = self.expr(e);
                        for x in v {
                            if x.volatile(&self.mir.objs) {
                                let t = self.temp(x.sc());
                                self.emit(St::Assign(Place::Var(t), x));
                            }
                        }
                    }
                }
            }
            Stmt::If(c, a, b) => {
                let yes = self.new_block();
                let no = if b.is_empty() {
                    None
                } else {
                    Some(self.new_block())
                };
                let done = self.new_block();
                self.branch(c, yes, no.unwrap_or(done));
                self.switch(yes);
                self.block(a);
                self.term(Term::Goto(done));
                if let Some(no) = no {
                    self.switch(no);
                    self.block(b);
                    self.term(Term::Goto(done));
                }
                self.switch(done);
            }
            Stmt::While(c, body) => {
                let head = self.new_block();
                self.term(Term::Goto(head));
                self.depth += 1;
                let b = self.new_block();
                let latch = head;
                self.depth -= 1;
                let done = self.new_block();
                self.switch(head);
                self.inst.blocks[head.0 as usize].depth = self.depth + 1;
                self.branch(c, b, done);
                self.depth += 1;
                self.loops.push((latch, done));
                self.switch(b);
                self.block(body);
                self.term(Term::Goto(head));
                self.loops.pop();
                self.depth -= 1;
                self.switch(done);
            }
            Stmt::Loop(body) => {
                self.depth += 1;
                let b = self.new_block();
                self.depth -= 1;
                let done = self.new_block();
                self.term(Term::Goto(b));
                self.depth += 1;
                self.loops.push((b, done));
                self.switch(b);
                self.block(body);
                self.term(Term::Goto(b));
                self.loops.pop();
                self.depth -= 1;
                self.switch(done);
            }
            Stmt::For {
                var,
                start,
                end,
                inclusive,
                reverse,
                body,
            } => self.for_loop(*var, start, end, *inclusive, *reverse, body),
            Stmt::Match(x, arms) => {
                let scrut = self.scalar(x);
                let scrut = if scrut.simple() {
                    scrut
                } else {
                    self.materialize(scrut)
                };
                let done = self.new_block();
                let full = self.types().bounds(&x.ty);
                self.dispatch(&scrut, arms, full, |b, arm: &hir::Arm<Vec<Stmt>>| {
                    b.block(&arm.body);
                    b.term(Term::Goto(done));
                });
                self.term(Term::Goto(done));
                self.switch(done);
            }
            Stmt::Break(_) => {
                if let Some((_, done)) = self.loops.last().copied() {
                    self.term(Term::Goto(done));
                    let dead = self.new_block();
                    self.switch(dead);
                }
            }
            Stmt::Continue(_) => {
                if let Some((latch, _)) = self.loops.last().copied() {
                    self.term(Term::Goto(latch));
                    let dead = self.new_block();
                    self.switch(dead);
                }
            }
            Stmt::Return(v, span) => {
                self.emit(St::Line(*span));
                if let Some(e) = v {
                    let vals = self.expr(e);
                    let rets = self.inst.rets.clone();
                    for (r, x) in rets.iter().zip(vals) {
                        self.emit(St::Assign(Place::Var(*r), x));
                    }
                }
                self.term(Term::Goto(self.exit));
                let dead = self.new_block();
                self.switch(dead);
            }
            Stmt::Block(b) => self.block(b),
        }
    }

    fn for_loop(
        &mut self,
        var: LocalId,
        start: &hir::Expr,
        end: &hir::Expr,
        inclusive: bool,
        reverse: bool,
        body: &[Stmt],
    ) {
        let LocalRepr::Scalars(vars) = self.locals[var.0 as usize].clone() else {
            return;
        };
        let v = vars[0];
        let sc = self.inst.vars[v.0 as usize].sc;
        let a = self.scalar(start);
        let a = self.convert(a, sc);
        let a = if a.simple() { a } else { self.materialize(a) };
        // One past the type's maximum: the counter wraps to its minimum.
        let (tmin, tmax) = match sc {
            Sc::U8 => (0, 255),
            Sc::I8 => (-128, 127),
            Sc::U16 => (0, 65535),
            Sc::I16 => (-32768, 32767),
        };
        let end_const = end.constant();
        let wraps = !inclusive && end_const == Some(tmax + 1);
        let b = if wraps {
            Ex::Const(sc.wrap(tmax + 1), sc)
        } else {
            let b = self.scalar(end);
            let b = self.convert(b, sc);
            if b.simple() {
                b
            } else {
                self.materialize(b)
            }
        };
        let body_b = {
            self.depth += 1;
            let x = self.new_block();
            self.depth -= 1;
            x
        };
        let latch = {
            self.depth += 1;
            let x = self.new_block();
            self.depth -= 1;
            x
        };
        let done = self.new_block();
        // Skip an empty range unless it is statically non-empty.
        let nonempty = match (a.konst(), b.konst()) {
            (Some(x), Some(y)) => {
                if inclusive {
                    x <= y
                } else {
                    wraps || x < y
                }
            }
            _ => false,
        };
        let empty_known = match (a.konst(), b.konst()) {
            (Some(x), Some(y)) => {
                if inclusive {
                    x > y
                } else {
                    !wraps && x >= y
                }
            }
            _ => false,
        };
        if empty_known {
            return;
        }
        let cmp = |op, x: &Ex, y: &Ex| Ex::Bin(op, Box::new(x.clone()), Box::new(y.clone()), sc);
        let init = if reverse {
            if inclusive || wraps {
                if wraps {
                    Ex::Const(sc.wrap(tmax), sc)
                } else {
                    b.clone()
                }
            } else {
                fold_bin(BinOp::Sub, b.clone(), Ex::Const(1, sc), sc)
            }
        } else {
            a.clone()
        };
        if !nonempty {
            let enter = self.new_block();
            let c = if inclusive {
                cmp(BinOp::Le, &a, &b)
            } else {
                cmp(BinOp::Lt, &a, &b)
            };
            let _ = tmin;
            self.term(Term::Branch(c, enter, done));
            self.switch(enter);
        }
        self.emit(St::Assign(Place::Var(v), init));
        self.term(Term::Goto(body_b));
        self.depth += 1;
        self.loops.push((latch, done));
        self.switch(body_b);
        self.block(body);
        self.term(Term::Goto(latch));
        self.loops.pop();
        self.switch(latch);
        let cur = Ex::Var(v, sc);
        if reverse {
            // Stop after the counter reaches the start.
            let step = self.new_block();
            self.term(Term::Branch(cmp(BinOp::Eq, &cur, &a), done, step));
            self.switch(step);
            self.emit(St::Assign(
                Place::Var(v),
                fold_bin(BinOp::Sub, cur.clone(), Ex::Const(1, sc), sc),
            ));
            self.term(Term::Goto(body_b));
        } else if inclusive {
            let step = self.new_block();
            self.term(Term::Branch(cmp(BinOp::Eq, &cur, &b), done, step));
            self.switch(step);
            self.emit(St::Assign(
                Place::Var(v),
                fold_bin(BinOp::Add, cur.clone(), Ex::Const(1, sc), sc),
            ));
            self.term(Term::Goto(body_b));
        } else {
            self.emit(St::Assign(
                Place::Var(v),
                fold_bin(BinOp::Add, cur.clone(), Ex::Const(1, sc), sc),
            ));
            self.term(Term::Branch(cmp(BinOp::Ne, &cur, &b), body_b, done));
        }
        self.depth -= 1;
        self.switch(done);
    }
}

/// Build a binary expression, folding constants.
pub fn fold_bin(op: BinOp, a: Ex, b: Ex, sc: Sc) -> Ex {
    if let (Some(x), Some(y)) = (a.konst(), b.konst()) {
        let t = match sc {
            Sc::U8 => Type::U8,
            Sc::I8 => Type::I8,
            Sc::U16 => Type::U16,
            Sc::I16 => Type::I16,
        };
        if let Some(v) = crate::consteval::binary(&Default::default(), op, x, y, &t) {
            let rsc = if op.is_compare() || op.is_logic() {
                Sc::U8
            } else {
                sc
            };
            return Ex::Const(rsc.wrap(v), rsc);
        }
    }
    Ex::Bin(op, Box::new(a), Box::new(b), sc)
}

#[allow(dead_code)]
fn unused(_: BTreeMap<u8, u8>) {}
