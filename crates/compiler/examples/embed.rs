//! Compile a two-file project held in memory: `cargo run -p nested-compiler --example embed`.
use nested_compiler::{compile, Options, Project};

fn main() {
    let mut project = Project::single(
        "cartridge { title = \"Embed\"; mapper = nrom; chr = \"tiles.chr\"; }
use std::video;
use counter;
fn update() {
    counter::tick();
    video::sprite(counter::value(), 100, 1, 0);
}",
    );
    project.sources.insert(
        "counter.nst".into(),
        "var n: u8 = 0;
pub fn tick() { n += 1; }
pub fn value() -> u8 { return n; }"
            .into(),
    );
    project.assets.insert("tiles.chr".into(), vec![0; 8192]);

    match compile(&project, Options::default()) {
        Ok(out) => {
            assert_eq!(&out.rom[..4], b"NES\x1a");
            println!("{} bytes for {}", out.rom.len(), out.board.name());
            for pass in &out.passes {
                println!("pass {}: {} lines", pass.name, pass.text.lines().count());
            }
            for s in out
                .linked
                .symbols
                .iter()
                .filter(|s| s.kind.starts_with("fn"))
            {
                println!("  ${:04X} {:>4} bytes  {}", s.address, s.size, s.name);
            }
        }
        Err(failure) => eprintln!("{}", failure.render()),
    }
}
