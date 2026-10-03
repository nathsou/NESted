//! Embed the compiler without depending on an emulator or filesystem loader.
use nested_compiler::{compile, Assets, Options};

fn main() {
    let source = r#"
        cartridge "Embedded counter" { mapper = nrom; region = ntsc; }
        var frames: u8 = 0;
        fn update() {
            frames += 1;
            // APU square wave: the ROM really programs the NES sound hardware.
            if (frames & 31) == 0 { tone(0, 213, 6); }
        }
    "#;
    match compile(source, &Assets::new(), Options::default()) {
        Ok(cartridge) => println!(
            "{}-byte ROM, {} bytes of code/data, {} bytes of compiler RAM, {} inspectable passes",
            cartridge.rom.len(),
            cartridge.assembly.bytes.len(),
            cartridge.generated.ram_bytes,
            cartridge.passes.len()
        ),
        Err(errors) => {
            eprintln!(
                "{}",
                nested_compiler::lsp::diagnostics(source, &errors).stringify()
            );
            std::process::exit(1);
        }
    }
}
