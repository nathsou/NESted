import { Wasm } from './wasm';
let wasm: Wasm;
self.onmessage = async (event: MessageEvent) => {
  const message = event.data;
  try {
    if (message.kind === 'init') {
      wasm = new Wasm(await WebAssembly.instantiate(message.module as WebAssembly.Module, {}));
      self.postMessage({ kind: 'ready' });
    } else if (message.kind === 'load') {
      const ok = wasm.input(new Uint8Array(message.rom), (p, n) => wasm.exports.nested_load_rom(p, n, message.sampleRate));
      if (!ok) throw new Error(JSON.stringify(wasm.result()));
      self.postMessage({ kind: 'loaded' });
    } else if (message.kind === 'frame') {
      const start = performance.now();
      for (let i = 0; i < (message.count ?? 1); i++) {
        if (!wasm.exports.nested_frame(message.buttons)) throw new Error(JSON.stringify(wasm.result()));
      }
      const e = wasm.exports;
      const pixels = new Uint8Array(e.memory.buffer, e.nested_pixels_ptr(), 256 * 240 * 3).slice().buffer;
      const audio = new Float32Array(e.memory.buffer, e.nested_audio_ptr(), e.nested_audio_len()).slice().buffer;
      const ram = new Uint8Array(e.memory.buffer, e.nested_ram_ptr(), 2048).slice().buffer;
      self.postMessage({ kind: 'frame', pixels, audio, ram, pc: e.nested_pc(), elapsed: performance.now() - start, count: message.count ?? 1 }, { transfer: [pixels, audio, ram] });
    } else if (message.kind === 'reset') { wasm.exports.nested_reset(); self.postMessage({ kind: 'reset' }); }
  } catch (error) { self.postMessage({ kind: 'error', message: String(error) }); }
};
