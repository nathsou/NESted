import { Wasm } from './wasm';
let wasm: Wasm;
self.onmessage = async (event: MessageEvent) => {
  const message = event.data;
  try {
    if (message.kind === 'init') {
      wasm = new Wasm(await WebAssembly.instantiate(message.module as WebAssembly.Module, {}));
      self.postMessage({ kind: 'ready' });
    } else if (message.kind === 'asset') {
      wasm.asset(message.name, new Uint8Array(message.data));
      self.postMessage({ kind: 'asset', id: message.id });
    } else if (message.kind === 'rpc') {
      self.postMessage({ kind: 'rpc', messages: wasm.rpc(message.body) });
    } else if (message.kind === 'compile') {
      const start = performance.now();
      const ok = wasm.input(message.source, (p, n) => wasm.exports.nested_compile(p, n, message.optimize ? 1 : 0));
      const report = wasm.result();
      const e = wasm.exports;
      const rom = ok ? new Uint8Array(e.memory.buffer, e.nested_rom_ptr(), e.nested_rom_len()).slice().buffer : new ArrayBuffer(0);
      self.postMessage({ kind: 'compiled', id: message.id, version: message.version, report, rom, elapsed: performance.now() - start }, { transfer: [rom] });
    }
  } catch (error) { self.postMessage({ kind: 'error', message: String(error), id: message.id }); }
};
