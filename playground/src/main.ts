// The NESted playground: projects with several files, a code editor backed
// by the language server, a build report, the emulator and inspectors.
import './style.css';
import { Editor, type LspClient } from './editor';
import { el, bytes, hex, Problems, Memory, Watch, Ppu, Listing, Passes, type Panel, type Machine, NES_COLORS } from './inspectors';
import { BOARDS, blankModule, deleteProject, listProjects, loadExample, loadExamples, saveProject, templateProject, type ExampleInfo, type Project, type ProjectFile } from './project';
import type { Report } from './wasm';

const FRAME_MS = 1000 / 60.0988;
const BUTTON = { A: 1, B: 2, SELECT: 4, START: 8, UP: 16, DOWN: 32, LEFT: 64, RIGHT: 128 } as const;
const KEYS: Record<string, number> = {
  ArrowUp: BUTTON.UP,
  ArrowDown: BUTTON.DOWN,
  ArrowLeft: BUTTON.LEFT,
  ArrowRight: BUTTON.RIGHT,
  KeyW: BUTTON.UP,
  KeyS: BUTTON.DOWN,
  KeyA: BUTTON.LEFT,
  KeyD: BUTTON.RIGHT,
  KeyX: BUTTON.A,
  KeyK: BUTTON.A,
  KeyZ: BUTTON.B,
  KeyJ: BUTTON.B,
  Enter: BUTTON.START,
  ShiftRight: BUTTON.SELECT,
  ShiftLeft: BUTTON.SELECT,
  Backspace: BUTTON.SELECT,
};

function store(key: string, value?: string): string | null {
  try {
    if (value === undefined) return localStorage.getItem(key);
    localStorage.setItem(key, value);
  } catch {
    // Storage can be unavailable; preferences are a convenience.
  }
  return null;
}

// -------------------------------------------------------------------- page

const app = document.getElementById('app')!;
app.innerHTML = `
<header class="topbar">
  <a class="brand" href="https://github.com/nathsou/NESted" target="_blank" rel="noopener" title="NESted on GitHub">
    <svg viewBox="0 0 32 32" aria-hidden="true"><rect width="32" height="32" rx="7" fill="var(--accent)"/><path d="M7 24V8h4l10 10V8h4v16h-4L11 14v10z" fill="var(--bg)"/></svg>
    <span>NESted</span>
  </a>
  <button id="project-button" class="project-button" aria-haspopup="dialog" title="Open a project (Ctrl+O)">
    <span id="project-name">Loading…</span><span id="project-board" class="badge"></span><svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>
  </button>
  <div class="spacer"></div>
  <div id="status" class="status" role="status" aria-live="polite"></div>
  <button id="build" class="primary" title="Build and run (Ctrl+Enter)" disabled>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 2.5v11l9-5.5z" fill="currentColor"/></svg><span>Run</span>
  </button>
  <div class="menu-wrap">
    <button id="more" class="icon" aria-haspopup="menu" aria-expanded="false" title="More">
      <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="3" cy="8" r="1.4" fill="currentColor"/><circle cx="8" cy="8" r="1.4" fill="currentColor"/><circle cx="13" cy="8" r="1.4" fill="currentColor"/></svg>
    </button>
    <div id="menu" class="menu" role="menu" hidden>
      <button role="menuitem" data-action="download">Download ROM (.nes)</button>
      <button role="menuitem" data-action="export">Download sources</button>
      <button role="menuitem" data-action="format">Format file <kbd>Shift+Alt+F</kbd></button>
      <label class="check" role="menuitemcheckbox"><input type="checkbox" id="optimize" checked> Optimize</label>
      <hr>
      <button role="menuitem" data-action="restore">Restore example</button>
      <button role="menuitem" data-action="shortcuts">Keyboard shortcuts</button>
      <a role="menuitem" href="https://github.com/nathsou/NESted/blob/main/docs/language.md" target="_blank" rel="noopener">Language guide ↗</a>
    </div>
  </div>
</header>
<main class="workspace" data-pane="code">
  <aside class="files" aria-label="Project files">
    <div class="files-head">
      <span>Files</span>
      <button class="icon small" data-action="new-file" title="New module" aria-label="New module"><svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.6"/></svg></button>
      <button class="icon small" data-action="upload" title="Add an asset (tiles, maps, data)" aria-label="Add an asset"><svg viewBox="0 0 16 16"><path d="M8 11V3M4.5 6.5 8 3l3.5 3.5M3 13h10" fill="none" stroke="currentColor" stroke-width="1.6"/></svg></button>
      <input type="file" id="upload" multiple hidden>
    </div>
    <ul id="file-list" class="file-list"></ul>
  </aside>
  <section class="editor-pane" aria-label="Editor">
    <div id="tabs" class="tabs" role="tablist"></div>
    <div id="editor" class="editor-host"></div>
    <div id="asset" class="asset-view" hidden></div>
  </section>
  <div class="divider" id="divider" role="separator" aria-orientation="vertical" tabindex="0" title="Drag to resize"></div>
  <section class="side" aria-label="Console">
    <div class="player">
      <div class="screen-wrap" id="screen-wrap">
        <canvas id="screen" width="256" height="240" tabindex="0" aria-label="NES screen. Click to play with the keyboard."></canvas>
        <div id="overlay" class="overlay"></div>
      </div>
      <div class="controls">
        <button id="pause" class="icon" title="Pause (Space when the screen has focus)" aria-label="Pause"></button>
        <button id="step" class="icon" title="Advance one frame" aria-label="Advance one frame"><svg viewBox="0 0 16 16"><path d="M3 3v10l7-5zM11 3h2v10h-2z" fill="currentColor"/></svg></button>
        <button id="reset" class="icon" title="Reset the console" aria-label="Reset"><svg viewBox="0 0 16 16"><path d="M3.5 8a4.5 4.5 0 1 0 1.4-3.3M3.5 2.5v3h3" fill="none" stroke="currentColor" stroke-width="1.6"/></svg></button>
        <span class="sep"></span>
        <button id="save-state" class="text" title="Save the machine state (F5)">Save</button>
        <button id="load-state" class="text" title="Restore the saved state (F8)" disabled>Load</button>
        <span class="spacer"></span>
        <button id="sound" class="icon" title="Sound" aria-label="Sound"></button>
        <button id="fullscreen" class="icon" title="Full screen" aria-label="Full screen"><svg viewBox="0 0 16 16"><path d="M2.5 6V2.5H6M10 2.5h3.5V6M13.5 10v3.5H10M6 13.5H2.5V10" fill="none" stroke="currentColor" stroke-width="1.6"/></svg></button>
      </div>
      <div class="pad" id="pad" aria-hidden="true">
        <div class="dpad">
          <button data-button="16" class="up"></button><button data-button="64" class="left"></button><button data-button="128" class="right"></button><button data-button="32" class="down"></button>
        </div>
        <div class="meta"><button data-button="4">SELECT</button><button data-button="8">START</button></div>
        <div class="ab"><button data-button="2">B</button><button data-button="1">A</button></div>
      </div>
      <p class="hint" id="input-hint">Click the screen, then play with <kbd>←↑↓→</kbd> <kbd>X</kbd> A <kbd>Z</kbd> B <kbd>Enter</kbd> Start <kbd>Shift</kbd> Select. Gamepads work too.</p>
    </div>
    <div class="inspector">
      <div class="inspector-tabs" role="tablist" id="inspector-tabs"></div>
      <div class="inspector-body" id="inspector-body"></div>
    </div>
  </section>
</main>
<nav class="mobile-nav" aria-label="Views">
  <button data-pane="code">Code</button><button data-pane="play">Play</button><button data-pane="inspect">Inspect</button>
</nav>
<dialog id="projects" class="projects" aria-label="Projects">
  <form method="dialog" class="dialog-head"><h2>Open a project</h2><button class="icon" value="close" aria-label="Close">✕</button></form>
  <div class="dialog-body">
    <section><h3>Example cartridges</h3><div id="examples" class="cards"></div></section>
    <section><h3>New project</h3>
      <div class="new-row"><label>Name <input id="new-name" value="My Game" maxlength="40"></label></div>
      <div id="boards" class="boards"></div>
    </section>
    <section id="mine-section"><h3>Your projects</h3><ul id="mine" class="mine"></ul></section>
  </div>
</dialog>
<dialog id="shortcuts" class="shortcuts">
  <form method="dialog" class="dialog-head"><h2>Keyboard shortcuts</h2><button class="icon" value="close" aria-label="Close">✕</button></form>
  <div class="dialog-body"><table>
    <tr><td><kbd>Ctrl</kbd> <kbd>Enter</kbd> / <kbd>Ctrl</kbd> <kbd>S</kbd></td><td>Build and run</td></tr>
    <tr><td><kbd>Ctrl</kbd> <kbd>O</kbd></td><td>Open a project</td></tr>
    <tr><td><kbd>F12</kbd> or <kbd>Ctrl</kbd>+click</td><td>Go to definition</td></tr>
    <tr><td><kbd>F2</kbd></td><td>Rename symbol</td></tr>
    <tr><td><kbd>Shift</kbd> <kbd>Alt</kbd> <kbd>F</kbd></td><td>Format file</td></tr>
    <tr><td><kbd>Ctrl</kbd> <kbd>Space</kbd></td><td>Complete</td></tr>
    <tr><td><kbd>Ctrl</kbd> <kbd>F</kbd></td><td>Find</td></tr>
    <tr><td><kbd>Esc</kbd></td><td>Leave the screen and return to the editor</td></tr>
    <tr><td><kbd>Space</kbd> (screen focused)</td><td>Pause or resume</td></tr>
    <tr><td><kbd>F5</kbd> / <kbd>F8</kbd></td><td>Save / load state</td></tr>
  </table></div>
</dialog>`;

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const screen = $<HTMLCanvasElement>('screen');
const ctx = screen.getContext('2d', { alpha: false })!;
const image = ctx.createImageData(256, 240);
const overlay = $('overlay');
const statusEl = $('status');
const buildButton = $<HTMLButtonElement>('build');
const workspace = document.querySelector<HTMLElement>('.workspace')!;

const ICONS = {
  pause: '<svg viewBox="0 0 16 16"><path d="M4 3h3v10H4zM9 3h3v10H9z" fill="currentColor"/></svg>',
  play: '<svg viewBox="0 0 16 16"><path d="M4 2.5v11l9-5.5z" fill="currentColor"/></svg>',
  sound: '<svg viewBox="0 0 16 16"><path d="M2 6h3l4-3v10l-4-3H2z" fill="currentColor"/><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.5 3.5a6 6 0 0 1 0 9" fill="none" stroke="currentColor" stroke-width="1.4"/></svg>',
  mute: '<svg viewBox="0 0 16 16"><path d="M2 6h3l4-3v10l-4-3H2z" fill="currentColor"/><path d="M11 6l4 4M15 6l-4 4" stroke="currentColor" stroke-width="1.4"/></svg>',
};

// ----------------------------------------------------------------- workers

const compiler = new Worker(new URL('./compiler.worker.ts', import.meta.url), { type: 'module' });
const emulator = new Worker(new URL('./emulator.worker.ts', import.meta.url), { type: 'module' });

let sequence = 1;
const pending = new Map<number, (value: unknown) => void>();
const lspPending = new Map<number, (value: unknown) => void>();

const lsp: LspClient = {
  request(method, params) {
    const id = sequence++;
    return new Promise((resolve) => {
      lspPending.set(id, resolve);
      compiler.postMessage({ kind: 'lsp', message: { jsonrpc: '2.0', id, method, params } });
    });
  },
  notify(method, params) {
    compiler.postMessage({ kind: 'lsp', message: { jsonrpc: '2.0', method, params } });
  },
};

function call<T>(message: Record<string, unknown>, transfer: Transferable[] = []): Promise<T> {
  const id = sequence++;
  return new Promise((resolve) => {
    pending.set(id, resolve as (v: unknown) => void);
    compiler.postMessage({ ...message, id }, transfer);
  });
}

compiler.onmessage = (event) => {
  const m = event.data;
  switch (m.kind) {
    case 'lsp':
      for (const msg of m.messages as { id?: number; method?: string; params?: { uri: string; diagnostics: never[] }; result?: unknown }[]) {
        if (msg.id !== undefined && lspPending.has(msg.id)) {
          lspPending.get(msg.id)!(msg.result ?? null);
          lspPending.delete(msg.id);
        } else if (msg.method === 'textDocument/publishDiagnostics' && msg.params) {
          const path = pathOf(msg.params.uri);
          if (path) {
            editor.setDiagnostics(path, msg.params.diagnostics);
            liveProblems.set(path, msg.params.diagnostics.length);
            renderFiles();
          }
        }
      }
      break;
    case 'compiled':
    case 'formatted':
    case 'error':
      if (m.id !== undefined && pending.has(m.id)) {
        pending.get(m.id)!(m);
        pending.delete(m.id);
      } else if (m.kind === 'error') console.error(m.message);
      break;
  }
};

const uri = (path: string) => `file:///project/${path}`;
function pathOf(u: string): string | null {
  return u.startsWith('file:///project/') ? decodeURIComponent(u.slice('file:///project/'.length)) : null;
}

// ------------------------------------------------------------------ editor

let project: Project | null = null;
let openTabs: string[] = [];
let current = '';
const liveProblems = new Map<string, number>();
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let dirtySinceBuild = false;

const editor = new Editor($('editor'), {
  lsp,
  uri,
  pathOf,
  open: (loc) => openFile(loc.path, loc.line, loc.character),
  changed(path, text) {
    const f = project?.files.find((x) => x.path === path);
    if (!f) return;
    f.text = text;
    dirtySinceBuild = true;
    scheduleSave();
    showStatus();
  },
  async format(text) {
    const r = await call<{ text: string }>({ kind: 'format', text, tab: 4 });
    return r.text ?? text;
  },
  applyEdits(changes) {
    for (const [u, edits] of Object.entries(changes)) {
      const path = pathOf(u);
      if (path && project?.files.some((f) => f.path === path)) editor.edit(path, edits);
    }
  },
});

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => project && saveProject(project), 400);
}

// ------------------------------------------------------------------- files

function fileIcon(path: string) {
  if (path.endsWith('.nst')) return 'nst';
  if (path.endsWith('.chr')) return 'chr';
  return 'bin';
}

function renderFiles() {
  const list = $('file-list');
  list.replaceChildren();
  if (!project) return;
  for (const f of project.files) {
    const problems = liveProblems.get(f.path) ?? 0;
    const item = el(
      'li',
      { class: `file ${f.path === current ? 'active' : ''}`, title: f.path },
      el(
        'button',
        { class: 'file-open', onclick: () => openFile(f.path) },
        el('span', { class: `ficon ${fileIcon(f.path)}`, text: fileIcon(f.path) === 'nst' ? '{}' : fileIcon(f.path) === 'chr' ? '▦' : '01' }),
        el('span', { class: 'fname', text: f.path }),
        problems ? el('span', { class: 'fproblems', text: String(problems) }) : null,
        f.data ? el('span', { class: 'fsize', text: bytes(f.data.length) }) : null,
      ),
      f.path === 'main.nst'
        ? null
        : el(
            'span',
            { class: 'factions' },
            el('button', { class: 'icon tiny', title: `Rename ${f.path}`, 'aria-label': `Rename ${f.path}`, text: '✎', onclick: () => renameFile(f.path) }),
            el('button', { class: 'icon tiny', title: `Delete ${f.path}`, 'aria-label': `Delete ${f.path}`, text: '✕', onclick: () => removeFile(f.path) }),
          ),
    );
    list.append(item);
  }
}

function renderTabs() {
  const tabs = $('tabs');
  tabs.replaceChildren();
  for (const path of openTabs) {
    const tab = el(
      'div',
      { class: `tab ${path === current ? 'active' : ''}`, role: 'tab', 'aria-selected': String(path === current) },
      el('button', { class: 'tab-name', text: path.split('/').pop()!, title: path, onclick: () => openFile(path) }),
      openTabs.length > 1 ? el('button', { class: 'tab-close', 'aria-label': `Close ${path}`, text: '×', onclick: () => closeTab(path) }) : null,
    );
    tabs.append(tab);
  }
}

function closeTab(path: string) {
  const i = openTabs.indexOf(path);
  openTabs = openTabs.filter((p) => p !== path);
  if (current === path && openTabs.length) openFile(openTabs[Math.max(0, i - 1)]);
  else renderTabs();
}

function openFile(path: string, line?: number, character = 0) {
  if (!project) return;
  const f = project.files.find((x) => x.path === path);
  if (!f) {
    // A definition inside the standard library: show it read-only.
    if (path.startsWith('std/') || path.startsWith('<std>')) return;
    return;
  }
  if (!openTabs.includes(path)) openTabs.push(path);
  current = path;
  project.open = path;
  const asset = $('asset');
  const host = $('editor');
  if (f.text !== undefined) {
    asset.hidden = true;
    host.hidden = false;
    editor.show(path);
    if (line !== undefined) editor.goto(line, character);
  } else {
    host.hidden = true;
    asset.hidden = false;
    showAsset(asset, f);
  }
  if (innerWidth <= 800) setPane('code');
  renderTabs();
  renderFiles();
}

function showAsset(root: HTMLElement, f: ProjectFile) {
  const data = f.data!;
  root.replaceChildren(el('div', { class: 'asset-head' }, el('strong', { text: f.path }), el('span', { text: ` ${bytes(data.length)}` })));
  if (f.path.endsWith('.chr') || data.length === 8192 || data.length === 4096) {
    const tiles = Math.floor(data.length / 16);
    const rows = Math.ceil(tiles / 16);
    const canvas = el('canvas', { width: 128, height: rows * 8, class: 'pixel chr' });
    const c = canvas.getContext('2d')!;
    const img = c.createImageData(128, rows * 8);
    const shades = [NES_COLORS[0x0f], NES_COLORS[0x00], NES_COLORS[0x10], NES_COLORS[0x30]];
    for (let t = 0; t < tiles; t++)
      for (let y = 0; y < 8; y++)
        for (let x = 0; x < 8; x++) {
          const v = ((data[t * 16 + y] >> (7 - x)) & 1) | (((data[t * 16 + y + 8] >> (7 - x)) & 1) << 1);
          const o = (((t >> 4) * 8 + y) * 128 + (t & 15) * 8 + x) * 4;
          const s = shades[v];
          img.data.set([s[0], s[1], s[2], 255], o);
        }
    c.putImageData(img, 0, 0);
    root.append(el('p', { class: 'muted', text: `${tiles} tiles, shown in grey. Reference this file with asset("${f.path}") or as the cartridge's chr.` }), canvas);
  } else {
    const lines: string[] = [];
    for (let i = 0; i < Math.min(data.length, 1024); i += 16) {
      const row = Array.from(data.subarray(i, i + 16));
      lines.push(`${hex(i, 4)}  ${row.map((b) => hex(b)).join(' ').padEnd(48)}  ${row.map((b) => (b >= 32 && b < 127 ? String.fromCharCode(b) : '.')).join('')}`);
    }
    if (data.length > 1024) lines.push('…');
    root.append(el('p', { class: 'muted', text: `Use it with asset("${f.path}") or asset("${f.path}", start..end).` }), el('pre', { class: 'code hexdump', text: lines.join('\n') }));
  }
}

async function newFile() {
  if (!project) return;
  let name = prompt('Module path, e.g. enemies.nst or world/map.nst', 'module.nst');
  if (!name) return;
  name = name.trim().replace(/^\/+/, '');
  if (!name.endsWith('.nst')) name += '.nst';
  if (!/^[a-z_][a-z0-9_]*(\/[a-z_][a-z0-9_]*)*\.nst$/.test(name)) {
    alert('Module paths use lowercase letters, digits and underscores, with / between folders.');
    return;
  }
  if (project.files.some((f) => f.path === name)) return openFile(name);
  const file = blankModule(name);
  project.files.push(file);
  sortFiles();
  editor.addFile(name, file.text!);
  scheduleSave();
  openFile(name);
}

async function upload(files: FileList) {
  if (!project) return;
  for (const file of Array.from(files)) {
    if (file.size > 1024 * 1024) {
      alert(`${file.name} is larger than 1 MiB.`);
      continue;
    }
    const isSource = file.name.endsWith('.nst');
    const path = isSource ? file.name : `assets/${file.name}`;
    const data = new Uint8Array(await file.arrayBuffer());
    const existing = project.files.find((f) => f.path === path);
    const entry: ProjectFile = isSource ? { path, text: new TextDecoder().decode(data) } : { path, data };
    if (existing) Object.assign(existing, entry);
    else project.files.push(entry);
    compiler.postMessage({ kind: 'files', reset: false, files: [{ path, data: entry.text ?? entry.data! }] });
    if (isSource) {
      if (existing) editor.setText(path, entry.text!);
      else editor.addFile(path, entry.text!);
    }
  }
  sortFiles();
  scheduleSave();
  renderFiles();
}

async function removeFile(path: string) {
  if (!project || !confirm(`Delete ${path}?`)) return;
  project.files = project.files.filter((f) => f.path !== path);
  openTabs = openTabs.filter((p) => p !== path);
  liveProblems.delete(path);
  scheduleSave();
  await loadWorkspace(false);
  openFile(openTabs[0] ?? 'main.nst');
}

async function renameFile(path: string) {
  if (!project) return;
  const next = prompt(`Rename ${path} to`, path)?.trim();
  if (!next || next === path || project.files.some((f) => f.path === next)) return;
  if (path.endsWith('.nst') !== next.endsWith('.nst')) {
    alert('Keep the extension: sources end in .nst.');
    return;
  }
  const f = project.files.find((x) => x.path === path)!;
  f.path = next;
  openTabs = openTabs.map((p) => (p === path ? next : p));
  liveProblems.delete(path);
  sortFiles();
  scheduleSave();
  await loadWorkspace(false);
  openFile(next);
}

function sortFiles() {
  const rank = (p: string) => (p === 'main.nst' ? 0 : p.endsWith('.nst') ? 1 : 2);
  project?.files.sort((a, b) => rank(a.path) - rank(b.path) || a.path.localeCompare(b.path));
}

/** Send every file to the compiler worker and reopen the language server. */
async function loadWorkspace(keepTabs: boolean) {
  if (!project) return;
  compiler.postMessage({ kind: 'files', reset: true, files: project.files.map((f) => ({ path: f.path, data: f.text ?? f.data! })) });
  editor.clear();
  current = '';
  editor.path = '';
  await lsp.request('initialize', { capabilities: {}, rootUri: 'file:///project/' });
  lsp.notify('initialized', {});
  for (const f of project.files) if (f.text !== undefined) editor.addFile(f.path, f.text);
  if (!keepTabs) openTabs = openTabs.filter((p) => project!.files.some((f) => f.path === p));
  renderFiles();
}

// ------------------------------------------------------------------- build

let report: Report | null = null;
let rom: Uint8Array | null = null;
let building = false;

function showStatus(text?: string, kind = '') {
  if (text !== undefined) {
    statusEl.textContent = text;
    statusEl.className = `status ${kind}`;
  }
  statusEl.classList.toggle('stale', dirtySinceBuild && !building);
  statusEl.title = dirtySinceBuild ? 'Edited since the last build. Press Run to rebuild.' : '';
}

async function build(run = true): Promise<Report | null> {
  if (!project || building) return report;
  building = true;
  buildButton.disabled = true;
  editor.flush();
  showStatus('Building…', 'busy');
  // Sources may have changed; send them before compiling.
  compiler.postMessage({ kind: 'files', reset: false, files: project.files.filter((f) => f.text !== undefined).map((f) => ({ path: f.path, data: f.text! })) });
  const optimize = $<HTMLInputElement>('optimize').checked;
  const result = await call<{ report: Report; rom: ArrayBuffer | null; ms: number; message?: string; kind: string }>({ kind: 'compile', entry: 'main.nst', optimize });
  building = false;
  buildButton.disabled = false;
  dirtySinceBuild = false;
  if (result.kind === 'error') {
    showStatus('Compiler crashed', 'error');
    console.error(result.message);
    return null;
  }
  report = result.report;
  for (const p of panels) p.build?.(report);
  updateBadges();
  if (report.ok && result.rom) {
    rom = new Uint8Array(result.rom);
    const warnings = report.diagnostics.filter((d) => d.severity === 'warning').length;
    showStatus(`${report.board} · ${bytes(report.romBytes ?? rom.length)} · ${Math.round(result.ms)} ms${warnings ? ` · ${warnings} warning${warnings > 1 ? 's' : ''}` : ''}`, 'ok');
    if (run) await boot();
  } else {
    const errors = report.diagnostics.filter((d) => d.severity === 'error').length;
    showStatus(`${errors || 'Build'} error${errors === 1 ? '' : 's'}`, 'error');
    showPanel('problems');
    const first = report.diagnostics.find((d) => d.severity === 'error');
    if (first && project.files.some((f) => f.path === first.file)) openFile(first.file, first.line, first.column);
  }
  return report;
}

// ---------------------------------------------------------------- emulator

let generation = 0;
let running = false;
let inFlight = false;
let frames = 0;
let lastTime = 0;
let accumulator = 0;
let saved: ArrayBuffer | null = null;
let machine: Machine | null = null;
let stepRequest = 0;
let crashed = '';
const pressed = new Set<string>();
let touchButtons = 0;

const audioContext: AudioContext | null = (() => {
  try {
    return new AudioContext({ latencyHint: 'interactive' });
  } catch {
    return null;
  }
})();
let audioNode: AudioWorkletNode | null = null;
let soundOn = store('nested.sound') !== 'off';
const audioReady = (async () => {
  if (!audioContext) return;
  try {
    await audioContext.audioWorklet.addModule(new URL('audio-worklet.js', document.baseURI).href);
    audioNode = new AudioWorkletNode(audioContext, 'nes-audio', { outputChannelCount: [2] });
    audioNode.connect(audioContext.destination);
  } catch (error) {
    console.warn('Audio is unavailable', error);
  }
})();

function unlockAudio() {
  if (soundOn && audioContext && audioContext.state !== 'running') void audioContext.resume();
}

async function boot() {
  if (!rom) return;
  await audioReady;
  generation++;
  crashed = '';
  saved = null;
  $<HTMLButtonElement>('load-state').disabled = true;
  const copy = rom.slice().buffer;
  emulator.postMessage({ kind: 'load', generation, rom: copy, sampleRate: audioContext?.sampleRate ?? 48000 }, [copy]);
  audioNode?.port.postMessage({ reset: true });
  frames = 0;
  setRunning(true);
}

function setRunning(on: boolean) {
  running = on;
  accumulator = 0;
  lastTime = performance.now();
  $('pause').innerHTML = on ? ICONS.pause : ICONS.play;
  $('pause').title = on ? 'Pause (Space when the screen has focus)' : 'Resume (Space when the screen has focus)';
  $('pause').setAttribute('aria-label', on ? 'Pause' : 'Resume');
  updateOverlay();
}

function updateOverlay() {
  overlay.replaceChildren();
  overlay.className = 'overlay';
  if (crashed) {
    overlay.classList.add('show', 'error');
    overlay.append(el('strong', { text: 'The console stopped' }), el('span', { text: crashed }));
  } else if (!rom) {
    overlay.classList.add('show');
    overlay.append(el('span', { text: report && !report.ok ? 'Fix the errors and press Run.' : 'Press Run to build the cartridge.' }));
  } else if (!running) {
    overlay.classList.add('show', 'paused');
    overlay.append(el('strong', { text: 'Paused' }), el('span', { text: `Frame ${frames}` }));
  }
}

function gamepadButtons(): number {
  let b = 0;
  for (const pad of navigator.getGamepads?.() ?? []) {
    if (!pad) continue;
    const on = (i: number) => pad.buttons[i]?.pressed;
    if (on(0)) b |= BUTTON.A;
    if (on(1) || on(2)) b |= BUTTON.B;
    if (on(8)) b |= BUTTON.SELECT;
    if (on(9)) b |= BUTTON.START;
    if (on(12) || pad.axes[1] < -0.5) b |= BUTTON.UP;
    if (on(13) || pad.axes[1] > 0.5) b |= BUTTON.DOWN;
    if (on(14) || pad.axes[0] < -0.5) b |= BUTTON.LEFT;
    if (on(15) || pad.axes[0] > 0.5) b |= BUTTON.RIGHT;
  }
  return b;
}

function buttons(): number {
  let b = touchButtons | gamepadButtons();
  for (const k of pressed) b |= KEYS[k] ?? 0;
  // Opposite directions at once confuse many games.
  if ((b & (BUTTON.LEFT | BUTTON.RIGHT)) === (BUTTON.LEFT | BUTTON.RIGHT)) b &= ~(BUTTON.LEFT | BUTTON.RIGHT);
  if ((b & (BUTTON.UP | BUTTON.DOWN)) === (BUTTON.UP | BUTTON.DOWN)) b &= ~(BUTTON.UP | BUTTON.DOWN);
  return b;
}

function wantsPpu(): boolean {
  return activePanel?.wantsPpu === true && inspectorVisible();
}

function requestFrames(count: number, input = buttons()) {
  inFlight = true;
  emulator.postMessage({ kind: 'frames', generation, buttons: input, count, ppu: wantsPpu() && (frames % 6 === 0 || !running) });
}

function tick(now: number) {
  requestAnimationFrame(tick);
  if (!rom || inFlight || crashed) return;
  if (stepRequest > 0) {
    const n = stepRequest;
    stepRequest = 0;
    requestFrames(n, stepButtons);
    return;
  }
  if (!running) return;
  accumulator += Math.min(100, now - lastTime);
  lastTime = now;
  const due = Math.floor(accumulator / FRAME_MS);
  if (due < 1) return;
  accumulator -= due * FRAME_MS;
  requestFrames(Math.min(due, 4));
}
requestAnimationFrame(tick);

let stepButtons = 0;
let stepResolve: (() => void) | null = null;

emulator.onmessage = (event) => {
  const m = event.data;
  switch (m.kind) {
    case 'frame': {
      if (m.generation !== generation) {
        inFlight = false;
        return;
      }
      inFlight = false;
      frames += m.count;
      const rgb = new Uint8Array(m.pixels);
      const out = image.data;
      for (let i = 0, j = 0; i < rgb.length; i += 3, j += 4) {
        out[j] = rgb[i];
        out[j + 1] = rgb[i + 1];
        out[j + 2] = rgb[i + 2];
        out[j + 3] = 255;
      }
      ctx.putImageData(image, 0, 0);
      if (soundOn && running && audioNode && audioContext?.state === 'running') audioNode.port.postMessage({ samples: new Float32Array(m.audio) });
      machine = { ram: new Uint8Array(m.ram), pc: m.pc, ppu: m.ppu ? new Uint8Array(m.ppu) : (machine?.ppu ?? null), oam: m.oam ? new Uint8Array(m.oam) : (machine?.oam ?? null), regs: m.regs ?? machine?.regs ?? 0 };
      if (m.error) {
        crashed = String(m.error).replace(/^\{"error":"|"\}$/g, '');
        setRunning(false);
      }
      if (inspectorVisible() && activePanel?.frame && (frames % 3 < m.count || !running)) activePanel.frame(machine);
      if (!running) updateOverlay();
      stepResolve?.();
      stepResolve = null;
      break;
    }
    case 'saved':
      saved = m.state;
      $<HTMLButtonElement>('load-state').disabled = false;
      flash('State saved');
      break;
    case 'error':
      inFlight = false;
      crashed = m.message;
      setRunning(false);
      break;
  }
};

function flash(text: string) {
  const note = el('div', { class: 'toast', text });
  $('screen-wrap').append(note);
  setTimeout(() => note.remove(), 1200);
}

// ---------------------------------------------------------------- panels

const panels: Panel[] = [
  new Problems((file, line, column) => openFile(file, line, column)),
  new Watch((address, value) => emulator.postMessage({ kind: 'poke', address, value })),
  new Ppu(),
  new Memory(),
  new Listing(
    (file, line) => openFile(file, line),
    () => report?.lines,
  ),
  new Passes(),
];
let activePanel: Panel | null = null;

function inspectorVisible(): boolean {
  return innerWidth > 800 || workspace.dataset.pane === 'inspect';
}

function updateBadges() {
  for (const p of panels) {
    const tab = document.querySelector<HTMLElement>(`[data-panel="${p.id}"] .count`);
    if (tab) {
      const b = p.badge?.() ?? '';
      tab.textContent = b;
      tab.hidden = !b;
      tab.classList.toggle('error', !!report?.diagnostics.some((d) => d.severity === 'error'));
    }
  }
}

function showPanel(id: string) {
  const p = panels.find((x) => x.id === id) ?? panels[0];
  activePanel = p;
  store('nested.panel', p.id);
  document.querySelectorAll<HTMLElement>('[data-panel]').forEach((t) => {
    t.classList.toggle('active', t.dataset.panel === p.id);
    t.setAttribute('aria-selected', String(t.dataset.panel === p.id));
  });
  $('inspector-body').replaceChildren(p.root);
  if (machine && p.frame) {
    if (p.wantsPpu && rom && !inFlight) requestFrames(0);
    p.frame(machine);
  }
}

for (const p of panels) {
  $('inspector-tabs').append(el('button', { 'data-panel': p.id, role: 'tab', onclick: () => showPanel(p.id) }, p.title, el('span', { class: 'count', hidden: true })));
}
showPanel(store('nested.panel') ?? 'problems');

// --------------------------------------------------------------- projects

let examples: ExampleInfo[] = [];

async function openProject(p: Project) {
  if (project) {
    clearTimeout(saveTimer);
    await saveProject(project);
  }
  project = p;
  sortFiles();
  liveProblems.clear();
  openTabs = [];
  rom = null;
  report = null;
  generation++;
  setRunning(false);
  ctx.fillStyle = '#000';
  ctx.fillRect(0, 0, 256, 240);
  $('project-name').textContent = p.name;
  $('project-board').textContent = BOARDS.find((b) => b.id === p.mapper)?.name ?? p.mapper;
  document.title = `${p.name} · NESted`;
  store('nested.last', p.id);
  history.replaceState(null, '', p.example ? `#example=${p.example}` : `#project=${p.id}`);
  document.querySelector<HTMLButtonElement>('[data-action="restore"]')!.hidden = !p.example;
  await loadWorkspace(false);
  openFile(p.open && p.files.some((f) => f.path === p.open) ? p.open : 'main.nst');
  buildButton.disabled = false;
  await build(true);
}

async function openExample(id: string, fresh = false) {
  const info = examples.find((e) => e.id === id);
  if (!info) return;
  if (!fresh) {
    const saved = (await listProjects()).find((p) => p.id === `example-${id}`);
    if (saved) return openProject(saved);
  }
  showStatus('Loading example…', 'busy');
  await openProject(await loadExample(info));
}

async function renderDialog() {
  const cards = $('examples');
  cards.replaceChildren(
    ...examples.map((e) =>
      el(
        'button',
        { class: 'card', onclick: () => ($<HTMLDialogElement>('projects').close(), openExample(e.id)) },
        el('img', { src: `examples/${e.id}/preview.png`, alt: '', loading: 'lazy', onerror: (ev: Event) => ((ev.target as HTMLElement).style.visibility = 'hidden') }),
        el('span', { class: 'card-title' }, el('strong', { text: e.title }), el('span', { class: 'badge', text: BOARDS.find((b) => b.id === e.mapper)?.name ?? e.mapper })),
        el('span', { class: 'card-text', text: e.summary }),
      ),
    ),
  );
  const boards = $('boards');
  boards.replaceChildren(
    ...BOARDS.map((b) =>
      el(
        'button',
        {
          class: 'board',
          onclick: async () => {
            const name = ($<HTMLInputElement>('new-name').value.trim() || 'My Game').slice(0, 40);
            $<HTMLDialogElement>('projects').close();
            const p = await templateProject(b.id, name);
            await saveProject(p);
            await openProject(p);
          },
        },
        el('strong', { text: b.name }),
        el('span', { text: b.summary }),
      ),
    ),
  );
  const mine = (await listProjects()).filter((p) => p.id !== project?.id || true);
  $('mine-section').hidden = !mine.length;
  $('mine').replaceChildren(
    ...mine.map((p) =>
      el(
        'li',
        {},
        el(
          'button',
          { class: 'mine-open', onclick: () => ($<HTMLDialogElement>('projects').close(), openProject(p)) },
          el('strong', { text: p.name }),
          el('span', { class: 'badge', text: BOARDS.find((b) => b.id === p.mapper)?.name ?? p.mapper }),
          el('span', { class: 'muted', text: p.example ? 'edited example' : `${p.files.length} file${p.files.length > 1 ? 's' : ''}` }),
          el('span', { class: 'muted when', text: new Date(p.updated).toLocaleDateString() }),
        ),
        el('button', {
          class: 'icon tiny',
          'aria-label': `Delete ${p.name}`,
          title: p.example ? 'Discard your edits' : 'Delete project',
          text: '✕',
          onclick: async () => {
            if (!confirm(p.example ? `Discard your edits to ${p.name}?` : `Delete ${p.name}? This cannot be undone.`)) return;
            await deleteProject(p.id);
            renderDialog();
          },
        }),
      ),
    ),
  );
}

async function showProjects() {
  if (project) await saveProject(project);
  await renderDialog();
  $<HTMLDialogElement>('projects').showModal();
}

// ------------------------------------------------------------------ layout

function setPane(pane: string) {
  workspace.dataset.pane = pane;
  document.querySelectorAll<HTMLElement>('.mobile-nav [data-pane]').forEach((b) => b.classList.toggle('active', b.dataset.pane === pane));
  if (pane === 'inspect' && activePanel) showPanel(activePanel.id);
}
document.querySelectorAll<HTMLElement>('.mobile-nav [data-pane]').forEach((b) => b.addEventListener('click', () => setPane(b.dataset.pane!)));
setPane('code');

{
  const divider = $('divider');
  const side = document.querySelector<HTMLElement>('.side')!;
  const set = (w: number) => {
    const max = Math.max(340, innerWidth - 520);
    const width = Math.round(Math.max(320, Math.min(max, w)));
    side.style.width = `${width}px`;
    store('nested.side', String(width));
  };
  const savedWidth = Number(store('nested.side'));
  if (savedWidth) set(savedWidth);
  divider.addEventListener('pointerdown', (e) => {
    divider.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => set(innerWidth - ev.clientX);
    const up = () => {
      divider.removeEventListener('pointermove', move);
      divider.removeEventListener('pointerup', up);
    };
    divider.addEventListener('pointermove', move);
    divider.addEventListener('pointerup', up);
  });
  divider.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowLeft') set(side.getBoundingClientRect().width + 24);
    else if (e.key === 'ArrowRight') set(side.getBoundingClientRect().width - 24);
    else return;
    e.preventDefault();
  });
}

// ------------------------------------------------------------------- input

function screenFocused() {
  return document.activeElement === screen;
}

addEventListener('keydown', (e) => {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && (e.key === 'Enter' || e.key === 's')) {
    e.preventDefault();
    void build(true);
    return;
  }
  if (mod && e.key === 'o') {
    e.preventDefault();
    void showProjects();
    return;
  }
  if (e.key === 'F5' && rom) {
    e.preventDefault();
    emulator.postMessage({ kind: 'save', slot: 0 });
    return;
  }
  if (e.key === 'F8' && saved) {
    e.preventDefault();
    restoreState();
    return;
  }
  if (!screenFocused()) return;
  unlockAudio();
  if (e.key === 'Escape') {
    pressed.clear();
    editor.view.focus();
    return;
  }
  if (e.code === 'Space') {
    e.preventDefault();
    if (rom && !crashed) setRunning(!running);
    return;
  }
  if (e.code in KEYS) {
    e.preventDefault();
    pressed.add(e.code);
  }
});
addEventListener('keyup', (e) => pressed.delete(e.code));
addEventListener('blur', () => pressed.clear());
screen.addEventListener('blur', () => pressed.clear());
screen.addEventListener('pointerdown', () => {
  unlockAudio();
  screen.focus();
});
document.addEventListener('pointerdown', unlockAudio, { once: true });

// Touch controls: every finger can press any button it slides over.
{
  const pad = $('pad');
  const touches = new Map<number, number>();
  const update = () => {
    touchButtons = 0;
    for (const b of touches.values()) touchButtons |= b;
    pad.querySelectorAll<HTMLElement>('[data-button]').forEach((x) => x.classList.toggle('pressed', (touchButtons & Number(x.dataset.button)) !== 0));
  };
  const hit = (e: PointerEvent) => {
    const target = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    return Number(target?.closest<HTMLElement>('[data-button]')?.dataset.button ?? 0);
  };
  pad.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    unlockAudio();
    pad.setPointerCapture(e.pointerId);
    touches.set(e.pointerId, hit(e));
    update();
  });
  pad.addEventListener('pointermove', (e) => {
    if (!touches.has(e.pointerId)) return;
    touches.set(e.pointerId, hit(e));
    update();
  });
  const end = (e: PointerEvent) => {
    touches.delete(e.pointerId);
    update();
  };
  pad.addEventListener('pointerup', end);
  pad.addEventListener('pointercancel', end);
  pad.addEventListener('contextmenu', (e) => e.preventDefault());
}

// ----------------------------------------------------------------- buttons

function restoreState() {
  if (!saved) return;
  const copy = saved.slice(0);
  emulator.postMessage({ kind: 'restore', state: copy }, [copy]);
  crashed = '';
  updateOverlay();
  flash('State restored');
  if (!running) stepFrames(0);
}

function stepFrames(n: number, input = 0): Promise<void> {
  return new Promise((resolve) => {
    stepButtons = input;
    stepRequest = Math.max(stepRequest, n);
    stepResolve = resolve;
    if (n === 0 && !inFlight) {
      requestFrames(0);
    }
  });
}

buildButton.addEventListener('click', () => {
  unlockAudio();
  void build(true);
});
$('project-button').addEventListener('click', () => void showProjects());
$('pause').addEventListener('click', () => {
  unlockAudio();
  if (rom && !crashed) setRunning(!running);
});
$('step').addEventListener('click', () => {
  if (!rom || crashed) return;
  setRunning(false);
  void stepFrames(1, buttons());
});
$('reset').addEventListener('click', () => {
  unlockAudio();
  if (!rom) return;
  void boot();
});
$('save-state').addEventListener('click', () => rom && emulator.postMessage({ kind: 'save', slot: 0 }));
$('load-state').addEventListener('click', restoreState);
const soundButton = $('sound');
const renderSound = () => {
  soundButton.innerHTML = soundOn ? ICONS.sound : ICONS.mute;
  soundButton.title = soundOn ? 'Sound on (click to mute)' : 'Sound off (click to unmute)';
  soundButton.setAttribute('aria-pressed', String(soundOn));
};
soundButton.addEventListener('click', () => {
  soundOn = !soundOn;
  store('nested.sound', soundOn ? 'on' : 'off');
  if (soundOn) unlockAudio();
  else audioNode?.port.postMessage({ reset: true });
  renderSound();
});
renderSound();
$('fullscreen').addEventListener('click', () => {
  const wrap = $('screen-wrap');
  if (document.fullscreenElement) void document.exitFullscreen();
  else void wrap.requestFullscreen?.();
  screen.focus();
});
$('upload').addEventListener('change', (e) => {
  const input = e.target as HTMLInputElement;
  if (input.files) void upload(input.files);
  input.value = '';
});
document.querySelector('[data-action="new-file"]')!.addEventListener('click', () => void newFile());
document.querySelector('[data-action="upload"]')!.addEventListener('click', () => $('upload').click());

// The overflow menu.
{
  const more = $('more');
  const menu = $('menu');
  const close = () => {
    menu.hidden = true;
    more.setAttribute('aria-expanded', 'false');
  };
  more.addEventListener('click', (e) => {
    e.stopPropagation();
    menu.hidden = !menu.hidden;
    more.setAttribute('aria-expanded', String(!menu.hidden));
  });
  document.addEventListener('click', (e) => {
    if (!menu.contains(e.target as Node)) close();
  });
  menu.addEventListener('click', async (e) => {
    const action = (e.target as HTMLElement).closest<HTMLElement>('[data-action]')?.dataset.action;
    if (!action) return;
    close();
    switch (action) {
      case 'download':
        if (!rom && !(await build(false))?.ok) return;
        if (rom) download(rom, `${slug(project?.name ?? 'game')}.nes`, 'application/x-nes-rom');
        break;
      case 'export':
        if (project) for (const f of project.files.filter((x) => x.text !== undefined)) download(new TextEncoder().encode(f.text), f.path.replaceAll('/', '_'), 'text/plain');
        break;
      case 'format':
        if (current.endsWith('.nst')) await editor.format();
        break;
      case 'restore':
        if (project?.example && confirm('Replace your edits with the original example?')) {
          await deleteProject(project.id);
          const id = project.example;
          project = null;
          await openExample(id, true);
        }
        break;
      case 'shortcuts':
        $<HTMLDialogElement>('shortcuts').showModal();
        break;
    }
  });
}

function slug(s: string) {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'game';
}

function download(data: Uint8Array, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([data as BlobPart], { type }));
  const a = el('a', { href: url, download: name });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

addEventListener('beforeunload', () => {
  if (project) void saveProject(project);
});

// ------------------------------------------------------------------ start

declare global {
  interface Window {
    __nested: Record<string, unknown>;
  }
}

async function start() {
  showStatus('Loading compiler…', 'busy');
  updateOverlay();
  const response = await fetch('nested.wasm');
  const module = await WebAssembly.compileStreaming(response).catch(async () => WebAssembly.compile(await (await fetch('nested.wasm')).arrayBuffer()));
  const ready = (w: Worker) =>
    new Promise<void>((resolve, reject) => {
      const listener = (e: MessageEvent) => {
        if (e.data.kind === 'ready') {
          w.removeEventListener('message', listener);
          resolve();
        } else if (e.data.kind === 'error') reject(new Error(e.data.message));
      };
      w.addEventListener('message', listener);
      w.postMessage({ kind: 'init', module });
    });
  await Promise.all([ready(compiler), ready(emulator)]);
  examples = await loadExamples().catch(() => []);
  const hash = new URLSearchParams(location.hash.slice(1));
  const mine = await listProjects();
  const last = store('nested.last');
  const wanted = hash.get('project') ?? (hash.get('example') ? null : last);
  const fromHash = hash.get('example');
  const existing = mine.find((p) => p.id === wanted);
  if (fromHash) await openExample(fromHash);
  else if (existing) await openProject(existing);
  else if (examples.length) await openExample(examples[0].id);
  else await openProject(await templateProject('nrom', 'My Game'));
  if (!last) {
    // First visit: point at the example picker.
    $('project-button').classList.add('pulse');
    setTimeout(() => $('project-button').classList.remove('pulse'), 4000);
  }
}

window.__nested = {
  get ready() {
    return !!rom && running;
  },
  get frames() {
    return frames;
  },
  get report() {
    return report;
  },
  get ram() {
    return machine?.ram;
  },
  get project() {
    return project;
  },
  get crashed() {
    return crashed;
  },
  examples: () => examples,
  openExample: (id: string) => openExample(id),
  newProject: async (board: string) => {
    const p = await templateProject(board, `Test ${board}`);
    await openProject(p);
    return report;
  },
  build: () => build(true),
  pause: () => setRunning(false),
  resume: () => setRunning(true),
  step: (input: number, n: number) => stepFrames(n, input),
  source: (path = current) => editor.text(path),
  setSource: (text: string, path = current) => editor.setText(path, text),
  open: (path: string) => openFile(path),
  rpc: (method: string, params: unknown) => {
    editor.flush();
    return lsp.request(method, params);
  },
  panel: (id: string) => showPanel(id),
  pane: (id: string) => setPane(id),
};

start().catch((error) => {
  console.error(error);
  showStatus('Failed to start', 'error');
  overlay.className = 'overlay show error';
  overlay.replaceChildren(el('strong', { text: 'The playground could not start' }), el('span', { text: String(error) }));
});
