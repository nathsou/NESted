// Inspector panels: problems, build size, live variables, the PPU, the
// assembly listing and every compiler pass.
import type { Diagnostic, Report, SymbolInfo } from './wasm';

const PALETTE_HEX = '808080 003da6 0012b0 440096 a1005e c70028 ba0600 8c1700 5c2f00 104500 054a00 00472e 004166 000000 050505 050505 c7c7c7 0077ff 2155ff 8237fa eb2fb5 ff2950 ff2200 d63200 c46200 358000 058f00 008a55 0099cc 212121 090909 090909 ffffff 0fd7ff 69a2ff d480ff ff45f3 ff618b ff8833 ff9c12 fabc20 9fe30e 2bf035 0cf0a4 05fbff 5e5e5e 0d0d0d 0d0d0d ffffff a6fcff b3ecff daabeb ffa8f9 ffabb3 ffd2b0 ffefa6 fff79c d7e895 a6edaf a2f2da 99fffc dddddd 111111 111111';
export const NES_COLORS: [number, number, number][] = PALETTE_HEX.split(' ').map((h) => [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)]);

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, unknown> = {}, ...children: (Node | string | null | undefined | false)[]): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === 'class') node.className = String(v);
    else if (k === 'text') node.textContent = String(v);
    else if (k.startsWith('on') && typeof v === 'function') node.addEventListener(k.slice(2), v as EventListener);
    else if (k === 'style' && typeof v === 'object') Object.assign(node.style, v);
    else if (k in node && typeof v !== 'string') (node as unknown as Record<string, unknown>)[k] = v;
    else node.setAttribute(k, String(v));
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) node.append(c);
  return node;
}

export const hex = (n: number, width = 2) => n.toString(16).toUpperCase().padStart(width, '0');

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  return `${(n / 1024).toFixed(n % 1024 ? 1 : 0)} KiB`;
}

export interface Machine {
  ram: Uint8Array;
  pc: number;
  ppu: Uint8Array | null;
  oam: Uint8Array | null;
  regs: number;
}

export interface Panel {
  id: string;
  title: string;
  root: HTMLElement;
  /** Called after each build. */
  build?(report: Report): void;
  /** Called with machine state while the panel is visible. */
  frame?(m: Machine): void;
  /** Whether the panel wants PPU memory with each frame. */
  wantsPpu?: boolean;
  badge?: () => string;
}

// ---------------------------------------------------------------- problems

export class Problems implements Panel {
  id = 'problems';
  title = 'Problems';
  root = el('div', { class: 'panel problems' });
  private items: Diagnostic[] = [];
  constructor(private readonly open: (file: string, line: number, column: number) => void) {}
  badge = () => {
    const errors = this.items.filter((d) => d.severity === 'error').length;
    return errors ? String(errors) : this.items.length ? String(this.items.length) : '';
  };
  build(report: Report) {
    this.items = report.diagnostics;
    this.root.replaceChildren();
    if (!this.items.length) {
      this.root.append(el('div', { class: 'empty' }, el('strong', { text: report.ok ? 'No problems.' : 'Build failed.' }), report.ok ? ' The cartridge built cleanly.' : ''));
      return;
    }
    const list = el('ul', { class: 'diagnostics' });
    for (const d of this.items) {
      list.append(
        el(
          'li',
          { class: `diag ${d.severity}`, tabindex: '0', onclick: () => this.open(d.file, d.line, d.column), onkeydown: (e: KeyboardEvent) => e.key === 'Enter' && this.open(d.file, d.line, d.column) },
          el('span', { class: 'sev', text: d.severity === 'error' ? '✕' : d.severity === 'warning' ? '!' : 'i' }),
          el('span', { class: 'msg', text: d.message }),
          el('span', { class: 'loc', text: `${d.file}:${d.line + 1}:${d.column + 1}` }),
        ),
      );
    }
    this.root.append(list);
  }
}

// ------------------------------------------------------------------ memory

function bar(label: string, used: number, total: number, detail?: string) {
  const ratio = total ? Math.min(1, used / total) : 0;
  const level = ratio > 0.95 ? 'full' : ratio > 0.8 ? 'high' : '';
  return el(
    'div',
    { class: 'meter' },
    el('div', { class: 'meter-label' }, el('span', { text: label }), el('span', { class: 'meter-value', text: detail ?? `${bytes(used)} / ${bytes(total)}` })),
    el('div', { class: `meter-track ${level}` }, el('div', { class: 'meter-fill', style: { width: `${(ratio * 100).toFixed(1)}%` } })),
  );
}

export class Memory implements Panel {
  id = 'memory';
  title = 'Size';
  root = el('div', { class: 'panel memory' });
  build(report: Report) {
    this.root.replaceChildren();
    if (!report.ok || !report.symbols) {
      this.root.append(el('div', { class: 'empty', text: 'Build the cartridge to see where its bytes go.' }));
      return;
    }
    const summary = el(
      'dl',
      { class: 'facts' },
      el('dt', { text: 'Board' }),
      el('dd', { text: `${report.board}`, title: `iNES mapper ${report.mapper}` }),
      el('dt', { text: 'ROM' }),
      el('dd', { text: bytes(report.romBytes ?? 0) }),
      el('dt', { text: 'PRG' }),
      el('dd', { text: bytes(report.prgBytes ?? 0) }),
      el('dt', { text: 'CHR' }),
      el('dd', { text: report.chrBytes ? bytes(report.chrBytes) : 'RAM' }),
    );
    const meters = el('div', { class: 'meters' });
    meters.append(bar('Zero page', report.zeroPage ?? 0, 256), bar('RAM', report.ram ?? 0, 2048), bar('Fixed code', report.fixedUsed ?? 0, report.fixedSize ?? 0));
    for (const b of report.banks ?? []) meters.append(bar(`Bank ${b.bank}`, b.used, report.bankSize ?? 16384));
    const top = (filter: (s: SymbolInfo) => boolean, title: string) => {
      const rows = report.symbols!.filter(filter).sort((a, b) => b.size - a.size).slice(0, 12);
      const max = rows[0]?.size ?? 1;
      return el(
        'section',
        { class: 'top' },
        el('h3', { text: title }),
        el(
          'table',
          {},
          ...rows.map((s) =>
            el(
              'tr',
              {},
              el('td', { class: 'name', text: s.name.replace(/^fn:/, ''), title: s.kind }),
              el('td', { class: 'num', text: bytes(s.size) }),
              el('td', { class: 'spark' }, el('div', { style: { width: `${(s.size / max) * 100}%` } })),
            ),
          ),
        ),
      );
    };
    this.root.append(
      summary,
      meters,
      el('div', { class: 'tops' }, top((s) => s.kind.startsWith('fn'), 'Largest functions'), top((s) => s.kind.startsWith('const'), 'Largest data'), top((s) => (s.kind.startsWith('var') || s.kind.startsWith('local')) && s.address < 0x800, 'Largest RAM users')),
    );
  }
}

// ------------------------------------------------------------------- watch

interface Watched {
  sym: SymbolInfo;
  row: HTMLElement;
  value: HTMLElement;
  last: string;
}

export class Watch implements Panel {
  id = 'watch';
  title = 'Variables';
  root = el('div', { class: 'panel watch' });
  private filter = el('input', { type: 'search', placeholder: 'Filter variables…', class: 'filter', 'aria-label': 'Filter variables' });
  private list = el('div', { class: 'watch-list' });
  private symbols: SymbolInfo[] = [];
  private rows: Watched[] = [];
  private pinned = new Set<string>(JSON.parse(safeGet('nested.pins') ?? '[]') as string[]);
  private ram: Uint8Array | null = null;
  constructor(private readonly poke: (address: number, value: number) => void) {
    this.filter.addEventListener('input', () => this.render());
    this.root.append(el('div', { class: 'toolbar' }, this.filter), this.list);
  }
  build(report: Report) {
    if (!report.ok || !report.symbols) return;
    this.symbols = report.symbols.filter((s) => s.address < 0x800 && (s.kind.startsWith('var') || s.kind.startsWith('local'))).sort((a, b) => a.name.localeCompare(b.name));
    this.render();
  }
  private render() {
    const q = this.filter.value.trim().toLowerCase();
    const shown = this.symbols.filter((s) => !q || s.name.toLowerCase().includes(q));
    shown.sort((a, b) => Number(this.pinned.has(b.name)) - Number(this.pinned.has(a.name)));
    this.list.replaceChildren();
    this.rows = [];
    if (!shown.length) {
      this.list.append(el('div', { class: 'empty', text: this.symbols.length ? 'No variable matches.' : 'Build the cartridge to watch its variables.' }));
      return;
    }
    for (const sym of shown.slice(0, 400)) {
      const value = el('span', { class: 'value' });
      const pin = el('button', { class: `pin ${this.pinned.has(sym.name) ? 'on' : ''}`, title: 'Pin to top', 'aria-label': `Pin ${sym.name}`, text: '★' });
      pin.addEventListener('click', () => {
        if (this.pinned.has(sym.name)) this.pinned.delete(sym.name);
        else this.pinned.add(sym.name);
        safeSet('nested.pins', JSON.stringify([...this.pinned]));
        this.render();
      });
      const row = el('div', { class: 'watch-row' }, pin, el('span', { class: 'name', text: sym.name, title: `${sym.kind} at $${hex(sym.address, 4)}` }), el('span', { class: 'addr', text: `$${hex(sym.address, 4)}` }), value);
      value.addEventListener('dblclick', () => this.edit(sym, value));
      value.title = 'Double-click to change';
      this.list.append(row);
      this.rows.push({ sym, row, value, last: '' });
    }
    if (this.ram) this.frame({ ram: this.ram, pc: 0, ppu: null, oam: null, regs: 0 });
  }
  private edit(sym: SymbolInfo, value: HTMLElement) {
    const current = this.read(sym);
    const next = prompt(`New value for ${sym.name}`, String(current ?? 0));
    if (next === null) return;
    const n = Number(next);
    if (!Number.isFinite(n)) return;
    this.poke(sym.address, n & 0xff);
    if (sym.size === 2 && !sym.kind.includes('[')) this.poke(sym.address + 1, (n >> 8) & 0xff);
    value.classList.add('changed');
  }
  private read(sym: SymbolInfo): number | null {
    if (!this.ram) return null;
    const signed = sym.kind.includes('i8') || sym.kind.includes('i16');
    if (sym.size === 1) {
      const v = this.ram[sym.address];
      return signed && v > 127 ? v - 256 : v;
    }
    if (sym.size === 2 && !sym.kind.includes('[')) {
      const v = this.ram[sym.address] | (this.ram[sym.address + 1] << 8);
      return signed && v > 32767 ? v - 65536 : v;
    }
    return null;
  }
  frame(m: Machine) {
    this.ram = m.ram;
    for (const w of this.rows) {
      const n = this.read(w.sym);
      let text: string;
      if (n !== null) text = `${n} <span>$${hex(n & (w.sym.size === 2 ? 0xffff : 0xff), w.sym.size * 2)}</span>`;
      else {
        const shown = Math.min(w.sym.size, 16);
        text = Array.from(m.ram.subarray(w.sym.address, w.sym.address + shown), (b) => hex(b)).join(' ') + (w.sym.size > shown ? ' …' : '');
      }
      if (text !== w.last) {
        if (w.last) {
          w.value.classList.remove('changed');
          void w.value.offsetWidth;
          w.value.classList.add('changed');
        }
        w.value.innerHTML = text;
        w.last = text;
      }
    }
  }
}

function safeGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function safeSet(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage may be unavailable.
  }
}

// --------------------------------------------------------------------- ppu

export class Ppu implements Panel {
  id = 'ppu';
  title = 'PPU';
  root = el('div', { class: 'panel ppu' });
  wantsPpu = true;
  private patterns = el('canvas', { width: 256, height: 128, class: 'pixel', 'aria-label': 'Pattern tables' });
  private names = el('canvas', { width: 512, height: 480, class: 'pixel', 'aria-label': 'Nametables' });
  private palettes = el('div', { class: 'swatches' });
  private sprites = el('div', { class: 'sprites' });
  private regs = el('div', { class: 'regs' });
  private paletteChoice = 0;
  private last: Machine | null = null;
  constructor() {
    const choose = el('div', { class: 'segmented', role: 'radiogroup', 'aria-label': 'Pattern palette' });
    for (let i = 0; i < 8; i++) {
      const b = el('button', { text: i < 4 ? `BG${i}` : `SP${i - 4}`, class: i === 0 ? 'on' : '', role: 'radio' });
      b.addEventListener('click', () => {
        this.paletteChoice = i;
        choose.querySelectorAll('button').forEach((x, j) => x.classList.toggle('on', j === i));
        if (this.last) this.frame(this.last);
      });
      choose.append(b);
    }
    this.root.append(
      el('section', {}, el('h3', { text: 'Nametables' }), this.names, this.regs),
      el('section', {}, el('h3', {}, 'Pattern tables ', choose), this.patterns, el('h3', { text: 'Palettes' }), this.palettes, el('h3', { text: 'Sprites' }), this.sprites),
    );
  }
  frame(m: Machine) {
    this.last = m;
    const ppu = m.ppu;
    if (!ppu) return;
    const pal = ppu.subarray(0x3f00, 0x3f20);
    const color = (i: number) => NES_COLORS[(i & 3) === 0 ? pal[0] & 0x3f : pal[i] & 0x3f];
    // Pattern tables.
    {
      const ctx = this.patterns.getContext('2d')!;
      const img = ctx.createImageData(256, 128);
      const base = this.paletteChoice * 4;
      for (let table = 0; table < 2; table++)
        for (let tile = 0; tile < 256; tile++) {
          const at = table * 0x1000 + tile * 16;
          const tx = table * 128 + (tile & 15) * 8;
          const ty = (tile >> 4) * 8;
          for (let y = 0; y < 8; y++) {
            const lo = ppu[at + y];
            const hi = ppu[at + y + 8];
            for (let x = 0; x < 8; x++) {
              const v = ((lo >> (7 - x)) & 1) | (((hi >> (7 - x)) & 1) << 1);
              const c = color(base + v);
              const o = ((ty + y) * 256 + tx + x) * 4;
              img.data[o] = c[0];
              img.data[o + 1] = c[1];
              img.data[o + 2] = c[2];
              img.data[o + 3] = 255;
            }
          }
        }
      ctx.putImageData(img, 0, 0);
    }
    // Nametables, all four (mirrored as the cartridge wires them).
    {
      const ctrl = m.regs & 0xff;
      const bgTable = ctrl & 0x10 ? 0x1000 : 0;
      const ctx = this.names.getContext('2d')!;
      const img = ctx.createImageData(512, 480);
      for (let n = 0; n < 4; n++) {
        const nt = 0x2000 + n * 0x400;
        const ox = (n & 1) * 256;
        const oy = (n >> 1) * 240;
        for (let row = 0; row < 30; row++)
          for (let col = 0; col < 32; col++) {
            const tile = ppu[nt + row * 32 + col];
            const attr = ppu[nt + 0x3c0 + (row >> 2) * 8 + (col >> 2)];
            const p = (attr >> (((row & 2) << 1) | (col & 2))) & 3;
            const at = bgTable + tile * 16;
            for (let y = 0; y < 8; y++) {
              const lo = ppu[at + y];
              const hi = ppu[at + y + 8];
              for (let x = 0; x < 8; x++) {
                const v = ((lo >> (7 - x)) & 1) | (((hi >> (7 - x)) & 1) << 1);
                const c = color(p * 4 + v);
                const o = ((oy + row * 8 + y) * 512 + ox + col * 8 + x) * 4;
                img.data[o] = c[0];
                img.data[o + 1] = c[1];
                img.data[o + 2] = c[2];
                img.data[o + 3] = 255;
              }
            }
          }
      }
      ctx.putImageData(img, 0, 0);
      const mask = m.regs >> 8;
      this.regs.replaceChildren(
        el('span', { text: `PPUCTRL $${hex(ctrl)}` }),
        el('span', { text: `PPUMASK $${hex(mask)}` }),
        el('span', { text: `BG ${ctrl & 0x10 ? '$1000' : '$0000'}` }),
        el('span', { text: `Sprites ${ctrl & 0x20 ? '8×16' : `8×8 ${ctrl & 0x08 ? '$1000' : '$0000'}`}` }),
        el('span', { text: `PC $${hex(m.pc, 4)}` }),
      );
    }
    // Palettes.
    this.palettes.replaceChildren(
      ...Array.from({ length: 32 }, (_, i) => {
        const v = pal[i] & 0x3f;
        const c = NES_COLORS[v];
        return el('span', { class: 'swatch', title: `$3F${hex(i)} = $${hex(v)}`, style: { background: `rgb(${c[0]},${c[1]},${c[2]})` } });
      }),
    );
    // Sprites in use.
    const oam = m.oam;
    if (oam) {
      const used: HTMLElement[] = [];
      for (let i = 0; i < 64; i++) {
        const y = oam[i * 4];
        if (y >= 0xef) continue;
        used.push(el('span', { class: 'oam', text: `#${i} ${oam[i * 4 + 3]},${y + 1} t${hex(oam[i * 4 + 1])} a${hex(oam[i * 4 + 2])}` }));
      }
      this.sprites.replaceChildren(used.length ? el('span', { class: 'count', text: `${used.length} of 64 visible` }) : el('span', { class: 'count', text: 'No sprites on screen' }), ...used);
    }
  }
}

// ----------------------------------------------------------------- listing

export class Listing implements Panel {
  id = 'listing';
  title = 'Assembly';
  root = el('div', { class: 'panel listing' });
  private filter = el('input', { type: 'search', placeholder: 'Find a function or address…', class: 'filter', 'aria-label': 'Find in listing' });
  private pre = el('pre', { class: 'code' });
  private text = '';
  constructor(private readonly open: (file: string, line: number) => void, private readonly lines: () => Report['lines']) {
    this.filter.addEventListener('keydown', (e) => e.key === 'Enter' && this.find());
    this.pre.addEventListener('dblclick', () => this.jump());
    this.root.append(el('div', { class: 'toolbar' }, this.filter, el('span', { class: 'hint', text: 'Double-click a line to open its source' })), this.pre);
  }
  build(report: Report) {
    this.text = report.passes.find((p) => p.name === 'Listing')?.text ?? '';
    this.pre.textContent = this.text || 'The listing appears after a successful build.';
  }
  private find() {
    const q = this.filter.value.trim().toLowerCase();
    if (!q) return;
    const lines = this.text.split('\n');
    const i = lines.findIndex((l) => l.toLowerCase().includes(q));
    if (i < 0) return;
    const lineHeight = parseFloat(getComputedStyle(this.pre).lineHeight) || 16;
    this.pre.scrollTop = i * lineHeight - 40;
  }
  private jump() {
    const sel = getSelection();
    const node = sel?.anchorNode;
    if (!node) return;
    const before = this.text.slice(0, this.offsetIn(node, sel!.anchorOffset));
    const line = this.text.split('\n')[before.split('\n').length - 1] ?? '';
    const m = /^\s*(?:\d+:)?([0-9A-Fa-f]{4})\b/.exec(line);
    if (!m) return;
    const address = parseInt(m[1], 16);
    const best = (this.lines() ?? []).filter((l) => l.address <= address).sort((a, b) => b.address - a.address)[0];
    if (best) this.open(best.file, best.line);
  }
  private offsetIn(node: Node, offset: number): number {
    let total = 0;
    const walker = document.createTreeWalker(this.pre, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      if (walker.currentNode === node) return total + offset;
      total += walker.currentNode.textContent?.length ?? 0;
    }
    return total;
  }
}

// ------------------------------------------------------------------ passes

export class Passes implements Panel {
  id = 'passes';
  title = 'Passes';
  root = el('div', { class: 'panel passes' });
  private select = el('select', { 'aria-label': 'Compiler pass' });
  private pre = el('pre', { class: 'code' });
  private passes: Report['passes'] = [];
  private chosen = safeGet('nested.pass') ?? 'MIR optimized';
  constructor() {
    this.select.addEventListener('change', () => {
      this.chosen = this.select.value;
      safeSet('nested.pass', this.chosen);
      this.show();
    });
    const copy = el('button', { class: 'ghost', text: 'Copy' });
    copy.addEventListener('click', () => navigator.clipboard?.writeText(this.pre.textContent ?? ''));
    this.root.append(el('div', { class: 'toolbar' }, this.select, copy), this.pre);
  }
  build(report: Report) {
    this.passes = report.passes;
    this.select.replaceChildren(...report.passes.map((p) => el('option', { value: p.name, text: p.name })));
    if (!report.passes.some((p) => p.name === this.chosen) && report.passes.length) this.chosen = report.passes[0].name;
    this.select.value = this.chosen;
    this.show();
  }
  private show() {
    this.pre.textContent = this.passes.find((p) => p.name === this.chosen)?.text ?? 'No output for this pass.';
  }
}
