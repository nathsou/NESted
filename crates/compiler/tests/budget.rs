//! `@budget(n)` verifies a worst-case cycle bound on the linked code.
use nested_compiler::{compile, Options, Project};

fn build(src: &str) -> Result<(), String> {
    compile(&Project::single(src), Options::default())
        .map(|_| ())
        .map_err(|f| f.render())
}

#[test]
fn straight_line_code_within_budget() {
    build("var a: u8 = 0; var b: u8 = 0;\n@budget(80) fn step() { a += 1; if a > 10 { b = a; } }\nfn update() { step(); }").unwrap();
}

#[test]
fn over_budget_reports_the_bound() {
    let e =
        build("var a: u8 = 0;\n@budget(4) fn step() { a += 1; a ^= 3; }\nfn update() { step(); }")
            .unwrap_err();
    assert!(e.contains("over its @budget(4)"), "{e}");
}

#[test]
fn callees_count_and_loops_are_rejected() {
    build("var a: u8 = 0;\n@noinline fn inc() { a += 1; }\n@budget(200) fn step() { inc(); inc(); }\nfn update() { step(); }").unwrap();
    let e = build("var t: [u8; 8] = [0; 8];\n@budget(500) fn clear() { for i in 0..8 { t[i] = 0; } }\nfn update() { clear(); }").unwrap_err();
    assert!(e.contains("loop"), "{e}");
}
