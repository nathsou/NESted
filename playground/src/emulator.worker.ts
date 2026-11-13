// The NES emulator, compiled from Rust to WebAssembly, in its own worker so
// the editor stays responsive.
import { Wasm } from './wasm';

let wasm: Wasm;
let rom: Uint8Array | null = null;
let sampleRate = 48000;
let generation = 0;

function load(): boolean {
  if (!rom) return false;
  const bytes = rom;
  return wasm.with(bytes, (p, n) => wasm.e.nested_load_rom(p, n, sampleRate)) === 1;
}

self.onmessage = async (event: MessageEvent) => {
  const m = event.data;
  try {
    switch (m.kind) {
      case 'init':
        wasm = new Wasm(await WebAssembly.instantiate(m.module as WebAssembly.Module, {}));
        self.postMessage({ kind: 'ready' });
        break;
      case 'load':
        generation = m.generation;
        rom = new Uint8Array(m.rom);
        sampleRate = m.sampleRate;
        if (!load()) throw new Error(wasm.text());
        self.postMessage({ kind: 'loaded', generation });
        break;
      case 'reset':
        if (m.generation === generation) load();
        break;
      case 'frames': {
        if (m.generation !== generation || !rom) return;
        const e = wasm.e;
        const chunks: Float32Array[] = [];
        let error: string | null = null;
        for (let i = 0; i < m.count; i++) {
          if (!e.nested_frame(m.buttons)) {
            error = wasm.text();
            break;
          }
          chunks.push(new Float32Array(e.memory.buffer, e.nested_audio_ptr(), e.nested_audio_len()).slice());
        }
        const pixels = new Uint8Array(e.memory.buffer, e.nested_pixels_ptr(), 256 * 240 * 3).slice();
        const total = chunks.reduce((n, c) => n + c.length, 0);
        const audio = new Float32Array(total);
        let at = 0;
        for (const c of chunks) {
          audio.set(c, at);
          at += c.length;
        }
        const ram = new Uint8Array(e.memory.buffer, e.nested_ram_ptr(), 2048).slice();
        const reply: Record<string, unknown> = { kind: 'frame', generation, pixels: pixels.buffer, audio: audio.buffer, ram: ram.buffer, pc: e.nested_pc(), count: m.count, error };
        const transfer: ArrayBuffer[] = [pixels.buffer, audio.buffer, ram.buffer];
        if (m.ppu) {
          const regs = e.nested_capture_ppu();
          const ppu = wasm.bytes(e.nested_ppu_ptr(), 0x4000);
          const oam = wasm.bytes(e.nested_oam_ptr(), 256);
          reply.ppu = ppu.buffer;
          reply.oam = oam.buffer;
          reply.regs = regs;
          transfer.push(ppu.buffer as ArrayBuffer, oam.buffer as ArrayBuffer);
        }
        self.postMessage(reply, { transfer });
        break;
      }
      case 'poke':
        wasm.e.nested_write_ram(m.address, m.value);
        break;
      case 'save':
        if (wasm.e.nested_save_state()) {
          const state = wasm.result();
          self.postMessage({ kind: 'saved', slot: m.slot, state: state.buffer }, { transfer: [state.buffer] });
        }
        break;
      case 'restore':
        wasm.with(new Uint8Array(m.state), (p, n) => wasm.e.nested_load_state(p, n));
        break;
    }
  } catch (error) {
    self.postMessage({ kind: 'error', generation, message: String(error) });
  }
};
