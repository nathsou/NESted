// Screenshots for the README: each example mid-game, captured from the
// playground's emulator, plus the workbench on desktop and phone.
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { Browser } from './browser.mjs';

const root = resolve(import.meta.dirname, '..');
const out = resolve(root, 'docs/screenshots');
mkdirSync(out, { recursive: true });
const url = process.env.PLAYGROUND_URL;
const server = url ? null : spawn('npm', ['exec', 'vite', '--', 'preview', 'playground', '--host', '127.0.0.1', '--port', '4175', '--strictPort'], { stdio: 'ignore' });
const b = new Browser();
const n = 'window.__nested';
const [A, , , START, UP, DOWN, LEFT, RIGHT] = [1, 2, 4, 8, 16, 32, 64, 128];

async function screen(name) {
  const data = await b.evaluate(`(() => { const c = document.createElement('canvas'); c.width = 512; c.height = 480; const x = c.getContext('2d'); x.imageSmoothingEnabled = false; x.drawImage(document.getElementById('screen'), 0, 0, 512, 480); return c.toDataURL('image/png'); })()`);
  writeFileSync(resolve(out, `${name}.png`), Buffer.from(data.split(',')[1], 'base64'));
}
const step = (buttons, frames) => b.evaluate(`${n}.step(${buttons}, ${frames})`);
const tap = async (buttons) => {
  await step(buttons, 3);
  await step(0, 3);
};
async function open(id) {
  await b.evaluate(`${n}.openExample('${id}')`);
  await b.wait(`${n}.ready && ${n}.project.example === '${id}'`);
  await b.evaluate(`${n}.pause()`);
}

try {
  if (server) {
    const start = Date.now();
    for (;;) {
      try {
        if ((await fetch('http://127.0.0.1:4175/')).ok) break;
      } catch {}
      if (Date.now() - start > 30000) throw new Error('Preview server startup failed');
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  await b.start(url ?? 'http://127.0.0.1:4175/', 1600, 1000);
  await b.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: 'dark' }] });
  await b.wait(`${n}?.ready`);

  // Glide: pick level 4 on the title screen and make a few slides.
  await open('glide');
  await step(0, 60);
  for (let i = 0; i < 3; i++) await tap(RIGHT);
  await tap(START);
  await step(0, 40);
  for (const d of [RIGHT, DOWN]) {
    await tap(d);
    await step(0, 40);
  }
  await screen('glide');

  // Sprout: run right, hopping over things.
  await open('sprout');
  await step(0, 40);
  await tap(START);
  await step(0, 30);
  for (let i = 0; i < 6; i++) {
    await step(RIGHT, 22);
    await step(RIGHT | A, 16);
  }
  await step(RIGHT, 8);
  await screen('sprout');

  // Nova: start, then fly and fire through the first waves.
  await open('nova');
  await step(0, 60);
  await tap(START);
  await step(A, 260);
  await step(A | LEFT, 24);
  await step(A, 60);
  await screen('nova');

  // Chess: play 1.e4 against level 1 and wait for the reply.
  await open('chess');
  await tap(UP);
  for (let i = 0; i < 4; i++) await tap(LEFT);
  await tap(START);
  await step(0, 30);
  for (const d of [A, UP, UP, A]) await tap(d);
  await step(0, 240);
  await screen('chess');

  // The workbench itself.
  await open('sprout');
  await b.evaluate(`${n}.resume()`);
  await b.evaluate(`${n}.panel('ppu')`);
  await new Promise((r) => setTimeout(r, 2500));
  await b.screenshot(resolve(out, 'workbench.png'));
  await b.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 2, mobile: true });
  await b.evaluate(`${n}.pane('play')`);
  await new Promise((r) => setTimeout(r, 1000));
  await b.screenshot(resolve(out, 'mobile.png'));
  console.log(`Screenshots written to ${out}`);
} finally {
  await b.close();
  server?.kill('SIGTERM');
}
