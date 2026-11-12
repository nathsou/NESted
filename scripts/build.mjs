// Build the compiler for WASM and native, package the example projects
// and build the playground.
import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync, copyFileSync, readFileSync } from 'node:fs';
import { resolve, join, relative } from 'node:path';

const root = resolve(import.meta.dirname, '..');
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run('cargo', ['build', '--release', '--locked', '-p', 'nested-wasm', '--target', 'wasm32-unknown-unknown']);
run('cargo', ['build', '--release', '--locked', '-p', 'nested-cli']);
const pub = resolve(root, 'playground/public');
copyFileSync(resolve(root, 'target/wasm32-unknown-unknown/release/nested_wasm.wasm'), resolve(pub, 'nested.wasm'));

// Examples: every games/<name> directory, with a manifest of its files.
const examples = resolve(pub, 'examples');
rmSync(examples, { recursive: true, force: true });
mkdirSync(examples, { recursive: true });
const manifest = [];
const ORDER = ['glide', 'sprout', 'chess'];
for (const name of readdirSync(resolve(root, 'games')).sort((a, b) => ORDER.indexOf(a) - ORDER.indexOf(b))) {
  const dir = resolve(root, 'games', name);
  if (!statSync(dir).isDirectory()) continue;
  const files = [];
  const walk = (d) => {
    for (const f of readdirSync(d)) {
      const p = join(d, f);
      if (statSync(p).isDirectory()) walk(p);
      else if (f.endsWith('.nst') || relative(dir, p).startsWith('assets/') && !f.endsWith('.png')) files.push(relative(dir, p).replaceAll('\\', '/'));
    }
  };
  walk(dir);
  cpSync(dir, resolve(examples, name), { recursive: true, filter: (src) => !src.endsWith('.nes') && !src.endsWith('.py') && !src.endsWith('.png') && !src.endsWith('.txt') && !src.includes('__pycache__') });
  const main = readFileSync(join(dir, 'main.nst'), 'utf8');
  const title = /title\s*=\s*"([^"]+)"/.exec(main)?.[1] ?? name;
  const mapper = /mapper\s*=\s*(\w+)/.exec(main)?.[1] ?? 'nrom';
  const summary = main.split('\n').filter((l) => l.startsWith('//')).map((l) => l.replace(/^\/\/\s?/, '')).join(' ').split('. ')[0]
    .replace(/^[A-Z][A-Z ]+\s*(:|—)\s*/, '')
    .replace(/^./, (c) => c.toUpperCase());
  manifest.push({ id: name, title, mapper, summary, files });
  // A screenshot a few seconds in, past the title screen, for the project picker.
  run(resolve(root, 'target/release/nested'), ['run', `games/${name}`, '240', '--input', 'start@90', '--shot', resolve(examples, name, 'preview.png')]);
}
writeFileSync(resolve(examples, 'index.json'), JSON.stringify(manifest, null, 2));
run('npm', ['run', 'typecheck']);
run('npm', ['run', 'build:web']);
