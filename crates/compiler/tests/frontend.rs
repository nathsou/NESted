use nested_compiler::{analyze, Project};

fn errors(src: &str) -> Vec<String> {
    let a = analyze(&Project::single(src));
    a.diagnostics
        .items
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.render(&a.sources))
        .collect()
}

fn ok(src: &str) {
    let e = errors(src);
    assert!(e.is_empty(), "{}", e.join("\n"));
}

#[test]
fn basics() {
    ok("var x: u8 = 3; fn update() { x += 1; let y = x + 2; var z: u16 = u16(y) << 3; z -= 1; }");
}

#[test]
fn structs_enums_ranges() {
    ok(r#"
enum Kind { Bat, Slime = 4, Ghost }
struct Enemy { x: u8, y: u8, kind: Kind, hp: u8 }
type Slot = u8 in 0..8;
var enemies: [Enemy; 8] = [Enemy { x: 0, y: 0, kind: Kind::Bat, hp: 3 }; 8];
fn hurt(i: Slot) {
    enemies[i].hp -= 1;
    match enemies[i].kind {
        Kind::Bat => { enemies[i].x += 1; }
        Slime | Ghost => {}
    }
}
fn update() {
    for i in 0..8 { hurt(i); }
    let k: u8 = 7;
    hurt(k & 7);
}
"#);
}

#[test]
fn proofs() {
    let e = errors("var t: [u8; 10] = [0; 10]; fn f(i: u8) -> u8 { return t[i]; }");
    assert!(e[0].contains("Cannot prove index"), "{e:?}");
    ok("var t: [u8; 10] = [0; 10]; fn f(i: u8) -> u8 { if i < 10 { return t[i]; } return 0; }");
    ok("var t: [u8; 10] = [0; 10]; fn f(i: u8) -> u8 { if i >= 10 { return 0; } return t[i]; }");
    ok("var t: [u8; 16] = [0; 16]; fn f(i: u8) -> u8 { return t[i & 15] + t[i % 16] + t[i >> 4]; }");
    ok("var t: [u8; 16] = [0; 16]; fn f(i: u8) -> u8 { return t[raw i]; }");
    let e = errors("type R = u8 in 0..4; fn f(i: u8) -> R { return i; }");
    assert!(e[0].contains("Cannot prove"), "{e:?}");
}

#[test]
fn const_functions() {
    let a = analyze(&Project::single(
        "fn sq(n: u8) -> u16 { return u16(n) * u16(n); }
         fn table() -> [u16; 8] { var t: [u16; 8] = [0; 8]; for i in 0..8 { t[i] = sq(i); } return t; }
         const SQUARES: [u16; 8] = table();
         const N: u16 = SQUARES[7];",
    ));
    let errs: Vec<_> = a
        .diagnostics
        .items
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.render(&a.sources))
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
    let n = a.program.globals.iter().find(|g| g.name == "N").unwrap();
    assert_eq!(n.value.int(), 49);
}

#[test]
fn exhaustive() {
    let e =
        errors("enum D { A, B, C } fn f(d: D) -> u8 { return match d { D::A => 1, D::B => 2 }; }");
    assert!(e[0].contains("does not cover C"), "{e:?}");
    let e = errors("fn f(d: u8) -> u8 { return match d { 0..=100 => 1, 102..=255 => 2 }; }");
    assert!(e[0].contains("does not cover 101"), "{e:?}");
}

#[test]
fn modules() {
    let mut p = Project::single(
        "use util; use util::double; fn update() { let x = util::triple(2); let y = double(x); }",
    );
    p.sources.insert("util.nst".into(), "pub fn double(x: u8) -> u8 { return x * 2; } pub fn triple(x: u8) -> u8 { return x * 3; } fn hidden() {}".into());
    let a = analyze(&p);
    let errs: Vec<_> = a
        .diagnostics
        .items
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.render(&a.sources))
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
    p.sources.insert(
        "main.nst".into(),
        "use util; fn update() { util::hidden(); }".into(),
    );
    let a = analyze(&p);
    assert!(a
        .diagnostics
        .items
        .iter()
        .any(|d| d.message.contains("private")));
}

#[test]
fn slice_length_guards() {
    ok("fn f(s: &[u8]) -> u8 { if len(s) > 2 { return s[0] + s[2]; } return 0; }");
    ok("fn f(s: &[u8]) -> u8 { if len(s) == 0 { return 0; } return s[0]; }");
    ok("fn f(s: &[u8]) -> u8 { if 1 < len(s) { return s[1]; } return 0; }");
    let e = errors("fn f(s: &[u8]) -> u8 { if len(s) > 2 { return s[3]; } return 0; }");
    assert!(e[0].contains("slice length"), "{e:?}");
    ok("var a: [u8; 4] = [0; 4]; fn g(s: &[u8]) -> u16 { return len(s); } fn f() { a[1] = 2; let n = g(a); }");
}
