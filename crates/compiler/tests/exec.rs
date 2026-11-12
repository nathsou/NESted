//! Compile programs, run them on the emulator and compare with the
//! compile-time interpreter.
use nested_compiler::{compile, Options, Project};
use nested_runtime::Emulator;

pub struct Run {
    pub out: nested_compiler::Output,
    pub emu: Emulator,
}

impl Run {
    pub fn new(src: &str, optimize: bool) -> Run {
        let out = compile(&Project::single(src), Options { optimize })
            .unwrap_or_else(|e| panic!("{}", e.render()));
        let mut emu = Emulator::new(out.rom.clone(), 44100.0).unwrap();
        for _ in 0..8 {
            emu.frame(0).unwrap();
        }
        Run { out, emu }
    }
    pub fn addr(&self, name: &str) -> u16 {
        self.out
            .linked
            .symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no symbol {name}"))
            .address
    }
    pub fn byte(&mut self, name: &str) -> u8 {
        let a = self.addr(name);
        self.emu.read_ram(a)
    }
    pub fn word(&mut self, name: &str) -> u16 {
        let a = self.addr(name);
        self.emu.read_ram(a) as u16 | (self.emu.read_ram(a + 1) as u16) << 8
    }
}

/// Run `body` (statements computing into `out: [u16; N]`) both ways.
fn check(decls: &str, n: usize) {
    let src = format!("var out: [u16; {n}] = [0; {n}];\n{decls}\nfn init() {{ run(); }}\n");
    let reference = {
        let a = nested_compiler::analyze(&Project::single(&src));
        assert!(
            !a.diagnostics.has_errors(),
            "{:?}",
            a.diagnostics
                .items
                .iter()
                .map(|d| d.render(&a.sources))
                .collect::<Vec<_>>()
        );
        let mut env = nested_compiler::consteval::ProgramEnv::new(&a.program);
        let run = a
            .program
            .functions
            .iter()
            .position(|f| f.name == "run")
            .unwrap();
        let mut interp = nested_compiler::consteval::Interp::new(&mut env);
        interp.mutable_globals = true;
        interp
            .call(
                nested_compiler::hir::FnId(run as u32),
                vec![],
                Default::default(),
            )
            .unwrap();
        let writes = interp.global_writes.clone();
        let g = a
            .program
            .globals
            .iter()
            .position(|g| g.name == "out")
            .unwrap();
        let v = writes
            .iter()
            .rev()
            .find(|(x, _)| x.0 as usize == g)
            .map(|(_, v)| v.clone())
            .unwrap();
        v.items()
            .iter()
            .map(|x| x.int() as u16)
            .collect::<Vec<u16>>()
    };
    for optimize in [false, true] {
        let mut r = Run::new(&src, optimize);
        let base = r.addr("out");
        let actual: Vec<u16> = (0..n as u16)
            .map(|i| {
                r.emu.read_ram(base + i) as u16 | (r.emu.read_ram(base + n as u16 + i) as u16) << 8
            })
            .collect();
        assert_eq!(
            actual,
            reference,
            "optimize={optimize}\n{}",
            r.out.passes.iter().find(|p| p.name == "6502").unwrap().text
        );
    }
}

#[test]
fn arithmetic8() {
    check(
        "var a: u8 = 200; var b: u8 = 100; var s: i8 = -5;
         fn run() {
            out[0] = u16(a + b); out[1] = u16(a - b); out[2] = u16(a & b); out[3] = u16(a | b);
            out[4] = u16(a ^ b); out[5] = u16(a << 1); out[6] = u16(a >> 3); out[7] = u16(a * 3);
            out[8] = u16(a / 7); out[9] = u16(a % 7); out[10] = u16(-a); out[11] = u16(~a);
            out[12] = u16(i16(s) * 3); out[13] = u16(i16(s >> 1)); out[14] = u16(i16(s / 2)); out[15] = u16(i16(s % 3));
            out[16] = u16(b * a);
         }",
        17,
    );
}

#[test]
fn arithmetic16() {
    check(
        "var a: u16 = 50000; var b: u16 = 1234; var s: i16 = -3000;
         fn run() {
            out[0] = a + b; out[1] = a - b; out[2] = a & b; out[3] = a | b; out[4] = a ^ b;
            out[5] = a << 3; out[6] = a >> 5; out[7] = b * 7; out[8] = a / b; out[9] = a % b;
            out[10] = u16(s * 3); out[11] = u16(s >> 2); out[12] = u16(s / 7); out[13] = u16(s % 7);
            out[14] = a >> 8; out[15] = u16(lo(a)) + u16(hi(b));
            out[16] = u16(u8(a)); out[17] = b * b;
         }",
        18,
    );
}

#[test]
fn control_flow() {
    check(
        "fn fib(n: u8) -> u16 { var a: u16 = 0; var b: u16 = 1; for i in 0..n { let t = a + b; a = b; b = t; } return a; }
         fn classify(x: u8) -> u8 { return match x { 0 => 10, 1..=9 => 20, 10 | 20 | 30 => 30, _ => 40 }; }
         fn run() {
            out[0] = fib(20); out[1] = fib(1); out[2] = fib(0);
            var k: u8 = 0;
            for i in 0..10 { out[3] += u16(classify(k)); k += 7; }
            var w: u16 = 0;
            while w < 1000 { w += 77; if w > 500 && w < 600 { break; } }
            out[4] = w;
            var c: u8 = 0;
            loop { c += 1; if c == 5 { continue; } if c >= 9 { break; } out[5] += u16(c); }
            for i in (0..5).rev() { out[6] = out[6] * 10 + u16(i); }
            for i in 250..=255 { out[7] += 1; }
            var s: i8 = -3;
            if s < 2 { out[8] = 1; }
            if s >= -3 { out[9] = 1; }
            if s > -3 { out[10] = 1; }
            let big: i16 = -1000;
            if big < 10 { out[11] = 1; }
            if a16() > 40000 { out[12] = 1; }
         }
         fn a16() -> u16 { return 45000; }",
        13,
    );
}

#[test]
fn structs_and_arrays() {
    check(
        "struct P { x: u8, y: u16, alive: bool }
         enum K { A, B, C }
         var ps: [P; 4] = [P { x: 1, y: 300, alive: true }; 4];
         var grid: [[u8; 8]; 8] = [[0; 8]; 8];
         var kinds: [K; 3] = [K::A, K::B, K::C];
         const TABLE: [u16; 4] = [1000, 2000, 3000, 4000];
         const BYTES: [u8; 5] = [9, 8, 7, 6, 5];
         fn make(i: u8) -> P { return P { x: i * 2, y: u16(i) * 100, alive: i != 2 }; }
         fn sum(s: &[u8]) -> u16 { var t: u16 = 0; for i in 0..len(s) { t += u16(s[i]); } return t; }
         fn run() {
            for i in 0..4 { ps[i] = make(i); }
            for i in 0..4 { out[i] = ps[i].y + u16(ps[i].x); if ps[i].alive { out[4] += 1; } }
            for y in 0..8 { for x in 0..8 { grid[y][x] = y * 8 + x; } }
            out[5] = u16(grid[7][3]) + u16(grid[2][6]);
            out[6] = TABLE[2] + TABLE[3];
            var j: u8 = 1;
            out[7] = TABLE[j & 3];
            out[8] = sum(BYTES);
            out[9] = sum(\"HELLO\");
            match kinds[1] { K::A => { out[10] = 1; } K::B => { out[10] = 2; } K::C => { out[10] = 3; } }
            let p = make(3);
            out[11] = p.y;
         }",
        12,
    );
}

#[test]
fn recursion() {
    check(
        "@recursive(12) fn f(n: u8) -> u16 { if n < 2 { return u16(n); } return f(n - 1) + f(n - 2); }
         fn run() { out[0] = f(10); }",
        1,
    );
}

#[test]
fn recursion_with_loops() {
    check(
        "var count: u16 = 0;
         var stack: [u8; 64] = [0; 64];
         var sp: u8 = 0;
         @recursive(8) fn walk(depth: u8) {
            let first = sp;
            let flag = depth & 1;
            for k in 0..3 { stack[raw sp] = k; sp += 1; }
            let last = sp;
            var i: u8 = first;
            while i < last {
                let v = stack[raw i];
                if depth <= 1 {
                    count += 1;
                } else {
                    walk(depth - 1);
                }
                count += u16(v) + u16(flag);
                i += 1;
            }
            sp = first;
         }
         fn run() { walk(4); out[0] = count; out[1] = u16(sp); }",
        2,
    );
}
