// A thin wrapper over the plain C ABI exported by crates/wasm.
export interface Exports extends WebAssembly.Exports {
  memory: WebAssembly.Memory;
  nested_alloc(length: number): number;
  nested_free(ptr: number, length: number): void;
  nested_reset_project(): void;
  nested_put_file(path: number, pathLength: number, data: number, dataLength: number): number;
  nested_remove_file(path: number, pathLength: number): void;
  nested_compile(entry: number, entryLength: number, optimize: number): number;
  nested_format(ptr: number, length: number, tab: number): number;
  nested_lsp(ptr: number, length: number): number;
  nested_result_ptr(): number;
  nested_result_len(): number;
  nested_rom_ptr(): number;
  nested_rom_len(): number;
  nested_load_rom(ptr: number, length: number, sampleRate: number): number;
  nested_frame(buttons: number): number;
  nested_pixels_ptr(): number;
  nested_audio_ptr(): number;
  nested_audio_len(): number;
  nested_ram_ptr(): number;
  nested_pc(): number;
  nested_capture_ppu(): number;
  nested_ppu_ptr(): number;
  nested_oam_ptr(): number;
  nested_write_ram(address: number, value: number): void;
  nested_save_state(): number;
  nested_load_state(ptr: number, length: number): number;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export class Wasm {
  readonly e: Exports;
  constructor(instance: WebAssembly.Instance) {
    this.e = instance.exports as Exports;
  }
  /** Copy data into module memory for the duration of `call`. */
  with<T>(data: string | Uint8Array, call: (ptr: number, length: number) => T): T {
    const bytes = typeof data === 'string' ? encoder.encode(data) : data;
    const ptr = this.e.nested_alloc(bytes.length);
    if (!ptr) throw new Error('WebAssembly allocation failed');
    new Uint8Array(this.e.memory.buffer, ptr, bytes.length).set(bytes);
    try {
      return call(ptr, bytes.length);
    } finally {
      this.e.nested_free(ptr, bytes.length);
    }
  }
  bytes(ptr: number, length: number): Uint8Array {
    return new Uint8Array(this.e.memory.buffer, ptr, length).slice();
  }
  result(): Uint8Array {
    return this.bytes(this.e.nested_result_ptr(), this.e.nested_result_len());
  }
  text(): string {
    return decoder.decode(this.result());
  }
  json<T = unknown>(): T {
    return JSON.parse(this.text()) as T;
  }
}

export interface Diagnostic {
  file: string;
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  severity: 'error' | 'warning' | 'hint';
  message: string;
}

export interface SymbolInfo {
  name: string;
  address: number;
  size: number;
  kind: string;
  bank: number | null;
}

export interface Report {
  ok: boolean;
  board?: string;
  mapper?: number;
  romBytes?: number;
  prgBytes?: number;
  chrBytes?: number;
  fixedUsed?: number;
  fixedSize?: number;
  bankSize?: number;
  zeroPage?: number;
  ram?: number;
  banks?: { bank: number; used: number }[];
  symbols?: SymbolInfo[];
  lines?: { address: number; bank: number | null; file: string; line: number }[];
  diagnostics: Diagnostic[];
  passes: { name: string; text: string }[];
}
