// The compiler and language server, running the Rust crates compiled to
// WebAssembly. Messages carry project files, build requests and LSP traffic.
import { Wasm } from './wasm';

let wasm: Wasm;

self.onmessage = async (event: MessageEvent) => {
  const m = event.data;
  try {
    switch (m.kind) {
      case 'init':
        wasm = new Wasm(await WebAssembly.instantiate(m.module as WebAssembly.Module, {}));
        self.postMessage({ kind: 'ready' });
        break;
      case 'files': {
        if (m.reset) wasm.e.nested_reset_project();
        for (const f of m.files as { path: string; data: string | Uint8Array | null }[]) {
          if (f.data === null) {
            wasm.with(f.path, (p, n) => wasm.e.nested_remove_file(p, n));
          } else {
            const data = f.data;
            wasm.with(f.path, (p, n) => wasm.with(data, (d, dn) => wasm.e.nested_put_file(p, n, d, dn)));
          }
        }
        break;
      }
      case 'compile': {
        const start = performance.now();
        const ok = wasm.with(m.entry, (p, n) => wasm.e.nested_compile(p, n, m.optimize ? 1 : 0));
        const report = wasm.json();
        const rom = ok ? wasm.bytes(wasm.e.nested_rom_ptr(), wasm.e.nested_rom_len()).buffer : null;
        self.postMessage({ kind: 'compiled', id: m.id, report, rom, ms: performance.now() - start }, rom ? { transfer: [rom] } : {});
        break;
      }
      case 'format': {
        wasm.with(m.text, (p, n) => wasm.e.nested_format(p, n, m.tab));
        self.postMessage({ kind: 'formatted', id: m.id, text: wasm.text() });
        break;
      }
      case 'lsp': {
        wasm.with(JSON.stringify(m.message), (p, n) => wasm.e.nested_lsp(p, n));
        self.postMessage({ kind: 'lsp', messages: wasm.json() });
        break;
      }
    }
  } catch (error) {
    self.postMessage({ kind: 'error', id: m.id, message: String(error) });
  }
};
