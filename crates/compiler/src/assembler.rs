//! Official-opcode 6502 assembler with monotonically relaxing conditional branches.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug)]
pub struct Instruction {
    pub address: u16,
    pub bytes: Vec<u8>,
    pub text: String,
    pub cycles_min: u8,
    pub cycles_max: u8,
    pub source: Option<(usize, usize)>,
}
#[derive(Clone, Debug)]
pub struct Assembly {
    pub bytes: Vec<u8>,
    pub symbols: BTreeMap<String, u16>,
    pub listing: String,
    pub instructions: Vec<Instruction>,
    pub relaxed: usize,
}
#[derive(Clone, Debug)]
enum Item {
    Label(String),
    Equ(String, String),
    Op(String, String),
    Data(Vec<String>, bool),
    Reserve(usize),
    Align(usize),
    Source(usize, usize),
    Barrier,
}
#[derive(Clone, Debug)]
struct Line {
    item: Item,
    text: String,
    long: bool,
    mode: Mode,
}

pub fn opcodes() -> Vec<(&'static str, Mode, u8)> {
    use Mode::*;
    vec![
        ("adc", Imm, 0x69),
        ("adc", Zp, 0x65),
        ("adc", ZpX, 0x75),
        ("adc", Abs, 0x6d),
        ("adc", AbsX, 0x7d),
        ("adc", AbsY, 0x79),
        ("adc", IndX, 0x61),
        ("adc", IndY, 0x71),
        ("and", Imm, 0x29),
        ("and", Zp, 0x25),
        ("and", ZpX, 0x35),
        ("and", Abs, 0x2d),
        ("and", AbsX, 0x3d),
        ("and", AbsY, 0x39),
        ("and", IndX, 0x21),
        ("and", IndY, 0x31),
        ("asl", Acc, 0x0a),
        ("asl", Zp, 0x06),
        ("asl", ZpX, 0x16),
        ("asl", Abs, 0x0e),
        ("asl", AbsX, 0x1e),
        ("bcc", Rel, 0x90),
        ("bcs", Rel, 0xb0),
        ("beq", Rel, 0xf0),
        ("bmi", Rel, 0x30),
        ("bne", Rel, 0xd0),
        ("bpl", Rel, 0x10),
        ("bvc", Rel, 0x50),
        ("bvs", Rel, 0x70),
        ("bit", Zp, 0x24),
        ("bit", Abs, 0x2c),
        ("brk", Imp, 0x00),
        ("clc", Imp, 0x18),
        ("cld", Imp, 0xd8),
        ("cli", Imp, 0x58),
        ("clv", Imp, 0xb8),
        ("cmp", Imm, 0xc9),
        ("cmp", Zp, 0xc5),
        ("cmp", ZpX, 0xd5),
        ("cmp", Abs, 0xcd),
        ("cmp", AbsX, 0xdd),
        ("cmp", AbsY, 0xd9),
        ("cmp", IndX, 0xc1),
        ("cmp", IndY, 0xd1),
        ("cpx", Imm, 0xe0),
        ("cpx", Zp, 0xe4),
        ("cpx", Abs, 0xec),
        ("cpy", Imm, 0xc0),
        ("cpy", Zp, 0xc4),
        ("cpy", Abs, 0xcc),
        ("dec", Zp, 0xc6),
        ("dec", ZpX, 0xd6),
        ("dec", Abs, 0xce),
        ("dec", AbsX, 0xde),
        ("dex", Imp, 0xca),
        ("dey", Imp, 0x88),
        ("eor", Imm, 0x49),
        ("eor", Zp, 0x45),
        ("eor", ZpX, 0x55),
        ("eor", Abs, 0x4d),
        ("eor", AbsX, 0x5d),
        ("eor", AbsY, 0x59),
        ("eor", IndX, 0x41),
        ("eor", IndY, 0x51),
        ("inc", Zp, 0xe6),
        ("inc", ZpX, 0xf6),
        ("inc", Abs, 0xee),
        ("inc", AbsX, 0xfe),
        ("inx", Imp, 0xe8),
        ("iny", Imp, 0xc8),
        ("jmp", Abs, 0x4c),
        ("jmp", Ind, 0x6c),
        ("jsr", Abs, 0x20),
        ("lda", Imm, 0xa9),
        ("lda", Zp, 0xa5),
        ("lda", ZpX, 0xb5),
        ("lda", Abs, 0xad),
        ("lda", AbsX, 0xbd),
        ("lda", AbsY, 0xb9),
        ("lda", IndX, 0xa1),
        ("lda", IndY, 0xb1),
        ("ldx", Imm, 0xa2),
        ("ldx", Zp, 0xa6),
        ("ldx", ZpY, 0xb6),
        ("ldx", Abs, 0xae),
        ("ldx", AbsY, 0xbe),
        ("ldy", Imm, 0xa0),
        ("ldy", Zp, 0xa4),
        ("ldy", ZpX, 0xb4),
        ("ldy", Abs, 0xac),
        ("ldy", AbsX, 0xbc),
        ("lsr", Acc, 0x4a),
        ("lsr", Zp, 0x46),
        ("lsr", ZpX, 0x56),
        ("lsr", Abs, 0x4e),
        ("lsr", AbsX, 0x5e),
        ("nop", Imp, 0xea),
        ("ora", Imm, 0x09),
        ("ora", Zp, 0x05),
        ("ora", ZpX, 0x15),
        ("ora", Abs, 0x0d),
        ("ora", AbsX, 0x1d),
        ("ora", AbsY, 0x19),
        ("ora", IndX, 0x01),
        ("ora", IndY, 0x11),
        ("pha", Imp, 0x48),
        ("php", Imp, 0x08),
        ("pla", Imp, 0x68),
        ("plp", Imp, 0x28),
        ("rol", Acc, 0x2a),
        ("rol", Zp, 0x26),
        ("rol", ZpX, 0x36),
        ("rol", Abs, 0x2e),
        ("rol", AbsX, 0x3e),
        ("ror", Acc, 0x6a),
        ("ror", Zp, 0x66),
        ("ror", ZpX, 0x76),
        ("ror", Abs, 0x6e),
        ("ror", AbsX, 0x7e),
        ("rti", Imp, 0x40),
        ("rts", Imp, 0x60),
        ("sbc", Imm, 0xe9),
        ("sbc", Zp, 0xe5),
        ("sbc", ZpX, 0xf5),
        ("sbc", Abs, 0xed),
        ("sbc", AbsX, 0xfd),
        ("sbc", AbsY, 0xf9),
        ("sbc", IndX, 0xe1),
        ("sbc", IndY, 0xf1),
        ("sec", Imp, 0x38),
        ("sed", Imp, 0xf8),
        ("sei", Imp, 0x78),
        ("sta", Zp, 0x85),
        ("sta", ZpX, 0x95),
        ("sta", Abs, 0x8d),
        ("sta", AbsX, 0x9d),
        ("sta", AbsY, 0x99),
        ("sta", IndX, 0x81),
        ("sta", IndY, 0x91),
        ("stx", Zp, 0x86),
        ("stx", ZpY, 0x96),
        ("stx", Abs, 0x8e),
        ("sty", Zp, 0x84),
        ("sty", ZpX, 0x94),
        ("sty", Abs, 0x8c),
        ("tax", Imp, 0xaa),
        ("tay", Imp, 0xa8),
        ("tsx", Imp, 0xba),
        ("txa", Imp, 0x8a),
        ("txs", Imp, 0x9a),
        ("tya", Imp, 0x98),
    ]
}
fn branch(op: &str) -> bool {
    ["bcc", "bcs", "beq", "bmi", "bne", "bpl", "bvc", "bvs"].contains(&op)
}
fn inverse(op: &str) -> &str {
    match op {
        "bcc" => "bcs",
        "bcs" => "bcc",
        "beq" => "bne",
        "bne" => "beq",
        "bmi" => "bpl",
        "bpl" => "bmi",
        "bvc" => "bvs",
        "bvs" => "bvc",
        _ => unreachable!(),
    }
}
fn literal(s: &str) -> Option<i64> {
    let s = s.trim().replace('_', "");
    if let Some(v) = s.strip_prefix('$').or_else(|| s.strip_prefix("0x")) {
        i64::from_str_radix(v, 16).ok()
    } else if let Some(v) = s.strip_prefix('%').or_else(|| s.strip_prefix("0b")) {
        i64::from_str_radix(v, 2).ok()
    } else {
        s.parse().ok()
    }
}
pub fn value(s: &str, symbols: &BTreeMap<String, u16>) -> Result<i64, String> {
    let s = s.trim();
    if let Some(v) = s.strip_prefix('<') {
        return Ok(value(v, symbols)? & 255);
    }
    if let Some(v) = s.strip_prefix('>') {
        return Ok((value(v, symbols)? >> 8) & 255);
    }
    for (p, c) in s.char_indices().rev() {
        if p > 0 && (c == '+' || c == '-') {
            let a = value(&s[..p], symbols)?;
            let b = value(&s[p + 1..], symbols)?;
            return Ok(if c == '+' { a + b } else { a - b });
        }
    }
    literal(s)
        .or_else(|| symbols.get(s).map(|v| *v as i64))
        .ok_or_else(|| format!("Unknown assembly value '{s}'"))
}
fn mode(op: &str, arg: &str, symbols: &BTreeMap<String, u16>) -> Mode {
    use Mode::*;
    let s = arg.trim();
    if branch(op) {
        return Rel;
    }
    if s.is_empty() {
        return Imp;
    }
    if s.eq_ignore_ascii_case("a") {
        return Acc;
    }
    if s.starts_with('#') {
        return Imm;
    }
    let l = s.to_ascii_lowercase().replace(' ', "");
    if l.starts_with('(') {
        if l.ends_with(",x)") {
            return IndX;
        }
        if l.ends_with("),y") {
            return IndY;
        }
        return Ind;
    }
    let (base, suffix) = if let Some((b, i)) = s.rsplit_once(',') {
        (b.trim(), i.trim().to_ascii_lowercase())
    } else {
        (s, String::new())
    };
    let zp = value(base, symbols).is_ok_and(|n| (0..256).contains(&n));
    match (zp, suffix.as_str()) {
        (true, "x") => ZpX,
        (true, "y") => ZpY,
        (false, "x") => AbsX,
        (false, "y") => AbsY,
        (true, _) => Zp,
        (false, _) => Abs,
    }
}
fn size(m: Mode) -> usize {
    use Mode::*;
    match m {
        Imp | Acc => 1,
        Imm | Zp | ZpX | ZpY | IndX | IndY | Rel => 2,
        _ => 3,
    }
}
fn operand(arg: &str) -> String {
    let s = arg.trim().replace(' ', "");
    if let Some(s) = s.strip_prefix('#') {
        return s.into();
    }
    if s.starts_with('(') {
        let s = s.trim_start_matches('(');
        if s.to_ascii_lowercase().ends_with(",x)") {
            return s[..s.len() - 3].into();
        }
        if s.to_ascii_lowercase().ends_with("),y") {
            return s[..s.len() - 3].into();
        }
        return s.trim_end_matches(')').into();
    }
    s.split(',').next().unwrap_or("").into()
}
fn cycles(op: &str, m: Mode) -> (u8, u8) {
    use Mode::*;
    if branch(op) {
        return (2, 4);
    }
    match op {
        "brk" => return (7, 7),
        "jsr" => return (6, 6),
        "rts" | "rti" => return (6, 6),
        "pha" | "php" => return (3, 3),
        "pla" | "plp" => return (4, 4),
        "jmp" => return if m == Ind { (5, 5) } else { (3, 3) },
        _ => {}
    }
    let rmw = ["asl", "lsr", "rol", "ror", "inc", "dec"].contains(&op);
    let store = ["sta", "stx", "sty"].contains(&op);
    let n = match m {
        Imp | Acc | Imm => 2,
        Zp => {
            if rmw {
                5
            } else {
                3
            }
        }
        ZpX | ZpY => {
            if rmw {
                6
            } else {
                4
            }
        }
        Abs => {
            if rmw {
                6
            } else {
                4
            }
        }
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
        n + u8::from(!store && !rmw && matches!(m, AbsX | AbsY | IndY)),
    )
}
fn lines(source: &str) -> Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for raw in source.lines() {
        let mut s = raw.trim();
        if let Some(v) = s.strip_prefix(";@src ") {
            let a = v
                .split(',')
                .map(str::trim)
                .map(str::parse)
                .collect::<Result<Vec<usize>, _>>()
                .map_err(|_| "Invalid source marker")?;
            if a.len() == 2 {
                out.push(Line {
                    item: Item::Source(a[0], a[1]),
                    text: raw.into(),
                    long: false,
                    mode: Mode::Imp,
                });
            }
            continue;
        }
        if s.starts_with(";@barrier") {
            out.push(Line {
                item: Item::Barrier,
                text: raw.into(),
                long: false,
                mode: Mode::Imp,
            });
            continue;
        }
        if let Some(p) = s.find(';') {
            s = s[..p].trim();
        }
        if let Some(p) = s.find("//") {
            s = s[..p].trim();
        }
        if s.is_empty() {
            continue;
        }
        if let Some((l, r)) = s.split_once(':') {
            out.push(Line {
                item: Item::Label(l.trim().into()),
                text: format!("{}:", l.trim()),
                long: false,
                mode: Mode::Imp,
            });
            s = r.trim();
            if s.is_empty() {
                continue;
            }
        }
        let item = if let Some((name, val)) = s.split_once('=') {
            Item::Equ(name.trim().into(), val.trim().into())
        } else {
            let mut split = s.splitn(2, char::is_whitespace);
            let op = split.next().unwrap().to_ascii_lowercase();
            let arg = split.next().unwrap_or("").trim();
            match op.as_str() {
                ".byte" | ".word" => Item::Data(
                    arg.split(',').map(|s| s.trim().into()).collect(),
                    op == ".word",
                ),
                ".res" => Item::Reserve(
                    literal(arg)
                        .filter(|n| (0..=65536).contains(n))
                        .ok_or("Invalid .res size")? as usize,
                ),
                ".align" => Item::Align(
                    literal(arg)
                        .filter(|n| *n > 0 && *n <= 256 && (*n as u64).is_power_of_two())
                        .ok_or("Alignment must be a power of two up to 256")?
                        as usize,
                ),
                _ => Item::Op(op, arg.into()),
            }
        };
        out.push(Line {
            item,
            text: s.into(),
            long: false,
            mode: Mode::Imp,
        });
    }
    Ok(out)
}
pub fn assemble(source: &str, base: u16) -> Result<Assembly, String> {
    let codes = opcodes();
    let mut lines = lines(source)?;
    let mut symbols = BTreeMap::new();
    for _ in 0..16 {
        let mut changed = false;
        for l in &lines {
            if let Item::Equ(name, expr) = &l.item {
                if let Ok(v) = value(expr, &symbols) {
                    if !(0..=65535).contains(&v) {
                        return Err(format!("Assembly constant {name} exceeds 16 bits"));
                    }
                    if symbols.insert(name.clone(), v as u16) != Some(v as u16) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    for _ in 0..64 {
        let mut pc = base as usize;
        let mut labels = BTreeMap::new();
        let equ_names = lines
            .iter()
            .filter_map(|l| {
                if let Item::Equ(n, _) = &l.item {
                    Some(n.clone())
                } else {
                    None
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        for l in &mut lines {
            match &l.item {
                Item::Label(n) => {
                    if labels.insert(n.clone(), pc as u16).is_some() || equ_names.contains(n) {
                        return Err(format!("Duplicate assembly symbol '{n}'"));
                    }
                }
                Item::Op(op, arg) => {
                    l.mode = mode(op, arg, &symbols);
                    if codes.iter().all(|(n, m, _)| *n != op || *m != l.mode) {
                        if l.mode == Mode::ZpY
                            && codes.iter().any(|(n, m, _)| *n == op && *m == Mode::AbsY)
                        {
                            l.mode = Mode::AbsY;
                        } else if l.mode == Mode::ZpX
                            && codes.iter().any(|(n, m, _)| *n == op && *m == Mode::AbsX)
                        {
                            l.mode = Mode::AbsX;
                        } else {
                            return Err(format!(
                                "Unsupported instruction/addressing mode: {}",
                                l.text
                            ));
                        }
                    }
                    pc += if l.long { 5 } else { size(l.mode) };
                }
                Item::Data(v, w) => pc += v.len() * if *w { 2 } else { 1 },
                Item::Reserve(n) => pc += n,
                Item::Align(n) => pc = (pc + n - 1) & !(n - 1),
                _ => {}
            }
            if pc > 65536 {
                return Err("Assembly exceeds CPU address space".into());
            }
        }
        for (n, v) in labels {
            symbols.insert(n, v);
        }
        let mut pc = base as usize;
        let mut changed = false;
        for l in &mut lines {
            match &l.item {
                Item::Op(op, arg) => {
                    if branch(op) && !l.long {
                        let target = value(arg, &symbols)?;
                        let offset = target - (pc + 2) as i64;
                        if !(-128..=127).contains(&offset) {
                            l.long = true;
                            changed = true;
                        }
                    }
                    pc += if l.long { 5 } else { size(l.mode) };
                }
                Item::Data(v, w) => pc += v.len() * if *w { 2 } else { 1 },
                Item::Reserve(n) => pc += n,
                Item::Align(n) => pc = (pc + n - 1) & !(n - 1),
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    let mut bytes = Vec::new();
    let mut listing = String::new();
    let mut instructions = Vec::new();
    let mut src = None;
    let mut relaxed = 0;
    for l in lines {
        let pc = base as usize + bytes.len();
        match l.item {
            Item::Source(a, b) => src = Some((a, b)),
            Item::Label(n) => listing.push_str(&format!("\n{pc:04X}            {n}:\n")),
            Item::Op(op, arg) => {
                let opcode = |op: &str, m: Mode| {
                    codes
                        .iter()
                        .find(|(n, mode, _)| *n == op && *mode == m)
                        .map(|v| v.2)
                        .unwrap()
                };
                let mut code = vec![opcode(&op, l.mode)];
                if l.long {
                    let n = value(&arg, &symbols)?;
                    code = vec![
                        opcode(inverse(&op), Mode::Rel),
                        3,
                        0x4c,
                        n as u8,
                        (n >> 8) as u8,
                    ];
                    relaxed += 1;
                } else if l.mode == Mode::Rel {
                    let n = value(&arg, &symbols)? - (pc + 2) as i64;
                    if !(-128..=127).contains(&n) {
                        return Err("Branch relaxation did not converge".into());
                    }
                    code.push(n as u8);
                } else if size(l.mode) > 1 {
                    let n = value(&operand(&arg), &symbols)?;
                    if l.mode == Mode::Imm {
                        if !(-128..=255).contains(&n) {
                            return Err(format!("Immediate operand exceeds byte: {}", l.text));
                        }
                    } else if !(0..=65535).contains(&n) {
                        return Err(format!("Address exceeds 16 bits: {}", l.text));
                    }
                    if l.mode == Mode::Ind && n & 255 == 255 {
                        return Err("Indirect JMP crosses a page boundary (6502 hardware bug); choose another pointer".into());
                    }
                    code.push(n as u8);
                    if size(l.mode) == 3 {
                        code.push((n >> 8) as u8);
                    }
                }
                let (min, max) = if l.long { (3, 5) } else { cycles(&op, l.mode) };
                listing.push_str(&format!(
                    "{pc:04X}  {:<14} {:<28} ; {min}{} cycles\n",
                    code.iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    l.text,
                    if min == max {
                        String::new()
                    } else {
                        format!("..{max}")
                    }
                ));
                instructions.push(Instruction {
                    address: pc as u16,
                    bytes: code.clone(),
                    text: l.text,
                    cycles_min: min,
                    cycles_max: max,
                    source: src,
                });
                bytes.extend(code);
            }
            Item::Data(v, word) => {
                let start = bytes.len();
                for s in v {
                    let n = value(&s, &symbols)?;
                    if !(if word { -32768..=65535 } else { -128..=255 }).contains(&n) {
                        return Err(format!("Data value out of range: {s}"));
                    }
                    bytes.push(n as u8);
                    if word {
                        bytes.push((n >> 8) as u8);
                    }
                }
                listing.push_str(&format!(
                    "{pc:04X}  {:<14} .data ({} bytes)\n",
                    bytes[start..]
                        .iter()
                        .take(4)
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                    bytes.len() - start
                ));
            }
            Item::Reserve(n) => bytes.resize(bytes.len() + n, 0),
            Item::Align(n) => {
                let count = ((pc + n - 1) & !(n - 1)) - pc;
                bytes.resize(bytes.len() + count, 0);
            }
            _ => {}
        }
    }
    Ok(Assembly {
        bytes,
        symbols,
        listing,
        instructions,
        relaxed,
    })
}

/// Local machine transforms preserve IO bus accesses and opaque asm blocks.
/// Labels/calls invalidate value knowledge; dead flag-setting loads are removed
/// only when a later instruction kills NZ before any control-flow observation.
pub fn optimize_machine(source: &str) -> (String, usize) {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let mut changes = 0;
    for _ in 0..4 {
        let mut keep = vec![true; lines.len()];
        let mut opaque = false;
        for i in 0..lines.len() {
            let a = lines[i].trim();
            if a == ";@barrier begin" {
                opaque = true;
                continue;
            }
            if a == ";@barrier end" {
                opaque = false;
                continue;
            }
            if opaque || a.is_empty() || a.starts_with(';') || a.ends_with(':') {
                continue;
            }
            let next = lines.get(i + 1).map(|s| s.trim()).unwrap_or("");
            if a.starts_with("jmp ") && next == format!("{}:", &a[4..]) {
                keep[i] = false;
                changes += 1;
                continue;
            }
            if (a == "rts" || a.starts_with("jmp "))
                && !next.is_empty()
                && !next.ends_with(':')
                && !next.starts_with(';')
                && !next.contains('=')
                && !next.starts_with('.')
            {
                keep[i + 1] = false;
                changes += 1;
                continue;
            }
            if (a.starts_with("ldx #") || a.starts_with("ldy #")) && next.starts_with(&a[..4]) {
                keep[i] = false;
                changes += 1;
                continue;
            }
            if a.starts_with("sta ") && next == format!("lda {}", &a[4..]) {
                let mut flags_dead = false;
                for q in lines.iter().skip(i + 2).take(8) {
                    let q = q.trim();
                    if q.starts_with(';') {
                        continue;
                    }
                    let op = q.split_whitespace().next().unwrap_or("");
                    if q.ends_with(':')
                        || branch(op)
                        || ["php", "jsr", "rts", "jmp", "adc", "sbc", "rol", "ror"].contains(&op)
                    {
                        break;
                    }
                    if [
                        "lda", "ldx", "ldy", "cmp", "cpx", "cpy", "and", "ora", "eor", "tax",
                        "tay", "txa", "tya", "pla", "asl", "lsr", "inc", "dec", "inx", "iny",
                        "dex", "dey",
                    ]
                    .contains(&op)
                    {
                        flags_dead = true;
                        break;
                    }
                }
                // Only ordinary compiler-managed RAM: volatile CPU addresses stay.
                if flags_dead && a[4..].starts_with("__v_") {
                    keep[i + 1] = false;
                    changes += 1;
                }
            }
        }
        // Branch to the adjacent true block becomes a fall-through with inverse branch.
        let mut opaque = false;
        for i in 0..lines.len().saturating_sub(2) {
            if lines[i].trim() == ";@barrier begin" {
                opaque = true;
                continue;
            }
            if lines[i].trim() == ";@barrier end" {
                opaque = false;
                continue;
            }
            if opaque {
                continue;
            }
            if !keep[i] || !keep[i + 1] || !keep[i + 2] {
                continue;
            }
            let a = lines[i].trim();
            let b = lines[i + 1].trim();
            let c = lines[i + 2].trim();
            if let Some((op, target)) = a.split_once(' ') {
                if branch(op) && b.starts_with("jmp ") && c == format!("{target}:") {
                    let inverse = match op {
                        "beq" => "bne",
                        "bne" => "beq",
                        "bcc" => "bcs",
                        "bcs" => "bcc",
                        "bmi" => "bpl",
                        "bpl" => "bmi",
                        "bvc" => "bvs",
                        _ => "bvc",
                    };
                    lines[i] = format!("  {inverse} {}", &b[4..]);
                    keep[i + 1] = false;
                    changes += 1;
                }
            }
        }
        let old = lines.len();
        lines = lines
            .into_iter()
            .zip(keep)
            .filter_map(|(s, k)| k.then_some(s))
            .collect();
        if old == lines.len() {
            break;
        }
    }
    (lines.join("\n") + "\n", changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_official_opcodes_unique() {
        let ops = opcodes();
        assert_eq!(ops.len(), 151);
        let mut values = ops.iter().map(|o| o.2).collect::<Vec<_>>();
        values.sort();
        values.dedup();
        assert_eq!(values.len(), 151);
    }
    #[test]
    fn addressing_and_relocation() {
        let a = assemble(
            "p = $12\nstart:\n lda #$80\n sta p\n lda (p),y\n jmp start\n .word start\n",
            0x8000,
        )
        .unwrap();
        assert_eq!(
            a.bytes,
            vec![0xa9, 0x80, 0x85, 0x12, 0xb1, 0x12, 0x4c, 0, 0x80, 0, 0x80]
        );
    }
    #[test]
    fn relaxes_forward_and_backward() {
        let a = assemble("start:\n beq end\n .res 200\nend:\n bne start\n", 0xc000).unwrap();
        assert_eq!(a.relaxed, 2);
        assert_eq!(&a.bytes[..3], &[0xd0, 3, 0x4c]);
        assert_eq!(a.symbols["end"], 0xc0cd);
    }
    #[test]
    fn preserves_opaque_assembly() {
        let s = ";@barrier begin\nldx #1\nldx #2\n;@barrier end\n";
        assert_eq!(optimize_machine(s).0, s);
    }
    #[test]
    fn rejects_page_bug() {
        assert!(assemble("jmp ($02ff)", 0x8000).is_err());
    }
}
