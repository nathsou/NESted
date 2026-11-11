//! Cartridge boards and linking.
pub enum Board {}
impl Board {
    pub const NAMES: &'static [&'static str] = &["nrom"];
    pub fn from_name(_n: &str) -> Option<()> { Some(()) }
}
