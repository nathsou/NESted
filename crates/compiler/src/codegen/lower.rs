//! Operand-directed lowering.
//!
//! Byte expressions are evaluated into A, using memory, immediate and indexed
//! operands directly instead of spilling every subexpression. 16-bit values
//! are computed byte-wise into a memory destination, which the 6502 handles
//! far better than an A/X register pair. Comparisons branch on flags; signed
//! comparisons use the overflow flag. Anything not covered here falls back to
//! the general lowering in `codegen.rs`.
use super::{Generator, Storage};
use crate::frontend::*;
use std::collections::BTreeMap;

/// A byte operand usable by `lda/adc/sbc/and/ora/eor/cmp`, with an optional
/// index-register load that must precede the instruction (it leaves A intact).
#[derive(Clone, Debug)]
pub(super) struct Operand {
    pub prep: Option<(&'static str, String)>,
    pub text: String,
}

/// A 16-bit operand: low and high byte spellings (immediate or memory).
#[derive(Clone, Debug)]
pub(super) struct Wide {
    pub lo: String,
    pub hi: String,
}

impl Wide {
    fn zero(&self) -> bool {
        self.lo == "#0" && self.hi == "#0"
    }
}

fn swapped(op: &str) -> &'static str {
    match op {
        "<" => ">",
        ">" => "<",
        "<=" => ">=",
        ">=" => "<=",
        "==" => "==",
        _ => "!=",
    }
}

fn cast_name(n: &str) -> bool {
    ["u8", "i8", "u16", "i16", "bool"].contains(&n)
}

impl Generator<'_> {
    fn plain_storage(&self, name: &str) -> Option<Storage> {
        let s = self.storage(name).ok()?;
        if s.bank.is_some() || self.volatile_label(&s.label) {
            return None;
        }
        Some(s)
    }

    /// Byte operand for `e`, indexing (if needed) through `reg` ('x' or 'y').
    pub(super) fn byte_operand(&self, e: &Expr, reg: char) -> Option<Operand> {
        if e.ty.size() != 1 {
            return None;
        }
        if let Some(n) = eval_const(e, &self.constants) {
            return Some(Operand {
                prep: None,
                text: format!("#{}", n & 255),
            });
        }
        match &e.kind {
            ExprKind::Name(n) => {
                let s = self.plain_storage(n)?;
                if let Some(c) = s.constant {
                    return Some(Operand {
                        prep: None,
                        text: format!("#{}", c & 255),
                    });
                }
                Some(Operand {
                    prep: None,
                    text: s.label,
                })
            }
            ExprKind::Call(n, args) if cast_name(n) && n != "bool" && args[0].ty.size() == 1 => {
                self.byte_operand(&args[0], reg)
            }
            ExprKind::Index(base, index, _) => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let s = self.plain_storage(name)?;
                let Ty::Array(t, _) = &s.ty else {
                    return None;
                };
                if t.size() != 1 || index.ty.wide() {
                    return None;
                }
                if let Some(n) = eval_const(index, &self.constants) {
                    return Some(Operand {
                        prep: None,
                        text: format!("{}+{}", s.label, n),
                    });
                }
                if let ExprKind::Name(n) = &index.kind {
                    if self.y_counter.as_deref() == Some(n.as_str()) {
                        return Some(Operand {
                            prep: None,
                            text: format!("{},y", s.label),
                        });
                    }
                }
                if reg == 'y' && self.y_counter.is_some() {
                    return None;
                }
                // `arr[i - c]` reads `arr-c` indexed by i: an index that
                // wrapped would be out of bounds when len + c <= 256.
                let (index, offset) = match &index.kind {
                    ExprKind::Binary(op, a, b) if op == "-" => {
                        match (eval_const(b, &self.constants), &s.ty) {
                            (Some(c), Ty::Array(_, len)) if c > 0 && *len as i64 + c <= 256 => {
                                (a.as_ref(), -c)
                            }
                            _ => (index.as_ref(), 0),
                        }
                    }
                    _ => (index.as_ref(), 0),
                };
                let index = self.byte_operand(index, reg)?;
                if index.prep.is_some() {
                    return None;
                }
                let load = if reg == 'x' { "ldx" } else { "ldy" };
                let base = if offset == 0 {
                    s.label.clone()
                } else {
                    format!("{}{offset}", s.label)
                };
                Some(Operand {
                    prep: Some((load, index.text)),
                    text: format!("{base},{reg}"),
                })
            }
            _ => None,
        }
    }

    fn apply(&mut self, inst: &str, operand: &Operand) {
        if let Some((load, arg)) = &operand.prep {
            self.op(load, arg);
        }
        self.op(inst, &operand.text);
    }

    /// Evaluates a byte (or bool) expression into A.
    pub(super) fn byte_expr(&mut self, e: &Expr) -> Result<(), Diagnostic> {
        let saved = self.temp;
        if let Some(o) = self.byte_operand(e, 'x') {
            self.apply("lda", &o);
            return Ok(());
        }
        match &e.kind {
            ExprKind::Index(base, index, _) if !index.ty.wide() => {
                let ExprKind::Name(name) = &base.kind else {
                    return self.expression_fallback(e);
                };
                match self.plain_storage(name) {
                    Some(Storage {
                        ty: Ty::Array(t, _),
                        label,
                        ..
                    }) if t.size() == 1 => {
                        self.byte_expr(index)?;
                        self.op("tax", "");
                        self.op("lda", format!("{label},x"));
                    }
                    _ => return self.expression_fallback(e),
                }
            }
            ExprKind::Binary(op, l, r) if ["+", "-", "&", "|", "^"].contains(&op.as_str()) => {
                let commutative = op != "-";
                let (l, r) = if self.byte_operand(r, 'x').is_none()
                    && commutative
                    && self.byte_operand(l, 'x').is_some()
                {
                    (r, l)
                } else {
                    (l, r)
                };
                let operand = match self.byte_operand(r, 'x') {
                    Some(o) => {
                        self.byte_expr(l)?;
                        o
                    }
                    None if has_call(l) => {
                        // Keep source order: left side first.
                        let tl = self.temp(1);
                        let tr = self.temp(1);
                        self.byte_expr(l)?;
                        self.op("sta", &tl);
                        self.byte_expr(r)?;
                        self.op("sta", &tr);
                        self.op("lda", &tl);
                        Operand {
                            prep: None,
                            text: tr,
                        }
                    }
                    None => {
                        let t = self.temp(1);
                        self.byte_expr(r)?;
                        self.op("sta", &t);
                        self.byte_expr(l)?;
                        Operand {
                            prep: None,
                            text: t,
                        }
                    }
                };
                let inst = match op.as_str() {
                    "+" => {
                        self.op("clc", "");
                        "adc"
                    }
                    "-" => {
                        self.op("sec", "");
                        "sbc"
                    }
                    "&" => "and",
                    "|" => "ora",
                    _ => "eor",
                };
                self.apply(inst, &operand);
            }
            ExprKind::Binary(op, l, r)
                if (op == "<<" || op == ">>") && eval_const(r, &self.constants).is_some() =>
            {
                let n = eval_const(r, &self.constants).unwrap();
                if !(0..8).contains(&n) {
                    return self.expression_fallback(e);
                }
                self.byte_expr(l)?;
                for _ in 0..n {
                    if op == "<<" {
                        self.op("asl", "a");
                    } else if l.ty.signed() {
                        self.op("cmp", "#128");
                        self.op("ror", "a");
                    } else {
                        self.op("lsr", "a");
                    }
                }
            }
            ExprKind::Unary(op, x) if op == "!" && x.ty == Ty::Bool => {
                self.byte_expr(x)?;
                self.op("eor", "#1");
            }
            ExprKind::Unary(op, x) if op == "~" => {
                self.byte_expr(x)?;
                self.op("eor", "#255");
            }
            ExprKind::Unary(op, x) if op == "-" => {
                self.byte_expr(x)?;
                self.op("eor", "#255");
                self.op("clc", "");
                self.op("adc", "#1");
            }
            ExprKind::Call(n, args) if cast_name(n) && n != "bool" => {
                if args[0].ty.wide() {
                    // Truncation: the low byte.
                    match self.wide_operand(&args[0]) {
                        Some(w) => self.op("lda", &w.lo),
                        None => {
                            let t = self.temp(2);
                            self.wide_to(&args[0], &t)?;
                            self.op("lda", &t);
                        }
                    }
                } else {
                    self.byte_expr(&args[0])?;
                }
            }
            _ => return self.expression_fallback(e),
        }
        self.temp = saved;
        Ok(())
    }

    /// 16-bit operand without code: immediates and plain memory.
    pub(super) fn wide_operand(&self, e: &Expr) -> Option<Wide> {
        if !e.ty.wide() {
            return None;
        }
        if let Some(n) = eval_const(e, &self.constants) {
            return Some(Wide {
                lo: format!("#{}", n & 255),
                hi: format!("#{}", (n >> 8) & 255),
            });
        }
        match &e.kind {
            ExprKind::Name(n) => {
                let s = self.plain_storage(n)?;
                if let Some(c) = s.constant {
                    return Some(Wide {
                        lo: format!("#{}", c & 255),
                        hi: format!("#{}", (c >> 8) & 255),
                    });
                }
                Some(Wide {
                    lo: s.label.clone(),
                    hi: format!("{}+1", s.label),
                })
            }
            ExprKind::Index(base, index, _) => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let s = self.plain_storage(name)?;
                let n = eval_const(index, &self.constants)?;
                Some(Wide {
                    lo: format!("{}+{}", s.label, n * 2),
                    hi: format!("{}+{}", s.label, n * 2 + 1),
                })
            }
            ExprKind::Call(n, args) if cast_name(n) && args[0].ty.wide() => {
                self.wide_operand(&args[0])
            }
            _ => None,
        }
    }

    /// A wide operand for `e`, materializing it into a temporary if needed.
    fn wide_value(&mut self, e: &Expr) -> Result<Wide, Diagnostic> {
        if let Some(w) = self.wide_operand(e) {
            return Ok(w);
        }
        let t = self.temp(2);
        self.wide_to(e, &t)?;
        Ok(Wide {
            lo: t.clone(),
            hi: format!("{t}+1"),
        })
    }

    /// Indexing a 16-bit array by a byte: Y = index * 2.
    fn wide_array<'e>(&self, e: &'e Expr) -> Option<(String, &'e Expr)> {
        let ExprKind::Index(base, index, _) = &e.kind else {
            return None;
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let s = self.plain_storage(name)?;
        match &s.ty {
            Ty::Array(t, n) if t.size() == 2 && *n <= 128 && !index.ty.wide() => {
                Some((s.label, index.as_ref()))
            }
            _ => None,
        }
    }

    /// Evaluates a 16-bit expression into `dst` (two bytes of memory).
    pub(super) fn wide_to(&mut self, e: &Expr, dst: &str) -> Result<(), Diagnostic> {
        let saved = self.temp;
        let hi = format!("{dst}+1");
        if let Some(w) = self.wide_operand(e) {
            self.op("lda", &w.lo);
            self.op("sta", dst);
            self.op("lda", &w.hi);
            self.op("sta", &hi);
            return Ok(());
        }
        if let Some((label, index)) = self.wide_array(e) {
            self.byte_expr(index)?;
            self.op("asl", "a");
            self.op("tay", "");
            self.op("lda", format!("{label},y"));
            self.op("sta", dst);
            self.op("lda", format!("{label}+1,y"));
            self.op("sta", &hi);
            self.temp = saved;
            return Ok(());
        }
        match &e.kind {
            ExprKind::Binary(op, l, r) if ["+", "-", "&", "|", "^"].contains(&op.as_str()) => {
                // Adding a widened byte: add A, then propagate carry/sign.
                if op == "+" || op == "-" {
                    if let Some(byte) = widened_byte(r) {
                        if !byte.ty.signed() {
                            let lw = self.wide_value(l)?;
                            self.byte_expr(byte)?;
                            if op == "+" {
                                self.op("clc", "");
                                self.op("adc", &lw.lo);
                                self.op("sta", dst);
                                self.op("lda", &lw.hi);
                                self.op("adc", "#0");
                                self.op("sta", &hi);
                            } else {
                                let t = self.temp(1);
                                self.op("sta", &t);
                                self.op("lda", &lw.lo);
                                self.op("sec", "");
                                self.op("sbc", &t);
                                self.op("sta", dst);
                                self.op("lda", &lw.hi);
                                self.op("sbc", "#0");
                                self.op("sta", &hi);
                            }
                            self.temp = saved;
                            return Ok(());
                        }
                    }
                }
                // A 16-bit table element as the right operand: index into Y
                // first, then use `tab,y` / `tab+1,y` directly.
                let (lw, rw) = match (self.wide_operand(l), self.wide_array(r)) {
                    (Some(lw), Some((label, index))) => {
                        self.byte_expr(index)?;
                        self.op("asl", "a");
                        self.op("tay", "");
                        (
                            lw,
                            Wide {
                                lo: format!("{label},y"),
                                hi: format!("{label}+1,y"),
                            },
                        )
                    }
                    _ => {
                        let lw = self.wide_value(l)?;
                        let rw = self.wide_value(r)?;
                        (lw, rw)
                    }
                };
                let inst = match op.as_str() {
                    "+" => "adc",
                    "-" => "sbc",
                    "&" => "and",
                    "|" => "ora",
                    _ => "eor",
                };
                if op == "+" {
                    self.op("clc", "");
                }
                if op == "-" {
                    self.op("sec", "");
                }
                self.op("lda", &lw.lo);
                self.op(inst, &rw.lo);
                self.op("sta", dst);
                self.op("lda", &lw.hi);
                self.op(inst, &rw.hi);
                self.op("sta", &hi);
            }
            ExprKind::Unary(op, x) if op == "-" => {
                let w = self.wide_value(x)?;
                self.op("sec", "");
                self.op("lda", "#0");
                self.op("sbc", &w.lo);
                self.op("sta", dst);
                self.op("lda", "#0");
                self.op("sbc", &w.hi);
                self.op("sta", &hi);
            }
            ExprKind::Unary(op, x) if op == "~" => {
                let w = self.wide_value(x)?;
                self.op("lda", &w.lo);
                self.op("eor", "#255");
                self.op("sta", dst);
                self.op("lda", &w.hi);
                self.op("eor", "#255");
                self.op("sta", &hi);
            }
            ExprKind::Binary(op, l, r)
                if (op == "<<" || op == ">>") && eval_const(r, &self.constants).is_some() =>
            {
                let n = eval_const(r, &self.constants).unwrap();
                if !(0..16).contains(&n) {
                    return self.wide_fallback(e, dst);
                }
                self.wide_to(l, dst)?;
                let mut n = n;
                if n >= 8 {
                    n -= 8;
                    if op == "<<" {
                        self.op("lda", dst);
                        self.op("sta", &hi);
                        self.op("lda", "#0");
                        self.op("sta", dst);
                    } else {
                        self.op("lda", &hi);
                        self.op("sta", dst);
                        if l.ty.signed() {
                            self.sign_byte();
                        } else {
                            self.op("lda", "#0");
                        }
                        self.op("sta", &hi);
                    }
                }
                for _ in 0..n {
                    if op == "<<" {
                        self.op("asl", dst);
                        self.op("rol", &hi);
                    } else {
                        if l.ty.signed() {
                            self.op("lda", &hi);
                            self.op("asl", "a");
                            self.op("ror", &hi);
                        } else {
                            self.op("lsr", &hi);
                        }
                        self.op("ror", dst);
                    }
                }
            }
            ExprKind::Call(n, args) if cast_name(n) && !args[0].ty.wide() => {
                self.byte_expr(&args[0])?;
                self.op("sta", dst);
                if args[0].ty.signed() {
                    self.sign_byte();
                } else {
                    self.op("lda", "#0");
                }
                self.op("sta", &hi);
            }
            _ => return self.wide_fallback(e, dst),
        }
        self.temp = saved;
        Ok(())
    }

    /// A = $FF if A is negative, else 0.
    fn sign_byte(&mut self) {
        self.op("asl", "a");
        self.op("lda", "#0");
        self.op("adc", "#255");
        self.op("eor", "#255");
    }

    fn wide_fallback(&mut self, e: &Expr, dst: &str) -> Result<(), Diagnostic> {
        self.expression_fallback(e)?;
        self.op("sta", dst);
        self.op("stx", format!("{dst}+1"));
        Ok(())
    }

    /// Evaluates a 16-bit expression into A (low) and X (high).
    pub(super) fn wide_ax(&mut self, e: &Expr) -> Result<(), Diagnostic> {
        let saved = self.temp;
        if let Some(w) = self.wide_operand(e) {
            self.op("lda", &w.lo);
            self.op("ldx", &w.hi);
            return Ok(());
        }
        if let Some((label, index)) = self.wide_array(e) {
            self.byte_expr(index)?;
            self.op("asl", "a");
            self.op("tay", "");
            self.op("ldx", format!("{label}+1,y"));
            self.op("lda", format!("{label},y"));
            return Ok(());
        }
        match &e.kind {
            ExprKind::Binary(..) | ExprKind::Unary(..) => {}
            ExprKind::Call(n, _) if cast_name(n) => {}
            _ => return self.expression_fallback(e),
        }
        let t = self.temp(2);
        self.wide_to(e, &t)?;
        self.op("lda", &t);
        self.op("ldx", format!("{t}+1"));
        self.temp = saved;
        Ok(())
    }

    /// Branches to `yes` or `no` on a comparison. Returns false if the form
    /// is not handled here.
    pub(super) fn compare(
        &mut self,
        op: &str,
        l: &Expr,
        r: &Expr,
        yes: &str,
        no: &str,
    ) -> Result<bool, Diagnostic> {
        if l.ty.wide() {
            return self.wide_compare(op, l, r, yes, no);
        }
        if l.ty.size() != 1 || r.ty.size() != 1 {
            return Ok(false);
        }
        let saved = self.temp;
        let (mut op, mut l, mut r) = (op, l, r);
        if self.byte_operand(r, 'x').is_none() && self.byte_operand(l, 'x').is_some() && !has_call(r) {
            (op, l, r) = (swapped(op), r, l);
        }
        let signed = l.ty.signed() && op != "==" && op != "!=";
        if signed && (op == ">" || op == "<=") && !has_call(l) && !has_call(r) {
            (op, l, r) = (swapped(op), r, l);
        }
        if signed && (op == ">" || op == "<=") {
            return Ok(false);
        }
        // `x > k` / `x <= k` against constants become `>= k+1` / `< k+1`.
        let (op, rhs_const) = match (op, eval_const(r, &self.constants)) {
            (">", Some(k)) if !signed && (k & 255) < 255 => (">=", Some((k & 255) + 1)),
            ("<=", Some(k)) if !signed && (k & 255) < 255 => ("<", Some((k & 255) + 1)),
            (o, k) => (o, k.map(|k| k & 255)),
        };
        let operand = if let Some(k) = rhs_const {
            self.byte_expr(l)?;
            Operand {
                prep: None,
                text: format!("#{k}"),
            }
        } else if let Some(o) = self.byte_operand(r, 'x') {
            self.byte_expr(l)?;
            o
        } else if has_call(l) {
            let tl = self.temp(1);
            let tr = self.temp(1);
            self.byte_expr(l)?;
            self.op("sta", &tl);
            self.byte_expr(r)?;
            self.op("sta", &tr);
            self.op("lda", &tl);
            Operand {
                prep: None,
                text: tr,
            }
        } else {
            let t = self.temp(1);
            self.byte_expr(r)?;
            self.op("sta", &t);
            self.byte_expr(l)?;
            Operand {
                prep: None,
                text: t,
            }
        };
        if signed {
            // l - r: N xor V is the signed "less than".
            self.op("sec", "");
            self.apply("sbc", &operand);
            let fix = self.label("signed");
            self.op("bvc", &fix);
            self.op("eor", "#128");
            self.at(&fix);
            self.op(if op == "<" { "bmi" } else { "bpl" }, yes);
            self.op("jmp", no);
            self.temp = saved;
            return Ok(true);
        }
        if rhs_const == Some(0) && (op == "==" || op == "!=") {
            // Flags already reflect A after every byte_expr lowering ends
            // with an A-setting instruction; the peephole removes this cmp
            // when that is provable.
            self.op("cmp", "#0");
        } else {
            self.apply("cmp", &operand);
        }
        match op {
            "==" => self.op("beq", yes),
            "!=" => self.op("bne", yes),
            "<" => self.op("bcc", yes),
            ">=" => self.op("bcs", yes),
            "<=" => {
                self.op("bcc", yes);
                self.op("beq", yes);
            }
            _ => {
                self.op("beq", no);
                self.op("bcs", yes);
            }
        }
        self.op("jmp", no);
        self.temp = saved;
        Ok(true)
    }

    fn wide_compare(
        &mut self,
        op: &str,
        l: &Expr,
        r: &Expr,
        yes: &str,
        no: &str,
    ) -> Result<bool, Diagnostic> {
        if !r.ty.wide() {
            return Ok(false);
        }
        let saved = self.temp;
        let signed = l.ty.signed();
        let lw = self.wide_value(l)?;
        let rw = self.wide_value(r)?;
        let (op, lw, rw) = match op {
            ">" => ("<", rw, lw),
            "<=" => (">=", rw, lw),
            o => (o, lw, rw),
        };
        match op {
            "==" | "!=" => {
                if rw.zero() {
                    self.op("lda", &lw.lo);
                    self.op("ora", &lw.hi);
                    self.op(if op == "==" { "beq" } else { "bne" }, yes);
                } else {
                    self.op("lda", &lw.lo);
                    self.op("cmp", &rw.lo);
                    self.op("bne", if op == "==" { no } else { yes });
                    self.op("lda", &lw.hi);
                    self.op("cmp", &rw.hi);
                    self.op(if op == "==" { "beq" } else { "bne" }, yes);
                }
            }
            _ => {
                if signed && rw.zero() {
                    self.op("lda", &lw.hi);
                    self.op(if op == "<" { "bmi" } else { "bpl" }, yes);
                } else {
                    self.op("lda", &lw.lo);
                    self.op("cmp", &rw.lo);
                    self.op("lda", &lw.hi);
                    self.op("sbc", &rw.hi);
                    if signed {
                        let fix = self.label("signed");
                        self.op("bvc", &fix);
                        self.op("eor", "#128");
                        self.at(&fix);
                        self.op(if op == "<" { "bmi" } else { "bpl" }, yes);
                    } else {
                        self.op(if op == "<" { "bcc" } else { "bcs" }, yes);
                    }
                }
            }
        }
        self.op("jmp", no);
        self.temp = saved;
        Ok(true)
    }

    /// `target = value` for byte and 16-bit names and byte-indexed arrays.
    /// Returns false if the form is left to the general lowering.
    pub(super) fn assign_direct(&mut self, target: &Expr, value: &Expr) -> Result<bool, Diagnostic> {
        let saved = self.temp;
        match &target.kind {
            ExprKind::Name(n) => {
                let Some(s) = self.plain_storage(n) else {
                    return Ok(false);
                };
                if target.ty.wide() {
                    self.wide_to(value, &s.label)?;
                } else {
                    self.byte_expr(value)?;
                    self.op("sta", &s.label);
                }
            }
            ExprKind::Index(base, index, _) if !index.ty.wide() => {
                let ExprKind::Name(n) = &base.kind else {
                    return Ok(false);
                };
                let Some(s) = self.plain_storage(n) else {
                    return Ok(false);
                };
                let Ty::Array(t, len) = &s.ty else {
                    return Ok(false);
                };
                // Source order: a call in the value may change the index.
                let ordered = has_call(value) && !self.byte_operand(index, 'x').is_some_and(|o| o.text.starts_with('#'));
                let in_y = matches!(&index.kind, ExprKind::Name(n) if self.y_counter.as_deref() == Some(n.as_str()));
                if t.size() == 1 {
                    if in_y {
                        self.byte_expr(value)?;
                        self.op("sta", format!("{},y", s.label));
                    } else if let Some(k) = eval_const(index, &self.constants) {
                        self.byte_expr(value)?;
                        self.op("sta", format!("{}+{k}", s.label));
                    } else if !ordered && self.byte_operand(index, 'x').is_some() {
                        let i = self.byte_operand(index, 'x').unwrap();
                        self.byte_expr(value)?;
                        self.apply_index(&i, "x");
                        self.op("sta", format!("{},x", s.label));
                    } else if self.byte_operand(value, 'y').is_some() && !ordered {
                        let v = self.byte_operand(value, 'y').unwrap();
                        self.byte_expr(index)?;
                        self.op("tax", "");
                        self.apply("lda", &v);
                        self.op("sta", format!("{},x", s.label));
                    } else {
                        let ti = self.temp(1);
                        self.byte_expr(index)?;
                        self.op("sta", &ti);
                        self.byte_expr(value)?;
                        self.op("ldx", &ti);
                        self.op("sta", format!("{},x", s.label));
                    }
                } else if t.size() == 2 && *len <= 128 {
                    let ti = self.temp(1);
                    self.byte_expr(index)?;
                    self.op("asl", "a");
                    self.op("sta", &ti);
                    let w = self.wide_value(value)?;
                    self.op("ldx", &ti);
                    self.op("lda", &w.lo);
                    self.op("sta", format!("{},x", s.label));
                    self.op("lda", &w.hi);
                    self.op("sta", format!("{}+1,x", s.label));
                } else {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
        self.temp = saved;
        Ok(true)
    }

    fn apply_index(&mut self, index: &Operand, reg: &str) {
        self.op(&format!("ld{reg}"), &index.text);
    }

    /// Direct parameter stores, when evaluating later arguments cannot
    /// observe or clobber earlier parameter stores.
    pub(super) fn direct_arguments(
        &mut self,
        callee: &Function,
        args: &[Expr],
    ) -> Result<bool, Diagnostic> {
        if callee.name == self.function || args.iter().any(has_call) {
            return Ok(false);
        }
        let params: BTreeMap<&str, String> = callee
            .params
            .iter()
            .map(|p| (p.0.as_str(), super::local_label(&callee.name, &p.0)))
            .collect();
        for (arg, param) in args.iter().zip(&callee.params) {
            let label = &params[param.0.as_str()];
            if param.1.wide() {
                self.wide_to(arg, label)?;
            } else {
                self.byte_expr(arg)?;
                self.op("sta", label);
            }
        }
        Ok(true)
    }
}

fn widened_byte(e: &Expr) -> Option<&Expr> {
    match &e.kind {
        ExprKind::Call(n, args) if (n == "u16" || n == "i16") && !args[0].ty.wide() => {
            Some(&args[0])
        }
        _ => None,
    }
}

/// Whether evaluating `e` may call user code, runtime helpers or assembly.
pub(super) fn has_call(e: &Expr) -> bool {
    let mut found = false;
    super::visit_expr(e, &mut |e| match &e.kind {
        ExprKind::Call(n, _) if !cast_name(n) && n != "buttons" && n != "pressed" => {
            found = true
        }
        ExprKind::Asm(..) | ExprKind::Match(..) => found = true,
        ExprKind::Binary(op, _, _) if ["*", "/", "%"].contains(&op.as_str()) => found = true,
        _ => {}
    });
    found
}
