//! Structured 6502 machine code: instructions, the inline-assembly parser,
//! and the encoder with branch relaxation.
use crate::mir::{InstId, ObjId, VarId};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Op {
    Adc,
    And,
    Asl,
    Bcc,
    Bcs,
    Beq,
    Bit,
    Bmi,
    Bne,
    Bpl,
    Brk,
    Bvc,
    Bvs,
    Clc,
    Cld,
    Cli,
    Clv,
    Cmp,
    Cpx,
    Cpy,
    Dec,
    Dex,
    Dey,
    Eor,
    Inc,
    Inx,
    Iny,
    Jmp,
    Jsr,
    Lda,
    Ldx,
    Ldy,
    Lsr,
    Nop,
    Ora,
    Pha,
    Php,
    Pla,
    Plp,
    Rol,
    Ror,
    Rti,
    Rts,
    Sbc,
    Sec,
    Sed,
    Sei,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Tsx,
    Txa,
    Txs,
    Tya,
}

const NAMES: &[&str] = &[
    "adc", "and", "asl", "bcc", "bcs", "beq", "bit", "bmi", "bne", "bpl", "brk", "bvc", "bvs",
    "clc", "cld", "cli", "clv", "cmp", "cpx", "cpy", "dec", "dex", "dey", "eor", "inc", "inx",
    "iny", "jmp", "jsr", "lda", "ldx", "ldy", "lsr", "nop", "ora", "pha", "php", "pla", "plp",
    "rol", "ror", "rti", "rts", "sbc", "sec", "sed", "sei", "sta", "stx", "sty", "tax", "tay",
    "tsx", "txa", "txs", "tya",
];

const ALL: [Op; 56] = {
    use Op::*;
    [
        Adc, And, Asl, Bcc, Bcs, Beq, Bit, Bmi, Bne, Bpl, Brk, Bvc, Bvs, Clc, Cld, Cli, Clv, Cmp,
        Cpx, Cpy, Dec, Dex, Dey, Eor, Inc, Inx, Iny, Jmp, Jsr, Lda, Ldx, Ldy, Lsr, Nop, Ora, Pha,
        Php, Pla, Plp, Rol, Ror, Rti, Rts, Sbc, Sec, Sed, Sei, Sta, Stx, Sty, Tax, Tay, Tsx, Txa,
        Txs, Tya,
    ]
};

impl Op {
    pub fn name(self) -> &'static str {
        NAMES[self as usize]
    }
    pub fn from_name(s: &str) -> Option<Op> {
        let s = s.to_ascii_lowercase();
        NAMES.iter().position(|n| *n == s).map(|i| ALL[i])
    }
    pub fn is_branch(self) -> bool {
        use Op::*;
        matches!(self, Bcc | Bcs | Beq | Bmi | Bne | Bpl | Bvc | Bvs)
    }
    pub fn inverse(self) -> Op {
        use Op::*;
        match self {
            Bcc => Bcs,
            Bcs => Bcc,
            Beq => Bne,
            Bne => Beq,
            Bmi => Bpl,
            Bpl => Bmi,
            Bvc => Bvs,
            Bvs => Bvc,
            o => o,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    ZpX,
    ZpY,
    Abs,
    AbsX,
    AbsY,
    Ind,
    IndX,
    IndY,
    Rel,
}

pub fn opcode(op: Op, mode: Mode) -> Option<u8> {
    use Mode::*;
    use Op::*;
    // ALU group: imm zp zpx abs absx absy indx indy.
    let alu = |base: u8| -> Option<u8> {
        Some(match mode {
            Imm => base + 0x08,
            Zp => base + 0x04,
            ZpX => base + 0x14,
            Abs => base + 0x0c,
            AbsX => base + 0x1c,
            AbsY => base + 0x18,
            IndX => base,
            IndY => base + 0x10,
            _ => return None,
        })
    };
    // Shift group: acc zp zpx abs absx.
    let shift = |base: u8| -> Option<u8> {
        Some(match mode {
            Acc => base + 0x0a,
            Zp => base + 0x06,
            ZpX => base + 0x16,
            Abs => base + 0x0e,
            AbsX => base + 0x1e,
            _ => return None,
        })
    };
    let one = |m: Mode, code: u8| if mode == m { Some(code) } else { None };
    match op {
        Ora => alu(0x01),
        And => alu(0x21),
        Eor => alu(0x41),
        Adc => alu(0x61),
        Lda => alu(0xa1),
        Cmp => alu(0xc1),
        Sbc => alu(0xe1),
        Sta => {
            if mode == Imm {
                None
            } else {
                alu(0x81)
            }
        }
        Asl => shift(0x00),
        Rol => shift(0x20),
        Lsr => shift(0x40),
        Ror => shift(0x60),
        Inc => match mode {
            Zp => Some(0xe6),
            ZpX => Some(0xf6),
            Abs => Some(0xee),
            AbsX => Some(0xfe),
            _ => None,
        },
        Dec => match mode {
            Zp => Some(0xc6),
            ZpX => Some(0xd6),
            Abs => Some(0xce),
            AbsX => Some(0xde),
            _ => None,
        },
        Ldx => match mode {
            Imm => Some(0xa2),
            Zp => Some(0xa6),
            ZpY => Some(0xb6),
            Abs => Some(0xae),
            AbsY => Some(0xbe),
            _ => None,
        },
        Ldy => match mode {
            Imm => Some(0xa0),
            Zp => Some(0xa4),
            ZpX => Some(0xb4),
            Abs => Some(0xac),
            AbsX => Some(0xbc),
            _ => None,
        },
        Stx => match mode {
            Zp => Some(0x86),
            ZpY => Some(0x96),
            Abs => Some(0x8e),
            _ => None,
        },
        Sty => match mode {
            Zp => Some(0x84),
            ZpX => Some(0x94),
            Abs => Some(0x8c),
            _ => None,
        },
        Cpx => match mode {
            Imm => Some(0xe0),
            Zp => Some(0xe4),
            Abs => Some(0xec),
            _ => None,
        },
        Cpy => match mode {
            Imm => Some(0xc0),
            Zp => Some(0xc4),
            Abs => Some(0xcc),
            _ => None,
        },
        Bit => match mode {
            Zp => Some(0x24),
            Abs => Some(0x2c),
            _ => None,
        },
        Jmp => match mode {
            Abs => Some(0x4c),
            Ind => Some(0x6c),
            _ => None,
        },
        Jsr => one(Abs, 0x20),
        Bpl => one(Rel, 0x10),
        Bmi => one(Rel, 0x30),
        Bvc => one(Rel, 0x50),
        Bvs => one(Rel, 0x70),
        Bcc => one(Rel, 0x90),
        Bcs => one(Rel, 0xb0),
        Bne => one(Rel, 0xd0),
        Beq => one(Rel, 0xf0),
        Brk => one(Imp, 0x00),
        Php => one(Imp, 0x08),
        Clc => one(Imp, 0x18),
        Plp => one(Imp, 0x28),
        Sec => one(Imp, 0x38),
        Rti => one(Imp, 0x40),
        Pha => one(Imp, 0x48),
        Cli => one(Imp, 0x58),
        Rts => one(Imp, 0x60),
        Pla => one(Imp, 0x68),
        Sei => one(Imp, 0x78),
        Dey => one(Imp, 0x88),
        Txa => one(Imp, 0x8a),
        Tya => one(Imp, 0x98),
        Txs => one(Imp, 0x9a),
        Tay => one(Imp, 0xa8),
        Tax => one(Imp, 0xaa),
        Clv => one(Imp, 0xb8),
        Tsx => one(Imp, 0xba),
        Iny => one(Imp, 0xc8),
        Dex => one(Imp, 0xca),
        Cld => one(Imp, 0xd8),
        Inx => one(Imp, 0xe8),
        Nop => one(Imp, 0xea),
        Sed => one(Imp, 0xf8),
    }
}

pub fn size(mode: Mode) -> usize {
    use Mode::*;
    match mode {
        Imp | Acc => 1,
        Imm | Zp | ZpX | ZpY | IndX | IndY | Rel => 2,
        Abs | AbsX | AbsY | Ind => 3,
    }
}

/// (minimum, maximum) cycles, excluding taken-branch page crossings beyond
/// the stated maximum.
pub fn cycles(op: Op, mode: Mode) -> (u8, u8) {
    use Mode::*;
    use Op::*;
    if op.is_branch() {
        return (2, 4);
    }
    match op {
        Brk => return (7, 7),
        Jsr | Rts | Rti => return (6, 6),
        Pha | Php => return (3, 3),
        Pla | Plp => return (4, 4),
        Jmp => return if mode == Ind { (5, 5) } else { (3, 3) },
        _ => {}
    }
    let rmw = matches!(op, Asl | Lsr | Rol | Ror | Inc | Dec);
    let store = matches!(op, Sta | Stx | Sty);
    let n = match mode {
        Imp | Acc | Imm => 2,
        Zp => 3 + 2 * rmw as u8,
        ZpX | ZpY => 4 + 2 * rmw as u8,
        Abs => 4 + 2 * rmw as u8,
        AbsX | AbsY => {
            if rmw {
                7
            } else if store {
                5
            } else {
                4
            }
        }
        IndX => 6,
        IndY => {
            if store {
                6
            } else {
                5
            }
        }
        _ => 2,
    };
    (
        n,
        n + u8::from(!store && !rmw && matches!(mode, AbsX | AbsY | IndY)),
    )
}

/// A symbolic address.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sym {
    Num,
    Var(InstId, VarId),
    /// Object base; `true` selects a wide array's high-byte table.
    Obj(ObjId, bool),
    Label(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Full,
    Lo,
    Hi,
}

/// `sym + off`, optionally reduced to its low or high byte.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Opnd {
    pub sym: Sym,
    pub off: i64,
    pub part: Part,
}

impl Opnd {
    pub fn num(n: i64) -> Self {
        Opnd {
            sym: Sym::Num,
            off: n,
            part: Part::Full,
        }
    }
    pub fn sym(sym: Sym) -> Self {
        Opnd {
            sym,
            off: 0,
            part: Part::Full,
        }
    }
    pub fn label(l: impl Into<String>) -> Self {
        Opnd::sym(Sym::Label(l.into()))
    }
    pub fn plus(&self, n: i64) -> Self {
        Opnd {
            off: self.off + n,
            ..self.clone()
        }
    }
    pub fn lo(mut self) -> Self {
        self.part = Part::Lo;
        self
    }
    pub fn hi(mut self) -> Self {
        self.part = Part::Hi;
        self
    }
    pub fn number(&self) -> Option<i64> {
        match (&self.sym, self.part) {
            (Sym::Num, Part::Full) => Some(self.off),
            (Sym::Num, Part::Lo) => Some(self.off & 255),
            (Sym::Num, Part::Hi) => Some((self.off >> 8) & 255),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    Imp,
    Acc,
    Imm(Opnd),
    Mem(Opnd),
    MemX(Opnd),
    MemY(Opnd),
    Ind(Opnd),
    IndX(Opnd),
    IndY(Opnd),
}

impl Arg {
    pub fn opnd(&self) -> Option<&Opnd> {
        match self {
            Arg::Imm(o)
            | Arg::Mem(o)
            | Arg::MemX(o)
            | Arg::MemY(o)
            | Arg::Ind(o)
            | Arg::IndX(o)
            | Arg::IndY(o) => Some(o),
            _ => None,
        }
    }
    pub fn opnd_mut(&mut self) -> Option<&mut Opnd> {
        match self {
            Arg::Imm(o)
            | Arg::Mem(o)
            | Arg::MemX(o)
            | Arg::MemY(o)
            | Arg::Ind(o)
            | Arg::IndX(o)
            | Arg::IndY(o) => Some(o),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ins {
    pub op: Op,
    pub arg: Arg,
}

impl Ins {
    pub fn new(op: Op, arg: Arg) -> Self {
        Ins { op, arg }
    }
    pub fn imp(op: Op) -> Self {
        Ins { op, arg: Arg::Imp }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Ins(Ins),
    Label(String),
    Bytes(Vec<Opnd>),
    Words(Vec<Opnd>),
    /// Reserve zero bytes.
    Fill(usize),
    /// Source location (file, start) for the line table.
    Src(u32, u32),
    /// Optimization barrier around inline assembly.
    Barrier,
}

impl fmt::Display for Opnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = match self.part {
            Part::Full => "",
            Part::Lo => "<",
            Part::Hi => ">",
        };
        let base = match &self.sym {
            Sym::Num => return write!(f, "{prefix}${:X}", self.off),
            Sym::Var(i, v) => format!("v{}_{}", i.0, v.0),
            Sym::Obj(o, hi) => format!("o{}{}", o.0, if *hi { "h" } else { "" }),
            Sym::Label(l) => l.clone(),
        };
        if self.off == 0 {
            write!(f, "{prefix}{base}")
        } else if self.off > 0 {
            write!(f, "{prefix}{base}+{}", self.off)
        } else {
            write!(f, "{prefix}{base}{}", self.off)
        }
    }
}

impl fmt::Display for Ins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.op.name();
        match &self.arg {
            Arg::Imp => write!(f, "{n}"),
            Arg::Acc => write!(f, "{n} a"),
            Arg::Imm(o) => write!(f, "{n} #{o}"),
            Arg::Mem(o) => write!(f, "{n} {o}"),
            Arg::MemX(o) => write!(f, "{n} {o},x"),
            Arg::MemY(o) => write!(f, "{n} {o},y"),
            Arg::Ind(o) => write!(f, "{n} ({o})"),
            Arg::IndX(o) => write!(f, "{n} ({o},x)"),
            Arg::IndY(o) => write!(f, "{n} ({o}),y"),
        }
    }
}

// ----------------------------------------------------------- inline parser

/// Parse inline assembly. `{N}` placeholders become `refs(N)`; labels are
/// prefixed with `prefix` so each block's labels are unique.
pub fn parse(
    text: &str,
    prefix: &str,
    refs: &dyn Fn(usize) -> Option<Opnd>,
) -> Result<Vec<Line>, (usize, String)> {
    let mut out = Vec::new();
    let mut local = std::collections::BTreeSet::new();
    for raw in text.lines() {
        let code = raw.split(';').next().unwrap_or("").trim();
        if let Some((label, _)) = code.split_once(':') {
            let l = label.trim();
            if is_ident(l) {
                local.insert(l.to_owned());
            }
        }
    }
    for (n, raw) in text.lines().enumerate() {
        let line = n + 1;
        let mut code = raw.split(';').next().unwrap_or("").trim();
        if code.is_empty() {
            continue;
        }
        if let Some((label, rest)) = code.split_once(':') {
            if is_ident(label.trim()) {
                out.push(Line::Label(format!("{prefix}{}", label.trim())));
                code = rest.trim();
                if code.is_empty() {
                    continue;
                }
            }
        }
        let (word, rest) = match code.split_once(char::is_whitespace) {
            Some((w, r)) => (w, r.trim()),
            None => (code, ""),
        };
        let value = |s: &str| -> Result<Opnd, (usize, String)> {
            expr(s, prefix, &local, refs).map_err(|e| (line, e))
        };
        match word.to_ascii_lowercase().as_str() {
            ".byte" | ".db" => {
                let mut v = Vec::new();
                for part in split_args(rest) {
                    if let Some(s) = part.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
                        v.extend(s.bytes().map(|b| Opnd::num(b as i64)));
                    } else {
                        v.push(value(&part)?);
                    }
                }
                out.push(Line::Bytes(v));
                continue;
            }
            ".word" | ".dw" => {
                let v = split_args(rest)
                    .iter()
                    .map(|p| value(p))
                    .collect::<Result<_, _>>()?;
                out.push(Line::Words(v));
                continue;
            }
            ".res" => {
                let n = value(rest)?
                    .number()
                    .ok_or((line, "'.res' needs a number".to_owned()))?;
                out.push(Line::Fill(n as usize));
                continue;
            }
            _ => {}
        }
        let op = Op::from_name(word).ok_or((line, format!("Unknown instruction '{word}'")))?;
        let arg = if rest.is_empty() {
            Arg::Imp
        } else if rest.eq_ignore_ascii_case("a") {
            Arg::Acc
        } else if let Some(v) = rest.strip_prefix('#') {
            Arg::Imm(value(v)?)
        } else if rest.starts_with('(') {
            let t = rest.replace(' ', "");
            let lower = t.to_ascii_lowercase();
            if lower.ends_with(",x)") {
                Arg::IndX(value(&t[1..t.len() - 3])?)
            } else if lower.ends_with("),y") {
                Arg::IndY(value(&t[1..t.len() - 3])?)
            } else if t.ends_with(')') {
                Arg::Ind(value(&t[1..t.len() - 1])?)
            } else {
                // A parenthesized expression used as an address.
                Arg::Mem(value(&t)?)
            }
        } else if let Some((v, r)) = rest.rsplit_once(',') {
            match r.trim().to_ascii_lowercase().as_str() {
                "x" => Arg::MemX(value(v)?),
                "y" => Arg::MemY(value(v)?),
                _ => return Err((line, format!("Unknown index '{r}'"))),
            }
        } else {
            Arg::Mem(value(rest)?)
        };
        let ins = Ins::new(op, arg);
        if !encodable(&ins) {
            return Err((line, format!("'{}' has no such addressing mode", op.name())));
        }
        if let (Op::Jmp, Arg::Ind(o)) = (&ins.op, &ins.arg) {
            if let Some(n) = o.number() {
                if n & 0xff == 0xff {
                    return Err((line, "jmp ($xxFF) triggers the 6502 page-wrap bug".into()));
                }
            }
        }
        out.push(Line::Ins(ins));
    }
    Ok(out)
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@' || c == '.')
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = false;
    for c in s.chars() {
        match c {
            '"' => {
                quote = !quote;
                cur.push(c);
            }
            ',' if !quote => {
                out.push(cur.trim().to_owned());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

fn expr(
    s: &str,
    prefix: &str,
    local: &std::collections::BTreeSet<String>,
    refs: &dyn Fn(usize) -> Option<Opnd>,
) -> Result<Opnd, String> {
    let s = s.trim();
    if let Some(v) = s.strip_prefix('<') {
        return Ok(expr(v, prefix, local, refs)?.lo());
    }
    if let Some(v) = s.strip_prefix('>') {
        return Ok(expr(v, prefix, local, refs)?.hi());
    }
    // Trailing `+n` / `-n` offsets.
    for (p, c) in s.char_indices().rev() {
        if p > 0 && (c == '+' || c == '-') {
            let rhs = s[p + 1..].trim();
            if let Some(n) = number(rhs) {
                let base = expr(&s[..p], prefix, local, refs)?;
                return Ok(base.plus(if c == '+' { n } else { -n }));
            }
        }
    }
    if let Some(n) = number(s) {
        return Ok(Opnd::num(n));
    }
    if let Some(i) = s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        let i: usize = i.parse().map_err(|_| format!("Bad placeholder '{s}'"))?;
        return refs(i).ok_or_else(|| format!("Unresolved placeholder '{s}'"));
    }
    if is_ident(s) {
        if local.contains(s) {
            return Ok(Opnd::label(format!("{prefix}{s}")));
        }
        return Ok(Opnd::label(s));
    }
    Err(format!("Cannot read operand '{s}'"))
}

fn number(s: &str) -> Option<i64> {
    let s = s.replace('_', "");
    if let Some(v) = s.strip_prefix('$').or_else(|| s.strip_prefix("0x")) {
        i64::from_str_radix(v, 16).ok()
    } else if let Some(v) = s.strip_prefix('%').or_else(|| s.strip_prefix("0b")) {
        i64::from_str_radix(v, 2).ok()
    } else if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        s.parse().ok()
    } else {
        None
    }
}

/// Some addressing mode of the instruction exists (zero page or absolute).
pub fn encodable(ins: &Ins) -> bool {
    let candidates: &[Mode] = match &ins.arg {
        Arg::Imp => &[Mode::Imp, Mode::Acc],
        Arg::Acc => &[Mode::Acc],
        Arg::Imm(_) => &[Mode::Imm],
        Arg::Mem(_) => &[Mode::Zp, Mode::Abs, Mode::Rel],
        Arg::MemX(_) => &[Mode::ZpX, Mode::AbsX],
        Arg::MemY(_) => &[Mode::ZpY, Mode::AbsY],
        Arg::Ind(_) => &[Mode::Ind],
        Arg::IndX(_) => &[Mode::IndX],
        Arg::IndY(_) => &[Mode::IndY],
    };
    candidates.iter().any(|m| opcode(ins.op, *m).is_some())
}

/// Check inline assembly syntax without resolving names.
pub fn validate(text: &str) -> Result<(), (usize, String)> {
    parse(text, "_", &|_| Some(Opnd::num(0x10))).map(|_| ())
}

// ------------------------------------------------------------------ encode

pub struct Encoded {
    pub bytes: Vec<u8>,
    pub labels: BTreeMap<String, u16>,
    /// (address, size, cycles, text, source) per instruction.
    pub listing: Vec<ListItem>,
    pub relaxed: usize,
}

#[derive(Clone, Debug)]
pub struct ListItem {
    pub address: u16,
    pub bytes: Vec<u8>,
    pub text: String,
    pub cycles: (u8, u8),
    pub src: Option<(u32, u32)>,
}

/// Addressing mode for a resolved operand value.
pub fn mode_of(ins: &Ins, value: Option<i64>) -> Option<Mode> {
    let zp = value.is_some_and(|v| (0..256).contains(&v));
    let pick = |zm: Mode, am: Mode| {
        if zp && opcode(ins.op, zm).is_some() {
            Some(zm)
        } else if opcode(ins.op, am).is_some() {
            Some(am)
        } else if opcode(ins.op, zm).is_some() && value.is_none() {
            Some(zm)
        } else {
            None
        }
    };
    match &ins.arg {
        Arg::Imp => {
            if opcode(ins.op, Mode::Imp).is_some() {
                Some(Mode::Imp)
            } else {
                Some(Mode::Acc).filter(|m| opcode(ins.op, *m).is_some())
            }
        }
        Arg::Acc => Some(Mode::Acc),
        Arg::Imm(_) => Some(Mode::Imm),
        Arg::Mem(_) if ins.op.is_branch() => Some(Mode::Rel),
        Arg::Mem(_) => pick(Mode::Zp, Mode::Abs),
        Arg::MemX(_) => pick(Mode::ZpX, Mode::AbsX),
        Arg::MemY(_) => pick(Mode::ZpY, Mode::AbsY),
        Arg::Ind(_) => Some(Mode::Ind),
        Arg::IndX(_) => Some(Mode::IndX),
        Arg::IndY(_) => Some(Mode::IndY),
    }
}

/// Encode a section at `base`. `resolve` maps non-label symbols to values;
/// labels defined in `lines` are resolved here, others through `external`.
pub fn encode(
    lines: &[Line],
    base: u16,
    resolve: &dyn Fn(&Sym) -> Option<i64>,
    external: &dyn Fn(&str) -> Option<i64>,
) -> Result<Encoded, String> {
    // Branches needing a long form (inverse branch over a jump).
    let mut long: Vec<bool> = vec![false; lines.len()];
    let mut relaxed = 0;
    for _round in 0..64 {
        let mut labels = BTreeMap::new();
        let mut pc = base as i64;
        // Pass 1: sizes.
        for (i, l) in lines.iter().enumerate() {
            match l {
                Line::Label(n) => {
                    labels.insert(n.clone(), pc as u16);
                }
                Line::Ins(ins) => {
                    if ins.op.is_branch() {
                        pc += if long[i] { 5 } else { 2 };
                    } else {
                        let v = ins.arg.opnd().and_then(|o| value(o, resolve, &|_| None));
                        let m = mode_of(ins, v).ok_or_else(|| format!("Cannot encode '{ins}'"))?;
                        pc += size(m) as i64;
                    }
                }
                Line::Bytes(v) => pc += v.len() as i64,
                Line::Words(v) => pc += 2 * v.len() as i64,
                Line::Fill(n) => pc += *n as i64,
                _ => {}
            }
        }
        // Pass 2: check branch ranges.
        let mut changed = false;
        pc = base as i64;
        for (i, l) in lines.iter().enumerate() {
            if let Line::Ins(ins) = l {
                if ins.op.is_branch() {
                    if !long[i] {
                        let target = ins.arg.opnd().and_then(|o| {
                            value(o, resolve, &|n| {
                                labels.get(n).map(|v| *v as i64).or_else(|| external(n))
                            })
                        });
                        if let Some(t) = target {
                            let d = t - (pc + 2);
                            if !(-128..=127).contains(&d) {
                                long[i] = true;
                                changed = true;
                                relaxed += 1;
                            }
                        }
                    }
                    pc += if long[i] { 5 } else { 2 };
                } else {
                    let v = ins.arg.opnd().and_then(|o| value(o, resolve, &|_| None));
                    pc += size(mode_of(ins, v).unwrap()) as i64;
                }
            } else {
                match l {
                    Line::Bytes(v) => pc += v.len() as i64,
                    Line::Words(v) => pc += 2 * v.len() as i64,
                    Line::Fill(n) => pc += *n as i64,
                    _ => {}
                }
            }
        }
        if changed {
            continue;
        }
        // Pass 3: emit.
        let lookup = |n: &str| labels.get(n).map(|v| *v as i64).or_else(|| external(n));
        let mut bytes = Vec::new();
        let mut listing = Vec::new();
        let mut src = None;
        pc = base as i64;
        for (i, l) in lines.iter().enumerate() {
            match l {
                Line::Src(f, s) => src = Some((*f, *s)),
                Line::Ins(ins) => {
                    let start = bytes.len();
                    let get = |o: &Opnd| {
                        value(o, resolve, &lookup)
                            .ok_or_else(|| format!("Unresolved symbol in '{ins}'"))
                    };
                    let cyc;
                    if ins.op.is_branch() {
                        let t = get(ins.arg.opnd().unwrap())?;
                        if long[i] {
                            bytes.push(opcode(ins.op.inverse(), Mode::Rel).unwrap());
                            bytes.push(3);
                            bytes.push(0x4c);
                            bytes.push(t as u8);
                            bytes.push((t >> 8) as u8);
                            cyc = (3, 5);
                        } else {
                            let d = t - (pc + 2);
                            if !(-128..=127).contains(&d) {
                                return Err(format!("Branch out of range in '{ins}'"));
                            }
                            bytes.push(opcode(ins.op, Mode::Rel).unwrap());
                            bytes.push(d as i8 as u8);
                            cyc = cycles(ins.op, Mode::Rel);
                        }
                    } else {
                        let pre = ins.arg.opnd().and_then(|o| value(o, resolve, &|_| None));
                        let m = mode_of(ins, pre).unwrap();
                        let v = match ins.arg.opnd() {
                            Some(o) => get(o)?,
                            None => 0,
                        };
                        if matches!(
                            m,
                            Mode::Zp | Mode::ZpX | Mode::ZpY | Mode::IndX | Mode::IndY
                        ) && !(0..256).contains(&v)
                        {
                            return Err(format!(
                                "'{ins}' needs a zero-page address, found ${v:04X}"
                            ));
                        }
                        if m == Mode::Imm && !(-128..256).contains(&v) {
                            return Err(format!(
                                "Immediate value {v} does not fit a byte in '{ins}'"
                            ));
                        }
                        if m == Mode::Ind && v & 0xff == 0xff {
                            return Err(format!(
                                "'{ins}' triggers the 6502 indirect-jump page bug"
                            ));
                        }
                        bytes.push(opcode(ins.op, m).unwrap());
                        match size(m) {
                            2 => bytes.push(v as u8),
                            3 => {
                                bytes.push(v as u8);
                                bytes.push((v >> 8) as u8);
                            }
                            _ => {}
                        }
                        cyc = cycles(ins.op, m);
                    }
                    listing.push(ListItem {
                        address: pc as u16,
                        bytes: bytes[start..].to_vec(),
                        text: ins.to_string(),
                        cycles: cyc,
                        src: src.take(),
                    });
                    pc = base as i64 + bytes.len() as i64;
                }
                Line::Bytes(v) => {
                    for o in v {
                        let x = value(o, resolve, &lookup).ok_or("Unresolved byte value")?;
                        bytes.push(x as u8);
                    }
                    pc = base as i64 + bytes.len() as i64;
                }
                Line::Words(v) => {
                    for o in v {
                        let x = value(o, resolve, &lookup).ok_or("Unresolved word value")?;
                        bytes.push(x as u8);
                        bytes.push((x >> 8) as u8);
                    }
                    pc = base as i64 + bytes.len() as i64;
                }
                Line::Fill(n) => {
                    bytes.extend(std::iter::repeat_n(0, *n));
                    pc = base as i64 + bytes.len() as i64;
                }
                Line::Label(_) | Line::Barrier => {}
            }
        }
        return Ok(Encoded {
            bytes,
            labels,
            listing,
            relaxed,
        });
    }
    Err("Branch relaxation did not converge".into())
}

pub fn value(
    o: &Opnd,
    resolve: &dyn Fn(&Sym) -> Option<i64>,
    labels: &dyn Fn(&str) -> Option<i64>,
) -> Option<i64> {
    let base = match &o.sym {
        Sym::Num => 0,
        Sym::Label(l) => labels(l)?,
        s => resolve(s)?,
    };
    let v = base + o.off;
    Some(match o.part {
        Part::Full => v,
        Part::Lo => v & 255,
        Part::Hi => (v >> 8) & 255,
    })
}
