use nested_compiler::{asset_paths, compile, json::Json, lsp::LanguageServer, Options};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    process,
};
fn main() {
    if let Err(e) = run() {
        eprintln!("nested: {e}");
        process::exit(1);
    }
}
const HELP: &str = "NESted — NES-only Rust/WASM toolchain

  nested build game.nst [-o game.nes] [--emit dir] [--no-opt]
  nested check game.nst
  nested lsp
  nested run game.nes [frames] [screenshot.ppm]
  nested --version";

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref().unwrap_or("help") {
        "build" => build(&mut args, true)?,
        "check" => build(&mut args, false)?,
        "lsp" => lsp()?,
        "run" => execute_rom(&mut args)?,
        "--version" | "version" => println!("NESted {}", env!("CARGO_PKG_VERSION")),
        _ => println!("{HELP}"),
    }
    Ok(())
}

fn build(args: &mut impl Iterator<Item = String>, write_rom: bool) -> Result<(), String> {
    let path = PathBuf::from(args.next().ok_or("Expected a cartridge source path")?);
    let mut output = path.with_extension("nes");
    let mut emit = None;
    let mut optimize = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => output = PathBuf::from(args.next().ok_or("-o requires a path")?),
            "--emit" => {
                emit = Some(PathBuf::from(
                    args.next().ok_or("--emit requires a directory")?,
                ))
            }
            "--no-opt" => optimize = false,
            _ => return Err(format!("Unknown option {arg}")),
        }
    }
    let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let assets = load_assets(&path, &source)?;
    let compiled = compile(&source, &assets, Options { optimize }).map_err(|errors| {
        for error in errors {
            let position = nested_compiler::lsp::position(&source, error.span.start);
            eprintln!(
                "{}:{}:{}: {}",
                path.display(),
                position.get("line").usize() + 1,
                position.get("character").usize() + 1,
                error.message
            );
        }
        "Compilation failed".to_owned()
    })?;
    if write_rom {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&output, &compiled.rom).map_err(|e| e.to_string())?;
    }
    if let Some(directory) = emit {
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        for (name, content) in [
            ("assembly.asm", compiled.generated.assembly.as_str()),
            ("listing.txt", compiled.assembly.listing.as_str()),
        ] {
            fs::write(directory.join(name), content).map_err(|e| e.to_string())?;
        }
        fs::write(directory.join("report.json"), compiled.json().stringify())
            .map_err(|e| e.to_string())?;
        for (index, pass) in compiled.passes.iter().enumerate() {
            fs::write(
                directory.join(format!("pass-{:02}.txt", index + 1)),
                &pass.text,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    println!(
        "{} · mapper {} · {} B code/data · {} B compiler RAM · {} B ROM",
        output.display(),
        compiled.generated.mapper.id(),
        compiled.assembly.bytes.len(),
        compiled.generated.ram_bytes,
        compiled.rom.len()
    );
    Ok(())
}

fn execute_rom(args: &mut impl Iterator<Item = String>) -> Result<(), String> {
    let path = args.next().ok_or("Expected a ROM path")?;
    let frames = args
        .next()
        .unwrap_or_else(|| "120".into())
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    let mut emulator =
        nested_runtime::Emulator::new(fs::read(path).map_err(|e| e.to_string())?, 44100.0)?;
    let mut energy = 0.0;
    let mut count = 0;
    for _ in 0..frames {
        for sample in emulator.frame(0)? {
            energy += f64::from(sample * sample);
            count += 1;
        }
    }
    if let Some(path) = args.next() {
        let mut image = b"P6\n256 240\n255\n".to_vec();
        image.extend(emulator.pixels());
        fs::write(path, image).map_err(|e| e.to_string())?;
    }
    let dropped = emulator.read_ram(0x37f);
    let rms = (energy / count.max(1) as f64).sqrt();
    println!("{frames} emulated frames · PC ${:04X} · {count} audio samples · RMS {rms:.5} · dropped VRAM writes {dropped}", emulator.cpu.pc);
    Ok(())
}

fn load_assets(path: &Path, source: &str) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let base = path.parent().unwrap_or(Path::new("."));
    let mut assets = BTreeMap::new();
    for asset in asset_paths(source) {
        if Path::new(&asset).is_absolute() {
            return Err("Asset paths must be relative to the source file".into());
        }
        let data = fs::read(base.join(&asset)).map_err(|e| format!("{asset}: {e}"))?;
        assets.insert(asset, data);
    }
    Ok(assets)
}
fn lsp() -> Result<(), String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut server = LanguageServer::new();
    loop {
        let mut length = None;
        let mut header_bytes = 0;
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Ok(());
            }
            header_bytes += line.len();
            if header_bytes > 8192 {
                return Err("LSP header exceeds 8 KiB".into());
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("Content-Length") {
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| "Invalid Content-Length")?,
                    );
                }
            }
        }
        let length = length.ok_or("Missing Content-Length")?;
        if length > 16 * 1024 * 1024 {
            return Err("LSP message exceeds 16 MiB".into());
        }
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        match Json::parse(&text) {
            Ok(request) => {
                for message in server.handle(request) {
                    let body = message.stringify();
                    write!(output, "Content-Length: {}\r\n\r\n{}", body.len(), body)
                        .map_err(|e| e.to_string())?;
                    output.flush().map_err(|e| e.to_string())?;
                }
            }
            Err(e) => {
                let body = Json::object([
                    ("jsonrpc", "2.0".into()),
                    ("id", Json::Null),
                    (
                        "error",
                        Json::object([("code", Json::Number(-32700.0)), ("message", e.into())]),
                    ),
                ])
                .stringify();
                write!(output, "Content-Length: {}\r\n\r\n{}", body.len(), body)
                    .map_err(|e| e.to_string())?;
                output.flush().map_err(|e| e.to_string())?;
            }
        }
        if server.exit {
            return Ok(());
        }
    }
}
