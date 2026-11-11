//! NESted: a statically allocated language for the NES, with an embeddable
//! compiler, assembler, linker and language service. No third-party
//! dependencies and no I/O: hosts supply sources and asset bytes.
pub mod asm;
pub mod ast;
pub mod check;
pub mod check_expr;
pub mod consteval;
pub mod diag;
pub mod hir;
pub mod json;
pub mod lexer;
pub mod link;
pub mod parser;
pub mod source;
pub mod stdlib;
pub mod types;

pub use check::{analyze, Analysis, Project};
pub use diag::{Diagnostic, Severity};
