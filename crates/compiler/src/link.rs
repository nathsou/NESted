//! Cartridge boards, memory allocation, static frame overlays, bank
//! placement, runtime glue and iNES output.
use crate::asm::{self, Arg, Ins, Line, Op, Opnd, Sym};
use crate::codegen::{fn_label, helper_label, Banking};
use crate::hir::{Cartridge, Context, Placement};
use crate::mir::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Nrom,
    Cnrom,
    Uxrom,
    Axrom,
    Bnrom,
    Gxrom,
    ColorDreams,
    Mmc1,
    Mmc3,
}

impl Board {
    pub const NAMES: &'static [&'static str] = &[
        "nrom", "cnrom", "uxrom", "axrom", "bnrom", "gxrom", "colordreams", "mmc1", "mmc3",
    ];
    pub fn from_name(n: &str) -> Option<Board> {
        Some(match n {
            "nrom" => Board::Nrom,
            "cnrom" => Board::Cnrom,
            "uxrom" | "unrom" => Board::Uxrom,
            "axrom" | "anrom" => Board::Axrom,
            "bnrom" => Board::Bnrom,
            "gxrom" | "gnrom" => Board::Gxrom,
            "colordreams" => Board::ColorDreams,
            "mmc1" | "sxrom" => Board::Mmc1,
            "mmc3" | "txrom" => Board::Mmc3,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Board::Nrom => "NROM",
            Board::Cnrom => "CNROM",
            Board::Uxrom => "UxROM",
            Board::Axrom => "AxROM",
            Board::Bnrom => "BNROM",
            Board::Gxrom => "GxROM",
            Board::ColorDreams => "Color Dreams",
            Board::Mmc1 => "MMC1",
            Board::Mmc3 => "MMC3",
        }
    }
    pub fn ines(self) -> u16 {
        match self {
            Board::Nrom => 0,
            Board::Mmc1 => 1,
            Board::Uxrom => 2,
            Board::Cnrom => 3,
            Board::Mmc3 => 4,
            Board::Axrom => 7,
            Board::ColorDreams => 11,
            Board::Bnrom => 34,
            Board::Gxrom => 66,
        }
    }
    /// Size of one switchable PRG bank (and of the whole image for fixed boards).
    pub fn bank_size(self) -> usize {
        match self {
            Board::Uxrom | Board::Mmc1 => 0x4000,
            Board::Mmc3 => 0x2000,
            _ => 0x8000,
        }
    }
    /// Boards whose banks replace all of $8000-$FFFF.
    pub fn whole(self) -> bool {
        matches!(self, Board::Axrom | Board::Bnrom | Board::Gxrom | Board::ColorDreams)
    }
    pub fn banking(self) -> Banking {
        match self {
            Board::Nrom | Board::Cnrom => Banking::None,
            Board::Mmc3 => Banking::Split,
            _ => Banking::Shared,
        }
    }
    pub fn max_prg_banks(self) -> usize {
        match self {
            Board::Nrom | Board::Cnrom => 1,
            Board::Uxrom | Board::Mmc1 => 16,
            Board::Mmc3 => 64,
            Board::Axrom | Board::Bnrom => 8,
            Board::Gxrom | Board::ColorDreams => 4,
        }
    }
    pub fn chr_rom(self) -> bool {
        !matches!(self, Board::Uxrom | Board::Axrom | Board::Bnrom)
    }
    pub fn max_chr(self) -> usize {
        match self {
            Board::Nrom => 0x2000,
            Board::Cnrom => 0x8000,
            Board::Gxrom => 0x8000,
            Board::ColorDreams => 0x20000,
            Board::Mmc1 => 0x20000,
            Board::Mmc3 => 0x40000,
            _ => 0x2000,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Symbol {
    pub name: String,
    pub address: u16,
    pub size: usize,
    pub kind: String,
    pub bank: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Linked {
    pub rom: Vec<u8>,
    pub symbols: Vec<Symbol>,
    pub labels: BTreeMap<String, u16>,
    pub listing: Vec<(Option<u32>, asm::ListItem)>,
    pub layout: String,
    pub prg_bytes: usize,
    pub chr_bytes: usize,
    pub fixed_used: usize,
    pub fixed_size: usize,
    pub banks_used: BTreeMap<u32, usize>,
    pub zp_used: usize,
    pub ram_used: usize,
    pub var_addr: BTreeMap<(InstId, VarId), u16>,
    pub obj_addr: BTreeMap<(ObjId, bool), u16>,
}

pub struct Unit {
    pub inst: InstId,
    pub lines: Vec<Line>,
    pub extra_vars: Vec<Var>,
    pub helpers: BTreeSet<&'static str>,
    pub far: BTreeSet<InstId>,
}

struct Ram {
    /// Free [start, end) byte ranges.
    free: Vec<(u16, u16)>,
}

impl Ram {
    fn take_at(&mut self, at: u16, size: u16) -> bool {
        for i in 0..self.free.len() {
            let (s, e) = self.free[i];
            if s <= at && at + size <= e {
                self.free.remove(i);
                if s < at {
                    self.free.push((s, at));
                }
                if at + size < e {
                    self.free.push((at + size, e));
                }
                self.free.sort();
                return true;
            }
        }
        false
    }
    fn alloc(&mut self, size: u16, lo: u16, hi: u16) -> Option<u16> {
        // Best fit inside [lo, hi).
        let mut best: Option<(u16, u16)> = None;
        for &(s, e) in &self.free {
            let s2 = s.max(lo);
            let e2 = e.min(hi);
            if e2 > s2 && e2 - s2 >= size {
                let slack = e2 - s2 - size;
                if best.is_none_or(|(_, b)| slack < b) {
                    best = Some((s2, slack));
                }
            }
        }
        let (at, _) = best?;
        self.take_at(at, size);
        Some(at)
    }
    fn largest(&self, lo: u16, hi: u16) -> (u16, u16) {
        self.free
            .iter()
            .map(|&(s, e)| (s.max(lo), e.min(hi)))
            .filter(|(s, e)| e > s)
            .max_by_key(|(s, e)| e - s)
            .unwrap_or((lo, lo))
    }
}

pub struct LinkInput<'a> {
    pub mir: &'a Mir,
    pub units: Vec<Unit>,
    pub cart: &'a Cartridge,
    pub board: Board,
}

fn helper_source(name: &str, h: &str) -> String {
    // Arguments: 8-bit helpers take A and `{h}_b`; 16-bit take `{h}_a`, `{h}_b`
    // and return A (low) / X (high).
    match name {
        "__mul8" => format!(
            "{h}:\n sta {h}_t\n lda #0\n ldx #8\n{h}_l:\n asl a\n asl {h}_t\n bcc {h}_s\n clc\n adc {h}_b\n{h}_s:\n dex\n bne {h}_l\n rts\n"
        ),
        "__div8" | "__rem8" => format!(
            "{h}:\n sta {h}_t\n lda #0\n ldx #8\n{h}_l:\n asl {h}_t\n rol a\n cmp {h}_b\n bcc {h}_s\n sbc {h}_b\n inc {h}_t\n{h}_s:\n dex\n bne {h}_l\n{}\n rts\n",
            if name == "__div8" { " lda {h}_t" } else { "" }
        )
        .replace("{h}", h),
        "__divs8" | "__rems8" => {
            // Sign-magnitude around the unsigned routine.
            let rem = name == "__rems8";
            format!(
                "{h}:\n sta {h}_t\n eor {h}_b\n sta {h}_s\n lda {h}_b\n bpl {h}_p1\n eor #$ff\n clc\n adc #1\n sta {h}_b\n{h}_p1:\n lda {h}_t\n {}\n bpl {h}_p2\n eor #$ff\n clc\n adc #1\n{h}_p2:\n sta {h}_t\n lda #0\n ldx #8\n{h}_l:\n asl {h}_t\n rol a\n cmp {h}_b\n bcc {h}_k\n sbc {h}_b\n inc {h}_t\n{h}_k:\n dex\n bne {h}_l\n{}\n rts\n",
                if rem { "sta {h}_u" } else { "cmp #0" },
                if rem {
                    " ldx {h}_u\n bpl {h}_d\n eor #$ff\n clc\n adc #1\n{h}_d:"
                } else {
                    " lda {h}_t\n ldx {h}_s\n bpl {h}_d\n eor #$ff\n clc\n adc #1\n{h}_d:"
                }
            )
            .replace("{h}", h)
        }
        "__mul16" => format!(
            "{h}:\n lda #0\n sta {h}_r\n sta {h}_r+1\n ldx #16\n{h}_l:\n asl {h}_r\n rol {h}_r+1\n asl {h}_a\n rol {h}_a+1\n bcc {h}_s\n clc\n lda {h}_r\n adc {h}_b\n sta {h}_r\n lda {h}_r+1\n adc {h}_b+1\n sta {h}_r+1\n{h}_s:\n dex\n bne {h}_l\n lda {h}_r\n ldx {h}_r+1\n rts\n"
        ),
        "__div16" | "__rem16" => format!(
            "{h}:\n lda #0\n sta {h}_r\n sta {h}_r+1\n ldx #16\n{h}_l:\n asl {h}_a\n rol {h}_a+1\n rol {h}_r\n rol {h}_r+1\n lda {h}_r\n sec\n sbc {h}_b\n tay\n lda {h}_r+1\n sbc {h}_b+1\n bcc {h}_s\n sta {h}_r+1\n sty {h}_r\n inc {h}_a\n{h}_s:\n dex\n bne {h}_l\n{}\n rts\n",
            if name == "__div16" { " lda {h}_a\n ldx {h}_a+1" } else { " lda {h}_r\n ldx {h}_r+1" }
        )
        .replace("{h}", h),
        "__divs16" | "__rems16" => {
            let rem = name == "__rems16";
            format!(
                "{h}:\n lda {h}_a+1\n sta {h}_sa\n eor {h}_b+1\n sta {h}_s\n lda {h}_a+1\n bpl {h}_p1\n lda #0\n sec\n sbc {h}_a\n sta {h}_a\n lda #0\n sbc {h}_a+1\n sta {h}_a+1\n{h}_p1:\n lda {h}_b+1\n bpl {h}_p2\n lda #0\n sec\n sbc {h}_b\n sta {h}_b\n lda #0\n sbc {h}_b+1\n sta {h}_b+1\n{h}_p2:\n lda #0\n sta {h}_r\n sta {h}_r+1\n ldx #16\n{h}_l:\n asl {h}_a\n rol {h}_a+1\n rol {h}_r\n rol {h}_r+1\n lda {h}_r\n sec\n sbc {h}_b\n tay\n lda {h}_r+1\n sbc {h}_b+1\n bcc {h}_k\n sta {h}_r+1\n sty {h}_r\n inc {h}_a\n{h}_k:\n dex\n bne {h}_l\n{}\n rts\n",
                if rem {
                    " lda {h}_r\n ldx {h}_r+1\n bit {h}_sa\n bpl {h}_d\n sta {h}_r\n lda #0\n sec\n sbc {h}_r\n pha\n lda #0\n sbc {h}_r+1\n tax\n pla\n{h}_d:"
                } else {
                    " lda {h}_a\n ldx {h}_a+1\n bit {h}_s\n bpl {h}_d\n lda #0\n sec\n sbc {h}_a\n pha\n lda #0\n sbc {h}_a+1\n tax\n pla\n{h}_d:"
                }
            )
            .replace("{h}", h)
        }
        _ => String::new(),
    }
}

fn helper_scratch(name: &str) -> &'static [(&'static str, u16)] {
    match name {
        "__mul8" | "__div8" | "__rem8" => &[("_b", 1), ("_t", 1)],
        "__divs8" | "__rems8" => &[("_b", 1), ("_t", 1), ("_s", 1), ("_u", 1)],
        "__mul16" | "__div16" | "__rem16" => &[("_a", 2), ("_b", 2), ("_r", 2)],
        _ => &[("_a", 2), ("_b", 2), ("_r", 2), ("_s", 1), ("_sa", 1)],
    }
}

/// Assembly text using global labels, parsed with a no-op prefix.
fn glue(text: &str) -> Vec<Line> {
    match asm::parse(text, "", &|_| None) {
        Ok(l) => l,
        Err((line, msg)) => panic!("internal glue error at line {line}: {msg}\n{text}"),
    }
}

/// Mapper register write sequence: select PRG bank in A for a window.
fn bank_routine(board: Board, window: &str) -> String {
    match (board, window) {
        (Board::Mmc3, "code") => " sta __code_shadow\n pha\n lda #6\n sta $8000\n pla\n sta $8001\n rts\n".into(),
        (Board::Mmc3, _) => " sta __bank_shadow\n pha\n lda #7\n sta $8000\n pla\n sta $8001\n rts\n".into(),
        (Board::Mmc1, _) => " sta __bank_shadow\n sta $E000\n lsr a\n sta $E000\n lsr a\n sta $E000\n lsr a\n sta $E000\n lsr a\n sta $E000\n lda __bank_shadow\n rts\n".into(),
        (Board::Uxrom | Board::Bnrom, _) => " sta __bank_shadow\n tay\n sta __bank_table,y\n rts\n".into(),
        (Board::Axrom, _) => " sta __bank_shadow\n ora __chr_shadow\n tay\n sta __bank_table,y\n lda __bank_shadow\n rts\n".into(),
        (Board::Gxrom, _) => " sta __bank_shadow\n asl a\n asl a\n asl a\n asl a\n ora __chr_shadow\n tay\n sta __bank_table,y\n lda __bank_shadow\n rts\n".into(),
        (Board::ColorDreams, _) => " sta __bank_shadow\n ora __chr_shadow\n tay\n sta __bank_table,y\n lda __bank_shadow\n rts\n".into(),
        _ => " rts\n".into(),
    }
}

fn mapper_init(board: Board, cart: &Cartridge) -> String {
    let mirror = cart.mirroring.as_str();
    match board {
        Board::Mmc1 => {
            let m = match mirror {
                "horizontal" => 3,
                "single" => 0,
                _ => 2,
            };
            // PRG mode 3 (fixed $C000), 4 KiB CHR banks.
            let control = m | 0x0c | 0x10;
            format!(
                " lda #$80\n sta $8000\n lda #{control}\n sta $8000\n lsr a\n sta $8000\n lsr a\n sta $8000\n lsr a\n sta $8000\n lsr a\n sta $8000\n lda #0\n jsr __mmc1_chr0\n lda #1\n jsr __mmc1_chr1\n lda #0\n jsr __bank_data\n"
            )
        }
        Board::Mmc3 => {
            let m = if mirror == "horizontal" { 1 } else { 0 };
            let mut s = String::new();
            for (r, v) in [(0, 0), (1, 2), (2, 4), (3, 5), (4, 6), (5, 7), (6, 0), (7, 1)] {
                s.push_str(&format!(" lda #{r}\n sta $8000\n lda #{v}\n sta $8001\n"));
            }
            s.push_str(&format!(
                " lda #{m}\n sta $A000\n lda #$80\n sta $A001\n sta $E000\n lda #0\n sta __code_shadow\n lda #1\n sta __bank_shadow\n"
            ));
            s
        }
        Board::Uxrom | Board::Bnrom | Board::Axrom | Board::Gxrom | Board::ColorDreams => {
            " lda #0\n sta __chr_shadow\n jsr __bank_data\n".into()
        }
        Board::Cnrom => " lda #0\n tay\n sta __bank_table,y\n".into(),
        Board::Nrom => String::new(),
    }
}

pub fn link(input: LinkInput) -> Result<Linked, Vec<String>> {
    let LinkInput { mir, units, cart, board } = input;
    let mut errors = Vec::new();
    let banking = board.banking();
    let mut out = Linked::default();

    // ------------------------------------------------------------- RAM
    let stack: u16 = cart.fields.get("stack").map(|s| *s as u16).unwrap_or(64).clamp(16, 256);
    let mut ram = Ram {
        free: vec![(0x0000, 0x0100), (0x0100, 0x0200 - stack), (0x0200, 0x0800)],
    };
    ram.free.retain(|(s, e)| e > s);
    let mut var_addr: BTreeMap<(InstId, VarId), u16> = BTreeMap::new();
    let mut obj_addr: BTreeMap<(ObjId, bool), u16> = BTreeMap::new();
    let mut extra_ram: BTreeMap<String, u16> = BTreeMap::new();
    let mut symbols = Vec::new();

    // Objects referenced by any instruction or data.
    let mut used_objs: BTreeSet<u32> = BTreeSet::new();
    for u in &units {
        for l in &u.lines {
            match l {
                Line::Ins(ins) => {
                    if let Some(Opnd { sym: Sym::Obj(o, _), .. }) = ins.arg.opnd() {
                        used_objs.insert(o.0);
                    }
                }
                Line::Bytes(v) | Line::Words(v) => {
                    for o in v {
                        if let Sym::Obj(id, _) = o.sym {
                            used_objs.insert(id.0);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Fixed placements first.
    for (i, o) in mir.objs.iter().enumerate() {
        let id = ObjId(i as u32);
        if let ObjKind::Ram(Placement::At(addr)) = o.kind {
            if !used_objs.contains(&(i as u32)) {
                continue;
            }
            let size = o.bytes() as u16;
            if !ram.take_at(addr, size) {
                errors.push(format!("'{}' at ${addr:04X} overlaps other memory", o.name));
            }
            place_obj(&mut obj_addr, id, o, addr);
        }
    }
    // Runtime glue storage in zero page.
    let recursive_bytes: usize = {
        let mut max_level = 0usize;
        let mut depth = 0u32;
        for inst in &mir.insts {
            if let Some(n) = inst.recursive {
                for b in &inst.blocks {
                    for s in &b.stmts {
                        if let St::Call { saved, target, .. } = s {
                            if *target == InstId(mir.insts.iter().position(|x| std::ptr::eq(x, inst)).unwrap() as u32) {
                                let bytes: usize = saved.iter().map(|v| inst.vars[v.0 as usize].sc.size()).sum();
                                max_level = max_level.max(bytes);
                            }
                        }
                    }
                }
                depth = depth.max(n);
            }
        }
        max_level * depth as usize
    };
    let mut glue_zp: Vec<(&str, u16)> = vec![("__bank_shadow", 1), ("__code_shadow", 1), ("__chr_shadow", 1), ("__far_a", 1), ("__far_x", 1)];
    if recursive_bytes > 0 {
        glue_zp.push(("__save_sp", 1));
    }
    for (name, size) in &glue_zp {
        match ram.alloc(*size, 0, 0x100) {
            Some(a) => {
                extra_ram.insert(name.to_string(), a);
            }
            None => errors.push("Zero page is full".into()),
        }
    }
    // @zp globals.
    for (i, o) in mir.objs.iter().enumerate() {
        let id = ObjId(i as u32);
        if o.kind == ObjKind::Ram(Placement::ZeroPage) && used_objs.contains(&(i as u32)) {
            match ram.alloc(o.bytes() as u16, 0, 0x100) {
                Some(a) => place_obj(&mut obj_addr, id, o, a),
                None => errors.push(format!("Zero page is full; cannot place '{}'", o.name)),
            }
        }
    }
    // Helper scratch, per context.
    let mut helper_set: BTreeSet<(&'static str, Context)> = BTreeSet::new();
    for u in &units {
        let ctx = mir.insts[u.inst.0 as usize].ctx;
        for h in &u.helpers {
            helper_set.insert((h, ctx));
        }
    }
    for (h, ctx) in &helper_set {
        let label = helper_label(h, *ctx);
        for (suffix, size) in helper_scratch(h) {
            match ram.alloc(*size, 0, 0x100) {
                Some(a) => {
                    extra_ram.insert(format!("{label}{suffix}"), a);
                }
                None => errors.push("Zero page is full (arithmetic helpers)".into()),
            }
        }
    }

    // Frames: overlay by call graph within each context.
    let n = mir.insts.len();
    let mut vars_of: Vec<Vec<Var>> = mir.insts.iter().map(|i| i.vars.clone()).collect();
    let mut lines_of: Vec<Vec<Line>> = vec![Vec::new(); n];
    let mut helpers_of: Vec<BTreeSet<&'static str>> = vec![BTreeSet::new(); n];
    let mut far_of: Vec<BTreeSet<InstId>> = vec![BTreeSet::new(); n];
    let mut generated = vec![false; n];
    for u in units {
        let i = u.inst.0 as usize;
        vars_of[i].extend(u.extra_vars);
        lines_of[i] = u.lines;
        helpers_of[i] = u.helpers;
        far_of[i] = u.far;
        generated[i] = true;
    }
    let mut callees: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for (i, lines) in lines_of.iter().enumerate() {
        for l in lines {
            if let Line::Ins(ins) = l {
                if let Some(Opnd { sym: Sym::Label(name), .. }) = ins.arg.opnd() {
                    if let Some(rest) = name.strip_prefix("__f").or_else(|| name.strip_prefix("__far")) {
                        if let Ok(j) = rest.parse::<usize>() {
                            if j != i && j < n {
                                callees[i].insert(j);
                            }
                        }
                    }
                }
            }
        }
    }
    // Variable use counts (zero-page priority).
    let mut uses: BTreeMap<(usize, u32), usize> = BTreeMap::new();
    for lines in lines_of.iter() {
        for l in lines {
            if let Line::Ins(ins) = l {
                if let Some(Opnd { sym: Sym::Var(inst, v), .. }) = ins.arg.opnd() {
                    *uses.entry((inst.0 as usize, v.0)).or_default() += 1;
                }
            }
        }
    }
    // Topological order (callers before callees).
    let mut order = Vec::new();
    {
        let mut state = vec![0u8; n];
        fn visit(i: usize, callees: &[BTreeSet<usize>], state: &mut [u8], order: &mut Vec<usize>, errors: &mut Vec<String>, mir: &Mir) {
            if state[i] == 2 {
                return;
            }
            if state[i] == 1 {
                errors.push(format!(
                    "Recursion through '{}' must be direct self-recursion marked @recursive(N)",
                    mir.insts[i].name
                ));
                return;
            }
            state[i] = 1;
            for c in &callees[i] {
                visit(*c, callees, state, order, errors, mir);
            }
            state[i] = 2;
            order.push(i);
        }
        for i in 0..n {
            visit(i, &callees, &mut state, &mut order, &mut errors, mir);
        }
        order.reverse();
    }
    // Every variable starts in zero page except non-pointer ones demoted
    // until the overlaid zero-page frames fit.
    let zp_free = |ram: &Ram| -> u16 {
        ram.free.iter().filter(|(s, _)| *s < 0x100).map(|(s, e)| e.min(&0x100) - s).sum()
    };
    let mut in_zp: Vec<Vec<bool>> = vars_of.iter().map(|v| vec![true; v.len()]).collect();
    let ctx_of: Vec<Context> = mir.insts.iter().map(|i| i.ctx).collect();
    let overlay = |in_zp: &Vec<Vec<bool>>, zp: bool| -> (Vec<u16>, BTreeMap<Context, u16>) {
        let mut offset = vec![0u16; n];
        let mut peak: BTreeMap<Context, u16> = BTreeMap::new();
        for &i in &order {
            let size: u16 = vars_of[i]
                .iter()
                .zip(&in_zp[i])
                .filter(|(_, z)| **z == zp)
                .map(|(v, _)| v.sc.size() as u16)
                .sum();
            let end = offset[i] + size;
            let p = peak.entry(ctx_of[i]).or_default();
            *p = (*p).max(end);
            for c in &callees[i] {
                if ctx_of[*c] == ctx_of[i] {
                    offset[*c] = offset[*c].max(end);
                }
            }
        }
        (offset, peak)
    };
    // Reserve some zero page for hot scalar globals.
    let reserve = 8u16;
    let budget = zp_free(&ram).saturating_sub(reserve);
    let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
    for (i, vars) in vars_of.iter().enumerate() {
        for (k, v) in vars.iter().enumerate() {
            if !v.pointer {
                candidates.push((*uses.get(&(i, k as u32)).unwrap_or(&0), i, k));
            }
        }
    }
    candidates.sort();
    let mut next = 0;
    loop {
        let (_, peak) = overlay(&in_zp, true);
        let total: u16 = peak.values().sum();
        if total <= budget || next >= candidates.len() {
            if total > budget + reserve {
                errors.push(format!("Zero-page frames need {total} bytes; only {budget} are free"));
            }
            break;
        }
        let (_, i, k) = candidates[next];
        in_zp[i][k] = false;
        next += 1;
    }
    let (zp_off, zp_peak) = overlay(&in_zp, true);
    let (ram_off, ram_peak) = overlay(&in_zp, false);
    // Place each context's frame region.
    let mut zp_base: BTreeMap<Context, u16> = BTreeMap::new();
    let mut ram_base: BTreeMap<Context, u16> = BTreeMap::new();
    for (ctx, size) in &zp_peak {
        if *size == 0 {
            zp_base.insert(*ctx, 0);
            continue;
        }
        match ram.alloc(*size, 0, 0x100) {
            Some(a) => {
                zp_base.insert(*ctx, a);
            }
            None => errors.push(format!("Zero page cannot hold {size} bytes of {ctx:?} frames")),
        }
    }
    for (ctx, size) in &ram_peak {
        if *size == 0 {
            ram_base.insert(*ctx, 0x200);
            continue;
        }
        match ram.alloc(*size, 0x100, 0x800) {
            Some(a) => {
                ram_base.insert(*ctx, a);
            }
            None => errors.push(format!("RAM cannot hold {size} bytes of {ctx:?} frames")),
        }
    }
    let mut frame_bytes = 0;
    for i in 0..n {
        let ctx = ctx_of[i];
        let mut zo = zp_off[i] + zp_base.get(&ctx).copied().unwrap_or(0);
        let mut ro = ram_off[i] + ram_base.get(&ctx).copied().unwrap_or(0x200);
        for (k, v) in vars_of[i].iter().enumerate() {
            let size = v.sc.size() as u16;
            let addr = if in_zp[i][k] {
                let a = zo;
                zo += size;
                a
            } else {
                let a = ro;
                ro += size;
                a
            };
            var_addr.insert((InstId(i as u32), VarId(k as u32)), addr);
            frame_bytes += size as usize;
            if !v.temp {
                symbols.push(Symbol {
                    name: format!("{}::{}", mir.insts[i].name, v.name),
                    address: addr,
                    size: size as usize,
                    kind: format!("local {}", v.sc.name()),
                    bank: None,
                });
            }
        }
    }
    // Frame-local arrays: after each context's frame region.
    for (i, o) in mir.objs.iter().enumerate() {
        if let ObjKind::Frame(_) = o.kind {
            match ram.alloc(o.bytes() as u16, 0x100, 0x800) {
                Some(a) => place_obj(&mut obj_addr, ObjId(i as u32), o, a),
                None => errors.push(format!("RAM is full; cannot place '{}'", o.name)),
            }
        }
    }
    // Hot scalar globals into the remaining zero page, then everything else.
    let mut global_uses: BTreeMap<u32, usize> = BTreeMap::new();
    for lines in &lines_of {
        for l in lines {
            if let Line::Ins(ins) = l {
                if let Some(Opnd { sym: Sym::Obj(o, _), .. }) = ins.arg.opnd() {
                    *global_uses.entry(o.0).or_default() += 1;
                }
            }
        }
    }
    let mut pending: Vec<usize> = (0..mir.objs.len())
        .filter(|i| matches!(mir.objs[*i].kind, ObjKind::Ram(Placement::Auto | Placement::Ram)) && !obj_addr.contains_key(&(ObjId(*i as u32), false)) && used_objs.contains(&(*i as u32)))
        .collect();
    pending.sort_by_key(|i| std::cmp::Reverse((global_uses.get(&(*i as u32)).copied().unwrap_or(0) * 4) / mir.objs[*i].bytes().max(1)));
    // Recursion save stack.
    if recursive_bytes > 0 {
        match ram.alloc(recursive_bytes as u16, 0x100, 0x800) {
            Some(a) => {
                extra_ram.insert("__save_stack".into(), a);
            }
            None => errors.push(format!("RAM cannot hold the {recursive_bytes}-byte recursion save stack")),
        }
    }
    for i in pending {
        let o = &mir.objs[i];
        let size = o.bytes() as u16;
        let zp_ok = o.kind == ObjKind::Ram(Placement::Auto) && o.len.is_none() && global_uses.get(&(i as u32)).copied().unwrap_or(0) > 0;
        let addr = if zp_ok { ram.alloc(size, 0, 0x100) } else { None }.or_else(|| ram.alloc(size, 0x100, 0x800)).or_else(|| ram.alloc(size, 0, 0x100));
        match addr {
            Some(a) => place_obj(&mut obj_addr, ObjId(i as u32), o, a),
            None => {
                let (s, e) = ram.largest(0x100, 0x800);
                errors.push(format!("RAM is full: '{}' needs {size} bytes (largest free block {} bytes)", o.name, e - s));
            }
        }
    }
    for (i, o) in mir.objs.iter().enumerate() {
        let id = ObjId(i as u32);
        if let Some(a) = obj_addr.get(&(id, false)) {
            symbols.push(Symbol {
                name: o.name.clone(),
                address: *a,
                size: o.bytes(),
                kind: format!("{} {}{}", if matches!(o.kind, ObjKind::Frame(_)) { "local" } else { "var" }, o.sc.name(), o.len.map(|n| format!("[{n}]")).unwrap_or_default()),
                bank: None,
            });
        } else if let ObjKind::Io(a) = o.kind {
            symbols.push(Symbol {
                name: o.name.clone(),
                address: a,
                size: o.bytes(),
                kind: "io".into(),
                bank: None,
            });
        }
    }
    out.zp_used = 256 - zp_free(&ram) as usize;
    out.ram_used = {
        let free: u16 = ram.free.iter().filter(|(s, _)| *s >= 0x100).map(|(s, e)| e - s).sum();
        (0x700 - stack) as usize - free as usize
    };

    // -------------------------------------------------------- sections
    let fixed_key: Option<u32> = None;
    let mut sections: BTreeMap<Option<u32>, Vec<Line>> = BTreeMap::new();
    sections.insert(fixed_key, Vec::new());
    for i in 0..n {
        if !generated[i] {
            continue;
        }
        let bank = mir.insts[i].bank;
        if bank.is_some() && banking == Banking::None {
            errors.push(format!("'{}' uses @bank but {} has no PRG banking", mir.insts[i].name, board.name()));
        }
        if bank.is_some() && mir.insts[i].ctx != Context::Main {
            errors.push(format!("Interrupt code '{}' must stay in the fixed bank", mir.insts[i].name));
        }
        sections.entry(bank).or_default().extend(lines_of[i].clone());
    }
    // Constant data.
    for (i, o) in mir.objs.iter().enumerate() {
        if o.kind != ObjKind::Rom {
            continue;
        }
        let used = global_uses.contains_key(&(i as u32)) || o.bank.is_some();
        if !used {
            continue;
        }
        if o.bank.is_some() && banking == Banking::None {
            errors.push(format!("'{}' uses @bank but {} has no PRG banking", o.name, board.name()));
        }
        let sec = sections.entry(o.bank).or_default();
        let values: Vec<i64> = if o.init.is_empty() { vec![0; o.len.unwrap_or(1) as usize] } else { o.init.clone() };
        sec.push(Line::Label(format!("__o{i}")));
        sec.push(Line::Bytes(values.iter().map(|v| Opnd::num(v & 0xff)).collect()));
        if o.sc.wide() {
            if o.len.is_some() {
                sec.push(Line::Label(format!("__o{i}h")));
            }
            sec.push(Line::Bytes(values.iter().map(|v| Opnd::num((v >> 8) & 0xff)).collect()));
        }
    }
    // Glue: helpers, bank routines, trampolines, start-up, vectors.
    let mut fixed_text = String::new();
    for (h, ctx) in &helper_set {
        fixed_text.push_str(&helper_source(h, &helper_label(h, *ctx)));
    }
    fixed_text.push_str("__bank_data:\n");
    fixed_text.push_str(&bank_routine(board, "data"));
    fixed_text.push_str("__bank_code:\n");
    fixed_text.push_str(&bank_routine(board, if board == Board::Mmc3 { "code" } else { "data" }));
    if board == Board::Mmc1 {
        fixed_text.push_str("__mmc1_chr0:\n sta $A000\n lsr a\n sta $A000\n lsr a\n sta $A000\n lsr a\n sta $A000\n lsr a\n sta $A000\n rts\n");
        fixed_text.push_str("__mmc1_chr1:\n sta $C000\n lsr a\n sta $C000\n lsr a\n sta $C000\n lsr a\n sta $C000\n lsr a\n sta $C000\n rts\n");
    }
    if matches!(board, Board::Uxrom | Board::Bnrom | Board::Axrom | Board::Gxrom | Board::ColorDreams | Board::Cnrom) {
        fixed_text.push_str("__bank_table:\n");
        let bytes: Vec<String> = (0..256).map(|v| v.to_string()).collect();
        for chunk in bytes.chunks(32) {
            fixed_text.push_str(&format!(" .byte {}\n", chunk.join(",")));
        }
    }
    let mut far_targets = BTreeSet::new();
    for f in &far_of {
        far_targets.extend(f.iter().copied());
    }
    for t in &far_targets {
        let bank = mir.insts[t.0 as usize].bank.unwrap_or(0);
        let shadow = if board == Board::Mmc3 { "__code_shadow" } else { "__bank_shadow" };
        fixed_text.push_str(&format!(
            "__far{0}:\n lda {shadow}\n pha\n lda #{bank}\n jsr __bank_code\n jsr {1}\n sta __far_a\n stx __far_x\n pla\n jsr __bank_code\n ldx __far_x\n lda __far_a\n rts\n",
            t.0,
            fn_label(*t)
        ));
    }
    // CHR RAM upload.
    let chr_ram = !board.chr_rom() || cart.chr_ram;
    let mut chr_upload = String::new();
    if chr_ram && !cart.chr.is_empty() {
        // Copied from PRG at boot; 8 KiB at most.
        chr_upload.push_str(" lda $2002\n lda #0\n sta $2006\n sta $2006\n lda #<__chr_data\n sta __far_a\n lda #>__chr_data\n sta __far_x\n ldy #0\n ldx #32\n__chr_copy:\n lda (__far_a),y\n sta $2007\n iny\n bne __chr_copy\n inc __far_x\n dex\n bne __chr_copy\n");
    }
    let reset = mir.insts.iter().position(|i| i.reset).map(|i| fn_label(InstId(i as u32)));
    let nmi = mir.insts.iter().position(|i| i.interrupt == Some(Context::Nmi)).map(|i| fn_label(InstId(i as u32)));
    let irq = mir.insts.iter().position(|i| i.interrupt == Some(Context::Irq)).map(|i| fn_label(InstId(i as u32)));
    if reset.is_none() {
        errors.push("No @reset entry point (the standard runtime provides one)".into());
    }
    // RAM initial values.
    let mut init = String::from(" lda #0\n tax\n__clear:\n sta $00,x\n sta $0100,x\n sta $0200,x\n sta $0300,x\n sta $0400,x\n sta $0500,x\n sta $0600,x\n sta $0700,x\n inx\n bne __clear\n");
    let mut init_tables = String::new();
    for (i, o) in mir.objs.iter().enumerate() {
        if !matches!(o.kind, ObjKind::Ram(_)) || o.init.iter().all(|v| *v == 0) {
            continue;
        }
        let Some(lo) = obj_addr.get(&(ObjId(i as u32), false)).copied() else { continue };
        let hi = obj_addr.get(&(ObjId(i as u32), true)).copied();
        let mut bytes: Vec<(u16, u8)> = Vec::new();
        for (k, v) in o.init.iter().enumerate() {
            if o.len.is_some() {
                bytes.push((lo + k as u16, (*v & 0xff) as u8));
                if let Some(h) = hi {
                    bytes.push((h + k as u16, ((*v >> 8) & 0xff) as u8));
                }
            } else {
                bytes.push((lo, (*v & 0xff) as u8));
                if o.sc.wide() {
                    bytes.push((lo + 1, ((*v >> 8) & 0xff) as u8));
                }
            }
        }
        let nonzero = bytes.iter().filter(|(_, v)| *v != 0).count();
        if nonzero <= 12 {
            for (a, v) in bytes.into_iter().filter(|(_, v)| *v != 0) {
                init.push_str(&format!(" lda #{v}\n sta ${a:04X}\n"));
            }
        } else {
            // Copy contiguous runs from ROM tables.
            let mut runs: Vec<(u16, Vec<u8>)> = Vec::new();
            for (a, v) in bytes {
                match runs.last_mut() {
                    Some((start, data)) if *start + data.len() as u16 == a && data.len() < 256 => data.push(v),
                    _ => runs.push((a, vec![v])),
                }
            }
            for (k, (start, data)) in runs.into_iter().enumerate() {
                let label = format!("__init{i}_{k}");
                init.push_str(&format!(" ldx #0\n{label}_l:\n lda {label},x\n sta ${start:04X},x\n inx\n cpx #{}\n bne {label}_l\n", data.len() & 0xff));
                init_tables.push_str(&format!("{label}:\n"));
                for chunk in data.chunks(32) {
                    init_tables.push_str(&format!(" .byte {}\n", chunk.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")));
                }
            }
        }
    }
    fixed_text.push_str(&init_tables);
    fixed_text.push_str(&format!(
        "__start:\n sei\n cld\n ldx #$ff\n txs\n inx\n stx $2000\n stx $2001\n stx $4010\n lda #$40\n sta $4017\n{init}{}{}{} jmp {}\n__rti:\n rti\n",
        mapper_init(board, cart),
        if recursive_bytes > 0 { " lda #0\n sta __save_sp\n" } else { "" },
        chr_upload,
        reset.clone().unwrap_or("__rti".into())
    ));
    let fixed = sections.get_mut(&fixed_key).unwrap();
    fixed.extend(glue(&fixed_text));
    if chr_ram && !cart.chr.is_empty() {
        fixed.push(Line::Label("__chr_data".into()));
        let mut data = cart.chr.clone();
        data.resize(8192, 0);
        fixed.push(Line::Bytes(data.iter().map(|b| Opnd::num(*b as i64)).collect()));
    }

    // Symbol resolution for RAM and objects.
    let resolve = |s: &Sym| -> Option<i64> {
        match s {
            Sym::Var(i, v) => var_addr.get(&(*i, *v)).map(|a| *a as i64),
            Sym::Obj(o, hi) => obj_addr.get(&(*o, *hi)).map(|a| *a as i64),
            _ => None,
        }
    };
    // ROM object references become labels.
    let rom_label = |lines: &mut Vec<Line>| {
        for l in lines.iter_mut() {
            let fix = |o: &mut Opnd| {
                if let Sym::Obj(id, hi) = o.sym {
                    if mir.objs[id.0 as usize].kind == ObjKind::Rom {
                        let wide_scalar = mir.objs[id.0 as usize].len.is_none();
                        o.sym = Sym::Label(format!("__o{}{}", id.0, if hi && !wide_scalar { "h" } else { "" }));
                    }
                }
            };
            match l {
                Line::Ins(ins) => {
                    if let Some(o) = ins.arg.opnd_mut() {
                        fix(o);
                    }
                }
                Line::Bytes(v) | Line::Words(v) => v.iter_mut().for_each(fix),
                _ => {}
            }
        }
    };
    for lines in sections.values_mut() {
        rom_label(lines);
    }
    let external_names: BTreeMap<String, u16> = extra_ram.clone();

    // Section bases and sizes.
    let fixed_base: u16 = match board {
        Board::Nrom | Board::Cnrom => 0x8000,
        Board::Uxrom | Board::Mmc1 | Board::Mmc3 => 0xC000,
        _ => 0, // computed from the common size
    };
    let vector_bytes = 6usize;
    let size_of = |lines: &[Line], base: u16| -> Result<asm::Encoded, String> {
        asm::encode(lines, base, &resolve, &|n| external_names.get(n).map(|a| *a as i64).or(Some(0x8000)))
    };
    let fixed_lines = sections.remove(&fixed_key).unwrap();
    let fixed_trial = match size_of(&fixed_lines, 0x8000) {
        Ok(e) => e,
        Err(e) => {
            errors.push(e);
            return Err(errors);
        }
    };
    let fixed_len = fixed_trial.bytes.len();
    let fixed_cap = match board {
        Board::Nrom | Board::Cnrom => 0x8000,
        Board::Uxrom | Board::Mmc1 | Board::Mmc3 => 0x4000,
        _ => 0x8000,
    } - vector_bytes - if board == Board::Mmc1 { 16 } else { 0 };
    let fixed_base = if board.whole() {
        // Common code sits at the top of every 32 KiB bank.
        let start = (0x10000 - vector_bytes - fixed_len) & !0xff;
        start as u16
    } else {
        fixed_base
    };
    if fixed_len > fixed_cap {
        errors.push(format!(
            "Fixed code and data use {fixed_len} bytes, exceeding {fixed_cap} on {}; move functions or constants into @bank(N)",
            board.name()
        ));
        return Err(errors);
    }
    let bank_base = |b: u32| -> u16 {
        let _ = b;
        match board {
            Board::Mmc3 => 0x8000,
            _ => 0x8000,
        }
    };
    let mut all_labels: BTreeMap<String, u16> = BTreeMap::new();
    let first = asm::encode(&fixed_lines, fixed_base, &resolve, &|n| external_names.get(n).map(|a| *a as i64).or(Some(0x8000)));
    let first = match first {
        Ok(e) => e,
        Err(e) => {
            errors.push(e);
            return Err(errors);
        }
    };
    all_labels.extend(first.labels.clone());
    let mut bank_trials = BTreeMap::new();
    for (b, lines) in &sections {
        let b = b.unwrap();
        // MMC3 data-only banks map at $A000.
        let base = if board == Board::Mmc3 && !lines.iter().any(|l| matches!(l, Line::Ins(_))) { 0xA000 } else { bank_base(b) };
        match asm::encode(lines, base, &resolve, &|n| external_names.get(n).map(|a| *a as i64).or(Some(0x8000))) {
            Ok(e) => {
                let cap = if board.whole() { fixed_base as usize - 0x8000 } else { board.bank_size() };
                if e.bytes.len() > cap {
                    errors.push(format!("Bank {b} uses {} bytes, exceeding {cap}", e.bytes.len()));
                }
                all_labels.extend(e.labels.clone());
                bank_trials.insert(b, base);
            }
            Err(e) => errors.push(e),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for (k, v) in &extra_ram {
        all_labels.insert(k.clone(), *v);
    }
    let external = |n: &str| all_labels.get(n).map(|a| *a as i64);
    let fixed_enc = match asm::encode(&fixed_lines, fixed_base, &resolve, &external) {
        Ok(e) => e,
        Err(e) => return Err(vec![e]),
    };
    let mut bank_encs = BTreeMap::new();
    for (b, lines) in &sections {
        let b = b.unwrap();
        match asm::encode(lines, bank_trials[&b], &resolve, &external) {
            Ok(e) => {
                bank_encs.insert(b, e);
            }
            Err(e) => return Err(vec![e]),
        }
    }
    if board == Board::Mmc3 {
        for (b, base) in &bank_trials {
            let has_code = sections[&Some(*b)].iter().any(|l| matches!(l, Line::Ins(_)));
            let has_data = sections[&Some(*b)].iter().any(|l| matches!(l, Line::Bytes(_)));
            if has_code && has_data {
                errors.push(format!("MMC3 bank {b} mixes code and constants; give banked functions and banked data different bank numbers"));
            }
            let _ = base;
        }
    }
    // Vectors.
    let vec_of = |l: &Option<String>| -> u16 {
        l.as_ref().and_then(|l| all_labels.get(l).copied()).unwrap_or(all_labels["__rti"])
    };
    let vectors = [vec_of(&nmi), all_labels["__start"], vec_of(&irq)];

    // ----------------------------------------------------------- image
    let max_bank = bank_encs.keys().max().copied();
    let bank_size = board.bank_size();
    let mut prg: Vec<u8>;
    match board {
        Board::Nrom | Board::Cnrom => {
            prg = vec![0xff; 0x8000];
            prg[..fixed_enc.bytes.len()].copy_from_slice(&fixed_enc.bytes);
            for (i, v) in vectors.iter().enumerate() {
                prg[0x7ffa + i * 2] = *v as u8;
                prg[0x7ffb + i * 2] = (*v >> 8) as u8;
            }
        }
        Board::Uxrom | Board::Mmc1 | Board::Mmc3 => {
            let fixed_banks = if board == Board::Mmc3 { 2 } else { 1 };
            let switch = max_bank.map_or(0, |b| b as usize + 1);
            let mut banks = (switch + fixed_banks).max(if board == Board::Mmc3 { 4 } else { 2 }).next_power_of_two();
            if let Some(kib) = cart.prg_kib {
                banks = banks.max(kib as usize * 1024 / bank_size);
            }
            if banks > board.max_prg_banks() {
                return Err(vec![format!("{} PRG banks exceed the {} limit of {}", banks, board.name(), board.max_prg_banks())]);
            }
            prg = vec![0xff; banks * bank_size];
            let fixed_at = prg.len() - 0x4000;
            prg[fixed_at..fixed_at + fixed_enc.bytes.len()].copy_from_slice(&fixed_enc.bytes);
            for (b, e) in &bank_encs {
                let at = *b as usize * bank_size;
                if at + bank_size > fixed_at {
                    return Err(vec![format!("Bank {b} overlaps the fixed bank")]);
                }
                prg[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
            }
            let end = prg.len();
            for (i, v) in vectors.iter().enumerate() {
                prg[end - 6 + i * 2] = *v as u8;
                prg[end - 5 + i * 2] = (*v >> 8) as u8;
            }
            if board == Board::Mmc1 {
                // A reset stub at the top of every 16 KiB bank.
                let stub_addr: u16 = 0xBFF0;
                let stub = [0x78, 0xa9, 0x80, 0x8d, 0x00, 0x80, 0x4c, all_labels["__start"] as u8, (all_labels["__start"] >> 8) as u8];
                let banks = prg.len() / 0x4000;
                for b in 0..banks - 1 {
                    let at = b * 0x4000 + 0x3ff0;
                    if prg[at..at + 16].iter().all(|x| *x == 0xff) {
                        prg[at..at + stub.len()].copy_from_slice(&stub);
                        let _ = stub_addr;
                    }
                }
            }
        }
        _ => {
            let banks = max_bank.map_or(1, |b| b as usize + 1).next_power_of_two();
            if banks > board.max_prg_banks() {
                return Err(vec![format!("{} PRG banks exceed the {} limit", banks, board.name())]);
            }
            prg = vec![0xff; banks * 0x8000];
            let common_at = fixed_base as usize - 0x8000;
            for b in 0..banks {
                let at = b * 0x8000;
                prg[at + common_at..at + common_at + fixed_enc.bytes.len()].copy_from_slice(&fixed_enc.bytes);
                if let Some(e) = bank_encs.get(&(b as u32)) {
                    prg[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
                }
                for (i, v) in vectors.iter().enumerate() {
                    prg[at + 0x7ffa + i * 2] = *v as u8;
                    prg[at + 0x7ffb + i * 2] = (*v >> 8) as u8;
                }
            }
        }
    }
    // CHR.
    let mut chr = Vec::new();
    if !chr_ram {
        chr = cart.chr.clone();
        if chr.is_empty() {
            chr = vec![0; 8192];
        }
        let size = chr.len().div_ceil(8192).max(1).next_power_of_two() * 8192;
        if size > board.max_chr() {
            return Err(vec![format!("{} KiB of CHR exceed the {} limit of {} KiB", size / 1024, board.name(), board.max_chr() / 1024)]);
        }
        chr.resize(size, 0);
    }
    let mut header = vec![0u8; 16];
    header[..4].copy_from_slice(b"NES\x1a");
    header[4] = (prg.len() / 16384) as u8;
    header[5] = (chr.len() / 8192) as u8;
    let mapper = board.ines();
    let mirroring = match cart.mirroring.as_str() {
        "vertical" => 1,
        "four" => 8,
        _ => 0,
    };
    header[6] = ((mapper as u8 & 0x0f) << 4) | mirroring | if cart.battery { 2 } else { 0 };
    header[7] = mapper as u8 & 0xf0;
    let mut rom = header;
    rom.extend_from_slice(&prg);
    rom.extend_from_slice(&chr);

    // ------------------------------------------------------------ report
    for (i, inst) in mir.insts.iter().enumerate() {
        if let Some(a) = all_labels.get(&fn_label(InstId(i as u32))) {
            symbols.push(Symbol {
                name: inst.name.clone(),
                address: *a,
                size: 0,
                kind: format!("fn {:?}", inst.ctx).to_lowercase(),
                bank: inst.bank,
            });
        }
    }
    for (i, o) in mir.objs.iter().enumerate() {
        if o.kind == ObjKind::Rom {
            if let Some(a) = all_labels.get(&format!("__o{i}")) {
                symbols.push(Symbol {
                    name: o.name.clone(),
                    address: *a,
                    size: o.bytes(),
                    kind: format!("const {}{}", o.sc.name(), o.len.map(|n| format!("[{n}]")).unwrap_or_default()),
                    bank: o.bank,
                });
            }
        }
    }
    symbols.sort_by_key(|s| (s.bank, s.address));
    let mut layout = format!(
        "{} (iNES mapper {}), PRG {} KiB, CHR {} {} KiB\nfixed code/data {} / {} bytes at ${:04X}\nzero page {} / 256 bytes · RAM {} bytes · frames {} bytes · stack {} bytes\n",
        board.name(),
        board.ines(),
        prg.len() / 1024,
        if chr_ram { "RAM" } else { "ROM" },
        if chr_ram { 8 } else { chr.len() / 1024 },
        fixed_enc.bytes.len(),
        fixed_cap,
        fixed_base,
        out.zp_used,
        out.ram_used,
        frame_bytes,
        stack
    );
    for (b, e) in &bank_encs {
        layout.push_str(&format!("bank {b}: {} / {} bytes at ${:04X}\n", e.bytes.len(), if board.whole() { fixed_base as usize - 0x8000 } else { bank_size }, bank_trials[b]));
        out.banks_used.insert(*b, e.bytes.len());
    }
    layout.push('\n');
    for s in &symbols {
        layout.push_str(&format!(
            "{}${:04X} {:>5} {:<10} {}\n",
            s.bank.map(|b| format!("{b:>2}:")).unwrap_or_else(|| "   ".into()),
            s.address,
            s.size,
            s.kind,
            s.name
        ));
    }
    out.listing = fixed_enc.listing.into_iter().map(|l| (None, l)).collect();
    for (b, e) in bank_encs {
        out.listing.extend(e.listing.into_iter().map(|l| (Some(b), l)));
    }
    out.fixed_used = fixed_enc.bytes.len();
    out.fixed_size = fixed_cap;
    out.prg_bytes = prg.len();
    out.chr_bytes = chr.len();
    out.rom = rom;
    out.symbols = symbols;
    out.labels = all_labels;
    out.layout = layout;
    out.var_addr = var_addr;
    out.obj_addr = obj_addr;
    if !errors.is_empty() {
        return Err(errors);
    }
    let _ = Ins::imp(Op::Nop);
    let _ = Arg::Imp;
    Ok(out)
}

fn place_obj(map: &mut BTreeMap<(ObjId, bool), u16>, id: ObjId, o: &Obj, addr: u16) {
    map.insert((id, false), addr);
    if o.sc.wide() {
        let len = o.len.unwrap_or(1) as u16;
        // Arrays: the high table follows the low table. Scalars: little endian.
        map.insert((id, true), if o.len.is_some() { addr + len } else { addr + 1 });
    }
}
