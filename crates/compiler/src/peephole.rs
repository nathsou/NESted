//! Machine-level optimization over generated 6502 assembly text.
//!
//! * Jump threading: jumps and branches to `jmp`/`rts` go straight through.
//! * Register tracking within straight-line code: loads of a value a register
//!   already holds are dropped (when the flags they set are not needed), as are
//!   stores of a value memory already holds, and `cmp #0` after an instruction
//!   whose flags already describe A.
//! * Dead stores: overwritten stores and stores to never-read temporaries.
//! * `jsr f` + `rts` becomes `jmp f`.
//!
//! Hardware addresses (`$xxxx` operands), shared bytes inside `;@barrier`
//! regions and inline assembly are never touched.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
enum Line {
    Instr { op: String, arg: String },
    Label(String),
    Other,
    Barrier(bool),
}

fn parse(line: &str) -> Line {
    let t = line.trim();
    if t == ";@barrier begin" {
        return Line::Barrier(true);
    }
    if t == ";@barrier end" {
        return Line::Barrier(false);
    }
    if t.is_empty() || t.starts_with(';') || t.starts_with('.') || t.contains('=') {
        return Line::Other;
    }
    if let Some(name) = t.strip_suffix(':') {
        if !name.contains(char::is_whitespace) {
            return Line::Label(name.to_owned());
        }
    }
    let code = t.split(';').next().unwrap_or("").trim();
    let mut parts = code.splitn(2, char::is_whitespace);
    let op = parts.next().unwrap_or("").to_ascii_lowercase();
    if op.len() != 3 || !op.chars().all(|c| c.is_ascii_alphabetic()) {
        return Line::Other;
    }
    let arg = canonical(parts.next().unwrap_or("").trim());
    Line::Instr { op, arg }
}

/// `label+0+2` and `label+2` name the same byte.
fn canonical(arg: &str) -> String {
    if !arg.starts_with("__") || arg.contains(',') {
        return arg.to_owned();
    }
    let mut parts = arg.split('+');
    let base = parts.next().unwrap_or("");
    let mut offset = 0i64;
    for part in parts {
        match part.trim().parse::<i64>() {
            Ok(n) => offset += n,
            Err(_) => return arg.to_owned(),
        }
    }
    if offset == 0 {
        base.to_owned()
    } else {
        format!("{base}+{offset}")
    }
}

fn is_branch(op: &str) -> bool {
    matches!(op, "bcc" | "bcs" | "beq" | "bne" | "bmi" | "bpl" | "bvc" | "bvs")
}

/// Compiler-owned memory whose value only changes through visible stores.
fn trackable(arg: &str) -> bool {
    arg.starts_with("__") && !arg.contains(',') && !arg.contains('(')
}

fn base(arg: &str) -> &str {
    let end = arg.find(['+', ',', ' ']).unwrap_or(arg.len());
    &arg[..end]
}

#[derive(Clone, Default)]
struct Regs {
    a: BTreeSet<String>,
    x: BTreeSet<String>,
    y: BTreeSet<String>,
    /// Which register N/Z currently describe.
    nz: Option<char>,
}

impl Regs {
    fn reg(&mut self, r: char) -> &mut BTreeSet<String> {
        match r {
            'a' => &mut self.a,
            'x' => &mut self.x,
            _ => &mut self.y,
        }
    }
    fn forget_memory(&mut self, pred: impl Fn(&str) -> bool) {
        for set in [&mut self.a, &mut self.x, &mut self.y] {
            set.retain(|v| v.starts_with('#') || !pred(v));
        }
    }
}

struct Program {
    lines: Vec<String>,
    parsed: Vec<Line>,
    labels: BTreeMap<String, usize>,
}

impl Program {
    fn new(lines: Vec<String>) -> Self {
        let parsed: Vec<Line> = lines.iter().map(|l| parse(l)).collect();
        let mut labels = BTreeMap::new();
        for (i, p) in parsed.iter().enumerate() {
            if let Line::Label(n) = p {
                labels.insert(n.clone(), i);
            }
        }
        Self {
            lines,
            parsed,
            labels,
        }
    }
    /// For each label: the indices of the plain jumps/branches that target it,
    /// and whether anything else (calls, data, assembly blocks) refers to it.
    fn references(&self) -> (BTreeMap<String, Vec<usize>>, BTreeSet<String>) {
        let mut jumps: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut other = BTreeSet::new();
        let mut opaque = false;
        for (i, line) in self.parsed.iter().enumerate() {
            match line {
                Line::Barrier(b) => opaque = *b,
                Line::Label(_) => {}
                Line::Instr { op, arg }
                    if !opaque && (op == "jmp" || is_branch(op)) && self.labels.contains_key(arg) =>
                {
                    jumps.entry(arg.clone()).or_default().push(i);
                }
                _ => {
                    for word in self.lines[i]
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    {
                        if self.labels.contains_key(word) {
                            other.insert(word.to_owned());
                        }
                    }
                }
            }
        }
        (jumps, other)
    }
    /// The first instruction at or after `i`, skipping labels and comments.
    fn next_instr(&self, mut i: usize) -> Option<usize> {
        while i < self.parsed.len() {
            match &self.parsed[i] {
                Line::Instr { .. } => return Some(i),
                Line::Label(_) | Line::Other => i += 1,
                Line::Barrier(_) => return None,
            }
        }
        None
    }
    /// Does any path from `i` read the given flags before they are rewritten?
    /// `carry`: track C; otherwise N/Z.
    fn reads_flag(&self, i: usize, carry: bool, budget: &mut usize, seen: &mut BTreeSet<usize>) -> bool {
        let mut i = i;
        loop {
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            if !seen.insert(i) {
                return false;
            }
            let Some(j) = self.next_instr(i) else {
                return true;
            };
            let Line::Instr { op, arg } = &self.parsed[j] else {
                return true;
            };
            let op = op.as_str();
            let (reads, writes) = if carry {
                (
                    matches!(op, "bcc" | "bcs" | "adc" | "sbc" | "rol" | "ror" | "php"),
                    matches!(op, "clc" | "sec" | "cmp" | "cpx" | "cpy" | "asl" | "lsr" | "plp"),
                )
            } else {
                (
                    matches!(op, "beq" | "bne" | "bmi" | "bpl" | "php"),
                    matches!(
                        op,
                        "lda" | "ldx" | "ldy" | "and" | "ora" | "eor" | "adc" | "sbc" | "cmp"
                            | "cpx" | "cpy" | "bit" | "inc" | "dec" | "inx" | "iny" | "dex"
                            | "dey" | "tax" | "tay" | "txa" | "tya" | "tsx" | "pla" | "asl"
                            | "lsr" | "rol" | "ror" | "plp"
                    ),
                )
            };
            if reads {
                return true;
            }
            if writes {
                return false;
            }
            match op {
                "rts" | "rti" | "jsr" => return false,
                "jmp" => {
                    let Some(&t) = self.labels.get(arg.as_str()) else {
                        return true;
                    };
                    i = t;
                }
                _ if is_branch(op) => {
                    let Some(&t) = self.labels.get(arg.as_str()) else {
                        return true;
                    };
                    if self.reads_flag(t, carry, budget, seen) {
                        return true;
                    }
                    i = j + 1;
                }
                _ => i = j + 1,
            }
        }
    }
    fn flag_read(&self, i: usize, carry: bool) -> bool {
        self.reads_flag(i, carry, &mut 48, &mut BTreeSet::new())
    }
}

/// Follows a label to the final target of a `jmp` chain; `rts` if it returns.
fn thread(p: &Program, label: &str) -> Option<String> {
    let mut target = label.to_owned();
    for _ in 0..8 {
        let &i = p.labels.get(&target)?;
        let j = p.next_instr(i)?;
        match &p.parsed[j] {
            Line::Instr { op, arg } if op == "jmp" && !arg.starts_with('(') => {
                if *arg == target {
                    return None;
                }
                target = arg.clone();
            }
            Line::Instr { op, .. } if op == "rts" => return Some("rts".into()),
            _ => break,
        }
    }
    (target != label).then_some(target)
}

pub fn optimize(source: &str) -> (String, usize) {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let mut changes = 0;
    for _ in 0..6 {
        let before = changes;
        lines = jumps(lines, &mut changes);
        lines = registers(lines, &mut changes);
        lines = dead_stores(lines, &mut changes);
        if changes == before {
            break;
        }
    }
    (lines.join("\n") + "\n", changes)
}

fn jumps(lines: Vec<String>, changes: &mut usize) -> Vec<String> {
    let p = Program::new(lines);
    let mut out = p.lines.clone();
    let mut opaque = false;
    for (i, line) in p.parsed.iter().enumerate() {
        match line {
            Line::Barrier(b) => opaque = *b,
            Line::Instr { op, arg } if !opaque => {
                if op == "jmp" || is_branch(op) {
                    if let Some(t) = thread(&p, arg) {
                        if t == "rts" {
                            if op == "jmp" {
                                out[i] = "  rts".into();
                                *changes += 1;
                            }
                        } else {
                            out[i] = format!("  {op} {t}");
                            *changes += 1;
                        }
                    }
                }
                if op == "jsr" {
                    if let Some(j) = p.next_instr(i + 1) {
                        // Only when nothing jumps between them.
                        let direct = (i + 1..j).all(|k| !matches!(p.parsed[k], Line::Label(_)));
                        if direct && matches!(&p.parsed[j], Line::Instr { op, .. } if op == "rts") {
                            out[i] = format!("  jmp {arg}");
                            *changes += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // `bxx L1; [unreferenced labels]; jmp L2; [labels incl. L1]`
    // becomes `b!xx L2`.
    let p = Program::new(out);
    let (refs, other) = p.references();
    let referenced = |l: &str| refs.contains_key(l) || other.contains(l);
    let mut out = p.lines.clone();
    let mut skip = vec![false; out.len()];
    let mut opaque = false;
    for (i, line) in p.parsed.iter().enumerate() {
        match line {
            Line::Barrier(b) => opaque = *b,
            Line::Instr { op, arg } if !opaque && is_branch(op) && !skip[i] => {
                let mut j = i + 1;
                let mut clean = true;
                while j < p.parsed.len() {
                    match &p.parsed[j] {
                        Line::Label(l) if !referenced(l) => j += 1,
                        Line::Other if p.lines[j].trim().is_empty() || p.lines[j].trim().starts_with(';') => j += 1,
                        _ => break,
                    }
                }
                let Some(Line::Instr { op: jop, arg: target }) = p.parsed.get(j) else {
                    continue;
                };
                if jop != "jmp" || target.starts_with('(') || skip[j] {
                    continue;
                }
                let mut k = j + 1;
                let mut found = false;
                while k < p.parsed.len() {
                    match &p.parsed[k] {
                        Line::Label(l) => {
                            if l == arg {
                                found = true;
                            }
                            k += 1;
                        }
                        Line::Other if p.lines[k].trim().is_empty() || p.lines[k].trim().starts_with(';') => k += 1,
                        _ => break,
                    }
                }
                if !found {
                    clean = false;
                }
                if clean {
                    let inverse = match op.as_str() {
                        "beq" => "bne",
                        "bne" => "beq",
                        "bcc" => "bcs",
                        "bcs" => "bcc",
                        "bmi" => "bpl",
                        "bpl" => "bmi",
                        "bvc" => "bvs",
                        _ => "bvc",
                    };
                    out[i] = format!("  {inverse} {target}");
                    skip[j] = true;
                    *changes += 1;
                }
            }
            _ => {}
        }
    }
    let out: Vec<String> = out
        .into_iter()
        .zip(skip)
        .filter_map(|(l, s)| (!s).then_some(l))
        .collect();
    // A jump to a label that follows it, with only labels in between.
    let p = Program::new(out);
    let mut keep = vec![true; p.lines.len()];
    let mut opaque = false;
    for (i, line) in p.parsed.iter().enumerate() {
        match line {
            Line::Barrier(b) => opaque = *b,
            Line::Instr { op, arg } if !opaque && op == "jmp" => {
                let mut k = i + 1;
                while k < p.parsed.len() {
                    match &p.parsed[k] {
                        Line::Label(l) if l == arg => {
                            keep[i] = false;
                            *changes += 1;
                            break;
                        }
                        Line::Label(_) | Line::Other => k += 1,
                        _ => break,
                    }
                }
            }
            _ => {}
        }
    }
    p.lines
        .into_iter()
        .zip(keep)
        .filter_map(|(l, k)| k.then_some(l))
        .collect()
}

fn meet(a: Option<Regs>, b: Option<Regs>) -> Option<Regs> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some(Regs {
            a: a.a.intersection(&b.a).cloned().collect(),
            x: a.x.intersection(&b.x).cloned().collect(),
            y: a.y.intersection(&b.y).cloned().collect(),
            nz: if a.nz == b.nz { a.nz } else { None },
        }),
    }
}

fn registers(lines: Vec<String>, changes: &mut usize) -> Vec<String> {
    let p = Program::new(lines);
    let (refs, other) = p.references();
    // Labels entered by a later (backward) jump or by anything we cannot
    // see start with no knowledge; forward joins meet their predecessors.
    let mut unknown_entry = other.clone();
    for (label, sources) in &refs {
        let at = p.labels[label];
        if sources.iter().any(|&j| j >= at) {
            unknown_entry.insert(label.clone());
        }
    }
    let mut pending: BTreeMap<String, Option<Regs>> = BTreeMap::new();
    let mut keep = vec![true; p.lines.len()];
    let mut out_text: Vec<Option<String>> = vec![None; p.lines.len()];
    // None: unreachable by fall-through.
    let mut cur: Option<Regs> = Some(Regs::default());
    let mut opaque = false;
    for (i, line) in p.parsed.iter().enumerate() {
        match line {
            Line::Barrier(b) => {
                opaque = *b;
                cur = Some(Regs::default());
                continue;
            }
            Line::Label(l) => {
                cur = if unknown_entry.contains(l) {
                    Some(Regs::default())
                } else {
                    match meet(cur.take(), pending.remove(l).flatten()) {
                        Some(r) => Some(r),
                        None => Some(Regs::default()),
                    }
                };
                continue;
            }
            Line::Other => continue,
            Line::Instr { .. } if opaque => continue,
            Line::Instr { op, arg } => {
                let Some(r) = cur.as_mut() else {
                    continue;
                };
                let op = op.as_str();
                let value = if arg.starts_with('#') || trackable(arg) {
                    Some(arg.clone())
                } else {
                    None
                };
                match op {
                    "lda" | "ldx" | "ldy" => {
                        let reg = op.chars().nth(2).unwrap();
                        // A value just stored from A: transfer instead of load.
                        if reg != 'a' && trackable(arg) && r.a.contains(arg) && !r.reg(reg).contains(arg) {
                            out_text[i] = Some(format!("  ta{reg}"));
                            *changes += 1;
                            let a = r.a.clone();
                            *r.reg(reg) = a;
                            r.nz = Some(reg);
                            continue;
                        }
                        if let Some(v) = &value {
                            let holds = r.reg(reg).contains(v);
                            if holds && (r.nz == Some(reg) || !p.flag_read(i + 1, false)) {
                                // Flags keep describing whatever they did.
                                keep[i] = false;
                                *changes += 1;
                                continue;
                            }
                        }
                        let set = r.reg(reg);
                        set.clear();
                        if let Some(v) = value {
                            set.insert(v);
                        }
                        r.nz = Some(reg);
                    }
                    "sta" | "stx" | "sty" => {
                        let reg = op.chars().nth(2).unwrap();
                        if trackable(arg) {
                            if r.reg(reg).contains(arg) {
                                keep[i] = false;
                                *changes += 1;
                                continue;
                            }
                            let a = arg.clone();
                            r.forget_memory(|v| v == a);
                            r.reg(reg).insert(arg.clone());
                        } else if arg.contains('(') {
                            r.forget_memory(|_| true);
                        } else {
                            let b = base(arg).to_owned();
                            r.forget_memory(|v| base(v) == b);
                        }
                    }
                    "cmp" if arg == "#0" && r.nz == Some('a') && !p.flag_read(i + 1, true) => {
                        keep[i] = false;
                        *changes += 1;
                    }
                    "cmp" | "cpx" | "cpy" | "bit" => r.nz = None,
                    "tax" | "tay" => {
                        let reg = op.chars().nth(2).unwrap();
                        let a = r.a.clone();
                        *r.reg(reg) = a;
                        r.nz = Some(reg);
                    }
                    "txa" | "tya" => {
                        let reg = op.chars().nth(1).unwrap();
                        let v = r.reg(reg).clone();
                        r.a = v;
                        r.nz = Some('a');
                    }
                    "inx" | "dex" | "iny" | "dey" => {
                        let reg = op.chars().nth(2).unwrap();
                        r.reg(reg).clear();
                        r.nz = Some(reg);
                    }
                    "inc" | "dec" | "asl" | "lsr" | "rol" | "ror" => {
                        if arg.is_empty() || arg == "a" {
                            r.a.clear();
                            r.nz = Some('a');
                        } else {
                            let b = base(arg).to_owned();
                            r.forget_memory(|v| base(v) == b);
                            r.nz = None;
                        }
                    }
                    "adc" | "sbc" | "and" | "ora" | "eor" | "pla" => {
                        r.a.clear();
                        r.nz = Some('a');
                    }
                    "clc" | "sec" | "clv" | "cld" | "sed" | "cli" | "sei" | "nop" | "pha"
                    | "php" => {}
                    "plp" => r.nz = None,
                    _ if is_branch(op) => {
                        if p.labels.contains_key(arg) {
                            let state = cur.clone();
                            let entry = pending.entry(arg.clone()).or_insert(None);
                            *entry = match entry.take() {
                                None => state,
                                Some(e) => meet(Some(e), state),
                            };
                        }
                    }
                    "jmp" => {
                        if p.labels.contains_key(arg) {
                            let state = cur.take();
                            let entry = pending.entry(arg.clone()).or_insert(None);
                            *entry = match entry.take() {
                                None => state,
                                Some(e) => meet(Some(e), state),
                            };
                        }
                        cur = None;
                    }
                    "rts" | "rti" | "brk" => cur = None,
                    _ => {
                        // jsr, txs, tsx: unknown state.
                        cur = Some(Regs::default());
                    }
                }
            }
        }
    }
    p.lines
        .into_iter()
        .zip(keep)
        .zip(out_text)
        .filter_map(|((l, k), t)| k.then(|| t.unwrap_or(l)))
        .collect()
}

/// Removes stores whose value is overwritten before any read on every path
/// within straight-line code, and stores to compiler temporaries or locals
/// that nothing ever reads.
fn dead_stores(lines: Vec<String>, changes: &mut usize) -> Vec<String> {
    let p = Program::new(lines);
    // Memory read anywhere (any operand spelling that is not a plain store).
    let mut read: BTreeSet<String> = BTreeSet::new();
    for (i, line) in p.parsed.iter().enumerate() {
        match line {
            Line::Instr { op, arg } => {
                if !matches!(op.as_str(), "sta" | "stx" | "sty") || !trackable(arg) {
                    if arg.contains(',') || arg.contains('(') {
                        // Indexed access may touch any byte of the base.
                        read.insert(base(arg).trim_start_matches('(').to_owned());
                    }
                    read.insert(arg.clone());
                }
            }
            Line::Other => {
                // Data and equates may reference labels; inline assembly
                // inside barriers is parsed as instructions above.
                for word in p.lines[i].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                    if word.starts_with("__") {
                        read.insert(word.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    let mut keep = vec![true; p.lines.len()];
    let mut opaque = false;
    for (i, line) in p.parsed.iter().enumerate() {
        match line {
            Line::Barrier(b) => opaque = *b,
            Line::Instr { op, arg }
                if !opaque
                    && matches!(op.as_str(), "sta" | "stx" | "sty")
                    && trackable(arg)
                    && (arg.starts_with("__t_") || arg.starts_with("__l_"))
                    && !read.contains(arg)
                    && !read.contains(base(arg)) =>
            {
                keep[i] = false;
                *changes += 1;
            }
            _ => {}
        }
    }
    p.lines
        .into_iter()
        .zip(keep)
        .filter_map(|(l, k)| k.then_some(l))
        .collect()
}
