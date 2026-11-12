//! Randomized differential testing: generated programs must compute the
//! same results on the emulator as in the reference interpreter.
use nested_compiler::{compile, Options, Project};
use nested_runtime::Emulator;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

const TYPES: [&str; 4] = ["u8", "i8", "u16", "i16"];

fn literal(r: &mut Rng, t: &str) -> String {
    let v: i64 = match t {
        "u8" => r.below(256) as i64,
        "i8" => r.below(256) as i64 - 128,
        "u16" => r.below(65536) as i64,
        _ => r.below(65536) as i64 - 32768,
    };
    let v = if r.below(3) == 0 { [0, 1, 2, 7, 8, 127, 128, 255][r.below(8) as usize].min(if t.starts_with('u') { 65535 } else { 127 }) } else { v };
    let v = match t {
        "u8" => v.clamp(0, 255),
        "i8" => v.clamp(-128, 127),
        "u16" => v.clamp(0, 65535),
        _ => v.clamp(-32768, 32767),
    };
    if v < 0 {
        format!("({v})")
    } else {
        format!("{v}")
    }
}

fn expr(r: &mut Rng, t: &str, depth: u32) -> String {
    if depth > 1 && r.below(9) == 0 {
        // Calls to the helper functions (small ones are inlined).
        let k = r.below(3);
        return format!("{t}(h{k}_{}({}, {}))", "x", expr(r, "u8", depth - 1), expr(r, "u16", depth - 1));
    }
    if depth == 0 || r.below(4) == 0 {
        return match r.below(4) {
            0 => format!("{t}({})", literal(r, t)),
            1 => format!("g_{t}[{}]", r.below(4)),
            2 => format!("l_{t}"),
            _ => format!("g_{t}[i & 3]"),
        };
    }
    match r.below(10) {
        0..=5 => {
            let op = *r.pick(&["+", "-", "&", "|", "^", "*", "+", "-"]);
            format!("({} {op} {})", expr(r, t, depth - 1), expr(r, t, depth - 1))
        }
        6 => {
            let op = *r.pick(&["<<", ">>"]);
            format!("({} {op} {})", expr(r, t, depth - 1), r.below(if t.ends_with('8') { 9 } else { 17 }))
        }
        7 => {
            // Division by a nonzero constant.
            let op = *r.pick(&["/", "%"]);
            let d = 1 + r.below(if t.ends_with('8') { 100 } else { 3000 });
            format!("({} {op} {d})", expr(r, t, depth - 1))
        }
        8 => {
            let from = *r.pick(&TYPES);
            format!("{t}({})", expr(r, from, depth - 1))
        }
        _ => {
            let op = if t.starts_with('i') { *r.pick(&["-", "~"]) } else { "~" };
            format!("{op}{}", expr(r, t, depth - 1))
        }
    }
}

fn cond(r: &mut Rng, depth: u32) -> String {
    let t = *r.pick(&TYPES);
    let op = *r.pick(&["<", "<=", ">", ">=", "==", "!="]);
    let c = format!("{} {op} {}", expr(r, t, depth), expr(r, t, depth));
    match r.below(5) {
        0 => format!("({c}) && ({})", cond(r, 0)),
        1 => format!("({c}) || ({})", cond(r, 0)),
        2 => format!("!({c})"),
        _ => c,
    }
}

fn program(seed: u64) -> (String, usize) {
    let mut r = Rng(seed * 0x9e3779b97f4a7c15 + 1);
    let mut src = String::new();
    for t in TYPES {
        let vals: Vec<String> = (0..4).map(|_| literal(&mut r, t)).collect();
        src.push_str(&format!("var g_{t}: [{t}; 4] = [{}];\n", vals.join(", ")));
    }
    let n = 24;
    // Helpers: h0 is tiny, h1 has control flow, h2 writes a global.
    src.push_str("fn h0_x(a: u8, b: u16) -> u16 { return u16(a) + b; }\n");
    src.push_str("fn h1_x(a: u8, b: u16) -> u16 { if a > 100 { return b - u16(a); } var t = b; for k in 0..(a & 3) { t = t * 3 + u16(k); } return t; }\n");
    src.push_str("fn h2_x(a: u8, b: u16) -> u16 { g_u8[a & 3] += 1; return b ^ u16(g_u8[0]); }\n");
    src.push_str(&format!("var out: [u16; {n}] = [0; {n}];\nfn run() {{\n"));
    for t in TYPES {
        src.push_str(&format!("    var l_{t}: {t} = {};\n", literal(&mut r, t)));
    }
    src.push_str("    var i: u8 = 0;\n");
    for k in 0..n {
        let t = *r.pick(&TYPES);
        match r.below(4) {
            0 => src.push_str(&format!("    if {} {{ out[{k}] = u16({}); }} else {{ out[{k}] = 7; }}\n", cond(&mut r, 2), expr(&mut r, t, 3))),
            1 => {
                src.push_str(&format!("    l_{t} = {};\n", expr(&mut r, t, 3)));
                src.push_str(&format!("    out[{k}] = u16(l_{t});\n"));
            }
            2 => {
                src.push_str(&format!("    for j in 0..{} {{ i += 1; g_{t}[i & 3] = {}; }}\n", 1 + r.below(5), expr(&mut r, t, 2)));
                src.push_str(&format!("    out[{k}] = u16(g_{t}[{}]) + u16(i);\n", r.below(4)));
            }
            _ => src.push_str(&format!("    out[{k}] = u16({});\n", expr(&mut r, t, 4))),
        }
    }
    src.push_str("}\nfn init() { run(); }\n");
    (src, n)
}

fn reference(src: &str, n: usize) -> Vec<u16> {
    let a = nested_compiler::analyze(&Project::single(src));
    assert!(!a.diagnostics.has_errors(), "{}\n{}", src, a.diagnostics.items.iter().map(|d| d.render(&a.sources)).collect::<Vec<_>>().join("\n"));
    let mut env = nested_compiler::consteval::ProgramEnv::new(&a.program);
    let run = a.program.functions.iter().position(|f| f.name == "run").unwrap();
    let mut interp = nested_compiler::consteval::Interp::new(&mut env);
    interp.mutable_globals = true;
    interp.call(nested_compiler::hir::FnId(run as u32), vec![], Default::default()).unwrap();
    let g = a.program.globals.iter().position(|g| g.name == "out").unwrap();
    let v = interp.global_writes.iter().rev().find(|(x, _)| x.0 as usize == g).map(|(_, v)| v.clone()).unwrap();
    let out: Vec<u16> = v.items().iter().map(|x| x.int() as u16).collect();
    assert_eq!(out.len(), n);
    out
}

fn emulate(src: &str, n: usize, optimize: bool) -> Vec<u16> {
    let out = compile(&Project::single(src), Options { optimize }).unwrap_or_else(|e| panic!("{src}\n{}", e.render()));
    let mut emu = Emulator::new(out.rom.clone(), 44100.0).unwrap();
    for _ in 0..30 {
        emu.frame(0).unwrap();
    }
    let base = out.linked.symbols.iter().find(|s| s.name == "out").unwrap().address;
    (0..n as u16).map(|i| emu.read_ram(base + i) as u16 | (emu.read_ram(base + n as u16 + i) as u16) << 8).collect()
}

#[test]
fn random_programs() {
    let count: u64 = std::env::var("FUZZ").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let first: u64 = std::env::var("FUZZ_FROM").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    for seed in first..count {
        eprintln!("seed {seed}");
        let (src, n) = program(seed);
        if std::env::var("FUZZ_SHOW").is_ok_and(|v| v == seed.to_string()) {
            println!("{src}");
        }
        let expected = reference(&src, n);
        for optimize in [false, true] {
            let got = emulate(&src, n, optimize);
            if got != expected {
                let diff: Vec<usize> = (0..n).filter(|i| got[*i] != expected[*i]).collect();
                let lines: Vec<&str> = src.lines().collect();
                panic!(
                    "seed {seed} optimize={optimize}: outputs {diff:?} differ\nexpected {expected:?}\ngot      {got:?}\n{}",
                    lines.join("\n")
                );
            }
        }
    }
}
