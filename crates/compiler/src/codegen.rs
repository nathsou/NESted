//! Instruction selection from MIR to structured 6502 code.
//!
//! Values are byte-oriented: an 8-bit expression is computed into A with
//! memory, immediate and indexed operands used directly; 16-bit values are
//! computed byte by byte into memory. X and Y index arrays and carry loop
//! state; register contents are cleaned up by the machine optimizer.
use crate::asm::{Arg, Ins, Line, Op, Opnd, Sym};
use crate::hir::{BinOp, Context, UnOp};
use crate::mir::*;
use std::collections::{BTreeMap, BTreeSet};

/// How bank switching works for the target board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banking {
    None,
    /// One switchable window shared by code and data.
    Shared,
    /// Separate code ($8000) and data ($A000) windows (MMC3).
    Split,
}

pub struct Output {
    pub lines: Vec<Line>,
    /// Extra frame variables created during selection.
    pub extra_vars: Vec<Var>,
    /// Helper routines this function calls (`__mul8` and so on).
    pub helpers: BTreeSet<&'static str>,
    pub far_calls: BTreeSet<InstId>,
    pub errors: Vec<(crate::source::Span, String)>,
}

pub fn fn_label(inst: InstId) -> String {
    format!("__f{}", inst.0)
}

pub fn helper_label(name: &str, ctx: Context) -> String {
    format!("{name}_{}", match ctx {
        Context::Main => "m",
        Context::Nmi => "n",
        Context::Irq => "i",
    })
}

struct Gen<'a> {
    mir: &'a Mir,
    inst: &'a Inst,
    id: InstId,
    out: Vec<Line>,
    labels: usize,
    extra: Vec<Var>,
    temps8: Vec<VarId>,
    temps16: Vec<VarId>,
    used8: usize,
    used16: usize,
    helpers: BTreeSet<&'static str>,
    far: BTreeSet<InstId>,
    errors: Vec<(crate::source::Span, String)>,
    /// X holds a value that must survive the current operand setup.
    x_busy: bool,
    banking: Banking,
    data_bank: Option<u32>,
    saved_bank: Option<VarId>,
    span: crate::source::Span,
}

pub fn generate(mir: &Mir, id: InstId, banking: Banking) -> Output {
    let inst = &mir.insts[id.0 as usize];
    let mut g = Gen {
        mir,
        inst,
        id,
        out: Vec::new(),
        labels: 0,
        extra: Vec::new(),
        temps8: Vec::new(),
        temps16: Vec::new(),
        used8: 0,
        used16: 0,
        helpers: BTreeSet::new(),
        far: BTreeSet::new(),
        errors: Vec::new(),
        x_busy: false,
        banking,
        data_bank: None,
        saved_bank: None,
        span: inst.span,
    };
    g.function();
    Output {
        lines: g.out,
        extra_vars: g.extra,
        helpers: g.helpers,
        far_calls: g.far,
        errors: g.errors,
    }
}

fn byte_of(o: &Opnd, b: usize) -> Opnd {
    o.plus(b as i64)
}

impl Gen<'_> {
    // ------------------------------------------------------------ emitting

    fn ins(&mut self, op: Op, arg: Arg) {
        self.out.push(Line::Ins(Ins::new(op, arg)));
    }
    fn imp(&mut self, op: Op) {
        self.out.push(Line::Ins(Ins::imp(op)));
    }
    fn imm(&mut self, op: Op, n: i64) {
        self.ins(op, Arg::Imm(Opnd::num(n & 0xff)));
    }
    fn label(&mut self, l: &str) {
        self.out.push(Line::Label(l.to_owned()));
    }
    fn new_label(&mut self) -> String {
        self.labels += 1;
        format!("{}_l{}", fn_label(self.id), self.labels)
    }
    fn block_label(&self, b: BlockId) -> String {
        format!("{}_b{}", fn_label(self.id), b.0)
    }
    fn jump(&mut self, op: Op, label: &str) {
        self.ins(op, Arg::Mem(Opnd::label(label)));
    }
    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push((self.span, msg.into()));
    }

    // ----------------------------------------------------------- locations

    fn var(&self, v: VarId) -> Opnd {
        Opnd::sym(Sym::Var(self.id, v))
    }
    fn var_of(&self, inst: InstId, v: VarId) -> Opnd {
        Opnd::sym(Sym::Var(inst, v))
    }
    fn new_var(&mut self, sc: Sc, pointer: bool) -> VarId {
        let id = VarId((self.inst.vars.len() + self.extra.len()) as u32);
        self.extra.push(Var {
            name: format!("m{}", id.0),
            sc,
            pointer,
            temp: true,
        });
        id
    }
    fn tmp8(&mut self) -> Opnd {
        if self.used8 == self.temps8.len() {
            let v = self.new_var(Sc::U8, false);
            self.temps8.push(v);
        }
        let v = self.temps8[self.used8];
        self.used8 += 1;
        self.var(v)
    }
    fn tmp16(&mut self) -> Opnd {
        if self.used16 == self.temps16.len() {
            let v = self.new_var(Sc::U16, true);
            self.temps16.push(v);
        }
        let v = self.temps16[self.used16];
        self.used16 += 1;
        self.var(v)
    }
    fn release(&mut self) {
        self.used8 = 0;
        self.used16 = 0;
    }

    fn obj(&self, o: ObjId) -> &Obj {
        &self.mir.objs[o.0 as usize]
    }
    /// Base operand of byte `b` of an object's elements.
    fn obj_byte(&self, o: ObjId, b: usize) -> Opnd {
        let obj = self.obj(o);
        match obj.kind {
            ObjKind::Io(addr) => Opnd::num(addr as i64 + b as i64),
            _ => {
                if obj.len.is_some() {
                    Opnd::sym(Sym::Obj(o, b == 1))
                } else {
                    Opnd::sym(Sym::Obj(o, false)).plus(b as i64)
                }
            }
        }
    }

    // ------------------------------------------------------------- banking

    fn bank_reads(&self, e: &Ex, out: &mut BTreeSet<u32>) {
        e.walk(&mut |x| {
            if let Ex::Load(o, _, _) = x {
                if let Some(b) = self.obj(*o).bank {
                    out.insert(b);
                }
            }
        });
    }

    /// Select the data window for banked reads in a statement.
    fn prepare_banks(&mut self, exprs: &[&Ex]) {
        let mut banks = BTreeSet::new();
        for e in exprs {
            self.bank_reads(e, &mut banks);
        }
        if banks.is_empty() {
            return;
        }
        if banks.len() > 1 {
            self.error("One statement reads constants from two different banks; split it");
        }
        let bank = *banks.iter().next().unwrap();
        if self.inst.ctx != Context::Main {
            self.error("Interrupt handlers cannot read banked constants");
            return;
        }
        match self.banking {
            Banking::None => {
                self.error("This board has no switchable PRG banks");
                return;
            }
            Banking::Shared if self.inst.bank.is_some() => {
                if self.inst.bank != Some(bank) {
                    self.error(format!(
                        "Code in bank {} cannot read constants in bank {bank}: they share one window on this board",
                        self.inst.bank.unwrap()
                    ));
                }
                return;
            }
            _ => {}
        }
        if self.data_bank == Some(bank) {
            return;
        }
        if self.saved_bank.is_none() {
            // Saved in the prologue (patched below).
            let v = self.new_var(Sc::U8, false);
            self.saved_bank = Some(v);
        }
        self.imm(Op::Lda, bank as i64);
        self.jump(Op::Jsr, "__bank_data");
        self.data_bank = Some(bank);
    }

    // ------------------------------------------------------------ operands

    /// A directly addressable byte operand for `e`'s byte `b`, plus the
    /// index register setup it needs. Never clobbers A.
    fn operand(&mut self, e: &Ex, b: usize) -> Option<Arg> {
        match e {
            Ex::Const(n, _) => Some(Arg::Imm(Opnd::num((n >> (8 * b)) & 0xff))),
            Ex::Addr(o, hi) => {
                let base = Opnd::sym(Sym::Obj(*o, *hi));
                Some(Arg::Imm(if b == 0 { base.lo() } else { base.hi() }))
            }
            Ex::Var(v, sc) => {
                if b >= sc.size() {
                    return if sc.signed() { None } else { Some(Arg::Imm(Opnd::num(0))) };
                }
                Some(Arg::Mem(byte_of(&self.var(*v), b)))
            }
            Ex::Load(o, idx, sc) => {
                if b >= sc.size() {
                    return if sc.signed() { None } else { Some(Arg::Imm(Opnd::num(0))) };
                }
                let base = self.obj_byte(*o, b);
                let big = self.obj(*o).len.unwrap_or(0) > 256;
                match idx.as_deref() {
                    None => Some(Arg::Mem(base)),
                    Some(Ex::Const(k, _)) => Some(Arg::Mem(base.plus(*k))),
                    Some(i) if big => {
                        let _ = i;
                        None
                    }
                    Some(i) => {
                        let reg = self.index_operand(i)?;
                        Some(match reg {
                            'x' => Arg::MemX(base),
                            _ => Arg::MemY(base),
                        })
                    }
                }
            }
            Ex::Cast(x, from, to) => {
                if from.size() == to.size() {
                    return self.operand(x, b);
                }
                if from.size() > to.size() {
                    // Truncation: the low byte.
                    return if b == 0 { self.operand(x, 0) } else { None };
                }
                // Widening.
                if b == 0 {
                    self.operand(x, 0)
                } else if !from.signed() {
                    Some(Arg::Imm(Opnd::num(0)))
                } else if let Ex::Const(n, _) = **x {
                    Some(Arg::Imm(Opnd::num(if n < 0 { 0xff } else { 0 })))
                } else {
                    None
                }
            }
            // `hi(x)`: the high byte of a word.
            _ => None,
        }
        .or_else(|| self.shifted_byte(e, b))
    }

    /// `u8(x >> 8)` and `x >> 8` on words read a byte directly.
    fn shifted_byte(&mut self, e: &Ex, b: usize) -> Option<Arg> {
        match e {
            Ex::Cast(x, from, Sc::U8 | Sc::I8) if from.wide() => match &**x {
                Ex::Bin(BinOp::Shr, w, k, _) if k.konst() == Some(8) && b == 0 => self.operand(w, 1),
                _ => None,
            },
            Ex::Bin(BinOp::Shr, w, k, sc) if k.konst() == Some(8) && !sc.signed() => {
                if b == 0 {
                    self.operand(w, 1)
                } else {
                    Some(Arg::Imm(Opnd::num(0)))
                }
            }
            _ => None,
        }
    }

    /// Load a simple index into X (or Y when X is busy). Returns the register.
    fn index_operand(&mut self, i: &Ex) -> Option<char> {
        let reg = if self.x_busy { 'y' } else { 'x' };
        let (lop, _) = if reg == 'x' { (Op::Ldx, ()) } else { (Op::Ldy, ()) };
        match i {
            Ex::Const(k, _) => {
                self.imm(lop, *k);
                Some(reg)
            }
            Ex::Var(v, sc) if !sc.wide() => {
                let o = self.var(*v);
                self.ins(lop, Arg::Mem(o));
                Some(reg)
            }
            Ex::Cast(x, from, _) if from.wide() => {
                // A proven small index read from a word's low byte.
                if let Ex::Var(v, _) = &**x {
                    let o = self.var(*v);
                    self.ins(lop, Arg::Mem(o));
                    return Some(reg);
                }
                None
            }
            Ex::Load(o, None, sc) if !sc.wide() => {
                let base = self.obj_byte(*o, 0);
                self.ins(lop, Arg::Mem(base));
                Some(reg)
            }
            Ex::Load(o, Some(k), sc) if !sc.wide() && k.konst().is_some() => {
                let base = self.obj_byte(*o, 0).plus(k.konst().unwrap());
                self.ins(lop, Arg::Mem(base));
                Some(reg)
            }
            _ => None,
        }
    }

    // ---------------------------------------------------------- 8-bit values

    /// Evaluate a byte expression into A.
    fn load_a(&mut self, e: &Ex) {
        if let Some(arg) = self.operand(e, 0) {
            self.ins(Op::Lda, arg);
            return;
        }
        match e {
            Ex::Load(o, Some(i), _) => {
                let big = self.obj(*o).len.unwrap_or(0) > 256;
                if big {
                    let p = self.big_address(*o, 0, i);
                    self.imm(Op::Ldy, 0);
                    self.ins(Op::Lda, Arg::IndY(p));
                } else {
                    // Complex index: compute it, then index through X.
                    self.load_a(i);
                    self.imp(Op::Tax);
                    let base = self.obj_byte(*o, 0);
                    self.ins(Op::Lda, Arg::MemX(base));
                }
            }
            Ex::Deref(p, i, _) => {
                let ptr = match &**p {
                    Ex::Var(v, _) if self.var_is_pointer(*v) => self.var(*v),
                    p => {
                        let t = self.tmp16();
                        self.eval16(p, &t);
                        t
                    }
                };
                if let Some(arg) = self.operand(i, 0) {
                    self.ins(Op::Ldy, arg);
                } else {
                    self.load_a(i);
                    self.imp(Op::Tay);
                }
                self.ins(Op::Lda, Arg::IndY(ptr));
            }
            Ex::Peek(a) => {
                if let Some(n) = a.konst() {
                    self.ins(Op::Lda, Arg::Mem(Opnd::num(n)));
                } else {
                    let t = self.tmp16();
                    self.eval16(a, &t);
                    self.imm(Op::Ldy, 0);
                    self.ins(Op::Lda, Arg::IndY(t));
                }
            }
            Ex::Un(op, a, _) => {
                self.load_a(a);
                match op {
                    UnOp::BitNot => self.imm(Op::Eor, 0xff),
                    UnOp::Neg => {
                        self.imm(Op::Eor, 0xff);
                        self.imp(Op::Clc);
                        self.imm(Op::Adc, 1);
                    }
                    UnOp::Not => {
                        // Lowered as `== 0`; kept for completeness.
                        self.imm(Op::Eor, 1);
                    }
                }
            }
            Ex::Bin(op, a, b, sc) if op.is_compare() || op.is_logic() => {
                let _ = sc;
                let t = self.new_label();
                let f = self.new_label();
                let done = self.new_label();
                self.branch(e, &t, &f, Some(&f));
                self.label(&f);
                self.imm(Op::Lda, 0);
                self.jump(Op::Jmp, &done);
                self.label(&t);
                self.imm(Op::Lda, 1);
                self.label(&done);
                let _ = (a, b);
            }
            Ex::Bin(op, a, b, sc) if !sc.wide() => self.bin8(*op, a, b, *sc),
            Ex::Cast(x, from, _) if from.wide() => {
                // Low byte of a computed word.
                let t = self.tmp16();
                self.eval16(x, &t);
                self.ins(Op::Lda, Arg::Mem(t));
            }
            Ex::Cast(x, _, _) => self.load_a(x),
            Ex::Bin(..) => {
                // A word operation truncated implicitly (should not occur).
                let t = self.tmp16();
                self.eval16(e, &t);
                self.ins(Op::Lda, Arg::Mem(t));
            }
            _ => self.error(format!("Cannot evaluate {e:?}")),
        }
    }

    fn var_is_pointer(&self, v: VarId) -> bool {
        let i = v.0 as usize;
        if i < self.inst.vars.len() {
            self.inst.vars[i].pointer
        } else {
            self.extra[i - self.inst.vars.len()].pointer
        }
    }

    /// Zero-page pointer to element `i` of a large array's byte table.
    fn big_address(&mut self, o: ObjId, b: usize, i: &Ex) -> Opnd {
        let p = self.tmp16();
        let base = self.obj_byte(o, b);
        let i16 = if i.sc().wide() { i.clone() } else { Ex::Cast(Box::new(i.clone()), i.sc(), Sc::U16) };
        // p = base + i
        self.eval16(&i16, &p);
        self.ins(Op::Lda, Arg::Mem(p.clone()));
        self.imp(Op::Clc);
        self.ins(Op::Adc, Arg::Imm(base.clone().lo()));
        self.ins(Op::Sta, Arg::Mem(p.clone()));
        self.ins(Op::Lda, Arg::Mem(p.plus(1)));
        self.ins(Op::Adc, Arg::Imm(base.hi()));
        self.ins(Op::Sta, Arg::Mem(p.plus(1)));
        p
    }

    /// Byte operand for the right-hand side of an operation, spilling to a
    /// temporary (computed before A is loaded) when complex.
    fn rhs8(&mut self, b: &Ex) -> Arg {
        // Called after A holds the left operand: setups must not touch A.
        match self.operand(b, 0) {
            Some(arg) => arg,
            None => {
                // Should have been prepared by `prep_rhs`.
                self.error("internal: complex operand");
                Arg::Imm(Opnd::num(0))
            }
        }
    }

    /// Is the byte operand available without computing into A?
    fn is_operand(&self, e: &Ex) -> bool {
        match e {
            Ex::Const(..) | Ex::Var(..) | Ex::Addr(..) => true,
            Ex::Load(o, idx, _) => {
                let big = self.obj(*o).len.unwrap_or(0) > 256;
                match idx.as_deref() {
                    None => true,
                    Some(Ex::Const(..)) => true,
                    Some(i) if !big => matches!(i, Ex::Var(_, s) if !s.wide())
                        || matches!(i, Ex::Load(_, None, s) if !s.wide())
                        || matches!(i, Ex::Load(_, Some(k), s) if !s.wide() && k.konst().is_some())
                        || matches!(i, Ex::Cast(x, f, _) if f.wide() && matches!(**x, Ex::Var(..))),
                    _ => false,
                }
            }
            Ex::Cast(x, from, to) => {
                if from.size() >= to.size() || !from.signed() {
                    self.is_operand(x) && (from.size() <= 1 || matches!(**x, Ex::Var(..) | Ex::Const(..) | Ex::Load(..)))
                } else {
                    matches!(**x, Ex::Const(..))
                }
            }
            Ex::Bin(BinOp::Shr, w, k, sc) if k.konst() == Some(8) && !sc.signed() => self.is_operand(w),
            _ => false,
        }
    }

    /// Spill a complex right operand to a temporary.
    fn spill8(&mut self, b: &Ex) -> Ex {
        if self.is_operand(b) {
            return b.clone();
        }
        self.load_a(b);
        let t = self.tmp8();
        self.ins(Op::Sta, Arg::Mem(t.clone()));
        match t.sym {
            Sym::Var(_, v) => Ex::Var(v, b.sc()),
            _ => unreachable!(),
        }
    }

    fn bin8(&mut self, op: BinOp, a: &Ex, b: &Ex, sc: Sc) {
        let commutative = matches!(op, BinOp::Add | BinOp::And | BinOp::Or | BinOp::Xor);
        // Prefer the complex operand on the left.
        let (a, b) = if commutative && self.is_operand(a) && !self.is_operand(b) { (b, a) } else { (a, b) };
        match op {
            BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor => {
                let b = self.spill8(b);
                self.load_a(a);
                let arg = self.rhs8(&b);
                match op {
                    BinOp::Add => {
                        self.imp(Op::Clc);
                        self.ins(Op::Adc, arg);
                    }
                    BinOp::Sub => {
                        self.imp(Op::Sec);
                        self.ins(Op::Sbc, arg);
                    }
                    BinOp::And => self.ins(Op::And, arg),
                    BinOp::Or => self.ins(Op::Ora, arg),
                    _ => self.ins(Op::Eor, arg),
                }
            }
            BinOp::Shl | BinOp::Shr => {
                if let Some(k) = b.konst() {
                    self.load_a(a);
                    self.shift8_const(op, k, sc);
                } else {
                    let b = self.spill8(b);
                    self.load_a(a);
                    let arg = self.rhs8(&b);
                    self.ins(Op::Ldx, arg);
                    let done = self.new_label();
                    let lp = self.new_label();
                    self.jump(Op::Beq, &done);
                    self.label(&lp);
                    self.shift8_const(op, 1, sc);
                    self.imp(Op::Dex);
                    self.jump(Op::Bne, &lp);
                    self.label(&done);
                }
            }
            BinOp::Mul => {
                if let Some(k) = b.konst() {
                    self.load_a(a);
                    self.mul8_const(k & 0xff);
                } else {
                    let b = self.spill8(b);
                    self.load_a(a);
                    let arg = self.rhs8(&b);
                    self.helper_call8("__mul8", arg);
                }
            }
            BinOp::Div | BinOp::Rem => {
                if let Some(k) = b.konst().filter(|k| *k > 0 && (k & (k - 1)) == 0 && !sc.signed()) {
                    self.load_a(a);
                    if op == BinOp::Div {
                        self.shift8_const(BinOp::Shr, k.trailing_zeros() as i64, sc);
                    } else {
                        self.imm(Op::And, k - 1);
                    }
                } else {
                    let b = self.spill8(b);
                    self.load_a(a);
                    let arg = self.rhs8(&b);
                    let name = match (op, sc.signed()) {
                        (BinOp::Div, false) => "__div8",
                        (BinOp::Rem, false) => "__rem8",
                        (BinOp::Div, true) => "__divs8",
                        _ => "__rems8",
                    };
                    self.helper_call8(name, arg);
                }
            }
            _ => self.error("internal: bad 8-bit operation"),
        }
    }

    /// Helper taking A and a second operand in the helper's scratch byte.
    fn helper_call8(&mut self, name: &'static str, arg: Arg) {
        self.helpers.insert(name);
        let scratch = Opnd::label(format!("{}_b", helper_label(name, self.inst.ctx)));
        self.ins(Op::Ldx, arg);
        self.ins(Op::Stx, Arg::Mem(scratch));
        self.jump(Op::Jsr, &helper_label(name, self.inst.ctx));
    }

    fn shift8_const(&mut self, op: BinOp, k: i64, sc: Sc) {
        if k >= 8 {
            if op == BinOp::Shr && sc.signed() {
                // Sign fill.
                let pos = self.new_label();
                self.imm(Op::Cmp, 0x80);
                self.imm(Op::Lda, 0);
                self.jump(Op::Bcc, &pos);
                self.imm(Op::Lda, 0xff);
                self.label(&pos);
            } else {
                self.imm(Op::Lda, 0);
            }
            return;
        }
        for _ in 0..k {
            match (op, sc.signed()) {
                (BinOp::Shl, _) => self.ins(Op::Asl, Arg::Acc),
                (_, false) => self.ins(Op::Lsr, Arg::Acc),
                (_, true) => {
                    self.imm(Op::Cmp, 0x80);
                    self.ins(Op::Ror, Arg::Acc);
                }
            }
        }
    }

    fn mul8_const(&mut self, k: i64) {
        match k {
            0 => self.imm(Op::Lda, 0),
            1 => {}
            k if k & (k - 1) == 0 => self.shift8_const(BinOp::Shl, k.trailing_zeros() as i64, Sc::U8),
            k if k.count_ones() <= 3 => {
                // Shift-and-add through a temporary.
                let t = self.tmp8();
                self.ins(Op::Sta, Arg::Mem(t.clone()));
                let mut first = true;
                let top = 63 - (k as u64).leading_zeros() as i64;
                // Horner: walk bits from the top.
                let _ = first;
                for bit in (0..top).rev() {
                    self.ins(Op::Asl, Arg::Acc);
                    if (k >> bit) & 1 == 1 {
                        self.imp(Op::Clc);
                        self.ins(Op::Adc, Arg::Mem(t.clone()));
                    }
                }
                first = false;
                let _ = first;
            }
            k => {
                self.helpers.insert("__mul8");
                let scratch = Opnd::label(format!("{}_b", helper_label("__mul8", self.inst.ctx)));
                self.imm(Op::Ldx, k);
                self.ins(Op::Stx, Arg::Mem(scratch));
                self.jump(Op::Jsr, &helper_label("__mul8", self.inst.ctx));
            }
        }
    }

    // --------------------------------------------------------- 16-bit values

    fn is_operand16(&self, e: &Ex) -> bool {
        match e {
            Ex::Const(..) | Ex::Addr(..) => true,
            Ex::Var(_, sc) => sc.wide() || !sc.signed(),
            Ex::Load(o, idx, sc) => {
                (sc.wide() || !sc.signed())
                    && self.obj(*o).len.unwrap_or(0) <= 256
                    && match idx.as_deref() {
                        None | Some(Ex::Const(..)) => true,
                        Some(i) => matches!(i, Ex::Var(_, s) if !s.wide()),
                    }
            }
            Ex::Cast(x, from, to) if to.wide() => {
                if from.wide() {
                    self.is_operand16(x)
                } else {
                    !from.signed() && self.is_operand(x)
                }
            }
            _ => false,
        }
    }

    /// Spill a complex word operand into a temporary.
    fn spill16(&mut self, e: &Ex) -> Ex {
        if self.is_operand16(e) {
            return e.clone();
        }
        let t = self.tmp16();
        self.eval16(e, &t);
        match t.sym {
            Sym::Var(_, v) => Ex::Var(v, e.sc()),
            _ => unreachable!(),
        }
    }

    fn store_byte(&mut self, dst: &Opnd, b: usize) {
        self.ins(Op::Sta, Arg::Mem(byte_of(dst, b)));
    }

    /// Evaluate a word expression into memory at `dst` (two bytes).
    fn eval16(&mut self, e: &Ex, dst: &Opnd) {
        if self.is_operand16(e) {
            for b in 0..2 {
                let arg = self.operand(e, b).unwrap();
                self.ins(Op::Lda, arg);
                self.store_byte(dst, b);
            }
            return;
        }
        match e {
            Ex::Cast(x, from, _) if !from.wide() => {
                // Widen a byte.
                self.load_a(x);
                self.store_byte(dst, 0);
                if from.signed() {
                    let pos = self.new_label();
                    self.imm(Op::Ldx, 0);
                    self.imm(Op::Cmp, 0x80);
                    self.jump(Op::Bcc, &pos);
                    self.imp(Op::Dex);
                    self.label(&pos);
                    self.ins(Op::Stx, Arg::Mem(byte_of(dst, 1)));
                } else {
                    self.imm(Op::Lda, 0);
                    self.store_byte(dst, 1);
                }
            }
            Ex::Cast(x, _, _) => self.eval16(x, dst),
            Ex::Load(o, Some(i), _) => {
                let big = self.obj(*o).len.unwrap_or(0) > 256;
                if big {
                    let lo = self.big_address(*o, 0, i);
                    self.imm(Op::Ldy, 0);
                    self.ins(Op::Lda, Arg::IndY(lo.clone()));
                    let t = self.tmp8();
                    self.ins(Op::Sta, Arg::Mem(t.clone()));
                    // High table: same offset from the high base.
                    let hi_base = self.obj_byte(*o, 1);
                    let lo_base = self.obj_byte(*o, 0);
                    let _ = lo_base;
                    let p2 = self.tmp16();
                    let i16 = if i.sc().wide() { (**i).clone() } else { Ex::Cast(i.clone(), i.sc(), Sc::U16) };
                    self.eval16(&i16, &p2);
                    self.ins(Op::Lda, Arg::Mem(p2.clone()));
                    self.imp(Op::Clc);
                    self.ins(Op::Adc, Arg::Imm(hi_base.clone().lo()));
                    self.ins(Op::Sta, Arg::Mem(p2.clone()));
                    self.ins(Op::Lda, Arg::Mem(p2.plus(1)));
                    self.ins(Op::Adc, Arg::Imm(hi_base.hi()));
                    self.ins(Op::Sta, Arg::Mem(p2.plus(1)));
                    self.ins(Op::Lda, Arg::IndY(p2));
                    self.store_byte(dst, 1);
                    self.ins(Op::Lda, Arg::Mem(t));
                    self.store_byte(dst, 0);
                } else {
                    self.load_a(i);
                    self.imp(Op::Tax);
                    for b in 0..2 {
                        let base = self.obj_byte(*o, b);
                        self.ins(Op::Lda, Arg::MemX(base));
                        self.store_byte(dst, b);
                    }
                }
            }
            Ex::Bin(op, a, b, sc) if !op.is_compare() && !op.is_logic() => self.bin16(*op, a, b, *sc, dst),
            Ex::Un(op, a, _) => {
                let a = self.spill16(a);
                match op {
                    UnOp::BitNot => {
                        for b in 0..2 {
                            let arg = self.operand(&a, b).unwrap();
                            self.ins(Op::Lda, arg);
                            self.imm(Op::Eor, 0xff);
                            self.store_byte(dst, b);
                        }
                    }
                    _ => {
                        self.imp(Op::Sec);
                        for b in 0..2 {
                            self.imm(Op::Lda, 0);
                            let arg = self.operand(&a, b).unwrap();
                            self.ins(Op::Sbc, arg);
                            self.store_byte(dst, b);
                        }
                    }
                }
            }
            e if e.sc() == Sc::U8 || e.sc() == Sc::I8 => {
                // A byte-valued expression used where a word is stored.
                self.load_a(e);
                self.store_byte(dst, 0);
                self.imm(Op::Lda, 0);
                self.store_byte(dst, 1);
            }
            _ => self.error(format!("Cannot evaluate word {e:?}")),
        }
    }

    fn bin16(&mut self, op: BinOp, a: &Ex, b: &Ex, sc: Sc, dst: &Opnd) {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor => {
                let commutative = op != BinOp::Sub;
                let (a, b) = if commutative && self.is_operand16(a) && !self.is_operand16(b) { (b, a) } else { (a, b) };
                let b = self.spill16(b);
                let a = self.spill16(a);
                match op {
                    BinOp::Add => self.imp(Op::Clc),
                    BinOp::Sub => self.imp(Op::Sec),
                    _ => {}
                }
                for i in 0..2 {
                    let x = self.operand(&a, i).unwrap();
                    self.ins(Op::Lda, x);
                    let y = self.operand(&b, i).unwrap();
                    let o = match op {
                        BinOp::Add => Op::Adc,
                        BinOp::Sub => Op::Sbc,
                        BinOp::And => Op::And,
                        BinOp::Or => Op::Ora,
                        _ => Op::Eor,
                    };
                    self.ins(o, y);
                    self.store_byte(dst, i);
                }
            }
            BinOp::Shl | BinOp::Shr => {
                if let Some(k) = b.konst() {
                    self.shift16_const(op, a, k, sc, dst);
                } else {
                    let b = self.spill8(b);
                    self.eval16(a, dst);
                    let arg = self.operand(&b, 0).unwrap();
                    self.ins(Op::Ldx, arg);
                    let done = self.new_label();
                    let lp = self.new_label();
                    self.jump(Op::Beq, &done);
                    self.label(&lp);
                    self.shift16_once(op, sc, dst);
                    self.imp(Op::Dex);
                    self.jump(Op::Bne, &lp);
                    self.label(&done);
                }
            }
            BinOp::Mul | BinOp::Div | BinOp::Rem => {
                if op == BinOp::Mul {
                    if let Some(k) = b.konst() {
                        if k & (k - 1) == 0 && k > 0 {
                            return self.shift16_const(BinOp::Shl, a, k.trailing_zeros() as i64, sc, dst);
                        }
                    }
                }
                if op != BinOp::Mul && !sc.signed() {
                    if let Some(k) = b.konst().filter(|k| *k > 0 && k & (k - 1) == 0) {
                        if op == BinOp::Div {
                            return self.shift16_const(BinOp::Shr, a, k.trailing_zeros() as i64, sc, dst);
                        }
                        let m = Ex::Const(k - 1, sc);
                        return self.bin16(BinOp::And, a, &m, sc, dst);
                    }
                }
                let name: &'static str = match (op, sc.signed()) {
                    (BinOp::Mul, _) => "__mul16",
                    (BinOp::Div, false) => "__div16",
                    (BinOp::Rem, false) => "__rem16",
                    (BinOp::Div, true) => "__divs16",
                    _ => "__rems16",
                };
                self.helpers.insert(name);
                let h = helper_label(name, self.inst.ctx);
                let x = Opnd::label(format!("{h}_a"));
                let y = Opnd::label(format!("{h}_b"));
                self.eval16(a, &x);
                self.eval16(b, &y);
                self.jump(Op::Jsr, &h);
                // Result in A (low) and X (high).
                self.store_byte(dst, 0);
                self.ins(Op::Stx, Arg::Mem(byte_of(dst, 1)));
            }
            _ => self.error("internal: bad 16-bit operation"),
        }
    }

    fn shift16_once(&mut self, op: BinOp, sc: Sc, dst: &Opnd) {
        match (op, sc.signed()) {
            (BinOp::Shl, _) => {
                self.ins(Op::Asl, Arg::Mem(dst.clone()));
                self.ins(Op::Rol, Arg::Mem(dst.plus(1)));
            }
            (_, false) => {
                self.ins(Op::Lsr, Arg::Mem(dst.plus(1)));
                self.ins(Op::Ror, Arg::Mem(dst.clone()));
            }
            (_, true) => {
                self.ins(Op::Lda, Arg::Mem(dst.plus(1)));
                self.imm(Op::Cmp, 0x80);
                self.ins(Op::Ror, Arg::Mem(dst.plus(1)));
                self.ins(Op::Ror, Arg::Mem(dst.clone()));
            }
        }
    }

    fn shift16_const(&mut self, op: BinOp, a: &Ex, k: i64, sc: Sc, dst: &Opnd) {
        let k = k.min(16);
        if k >= 8 && !(op == BinOp::Shr && sc.signed()) {
            // Move a byte, then shift the remainder in A.
            let a = self.spill16(a);
            if op == BinOp::Shl {
                let arg = self.operand(&a, 0).unwrap();
                self.ins(Op::Lda, arg);
                for _ in 8..k {
                    self.ins(Op::Asl, Arg::Acc);
                }
                self.store_byte(dst, 1);
                self.imm(Op::Lda, 0);
                self.store_byte(dst, 0);
            } else {
                let arg = self.operand(&a, 1).unwrap();
                self.ins(Op::Lda, arg);
                for _ in 8..k {
                    self.ins(Op::Lsr, Arg::Acc);
                }
                self.store_byte(dst, 0);
                self.imm(Op::Lda, 0);
                self.store_byte(dst, 1);
            }
            return;
        }
        self.eval16(a, dst);
        for _ in 0..k {
            self.shift16_once(op, sc, dst);
        }
    }

    // ----------------------------------------------------------- branches

    /// Emit code that continues at `t` when `cond` holds and `f` otherwise.
    /// `next` is the label that immediately follows, if known.
    fn branch(&mut self, cond: &Ex, t: &str, f: &str, next: Option<&str>) {
        let finish = |g: &mut Self, op_true: Op| {
            // Branch on op_true to t, else to f.
            if next == Some(t) {
                g.jump(op_true.inverse(), f);
            } else {
                g.jump(op_true, t);
                if next != Some(f) {
                    g.jump(Op::Jmp, f);
                }
            }
        };
        match cond {
            Ex::Const(n, _) => {
                let target = if *n != 0 { t } else { f };
                if next != Some(target) {
                    self.jump(Op::Jmp, target);
                }
            }
            Ex::Bin(op, a, b, sc) if op.is_compare() => {
                self.prepare_banks(&[a, b]);
                self.compare(*op, a, b, *sc, t, f, next);
            }
            e => {
                self.prepare_banks(&[e]);
                if e.sc().wide() {
                    let x = self.spill16(e);
                    self.op_byte(Op::Lda, &x, 0);
                    self.op_byte(Op::Ora, &x, 1);
                } else {
                    self.load_a(e);
                    self.imm(Op::Cmp, 0);
                }
                finish(self, Op::Bne);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compare(&mut self, op: BinOp, a: &Ex, b: &Ex, sc: Sc, t: &str, f: &str, next: Option<&str>) {
        // Normalize constants to the right.
        let (mut op, mut a, mut b) = (op, a.clone(), b.clone());
        if a.konst().is_some() && b.konst().is_none() {
            std::mem::swap(&mut a, &mut b);
            op = swap(op);
        }
        let (lo, hi) = match sc {
            Sc::U8 => (0, 255),
            Sc::I8 => (-128, 127),
            Sc::U16 => (0, 65535),
            Sc::I16 => (-32768, 32767),
        };
        if let Some(k) = b.konst() {
            // `>` and `<=` become `>=` and `<` against k + 1.
            let k = if sc.signed() { k } else { k };
            match op {
                BinOp::Gt if k < hi => {
                    op = BinOp::Ge;
                    b = Ex::Const(k + 1, sc);
                }
                BinOp::Le if k < hi => {
                    op = BinOp::Lt;
                    b = Ex::Const(k + 1, sc);
                }
                BinOp::Gt => return self.branch(&Ex::Const(0, Sc::U8), t, f, next),
                BinOp::Le => return self.branch(&Ex::Const(1, Sc::U8), t, f, next),
                _ => {}
            }
            let k = b.konst().unwrap();
            if op == BinOp::Ge && k == lo {
                return self.branch(&Ex::Const(1, Sc::U8), t, f, next);
            }
            if op == BinOp::Lt && k == lo {
                return self.branch(&Ex::Const(0, Sc::U8), t, f, next);
            }
        } else if matches!(op, BinOp::Gt | BinOp::Le) {
            std::mem::swap(&mut a, &mut b);
            op = swap(op);
        }
        let emit = |g: &mut Self, op_true: Op| {
            if next == Some(t) {
                g.jump(op_true.inverse(), f);
            } else {
                g.jump(op_true, t);
                if next != Some(f) {
                    g.jump(Op::Jmp, f);
                }
            }
        };
        if !sc.wide() {
            let b = self.spill8(&b);
            if sc.signed() && matches!(op, BinOp::Lt | BinOp::Ge) {
                if b.konst() == Some(0) {
                    self.load_a(&a);
                    self.imm(Op::Cmp, 0);
                } else {
                    self.load_a(&a);
                    let arg = self.rhs8(&b);
                    let ok = self.new_label();
                    self.imp(Op::Sec);
                    self.ins(Op::Sbc, arg);
                    self.jump(Op::Bvc, &ok);
                    self.imm(Op::Eor, 0x80);
                    self.label(&ok);
                }
                return emit(self, if op == BinOp::Lt { Op::Bmi } else { Op::Bpl });
            }
            self.load_a(&a);
            let arg = self.rhs8(&b);
            self.ins(Op::Cmp, arg);
            return emit(
                self,
                match op {
                    BinOp::Eq => Op::Beq,
                    BinOp::Ne => Op::Bne,
                    BinOp::Lt => Op::Bcc,
                    _ => Op::Bcs,
                },
            );
        }
        // Words.
        let b = self.spill16(&b);
        let a = self.spill16(&a);
        match op {
            BinOp::Eq | BinOp::Ne => {
                let ne = if op == BinOp::Eq { f } else { t };
                if b.konst() == Some(0) {
                    self.op_byte(Op::Lda, &a, 0);
                    self.op_byte(Op::Ora, &a, 1);
                } else {
                    self.op_byte(Op::Lda, &a, 0);
                    self.op_byte(Op::Cmp, &b, 0);
                    self.jump(Op::Bne, ne);
                    self.op_byte(Op::Lda, &a, 1);
                    self.op_byte(Op::Cmp, &b, 1);
                }
                emit(self, if op == BinOp::Eq { Op::Beq } else { Op::Bne });
            }
            _ => {
                let low_zero = b.konst().is_some_and(|k| k & 0xff == 0) && !sc.signed();
                if low_zero {
                    self.op_byte(Op::Lda, &a, 1);
                    self.op_byte(Op::Cmp, &b, 1);
                } else {
                    self.op_byte(Op::Lda, &a, 0);
                    self.op_byte(Op::Cmp, &b, 0);
                    self.op_byte(Op::Lda, &a, 1);
                    self.op_byte(Op::Sbc, &b, 1);
                }
                if sc.signed() {
                    let ok = self.new_label();
                    self.jump(Op::Bvc, &ok);
                    self.imm(Op::Eor, 0x80);
                    self.label(&ok);
                    emit(self, if op == BinOp::Lt { Op::Bmi } else { Op::Bpl });
                } else {
                    emit(self, if op == BinOp::Lt { Op::Bcc } else { Op::Bcs });
                }
            }
        }
    }

    fn op_byte(&mut self, op: Op, e: &Ex, b: usize) {
        let arg = self.operand(e, b).unwrap_or(Arg::Imm(Opnd::num(0)));
        self.ins(op, arg);
    }

    // --------------------------------------------------------- statements

    fn place_operand(&mut self, p: &Place, b: usize) -> Option<Arg> {
        match p {
            Place::Var(v) => Some(Arg::Mem(byte_of(&self.var(*v), b))),
            Place::Mem(o, idx) => {
                let ex = Ex::Load(*o, idx.clone().map(Box::new), self.obj(*o).sc);
                self.operand(&ex, b)
            }
            _ => None,
        }
    }

    fn place_sc(&self, p: &Place) -> Sc {
        match p {
            Place::Var(v) => {
                let i = v.0 as usize;
                if i < self.inst.vars.len() {
                    self.inst.vars[i].sc
                } else {
                    self.extra[i - self.inst.vars.len()].sc
                }
            }
            Place::Mem(o, _) => self.obj(*o).sc,
            _ => Sc::U8,
        }
    }

    fn assign(&mut self, p: &Place, e: &Ex) {
        let sc = self.place_sc(p);
        let mut reads: Vec<&Ex> = vec![e];
        if let Place::Mem(_, Some(i)) = p {
            reads.push(i);
        }
        self.prepare_banks(&reads);
        if let Place::Mem(o, _) = p {
            if self.obj(*o).kind == ObjKind::Rom {
                self.error("Cannot write to a constant");
                return;
            }
        }
        if sc.wide() {
            return self.assign16(p, e);
        }
        // Read-modify-write forms on simple places.
        if let Ex::Bin(op, a, b, _) = e {
            if self.same_place(p, a) {
                let k = b.konst();
                let rmw = match (op, k) {
                    (BinOp::Add, Some(1)) => Some(Op::Inc),
                    (BinOp::Sub, Some(1)) => Some(Op::Dec),
                    (BinOp::Add, Some(255)) => Some(Op::Dec),
                    (BinOp::Shl, Some(1)) => Some(Op::Asl),
                    (BinOp::Shr, Some(1)) if !sc.signed() => Some(Op::Lsr),
                    _ => None,
                };
                if let Some(op) = rmw {
                    if let Some(arg) = self.place_operand(p, 0) {
                        if !matches!(arg, Arg::MemY(_)) {
                            self.ins(op, arg);
                            return;
                        }
                    }
                }
            }
        }
        match p {
            Place::Var(v) => {
                self.load_a(e);
                let o = self.var(*v);
                self.ins(Op::Sta, Arg::Mem(o));
            }
            Place::Mem(o, idx) => {
                let big = self.obj(*o).len.unwrap_or(0) > 256;
                match idx {
                    Some(i) if big => {
                        self.load_a(e);
                        let t = self.tmp8();
                        self.ins(Op::Sta, Arg::Mem(t.clone()));
                        let ptr = self.big_address(*o, 0, i);
                        self.ins(Op::Lda, Arg::Mem(t));
                        self.imm(Op::Ldy, 0);
                        self.ins(Op::Sta, Arg::IndY(ptr));
                    }
                    Some(i) if !matches!(i, Ex::Const(..)) && !self.is_index_simple(i) => {
                        // Complex index: compute it first into a temporary.
                        self.load_a(i);
                        let t = self.tmp8();
                        self.ins(Op::Sta, Arg::Mem(t.clone()));
                        self.load_a(e);
                        self.ins(Op::Ldx, Arg::Mem(t));
                        let base = self.obj_byte(*o, 0);
                        self.ins(Op::Sta, Arg::MemX(base));
                    }
                    _ => {
                        self.load_a(e);
                        // The value may have used X; index afterwards.
                        let arg = self.place_operand(p, 0).unwrap();
                        self.ins(Op::Sta, arg);
                    }
                }
            }
            Place::Deref(ptr, i) => {
                self.load_a(e);
                let t = self.tmp8();
                self.ins(Op::Sta, Arg::Mem(t.clone()));
                let pv = match ptr {
                    Ex::Var(v, _) if self.var_is_pointer(*v) => self.var(*v),
                    ptr => {
                        let p = self.tmp16();
                        self.eval16(ptr, &p);
                        p
                    }
                };
                self.load_a(i);
                self.imp(Op::Tay);
                self.ins(Op::Lda, Arg::Mem(t));
                self.ins(Op::Sta, Arg::IndY(pv));
            }
            Place::Poke(addr) => {
                if let Some(n) = addr.konst() {
                    self.load_a(e);
                    self.ins(Op::Sta, Arg::Mem(Opnd::num(n)));
                } else {
                    let ptr = self.tmp16();
                    self.eval16(addr, &ptr);
                    self.load_a(e);
                    self.imm(Op::Ldy, 0);
                    self.ins(Op::Sta, Arg::IndY(ptr));
                }
            }
        }
    }

    fn is_index_simple(&self, i: &Ex) -> bool {
        matches!(i, Ex::Var(_, s) if !s.wide())
            || matches!(i, Ex::Load(_, None, s) if !s.wide())
            || matches!(i, Ex::Cast(x, f, _) if f.wide() && matches!(**x, Ex::Var(..)))
    }

    fn same_place(&self, p: &Place, e: &Ex) -> bool {
        match (p, e) {
            (Place::Var(v), Ex::Var(w, _)) => v == w,
            (Place::Mem(o, i), Ex::Load(o2, j, _)) => {
                o == o2
                    && match (i, j) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a == &**b && (a.konst().is_some() || self.is_index_simple(a)),
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    fn assign16(&mut self, p: &Place, e: &Ex) {
        // Increment / decrement / small additions in place.
        if let Ex::Bin(op, a, b, _) = e {
            if self.same_place(p, a) {
                if let (Some(d0), Some(d1)) = (self.place_operand(p, 0), self.place_operand(p, 1)) {
                    let simple = !matches!(d0, Arg::MemY(_));
                    match (op, b.konst()) {
                        (BinOp::Add, Some(1)) if simple => {
                            let skip = self.new_label();
                            self.ins(Op::Inc, d0);
                            self.jump(Op::Bne, &skip);
                            self.ins(Op::Inc, d1);
                            self.label(&skip);
                            return;
                        }
                        (BinOp::Sub, Some(1)) if simple => {
                            let skip = self.new_label();
                            self.ins(Op::Lda, d0.clone());
                            self.jump(Op::Bne, &skip);
                            self.ins(Op::Dec, d1);
                            self.label(&skip);
                            self.ins(Op::Dec, d0);
                            return;
                        }
                        (BinOp::Add, Some(k)) if (2..256).contains(&k) => {
                            let skip = self.new_label();
                            self.ins(Op::Lda, d0.clone());
                            self.imp(Op::Clc);
                            self.imm(Op::Adc, k);
                            self.ins(Op::Sta, d0);
                            self.jump(Op::Bcc, &skip);
                            if simple {
                                self.ins(Op::Inc, d1);
                            } else {
                                self.ins(Op::Lda, d1.clone());
                                self.imm(Op::Adc, 0);
                                self.ins(Op::Sta, d1);
                            }
                            self.label(&skip);
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }
        match p {
            Place::Var(v) => {
                let dst = self.var(*v);
                self.eval16(e, &dst);
            }
            Place::Mem(o, idx) => {
                let big = self.obj(*o).len.unwrap_or(0) > 256;
                let simple_idx = idx.as_ref().is_none_or(|i| i.konst().is_some() || self.is_index_simple(i));
                if !big && simple_idx {
                    // Direct byte stores; operands may be computed.
                    let t = self.tmp16();
                    let direct = self.is_operand16(e);
                    if direct {
                        for b in 0..2 {
                            let arg = self.operand(e, b).unwrap();
                            self.ins(Op::Lda, arg);
                            let d = self.place_operand(p, b).unwrap();
                            self.ins(Op::Sta, d);
                        }
                    } else {
                        self.eval16(e, &t);
                        for b in 0..2 {
                            self.ins(Op::Lda, Arg::Mem(byte_of(&t, b)));
                            let d = self.place_operand(p, b).unwrap();
                            self.ins(Op::Sta, d);
                        }
                    }
                } else {
                    let t = self.tmp16();
                    self.eval16(e, &t);
                    let i = idx.clone().unwrap();
                    if big {
                        for b in 0..2 {
                            let ptr = self.big_address(*o, b, &i);
                            self.ins(Op::Lda, Arg::Mem(byte_of(&t, b)));
                            self.imm(Op::Ldy, 0);
                            self.ins(Op::Sta, Arg::IndY(ptr));
                        }
                    } else {
                        self.load_a(&i);
                        self.imp(Op::Tax);
                        for b in 0..2 {
                            self.ins(Op::Lda, Arg::Mem(byte_of(&t, b)));
                            let base = self.obj_byte(*o, b);
                            self.ins(Op::Sta, Arg::MemX(base));
                        }
                    }
                }
            }
            _ => self.error("Words cannot be written through slices or poke"),
        }
    }

    fn stmt(&mut self, s: &St) {
        self.release();
        match s {
            St::Line(span) => {
                self.span = *span;
                self.out.push(Line::Src(span.file.0, span.start));
            }
            St::Assign(p, e) => self.assign(p, e),
            St::Call { target, args, rets, saved, span } => {
                self.span = *span;
                self.call(*target, args, rets, saved);
            }
            St::Asm { body, operands, inputs, output, span } => {
                self.span = *span;
                self.out.push(Line::Barrier);
                let mut regs: Vec<(char, Ex)> = inputs.clone();
                regs.sort_by_key(|(r, _)| if *r == 'a' { 1 } else { 0 });
                for (r, e) in &regs {
                    match r {
                        'a' => self.load_a(e),
                        reg => {
                            let arg = match self.operand(e, 0) {
                                Some(a) => a,
                                None => {
                                    self.load_a(e);
                                    let t = self.tmp8();
                                    self.ins(Op::Sta, Arg::Mem(t.clone()));
                                    Arg::Mem(t)
                                }
                            };
                            self.ins(if *reg == 'x' { Op::Ldx } else { Op::Ldy }, arg);
                        }
                    }
                }
                let id = self.id;
                let refs = |i: usize| -> Option<Opnd> {
                    operands.get(i).map(|o| match o {
                        AsmOperand::Var(v) => Opnd::sym(Sym::Var(id, *v)),
                        AsmOperand::Obj(o) => Opnd::sym(Sym::Obj(*o, false)),
                        AsmOperand::Fn(f) => Opnd::label(fn_label(*f)),
                        AsmOperand::Const(n) => Opnd::num(*n),
                    })
                };
                let prefix = self.new_label() + "_";
                match crate::asm::parse(body, &prefix, &refs) {
                    Ok(lines) => self.out.extend(lines),
                    Err((line, msg)) => self.error(format!("Assembly line {line}: {msg}")),
                }
                self.out.push(Line::Barrier);
                if let Some(p) = output {
                    // A holds the result.
                    let t = self.tmp8();
                    self.ins(Op::Sta, Arg::Mem(t.clone()));
                    let ex = Ex::Var(match t.sym { Sym::Var(_, v) => v, _ => unreachable!() }, Sc::U8);
                    self.assign(p, &ex);
                }
                self.data_bank = None;
            }
        }
    }

    fn call(&mut self, target: InstId, args: &[Ex], rets: &[Place], saved: &[VarId]) {
        let callee = &self.mir.insts[target.0 as usize];
        let recursive = target == self.id;
        // Save live state across a self-call.
        if recursive && !saved.is_empty() {
            let bytes: Vec<Opnd> = saved
                .iter()
                .flat_map(|v| {
                    let sc = self.inst.vars[v.0 as usize].sc;
                    (0..sc.size()).map(move |b| (v, b))
                })
                .map(|(v, b)| byte_of(&self.var(*v), b))
                .collect();
            self.ins(Op::Ldx, Arg::Mem(Opnd::label("__save_sp")));
            for o in &bytes {
                self.ins(Op::Lda, Arg::Mem(o.clone()));
                self.ins(Op::Sta, Arg::MemX(Opnd::label("__save_stack")));
                self.imp(Op::Inx);
            }
            self.ins(Op::Stx, Arg::Mem(Opnd::label("__save_sp")));
        }
        // Arguments: to temporaries first when they may read the callee's frame.
        let params = callee.params.clone();
        let mut reads_callee = recursive;
        if !recursive {
            for a in args {
                a.walk(&mut |e| {
                    if let Ex::Var(..) = e {
                        // Caller variables never alias a callee's frame.
                    }
                });
            }
        }
        let mut banks = Vec::new();
        for a in args {
            banks.push(a);
        }
        self.prepare_banks(&banks);
        if reads_callee && args.iter().any(|a| !a.simple() || matches!(a, Ex::Var(..))) {
            let mut staged = Vec::new();
            for a in args {
                if matches!(a, Ex::Const(..)) {
                    staged.push(a.clone());
                    continue;
                }
                let sc = a.sc();
                let v = self.new_var(sc, false);
                let dst = self.var(v);
                if sc.wide() {
                    self.eval16(a, &dst);
                } else {
                    self.load_a(a);
                    self.ins(Op::Sta, Arg::Mem(dst));
                }
                staged.push(Ex::Var(v, sc));
            }
            for (p, a) in params.iter().zip(&staged) {
                self.store_param(target, *p, a);
            }
        } else {
            for (p, a) in params.iter().zip(args) {
                self.store_param(target, *p, a);
            }
        }
        reads_callee = false;
        let _ = reads_callee;
        let far = callee.bank.is_some() && callee.bank != self.inst.bank;
        if far {
            self.far.insert(target);
            self.jump(Op::Jsr, &format!("__far{}", target.0));
        } else {
            self.jump(Op::Jsr, &fn_label(target));
        }
        // Results.
        let ret_scs: Vec<Sc> = callee.rets.iter().map(|r| callee.vars[r.0 as usize].sc).collect();
        let callee_rets = callee.rets.clone();
        if ret_scs.len() == 1 && !rets.is_empty() {
            let sc = ret_scs[0];
            let t = self.new_var(sc, false);
            let dst = self.var(t);
            self.ins(Op::Sta, Arg::Mem(dst.clone()));
            if sc.wide() {
                self.ins(Op::Stx, Arg::Mem(dst.plus(1)));
            }
            let ex = Ex::Var(t, sc);
            if recursive && !saved.is_empty() {
                self.restore(saved);
            }
            self.assign(&rets[0], &ex);
        } else {
            let mut values = Vec::new();
            for (r, sc) in callee_rets.iter().zip(&ret_scs) {
                let o = self.var_of(target, *r);
                if recursive {
                    let t = self.new_var(*sc, false);
                    let dst = self.var(t);
                    for b in 0..sc.size() {
                        self.ins(Op::Lda, Arg::Mem(o.plus(b as i64)));
                        self.ins(Op::Sta, Arg::Mem(dst.plus(b as i64)));
                    }
                    values.push(Ex::Var(t, *sc));
                } else {
                    values.push(Ex::Var(*r, *sc));
                }
            }
            if recursive && !saved.is_empty() {
                self.restore(saved);
            }
            for (p, v) in rets.iter().zip(values) {
                if recursive {
                    self.assign(p, &v);
                } else {
                    // Read the callee's result variable directly.
                    let Ex::Var(r, sc) = v else { continue };
                    let src = self.var_of(target, r);
                    if sc.wide() {
                        let t = self.tmp16();
                        for b in 0..2 {
                            self.ins(Op::Lda, Arg::Mem(src.plus(b)));
                            self.ins(Op::Sta, Arg::Mem(t.plus(b)));
                        }
                        let tv = match t.sym { Sym::Var(_, v) => v, _ => unreachable!() };
                        self.assign(p, &Ex::Var(tv, sc));
                    } else {
                        self.ins(Op::Lda, Arg::Mem(src));
                        let t = self.tmp8();
                        self.ins(Op::Sta, Arg::Mem(t.clone()));
                        let tv = match t.sym { Sym::Var(_, v) => v, _ => unreachable!() };
                        self.assign(p, &Ex::Var(tv, sc));
                    }
                }
            }
        }
        if ret_scs.is_empty() && recursive && !saved.is_empty() {
            self.restore(saved);
        }
    }

    fn restore(&mut self, saved: &[VarId]) {
        let bytes: Vec<Opnd> = saved
            .iter()
            .flat_map(|v| {
                let sc = self.inst.vars[v.0 as usize].sc;
                (0..sc.size()).map(move |b| (v, b))
            })
            .map(|(v, b)| byte_of(&self.var(*v), b))
            .collect();
        self.ins(Op::Ldx, Arg::Mem(Opnd::label("__save_sp")));
        for o in bytes.iter().rev() {
            self.imp(Op::Dex);
            self.ins(Op::Lda, Arg::MemX(Opnd::label("__save_stack")));
            self.ins(Op::Sta, Arg::Mem(o.clone()));
        }
        self.ins(Op::Stx, Arg::Mem(Opnd::label("__save_sp")));
    }

    fn store_param(&mut self, target: InstId, p: VarId, a: &Ex) {
        let sc = self.mir.insts[target.0 as usize].vars[p.0 as usize].sc;
        let dst = self.var_of(target, p);
        if sc.wide() {
            self.eval16(a, &dst);
        } else {
            self.load_a(a);
            self.ins(Op::Sta, Arg::Mem(dst));
        }
        self.release();
    }

    // ------------------------------------------------------------ layout

    fn order(&self) -> Vec<BlockId> {
        let n = self.inst.blocks.len();
        let mut reachable = vec![false; n];
        let mut stack = vec![self.inst.entry];
        while let Some(b) = stack.pop() {
            if reachable[b.0 as usize] {
                continue;
            }
            reachable[b.0 as usize] = true;
            stack.extend(self.inst.successors(b));
        }
        let mut placed = vec![false; n];
        let mut order = Vec::new();
        let mut next = Some(self.inst.entry);
        loop {
            let b = match next.take() {
                Some(b) if !placed[b.0 as usize] => b,
                _ => match (0..n).find(|i| reachable[*i] && !placed[*i]) {
                    Some(i) => BlockId(i as u32),
                    None => break,
                },
            };
            placed[b.0 as usize] = true;
            order.push(b);
            // Prefer the false edge (fall-through) or the jump target.
            next = match &self.inst.blocks[b.0 as usize].term {
                Term::Goto(t) => Some(*t),
                Term::Branch(_, t, f) => {
                    if !placed[f.0 as usize] {
                        Some(*f)
                    } else {
                        Some(*t)
                    }
                }
                _ => None,
            };
        }
        order
    }

    fn function(&mut self) {
        let label = fn_label(self.id);
        self.label(&label);
        let interrupt = self.inst.interrupt.is_some();
        if interrupt {
            self.imp(Op::Pha);
            self.imp(Op::Txa);
            self.imp(Op::Pha);
            self.imp(Op::Tya);
            self.imp(Op::Pha);
        }
        // Placeholder for the bank-save prologue.
        let prologue_at = self.out.len();
        let order = self.order();
        for (i, b) in order.iter().enumerate() {
            let next_label = order.get(i + 1).map(|n| self.block_label(*n));
            let l = self.block_label(*b);
            self.label(&l);
            self.data_bank = None;
            let block = &self.inst.blocks[b.0 as usize];
            for s in &block.stmts.clone() {
                self.stmt(s);
            }
            self.release();
            match block.term.clone() {
                Term::Goto(t) => {
                    let tl = self.block_label(t);
                    if self.is_return_block(t) && !interrupt && self.saved_bank.is_none() {
                        self.epilogue();
                    } else if next_label.as_deref() != Some(tl.as_str()) {
                        self.jump(Op::Jmp, &tl);
                    }
                }
                Term::Branch(c, t, f) => {
                    let tl = self.block_label(t);
                    let fl = self.block_label(f);
                    self.branch(&c, &tl, &fl, next_label.as_deref());
                }
                Term::Return | Term::Open => self.epilogue(),
            }
        }
        if let Some(v) = self.saved_bank {
            let o = self.var(v);
            let pre = vec![
                Line::Ins(Ins::new(Op::Lda, Arg::Mem(Opnd::label("__bank_shadow")))),
                Line::Ins(Ins::new(Op::Sta, Arg::Mem(o))),
            ];
            self.out.splice(prologue_at..prologue_at, pre);
        }
    }

    fn is_return_block(&self, b: BlockId) -> bool {
        let block = &self.inst.blocks[b.0 as usize];
        block.stmts.iter().all(|s| matches!(s, St::Line(_))) && matches!(block.term, Term::Return)
    }

    fn epilogue(&mut self) {
        if let Some(v) = self.saved_bank {
            let o = self.var(v);
            self.ins(Op::Lda, Arg::Mem(o));
            self.jump(Op::Jsr, "__bank_data");
        }
        if let Some(_ctx) = self.inst.interrupt {
            self.imp(Op::Pla);
            self.imp(Op::Tay);
            self.imp(Op::Pla);
            self.imp(Op::Tax);
            self.imp(Op::Pla);
            self.imp(Op::Rti);
            return;
        }
        if self.inst.reset {
            let l = self.new_label();
            self.label(&l);
            self.jump(Op::Jmp, &l);
            return;
        }
        match self.inst.rets.as_slice() {
            [r] => {
                let sc = self.inst.vars[r.0 as usize].sc;
                let o = self.var(*r);
                self.ins(Op::Lda, Arg::Mem(o.clone()));
                if sc.wide() {
                    self.ins(Op::Ldx, Arg::Mem(o.plus(1)));
                }
            }
            _ => {}
        }
        self.imp(Op::Rts);
    }
}

fn swap(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Le => BinOp::Ge,
        BinOp::Gt => BinOp::Lt,
        BinOp::Ge => BinOp::Le,
        o => o,
    }
}

#[allow(dead_code)]
fn unused(_: BTreeMap<u8, u8>) {}
