//! Drives the cartridge on the full emulator (PPU included) with a scripted
//! controller sequence, saving screenshots along the way.
//!
//! Script: comma-separated steps. `BUTTONS:FRAMES` holds the buttons for that
//! many frames (letters A B s=Select S=Start U D L R, `-` for none);
//! `shot:NAME` writes NAME.png into the output directory.
use std::path::Path;

/// RAM addresses of the mailbox and board, for `fen:` steps.
pub struct Symbols {
    pub board: u16,
    pub io_arg: u16,
    pub io_cmd: u16,
}

pub fn run(rom: &[u8], script: &str, out: &Path, sym: &Symbols) -> Result<(), String> {
    let mut emu = nested_runtime::Emulator::new(rom.to_vec(), 44100.0)?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for step in script.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (what, arg) = step.split_once(':').ok_or(format!("bad step {step}"))?;
        if what == "fen" {
            // FEN with '_' for spaces: board, side, castling only.
            let fen = arg.replace('_', " ");
            let mut parts = fen.split_whitespace();
            for i in 0..128 {
                emu.write_ram(sym.board + i, 0);
            }
            let (mut rank, mut file) = (7i32, 0i32);
            for c in parts.next().unwrap_or("").chars() {
                match c {
                    '/' => {
                        rank -= 1;
                        file = 0;
                    }
                    '1'..='8' => file += c as i32 - '0' as i32,
                    _ => {
                        let t = " pnbrqk".find(c.to_ascii_lowercase()).unwrap_or(0) as u8;
                        let color = if c.is_ascii_lowercase() { 8 } else { 0 };
                        emu.write_ram(sym.board + (rank * 16 + file) as u16, t | color);
                        file += 1;
                    }
                }
            }
            let side = if parts.next() == Some("b") { 8 } else { 0 };
            emu.write_ram(sym.io_arg, side);
            emu.write_ram(sym.io_arg + 1, 0);
            emu.write_ram(sym.io_arg + 2, 128);
            emu.write_ram(sym.io_cmd, 5);
            emu.frame(0)?;
            continue;
        }
        if what == "shot" {
            save_png(emu.pixels(), &out.join(format!("{arg}.png")))?;
            continue;
        }
        let mut mask = 0u8;
        for c in what.chars() {
            mask |= match c {
                'A' => 1,
                'B' => 2,
                's' => 4,
                'S' => 8,
                'U' => 16,
                'D' => 32,
                'L' => 64,
                'R' => 128,
                _ => 0,
            };
        }
        let frames: usize = arg.parse().map_err(|_| format!("bad frames in {step}"))?;
        for _ in 0..frames {
            emu.frame(mask)?;
        }
    }
    Ok(())
}

/// Minimal PNG writer (stored deflate blocks), 2x nearest-neighbour scale.
fn save_png(rgb: &[u8], path: &Path) -> Result<(), String> {
    let (w, h, s) = (256usize, 240usize, 2usize);
    let mut raw = Vec::with_capacity((w * s * 3 + 1) * h * s);
    for y in 0..h * s {
        raw.push(0);
        for x in 0..w * s {
            let i = ((y / s) * w + x / s) * 3;
            raw.extend_from_slice(&rgb[i..i + 3]);
        }
    }
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(u8::from(last));
        let n = chunk.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &v in &raw {
        a = (a + u32::from(v)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        png.extend_from_slice(&body);
        png.extend_from_slice(&crc(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&((w * s) as u32).to_be_bytes());
    ihdr.extend_from_slice(&((h * s) as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    std::fs::write(path, png).map_err(|e| e.to_string())
}

fn crc(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}
