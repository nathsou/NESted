//! Report the program counter whenever a RAM byte changes:
//! `cargo run --example watch -- game.nes ADDR [frames]`.
use nessy::cpu::memory::Memory;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom = std::fs::read(&args[1]).unwrap();
    let addr = u16::from_str_radix(args[2].trim_start_matches('$'), 16).unwrap();
    let frames: u64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(60);
    let mut emu = nested_runtime::Emulator::new(rom, 44100.0).unwrap();
    let mut last = emu.read_ram(addr);
    let mut reports = 0;
    for f in 0..frames {
        emu.cpu.bus.ppu.frame_complete = false;
        while !emu.cpu.bus.ppu.frame_complete {
            let pc = emu.cpu.pc;
            emu.cpu.step();
            let v = emu.cpu.bus.ram.read_byte(addr);
            if v != last {
                println!("frame {f}: ${addr:04X} {last:02X} -> {v:02X} after PC ${pc:04X}");
                last = v;
                reports += 1;
                if reports > 40 {
                    return;
                }
            }
        }
    }
}
