import { Wasm } from './wasm';
let wasm: Wasm;
let cartridgeId = 0;
self.onmessage = async (event: MessageEvent) => {
  const message = event.data;
  try {
    if (message.kind === 'init') {
      wasm = new Wasm(await WebAssembly.instantiate(message.module as WebAssembly.Module, {}));
      self.postMessage({ kind: 'ready' });
    } else if (message.kind === 'load') {
      cartridgeId = message.id;
      const ok = wasm.input(new Uint8Array(message.rom), (p, n) => wasm.exports.nested_load_rom(p, n, message.sampleRate));
      if (!ok) throw new Error(JSON.stringify(wasm.result()));
      self.postMessage({ kind: 'loaded', id: cartridgeId });
    } else if (message.kind === 'frame') {
      if (message.id !== cartridgeId) return;
      const start = performance.now();
      const chunks: Float32Array[] = [];
      for (let i = 0; i < (message.count ?? 1); i++) {
        if (!wasm.exports.nested_frame(message.buttons)) throw new Error(JSON.stringify(wasm.result()));
        chunks.push(new Float32Array(wasm.exports.memory.buffer, wasm.exports.nested_audio_ptr(), wasm.exports.nested_audio_len()).slice());
      }
      const e = wasm.exports;
      const pixels = new Uint8Array(e.memory.buffer, e.nested_pixels_ptr(), 256 * 240 * 3).slice().buffer;
      const samples = new Float32Array(chunks.reduce((count, chunk) => count + chunk.length, 0));
      let offset = 0;
      for (const chunk of chunks) { samples.set(chunk, offset); offset += chunk.length; }
      const audio = samples.buffer;
      const ram = new Uint8Array(e.memory.buffer, e.nested_ram_ptr(), 2048).slice().buffer;
      self.postMessage({ kind: 'frame', id: cartridgeId, pixels, audio, ram, pc: e.nested_pc(), elapsed: performance.now() - start, count: message.count ?? 1 }, { transfer: [pixels, audio, ram] });
    } else if (message.kind === 'reset' && message.id === cartridgeId) { wasm.exports.nested_reset(); self.postMessage({ kind: 'reset', id: cartridgeId }); }
  } catch (error) { self.postMessage({ kind: 'error', id: cartridgeId, message: String(error) }); }
};
