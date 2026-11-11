//! The standard library, embedded in the compiler.
pub const FILES: &[(&str, &str)] = &[
    ("std/runtime.nst", include_str!("std/runtime.nst")),
];
