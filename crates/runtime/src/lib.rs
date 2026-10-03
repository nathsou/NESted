//! NES execution adapter. All game logic is executed from compiled ROM bytes.
use nessy::{bus::Bus,cpu::{CPU,rom::ROM,memory::Memory}};
pub struct Emulator {pub cpu:Box<CPU>,pub frames:u64}
impl Emulator {
 pub fn new(rom:Vec<u8>,sample_rate:f64)->Result<Self,String>{let rom=ROM::new(rom).map_err(|e|format!("{e:?}"))?;Ok(Self{cpu:Box::new(CPU::new(Bus::new(rom,sample_rate))),frames:0})}
 pub fn frame(&mut self,buttons:u8)->Result<Vec<f32>,String>{self.cpu.bus.joypad1.update(buttons);self.cpu.bus.ppu.frame_complete=false;let mut steps=0;while !self.cpu.bus.ppu.frame_complete{self.cpu.step();steps+=1;if steps>200_000{return Err("Emulator instruction budget exceeded".into());}}self.frames+=1;let n=self.cpu.bus.apu.remaining_samples() as usize;let mut audio=vec![0.0;n];self.cpu.bus.apu.fill(&mut audio);Ok(audio)}
 pub fn pixels(&self)->&[u8]{self.cpu.bus.ppu.get_frame()}
 pub fn read_ram(&mut self,address:u16)->u8{self.cpu.bus.ram.read_byte(address)}
 pub fn write_ram(&mut self,address:u16,value:u8){self.cpu.bus.ram.write_byte(address,value);}
}
