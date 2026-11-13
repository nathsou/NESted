//! The standard library, embedded in the compiler.
pub const FILES: &[(&str, &str)] = &[
    ("std/runtime.nst", include_str!("std/runtime.nst")),
    ("std/nes.nst", include_str!("std/nes.nst")),
    ("std/video.nst", include_str!("std/video.nst")),
    ("std/input.nst", include_str!("std/input.nst")),
    ("std/math.nst", include_str!("std/math.nst")),
    ("std/audio.nst", include_str!("std/audio.nst")),
    ("std/mapper.nst", include_str!("std/mapper.nst")),
];
