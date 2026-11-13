//! Headless NES execution for NESted tools and tests. Every frame comes
//! from executing compiled ROM bytes on the Nessy emulator.
use nessy::{
    bus::Bus,
    cpu::{memory::Memory, rom::ROM, CPU},
    savestate::{Save, SaveState},
};

pub struct Emulator {
    pub cpu: Box<CPU>,
    pub frames: u64,
}

impl Emulator {
    pub fn new(rom: Vec<u8>, sample_rate: f64) -> Result<Self, String> {
        let rom = ROM::new(rom).map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            cpu: Box::new(CPU::new(Bus::new(rom, sample_rate))),
            frames: 0,
        })
    }
    /// Run one frame with controller 1 holding `buttons`; returns audio samples.
    pub fn frame(&mut self, buttons: u8) -> Result<Vec<f32>, String> {
        self.cpu.bus.joypad1.update(buttons);
        self.cpu.bus.ppu.frame_complete = false;
        let mut steps = 0;
        while !self.cpu.bus.ppu.frame_complete {
            self.cpu.step();
            steps += 1;
            if steps > 200_000 {
                return Err("Emulator instruction budget exceeded".into());
            }
        }
        self.frames += 1;
        let n = self.cpu.bus.apu.remaining_samples() as usize;
        let mut audio = vec![0.0; n];
        self.cpu.bus.apu.fill(&mut audio);
        Ok(audio)
    }
    /// 256x240 RGB pixels.
    pub fn pixels(&self) -> &[u8] {
        self.cpu.bus.ppu.get_frame()
    }
    pub fn read_ram(&mut self, address: u16) -> u8 {
        self.cpu.bus.ram.read_byte(address)
    }
    pub fn write_ram(&mut self, address: u16, value: u8) {
        self.cpu.bus.ram.write_byte(address, value);
    }
    pub fn pc(&self) -> u16 {
        self.cpu.pc
    }
    /// PPU memory ($0000-$3FFF) as the program sees it, without side effects.
    pub fn ppu_peek(&mut self, address: u16) -> u8 {
        self.cpu.bus.ppu.inspect(address)
    }
    /// 16 KiB snapshot of the PPU address space.
    pub fn ppu_memory(&mut self) -> Vec<u8> {
        (0..0x4000u16).map(|a| self.ppu_peek(a)).collect()
    }
    pub fn oam(&self) -> Vec<u8> {
        self.cpu.bus.ppu.oam().to_vec()
    }
    /// PPUCTRL and PPUMASK.
    pub fn ppu_control(&self) -> (u8, u8) {
        self.cpu.bus.ppu.control()
    }
    pub fn save_state(&self) -> Vec<u8> {
        let mut state = SaveState::new(&self.cpu.bus.ppu.rom.cart.hash);
        self.cpu.save(state.get_root_mut());
        state.encode()
    }
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut state = SaveState::decode(data).map_err(|e| format!("{e:?}"))?;
        self.cpu
            .load(state.get_root_mut())
            .map_err(|e| format!("{e:?}"))
    }
    /// The current frame as a PNG image, optionally scaled.
    pub fn png(&self, scale: usize) -> Vec<u8> {
        let src = self.pixels();
        let (w, h) = (256 * scale, 240 * scale);
        let mut raw = Vec::with_capacity((w * 3 + 1) * h);
        for y in 0..h {
            raw.push(0);
            for x in 0..w {
                let i = ((y / scale) * 256 + x / scale) * 3;
                raw.extend_from_slice(&src[i..i + 3]);
            }
        }
        png(w as u32, h as u32, &raw)
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// Minimal PNG writer: RGB, uncompressed deflate blocks.
pub fn png(width: u32, height: u32, filtered: &[u8]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = filtered.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in filtered {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
