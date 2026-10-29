//! A lean, cycle-counted NMOS 6502 for headless cartridge benchmarking.
//!
//! Only what compiled NESted programs need: official opcodes, NROM PRG,
//! 2 KiB internal RAM, a vblank flag and frame NMIs. No PPU rendering or APU,
//! which keeps engine searches tens of times faster than full emulation.

pub const FRAME_CYCLES: u64 = 29781; // NTSC: 341 * 262 / 3, rounded

pub struct Machine {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    // Flags kept unpacked for speed.
    c: bool,
    z: bool,
    i: bool,
    d: bool,
    v: bool,
    n: bool,
    pub ram: [u8; 2048],
    /// PRG mirrored to a full 32 KiB so reads need only a mask.
    prg: Box<[u8; 32768]>,
    pub cycles: u64,
    next_frame: u64,
    pub frames: u64,
    ppu_ctrl: u8,
    vblank: bool,
    nmi_pending: bool,
    pub buttons: u8,
    shift: u8,
    strobe: bool,
    /// Per-PC cycle accumulator for profiling (optional).
    pub profile: Option<Vec<u64>>,
    /// Per-PC execution counts, kept alongside `profile`.
    pub counts: Option<Vec<u64>>,
}

impl Machine {
    pub fn new(rom: &[u8]) -> Result<Self, String> {
        if rom.len() < 16 || &rom[..4] != b"NES\x1a" {
            return Err("Not an iNES image".into());
        }
        if rom[6] >> 4 != 0 || rom[7] >> 4 != 0 {
            return Err("chessbench supports NROM only".into());
        }
        let banks = rom[4] as usize;
        let image = &rom[16..16 + banks * 16384];
        let mut prg = Box::new([0u8; 32768]);
        for (i, b) in prg.iter_mut().enumerate() {
            *b = image[i % image.len()];
        }
        let mut m = Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xfd,
            pc: 0,
            c: false,
            z: false,
            i: true,
            d: false,
            v: false,
            n: false,
            ram: [0; 2048],
            prg,
            cycles: 0,
            next_frame: FRAME_CYCLES,
            frames: 0,
            ppu_ctrl: 0,
            vblank: false,
            nmi_pending: false,
            buttons: 0,
            shift: 0,
            strobe: false,
            profile: None,
            counts: None,
        };
        m.pc = m.read16(0xfffc);
        Ok(m)
    }

    #[inline(always)]
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0..=0x1fff => self.ram[(addr & 0x7ff) as usize],
            0x2000..=0x3fff => {
                if addr & 7 == 2 {
                    let v = if self.vblank { 0x80 } else { 0 };
                    self.vblank = false;
                    v
                } else {
                    0
                }
            }
            0x4016 => {
                let bit = self.shift & 1;
                if !self.strobe {
                    self.shift = (self.shift >> 1) | 0x80;
                }
                bit | 0x40
            }
            0x8000..=0xffff => self.prg[(addr & 0x7fff) as usize],
            _ => 0,
        }
    }
    #[inline(always)]
    fn write(&mut self, addr: u16, v: u8) {
        match addr {
            0..=0x1fff => self.ram[(addr & 0x7ff) as usize] = v,
            0x2000..=0x3fff => {
                if addr & 7 == 0 {
                    let was = self.ppu_ctrl & 0x80;
                    self.ppu_ctrl = v;
                    if was == 0 && v & 0x80 != 0 && self.vblank {
                        self.nmi_pending = true;
                    }
                }
            }
            0x4014 => self.cycles += 513,
            0x4016 => {
                self.strobe = v & 1 != 0;
                if self.strobe {
                    self.shift = self.buttons;
                } else {
                    self.shift = self.buttons;
                }
            }
            _ => {}
        }
    }
    fn read16(&mut self, addr: u16) -> u16 {
        u16::from(self.read(addr)) | u16::from(self.read(addr.wrapping_add(1))) << 8
    }
    fn push(&mut self, v: u8) {
        self.ram[0x100 + self.sp as usize] = v;
        self.sp = self.sp.wrapping_sub(1);
    }
    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.ram[0x100 + self.sp as usize]
    }
    fn status(&self, b: bool) -> u8 {
        (self.c as u8)
            | (self.z as u8) << 1
            | (self.i as u8) << 2
            | (self.d as u8) << 3
            | (b as u8) << 4
            | 0x20
            | (self.v as u8) << 6
            | (self.n as u8) << 7
    }
    fn set_status(&mut self, p: u8) {
        self.c = p & 1 != 0;
        self.z = p & 2 != 0;
        self.i = p & 4 != 0;
        self.d = p & 8 != 0;
        self.v = p & 0x40 != 0;
        self.n = p & 0x80 != 0;
    }
    #[inline(always)]
    fn nz(&mut self, v: u8) -> u8 {
        self.z = v == 0;
        self.n = v & 0x80 != 0;
        v
    }
    fn fetch(&mut self) -> u8 {
        let v = self.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch();
        let hi = self.fetch();
        u16::from(lo) | u16::from(hi) << 8
    }

    /// Runs until the next frame boundary (one NMI period).
    pub fn run_frame(&mut self) {
        let end = self.next_frame;
        while self.cycles < end {
            self.step();
        }
        self.next_frame += FRAME_CYCLES;
        self.frames += 1;
        self.vblank = true;
        if self.ppu_ctrl & 0x80 != 0 {
            self.nmi_pending = true;
        }
    }

    fn interrupt(&mut self, vector: u16) {
        let pc = self.pc;
        self.push((pc >> 8) as u8);
        self.push(pc as u8);
        let p = self.status(false);
        self.push(p);
        self.i = true;
        self.pc = self.read16(vector);
        self.cycles += 7;
    }

    // Addressing helpers return (address, page crossed).
    fn zp(&mut self) -> u16 {
        u16::from(self.fetch())
    }
    fn zpx(&mut self) -> u16 {
        u16::from(self.fetch().wrapping_add(self.x))
    }
    fn zpy(&mut self) -> u16 {
        u16::from(self.fetch().wrapping_add(self.y))
    }
    fn abs(&mut self) -> u16 {
        self.fetch16()
    }
    fn absx(&mut self) -> (u16, bool) {
        let b = self.fetch16();
        let a = b.wrapping_add(u16::from(self.x));
        (a, (a ^ b) & 0xff00 != 0)
    }
    fn absy(&mut self) -> (u16, bool) {
        let b = self.fetch16();
        let a = b.wrapping_add(u16::from(self.y));
        (a, (a ^ b) & 0xff00 != 0)
    }
    fn indx(&mut self) -> u16 {
        let p = self.fetch().wrapping_add(self.x);
        u16::from(self.ram[p as usize]) | u16::from(self.ram[p.wrapping_add(1) as usize]) << 8
    }
    fn indy(&mut self) -> (u16, bool) {
        let p = self.fetch();
        let b =
            u16::from(self.ram[p as usize]) | u16::from(self.ram[p.wrapping_add(1) as usize]) << 8;
        let a = b.wrapping_add(u16::from(self.y));
        (a, (a ^ b) & 0xff00 != 0)
    }

    fn adc(&mut self, m: u8) {
        let sum = u16::from(self.a) + u16::from(m) + u16::from(self.c);
        let r = sum as u8;
        self.v = (!(self.a ^ m) & (self.a ^ r) & 0x80) != 0;
        self.c = sum > 0xff;
        self.a = self.nz(r);
    }
    fn sbc(&mut self, m: u8) {
        self.adc(!m);
    }
    fn cmp(&mut self, r: u8, m: u8) {
        let d = r.wrapping_sub(m);
        self.c = r >= m;
        self.nz(d);
    }
    fn branch(&mut self, cond: bool) {
        let off = self.fetch() as i8;
        if cond {
            let old = self.pc;
            self.pc = self.pc.wrapping_add(off as u16);
            self.cycles += if (old ^ self.pc) & 0xff00 != 0 { 2 } else { 1 };
        }
    }

    pub fn step(&mut self) {
        if self.nmi_pending {
            self.nmi_pending = false;
            self.interrupt(0xfffa);
        }
        let pc = self.pc;
        let start = self.cycles;
        let op = self.fetch();
        macro_rules! rd {
            ($addr:expr, $c:expr) => {{
                let a = $addr;
                self.cycles += $c;
                self.read(a)
            }};
        }
        macro_rules! rdp {
            ($pair:expr, $c:expr) => {{
                let (a, crossed) = $pair;
                self.cycles += $c + crossed as u64;
                self.read(a)
            }};
        }
        macro_rules! rmw {
            ($addr:expr, $c:expr, $f:ident) => {{
                let a = $addr;
                self.cycles += $c;
                let v = self.read(a);
                let r = self.$f(v);
                self.write(a, r);
            }};
        }
        match op {
            // LDA
            0xa9 => { let v = self.fetch(); self.a = self.nz(v); self.cycles += 2; }
            0xa5 => { let a = self.zp(); let v = rd!(a, 3); self.a = self.nz(v); }
            0xb5 => { let a = self.zpx(); let v = rd!(a, 4); self.a = self.nz(v); }
            0xad => { let a = self.abs(); let v = rd!(a, 4); self.a = self.nz(v); }
            0xbd => { let p = self.absx(); let v = rdp!(p, 4); self.a = self.nz(v); }
            0xb9 => { let p = self.absy(); let v = rdp!(p, 4); self.a = self.nz(v); }
            0xa1 => { let a = self.indx(); let v = rd!(a, 6); self.a = self.nz(v); }
            0xb1 => { let p = self.indy(); let v = rdp!(p, 5); self.a = self.nz(v); }
            // LDX
            0xa2 => { let v = self.fetch(); self.x = self.nz(v); self.cycles += 2; }
            0xa6 => { let a = self.zp(); let v = rd!(a, 3); self.x = self.nz(v); }
            0xb6 => { let a = self.zpy(); let v = rd!(a, 4); self.x = self.nz(v); }
            0xae => { let a = self.abs(); let v = rd!(a, 4); self.x = self.nz(v); }
            0xbe => { let p = self.absy(); let v = rdp!(p, 4); self.x = self.nz(v); }
            // LDY
            0xa0 => { let v = self.fetch(); self.y = self.nz(v); self.cycles += 2; }
            0xa4 => { let a = self.zp(); let v = rd!(a, 3); self.y = self.nz(v); }
            0xb4 => { let a = self.zpx(); let v = rd!(a, 4); self.y = self.nz(v); }
            0xac => { let a = self.abs(); let v = rd!(a, 4); self.y = self.nz(v); }
            0xbc => { let p = self.absx(); let v = rdp!(p, 4); self.y = self.nz(v); }
            // STA
            0x85 => { let a = self.zp(); self.cycles += 3; self.write(a, self.a); }
            0x95 => { let a = self.zpx(); self.cycles += 4; self.write(a, self.a); }
            0x8d => { let a = self.abs(); self.cycles += 4; self.write(a, self.a); }
            0x9d => { let (a, _) = self.absx(); self.cycles += 5; self.write(a, self.a); }
            0x99 => { let (a, _) = self.absy(); self.cycles += 5; self.write(a, self.a); }
            0x81 => { let a = self.indx(); self.cycles += 6; self.write(a, self.a); }
            0x91 => { let (a, _) = self.indy(); self.cycles += 6; self.write(a, self.a); }
            // STX / STY
            0x86 => { let a = self.zp(); self.cycles += 3; self.write(a, self.x); }
            0x96 => { let a = self.zpy(); self.cycles += 4; self.write(a, self.x); }
            0x8e => { let a = self.abs(); self.cycles += 4; self.write(a, self.x); }
            0x84 => { let a = self.zp(); self.cycles += 3; self.write(a, self.y); }
            0x94 => { let a = self.zpx(); self.cycles += 4; self.write(a, self.y); }
            0x8c => { let a = self.abs(); self.cycles += 4; self.write(a, self.y); }
            // Transfers
            0xaa => { self.x = self.nz(self.a); self.cycles += 2; }
            0xa8 => { self.y = self.nz(self.a); self.cycles += 2; }
            0x8a => { self.a = self.nz(self.x); self.cycles += 2; }
            0x98 => { self.a = self.nz(self.y); self.cycles += 2; }
            0xba => { self.x = self.nz(self.sp); self.cycles += 2; }
            0x9a => { self.sp = self.x; self.cycles += 2; }
            // Stack
            0x48 => { self.push(self.a); self.cycles += 3; }
            0x68 => { let v = self.pop(); self.a = self.nz(v); self.cycles += 4; }
            0x08 => { let p = self.status(true); self.push(p); self.cycles += 3; }
            0x28 => { let p = self.pop(); self.set_status(p); self.cycles += 4; }
            // ALU
            0x69 => { let v = self.fetch(); self.adc(v); self.cycles += 2; }
            0x65 => { let a = self.zp(); let v = rd!(a, 3); self.adc(v); }
            0x75 => { let a = self.zpx(); let v = rd!(a, 4); self.adc(v); }
            0x6d => { let a = self.abs(); let v = rd!(a, 4); self.adc(v); }
            0x7d => { let p = self.absx(); let v = rdp!(p, 4); self.adc(v); }
            0x79 => { let p = self.absy(); let v = rdp!(p, 4); self.adc(v); }
            0x61 => { let a = self.indx(); let v = rd!(a, 6); self.adc(v); }
            0x71 => { let p = self.indy(); let v = rdp!(p, 5); self.adc(v); }
            0xe9 => { let v = self.fetch(); self.sbc(v); self.cycles += 2; }
            0xe5 => { let a = self.zp(); let v = rd!(a, 3); self.sbc(v); }
            0xf5 => { let a = self.zpx(); let v = rd!(a, 4); self.sbc(v); }
            0xed => { let a = self.abs(); let v = rd!(a, 4); self.sbc(v); }
            0xfd => { let p = self.absx(); let v = rdp!(p, 4); self.sbc(v); }
            0xf9 => { let p = self.absy(); let v = rdp!(p, 4); self.sbc(v); }
            0xe1 => { let a = self.indx(); let v = rd!(a, 6); self.sbc(v); }
            0xf1 => { let p = self.indy(); let v = rdp!(p, 5); self.sbc(v); }
            0x29 => { let v = self.fetch(); self.a = self.nz(self.a & v); self.cycles += 2; }
            0x25 => { let a = self.zp(); let v = rd!(a, 3); self.a = self.nz(self.a & v); }
            0x35 => { let a = self.zpx(); let v = rd!(a, 4); self.a = self.nz(self.a & v); }
            0x2d => { let a = self.abs(); let v = rd!(a, 4); self.a = self.nz(self.a & v); }
            0x3d => { let p = self.absx(); let v = rdp!(p, 4); self.a = self.nz(self.a & v); }
            0x39 => { let p = self.absy(); let v = rdp!(p, 4); self.a = self.nz(self.a & v); }
            0x21 => { let a = self.indx(); let v = rd!(a, 6); self.a = self.nz(self.a & v); }
            0x31 => { let p = self.indy(); let v = rdp!(p, 5); self.a = self.nz(self.a & v); }
            0x09 => { let v = self.fetch(); self.a = self.nz(self.a | v); self.cycles += 2; }
            0x05 => { let a = self.zp(); let v = rd!(a, 3); self.a = self.nz(self.a | v); }
            0x15 => { let a = self.zpx(); let v = rd!(a, 4); self.a = self.nz(self.a | v); }
            0x0d => { let a = self.abs(); let v = rd!(a, 4); self.a = self.nz(self.a | v); }
            0x1d => { let p = self.absx(); let v = rdp!(p, 4); self.a = self.nz(self.a | v); }
            0x19 => { let p = self.absy(); let v = rdp!(p, 4); self.a = self.nz(self.a | v); }
            0x01 => { let a = self.indx(); let v = rd!(a, 6); self.a = self.nz(self.a | v); }
            0x11 => { let p = self.indy(); let v = rdp!(p, 5); self.a = self.nz(self.a | v); }
            0x49 => { let v = self.fetch(); self.a = self.nz(self.a ^ v); self.cycles += 2; }
            0x45 => { let a = self.zp(); let v = rd!(a, 3); self.a = self.nz(self.a ^ v); }
            0x55 => { let a = self.zpx(); let v = rd!(a, 4); self.a = self.nz(self.a ^ v); }
            0x4d => { let a = self.abs(); let v = rd!(a, 4); self.a = self.nz(self.a ^ v); }
            0x5d => { let p = self.absx(); let v = rdp!(p, 4); self.a = self.nz(self.a ^ v); }
            0x59 => { let p = self.absy(); let v = rdp!(p, 4); self.a = self.nz(self.a ^ v); }
            0x41 => { let a = self.indx(); let v = rd!(a, 6); self.a = self.nz(self.a ^ v); }
            0x51 => { let p = self.indy(); let v = rdp!(p, 5); self.a = self.nz(self.a ^ v); }
            // Compare
            0xc9 => { let v = self.fetch(); self.cmp(self.a, v); self.cycles += 2; }
            0xc5 => { let a = self.zp(); let v = rd!(a, 3); self.cmp(self.a, v); }
            0xd5 => { let a = self.zpx(); let v = rd!(a, 4); self.cmp(self.a, v); }
            0xcd => { let a = self.abs(); let v = rd!(a, 4); self.cmp(self.a, v); }
            0xdd => { let p = self.absx(); let v = rdp!(p, 4); self.cmp(self.a, v); }
            0xd9 => { let p = self.absy(); let v = rdp!(p, 4); self.cmp(self.a, v); }
            0xc1 => { let a = self.indx(); let v = rd!(a, 6); self.cmp(self.a, v); }
            0xd1 => { let p = self.indy(); let v = rdp!(p, 5); self.cmp(self.a, v); }
            0xe0 => { let v = self.fetch(); self.cmp(self.x, v); self.cycles += 2; }
            0xe4 => { let a = self.zp(); let v = rd!(a, 3); self.cmp(self.x, v); }
            0xec => { let a = self.abs(); let v = rd!(a, 4); self.cmp(self.x, v); }
            0xc0 => { let v = self.fetch(); self.cmp(self.y, v); self.cycles += 2; }
            0xc4 => { let a = self.zp(); let v = rd!(a, 3); self.cmp(self.y, v); }
            0xcc => { let a = self.abs(); let v = rd!(a, 4); self.cmp(self.y, v); }
            0x24 => { let a = self.zp(); let v = rd!(a, 3); self.bit(v); }
            0x2c => { let a = self.abs(); let v = rd!(a, 4); self.bit(v); }
            // Inc/dec
            0xe6 => rmw!(self.zp(), 5, inc_v),
            0xf6 => rmw!(self.zpx(), 6, inc_v),
            0xee => rmw!(self.abs(), 6, inc_v),
            0xfe => rmw!(self.absx().0, 7, inc_v),
            0xc6 => rmw!(self.zp(), 5, dec_v),
            0xd6 => rmw!(self.zpx(), 6, dec_v),
            0xce => rmw!(self.abs(), 6, dec_v),
            0xde => rmw!(self.absx().0, 7, dec_v),
            0xe8 => { self.x = self.nz(self.x.wrapping_add(1)); self.cycles += 2; }
            0xca => { self.x = self.nz(self.x.wrapping_sub(1)); self.cycles += 2; }
            0xc8 => { self.y = self.nz(self.y.wrapping_add(1)); self.cycles += 2; }
            0x88 => { self.y = self.nz(self.y.wrapping_sub(1)); self.cycles += 2; }
            // Shifts
            0x0a => { self.a = self.asl_v(self.a); self.cycles += 2; }
            0x06 => rmw!(self.zp(), 5, asl_v),
            0x16 => rmw!(self.zpx(), 6, asl_v),
            0x0e => rmw!(self.abs(), 6, asl_v),
            0x1e => rmw!(self.absx().0, 7, asl_v),
            0x4a => { self.a = self.lsr_v(self.a); self.cycles += 2; }
            0x46 => rmw!(self.zp(), 5, lsr_v),
            0x56 => rmw!(self.zpx(), 6, lsr_v),
            0x4e => rmw!(self.abs(), 6, lsr_v),
            0x5e => rmw!(self.absx().0, 7, lsr_v),
            0x2a => { self.a = self.rol_v(self.a); self.cycles += 2; }
            0x26 => rmw!(self.zp(), 5, rol_v),
            0x36 => rmw!(self.zpx(), 6, rol_v),
            0x2e => rmw!(self.abs(), 6, rol_v),
            0x3e => rmw!(self.absx().0, 7, rol_v),
            0x6a => { self.a = self.ror_v(self.a); self.cycles += 2; }
            0x66 => rmw!(self.zp(), 5, ror_v),
            0x76 => rmw!(self.zpx(), 6, ror_v),
            0x6e => rmw!(self.abs(), 6, ror_v),
            0x7e => rmw!(self.absx().0, 7, ror_v),
            // Branches
            0x10 => { self.cycles += 2; self.branch(!self.n); }
            0x30 => { self.cycles += 2; self.branch(self.n); }
            0x50 => { self.cycles += 2; self.branch(!self.v); }
            0x70 => { self.cycles += 2; self.branch(self.v); }
            0x90 => { self.cycles += 2; self.branch(!self.c); }
            0xb0 => { self.cycles += 2; self.branch(self.c); }
            0xd0 => { self.cycles += 2; self.branch(!self.z); }
            0xf0 => { self.cycles += 2; self.branch(self.z); }
            // Jumps
            0x4c => { self.pc = self.fetch16(); self.cycles += 3; }
            0x6c => {
                let p = self.fetch16();
                let lo = self.read(p);
                let hi = self.read((p & 0xff00) | (p.wrapping_add(1) & 0xff));
                self.pc = u16::from(lo) | u16::from(hi) << 8;
                self.cycles += 5;
            }
            0x20 => {
                let t = self.fetch16();
                let r = self.pc.wrapping_sub(1);
                self.push((r >> 8) as u8);
                self.push(r as u8);
                self.pc = t;
                self.cycles += 6;
            }
            0x60 => {
                let lo = self.pop();
                let hi = self.pop();
                self.pc = (u16::from(lo) | u16::from(hi) << 8).wrapping_add(1);
                self.cycles += 6;
            }
            0x40 => {
                let p = self.pop();
                self.set_status(p);
                let lo = self.pop();
                let hi = self.pop();
                self.pc = u16::from(lo) | u16::from(hi) << 8;
                self.cycles += 6;
            }
            0x00 => {
                self.pc = self.pc.wrapping_add(1);
                let pc = self.pc;
                self.push((pc >> 8) as u8);
                self.push(pc as u8);
                let p = self.status(true);
                self.push(p);
                self.i = true;
                self.pc = self.read16(0xfffe);
                self.cycles += 7;
            }
            // Flags
            0x18 => { self.c = false; self.cycles += 2; }
            0x38 => { self.c = true; self.cycles += 2; }
            0x58 => { self.i = false; self.cycles += 2; }
            0x78 => { self.i = true; self.cycles += 2; }
            0xb8 => { self.v = false; self.cycles += 2; }
            0xd8 => { self.d = false; self.cycles += 2; }
            0xf8 => { self.d = true; self.cycles += 2; }
            0xea => { self.cycles += 2; }
            _ => panic!("Unsupported opcode ${op:02X} at ${pc:04X}"),
        }
        if let Some(p) = &mut self.profile {
            p[pc as usize] += self.cycles - start;
            if let Some(c) = &mut self.counts {
                c[pc as usize] += 1;
            }
        }
    }
    fn bit(&mut self, v: u8) {
        self.z = self.a & v == 0;
        self.n = v & 0x80 != 0;
        self.v = v & 0x40 != 0;
    }
    fn inc_v(&mut self, v: u8) -> u8 {
        self.nz(v.wrapping_add(1))
    }
    fn dec_v(&mut self, v: u8) -> u8 {
        self.nz(v.wrapping_sub(1))
    }
    fn asl_v(&mut self, v: u8) -> u8 {
        self.c = v & 0x80 != 0;
        self.nz(v << 1)
    }
    fn lsr_v(&mut self, v: u8) -> u8 {
        self.c = v & 1 != 0;
        self.nz(v >> 1)
    }
    fn rol_v(&mut self, v: u8) -> u8 {
        let c = self.c as u8;
        self.c = v & 0x80 != 0;
        self.nz(v << 1 | c)
    }
    fn ror_v(&mut self, v: u8) -> u8 {
        let c = self.c as u8;
        self.c = v & 1 != 0;
        self.nz(v >> 1 | c << 7)
    }
    pub fn peek(&self, addr: u16) -> u8 {
        self.ram[(addr & 0x7ff) as usize]
    }
    pub fn poke(&mut self, addr: u16, v: u8) {
        self.ram[(addr & 0x7ff) as usize] = v;
    }
}
