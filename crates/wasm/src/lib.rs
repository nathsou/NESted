//! Plain WebAssembly ABI: no bindgen or JavaScript runtime dependencies.
//! Allocations passed in by the host remain host-owned until nested_free.
use nested_compiler::{
    json::Json,
    lsp::{diagnostics, LanguageServer},
    Assets, Options,
};
use nested_runtime::Emulator;
use std::sync::Mutex;
struct Session {
    assets: Assets,
    result: Vec<u8>,
    rom: Vec<u8>,
    lsp: Option<LanguageServer>,
    emulator: Option<Emulator>,
    audio: Vec<f32>,
    ram: Vec<u8>,
}
static SESSION: Mutex<Session> = Mutex::new(Session {
    assets: Assets::new(),
    result: Vec::new(),
    rom: Vec::new(),
    lsp: None,
    emulator: None,
    audio: Vec::new(),
    ram: Vec::new(),
});
#[no_mangle]
pub extern "C" fn nested_version() -> u32 {
    1
}
#[no_mangle]
pub extern "C" fn nested_alloc(length: usize) -> *mut u8 {
    if length > 16 * 1024 * 1024 {
        return std::ptr::null_mut();
    }
    let mut buffer = vec![0u8; length].into_boxed_slice();
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}
/// # Safety
/// The pointer must originate from nested_alloc with the exact same length.
#[no_mangle]
pub unsafe extern "C" fn nested_free(ptr: *mut u8, length: usize) {
    if !ptr.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr, length,
        )));
    }
}
unsafe fn input<'a>(ptr: *const u8, length: usize) -> Result<&'a str, String> {
    if ptr.is_null() || length > 16 * 1024 * 1024 {
        return Err("Invalid input buffer".into());
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, length)).map_err(|e| e.to_string())
}
/// # Safety
/// Inputs must point to valid live WASM buffers of the stated lengths.
#[no_mangle]
pub unsafe extern "C" fn nested_asset(
    name: *const u8,
    name_len: usize,
    data: *const u8,
    data_len: usize,
) -> i32 {
    let Ok(name) = input(name, name_len) else {
        return 0;
    };
    if data.is_null() || data_len > 1024 * 1024 {
        return 0;
    }
    SESSION.lock().unwrap().assets.insert(
        name.into(),
        std::slice::from_raw_parts(data, data_len).to_vec(),
    );
    1
}
#[no_mangle]
pub extern "C" fn nested_clear_assets() {
    SESSION.lock().unwrap().assets.clear();
}
/// # Safety
/// Source must point to a UTF-8 buffer allocated in this module.
#[no_mangle]
pub unsafe extern "C" fn nested_compile(ptr: *const u8, length: usize, optimize: u32) -> i32 {
    let source = match input(ptr, length) {
        Ok(s) => s,
        Err(e) => {
            SESSION.lock().unwrap().result =
                Json::object([("ok", false.into()), ("error", e.into())])
                    .stringify()
                    .into_bytes();
            return 0;
        }
    };
    let mut state = SESSION.lock().unwrap();
    match nested_compiler::compile(
        source,
        &state.assets,
        Options {
            optimize: optimize != 0,
        },
    ) {
        Ok(compiled) => {
            state.result = compiled.json().stringify().into_bytes();
            state.rom = compiled.rom;
            1
        }
        Err(errors) => {
            state.result = Json::object([
                ("ok", false.into()),
                ("diagnostics", diagnostics(source, &errors)),
            ])
            .stringify()
            .into_bytes();
            state.rom.clear();
            0
        }
    }
}
/// # Safety
/// Request must point to a UTF-8 JSON-RPC message in this module's memory.
#[no_mangle]
pub unsafe extern "C" fn nested_lsp(ptr: *const u8, length: usize) -> i32 {
    let request = match input(ptr, length).and_then(Json::parse) {
        Ok(r) => r,
        Err(e) => {
            SESSION.lock().unwrap().result =
                Json::object([("error", e.into())]).stringify().into_bytes();
            return 0;
        }
    };
    let mut state = SESSION.lock().unwrap();
    let server = state.lsp.get_or_insert_with(LanguageServer::new);
    let messages = server.handle(request);
    state.result = Json::Array(messages).stringify().into_bytes();
    1
}
#[no_mangle]
pub extern "C" fn nested_result_ptr() -> *const u8 {
    SESSION.lock().unwrap().result.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_result_len() -> usize {
    SESSION.lock().unwrap().result.len()
}
#[no_mangle]
pub extern "C" fn nested_rom_ptr() -> *const u8 {
    SESSION.lock().unwrap().rom.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_rom_len() -> usize {
    SESSION.lock().unwrap().rom.len()
}
/// # Safety
/// ROM bytes must live in this module's memory for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn nested_load_rom(ptr: *const u8, length: usize, sample_rate: f64) -> i32 {
    if ptr.is_null() || length > 4 * 1024 * 1024 {
        return 0;
    }
    let bytes = std::slice::from_raw_parts(ptr, length).to_vec();
    let mut state = SESSION.lock().unwrap();
    match Emulator::new(bytes, sample_rate) {
        Ok(n) => {
            state.emulator = Some(n);
            state.audio.clear();
            state.ram = vec![0; 2048];
            1
        }
        Err(e) => {
            state.result = Json::object([("error", e.into())]).stringify().into_bytes();
            0
        }
    }
}
#[no_mangle]
pub extern "C" fn nested_frame(buttons: u32) -> i32 {
    let mut state = SESSION.lock().unwrap();
    let Some(n) = state.emulator.as_mut() else {
        return 0;
    };
    match n.frame(buttons as u8) {
        Ok(audio) => {
            state.audio = audio;
            let state = &mut *state;
            let n = state.emulator.as_mut().unwrap();
            for i in 0..2048 {
                state.ram[i] = n.read_ram(i as u16);
            }
            1
        }
        Err(e) => {
            state.result = Json::object([("error", e.into())]).stringify().into_bytes();
            0
        }
    }
}
#[no_mangle]
pub extern "C" fn nested_pixels_ptr() -> *const u8 {
    SESSION
        .lock()
        .unwrap()
        .emulator
        .as_ref()
        .map(|n| n.pixels().as_ptr())
        .unwrap_or(std::ptr::null())
}
#[no_mangle]
pub extern "C" fn nested_audio_ptr() -> *const f32 {
    SESSION.lock().unwrap().audio.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_audio_len() -> usize {
    SESSION.lock().unwrap().audio.len()
}
#[no_mangle]
pub extern "C" fn nested_ram_ptr() -> *const u8 {
    SESSION.lock().unwrap().ram.as_ptr()
}
#[no_mangle]
pub extern "C" fn nested_pc() -> u32 {
    SESSION
        .lock()
        .unwrap()
        .emulator
        .as_ref()
        .map(|n| n.cpu.pc as u32)
        .unwrap_or(0)
}
#[no_mangle]
pub extern "C" fn nested_reset() {
    if let Some(n) = SESSION.lock().unwrap().emulator.as_mut() {
        n.cpu.soft_reset();
    }
}
