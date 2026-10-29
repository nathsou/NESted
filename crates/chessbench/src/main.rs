//! Headless driver for the NESted Chess cartridge.
//!
//! The cartridge exposes a RAM mailbox (`io_cmd`, `io_arg`, `io_out`) that the
//! console itself never writes. This tool compiles the cartridge source,
//! executes the ROM on a lean 6502 core and drives that mailbox to run perft,
//! fixed-time searches and complete games through a UCI front end.
mod cpu;
mod ui;

use cpu::Machine;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const CMD_NEW_GAME: u8 = 1;
const CMD_PLAY: u8 = 2;
const CMD_THINK: u8 = 3;
const CMD_PERFT: u8 = 4;
const CMD_SETUP: u8 = 5;
const CMD_EVALUATE: u8 = 6;

thread_local! {
    static LAST_ROM: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub struct Cartridge {
    pub m: Machine,
    symbols: BTreeMap<String, u16>,
    /// Address -> (instruction text, source span) for profiling.
    listing: BTreeMap<u16, (String, Option<(usize, usize)>)>,
    source: String,
}

pub struct Thought {
    pub from: u8,
    pub to: u8,
    pub score: i16,
    pub depth: u8,
    pub nodes: u32,
    pub frames: u16,
    pub cycles: u64,
}

impl Cartridge {
    pub fn load(source_path: &Path) -> Result<Self, String> {
        let source = std::fs::read_to_string(source_path).map_err(|e| e.to_string())?;
        let base = source_path.parent().unwrap_or(Path::new("."));
        let mut assets = BTreeMap::new();
        for asset in nested_compiler::asset_paths(&source) {
            let data = std::fs::read(base.join(&asset)).map_err(|e| format!("{asset}: {e}"))?;
            assets.insert(asset, data);
        }
        let compiled = nested_compiler::compile(&source, &assets, Default::default())
            .map_err(|d| format!("{:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>()))?;
        let mut m = Machine::new(&compiled.rom)?;
        LAST_ROM.with(|r| *r.borrow_mut() = compiled.rom.clone());
        // Boot: run init and settle into the frame loop.
        for _ in 0..8 {
            m.run_frame();
        }
        Ok(Self {
            m,
            symbols: compiled.assembly.symbols.clone(),
            listing: compiled
                .assembly
                .instructions
                .iter()
                .map(|i| (i.address, (i.text.clone(), i.source)))
                .collect(),
            source,
        })
    }
    pub fn symbol(&self, name: &str) -> u16 {
        *self
            .symbols
            .get(&format!("__v_{name}"))
            .unwrap_or_else(|| panic!("missing symbol {name}"))
    }
    fn command(&mut self, cmd: u8, args: &[u8], max_frames: u64) -> Result<(), String> {
        let arg = self.symbol("io_arg");
        for (i, a) in args.iter().enumerate() {
            self.m.poke(arg + i as u16, *a);
        }
        let c = self.symbol("io_cmd");
        self.m.poke(c, cmd);
        for _ in 0..max_frames {
            self.m.run_frame();
            if self.m.peek(c) == 0 {
                return Ok(());
            }
        }
        Err(format!("command {cmd} did not complete"))
    }
    fn out(&self, i: u16) -> u8 {
        self.m.peek(self.symbol("io_out") + i)
    }
    pub fn new_game(&mut self) -> Result<(), String> {
        self.command(CMD_NEW_GAME, &[], 10)
    }
    /// Plays a move given in UCI notation; returns whether it was legal.
    pub fn play(&mut self, uci: &str) -> Result<bool, String> {
        let b = uci.as_bytes();
        if b.len() < 4 {
            return Err(format!("bad move {uci}"));
        }
        let sq = |f: u8, r: u8| (r - b'1') * 16 + (f - b'a');
        let promo = match b.get(4) {
            Some(b'n') => 2,
            Some(b'b') => 3,
            Some(b'r') => 4,
            Some(b'q') => 5,
            _ => 0,
        };
        self.command(CMD_PLAY, &[sq(b[0], b[1]), sq(b[2], b[3]), promo], 10)?;
        Ok(self.out(0) == 1)
    }
    pub fn setup_fen(&mut self, fen: &str) -> Result<(), String> {
        let mut parts = fen.split_whitespace();
        let placement = parts.next().ok_or("empty FEN")?;
        let board = self.symbol("board");
        for i in 0..128 {
            self.m.poke(board + i, 0);
        }
        let (mut rank, mut file) = (7i32, 0i32);
        for c in placement.chars() {
            match c {
                '/' => {
                    rank -= 1;
                    file = 0;
                }
                '1'..='8' => file += c as i32 - '0' as i32,
                _ => {
                    let t = match c.to_ascii_lowercase() {
                        'p' => 1,
                        'n' => 2,
                        'b' => 3,
                        'r' => 4,
                        'q' => 5,
                        'k' => 6,
                        _ => return Err(format!("bad FEN piece {c}")),
                    };
                    let color = if c.is_ascii_lowercase() { 8 } else { 0 };
                    self.m.poke(board + (rank * 16 + file) as u16, t | color);
                    file += 1;
                }
            }
        }
        let side = if parts.next() == Some("b") { 8 } else { 0 };
        let mut castle = 0;
        for c in parts.next().unwrap_or("-").chars() {
            castle |= match c {
                'K' => 1,
                'Q' => 2,
                'k' => 4,
                'q' => 8,
                _ => 0,
            };
        }
        let ep = match parts.next() {
            Some(s) if s.len() == 2 => {
                let b = s.as_bytes();
                (b[1] - b'1') * 16 + (b[0] - b'a')
            }
            _ => 128,
        };
        self.command(CMD_SETUP, &[side, castle, ep], 10)
    }
    /// (side-relative eval, white mg, white eg, phase)
    pub fn evaluate(&mut self) -> Result<(i16, i16, i16, u8), String> {
        self.command(CMD_EVALUATE, &[], 10)?;
        Ok((
            i16::from_le_bytes([self.out(0), self.out(1)]),
            i16::from_le_bytes([self.out(2), self.out(3)]),
            i16::from_le_bytes([self.out(4), self.out(5)]),
            self.out(6),
        ))
    }
    pub fn perft(&mut self, depth: u8) -> Result<u32, String> {
        self.command(CMD_PERFT, &[depth], 1 << 40)?;
        Ok(u32::from_le_bytes([
            self.out(0),
            self.out(1),
            self.out(2),
            self.out(3),
        ]))
    }
    pub fn think(&mut self, frames: u16, depth: u8) -> Result<Thought, String> {
        self.think_book(frames, depth, false)
    }
    pub fn think_book(&mut self, frames: u16, depth: u8, book: bool) -> Result<Thought, String> {
        let start = self.m.cycles;
        let [lo, hi] = frames.to_le_bytes();
        self.command(CMD_THINK, &[lo, hi, depth, u8::from(book)], u64::from(frames) * 4 + 1_000_000)?;
        Ok(Thought {
            from: self.out(0),
            to: self.out(1),
            score: i16::from_le_bytes([self.out(2), self.out(3)]),
            depth: self.out(4),
            nodes: u32::from_le_bytes([self.out(5), self.out(6), self.out(7), 0]),
            frames: u16::from_le_bytes([self.out(8), self.out(9)]),
            cycles: self.m.cycles - start,
        })
    }
}

fn square(s: u8) -> String {
    let s = s & 0x77;
    format!("{}{}", (b'a' + (s & 7)) as char, (s >> 4) + 1)
}

pub fn uci_move(from: u8, to: u8) -> String {
    let mut m = square(from) + &square(to);
    if from & 0x80 != 0 {
        let kind = (to >> 7) | ((from >> 2) & 2);
        m.push(['q', 'n', 'r', 'b'][kind as usize]);
    }
    m
}

const PERFT_SUITE: &[(&str, &[u32])] = &[
    (
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        &[20, 400, 8902, 197281],
    ),
    (
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        &[48, 2039, 97862],
    ),
    ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", &[14, 191, 2812, 43238]),
    (
        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        &[6, 264, 9467],
    ),
    (
        "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        &[44, 1486, 62379],
    ),
    (
        "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
        &[46, 2079, 89890],
    ),
];

/// Typical game positions for speed benchmarks (opening to endgame).
const BENCH: &[&str] = &[
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r1bqk2r/1pppbppp/p1n2n2/4p3/B3P3/5N2/PPPP1PPP/RNBQ1RK1 w kq - 4 6",
    "rnbqkb1r/ppp2ppp/4pn2/3p4/2PP4/2N5/PP2PPPP/R1BQKBNR w KQkq - 2 4",
    "rnbqkb1r/1p2pppp/p2p1n2/8/3NP3/2N5/PPP2PPP/R1BQKB1R w KQkq - 0 6",
    "r2q1rk1/pp2bppp/2n1pn2/3p4/3P4/2NBPN2/PP3PPP/R2Q1RK1 w - - 0 10",
    "2r2rk1/pp1bqppp/2nbpn2/3p4/3P4/1PNBPN2/PB2QPPP/2R2RK1 w - - 0 14",
    "1k1r4/pp1b1R2/3q2pp/4p3/2B5/4Q3/PPP2B2/2K5 b - - 0 1",
    "3r1k2/4npp1/1ppr3p/p6P/P2PPPP1/1NR5/5K2/2R5 w - - 0 1",
    "r3r1k1/pp3pbp/1qp3p1/2B5/2BP2b1/Q1n2N2/P4PPP/3R1K1R b - - 0 1",
    "8/5pk1/6p1/1r6/5P2/6PK/3R4/8 w - - 0 40",
    "8/8/4k3/3p4/3P4/4K3/8/8 w - - 0 50",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
];

fn main() {
    if let Err(e) = run() {
        eprintln!("chessbench: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let source = PathBuf::from(args.next().unwrap_or_else(|| "games/chess.nst".into()));
    let mut c = Cartridge::load(&source)?;
    match mode.as_str() {
        "perft" => {
            let mut ok = true;
            for (fen, counts) in PERFT_SUITE {
                for (d, expected) in counts.iter().enumerate() {
                    c.setup_fen(fen)?;
                    let start = c.m.cycles;
                    let n = c.perft(d as u8 + 1)?;
                    let secs = (c.m.cycles - start) as f64 / 1_789_773.0;
                    let mark = if n == *expected { "ok" } else { "FAIL" };
                    ok &= n == *expected;
                    println!(
                        "{mark:4} depth {} {n:>8} (expected {expected:>8}) {secs:8.1}s NES  {fen}",
                        d + 1
                    );
                }
            }
            if !ok {
                return Err("perft mismatch".into());
            }
        }
        "bench" => {
            // Fixed-depth searches: deterministic node counts and NES time.
            let depth: u8 = args.next().map(|s| s.parse().unwrap()).unwrap_or(5);
            let frames: u16 = args.next().map(|s| s.parse().unwrap()).unwrap_or(60000);
            let mut total_nodes = 0u64;
            let mut total_cycles = 0u64;
            let wall = Instant::now();
            for fen in BENCH {
                c.setup_fen(fen)?;
                let t = c.think(frames, depth)?;
                total_nodes += u64::from(t.nodes);
                total_cycles += t.cycles;
                println!(
                    "{:6} depth {} score {:6} nodes {:7} {:7.1}s NES  {}",
                    uci_move(t.from, t.to),
                    t.depth,
                    t.score,
                    t.nodes,
                    t.cycles as f64 / 1_789_773.0,
                    fen
                );
            }
            let secs = total_cycles as f64 / 1_789_773.0;
            println!(
                "total nodes {total_nodes} · NES {secs:.1}s · {:.0} nodes/s · {:.0} cycles/node · host {:.1}s",
                total_nodes as f64 / secs,
                total_cycles as f64 / total_nodes as f64,
                wall.elapsed().as_secs_f64()
            );
        }
        "profile" => {
            let depth: u8 = args.next().map(|s| s.parse().unwrap()).unwrap_or(5);
            c.m.profile = Some(vec![0; 65536]);
            c.m.counts = Some(vec![0; 65536]);
            for fen in BENCH {
                c.setup_fen(fen)?;
                c.think(60000, depth)?;
            }
            let profile = c.m.profile.take().unwrap();
            let counts = c.m.counts.take().unwrap();
            let mut functions: Vec<(String, u16)> = c
                .symbols
                .iter()
                .filter(|(n, _)| n.starts_with("__fn_") || n.starts_with("__rt_"))
                .map(|(n, a)| (n.clone(), *a))
                .collect();
            functions.sort_by_key(|f| f.1);
            let total: u64 = profile.iter().sum();
            let mut rows = Vec::new();
            for (i, (name, start)) in functions.iter().enumerate() {
                let end = functions.get(i + 1).map_or(0xfffa, |f| f.1);
                let cycles: u64 = profile[*start as usize..end as usize].iter().sum();
                rows.push((cycles, name.clone(), counts[*start as usize]));
            }
            rows.sort_by(|a, b| b.0.cmp(&a.0));
            if let Some(function) = args.next() {
                // Instruction-level view of one function.
                let start = c.symbols[&format!("__fn_{function}")];
                let end = functions
                    .iter()
                    .map(|f| f.1)
                    .filter(|a| *a > start)
                    .min()
                    .unwrap_or(0xfffa);
                for (address, (text, span)) in c.listing.range(start..end) {
                    let cycles = profile[*address as usize];
                    let src = span
                        .map(|(a, b)| c.source[a..b.min(a + 40)].replace('\n', " "))
                        .unwrap_or_default();
                    println!(
                        "{address:04X} {:>10} {:>8}  {text:32} {src}",
                        cycles, counts[*address as usize]
                    );
                }
                return Ok(());
            }
            for (cycles, name, calls) in rows.iter().take(30) {
                println!(
                    "{:6.2}%  {:>12}  {:>9} calls {:>7.0} cycles/call  {name}",
                    *cycles as f64 * 100.0 / total as f64,
                    cycles,
                    calls,
                    *cycles as f64 / (*calls).max(1) as f64
                );
            }
        }
        "uci" => uci(&mut c)?,
        "ui" => {
            let script = args.next().ok_or("ui needs a script")?;
            let out = PathBuf::from(args.next().unwrap_or_else(|| "ui-shots".into()));
            let rom = LAST_ROM.with(|r| r.borrow().clone());
            let sym = ui::Symbols {
                board: c.symbol("board"),
                io_arg: c.symbol("io_arg"),
                io_cmd: c.symbol("io_cmd"),
            };
            ui::run(&rom, &script, &out, &sym)?;
        }
        "go" => {
            let depth: u8 = args.next().map(|s| s.parse().unwrap()).unwrap_or(3);
            for line in io::stdin().lock().lines() {
                let fen = line.map_err(|e| e.to_string())?;
                c.setup_fen(&fen)?;
                let t = c.think(60000, depth)?;
                println!("{} {} {} {}", uci_move(t.from, t.to), t.depth, t.score, t.nodes);
            }
        }
        "moves" => {
            for line in io::stdin().lock().lines() {
                let fen = line.map_err(|e| e.to_string())?;
                c.setup_fen(&fen)?;
                c.command(7, &[], 10)?;
                let split = c.out(0) as u16;
                let end = c.out(1) as u16;
                let from = c.symbol("move_from");
                let to = c.symbol("move_to");
                let list = |a: u16, b: u16| -> Vec<String> {
                    (a..b).map(|i| uci_move(c.m.peek(from + i), c.m.peek(to + i))).collect()
                };
                println!("{}|{}", list(0, split).join(" "), list(split, end).join(" "));
            }
        }
        "eval" => {
            for line in io::stdin().lock().lines() {
                let fen = line.map_err(|e| e.to_string())?;
                c.setup_fen(&fen)?;
                let (v, mg, eg, phase) = c.evaluate()?;
                println!("{v} {mg} {eg} {phase}");
            }
        }
        _ => {
            println!("usage: chessbench perft|bench [depth]|profile [depth]|uci [source.nst]");
        }
    }
    Ok(())
}

fn uci(c: &mut Cartridge) -> Result<(), String> {
    let stdin = io::stdin();
    let mut out = io::stdout();
    let mut frames: u16 = 600;
    let mut depth: u8 = 0;
    let mut book = true;
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let mut words = line.split_whitespace();
        match words.next() {
            Some("uci") => {
                writeln!(out, "id name NESted Chess").ok();
                writeln!(out, "id author NESted").ok();
                writeln!(out, "option name Frames type spin default 600 min 1 max 60000").ok();
                writeln!(out, "option name Depth type spin default 0 min 0 max 64").ok();
                writeln!(out, "option name OwnBook type check default true").ok();
                writeln!(out, "uciok").ok();
            }
            Some("isready") => {
                writeln!(out, "readyok").ok();
            }
            Some("setoption") => {
                let words: Vec<&str> = line.split_whitespace().collect();
                if let (Some(n), Some(v)) = (
                    words.iter().position(|w| *w == "name"),
                    words.iter().position(|w| *w == "value"),
                ) {
                    let name = words[n + 1..v].join(" ");
                    let value = words.get(v + 1).copied().unwrap_or("0");
                    match name.as_str() {
                        "Frames" => frames = value.parse().unwrap_or(600),
                        "Depth" => depth = value.parse().unwrap_or(0),
                        "OwnBook" => book = value == "true",
                        _ => {}
                    }
                }
            }
            Some("ucinewgame") => c.new_game()?,
            Some("position") => {
                let rest: Vec<&str> = words.collect();
                let moves_at = rest.iter().position(|w| *w == "moves");
                if rest.first() == Some(&"fen") {
                    let end = moves_at.unwrap_or(rest.len());
                    c.setup_fen(&rest[1..end].join(" "))?;
                } else {
                    c.new_game()?;
                }
                if let Some(i) = moves_at {
                    for m in &rest[i + 1..] {
                        if !c.play(m)? {
                            return Err(format!("illegal move {m} in position command"));
                        }
                    }
                }
            }
            Some("go") => {
                let t = c.think_book(frames, depth, book)?;
                writeln!(
                    out,
                    "info depth {} score cp {} nodes {} time {}",
                    t.depth,
                    t.score,
                    t.nodes,
                    t.frames as u64 * 1000 / 60
                )
                .ok();
                if t.from & 0x80 != 0 && t.from == 0x80 {
                    writeln!(out, "bestmove 0000").ok();
                } else {
                    writeln!(out, "bestmove {}", uci_move(t.from, t.to)).ok();
                }
            }
            Some("quit") => break,
            _ => {}
        }
        out.flush().ok();
    }
    Ok(())
}
