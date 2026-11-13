//! Print the passes for a source file: `cargo run --example dump -- file.nst [pass]`.
use nested_compiler::{compile, Options, Project};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&args[1]).unwrap();
    let mut p = Project::single(&src);
    let dir = std::path::Path::new(&args[1])
        .parent()
        .unwrap()
        .to_path_buf();
    for e in walk(&dir) {
        let rel = e
            .strip_prefix(&dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.ends_with(".nst") {
            p.sources.insert(rel, std::fs::read_to_string(&e).unwrap());
        } else {
            p.assets.insert(rel, std::fs::read(&e).unwrap());
        }
    }
    p.sources.insert("main.nst".into(), src);
    match compile(&p, Options::default()) {
        Ok(out) => {
            for pass in &out.passes {
                if args.get(2).is_none_or(|n| *n == pass.name) {
                    println!("==== {}\n{}", pass.name, pass.text);
                }
            }
        }
        Err(e) => eprintln!("{}", e.render()),
    }
}
fn walk(d: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
