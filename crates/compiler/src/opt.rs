//! MIR analyses and optimizations.
//!
//! * liveness (also used for recursion save sets),
//! * constant and copy propagation within blocks,
//! * forward substitution of single-use temporaries into expression trees,
//! * dead assignment elimination,
//! * inlining of `@inline` and tiny functions,
//! * CFG simplification: jump threading, constant branches, block merging.
use crate::build_mir::fold_bin;
use crate::diag::Diagnostics;
use crate::hir::{BinOp, UnOp};
use crate::mir::*;
use std::collections::{BTreeMap, BTreeSet};

type Live = BTreeSet<VarId>;

fn ex_uses(e: &Ex, out: &mut Live) {
    e.walk(&mut |x| {
        if let Ex::Var(v, _) = x {
            out.insert(*v);
        }
    });
}

fn place_uses(p: &Place, out: &mut Live) {
    match p {
        Place::Var(_) => {}
        Place::Mem(_, Some(i)) => ex_uses(i, out),
        Place::Mem(_, None) => {}
        Place::Deref(a, b) => {
            ex_uses(a, out);
            ex_uses(b, out);
        }
        Place::Poke(a) => ex_uses(a, out),
    }
}

/// Variables a statement reads and writes.
fn stmt_effects(s: &St) -> (Live, Live) {
    let mut uses = Live::new();
    let mut defs = Live::new();
    match s {
        St::Assign(p, e) => {
            ex_uses(e, &mut uses);
            place_uses(p, &mut uses);
            if let Place::Var(v) = p {
                defs.insert(*v);
            }
        }
        St::Call { args, rets, .. } => {
            for a in args {
                ex_uses(a, &mut uses);
            }
            for r in rets {
                place_uses(r, &mut uses);
                if let Place::Var(v) = r {
                    defs.insert(*v);
                }
            }
        }
        St::Asm { operands, inputs, output, .. } => {
            for (_, e) in inputs {
                ex_uses(e, &mut uses);
            }
            for o in operands {
                if let AsmOperand::Var(v) = o {
                    // Assembly may read and write named variables.
                    uses.insert(*v);
                }
            }
            if let Some(p) = output {
                place_uses(p, &mut uses);
                if let Place::Var(v) = p {
                    defs.insert(*v);
                }
            }
        }
        St::Line(_) => {}
    }
    (uses, defs)
}

fn term_uses(t: &Term, inst: &Inst, out: &mut Live) {
    match t {
        Term::Branch(c, _, _) => ex_uses(c, out),
        Term::Return => out.extend(inst.rets.iter().copied()),
        _ => {}
    }
}

/// Live-in sets per block, and live-out sets.
pub fn liveness(inst: &Inst) -> (Vec<Live>, Vec<Live>) {
    let n = inst.blocks.len();
    let mut live_in = vec![Live::new(); n];
    let mut live_out = vec![Live::new(); n];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for b in 0..n {
        for s in inst.successors(BlockId(b as u32)) {
            preds[s.0 as usize].push(b);
        }
    }
    let mut work: Vec<usize> = (0..n).collect();
    while let Some(b) = work.pop() {
        let mut out = Live::new();
        for s in inst.successors(BlockId(b as u32)) {
            out.extend(live_in[s.0 as usize].iter().copied());
        }
        let mut live = out.clone();
        term_uses(&inst.blocks[b].term, inst, &mut live);
        for s in inst.blocks[b].stmts.iter().rev() {
            let (u, d) = stmt_effects(s);
            for v in d {
                live.remove(&v);
            }
            live.extend(u);
        }
        live_out[b] = out;
        if live != live_in[b] {
            live_in[b] = live;
            work.extend(preds[b].iter().copied());
        }
    }
    (live_in, live_out)
}

/// Mandatory passes: recursion save sets, recursion checks.
pub fn prepare(mir: &mut Mir, diags: &mut Diagnostics) {
    for i in 0..mir.insts.len() {
        let id = InstId(i as u32);
        let inst = &mir.insts[i];
        let self_calls = inst.blocks.iter().any(|b| {
            b.stmts
                .iter()
                .any(|s| matches!(s, St::Call { target, .. } if *target == id))
        });
        if !self_calls {
            continue;
        }
        if inst.recursive.is_none() {
            diags.error(inst.span, format!("'{}' calls itself; mark it @recursive(N) with its maximum depth", inst.name));
            continue;
        }
        if inst.ctx != crate::hir::Context::Main {
            diags.error(inst.span, "Recursive functions cannot run in interrupt handlers");
            continue;
        }
        let (_, live_out) = liveness(inst);
        let mut saves: Vec<(usize, usize, Vec<VarId>)> = Vec::new();
        for (b, block) in inst.blocks.iter().enumerate() {
            let mut live = live_out[b].clone();
            term_uses(&block.term, inst, &mut live);
            for (k, s) in block.stmts.iter().enumerate().rev() {
                let (u, d) = stmt_effects(s);
                if let St::Call { target, .. } = s {
                    if *target == id {
                        let after: Vec<VarId> = live.iter().copied().filter(|v| !d.contains(v)).collect();
                        saves.push((b, k, after));
                    }
                }
                for v in d {
                    live.remove(&v);
                }
                live.extend(u);
            }
        }
        let inst = &mut mir.insts[i];
        for (b, k, vars) in saves {
            if let St::Call { saved, .. } = &mut inst.blocks[b].stmts[k] {
                *saved = vars;
            }
        }
    }
}

// --------------------------------------------------------------- optimize

pub fn optimize(mir: &mut Mir) {
    inline(mir);
    for i in 0..mir.insts.len() {
        let objs = mir.objs.clone();
        let inst = &mut mir.insts[i];
        if inst.blocks.is_empty() {
            continue;
        }
        for _ in 0..4 {
            let mut changed = false;
            changed |= simplify_cfg(inst);
            for b in 0..inst.blocks.len() {
                changed |= propagate(inst, b, &objs);
            }
            changed |= dead_assignments(inst, &objs);
            if !changed {
                break;
            }
        }
        simplify_cfg(inst);
    }
}

/// Algebraic simplification of an expression tree.
pub fn simplify(e: &mut Ex) {
    e.walk_mut(&mut |x| {
        let new = match x {
            Ex::Bin(op, a, b, sc) => {
                let (op, sc) = (*op, *sc);
                if let (Some(p), Some(q)) = (a.konst(), b.konst()) {
                    Some(fold_bin(op, Ex::Const(p, sc), Ex::Const(q, sc), sc))
                } else {
                    let ka = a.konst().map(|k| sc.wrap(k));
                    let kb = b.konst().map(|k| sc.wrap(k));
                    let all = sc.wrap(-1);
                    match (op, ka, kb) {
                        (BinOp::Add | BinOp::Sub | BinOp::Or | BinOp::Xor | BinOp::Shl | BinOp::Shr, _, Some(0)) => Some((**a).clone()),
                        (BinOp::Add | BinOp::Or | BinOp::Xor, Some(0), _) => Some((**b).clone()),
                        (BinOp::Mul, _, Some(1)) | (BinOp::Div, _, Some(1)) => Some((**a).clone()),
                        (BinOp::Mul, Some(1), _) => Some((**b).clone()),
                        (BinOp::Mul | BinOp::And, _, Some(0)) => Some(Ex::Const(0, sc)),
                        (BinOp::Mul | BinOp::And, Some(0), _) => Some(Ex::Const(0, sc)),
                        (BinOp::Shl | BinOp::Shr, _, Some(k)) if !sc.signed() && k >= (sc.size() * 8) as i64 => Some(Ex::Const(0, sc)),
                        (BinOp::Shl, _, Some(k)) if k >= (sc.size() * 8) as i64 => Some(Ex::Const(0, sc)),
                        (BinOp::And, _, Some(k)) if k == all => Some((**a).clone()),
                        (BinOp::And, Some(k), _) if k == all => Some((**b).clone()),
                        _ => None,
                    }
                }
            }
            Ex::Un(UnOp::BitNot, a, sc) => a.konst().map(|k| Ex::Const(sc.wrap(!k), *sc)),
            Ex::Un(UnOp::Neg, a, sc) => a.konst().map(|k| Ex::Const(sc.wrap(-k), *sc)),
            Ex::Cast(a, from, to) => {
                if let Some(k) = a.konst() {
                    Some(Ex::Const(to.wrap(k), *to))
                } else if from == to {
                    Some((**a).clone())
                } else if let Ex::Cast(inner, f2, _) = &**a {
                    // Widen then narrow back.
                    if f2 == to && from.size() > to.size() {
                        Some((**inner).clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(n) = new {
            *x = n;
        }
    });
}

/// Replace uses of a variable inside an expression.
fn substitute(e: &mut Ex, v: VarId, with: &Ex) {
    e.walk_mut(&mut |x| {
        if let Ex::Var(w, _) = x {
            if *w == v {
                *x = with.clone();
            }
        }
    });
}

fn place_reads_var(p: &Place, v: VarId) -> bool {
    let mut l = Live::new();
    place_uses(p, &mut l);
    l.contains(&v)
}

/// Does a statement write memory that `e` may read?
fn clobbers(s: &St, e: &Ex, objs: &[Obj]) -> bool {
    let reads_obj = |o: ObjId| {
        let mut hit = false;
        e.walk(&mut |x| {
            if let Ex::Load(p, _, _) = x {
                if *p == o {
                    hit = true;
                }
            }
        });
        hit
    };
    match s {
        St::Assign(Place::Mem(o, _), _) => reads_obj(*o),
        St::Assign(Place::Deref(..) | Place::Poke(_), _) => e.reads_memory(),
        St::Assign(Place::Var(_), _) | St::Line(_) => false,
        St::Call { .. } | St::Asm { .. } => e.reads_memory() || e.volatile(objs),
    }
}


/// Uses and definitions of every variable in a function.
fn use_counts(inst: &Inst) -> (BTreeMap<VarId, usize>, BTreeMap<VarId, usize>) {
    let mut count: BTreeMap<VarId, usize> = BTreeMap::new();
    let mut defs: BTreeMap<VarId, usize> = BTreeMap::new();
    for block in &inst.blocks {
        for s in &block.stmts {
            let (u, d) = stmt_effects(s);
            let mut tally = |e: &Ex| {
                e.walk(&mut |x| {
                    if let Ex::Var(v, _) = x {
                        *count.entry(*v).or_default() += 1;
                    }
                })
            };
            match s {
                St::Assign(p, e) => {
                    tally(e);
                    if let Place::Mem(_, Some(i)) = p {
                        tally(i);
                    }
                    if let Place::Deref(a, c) = p {
                        tally(a);
                        tally(c);
                    }
                    if let Place::Poke(a) = p {
                        tally(a);
                    }
                }
                St::Call { args, .. } => args.iter().for_each(&mut tally),
                St::Asm { inputs, .. } => inputs.iter().for_each(|(_, e)| tally(e)),
                St::Line(_) => {}
            }
            for v in u {
                if !matches!(s, St::Assign(..) | St::Call { .. }) {
                    *count.entry(v).or_default() += 100;
                }
            }
            for v in d {
                *defs.entry(v).or_default() += 1;
            }
        }
        if let Term::Branch(c, _, _) = &block.term {
            c.walk(&mut |x| {
                if let Ex::Var(v, _) = x {
                    *count.entry(*v).or_default() += 1;
                }
            });
        }
    }
    for r in &inst.rets {
        *count.entry(*r).or_default() += 100;
    }
    (count, defs)
}

/// Within a block: constant/copy propagation and forward substitution of
/// temporaries used once.
fn propagate(inst: &mut Inst, b: usize, objs: &[Obj]) -> bool {
    let mut changed = false;
    let pointers: BTreeSet<VarId> = inst
        .vars
        .iter()
        .enumerate()
        .filter(|(_, v)| v.pointer)
        .map(|(i, _)| VarId(i as u32))
        .collect();
    let block = &mut inst.blocks[b];
    // Constants and copies known at each point (variables only).
    let mut known: BTreeMap<VarId, Ex> = BTreeMap::new();
    let n = block.stmts.len();
    let mut k = 0;
    while k < n {
        // Rewrite uses with known values.
        {
            let s = &mut block.stmts[k];
            let rewrite = |e: &mut Ex, known: &BTreeMap<VarId, Ex>, changed: &mut bool| {
                for (v, val) in known {
                    if e.reads_var(*v) {
                        substitute(e, *v, val);
                        *changed = true;
                    }
                }
                simplify(e);
            };
            match s {
                St::Assign(p, e) => {
                    rewrite(e, &known, &mut changed);
                    if let Place::Mem(_, Some(i)) = p {
                        rewrite(i, &known, &mut changed);
                    }
                }
                St::Call { args, .. } => {
                    for a in args.iter_mut() {
                        rewrite(a, &known, &mut changed);
                    }
                }
                St::Asm { inputs, .. } => {
                    for (_, e) in inputs.iter_mut() {
                        rewrite(e, &known, &mut changed);
                    }
                }
                St::Line(_) => {}
            }
        }
        let (_, d) = stmt_effects(&block.stmts[k]);
        // Assembly may write the variables it names.
        if let St::Asm { operands, .. } = &block.stmts[k] {
            for o in operands {
                if let AsmOperand::Var(v) = o {
                    known.remove(v);
                    known.retain(|_, e| !e.reads_var(*v));
                }
            }
        }
        for v in &d {
            known.remove(v);
            known.retain(|_, e| !e.reads_var(*v));
        }
        // Memory writes invalidate copies of memory.
        let s = block.stmts[k].clone();
        known.retain(|_, e| !clobbers(&s, e, objs));
        if let St::Assign(Place::Var(v), e) = &block.stmts[k] {
            let v = *v;
            let simple = matches!(e, Ex::Const(..) | Ex::Addr(..)) || matches!(e, Ex::Var(w, _) if *w != v);
            if simple && !pointers.contains(&v) && !e.reads_var(v) {
                known.insert(v, e.clone());
            }
        }
        k += 1;
    }
    // Counts after copy propagation, which may have added uses.
    let block_index = b;
    let (count, defs) = use_counts(inst);
    let block = &mut inst.blocks[block_index];
    // Forward substitution: `t = e; ... use(t)` where t is a single-use
    // temporary defined once, and nothing in between changes e's inputs.
    let mut k = 0;
    while k < block.stmts.len() {
        let St::Assign(Place::Var(t), e) = &block.stmts[k] else {
            k += 1;
            continue;
        };
        let t = *t;
        let e = e.clone();
        let temp = inst.vars[t.0 as usize].temp;
        if !temp || count.get(&t).copied().unwrap_or(0) != 1 || defs.get(&t).copied().unwrap_or(0) != 1 || e.volatile(objs) || pointers.contains(&t) {
            k += 1;
            continue;
        }
        let mut inputs = Live::new();
        ex_uses(&e, &mut inputs);
        // Find the use.
        let mut j = k + 1;
        let mut done = false;
        while j < block.stmts.len() {
            let s = &block.stmts[j];
            let uses_t = {
                let (u, _) = stmt_effects(s);
                u.contains(&t)
            };
            if uses_t {
                // Only into plain assignments and call arguments.
                let ok = match s {
                    St::Assign(p, val) => val.reads_var(t) && !place_reads_var(p, t) || place_reads_var(p, t) && !val.reads_var(t),
                    St::Call { args, .. } => args.iter().filter(|a| a.reads_var(t)).count() == 1 && !e.reads_memory(),
                    _ => false,
                };
                if ok {
                    let s = &mut block.stmts[j];
                    match s {
                        St::Assign(p, val) => {
                            substitute(val, t, &e);
                            if let Place::Mem(_, Some(i)) = p {
                                substitute(i, t, &e);
                            }
                            simplify(val);
                        }
                        St::Call { args, .. } => {
                            for a in args.iter_mut() {
                                substitute(a, t, &e);
                            }
                        }
                        _ => {}
                    }
                    block.stmts[k] = St::Line(crate::source::Span::default());
                    changed = true;
                }
                done = true;
                break;
            }
            let (_, d) = stmt_effects(s);
            if d.iter().any(|v| inputs.contains(v)) || clobbers(s, &e, objs) {
                break;
            }
            j += 1;
        }
        if !done && j >= block.stmts.len() {
            // Used by the terminator.
            let used_in_term = matches!(&block.term, Term::Branch(c, _, _) if c.reads_var(t));
            if used_in_term {
                if let Term::Branch(c, _, _) = &mut block.term {
                    substitute(c, t, &e);
                    simplify(c);
                }
                block.stmts[k] = St::Line(crate::source::Span::default());
                changed = true;
            }
        }
        k += 1;
    }
    if let Term::Branch(c, _, _) = &mut block.term {
        for (v, val) in &known {
            if c.reads_var(*v) {
                substitute(c, *v, val);
                changed = true;
            }
        }
        simplify(c);
    }
    block.stmts.retain(|s| !matches!(s, St::Line(sp) if *sp == crate::source::Span::default()));
    changed
}

/// Remove assignments to variables that are never read afterwards.
fn dead_assignments(inst: &mut Inst, objs: &[Obj]) -> bool {
    let (_, live_out) = liveness(inst);
    let mut changed = false;
    let rets: BTreeSet<VarId> = inst.rets.iter().copied().collect();
    let asm_vars: BTreeSet<VarId> = inst
        .blocks
        .iter()
        .flat_map(|b| b.stmts.iter())
        .flat_map(|s| match s {
            St::Asm { operands, .. } => operands
                .iter()
                .filter_map(|o| if let AsmOperand::Var(v) = o { Some(*v) } else { None })
                .collect::<Vec<_>>(),
            _ => vec![],
        })
        .collect();
    let params: BTreeSet<VarId> = inst.params.iter().copied().collect();
    let term_live: Vec<Live> = inst
        .blocks
        .iter()
        .map(|b| {
            let mut l = Live::new();
            term_uses(&b.term, inst, &mut l);
            l
        })
        .collect();
    for (b, block) in inst.blocks.iter_mut().enumerate() {
        let mut live = live_out[b].clone();
        live.extend(term_live[b].iter().copied());
        let mut keep = vec![true; block.stmts.len()];
        for (k, s) in block.stmts.iter().enumerate().rev() {
            let (u, d) = stmt_effects(s);
            if let St::Assign(Place::Var(v), e) = s {
                if !live.contains(v) && !rets.contains(v) && !asm_vars.contains(v) && !e.volatile(objs) && !params.contains(v) {
                    keep[k] = false;
                    changed = true;
                    continue;
                }
            }
            for v in d {
                live.remove(&v);
            }
            live.extend(u);
        }
        let mut i = 0;
        block.stmts.retain(|_| {
            i += 1;
            keep[i - 1]
        });
    }
    changed
}

/// Jump threading, constant branches, unreachable blocks, block merging.
fn simplify_cfg(inst: &mut Inst) -> bool {
    let mut changed = false;
    let n = inst.blocks.len();
    // Constant branches.
    for b in &mut inst.blocks {
        if let Term::Branch(c, t, f) = &mut b.term {
            simplify(c);
            if let Some(k) = c.konst() {
                b.term = Term::Goto(if k != 0 { *t } else { *f });
                changed = true;
            } else if t == f {
                b.term = Term::Goto(*t);
                changed = true;
            }
        }
    }
    // Thread empty blocks that only jump.
    let target_of = |inst: &Inst, mut b: BlockId| {
        let mut seen = 0;
        while seen < n {
            let block = &inst.blocks[b.0 as usize];
            match block.term {
                Term::Goto(t) if block.stmts.iter().all(|s| matches!(s, St::Line(_))) && t != b => b = t,
                _ => break,
            }
            seen += 1;
        }
        b
    };
    for i in 0..n {
        let term = inst.blocks[i].term.clone();
        let new = match term {
            Term::Goto(t) => Term::Goto(target_of(inst, t)),
            Term::Branch(c, t, f) => Term::Branch(c, target_of(inst, t), target_of(inst, f)),
            t => t,
        };
        if format!("{:?}", new) != format!("{:?}", inst.blocks[i].term) {
            changed = true;
            inst.blocks[i].term = new;
        }
    }
    inst.entry = target_of(inst, inst.entry);
    // Merge a block into its single predecessor's goto.
    let mut preds = vec![0usize; n];
    let mut reach = vec![false; n];
    let mut stack = vec![inst.entry];
    while let Some(b) = stack.pop() {
        if reach[b.0 as usize] {
            continue;
        }
        reach[b.0 as usize] = true;
        for s in inst.successors(b) {
            stack.push(s);
        }
    }
    for b in 0..n {
        if !reach[b] {
            continue;
        }
        for s in inst.successors(BlockId(b as u32)) {
            preds[s.0 as usize] += 1;
        }
    }
    for b in 0..n {
        if !reach[b] {
            if !inst.blocks[b].stmts.is_empty() || !matches!(inst.blocks[b].term, Term::Return) {
                inst.blocks[b].stmts.clear();
                if !matches!(inst.blocks[b].term, Term::Return) {
                    inst.blocks[b].term = Term::Return;
                    changed = true;
                }
            }
            continue;
        }
        while let Term::Goto(t) = inst.blocks[b].term {
            let ti = t.0 as usize;
            if ti == b || preds[ti] != 1 || ti == inst.entry.0 as usize || matches!(inst.blocks[ti].term, Term::Return) {
                break;
            }
            let moved = std::mem::take(&mut inst.blocks[ti].stmts);
            let term = std::mem::replace(&mut inst.blocks[ti].term, Term::Return);
            inst.blocks[b].stmts.extend(moved);
            inst.blocks[b].term = term;
            preds[ti] = 0;
            reach[ti] = false;
            changed = true;
        }
    }
    changed
}

// ------------------------------------------------------------------ inline

fn inline_cost(inst: &Inst) -> usize {
    inst.blocks
        .iter()
        .map(|b| {
            b.stmts
                .iter()
                .map(|s| match s {
                    St::Assign(_, e) => 1 + e.size(),
                    St::Call { args, .. } => 4 + args.len(),
                    St::Asm { .. } => 100,
                    St::Line(_) => 0,
                })
                .sum::<usize>()
                + match &b.term {
                    Term::Branch(c, _, _) => 1 + c.size(),
                    _ => 0,
                }
        })
        .sum()
}

fn inline(mir: &mut Mir) {
    for _round in 0..3 {
        let mut changed = false;
        for caller in 0..mir.insts.len() {
            let mut b = 0;
            while b < mir.insts[caller].blocks.len() {
                let mut k = 0;
                while k < mir.insts[caller].blocks[b].stmts.len() {
                    let St::Call { target, .. } = &mir.insts[caller].blocks[b].stmts[k] else {
                        k += 1;
                        continue;
                    };
                    let target = *target;
                    let callee = &mir.insts[target.0 as usize];
                    let small = inline_cost(callee) <= 6 && callee.blocks.len() <= 3;
                    let eligible = target.0 as usize != caller
                        && callee.recursive.is_none()
                        && !callee.noinline
                        && (callee.inline || small)
                        && callee.interrupt.is_none()
                        && !callee.has_asm
                        && callee.bank == mir.insts[caller].bank
                        && !callee.blocks.is_empty();
                    if !eligible {
                        k += 1;
                        continue;
                    }
                    let callee = callee.clone();
                    inline_at(&mut mir.insts[caller], b, k, &callee);
                    changed = true;
                    // Continue in the block that follows the call.
                    break;
                }
                b += 1;
            }
        }
        if !changed {
            break;
        }
    }
    // Instances no longer called anywhere are dropped.
    let mut called = BTreeSet::new();
    for inst in &mir.insts {
        for blk in &inst.blocks {
            for s in &blk.stmts {
                match s {
                    St::Call { target, .. } => {
                        called.insert(*target);
                    }
                    St::Asm { operands, .. } => {
                        for o in operands {
                            if let AsmOperand::Fn(f) = o {
                                called.insert(*f);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Transitively reachable from roots.
    let mut reach: BTreeSet<InstId> = BTreeSet::new();
    let mut stack: Vec<InstId> = mir
        .insts
        .iter()
        .enumerate()
        .filter(|(_, i)| i.reset || i.interrupt.is_some())
        .map(|(k, _)| InstId(k as u32))
        .collect();
    while let Some(i) = stack.pop() {
        if !reach.insert(i) {
            continue;
        }
        for blk in &mir.insts[i.0 as usize].blocks {
            for s in &blk.stmts {
                match s {
                    St::Call { target, .. } => stack.push(*target),
                    St::Asm { operands, .. } => {
                        for o in operands {
                            if let AsmOperand::Fn(f) = o {
                                stack.push(*f);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = called;
    for (k, inst) in mir.insts.iter_mut().enumerate() {
        if !reach.contains(&InstId(k as u32)) {
            inst.blocks.clear();
        }
    }
}

fn inline_at(caller: &mut Inst, b: usize, k: usize, callee: &Inst) {
    let St::Call { args, rets, .. } = caller.blocks[b].stmts[k].clone() else { return };
    // Fresh variables for the callee's.
    let base = caller.vars.len() as u32;
    for v in &callee.vars {
        let mut v = v.clone();
        v.name = format!("{}.{}", callee.name.rsplit("::").next().unwrap_or(""), v.name);
        v.temp = true;
        caller.vars.push(v);
    }
    let map_var = |v: VarId| VarId(v.0 + base);
    let block_base = caller.blocks.len() as u32;
    let map_block = |x: BlockId| BlockId(x.0 + block_base);
    let depth = caller.blocks[b].depth;
    // Split the caller block.
    let tail_stmts = caller.blocks[b].stmts.split_off(k + 1);
    caller.blocks[b].stmts.pop();
    let tail_term = std::mem::replace(&mut caller.blocks[b].term, Term::Goto(map_block(callee.entry)));
    let cont = BlockId(block_base + callee.blocks.len() as u32);
    // Arguments.
    for (p, a) in callee.params.iter().zip(args) {
        caller.blocks[b].stmts.push(St::Assign(Place::Var(map_var(*p)), a));
    }
    let rename_ex = |e: &mut Ex| {
        e.walk_mut(&mut |x| {
            if let Ex::Var(v, _) = x {
                *v = map_var(*v);
            }
        });
    };
    let rename_place = |p: &mut Place| match p {
        Place::Var(v) => *v = map_var(*v),
        Place::Mem(_, Some(i)) => rename_ex(i),
        Place::Deref(a, c) => {
            rename_ex(a);
            rename_ex(c);
        }
        Place::Poke(a) => rename_ex(a),
        _ => {}
    };
    for blk in &callee.blocks {
        let mut nb = blk.clone();
        nb.depth += depth;
        for s in nb.stmts.iter_mut() {
            match s {
                St::Assign(p, e) => {
                    rename_place(p);
                    rename_ex(e);
                }
                St::Call { args, rets, saved, .. } => {
                    args.iter_mut().for_each(rename_ex);
                    rets.iter_mut().for_each(rename_place);
                    saved.iter_mut().for_each(|v| *v = map_var(*v));
                }
                St::Asm { inputs, output, operands, .. } => {
                    inputs.iter_mut().for_each(|(_, e)| rename_ex(e));
                    if let Some(p) = output {
                        rename_place(p);
                    }
                    for o in operands.iter_mut() {
                        if let AsmOperand::Var(v) = o {
                            *v = map_var(*v);
                        }
                    }
                }
                St::Line(_) => {}
            }
        }
        nb.term = match nb.term {
            Term::Goto(t) => Term::Goto(map_block(t)),
            Term::Branch(mut c, t, f) => {
                rename_ex(&mut c);
                Term::Branch(c, map_block(t), map_block(f))
            }
            Term::Return => Term::Goto(cont),
            Term::Open => Term::Goto(cont),
        };
        caller.blocks.push(nb);
    }
    // Continuation: results, then the rest of the original block.
    let mut stmts = Vec::new();
    for (r, cr) in rets.iter().zip(&callee.rets) {
        let sc = callee.vars[cr.0 as usize].sc;
        stmts.push(St::Assign(r.clone(), Ex::Var(map_var(*cr), sc)));
    }
    stmts.extend(tail_stmts);
    caller.blocks.push(Block {
        stmts,
        term: tail_term,
        depth,
    });
}
