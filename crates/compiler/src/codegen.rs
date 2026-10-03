use crate::{frontend::*, optimizer::calls_block};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapper {
    Nrom,
    Mmc1,
    Uxrom,
    Mmc3,
}
impl Mapper {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "" | "nrom" | "0" => Some(Self::Nrom),
            "mmc1" | "1" => Some(Self::Mmc1),
            "uxrom" | "unrom" | "2" => Some(Self::Uxrom),
            "mmc3" | "4" => Some(Self::Mmc3),
            _ => None,
        }
    }
    pub fn id(self) -> u8 {
        match self {
            Self::Nrom => 0,
            Self::Mmc1 => 1,
            Self::Uxrom => 2,
            Self::Mmc3 => 4,
        }
    }
    pub fn base(self) -> u16 {
        if self == Self::Nrom {
            0x8000
        } else {
            0xc000
        }
    }
    pub fn bank_size(self) -> usize {
        if self == Self::Mmc3 {
            8192
        } else {
            16384
        }
    }
}
#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub name: String,
    pub address: u16,
    pub size: usize,
    pub kind: String,
}
#[derive(Clone, Debug)]
pub struct Generated {
    pub assembly: String,
    pub memory: Vec<MemoryEntry>,
    pub ram_bytes: usize,
    pub zero_page_bytes: usize,
    pub frame_bytes: usize,
    pub bank_data: BTreeMap<u8, Vec<u8>>,
    pub mapper: Mapper,
    pub bank_reads: usize,
    pub call_depth: usize,
    pub stack_bytes: usize,
}
#[derive(Clone)]
struct Storage {
    label: String,
    ty: Ty,
    constant: Option<i64>,
    bank: Option<u8>,
}
struct Generator<'a> {
    program: &'a Program,
    assets: &'a BTreeMap<String, Vec<u8>>,
    mapper: Mapper,
    lines: Vec<String>,
    data: Vec<String>,
    globals: BTreeMap<String, Storage>,
    signatures: BTreeMap<String, (Vec<Ty>, Ty)>,
    constants: BTreeMap<String, i64>,
    function: String,
    locals: BTreeMap<String, Storage>,
    temp: usize,
    temp_max: usize,
    label: usize,
    loops: Vec<(String, String)>,
    memory: Vec<MemoryEntry>,
    bank_data: BTreeMap<u8, Vec<u8>>,
    bank_reads: usize,
}
fn err(span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(span, message)
}
fn bytes(label: &str, data: &[u8]) -> String {
    let mut s = format!("{label}:\n");
    for v in data.chunks(24) {
        s.push_str("  .byte ");
        s.push_str(&v.iter().map(u8::to_string).collect::<Vec<_>>().join(","));
        s.push('\n');
    }
    s
}
impl Generator<'_> {
    fn emit(&mut self, s: impl Into<String>) {
        self.lines.push(s.into());
    }
    fn op(&mut self, op: &str, arg: impl AsRef<str>) {
        self.emit(if arg.as_ref().is_empty() {
            format!("  {op}")
        } else {
            format!("  {op} {}", arg.as_ref())
        });
    }
    fn mark(&mut self, s: Span) {
        self.emit(format!(";@src {},{}", s.start, s.end));
    }
    fn label(&mut self, hint: &str) -> String {
        let s = format!("__{}_{}_{}", self.function, hint, self.label);
        self.label += 1;
        s
    }
    fn at(&mut self, label: &str) {
        self.emit(format!("{label}:"));
    }
    fn temp(&mut self, n: usize) -> String {
        let s = format!("__v_{}_tmp+{}", self.function, self.temp);
        self.temp += n;
        self.temp_max = self.temp_max.max(self.temp);
        s
    }
    fn storage(&self, n: &str) -> Result<Storage, Diagnostic> {
        self.locals
            .get(n)
            .or(self.globals.get(n))
            .cloned()
            .ok_or_else(|| err(Span::default(), format!("Unknown storage '{n}'")))
    }
    fn number(&mut self, n: i64, t: &Ty) {
        self.op("lda", format!("#{}", n & 255));
        if t.wide() {
            self.op("ldx", format!("#{}", (n >> 8) & 255));
        }
    }
    fn volatile_label(&self, label: &str) -> bool {
        self.program
            .globals
            .iter()
            .any(|g| g.attrs.contains_key("shared") && label == format!("__v_{}", g.name))
    }
    fn save(&mut self, label: &str, t: &Ty) {
        let shared = self.volatile_label(label);
        if shared {
            self.emit(";@barrier begin");
        }
        self.op("sta", label);
        if t.wide() {
            self.op("stx", format!("{label}+1"));
        }
        if shared {
            self.emit(";@barrier end");
        }
    }
    fn load(&mut self, label: &str, t: &Ty) {
        let shared = self.volatile_label(label);
        if shared {
            self.emit(";@barrier begin");
        }
        self.op("lda", label);
        if t.wide() {
            self.op("ldx", format!("{label}+1"));
        }
        if shared {
            self.emit(";@barrier end");
        }
    }
    fn extend(&mut self, t: &Ty) {
        if !t.wide() {
            if t.signed() {
                let positive = self.label("extend_pos");
                let done = self.label("extend_done");
                self.op("cmp", "#128");
                self.op("bcc", &positive);
                self.op("ldx", "#255");
                self.op("jmp", &done);
                self.at(&positive);
                self.op("ldx", "#0");
                self.at(&done);
            } else {
                self.op("ldx", "#0");
            }
        }
    }
    fn bank_for(&mut self, s: &Storage) {
        if let Some(n) = s.bank {
            self.op("lda", format!("#{n}"));
            self.op("sta", "$04");
            self.op("jsr", "__rt_bank");
            self.bank_reads += 1;
        }
    }
    fn address(&mut self, s: &Storage, index: &Expr) -> Result<(), Diagnostic> {
        self.expression(index)?;
        self.extend(&index.ty);
        if let Ty::Array(t, _) = &s.ty {
            if t.size() == 2 {
                self.op("asl", "a");
                self.op("pha", "");
                self.op("txa", "");
                self.op("rol", "a");
                self.op("tax", "");
                self.op("pla", "");
            }
        }
        self.op("clc", "");
        self.op("adc", format!("#<{}", s.label));
        self.op("sta", "$0a");
        self.op("txa", "");
        self.op("adc", format!("#>{}", s.label));
        self.op("sta", "$0b");
        Ok(())
    }
    fn expression(&mut self, e: &Expr) -> Result<(), Diagnostic> {
        let saved = self.temp;
        match &e.kind {
            ExprKind::Number(n) => self.number(*n, &e.ty),
            ExprKind::String(_) => {
                return Err(err(
                    e.span,
                    "String values are only valid in text/screen calls",
                ))
            }
            ExprKind::Name(n) => {
                let s = self.storage(n)?;
                if let Some(n) = s.constant {
                    self.number(n, &e.ty);
                } else {
                    self.load(&s.label, &e.ty);
                }
            }
            ExprKind::Index(base, index, _) => {
                let name = if let ExprKind::Name(n) = &base.kind {
                    n
                } else {
                    return Err(err(base.span, "Array base must be named storage"));
                };
                let s = self.storage(name)?;
                if s.bank.is_some() {
                    let t = self.temp(if index.ty.wide() { 2 } else { 1 });
                    self.expression(index)?;
                    self.save(&t, &index.ty);
                    self.bank_for(&s);
                    self.load(&t, &index.ty);
                    let idx = Expr {
                        kind: ExprKind::Name(t.to_string()),
                        ty: index.ty.clone(),
                        span: index.span,
                    };
                    self.locals.insert(
                        t.clone(),
                        Storage {
                            label: t.clone(),
                            ty: index.ty.clone(),
                            constant: None,
                            bank: None,
                        },
                    );
                    self.read_index(&s, &idx, &e.ty)?;
                    self.locals.remove(&t);
                } else {
                    self.read_index(&s, index, &e.ty)?;
                }
            }
            ExprKind::Unary(op, x) => {
                self.expression(x)?;
                match op.as_str() {
                    "+" => {}
                    "!" => {
                        let yes = self.label("not_yes");
                        let done = self.label("not_done");
                        self.op("cmp", "#0");
                        self.op("beq", &yes);
                        self.op("lda", "#0");
                        self.op("jmp", &done);
                        self.at(&yes);
                        self.op("lda", "#1");
                        self.at(&done);
                    }
                    "~" => {
                        self.op("eor", "#255");
                        if e.ty.wide() {
                            let t = self.temp(1);
                            self.op("sta", &t);
                            self.op("txa", "");
                            self.op("eor", "#255");
                            self.op("tax", "");
                            self.op("lda", &t);
                        }
                    }
                    "-" => {
                        self.op("eor", "#255");
                        self.op("clc", "");
                        self.op("adc", "#1");
                        if e.ty.wide() {
                            let t = self.temp(1);
                            self.op("sta", &t);
                            self.op("txa", "");
                            self.op("eor", "#255");
                            self.op("adc", "#0");
                            self.op("tax", "");
                            self.op("lda", &t);
                        }
                    }
                    _ => {}
                }
            }
            ExprKind::Binary(op, l, r) => {
                if ["==", "!=", "<", ">", "<=", ">=", "&&", "||"].contains(&op.as_str()) {
                    let yes = self.label("bool_yes");
                    let no = self.label("bool_no");
                    let end = self.label("bool_end");
                    self.condition(e, &yes, &no)?;
                    self.at(&yes);
                    self.op("lda", "#1");
                    self.op("jmp", &end);
                    self.at(&no);
                    self.op("lda", "#0");
                    self.at(&end);
                } else if ["+", "-", "&", "|", "^"].contains(&op.as_str())
                    && eval_const(r, &self.constants).is_some()
                {
                    let n = eval_const(r, &self.constants).unwrap();
                    self.expression(l)?;
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
                    self.op(inst, format!("#{}", n & 255));
                    if e.ty.wide() {
                        let t = self.temp(1);
                        self.op("sta", &t);
                        self.op("txa", "");
                        self.op(inst, format!("#{}", (n >> 8) & 255));
                        self.op("tax", "");
                        self.op("lda", &t);
                    }
                } else if op == "<<" || op == ">>" {
                    self.shift(op, l, r)?;
                } else if ["*", "/", "%"].contains(&op.as_str()) {
                    self.math(op, l, r)?;
                } else {
                    let t = self.temp(if e.ty.wide() { 4 } else { 2 });
                    self.expression(l)?;
                    self.save(&t, &e.ty);
                    self.expression(r)?;
                    if e.ty.wide() {
                        self.save(&format!("{t}+2"), &e.ty);
                        self.op("lda", &t);
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
                            "^" => "eor",
                            _ => return Err(err(e.span, "Unsupported binary operator")),
                        };
                        self.op(inst, format!("{t}+2"));
                        self.op("sta", format!("{t}+2"));
                        self.op("lda", format!("{t}+1"));
                        self.op(inst, format!("{t}+3"));
                        self.op("tax", "");
                        self.op("lda", format!("{t}+2"));
                    } else {
                        let inst = match op.as_str() {
                            "+" => {
                                self.op("clc", "");
                                "adc"
                            }
                            "-" => {
                                self.op("sta", format!("{t}+1"));
                                self.op("lda", &t);
                                self.op("sec", "");
                                "sbc"
                            }
                            "&" => "and",
                            "|" => "ora",
                            "^" => "eor",
                            _ => return Err(err(e.span, "Unsupported binary operator")),
                        };
                        self.op(inst, if op == "-" { format!("{t}+1") } else { t });
                    }
                }
            }
            ExprKind::Call(name, args) => self.call(name, args, e)?,
            ExprKind::Asm(inputs, body) => {
                let mut temps = Vec::new();
                for (reg, value) in inputs {
                    let t = self.temp(1);
                    self.expression(value)?;
                    self.op("sta", &t);
                    temps.push((reg, t));
                }
                for (r, t) in temps {
                    self.op(&format!("ld{r}"), t);
                }
                self.emit(";@barrier begin");
                let mut replacements = BTreeMap::new();
                for (n, s) in self.globals.iter().chain(self.locals.iter()) {
                    replacements.insert(n.clone(), s.label.clone());
                }
                for n in self.signatures.keys() {
                    replacements.insert(n.clone(), format!("__fn_{n}"));
                }
                for line in body.lines() {
                    if let Some((label, _)) = asm_code(line).trim().split_once(':') {
                        if !label.contains(char::is_whitespace) {
                            let unique = self.label("asm");
                            replacements.insert(label.into(), unique);
                        }
                    }
                }
                let raw = transform_asm_symbols(body, |name| {
                    replacements
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.into())
                });
                self.emit(raw);
                self.emit(";@barrier end");
            }
        }
        self.temp = saved;
        Ok(())
    }
    fn read_index(&mut self, s: &Storage, index: &Expr, result: &Ty) -> Result<(), Diagnostic> {
        if let Some(n) = eval_const(index, &self.constants) {
            let offset = n * result.size() as i64;
            self.op("lda", format!("{}+{offset}", s.label));
            if result.wide() {
                self.op("ldx", format!("{}+{}", s.label, offset + 1));
            }
        } else if !index.ty.wide() && result.size() == 1 {
            self.expression(index)?;
            self.op("tax", "");
            self.op("lda", format!("{},x", s.label));
        } else {
            self.address(s, index)?;
            self.op("ldy", "#0");
            self.op("lda", "($0a),y");
            if result.wide() {
                let t = self.temp(1);
                self.op("sta", &t);
                self.op("iny", "");
                self.op("lda", "($0a),y");
                self.op("tax", "");
                self.op("lda", t);
            }
        }
        Ok(())
    }
    fn shift_once(&mut self, op: &str, t: &Ty, temp: &str) {
        if op == "<<" {
            self.op("asl", "a");
            if t.wide() {
                self.op("pha", "");
                self.op("txa", "");
                self.op("rol", "a");
                self.op("tax", "");
                self.op("pla", "");
            }
        } else if t.wide() {
            self.op("stx", temp);
            if t.signed() {
                self.op("pha", "");
                self.op("txa", "");
                self.op("cmp", "#128");
                self.op("pla", "");
                self.op("ror", temp);
            } else {
                self.op("lsr", temp);
            }
            self.op("ror", "a");
            self.op("ldx", temp);
        } else if t.signed() {
            self.op("cmp", "#128");
            self.op("ror", "a");
        } else {
            self.op("lsr", "a");
        }
    }
    fn shift(&mut self, op: &str, l: &Expr, r: &Expr) -> Result<(), Diagnostic> {
        let scratch = self.temp(4);
        if let Some(n) = eval_const(r, &self.constants) {
            self.expression(l)?;
            let width = if l.ty.wide() { 16 } else { 8 };
            if n < 0 || n >= width {
                if op == ">>" && l.ty.signed() {
                    self.op("cmp", if l.ty.wide() { "#0" } else { "#128" });
                    if l.ty.wide() {
                        self.op("txa", "");
                        self.op("cmp", "#128");
                    }
                    let zero = self.label("shift_zero");
                    let done = self.label("shift_done");
                    self.op("bcc", &zero);
                    self.number(-1, &l.ty);
                    self.op("jmp", &done);
                    self.at(&zero);
                    self.number(0, &l.ty);
                    self.at(&done);
                } else {
                    self.number(0, &l.ty);
                }
            } else {
                for _ in 0..n {
                    self.shift_once(op, &l.ty, &scratch);
                }
            }
        } else {
            self.expression(l)?;
            self.save(&scratch, &l.ty);
            self.expression(r)?;
            self.op("sta", format!("{scratch}+2"));
            let zero = self.label("dynamic_shift_zero");
            let loop_l = self.label("dynamic_shift");
            let end = self.label("dynamic_shift_done");
            self.op("cmp", if l.ty.wide() { "#16" } else { "#8" });
            self.op("bcs", &zero);
            self.load(&scratch, &l.ty);
            self.at(&loop_l);
            self.op("ldy", format!("{scratch}+2"));
            self.op("beq", &end);
            self.shift_once(op, &l.ty, &format!("{scratch}+3"));
            self.op("dec", format!("{scratch}+2"));
            self.op("jmp", &loop_l);
            self.at(&zero);
            if op == ">>" && l.ty.signed() {
                self.op(
                    "lda",
                    if l.ty.wide() {
                        format!("{scratch}+1")
                    } else {
                        scratch.clone()
                    },
                );
                self.op("cmp", "#128");
                let positive = self.label("dynamic_shift_positive");
                self.op("bcc", &positive);
                self.number(-1, &l.ty);
                self.op("jmp", &end);
                self.at(&positive);
            }
            self.number(0, &l.ty);
            self.at(&end);
        }
        Ok(())
    }
    fn math(&mut self, op: &str, l: &Expr, r: &Expr) -> Result<(), Diagnostic> {
        if op == "*" {
            if let Some(n) = eval_const(r, &self.constants) {
                let bits = (n as u16) & if l.ty.wide() { 65535 } else { 255 };
                if bits.count_ones() <= 4 {
                    let t = self.temp(if l.ty.wide() { 3 } else { 1 });
                    self.expression(l)?;
                    self.save(&t, &l.ty);
                    if bits == 0 {
                        self.number(0, &l.ty);
                        return Ok(());
                    }
                    let top = 15 - bits.leading_zeros();
                    for bit in (0..top).rev() {
                        self.shift_once("<<", &l.ty, &format!("{t}+2"));
                        if bits & (1 << bit) != 0 {
                            self.op("clc", "");
                            self.op("adc", &t);
                            if l.ty.wide() {
                                self.op("sta", format!("{t}+2"));
                                self.op("txa", "");
                                self.op("adc", format!("{t}+1"));
                                self.op("tax", "");
                                self.op("lda", format!("{t}+2"));
                            }
                        }
                    }
                    return Ok(());
                }
            }
        }
        let t = self.temp(6);
        self.expression(l)?;
        self.extend(&l.ty);
        self.save(&t, &Ty::U16);
        self.expression(r)?;
        self.extend(&r.ty);
        self.save(&format!("{t}+2"), &Ty::U16);
        if l.ty.signed() && op != "*" {
            self.op("lda", format!("{t}+1"));
            self.op("and", "#128");
            self.op("sta", format!("{t}+4"));
            self.op("lda", format!("{t}+3"));
            self.op("and", "#128");
            self.op("eor", format!("{t}+4"));
            self.op("sta", format!("{t}+5"));
            for offset in [0, 2] {
                let positive = self.label("math_abs");
                self.op("lda", format!("{t}+{}", offset + 1));
                self.op("bpl", &positive);
                self.op("lda", format!("{t}+{offset}"));
                self.op("eor", "#255");
                self.op("clc", "");
                self.op("adc", "#1");
                self.op("sta", format!("{t}+{offset}"));
                self.op("lda", format!("{t}+{}", offset + 1));
                self.op("eor", "#255");
                self.op("adc", "#0");
                self.op("sta", format!("{t}+{}", offset + 1));
                self.at(&positive);
            }
        }
        for i in 0..4 {
            self.op("lda", format!("{t}+{i}"));
            self.op("sta", format!("${:02x}", 4 + i));
        }
        self.op("jsr", if op == "*" { "__rt_mul" } else { "__rt_div" });
        if op == "%" {
            self.op("lda", "$0f");
            self.op("ldx", "$1f");
        }
        if l.ty.signed() && op != "*" {
            self.save(&t, &Ty::U16);
            self.op("lda", format!("{t}+{}", if op == "%" { 4 } else { 5 }));
            let positive = self.label("math_sign");
            self.op("beq", &positive);
            self.op("lda", &t);
            self.op("eor", "#255");
            self.op("clc", "");
            self.op("adc", "#1");
            self.op("sta", &t);
            self.op("lda", format!("{t}+1"));
            self.op("eor", "#255");
            self.op("adc", "#0");
            self.op("sta", format!("{t}+1"));
            self.at(&positive);
            self.load(&t, &Ty::U16);
        }
        Ok(())
    }
    fn condition(&mut self, e: &Expr, yes: &str, no: &str) -> Result<(), Diagnostic> {
        let saved = self.temp;
        match &e.kind {
            ExprKind::Number(n) => self.op("jmp", if *n != 0 { yes } else { no }),
            ExprKind::Unary(op, x) if op == "!" => self.condition(x, no, yes)?,
            ExprKind::Binary(op, l, r) if op == "&&" => {
                let next = self.label("and_rhs");
                self.condition(l, &next, no)?;
                self.at(&next);
                self.condition(r, yes, no)?;
            }
            ExprKind::Binary(op, l, r) if op == "||" => {
                let next = self.label("or_rhs");
                self.condition(l, yes, &next)?;
                self.at(&next);
                self.condition(r, yes, no)?;
            }
            ExprKind::Binary(op, l, r)
                if ["==", "!=", "<", ">", "<=", ">="].contains(&op.as_str()) =>
            {
                if !l.ty.wide() {
                    if let Some(n) = eval_const(r, &self.constants) {
                        self.expression(l)?;
                        let equal = op == "==" || op == "!=";
                        let mut rhs = n & 255;
                        if l.ty.signed() && !equal {
                            self.op("eor", "#128");
                            rhs ^= 128;
                        }
                        self.op("cmp", format!("#{rhs}"));
                        match op.as_str() {
                            "==" => self.op("beq", yes),
                            "!=" => self.op("bne", yes),
                            "<" => self.op("bcc", yes),
                            ">=" => self.op("bcs", yes),
                            "<=" => {
                                self.op("bcc", yes);
                                self.op("beq", yes);
                            }
                            ">" => {
                                self.op("bcc", no);
                                self.op("bne", yes);
                            }
                            _ => {}
                        }
                        self.op("jmp", no);
                        self.temp = saved;
                        return Ok(());
                    }
                }
                let t = self.temp(4);
                self.expression(l)?;
                self.save(&t, &l.ty);
                self.expression(r)?;
                self.save(&format!("{t}+2"), &r.ty);
                let equal = op == "==" || op == "!=";
                if l.ty.wide() {
                    if l.ty.signed() && !equal {
                        self.op("lda", format!("{t}+3"));
                        self.op("eor", "#128");
                        self.op("sta", format!("{t}+3"));
                        self.op("lda", format!("{t}+1"));
                        self.op("eor", "#128");
                    } else {
                        self.op("lda", format!("{t}+1"));
                    }
                    self.op("cmp", format!("{t}+3"));
                    match op.as_str() {
                        "==" => self.op("bne", no),
                        "!=" => self.op("bne", yes),
                        "<" | "<=" => {
                            self.op("bcc", yes);
                            self.op("bne", no);
                        }
                        ">" | ">=" => {
                            self.op("bcc", no);
                            self.op("bne", yes);
                        }
                        _ => {}
                    }
                }
                if l.ty.signed() && !l.ty.wide() && !equal {
                    self.op("lda", format!("{t}+2"));
                    self.op("eor", "#128");
                    self.op("sta", format!("{t}+2"));
                    self.op("lda", &t);
                    self.op("eor", "#128");
                } else {
                    self.op("lda", &t);
                }
                self.op("cmp", format!("{t}+2"));
                match op.as_str() {
                    "==" => self.op("beq", yes),
                    "!=" => self.op("bne", yes),
                    "<" => self.op("bcc", yes),
                    ">=" => self.op("bcs", yes),
                    "<=" => {
                        self.op("bcc", yes);
                        self.op("beq", yes);
                    }
                    ">" => {
                        self.op("bcc", no);
                        self.op("bne", yes);
                    }
                    _ => {}
                }
                self.op("jmp", no);
            }
            _ => {
                self.expression(e)?;
                self.op("cmp", "#0");
                self.op("bne", yes);
                self.op("jmp", no);
            }
        }
        self.temp = saved;
        Ok(())
    }
    fn call(&mut self, name: &str, args: &[Expr], e: &Expr) -> Result<(), Diagnostic> {
        if ["u8", "i8", "u16", "i16", "bool"].contains(&name) {
            self.expression(&args[0])?;
            if e.ty.wide() && !args[0].ty.wide() {
                self.extend(&args[0].ty);
            }
            if e.ty == Ty::Bool {
                let nonzero = self.label("cast_true");
                let done = self.label("cast_done");
                if args[0].ty.wide() {
                    let t = self.temp(1);
                    self.op("sta", &t);
                    self.op("txa", "");
                    self.op("ora", t);
                }
                self.op("cmp", "#0");
                self.op("bne", &nonzero);
                self.op("lda", "#0");
                self.op("jmp", &done);
                self.at(&nonzero);
                self.op("lda", "#1");
                self.at(&done);
            }
            return Ok(());
        }
        if name == "buttons" || name == "pressed" {
            self.op("lda", if name == "buttons" { "$01" } else { "$02" });
            return Ok(());
        }
        if name == "peek" {
            if let Some(n) = eval_const(&args[0], &self.constants) {
                self.op("lda", format!("${:04x}", n as u16));
            } else {
                self.expression(&args[0])?;
                self.op("sta", "$0a");
                self.op("stx", "$0b");
                self.op("ldy", "#0");
                self.op("lda", "($0a),y");
            }
            return Ok(());
        }
        if name == "poke" {
            if let Some(n) = eval_const(&args[0], &self.constants) {
                self.expression(&args[1])?;
                self.op("sta", format!("${:04x}", n as u16));
            } else {
                let t = self.temp(3);
                self.expression(&args[0])?;
                self.save(&t, &Ty::U16);
                self.expression(&args[1])?;
                self.op("sta", format!("{t}+2"));
                self.op("lda", &t);
                self.op("sta", "$0a");
                self.op("lda", format!("{t}+1"));
                self.op("sta", "$0b");
                self.op("lda", format!("{t}+2"));
                self.op("ldy", "#0");
                self.op("sta", "($0a),y");
            }
            return Ok(());
        }
        if ["bank", "bank_peek", "screen_bank"].contains(&name) && self.mapper == Mapper::Nrom {
            return Err(err(e.span, "NROM has no PRG bank switching"));
        }
        let builtin = builtins().into_iter().find(|b| b.name == name);
        let mut values = Vec::new();
        for arg in args {
            if let ExprKind::String(s) = &arg.kind {
                let label = self.label("literal");
                let data = if name == "screen" {
                    let d = self
                        .assets
                        .get(s)
                        .ok_or_else(|| err(arg.span, format!("Missing screen asset '{s}'")))?;
                    if d.len() != 1024 {
                        return Err(err(arg.span, "Screen assets must contain 1024 bytes"));
                    }
                    d.clone()
                } else {
                    if !s.is_ascii() || s.len() > 255 {
                        return Err(err(arg.span, "Text must contain at most 255 ASCII bytes"));
                    }
                    s.as_bytes().to_vec()
                };
                self.data.push(bytes(&label, &data));
                values.push((label, Ty::Str, data.len()));
            } else {
                let t = self.temp(arg.ty.size());
                self.expression(arg)?;
                self.save(&t, &arg.ty);
                values.push((t, arg.ty.clone(), 0));
            }
        }
        if builtin.is_some() {
            let mut offset = 4;
            for (v, t, len) in values {
                if t == Ty::Str {
                    self.op("lda", format!("#<{v}"));
                    self.op("sta", format!("${offset:02x}"));
                    offset += 1;
                    self.op("lda", format!("#>{v}"));
                    self.op("sta", format!("${offset:02x}"));
                    offset += 1;
                    if name == "text" {
                        self.op("lda", format!("#{len}"));
                        self.op("sta", format!("${offset:02x}"));
                        offset += 1;
                    }
                } else {
                    self.op("lda", &v);
                    self.op("sta", format!("${offset:02x}"));
                    offset += 1;
                    if t.wide() {
                        self.op("lda", format!("{v}+1"));
                        self.op("sta", format!("${offset:02x}"));
                        offset += 1;
                    }
                }
            }
            self.op("jsr", format!("__rt_{name}"));
        } else {
            let sig = self
                .signatures
                .get(name)
                .cloned()
                .ok_or_else(|| err(e.span, "Unknown callee"))?;
            let f = self
                .program
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap();
            for ((value, ty, _), param) in values.into_iter().zip(&f.params) {
                self.load(&value, &ty);
                self.save(&format!("__v_{}_{}", name, param.0), &param.1);
            }
            self.op("jsr", format!("__fn_{name}"));
            if sig.1 != e.ty {
                return Err(err(e.span, "Call result type changed during lowering"));
            }
        }
        Ok(())
    }
    fn assign(&mut self, target: &Expr, value: &Expr) -> Result<(), Diagnostic> {
        let saved = self.temp;
        match &target.kind {
            ExprKind::Name(n) => {
                let s = self.storage(n)?;
                self.expression(value)?;
                self.save(&s.label, &target.ty);
            }
            ExprKind::Index(base, index, _) => {
                let n = if let ExprKind::Name(n) = &base.kind {
                    n
                } else {
                    return Err(err(target.span, "Array target must be named"));
                };
                let s = self.storage(n)?;
                let t = self.temp(if target.ty.wide() { 4 } else { 3 });
                self.expression(index)?;
                self.save(&t, &index.ty);
                self.expression(value)?;
                self.save(&format!("{t}+2"), &target.ty);
                if !index.ty.wide() && !target.ty.wide() {
                    self.op("ldx", &t);
                    self.op("lda", format!("{t}+2"));
                    self.op("sta", format!("{},x", s.label));
                } else {
                    let idx = Expr {
                        kind: ExprKind::Name(t.clone()),
                        ty: index.ty.clone(),
                        span: index.span,
                    };
                    self.locals.insert(
                        t.clone(),
                        Storage {
                            label: t.clone(),
                            ty: index.ty.clone(),
                            constant: None,
                            bank: None,
                        },
                    );
                    self.address(&s, &idx)?;
                    self.locals.remove(&t);
                    self.op("ldy", "#0");
                    self.op("lda", format!("{t}+2"));
                    self.op("sta", "($0a),y");
                    if target.ty.wide() {
                        self.op("iny", "");
                        self.op("lda", format!("{t}+3"));
                        self.op("sta", "($0a),y");
                    }
                }
            }
            _ => return Err(err(target.span, "Invalid assignment target")),
        }
        self.temp = saved;
        Ok(())
    }
    fn statements(&mut self, body: &[Stmt]) -> Result<(), Diagnostic> {
        for s in body {
            match s {
                Stmt::Local { name, value, .. } => {
                    self.mark(value.span);
                    let t = Expr {
                        kind: ExprKind::Name(name.clone()),
                        ty: value.ty.clone(),
                        span: value.span,
                    };
                    self.assign(&t, value)?;
                }
                Stmt::Assign { target, op, value } => {
                    self.mark(target.span);
                    if op == "=" {
                        self.assign(target, value)?;
                    } else if (op == "+=" || op == "-=")
                        && eval_const(value, &self.constants) == Some(1)
                        && target.ty.size() == 1
                        && matches!(target.kind, ExprKind::Name(_))
                    {
                        if let ExprKind::Name(n) = &target.kind {
                            let s = self.storage(n)?;
                            self.op(if op == "+=" { "inc" } else { "dec" }, s.label);
                        }
                    } else {
                        let saved = self.temp;
                        let mut stable = target.clone();
                        let mut temporary = None;
                        if let ExprKind::Index(_, index, _) = &mut stable.kind {
                            let label = self.temp(index.ty.size());
                            self.expression(index)?;
                            self.save(&label, &index.ty);
                            self.locals.insert(
                                label.clone(),
                                Storage {
                                    label: label.clone(),
                                    ty: index.ty.clone(),
                                    constant: None,
                                    bank: None,
                                },
                            );
                            index.kind = ExprKind::Name(label.clone());
                            temporary = Some(label);
                        }
                        let v = Expr {
                            kind: ExprKind::Binary(
                                op.trim_end_matches('=').into(),
                                Box::new(stable.clone()),
                                Box::new(value.clone()),
                            ),
                            ty: target.ty.clone(),
                            span: target.span,
                        };
                        self.assign(&stable, &v)?;
                        if let Some(label) = temporary {
                            self.locals.remove(&label);
                        }
                        self.temp = saved;
                    }
                }
                Stmt::Expr(e) => {
                    self.mark(e.span);
                    self.expression(e)?;
                }
                Stmt::If { condition, yes, no } => {
                    self.mark(condition.span);
                    let y = self.label("if");
                    let n = self.label("else");
                    let done = self.label("endif");
                    self.condition(condition, &y, &n)?;
                    self.at(&y);
                    self.statements(yes)?;
                    self.op("jmp", &done);
                    self.at(&n);
                    self.statements(no)?;
                    self.at(&done);
                }
                Stmt::While { condition, body } => {
                    let test = self.label("while_test");
                    let yes = self.label("while_body");
                    let done = self.label("while_end");
                    self.at(&test);
                    self.mark(condition.span);
                    self.condition(condition, &yes, &done)?;
                    self.at(&yes);
                    self.loops.push((done.clone(), test.clone()));
                    self.statements(body)?;
                    self.loops.pop();
                    self.op("jmp", &test);
                    self.at(&done);
                }
                Stmt::For {
                    name,
                    start,
                    end,
                    body,
                    span,
                } => {
                    let t = self.storage(name)?.ty;
                    let counter = Expr {
                        kind: ExprKind::Name(name.clone()),
                        ty: t.clone(),
                        span: *span,
                    };
                    self.assign(&counter, start)?;
                    let test = self.label("for_test");
                    let yes = self.label("for_body");
                    let next = self.label("for_next");
                    let done = self.label("for_end");
                    self.at(&test);
                    let condition = Expr {
                        kind: ExprKind::Binary(
                            "<".into(),
                            Box::new(counter.clone()),
                            Box::new(end.clone()),
                        ),
                        ty: Ty::Bool,
                        span: *span,
                    };
                    self.condition(&condition, &yes, &done)?;
                    self.at(&yes);
                    self.loops.push((done.clone(), next.clone()));
                    self.statements(body)?;
                    self.loops.pop();
                    self.at(&next);
                    let label = self.storage(name)?.label;
                    self.op("inc", &label);
                    if t.wide() {
                        let no_carry = self.label("for_no_carry");
                        self.op("bne", &no_carry);
                        self.op("inc", format!("{label}+1"));
                        self.at(&no_carry);
                    }
                    self.op("jmp", &test);
                    self.at(&done);
                }
                Stmt::Loop(body) => {
                    let start = self.label("loop");
                    let done = self.label("loop_end");
                    self.at(&start);
                    self.loops.push((done.clone(), start.clone()));
                    self.statements(body)?;
                    self.loops.pop();
                    self.op("jmp", &start);
                    self.at(&done);
                }
                Stmt::Break(s) => {
                    self.mark(*s);
                    let l = self.loops.last().unwrap().0.clone();
                    self.op("jmp", l);
                }
                Stmt::Continue(s) => {
                    self.mark(*s);
                    let l = self.loops.last().unwrap().1.clone();
                    self.op("jmp", l);
                }
                Stmt::Return(value, s) => {
                    self.mark(*s);
                    if let Some(e) = value {
                        self.expression(e)?;
                    }
                    self.op("rts", "");
                }
            }
        }
        Ok(())
    }
}

fn reach(
    name: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> Result<(), String> {
    if done.contains(name) {
        return Ok(());
    }
    if visiting.len() >= 64 {
        return Err("Call depth exceeds 64; the 6502 hardware stack is finite".into());
    }
    if !visiting.insert(name.into()) {
        return Err(format!(
            "Recursive call cycle involving '{name}' is incompatible with static frames"
        ));
    }
    if let Some(calls) = graph.get(name) {
        for c in calls {
            if graph.contains_key(c) {
                reach(c, graph, visiting, done)?;
            }
        }
    }
    visiting.remove(name);
    done.insert(name.into());
    Ok(())
}
fn frame_offset(
    name: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    sizes: &BTreeMap<String, usize>,
    offsets: &mut BTreeMap<String, usize>,
    offset: usize,
) {
    if offsets.get(name).is_some_and(|n| *n >= offset) {
        return;
    }
    offsets.insert(name.into(), offset);
    if let Some(calls) = graph.get(name) {
        for child in calls {
            if sizes.contains_key(child) {
                frame_offset(child, graph, sizes, offsets, offset + sizes[name]);
            }
        }
    }
}
pub fn generate(
    program: &Program,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<Generated, Diagnostic> {
    let mapper = Mapper::parse(
        program
            .config
            .get("mapper")
            .map(String::as_str)
            .unwrap_or(""),
    )
    .ok_or_else(|| {
        err(
            Span::default(),
            "Supported mappers are nrom, mmc1, uxrom, and mmc3",
        )
    })?;
    for key in program.config.keys() {
        if ![
            "title",
            "mapper",
            "region",
            "mirroring",
            "chr",
            "palette",
            "screen",
        ]
        .contains(&key.as_str())
        {
            return Err(err(
                Span::default(),
                format!("Unknown cartridge property '{key}'"),
            ));
        }
    }
    if program
        .config
        .get("mirroring")
        .is_some_and(|s| !["vertical", "horizontal"].contains(&s.as_str()))
    {
        return Err(err(
            Span::default(),
            "Mirroring must be vertical or horizontal",
        ));
    }
    for attr in ["reset", "nmi", "irq"] {
        if program
            .functions
            .iter()
            .filter(|f| f.attrs.contains_key(attr))
            .count()
            > 1
        {
            return Err(err(
                Span::default(),
                format!("Only one @{attr} entry point is allowed"),
            ));
        }
    }
    if program
        .config
        .get("region")
        .is_some_and(|s| !s.eq_ignore_ascii_case("ntsc"))
    {
        return Err(err(
            Span::default(),
            "This runtime currently supports NTSC; PAL requires a different timing/audio profile",
        ));
    }
    let mut g = Generator {
        program,
        assets,
        mapper,
        lines: Vec::new(),
        data: Vec::new(),
        globals: BTreeMap::new(),
        signatures: program
            .functions
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    (
                        f.params.iter().map(|p| p.1.clone()).collect(),
                        f.result.clone(),
                    ),
                )
            })
            .collect(),
        constants: BTreeMap::new(),
        function: String::new(),
        locals: BTreeMap::new(),
        temp: 0,
        temp_max: 0,
        label: 0,
        loops: Vec::new(),
        memory: Vec::new(),
        bank_data: BTreeMap::new(),
        bank_reads: 0,
    };
    let mut zp = 0x20usize;
    let mut ram = 0x400usize;
    let mut eq = Vec::new();
    if mapper == Mapper::Uxrom {
        let path = program
            .config
            .get("chr")
            .ok_or_else(|| err(Span::default(), "UxROM needs a CHR asset for its CHR RAM"))?;
        let chr = assets
            .get(path)
            .ok_or_else(|| err(Span::default(), format!("Missing CHR asset '{path}'")))?;
        if chr.len() != 8192 {
            return Err(err(Span::default(), "CHR asset must be 8192 bytes"));
        }
        g.bank_data.insert(0, chr.clone());
    }
    for global in &program.globals {
        let label = format!("__v_{}", global.name);
        let mut constant = None;
        let mut bank = None;
        if global.constant {
            if let Initial::Scalar(e) = &global.initial {
                constant = eval_const(e, &g.constants);
                if let Some(n) = constant {
                    g.constants.insert(global.name.clone(), n);
                    eq.push(format!("{label} = {}", n & 65535));
                }
            } else {
                if let Some(b) = global.attrs.get("bank") {
                    if mapper == Mapper::Nrom {
                        return Err(err(global.span, "NROM has no switchable banks"));
                    }
                    let n = b
                        .parse::<u8>()
                        .map_err(|_| err(global.span, "Bank index must be an integer 0..255"))?;
                    bank = Some(n);
                    let data = g.bank_data.entry(n).or_default();
                    if data.len() + global.data.len() > mapper.bank_size() {
                        return Err(err(global.span, "Banked data exceeds mapper window"));
                    }
                    eq.push(format!("{label} = {}", 0x8000 + data.len()));
                    data.extend(&global.data);
                } else {
                    g.data.push(bytes(&label, &global.data));
                }
            }
        } else {
            let size = global.ty.size();
            let use_zp = !global.attrs.contains_key("ram")
                && (size <= 2 || global.attrs.contains_key("zp"))
                && zp + size <= 256;
            let address = if use_zp {
                let n = zp;
                zp += size;
                n
            } else {
                if global.attrs.contains_key("zp") {
                    return Err(err(
                        global.span,
                        "Explicit zero-page allocation exceeds available space",
                    ));
                }
                let n = ram;
                ram += size;
                n
            };
            eq.push(format!("{label} = {address}"));
            g.memory.push(MemoryEntry {
                name: global.name.clone(),
                address: address as u16,
                size,
                kind: if use_zp { "zero-page" } else { "RAM" }.into(),
            });
        }
        g.globals.insert(
            global.name.clone(),
            Storage {
                label,
                ty: global.ty.clone(),
                constant,
                bank,
            },
        );
    }
    let mut functions = BTreeMap::new();
    let mut sizes = BTreeMap::new();
    let mut local_layouts = BTreeMap::new();
    let mut graph = BTreeMap::new();
    for f in &program.functions {
        let mut calls = BTreeSet::new();
        let mut asm = false;
        calls_block(&f.body, &mut calls, &mut asm);
        graph.insert(f.name.clone(), calls);
        g.function = f.name.clone();
        g.locals.clear();
        g.temp = 0;
        g.temp_max = 0;
        g.lines.clear();
        let mut layout = Vec::new();
        let mut offset = 0;
        for (name, ty) in &f.locals {
            let label = format!("__v_{}_{}", f.name, name);
            g.locals.insert(
                name.clone(),
                Storage {
                    label: label.clone(),
                    ty: ty.clone(),
                    constant: None,
                    bank: None,
                },
            );
            layout.push((label, offset, ty.size()));
            offset += ty.size();
        }
        g.at(&format!("__fn_{}", f.name));
        g.statements(&f.body)?;
        g.op("rts", "");
        g.at(&format!("__fn_{}_end", f.name));
        layout.push((format!("__v_{}_tmp", f.name), offset, g.temp_max));
        sizes.insert(f.name.clone(), offset + g.temp_max);
        local_layouts.insert(f.name.clone(), layout);
        functions.insert(f.name.clone(), g.lines.join("\n"));
    }
    let mut done = BTreeSet::new();
    for f in &program.functions {
        reach(&f.name, &graph, &mut BTreeSet::new(), &mut done).map_err(|m| err(f.name_span, m))?;
    }
    let depths = call_depths(&graph).map_err(|m| err(Span::default(), m))?;
    let mut pools: [BTreeMap<String, usize>; 3] = std::array::from_fn(|_| BTreeMap::new());
    for f in &program.functions {
        let context = if f.attrs.contains_key("nmi") {
            Some(1)
        } else if f.attrs.contains_key("irq") {
            Some(2)
        } else if ["init", "update"].contains(&f.name.as_str())
            || f.attrs.contains_key("reset")
            || f.attrs.contains_key("export")
        {
            Some(0)
        } else {
            None
        };
        if let Some(context) = context {
            frame_offset(&f.name, &graph, &sizes, &mut pools[context], 0);
        }
    }
    for i in 0..3 {
        for j in i + 1..3 {
            if let Some(n) = pools[i].keys().find(|n| pools[j].contains_key(*n)) {
                return Err(err(Span::default(),format!("'{n}' is reachable in multiple execution contexts; static frames require distinct main/NMI/IRQ helpers")));
            }
        }
    }
    let stack_bytes = 32
        + 2 * pools
            .iter()
            .map(|p| p.keys().map(|n| depths[n]).max().unwrap_or(0))
            .sum::<usize>();
    if stack_bytes > 256 {
        return Err(err(
            Span::default(),
            "Nested main/IRQ/NMI call paths exceed the 256-byte hardware stack allowance",
        ));
    }
    let peaks = pools
        .each_ref()
        .map(|p| p.iter().map(|(n, o)| o + sizes[n]).max().unwrap_or(0));
    let peak = peaks.iter().sum::<usize>();
    let pool = if zp + peak <= 256 {
        let base = zp;
        zp += peak;
        base
    } else {
        let base = ram;
        ram += peak;
        base
    };
    for f in &program.functions {
        let offset = (0..3)
            .find_map(|i| {
                pools[i]
                    .get(&f.name)
                    .map(|o| o + peaks[..i].iter().sum::<usize>())
            })
            .unwrap_or(0);
        for (label, local_offset, size) in &local_layouts[&f.name] {
            eq.push(format!("{label} = {}", pool + offset + local_offset));
            if *size > 0 {
                g.memory.push(MemoryEntry {
                    name: label.trim_start_matches("__v_").into(),
                    address: (pool + offset + local_offset) as u16,
                    size: *size,
                    kind: "static frame (overlaid)".into(),
                });
            }
        }
    }
    if ram > 0x800 {
        return Err(err(
            Span::default(),
            format!("RAM allocation ends at ${ram:04x}, exceeding internal RAM ($0800)"),
        ));
    }
    let reset = program
        .functions
        .iter()
        .find(|f| f.attrs.contains_key("reset"))
        .map(|f| f.name.as_str());
    let nmi = program
        .functions
        .iter()
        .find(|f| f.attrs.contains_key("nmi"))
        .map(|f| f.name.as_str());
    let irq = program
        .functions
        .iter()
        .find(|f| f.attrs.contains_key("irq"))
        .map(|f| f.name.as_str());
    for f in &program.functions {
        if (["init", "update"].contains(&f.name.as_str())
            || f.attrs
                .keys()
                .any(|a| ["reset", "nmi", "irq"].contains(&a.as_str())))
            && (!f.params.is_empty() || f.result != Ty::Void)
        {
            return Err(err(
                f.name_span,
                "Entry points require no parameters and a void result",
            ));
        }
    }
    if reset.is_none() && !functions.contains_key("update") {
        return Err(err(
            Span::default(),
            "Define fn update() for the game runtime, or an @reset entry point",
        ));
    }
    for f in &program.functions {
        if pools[1].contains_key(&f.name) || pools[2].contains_key(&f.name) {
            let mut opaque = false;
            for line in functions[&f.name].lines() {
                let line = line.trim();
                if line == ";@barrier begin" {
                    opaque = true;
                } else if line == ";@barrier end" {
                    opaque = false;
                } else if !opaque
                    && (line.starts_with("jsr __rt_") || ["sta $0a", "sta $0b"].contains(&line))
                {
                    return Err(err(f.name_span,"Interrupt lowering uses main-runtime scratch; use byte operations, constant bus addresses, or explicit assembly"));
                }
            }
            let calls = &graph[&f.name];
            if calls.iter().any(|n| {
                builtins().iter().any(|b| b.name == n)
                    && !["peek", "poke", "buttons", "pressed"].contains(&n.as_str())
            }) {
                return Err(err(f.name_span,"Interrupt helpers may use scalar operations, assembly, and explicit peek/poke; game-runtime calls share main scratch"));
            }
        }
    }
    for f in &program.functions {
        if pools[1].contains_key(&f.name) || pools[2].contains_key(&f.name) {
            check_interrupt_bus(&f.body, &g.constants)?;
        }
    }
    for global in &program.globals {
        if global.constant {
            continue;
        }
        let mut contexts = 0u8;
        for (i, p) in pools.iter().enumerate() {
            if program
                .functions
                .iter()
                .any(|f| p.contains_key(&f.name) && uses_name(&f.body, &global.name))
            {
                contexts |= 1 << i;
            }
        }
        if contexts.count_ones() > 1 && !global.attrs.contains_key("shared") {
            return Err(err(
                global.span,
                format!(
                    "'{}' crosses main/NMI/IRQ contexts; declare @shared byte storage",
                    global.name
                ),
            ));
        }
        if global.attrs.contains_key("shared") && !matches!(global.ty, Ty::U8 | Ty::I8 | Ty::Bool) {
            return Err(err(global.span,"@shared storage must be one byte; multi-byte publication requires an explicit assembly protocol"));
        }
    }
    let mut init = String::new();
    for global in &program.globals {
        if !global.constant && global.data.iter().any(|b| *b != 0) {
            let source = format!("__init_{}", global.name);
            g.data.push(bytes(&source, &global.data));
            let label = format!("__v_{}", global.name);
            let low = global.data.len() & 255;
            let high = global.data.len() >> 8;
            init.push_str(&format!(
                r#"lda #<{source}
sta $0a
lda #>{source}
sta $0b
lda #<{label}
sta $10
lda #>{label}
sta $11
lda #{low}
sta $12
lda #{high}
sta $13
jsr __rt_copy_ram
"#
            ));
        }
    }
    for (key, label, len) in [
        ("screen", "__initial_screen", 1024),
        ("palette", "__initial_palette", 32),
    ] {
        if let Some(path) = program.config.get(key) {
            let data = assets
                .get(path)
                .ok_or_else(|| err(Span::default(), format!("Missing {key} asset '{path}'")))?;
            if data.len() != len {
                return Err(err(
                    Span::default(),
                    format!("{key} asset must contain {len} bytes"),
                ));
            }
            g.data.push(bytes(label, data));
        }
    }
    let mut assembly = eq.join("\n");
    assembly.push('\n');
    assembly.push_str(&crate::runtime6502::runtime(
        mapper,
        program,
        &init,
        reset,
        nmi,
        irq,
        functions.contains_key("init"),
    ));
    for f in &program.functions {
        assembly.push_str(&functions[&f.name]);
        assembly.push('\n');
    }
    for data in g.data {
        assembly.push_str(&data);
    }
    let call_depth = depths.values().copied().max().unwrap_or(0);
    Ok(Generated {
        assembly,
        memory: g.memory,
        ram_bytes: ram - 0x400 + zp - 0x20,
        zero_page_bytes: zp - 0x20,
        frame_bytes: peak,
        bank_data: g.bank_data,
        mapper,
        bank_reads: g.bank_reads,
        call_depth,
        stack_bytes,
    })
}
fn call_depths(
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeMap<String, usize>, String> {
    let mut depths = BTreeMap::new();
    for _ in 0..64 {
        for (name, calls) in graph {
            if depths.contains_key(name) {
                continue;
            }
            let children = calls
                .iter()
                .filter(|n| graph.contains_key(*n))
                .collect::<Vec<_>>();
            if children.iter().all(|n| depths.contains_key(*n)) {
                let depth = 1 + children.iter().map(|n| depths[*n]).max().unwrap_or(0);
                if depth > 64 {
                    return Err("Call depth exceeds 64; the hardware stack is finite".into());
                }
                depths.insert(name.clone(), depth);
            }
        }
        if depths.len() == graph.len() {
            return Ok(depths);
        }
    }
    Err("Call graph exceeds 64 levels".into())
}

fn visit_expr(e: &Expr, visit: &mut impl FnMut(&Expr)) {
    visit(e);
    match &e.kind {
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => {
            visit_expr(a, visit);
            visit_expr(b, visit);
        }
        ExprKind::Unary(_, a) => visit_expr(a, visit),
        ExprKind::Call(_, v) => {
            for e in v {
                visit_expr(e, visit);
            }
        }
        ExprKind::Asm(v, _) => {
            for (_, e) in v {
                visit_expr(e, visit);
            }
        }
        _ => {}
    }
}
fn visit_block(body: &[Stmt], visit: &mut impl FnMut(&Expr)) {
    for s in body {
        match s {
            Stmt::Local { value, .. } | Stmt::Expr(value) => visit_expr(value, visit),
            Stmt::Assign { target, value, .. } => {
                visit_expr(target, visit);
                visit_expr(value, visit);
            }
            Stmt::If { condition, yes, no } => {
                visit_expr(condition, visit);
                visit_block(yes, visit);
                visit_block(no, visit);
            }
            Stmt::While { condition, body } => {
                visit_expr(condition, visit);
                visit_block(body, visit);
            }
            Stmt::For {
                start, end, body, ..
            } => {
                visit_expr(start, visit);
                visit_expr(end, visit);
                visit_block(body, visit);
            }
            Stmt::Loop(body) => visit_block(body, visit),
            Stmt::Return(Some(e), _) => visit_expr(e, visit),
            _ => {}
        }
    }
}
fn uses_name(body: &[Stmt], name: &str) -> bool {
    let mut found = false;
    visit_block(body, &mut |e| {
        if matches!(&e.kind,ExprKind::Name(n) if n==name) {
            found = true;
        }
        if let ExprKind::Asm(_, raw) = &e.kind {
            transform_asm_symbols(raw, |symbol| {
                found |= symbol == name;
                symbol.into()
            });
        }
    });
    found
}

fn asm_code(line: &str) -> &str {
    let end = [line.find(';'), line.find("//")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(line.len());
    &line[..end]
}

// Only symbols are host-language names: opcodes, hex literals and registers
// retain their 6502 meaning even when a variable has the same spelling.
fn transform_asm_symbols(body: &str, mut transform: impl FnMut(&str) -> String) -> String {
    let mut result = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        let code = asm_code(line);
        let after_label = code.rfind(':').map_or(0, |p| p + 1);
        let opcode_start =
            after_label + code[after_label..].len() - code[after_label..].trim_start().len();
        let opcode_end = code[opcode_start..]
            .find(char::is_whitespace)
            .map_or(code.len(), |p| opcode_start + p);
        let opcode = &code[opcode_start..opcode_end];
        let accumulator = ["asl", "lsr", "rol", "ror"]
            .iter()
            .any(|op| opcode.eq_ignore_ascii_case(op))
            && code[opcode_end..].trim().eq_ignore_ascii_case("a");
        let bytes = code.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos].is_ascii_alphabetic() || bytes[pos] == b'_' {
                let start = pos;
                pos += 1;
                while pos < bytes.len()
                    && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                {
                    pos += 1;
                }
                let symbol = &code[start..pos];
                let is_opcode = start >= opcode_start && start < opcode_end;
                let is_number =
                    start > 0 && (bytes[start - 1] == b'$' || bytes[start - 1].is_ascii_digit());
                let is_register = start >= opcode_end
                    && (accumulator
                        || (code[..start].trim_end().ends_with(',')
                            && (symbol.eq_ignore_ascii_case("x")
                                || symbol.eq_ignore_ascii_case("y"))));
                if is_opcode || is_number || is_register {
                    result.push_str(symbol);
                } else {
                    result.push_str(&transform(symbol));
                }
            } else {
                let c = code[pos..].chars().next().unwrap();
                result.push(c);
                pos += c.len_utf8();
            }
        }
        result.push_str(&line[code.len()..]);
    }
    result
}
fn check_interrupt_bus(body: &[Stmt], constants: &BTreeMap<String, i64>) -> Result<(), Diagnostic> {
    let mut error = None;
    visit_block(body, &mut |e| {
        if let ExprKind::Call(n, args) = &e.kind {
            if ["peek", "poke"].contains(&n.as_str())
                && args
                    .first()
                    .is_some_and(|a| eval_const(a, constants).is_none())
            {
                error=Some(err(e.span,"Interrupt bus accesses require a constant address; dynamic addressing shares runtime scratch"));
            }
        }
    });
    error.map_or(Ok(()), Err)
}
