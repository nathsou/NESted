//! Plain WebAssembly ABI for the playground: no bindgen and no JavaScript
//! runtime dependencies. Buffers passed in by the host stay host-owned until
//! `nested_free`; results are read through the `*_ptr`/`*_len` exports.
use nested_compiler::{json::Json, lsp::LanguageServer, Options, Project, Severity};
use nested_runtime::Emulator;
use std::sync::Mutex;

struct Session {
    project: Project,
    result: Vec<u8>,
    rom: Vec<u8>,
    lsp: Option<LanguageServer>,
    emulator: Option<Emulator>,
    audio: Vec<f32>,
    ram: Vec<u8>,
    ppu: Vec<u8>,
    oam: Vec<u8>,
}

/// WebAssembly runs this module on one thread; the language server's
/// caches are not `Send`, so the session is shared through this wrapper.
struct Shared(Session);
// SAFETY: wasm32-unknown-unknown without atomics is single-threaded.
unsafe impl Send for Shared {}

struct Guard(std::sync::MutexGuard<'static, Shared>);
impl std::ops::Deref for Guard {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.0 .0
    }
}
impl std::ops::DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.0 .0
    }
}

static SESSION: Mutex<Shared> = Mutex::new(Shared(Session {
    project: Project {
        sources: std::collections::BTreeMap::new(),
        assets: std::collections::BTreeMap::new(),
        entry: String::new(),
    },
    result: Vec::new(),
    rom: Vec::new(),
    lsp: None,
    emulator: None,
    audio: Vec::new(),
    ram: Vec::new(),
    ppu: Vec::new(),
    oam: Vec::new(),
}));

fn session() -> Guard {
    Guard(SESSION.lock().unwrap_or_else(|e| e.into_inner()))
}

#[no_mangle]
pub extern "C" fn nested_version() -> u32 {
    2
}

#[no_mangle]
pub extern "C" fn nested_alloc(length: usize) -> *mut u8 {
    if length > 16 * 1024 * 1024 {
        return std::ptr::null_mut();
    }
    let mut buffer = vec![0u8; length.max(1)].into_boxed_slice();
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// # Safety
/// The pointer must come from `nested_alloc` with the same length.
#[no_mangle]
pub unsafe extern "C" fn nested_free(ptr: *mut u8, length: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr,
            length.max(1),
        )));
    }
}

unsafe fn bytes<'a>(ptr: *const u8, length: usize) -> &'a [u8] {
    if ptr.is_null() || length == 0 {
        return &[];
    }
    std::slice::from_raw_parts(ptr, length)
}

unsafe fn text<'a>(ptr: *const u8, length: usize) -> &'a str {
    std::str::from_utf8(bytes(ptr, length)).unwrap_or("")
}

fn set_result(s: &mut Session, json: Json) {
    s.result = json.stringify().into_bytes();
}

// ------------------------------------------------------------------ project

#[no_mangle]
pub extern "C" fn nested_reset_project() {
    let mut s = session();
    s.project.sources.clear();
    s.project.assets.clear();
    s.lsp = None;
}

/// Add or replace a project file: `.nst` sources, anything else an asset.
///
/// # Safety
/// Pointers must reference live buffers of the given lengths.
#[no_mangle]
pub unsafe extern "C" fn nested_put_file(
    path: *const u8,
    path_len: usize,
    data: *const u8,
    data_len: usize,
) -> i32 {
    let path = text(path, path_len).to_owned();
    if path.is_empty() || data_len > 1024 * 1024 {
        return 0;
    }
    let data = bytes(data, data_len).to_vec();
    let mut s = session();
    if path.ends_with(".nst") {
        s.project
            .sources
            .insert(path, String::from_utf8_lossy(&data).into_owned());
    } else {
        if let Some(lsp) = &mut s.lsp {
            lsp.set_asset(&format!("file:///project/{path}"), data.clone());
        }
        s.project.assets.insert(path, data);
    }
    1
}

/// # Safety
/// The pointer must reference a live buffer of the given length.
#[no_mangle]
pub unsafe extern "C" fn nested_remove_file(path: *const u8, path_len: usize) {
    let path = text(path, path_len).to_owned();
    let mut s = session();
    s.project.sources.remove(&path);
    s.project.assets.remove(&path);
}

fn diagnostics_json(
    sources: &nested_compiler::SourceMap,
    items: &[nested_compiler::Diagnostic],
) -> Json {
    Json::Array(
        items
            .iter()
            .map(|d| {
                let file = sources.get(d.span.file);
                let (l1, c1) = file.position(d.span.start as usize);
                let (l2, c2) = file.position(d.span.end as usize);
                Json::object([
                    ("file", file.path.clone().into()),
                    ("line", l1.into()),
                    ("column", c1.into()),
                    ("endLine", l2.into()),
                    ("endColumn", c2.into()),
                    (
                        "severity",
                        match d.severity {
                            Severity::Error => "error",
                            Severity::Warning => "warning",
                            Severity::Hint => "hint",
                        }
                        .into(),
                    ),
                    ("message", d.message.clone().into()),
                ])
            })
            .collect(),
    )
}

/// Compile the project from `entry`; returns 1 on success. The report is
/// JSON in the result buffer and the ROM in the ROM buffer.
///
/// # Safety
/// The pointer must reference a live buffer of the given length.
#[no_mangle]
pub unsafe extern "C" fn nested_compile(entry: *const u8, entry_len: usize, optimize: u32) -> i32 {
    let entry = text(entry, entry_len).to_owned();
    let mut s = session();
    s.project.entry = if entry.is_empty() {
        "main.nst".into()
    } else {
        entry
    };
    let compiled = nested_compiler::compile(
        &s.project,
        Options {
            optimize: optimize != 0,
        },
    );
    match compiled {
        Ok(out) => {
            let l = &out.linked;
            let json = Json::object([
                ("ok", true.into()),
                ("board", out.board.name().into()),
                ("mapper", (out.board.ines() as usize).into()),
                ("romBytes", out.rom.len().into()),
                ("prgBytes", l.prg_bytes.into()),
                ("chrBytes", l.chr_bytes.into()),
                ("fixedUsed", l.fixed_used.into()),
                ("fixedSize", l.fixed_size.into()),
                ("bankSize", l.bank_size.into()),
                ("zeroPage", l.zp_used.into()),
                ("ram", l.ram_used.into()),
                (
                    "banks",
                    Json::Array(
                        l.banks_used
                            .iter()
                            .map(|(b, n)| {
                                Json::object([
                                    ("bank", (*b as usize).into()),
                                    ("used", (*n).into()),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "symbols",
                    Json::Array(
                        l.symbols
                            .iter()
                            .map(|sym| {
                                Json::object([
                                    ("name", sym.name.clone().into()),
                                    ("address", (sym.address as usize).into()),
                                    ("size", sym.size.into()),
                                    ("kind", sym.kind.clone().into()),
                                    (
                                        "bank",
                                        sym.bank
                                            .map(|b| Json::Number(b as f64))
                                            .unwrap_or(Json::Null),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "lines",
                    Json::Array(
                        l.listing
                            .iter()
                            .filter_map(|(bank, item)| {
                                let (file, start) = item.src?;
                                let f = out
                                    .analysis
                                    .sources
                                    .get(nested_compiler::source::FileId(file));
                                let (line, _) = f.position(start as usize);
                                Some(Json::object([
                                    ("address", (item.address as usize).into()),
                                    (
                                        "bank",
                                        bank.map(|b| Json::Number(b as f64)).unwrap_or(Json::Null),
                                    ),
                                    ("file", f.path.clone().into()),
                                    ("line", line.into()),
                                ]))
                            })
                            .collect(),
                    ),
                ),
                (
                    "diagnostics",
                    diagnostics_json(&out.analysis.sources, &out.warnings),
                ),
                (
                    "passes",
                    Json::Array(
                        out.passes
                            .iter()
                            .map(|p| {
                                Json::object([
                                    ("name", p.name.clone().into()),
                                    ("text", p.text.clone().into()),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]);
            s.rom = out.rom.clone();
            set_result(&mut s, json);
            1
        }
        Err(f) => {
            let json = Json::object([
                ("ok", false.into()),
                ("diagnostics", diagnostics_json(&f.sources, &f.diagnostics)),
                (
                    "passes",
                    Json::Array(
                        f.passes
                            .iter()
                            .map(|p| {
                                Json::object([
                                    ("name", p.name.clone().into()),
                                    ("text", p.text.clone().into()),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]);
            s.rom.clear();
            set_result(&mut s, json);
            0
        }
    }
}

/// Format a source file; the result buffer holds the formatted text.
///
/// # Safety
/// The pointer must reference a live buffer of the given length.
#[no_mangle]
pub unsafe extern "C" fn nested_format(src: *const u8, len: usize, tab: u32) -> i32 {
    let src = text(src, len);
    let indent = if tab == 0 {
        "\t".to_owned()
    } else {
        " ".repeat(tab as usize)
    };
    let out =
        nested_compiler::fmt::format(src, &nested_compiler::fmt::Style { indent, width: 100 });
    session().result = out.into_bytes();
    1
}

/// Handle one LSP JSON-RPC message; the result buffer holds a JSON array
/// of replies and notifications.
///
/// # Safety
/// The pointer must reference a live buffer of the given length.
#[no_mangle]
pub unsafe extern "C" fn nested_lsp(ptr: *const u8, length: usize) -> i32 {
    let input = text(ptr, length);
    let mut s = session();
    let request = match Json::parse(input) {
        Ok(r) => r,
        Err(e) => {
            set_result(
                &mut s,
                Json::Array(vec![Json::object([("error", e.into())])]),
            );
            return 0;
        }
    };
    if s.lsp.is_none() {
        let mut lsp = LanguageServer::new();
        let assets: Vec<(String, Vec<u8>)> = s
            .project
            .assets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (k, v) in assets {
            lsp.set_asset(&format!("file:///project/{k}"), v);
        }
        s.lsp = Some(lsp);
    }
    let replies = s.lsp.as_mut().unwrap().handle(request);
    set_result(&mut s, Json::Array(replies));
    1
}

#[no_mangle]
pub extern "C" fn nested_result_ptr() -> *const u8 {
    session().result.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_result_len() -> usize {
    session().result.len()
}
#[no_mangle]
pub extern "C" fn nested_rom_ptr() -> *const u8 {
    session().rom.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_rom_len() -> usize {
    session().rom.len()
}

// ----------------------------------------------------------------- emulator

/// # Safety
/// The pointer must reference a live buffer of the given length.
#[no_mangle]
pub unsafe extern "C" fn nested_load_rom(ptr: *const u8, length: usize, sample_rate: f64) -> i32 {
    let rom = bytes(ptr, length).to_vec();
    let mut s = session();
    match Emulator::new(rom, sample_rate) {
        Ok(e) => {
            s.emulator = Some(e);
            1
        }
        Err(e) => {
            set_result(&mut s, Json::object([("error", e.into())]));
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn nested_frame(buttons: u32) -> i32 {
    let mut s = session();
    let Some(emu) = s.emulator.as_mut() else {
        return 0;
    };
    match emu.frame(buttons as u8) {
        Ok(audio) => {
            let ram: Vec<u8> = (0..2048u16).map(|a| emu.read_ram(a)).collect();
            s.audio = audio;
            s.ram = ram;
            1
        }
        Err(e) => {
            set_result(&mut s, Json::object([("error", e.into())]));
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn nested_pixels_ptr() -> *const u8 {
    let s = session();
    match &s.emulator {
        Some(e) => e.pixels().as_ptr(),
        None => std::ptr::null(),
    }
}
#[no_mangle]
pub extern "C" fn nested_audio_ptr() -> *const f32 {
    session().audio.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_audio_len() -> usize {
    session().audio.len()
}
#[no_mangle]
pub extern "C" fn nested_ram_ptr() -> *const u8 {
    session().ram.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_pc() -> u32 {
    session().emulator.as_ref().map_or(0, |e| e.pc() as u32)
}

/// Snapshot PPU memory (16 KiB) and sprite memory (256 bytes).
#[no_mangle]
pub extern "C" fn nested_capture_ppu() -> u32 {
    let mut s = session();
    let Some(emu) = s.emulator.as_mut() else {
        return 0;
    };
    let ppu = emu.ppu_memory();
    let oam = emu.oam();
    let (ctrl, mask) = emu.ppu_control();
    s.ppu = ppu;
    s.oam = oam;
    (ctrl as u32) | (mask as u32) << 8
}
#[no_mangle]
pub extern "C" fn nested_ppu_ptr() -> *const u8 {
    session().ppu.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_oam_ptr() -> *const u8 {
    session().oam.as_ptr()
}

#[no_mangle]
pub extern "C" fn nested_write_ram(address: u32, value: u32) {
    if let Some(e) = session().emulator.as_mut() {
        e.write_ram(address as u16 & 0x7ff, value as u8);
    }
}

/// Save the machine state into the result buffer.
#[no_mangle]
pub extern "C" fn nested_save_state() -> i32 {
    let mut s = session();
    let Some(e) = s.emulator.as_ref() else {
        return 0;
    };
    let state = e.save_state();
    s.result = state;
    1
}

/// # Safety
/// The pointer must reference a live buffer of the given length.
#[no_mangle]
pub unsafe extern "C" fn nested_load_state(ptr: *const u8, length: usize) -> i32 {
    let data = bytes(ptr, length);
    let mut s = session();
    let Some(e) = s.emulator.as_mut() else {
        return 0;
    };
    e.load_state(data).is_ok() as i32
}
