// End-to-end checks of the production playground in headless Chromium.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { Browser } from './browser.mjs';

const server = spawn('npm', ['exec', 'vite', '--', 'preview', 'playground', '--host', '127.0.0.1', '--port', '4174', '--strictPort'], { stdio: 'ignore' });
const browser = new Browser();
const n = 'window.__nested';
try {
  const start = Date.now();
  for (;;) {
    try {
      if ((await fetch('http://127.0.0.1:4174/')).ok) break;
    } catch {}
    if (Date.now() - start > 30000) throw new Error('Preview server startup failed');
    await new Promise((r) => setTimeout(r, 100));
  }
  await browser.start('http://127.0.0.1:4174/', 1440, 900);
  await browser.wait(`${n}?.ready && ${n}.frames > 30`);

  // Every example builds, runs without dropping video updates, and is listed.
  const examples = await browser.evaluate(`${n}.examples().map(e => e.id)`);
  assert.deepEqual(examples, ['glide', 'sprout', 'chess']);
  for (const id of examples) {
    await browser.evaluate(`${n}.openExample('${id}')`);
    await browser.wait(`${n}.ready && ${n}.project.example === '${id}' && ${n}.frames > 60`);
    const r = await browser.evaluate(`({ ok: ${n}.report.ok, passes: ${n}.report.passes.length, crashed: ${n}.crashed })`);
    assert.equal(r.ok, true, id);
    assert.equal(r.crashed, '', id);
    assert.ok(r.passes >= 5, id);
  }

  // Starter projects for every board build and run.
  for (const board of ['nrom', 'cnrom', 'uxrom', 'axrom', 'bnrom', 'gxrom', 'colordreams', 'mmc1', 'mmc3']) {
    const report = await browser.evaluate(`${n}.newProject('${board}').then(r => ({ ok: r.ok, board: r.board, d: r.diagnostics.map(d => d.message) }))`);
    assert.equal(report.ok, true, `${board}: ${report.d.join('; ')}`);
    await browser.wait(`${n}.frames > 20`);
    assert.equal(await browser.evaluate(`${n}.crashed`), '', board);
  }

  // The language server: completion, hover, definition across modules, rename.
  await browser.evaluate(`${n}.newProject('nrom')`);
  const uri = 'file:///project/main.nst';
  const source = await browser.evaluate(`${n}.source()`);
  const line = source.split('\n').findIndex((l) => l.includes('fn update()'));
  const completion = await browser.evaluate(`${n}.rpc('textDocument/completion', { textDocument: { uri: '${uri}' }, position: { line: ${line + 1}, character: 4 } })`);
  assert.ok(completion.some((i) => i.label === 'sprite16'), 'completion lists project functions');
  const hover = await browser.evaluate(`${n}.rpc('textDocument/hover', { textDocument: { uri: '${uri}' }, position: { line: ${line}, character: 4 } })`);
  assert.match(hover.contents.value, /update/);

  // A second module, used from main.
  await browser.evaluate(`(() => { window.prompt = () => 'util.nst'; document.querySelector('[data-action="new-file"]').click(); })()`);
  await browser.wait(`${n}.project.files.some(f => f.path === 'util.nst')`);
  await browser.evaluate(`${n}.open('main.nst')`);
  await browser.evaluate(`${n}.setSource(${n}.source().replace('use std::audio;', 'use std::audio;\\nuse util;').replace('x -= 2;', 'x -= util::hello() / 21;'))`);
  const src2 = await browser.evaluate(`${n}.source()`);
  const useLine = src2.split('\n').findIndex((l) => l.includes('util::hello'));
  const def = await browser.evaluate(`${n}.rpc('textDocument/definition', { textDocument: { uri: '${uri}' }, position: { line: ${useLine}, character: ${src2.split('\n')[useLine].indexOf('hello') + 1} } })`);
  assert.equal(def.uri, 'file:///project/util.nst');
  assert.equal((await browser.evaluate(`${n}.build()`)).ok, true);

  // Errors show in Problems with a badge, and the build reports them.
  await browser.evaluate(`${n}.setSource(${n}.source().replace('sprite16(x, y, 0, 0);', 'sprite16(x, y, 0);'))`);
  const failed = await browser.evaluate(`${n}.build()`);
  assert.equal(failed.ok, false);
  await browser.wait(`document.querySelector('.problems .diag.error') !== null`);
  assert.ok(Number(await browser.evaluate(`document.querySelector('[data-panel="problems"] .count').textContent`)) >= 1);

  // Formatting goes through the compiler worker.
  await browser.evaluate(`${n}.setSource('fn init(){let a:u8=1;}\\nfn update(){}')`);
  await browser.evaluate(`document.querySelector('[data-action="format"]').click()`);
  await browser.wait(`${n}.source().includes('    let a: u8 = 1;')`);

  // Inspectors render with machine state.
  await browser.evaluate(`${n}.openExample('glide')`);
  await browser.wait(`${n}.ready && ${n}.frames > 60`);
  for (const id of ['watch', 'ppu', 'memory', 'listing', 'passes', 'problems']) {
    await browser.evaluate(`${n}.panel('${id}')`);
    await new Promise((r) => setTimeout(r, 150));
    assert.ok(await browser.evaluate(`document.querySelector('#inspector-body .panel').textContent.length > 10`), id);
  }
  await browser.evaluate(`${n}.panel('watch')`);
  await browser.wait(`document.querySelectorAll('.watch-row').length > 10`);

  // Layouts never overflow horizontally; the phone layout switches panes.
  for (const width of [320, 390, 820, 1024, 1440]) {
    await browser.send('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: width < 801 });
    await new Promise((r) => setTimeout(r, 100));
    assert.equal(await browser.evaluate('document.documentElement.scrollWidth > innerWidth'), false, `viewport ${width} overflows`);
  }
  await browser.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
  for (const [pane, selector] of [['play', '.player'], ['inspect', '.inspector'], ['code', '.editor-pane']]) {
    await browser.evaluate(`document.querySelector('.mobile-nav [data-pane="${pane}"]').click()`);
    assert.equal(await browser.evaluate(`getComputedStyle(document.querySelector('${selector}')).display`), 'flex', pane);
  }
  assert.equal(browser.errors.length, 0, JSON.stringify(browser.errors));
  console.log('Playground: three examples, nine board templates, LSP across modules, diagnostics, formatting, inspectors and responsive layouts passed.');
} finally {
  await browser.close();
  server.kill('SIGTERM');
}
