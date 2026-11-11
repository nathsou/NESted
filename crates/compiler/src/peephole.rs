//! Machine-level optimization of one function's structured 6502 code:
//! register-content tracking, flag liveness, dead temporary stores, jump
//! threading and tail calls.
use crate::asm::{Arg, Ins, Line, Op, Opnd, Sym};
use crate::mir::{InstId, Mir, VarId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Imm(i64),
    Mem(Opnd),
}

#[derive(Clone, Debug, Default)]
struct Regs {
    a: Option<Val>,
    x: Option<Val>,
    y: Option<Val>,
}

impl Regs {
    fn forget_mem(&mut self, pred: impl Fn(&Opnd) -> bool) {
        for r in [&mut self.a, &mut self.x, &mut self.y] {
            if let Some(Val::Mem(m)) = r {
                if pred(m) {
                    *r = None;
                }
            }
        }
    }
}

fn reads_nz(op: Op) -> bool {
    matches!(op, Op::Beq | Op::Bne | Op::Bmi | Op::Bpl | Op::Php)
}

fn writes_nz(ins: &Ins) -> bool {
    use Op::*;
    match ins.op {
        Lda | Ldx | Ldy | Adc | Sbc | And | Ora | Eor | Cmp | Cpx | Cpy | Bit | Inc | Dec | Inx | Iny | Dex | Dey
        | Asl | Lsr | Rol | Ror | Tax | Tay | Txa | Tya | Tsx | Pla | Plp => true,
        _ => false,
    }
}

/// Instruction sets N/Z from the value now in A.
fn nz_from_a(ins: &Ins) -> bool {
    use Op::*;
    match ins.op {
        Lda | Adc | Sbc | And | Ora | Eor | Txa | Tya | Pla => true,
        Asl | Lsr | Rol | Ror => ins.arg == Arg::Acc,
        _ => false,
    }
}

fn same_place(a: &Opnd, b: &Opnd) -> bool {
    a == b
}

fn may_alias(store: &Arg, m: &Opnd) -> bool {
    match store {
        Arg::Mem(o) => same_place(o, m) || (matches!(o.sym, Sym::Num) && matches!(m.sym, Sym::Num) && o.off == m.off),
        Arg::MemX(o) | Arg::MemY(o) => o.sym == m.sym || matches!((&o.sym, &m.sym), (Sym::Num, Sym::Num)),
        Arg::IndX(_) | Arg::IndY(_) | Arg::Ind(_) => true,
        _ => false,
    }
}

pub fn optimize(lines: &mut Vec<Line>, mir: &Mir, inst: InstId) {
    for _ in 0..6 {
        let before = lines.len();
        let mut changed = false;
        changed |= jumps(lines);
        let volatile = |o: &Opnd| match o.sym {
            Sym::Num => o.off >= 0x2000,
            Sym::Obj(id, _) => mir.objs[id.0 as usize].volatile(),
            _ => false,
        };
        changed |= registers(lines, &volatile);
        changed |= dead_stores(lines, mir, inst);
        changed |= tails(lines);
        if !changed && lines.len() == before {
            break;
        }
    }
}

/// Flag liveness: for each instruction, whether its N/Z result is read.
fn nz_needed(lines: &[Line]) -> Vec<bool> {
    let mut at_label: BTreeMap<String, bool> = BTreeMap::new();
    let target = |ins: &Ins| match ins.arg.opnd() {
        Some(Opnd { sym: Sym::Label(n), .. }) => Some(n.clone()),
        _ => None,
    };
    let mut need = vec![true; lines.len()];
    for _round in 0..8 {
        let mut changed = false;
        let mut live = false;
        for i in (0..lines.len()).rev() {
            match &lines[i] {
                Line::Ins(ins) => {
                    if matches!(ins.op, Op::Rts | Op::Rti | Op::Jsr) {
                        live = false;
                        need[i] = false;
                        continue;
                    }
                    if ins.op == Op::Jmp {
                        live = match (&ins.arg, target(ins)) {
                            (Arg::Mem(_), Some(t)) => at_label.get(&t).copied().unwrap_or(!t.starts_with("__f")),
                            _ => true,
                        };
                        need[i] = live;
                        continue;
                    }
                    need[i] = live;
                    if writes_nz(ins) {
                        live = false;
                    }
                    if reads_nz(ins.op) {
                        live = true;
                    }
                    if ins.op.is_branch() {
                        if let Some(t) = target(ins) {
                            live |= at_label.get(&t).copied().unwrap_or(!t.starts_with("__f"));
                        }
                    }
                }
                Line::Label(n) => {
                    if at_label.get(n) != Some(&live) {
                        let old = at_label.get(n).copied().unwrap_or(false);
                        at_label.insert(n.clone(), live || old);
                        if live && !old {
                            changed = true;
                        }
                    }
                }
                Line::Barrier => live = true,
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    need
}

fn registers(lines: &mut Vec<Line>, volatile: &dyn Fn(&Opnd) -> bool) -> bool {
    let need = nz_needed(lines);
    let mut regs = Regs::default();
    let mut remove = vec![false; lines.len()];
    for i in 0..lines.len() {
        let line = lines[i].clone();
        match &line {
            Line::Label(_) | Line::Barrier | Line::Bytes(_) | Line::Words(_) | Line::Fill(_) => {
                regs = Regs::default();
                continue;
            }
            Line::Src(..) => continue,
            Line::Ins(ins) => {
                let val = |arg: &Arg| -> Option<Val> {
                    match arg {
                        Arg::Imm(o) => o.number().map(Val::Imm),
                        Arg::Mem(o) => Some(Val::Mem(o.clone())),
                        _ => None,
                    }
                };
                use Op::*;
                match ins.op {
                    Lda | Ldx | Ldy => {
                        let v = val(&ins.arg);
                        let reg = match ins.op {
                            Lda => &mut regs.a,
                            Ldx => &mut regs.x,
                            _ => &mut regs.y,
                        };
                        // Volatile hardware reads always happen.
                        let volatile = matches!(&ins.arg, Arg::Mem(o) if volatile(o));
                        if v.is_some() && *reg == v && !need[i] && !volatile {
                            remove[i] = true;
                            continue;
                        }
                        *reg = if volatile { None } else { v };
                        // A transfer can satisfy the load: lda #k while X == #k.
                    }
                    Sta | Stx | Sty => {
                        let src = match ins.op {
                            Sta => regs.a.clone(),
                            Stx => regs.x.clone(),
                            _ => regs.y.clone(),
                        };
                        // Storing a value back where it came from.
                        if let (Arg::Mem(m), Some(Val::Mem(r))) = (&ins.arg, &src) {
                            if m == r && !volatile(m) {
                                remove[i] = true;
                                continue;
                            }
                        }
                        let arg = ins.arg.clone();
                        regs.forget_mem(|m| may_alias(&arg, m));
                        if let Arg::Mem(m) = &ins.arg {
                            if !volatile(m) {
                                let reg = match ins.op {
                                    Sta => &mut regs.a,
                                    Stx => &mut regs.x,
                                    _ => &mut regs.y,
                                };
                                if reg.is_none() {
                                    *reg = Some(Val::Mem(m.clone()));
                                }
                            }
                        }
                    }
                    Tax => {
                        if regs.x.is_some() && regs.x == regs.a && !need[i] {
                            remove[i] = true;
                            continue;
                        }
                        regs.x = regs.a.clone();
                    }
                    Tay => {
                        if regs.y.is_some() && regs.y == regs.a && !need[i] {
                            remove[i] = true;
                            continue;
                        }
                        regs.y = regs.a.clone();
                    }
                    Txa => {
                        if regs.a.is_some() && regs.a == regs.x && !need[i] {
                            remove[i] = true;
                            continue;
                        }
                        regs.a = regs.x.clone();
                    }
                    Tya => {
                        if regs.a.is_some() && regs.a == regs.y && !need[i] {
                            remove[i] = true;
                            continue;
                        }
                        regs.a = regs.y.clone();
                    }
                    Inx | Dex => {
                        regs.x = match regs.x {
                            Some(Val::Imm(k)) => Some(Val::Imm((k + if ins.op == Inx { 1 } else { -1 }) & 0xff)),
                            _ => None,
                        }
                    }
                    Iny | Dey => {
                        regs.y = match regs.y {
                            Some(Val::Imm(k)) => Some(Val::Imm((k + if ins.op == Iny { 1 } else { -1 }) & 0xff)),
                            _ => None,
                        }
                    }
                    Inc | Dec | Asl | Lsr | Rol | Ror => {
                        if ins.arg == Arg::Acc {
                            regs.a = None;
                        } else {
                            let arg = ins.arg.clone();
                            regs.forget_mem(|m| may_alias(&arg, m));
                        }
                    }
                    Cmp | Cpx | Cpy | Bit | Clc | Sec | Cld | Cli | Clv | Sed | Sei | Nop | Php | Pha => {
                        // `cmp #0` right after A was set by a flag-setting op.
                        if ins.op == Cmp && ins.arg == Arg::Imm(Opnd::num(0)) {
                            let prev = (0..i).rev().find(|j| matches!(lines[*j], Line::Ins(_)) || matches!(lines[*j], Line::Label(_) | Line::Barrier));
                            if let Some(j) = prev {
                                if let Line::Ins(p) = &lines[j] {
                                    if nz_from_a(p) && !remove[j] {
                                        // The carry differs (cmp #0 sets C); only N/Z readers follow?
                                        let next_c = lines[i + 1..].iter().find_map(|l| match l {
                                            Line::Ins(n) => Some(n.op),
                                            Line::Label(_) | Line::Barrier => Some(Op::Jmp),
                                            _ => None,
                                        });
                                        if matches!(next_c, Some(Beq | Bne | Bmi | Bpl)) {
                                            remove[i] = true;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Adc | Sbc | And | Ora | Eor | Pla => regs.a = None,
                    Plp | Rti | Rts | Brk => regs = Regs::default(),
                    Jsr => regs = Regs::default(),
                    Jmp => regs = Regs::default(),
                    Txs | Tsx => {
                        if ins.op == Tsx {
                            regs.x = None
                        }
                    }
                    Bcc | Bcs | Beq | Bmi | Bne | Bpl | Bvc | Bvs => {}
                }
            }
        }
    }
    let changed = remove.iter().any(|r| *r);
    let mut i = 0;
    lines.retain(|_| {
        i += 1;
        !remove[i - 1]
    });
    changed
}

/// Stores to this function's temporaries that are never read.
fn dead_stores(lines: &mut Vec<Line>, mir: &Mir, inst: InstId) -> bool {
    let me = &mir.insts[inst.0 as usize];
    let params: BTreeSet<u32> = me.params.iter().map(|v| v.0).collect();
    // Multi-value results are read by callers.
    let shared_rets: BTreeSet<u32> = if me.rets.len() > 1 { me.rets.iter().map(|v| v.0).collect() } else { BTreeSet::new() };
    let mut read: BTreeMap<(u32, i64), usize> = BTreeMap::new();
    let mut written_only: BTreeSet<u32> = BTreeSet::new();
    let mut any_asm = false;
    for l in lines.iter() {
        match l {
            Line::Barrier => any_asm = true,
            Line::Ins(ins) => {
                if let Some(Opnd { sym: Sym::Var(i, v), off, .. }) = ins.arg.opnd() {
                    if *i != inst {
                        continue;
                    }
                    let store = matches!(ins.op, Op::Sta | Op::Stx | Op::Sty) && matches!(ins.arg, Arg::Mem(_));
                    if store {
                        written_only.insert(v.0);
                    } else {
                        *read.entry((v.0, *off)).or_default() += 1;
                        if matches!(ins.arg, Arg::IndY(_) | Arg::IndX(_)) {
                            *read.entry((v.0, off + 1)).or_default() += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let _ = any_asm;
    let n_vars = me.vars.len() as u32;
    let mut changed = false;
    lines.retain(|l| {
        if let Line::Ins(ins) = l {
            if matches!(ins.op, Op::Sta | Op::Stx | Op::Sty) {
                if let Arg::Mem(Opnd { sym: Sym::Var(i, v), off, .. }) = &ins.arg {
                    let temp = v.0 >= n_vars || me.vars[v.0 as usize].temp || (me.rets.len() == 1 && me.rets[0] == VarId(v.0));
                    let user_named = v.0 < n_vars && !me.vars[v.0 as usize].temp;
                    let _ = user_named;
                    if *i == inst && temp && !params.contains(&v.0) && !shared_rets.contains(&v.0) && !read.contains_key(&(v.0, *off)) && !me.vars.get(v.0 as usize).is_some_and(|x| x.pointer) {
                        changed = true;
                        return false;
                    }
                }
            }
        }
        true
    });
    changed
}

/// Jump threading, unreachable code, jumps to the next line.
fn jumps(lines: &mut Vec<Line>) -> bool {
    let mut changed = false;
    // Label → index of the first instruction after it.
    let index = |lines: &[Line]| {
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for (i, l) in lines.iter().enumerate() {
            if let Line::Label(n) = l {
                m.insert(n.clone(), i);
            }
        }
        m
    };
    let first_ins = |lines: &[Line], mut i: usize| -> Option<Ins> {
        while i < lines.len() {
            match &lines[i] {
                Line::Ins(ins) => return Some(ins.clone()),
                Line::Label(_) | Line::Src(..) => i += 1,
                _ => return None,
            }
        }
        None
    };
    let labels = index(lines);
    for i in 0..lines.len() {
        let Line::Ins(ins) = &lines[i] else { continue };
        let Some(Opnd { sym: Sym::Label(target), .. }) = ins.arg.opnd() else { continue };
        if !(ins.op.is_branch() || ins.op == Op::Jmp) || !matches!(ins.arg, Arg::Mem(_)) {
            continue;
        }
        let mut t = target.clone();
        let mut hops = 0;
        while let Some(&at) = labels.get(&t) {
            match first_ins(lines, at) {
                Some(Ins { op: Op::Jmp, arg: Arg::Mem(Opnd { sym: Sym::Label(n), .. }) }) if hops < 8 && n != t => {
                    t = n;
                    hops += 1;
                }
                Some(Ins { op: Op::Rts, .. }) if ins.op == Op::Jmp => {
                    lines[i] = Line::Ins(Ins::imp(Op::Rts));
                    changed = true;
                    break;
                }
                _ => break,
            }
        }
        if let Line::Ins(ins) = &mut lines[i] {
            if let Some(o) = ins.arg.opnd_mut() {
                if let Sym::Label(n) = &o.sym {
                    if *n != t {
                        o.sym = Sym::Label(t);
                        changed = true;
                    }
                }
            }
        }
    }
    // Branch over jump: `bxx L1; jmp L2; L1:` → `b!xx L2; L1:`.
    let mut i = 0;
    while i + 2 < lines.len() {
        if let (Line::Ins(b), Line::Ins(j), Line::Label(l1)) = (&lines[i], &lines[i + 1], &lines[i + 2]) {
            if b.op.is_branch() && j.op == Op::Jmp && matches!(j.arg, Arg::Mem(_)) {
                if let Some(Opnd { sym: Sym::Label(t), .. }) = b.arg.opnd() {
                    if t == l1 {
                        let inv = Ins::new(b.op.inverse(), j.arg.clone());
                        lines[i] = Line::Ins(inv);
                        lines.remove(i + 1);
                        changed = true;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    // Jumps to the immediately following label; code after unconditional transfers.
    let mut out: Vec<Line> = Vec::new();
    let mut dead = false;
    for (i, l) in lines.iter().enumerate() {
        match l {
            Line::Label(_) | Line::Barrier | Line::Bytes(_) | Line::Words(_) | Line::Fill(_) => {
                dead = false;
                out.push(l.clone());
            }
            Line::Ins(ins) if dead => {
                let _ = ins;
                changed = true;
            }
            Line::Ins(ins) => {
                if (ins.op == Op::Jmp || ins.op.is_branch()) && matches!(ins.arg, Arg::Mem(_)) {
                    if let Some(Opnd { sym: Sym::Label(t), .. }) = ins.arg.opnd() {
                        let mut k = i + 1;
                        let mut falls = false;
                        while k < lines.len() {
                            match &lines[k] {
                                Line::Label(n) if n == t => {
                                    falls = true;
                                    break;
                                }
                                Line::Label(_) | Line::Src(..) => k += 1,
                                _ => break,
                            }
                        }
                        if falls {
                            changed = true;
                            continue;
                        }
                    }
                }
                out.push(l.clone());
                if matches!(ins.op, Op::Jmp | Op::Rts | Op::Rti) {
                    dead = true;
                }
            }
            _ => out.push(l.clone()),
        }
    }
    // Labels that nothing references (keep function entry labels).
    let mut used: BTreeSet<String> = BTreeSet::new();
    for l in &out {
        match l {
            Line::Ins(ins) => {
                if let Some(Opnd { sym: Sym::Label(n), .. }) = ins.arg.opnd() {
                    used.insert(n.clone());
                }
            }
            Line::Bytes(v) | Line::Words(v) => {
                for o in v {
                    if let Sym::Label(n) = &o.sym {
                        used.insert(n.clone());
                    }
                }
            }
            _ => {}
        }
    }
    let before = out.len();
    let mut in_asm = false;
    out.retain(|l| match l {
        Line::Barrier => {
            in_asm = !in_asm;
            true
        }
        Line::Label(n) => in_asm || used.contains(n) || !n.contains("_b") && !n.contains("_l") || !n.starts_with("__f"),
        _ => true,
    });
    if out.len() != before {
        changed = true;
    }
    *lines = out;
    changed
}

/// `jsr f; rts` → `jmp f`.
fn tails(lines: &mut Vec<Line>) -> bool {
    let mut changed = false;
    for i in 0..lines.len().saturating_sub(1) {
        if let (Line::Ins(a), Line::Ins(b)) = (&lines[i], &lines[i + 1]) {
            if a.op == Op::Jsr && b.op == Op::Rts {
                let target = a.arg.clone();
                lines[i] = Line::Ins(Ins::new(Op::Jmp, target));
                changed = true;
            }
        }
    }
    changed
}
