//! Machine-level optimization.
use crate::asm::Line;
use crate::mir::{InstId, Mir};

pub fn optimize(_lines: &mut Vec<Line>, _mir: &Mir, _inst: InstId) {}
