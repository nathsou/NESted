export interface Exports extends WebAssembly.Exports {
  memory: WebAssembly.Memory;
  nested_alloc(length: number): number;
  nested_free(ptr: number, length: number): void;
  nested_asset(name: number, nameLength: number, data: number, dataLength: number): number;
  nested_compile(ptr: number, length: number, optimize: number): number;
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
  nested_reset(): void;
}
export class Wasm {
  readonly exports: Exports;
  readonly encoder = new TextEncoder();
  readonly decoder = new TextDecoder();
  constructor(instance: WebAssembly.Instance) { this.exports = instance.exports as Exports; }
  input<T>(data: string | Uint8Array, call: (pointer: number, length: number) => T): T {
    const bytes = typeof data === 'string' ? this.encoder.encode(data) : data;
    const pointer = this.exports.nested_alloc(bytes.length);
    if (!pointer) throw new Error('WASM input allocation failed');
    new Uint8Array(this.exports.memory.buffer, pointer, bytes.length).set(bytes);
    try { return call(pointer, bytes.length); }
    finally { this.exports.nested_free(pointer, bytes.length); }
  }
  result(): unknown {
    const e = this.exports;
    return JSON.parse(this.decoder.decode(new Uint8Array(e.memory.buffer, e.nested_result_ptr(), e.nested_result_len())));
  }
  asset(name: string, data: Uint8Array): void {
    this.input(name, (p, n) => this.input(data, (d, size) => {
      if (!this.exports.nested_asset(p, n, d, size)) throw new Error(`Could not load asset ${name}`);
    }));
  }
  rpc(body: unknown): unknown { this.input(JSON.stringify(body), (p, n) => this.exports.nested_lsp(p, n)); return this.result(); }
}
export interface Build {
  ok: boolean;
  diagnostics?: Diagnostic[];
  error?: string;
  romBytes: number; codeBytes: number; prgBytes: number; chrBytes: number;
  ramBytes: number; zeroPageBytes: number; frameBytes: number; mapper: number;
  passes: { name: string; text: string }[];
  symbols: Record<string, number>;
  memory: { name: string; address: number; size: number; kind: string }[];
  optimizations: Record<string, number>;
  budgets: Record<string, number>;
}
export interface Position { line: number; character: number }
export interface Range { start: Position; end: Position }
export interface Diagnostic { range: Range; severity: number; message: string }
export interface Rpc { jsonrpc?: string; id?: number; method?: string; params?: unknown; result?: unknown; error?: { message: string } }
