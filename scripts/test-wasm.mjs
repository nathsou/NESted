// The WebAssembly build must produce byte-identical ROMs to the native
// compiler, run every example with real audio, and serve the language
// server; the native server must work through the VS Code transport.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync, mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { resolve, join, relative } from 'node:path';
import { Wasm } from '../playground/src/wasm.ts';

const root = resolve(import.meta.dirname, '..');
const module = await WebAssembly.compile(readFileSync(resolve(root, 'playground/public/nested.wasm')));
const wasm = new Wasm(await WebAssembly.instantiate(module, {}));
const e = wasm.e;

function files(dir) {
  const out = [];
  const walk = (d) => {
    for (const f of readdirSync(d)) {
      const p = join(d, f);
      if (statSync(p).isDirectory()) walk(p);
      else if (f.endsWith('.nst') || relative(dir, p).startsWith('assets')) out.push(relative(dir, p));
    }
  };
  walk(dir);
  return out;
}

const results = [];
mkdirSync(resolve(root, 'artifacts'), { recursive: true });
for (const game of ['glide', 'sprout', 'nova', 'chess']) {
  const dir = resolve(root, 'games', game);
  e.nested_reset_project();
  for (const path of files(dir)) {
    assert.equal(wasm.with(path, (p, n) => wasm.with(readFileSync(join(dir, path)), (d, dn) => e.nested_put_file(p, n, d, dn))), 1, path);
  }
  const start = performance.now();
  const ok = wasm.with('main.nst', (p, n) => e.nested_compile(p, n, 1));
  const report = wasm.json();
  assert.equal(ok, 1, JSON.stringify(report.diagnostics));
  const compileMs = performance.now() - start;
  const rom = wasm.bytes(e.nested_rom_ptr(), e.nested_rom_len());
  const native = spawnSync(resolve(root, 'target/release/nested'), ['build', `games/${game}`, '-o', `artifacts/${game}.nes`], { cwd: root, encoding: 'utf8' });
  assert.equal(native.status, 0, native.stderr);
  assert.deepEqual(Buffer.from(rom), readFileSync(resolve(root, `artifacts/${game}.nes`)), `${game}: native and WASM ROMs must match byte for byte`);
  assert.ok(report.symbols.some((s) => s.kind.startsWith('fn') && s.size > 0), 'functions report their code size');

  assert.equal(wasm.with(rom, (p, n) => e.nested_load_rom(p, n, 48000)), 1);
  let squares = 0;
  let samples = 0;
  const runStart = performance.now();
  for (let frame = 0; frame < 300; frame++) {
    // Press Start once to leave the title screen.
    assert.equal(e.nested_frame(frame >= 90 && frame < 94 ? 8 : 0), 1, wasm.text());
    for (const value of new Float32Array(e.memory.buffer, e.nested_audio_ptr(), e.nested_audio_len())) {
      assert.ok(Number.isFinite(value));
      squares += value * value;
      samples++;
    }
  }
  const fps = 300000 / (performance.now() - runStart);
  const rms = Math.sqrt(squares / samples);
  if (game !== 'chess') assert.ok(rms > 0.001, `${game} must play music`);
  results.push({ game, board: report.board, rom: report.romBytes, fixed: `${report.fixedUsed}/${report.fixedSize}`, ram: report.ram, compileMs: Math.round(compileMs), emulatorFps: Math.round(fps), rms: rms.toFixed(4) });

  // Save states round-trip.
  assert.equal(e.nested_save_state(), 1);
  const state = wasm.result();
  e.nested_frame(0);
  assert.equal(wasm.with(state, (p, n) => e.nested_load_state(p, n)), 1);
}
console.table(results);

// The language server inside the WASM module.
e.nested_reset_project();
let sequence = 1;
const rpc = (method, params) => {
  wasm.with(JSON.stringify({ jsonrpc: '2.0', id: sequence++, method, params }), (p, n) => e.nested_lsp(p, n));
  return wasm.json();
};
const notify = (method, params) => {
  wasm.with(JSON.stringify({ jsonrpc: '2.0', method, params }), (p, n) => e.nested_lsp(p, n));
  return wasm.json();
};
assert.equal(rpc('initialize', {})[0].result.capabilities.positionEncoding, 'utf-16');
const uri = 'file:///project/main.nst';
const source = 'var score: u8 = 0;\nfn update() {\n    score += 1;\n}\n';
const published = notify('textDocument/didOpen', { textDocument: { uri, languageId: 'nested', version: 1, text: source } });
assert.ok(published.some((m) => m.method === 'textDocument/publishDiagnostics'));
assert.ok(rpc('textDocument/completion', { textDocument: { uri }, position: { line: 2, character: 4 } })[0].result.some((i) => i.label === 'score'));
assert.equal(rpc('textDocument/rename', { textDocument: { uri }, position: { line: 0, character: 5 }, newName: 'points' })[0].result.changes[uri].length, 2);
notify('textDocument/didChange', { textDocument: { uri, version: 2 }, contentChanges: [{ text: 'var score: u8 = 0;\nfn update() {\n    score += \n' }] });
assert.ok(rpc('textDocument/completion', { textDocument: { uri }, position: { line: 2, character: 13 } })[0].result.some((i) => i.label === 'score'), 'completion survives unfinished statements');

// The native server through the VS Code transport.
const require = createRequire(import.meta.url);
const { LspTransport } = require('../extension/transport.cjs');
const transport = new LspTransport(resolve(root, 'target/release/nested'));
try {
  const init = await transport.request('initialize', {});
  assert.equal(init.serverInfo.name, 'NESted');
  transport.notify('initialized', {});
  transport.notify('textDocument/didOpen', { textDocument: { uri: 'file:///tmp/typing.nst', languageId: 'nested', version: 1, text: source } });
  const hover = await transport.request('textDocument/hover', { textDocument: { uri: 'file:///tmp/typing.nst' }, position: { line: 0, character: 5 } });
  assert.match(hover.contents.value, /score/);
  const tokens = await transport.request('textDocument/semanticTokens/full', { textDocument: { uri: 'file:///tmp/typing.nst' } });
  assert.ok(tokens.data.length > 10);
} finally {
  await transport.close();
}
console.log('WASM/native parity, music, save states, the WASM language server and the VS Code transport passed.');
