//! Discrete-logic boards: a latch written through $8000-$FFFF selects whole
//! 32 KiB PRG banks and/or 8 KiB CHR banks.
//!
//! * CNROM (3): CHR bank in the low bits, fixed 16/32 KiB PRG.
//! * AxROM (7): 32 KiB PRG bank in bits 0-2, one-screen nametable in bit 4, CHR RAM.
//! * Color Dreams (11): PRG bank in bits 0-1, CHR bank in bits 4-7.
//! * BNROM (34): 32 KiB PRG bank, CHR RAM.
//! * GxROM (66): PRG bank in bits 4-5, CHR bank in bits 0-1.
use crate::{
    cpu::rom::{Cart, Mirroring},
    savestate::{self, SaveStateError},
};

use super::Mapper;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Board {
    Cnrom,
    Axrom,
    ColorDreams,
    Bnrom,
    Gxrom,
}

pub struct Discrete {
    board: Board,
    prg_ram: [u8; 0x2000],
    chr_ram: [u8; 0x2000],
    prg_bank: u8,
    chr_bank: u8,
}

impl Discrete {
    pub fn new(board: Board, cart: &mut Cart) -> Self {
        if board == Board::Axrom {
            cart.mirroring = Mirroring::OneScreenLowerBank;
        }
        Self {
            board,
            prg_ram: [0; 0x2000],
            chr_ram: [0; 0x2000],
            prg_bank: 0,
            chr_bank: 0,
        }
    }

    fn prg_offset(&self, cart: &Cart, addr: u16) -> usize {
        let size = cart.prg_rom_size as usize * 0x4000;
        let offset = match self.board {
            // CNROM has no PRG banking; 16 KiB images mirror.
            Board::Cnrom => (addr as usize - 0x8000) % size,
            _ => (self.prg_bank as usize * 0x8000 + (addr as usize - 0x8000)) % size,
        };
        cart.prg_rom_start + offset
    }

    fn chr_offset(&self, cart: &Cart, addr: u16) -> usize {
        let size = (cart.chr_rom_size as usize * 0x2000).max(0x2000);
        cart.chr_rom_start + (self.chr_bank as usize * 0x2000 + (addr as usize & 0x1fff)) % size
    }
}

impl Mapper for Discrete {
    fn read(&mut self, cart: &mut Cart, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1fff => {
                if cart.chr_rom_size == 0 {
                    self.chr_ram[addr as usize]
                } else {
                    cart.bytes[self.chr_offset(cart, addr)]
                }
            }
            0x6000..=0x7fff => self.prg_ram[(addr - 0x6000) as usize],
            0x8000..=0xffff => cart.bytes[self.prg_offset(cart, addr)],
            _ => 0,
        }
    }

    fn write(&mut self, cart: &mut Cart, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1fff => {
                if cart.chr_rom_size == 0 {
                    self.chr_ram[addr as usize] = val;
                }
            }
            0x6000..=0x7fff => self.prg_ram[(addr - 0x6000) as usize] = val,
            0x8000..=0xffff => {
                // Bus conflicts: the latch sees the AND of the written value
                // and the ROM byte at that address (except AxROM revisions
                // that avoid them; programs write through a matching table).
                let rom = cart.bytes[self.prg_offset(cart, addr)];
                let val = if self.board == Board::Axrom {
                    val
                } else {
                    val & rom
                };
                match self.board {
                    Board::Cnrom => self.chr_bank = val & 0x0f,
                    Board::Axrom => {
                        self.prg_bank = val & 0x07;
                        cart.mirroring = if val & 0x10 == 0 {
                            Mirroring::OneScreenLowerBank
                        } else {
                            Mirroring::OneScreenUpperBank
                        };
                    }
                    Board::ColorDreams => {
                        self.prg_bank = val & 0x03;
                        self.chr_bank = val >> 4;
                    }
                    Board::Bnrom => self.prg_bank = val & 0x0f,
                    Board::Gxrom => {
                        self.prg_bank = (val >> 4) & 0x03;
                        self.chr_bank = val & 0x03;
                    }
                }
            }
            _ => {}
        }
    }
}

const SECTION: &str = "DISCRETE";

impl savestate::Save for Discrete {
    fn save(&self, parent: &mut savestate::Section) {
        let s = parent.create_child(SECTION);
        s.data.write_u8_slice(&self.prg_ram);
        s.data.write_u8_slice(&self.chr_ram);
        s.data.write_u8(self.prg_bank);
        s.data.write_u8(self.chr_bank);
    }

    fn load(&mut self, parent: &mut savestate::Section) -> Result<(), SaveStateError> {
        let s = parent.get(SECTION)?;
        s.data.read_u8_slice(&mut self.prg_ram)?;
        s.data.read_u8_slice(&mut self.chr_ram)?;
        self.prg_bank = s.data.read_u8()?;
        self.chr_bank = s.data.read_u8()?;
        Ok(())
    }
}
