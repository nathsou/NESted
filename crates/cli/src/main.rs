//! The `nested` command line: build, check, run and format cartridges, and
//! serve the language server over stdio.
use nested_compiler::{compile, Options, Project};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

const HELP: &str = "NESted — a programming language and toolchain for the NES

  nested build <project> [-o game.nes] [--emit dir] [--no-opt]
  nested check <project>
  nested run <project|game.nes> [frames] [--shot out.png] [--hold buttons]
  nested fmt [--check] <file.nst>...
  nested lsp
  nested --version

A project is a directory containing main.nst (with any other modules and
assets beneath it), or a single .nst file. Buttons: a b select start up down
left right, joined with '+'.";

fn main() {
    if let Err(e) = run() {
        eprintln!("nested: {e}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref().unwrap_or("help") {
        "build" => build(&mut args, true),
        "check" => build(&mut args, false),
        "run" => execute(&mut args),
        "fmt" => format_files(&mut args),
        "lsp" => nested_compiler::lsp::serve_stdio(),
        "--version" | "version" => {
            println!("NESted {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

/// Load a project directory (or a single file) into memory.
pub fn load_project(path: &Path) -> Result<(Project, PathBuf), String> {
    let (root, entry) = if path.is_dir() {
        (path.to_path_buf(), "main.nst".to_owned())
    } else {
        let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        (root, path.file_name().unwrap().to_string_lossy().into_owned())
    };
    let mut project = Project {
        entry: entry.clone(),
        ..Default::default()
    };
    fn walk(dir: &Path, base: &Path, p: &mut Project, depth: usize) -> Result<(), String> {
        if depth > 6 {
            return Ok(());
        }
        let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                walk(&path, base, p, depth + 1)?;
                continue;
            }
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if rel.ends_with(".nst") {
                p.sources.insert(rel, fs::read_to_string(&path).map_err(|e| e.to_string())?);
            } else if fs::metadata(&path).map(|m| m.len() <= 1 << 20).unwrap_or(false) {
                p.assets.insert(rel, fs::read(&path).map_err(|e| e.to_string())?);
            }
        }
        Ok(())
    }
    if path.is_dir() {
        walk(&root, &root, &mut project, 0)?;
    } else {
        project.sources.insert(entry, fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?);
        // Assets next to a single file.
        if let Ok(entries) = fs::read_dir(&root) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    let _ = walk(&e.path(), &root, &mut project, 1);
                }
            }
        }
    }
    Ok((project, root))
}

fn build(args: &mut impl Iterator<Item = String>, write: bool) -> Result<(), String> {
    let path = PathBuf::from(args.next().ok_or("Expected a project path")?);
    let mut output = None;
    let mut emit = None;
    let mut optimize = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => output = Some(PathBuf::from(args.next().ok_or("-o needs a path")?)),
            "--emit" => emit = Some(PathBuf::from(args.next().ok_or("--emit needs a directory")?)),
            "--no-opt" => optimize = false,
            a => return Err(format!("Unknown argument '{a}'")),
        }
    }
    let (project, _) = load_project(&path)?;
    let out = compile(&project, Options { optimize }).map_err(|f| f.render())?;
    for w in &out.warnings {
        eprintln!("{}", w.render(&out.analysis.sources));
    }
    if write {
        let target = output.unwrap_or_else(|| {
            if path.is_dir() {
                path.join(format!("{}.nes", path.file_name().unwrap().to_string_lossy()))
            } else {
                path.with_extension("nes")
            }
        });
        fs::write(&target, &out.rom).map_err(|e| e.to_string())?;
        println!(
            "{} → {} ({} bytes, {}, fixed {} / {} bytes, zero page {} bytes, RAM {} bytes)",
            path.display(),
            target.display(),
            out.rom.len(),
            out.board.name(),
            out.linked.fixed_used,
            out.linked.fixed_size,
            out.linked.zp_used,
            out.linked.ram_used
        );
    } else {
        println!("{}: OK", path.display());
    }
    if let Some(dir) = emit {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for p in &out.passes {
            let name = p.name.to_lowercase().replace(' ', "-");
            fs::write(dir.join(format!("{name}.txt")), &p.text).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn buttons(spec: &str) -> u8 {
    spec.split('+')
        .map(|b| match b.trim() {
            "a" => 1,
            "b" => 2,
            "select" => 4,
            "start" => 8,
            "up" => 16,
            "down" => 32,
            "left" => 64,
            "right" => 128,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

fn execute(args: &mut impl Iterator<Item = String>) -> Result<(), String> {
    let path = PathBuf::from(args.next().ok_or("Expected a project or ROM")?);
    let mut frames = 120;
    let mut shot = None;
    let mut hold = 0;
    let mut watch: Vec<String> = Vec::new();
    let mut script: Vec<(u64, u8)> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--input" => {
                for part in args.next().ok_or("--input needs button@frame,...")?.split(',') {
                    let (b, f) = part.split_once('@').ok_or("expected button@frame")?;
                    script.push((f.parse().map_err(|_| "bad frame")?, buttons(b)));
                }
            }
            "--watch" => watch = args.next().ok_or("--watch needs names")?.split(',').map(str::to_owned).collect(),
            "--shot" => shot = Some(PathBuf::from(args.next().ok_or("--shot needs a path")?)),
            "--hold" => hold = buttons(&args.next().ok_or("--hold needs buttons")?),
            n => frames = n.parse().map_err(|_| format!("Unknown argument '{n}'"))?,
        }
    }
    let mut symbols = Vec::new();
    let rom = if path.extension().is_some_and(|e| e == "nes") {
        fs::read(&path).map_err(|e| e.to_string())?
    } else {
        let (project, _) = load_project(&path)?;
        let out = compile(&project, Options::default()).map_err(|f| f.render())?;
        symbols = out.linked.symbols.clone();
        out.rom
    };
    let mut emu = nested_runtime::Emulator::new(rom, 48000.0)?;
    let mut energy = 0.0f64;
    for f in 0..frames {
        let extra = script.iter().filter(|(at, _)| *at == f).fold(0, |a, (_, b)| a | b);
        let audio = emu.frame(hold | extra)?;
        energy += audio.iter().map(|s| (*s as f64).abs()).sum::<f64>();
    }
    println!("ran {frames} frames, PC ${:04X}, audio energy {energy:.1}", emu.pc());
    for name in &watch {
        match symbols.iter().find(|s| s.name == *name || s.name.ends_with(&format!("::{name}"))) {
            Some(s) => {
                let bytes: Vec<String> = (0..s.size.clamp(1, 16)).map(|i| format!("{:02X}", emu.read_ram(s.address + i as u16))).collect();
                println!("  {} @ ${:04X}: {}", s.name, s.address, bytes.join(" "));
            }
            None => println!("  {name}: no such symbol"),
        }
    }
    if let Some(s) = symbols.iter().rev().find(|s| s.kind.starts_with("fn") && s.address <= emu.pc()) {
        println!("  PC in {}", s.name);
    }
    if let Some(p) = shot {
        fs::write(&p, emu.png(2)).map_err(|e| e.to_string())?;
        println!("wrote {}", p.display());
    }
    Ok(())
}

fn format_files(args: &mut impl Iterator<Item = String>) -> Result<(), String> {
    let mut check = false;
    let mut paths = Vec::new();
    for a in args {
        if a == "--check" {
            check = true;
        } else {
            paths.push(PathBuf::from(a));
        }
    }
    let mut differs = false;
    for p in paths {
        let src = fs::read_to_string(&p).map_err(|e| e.to_string())?;
        let formatted = nested_compiler::fmt::format(&src, &Default::default());
        if formatted != src {
            differs = true;
            if check {
                println!("needs formatting: {}", p.display());
            } else {
                fs::write(&p, formatted).map_err(|e| e.to_string())?;
                println!("formatted {}", p.display());
            }
        }
    }
    if check && differs {
        return Err("formatting differs".into());
    }
    Ok(())
}
