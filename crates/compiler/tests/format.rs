use nested_compiler::fmt::{format, Style};
use nested_compiler::lexer::{lex, Tok};

fn tokens(src: &str) -> Vec<Tok> {
    let mut d = Default::default();
    lex(Default::default(), src, &mut d).tokens.into_iter().map(|t| match t.tok {
        Tok::Asm(body) => Tok::Asm(body.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")),
        t => t,
    }).collect()
}

pub fn check_file(name: &str, src: &str) {
    let out = format(src, &Style::default());
    assert_eq!(tokens(src), tokens(&out), "{name}: formatting changed tokens\n{out}");
    let again = format(&out, &Style::default());
    assert_eq!(out, again, "{name}: formatting is not idempotent");
}

#[test]
fn standard_library() {
    for (name, src) in nested_compiler::stdlib::FILES {
        check_file(name, src);
    }
}

#[test]
fn shapes() {
    let src = "struct P{x:u8,y:u8}\nenum K:u8{A=1,B}\nfn f(a:u8)->u8{let p=P{x:1,y:a};if a>3&&-a<2{return p.x;}else{return match a{0=>1,1|2=>2,_=>3};}}\n";
    let out = format(src, &Style::default());
    assert_eq!(out, "struct P {\n    x: u8,\n    y: u8,\n}\nenum K: u8 {\n    A = 1,\n    B,\n}\nfn f(a: u8) -> u8 {\n    let p = P { x: 1, y: a };\n    if a > 3 && -a < 2 {\n        return p.x;\n    } else {\n        return match a {\n            0 => 1,\n            1 | 2 => 2,\n            _ => 3,\n        };\n    }\n}\n");
}
