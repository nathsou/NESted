// Projects: example cartridges, templates for every supported board, and
// the user's own projects saved in IndexedDB.

export interface ProjectFile {
  path: string;
  /** Source text for .nst files. */
  text?: string;
  /** Bytes for assets. */
  data?: Uint8Array;
}

export interface Project {
  id: string;
  name: string;
  mapper: string;
  files: ProjectFile[];
  /** Example this project came from, if any. */
  example?: string;
  updated: number;
  open?: string;
}

export interface ExampleInfo {
  id: string;
  title: string;
  mapper: string;
  summary: string;
  files: string[];
}

export const BOARDS: { id: string; name: string; summary: string }[] = [
  { id: 'nrom', name: 'NROM', summary: '32 KiB of code, 8 KiB of tiles. The simplest board.' },
  { id: 'cnrom', name: 'CNROM', summary: 'Switches whole 8 KiB tile banks.' },
  { id: 'uxrom', name: 'UxROM', summary: '16 KiB switchable code banks, tiles in CHR RAM.' },
  { id: 'axrom', name: 'AxROM', summary: '32 KiB banks and one-screen mirroring.' },
  { id: 'bnrom', name: 'BNROM', summary: '32 KiB banks, tiles in CHR RAM.' },
  { id: 'gxrom', name: 'GxROM', summary: '32 KiB code banks and 8 KiB tile banks.' },
  { id: 'colordreams', name: 'Color Dreams', summary: '32 KiB code banks and 8 KiB tile banks.' },
  { id: 'mmc1', name: 'MMC1', summary: '16 KiB code banks and 4 KiB tile banks.' },
  { id: 'mmc3', name: 'MMC3', summary: '8 KiB banks, 1 KiB tile banks and a scanline IRQ.' },
];

export async function loadExamples(): Promise<ExampleInfo[]> {
  const response = await fetch('examples/index.json');
  if (!response.ok) throw new Error('Could not load the example list');
  return (await response.json()) as ExampleInfo[];
}

export async function loadExample(info: ExampleInfo): Promise<Project> {
  const files = await Promise.all(
    info.files.map(async (path) => {
      const response = await fetch(`examples/${info.id}/${path}`);
      if (!response.ok) throw new Error(`Could not load ${path}`);
      if (path.endsWith('.nst')) return { path, text: await response.text() };
      return { path, data: new Uint8Array(await response.arrayBuffer()) };
    }),
  );
  files.sort((a, b) => order(a.path) - order(b.path) || a.path.localeCompare(b.path));
  return { id: `example-${info.id}`, name: info.title, mapper: info.mapper, files, example: info.id, updated: 0, open: 'main.nst' };
}

function order(path: string): number {
  if (path === 'main.nst') return 0;
  if (path.endsWith('.nst')) return 1;
  return 2;
}

// --------------------------------------------------------------- storage

const DB = 'nested-playground';
const STORE = 'projects';

function db(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: 'id' });
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function tx<T>(mode: IDBTransactionMode, body: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const d = await db();
  return new Promise((resolve, reject) => {
    const t = d.transaction(STORE, mode);
    const request = body(t.objectStore(STORE));
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export async function saveProject(p: Project): Promise<void> {
  try {
    await tx('readwrite', (s) => s.put({ ...p, updated: Date.now() }));
  } catch {
    // Private windows may refuse storage; the session keeps working.
  }
}

export async function listProjects(): Promise<Project[]> {
  try {
    const all = (await tx('readonly', (s) => s.getAll())) as Project[];
    return all.sort((a, b) => b.updated - a.updated);
  } catch {
    return [];
  }
}

export async function deleteProject(id: string): Promise<void> {
  try {
    await tx('readwrite', (s) => s.delete(id));
  } catch {
    // Ignored, as above.
  }
}

export function newId(): string {
  return `p-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

// -------------------------------------------------------------- templates

async function starterTiles(name: string): Promise<Uint8Array> {
  const response = await fetch(`templates/${name}`);
  return new Uint8Array(await response.arrayBuffer());
}

const COMMON = `
use std::video;
use std::input;
use std::audio;

/// A 16x16 sprite: tiles t, t+1 above t+2, t+3.
fn sprite16(x: u8, y: u8, t: u8, attr: u8) {
    video::sprite(x, y, t, attr);
    video::sprite(x + 8, y, t + 1, attr);
    video::sprite(x, y + 8, t + 2, attr);
    video::sprite(x + 8, y + 8, t + 3, attr);
}

var x: u8 = 120;
var y: u8 = 112;
const BLIP: [u8; _] = [audio::INSTRUMENT, 1, audio::C6, 4, audio::G6, 8, audio::END];
`;

const BLOCK_TILE = '0x60';

function mainFor(board: string, title: string): string {
  const header = `// ${title}: a starter project for the ${BOARDS.find((b) => b.id === board)?.name} board.
// Arrows move the ball, A plays a sound. Edit anything and press Build.
cartridge {
    title = "${title}";
    mapper = ${board};
    mirroring = vertical;
`;
  const chrRam = ['uxrom', 'axrom', 'bnrom'].includes(board);
  const chr = board === 'cnrom' || board === 'gxrom' || board === 'colordreams'
    ? '    chr = ["assets/tiles.chr", "assets/tiles-alt.chr"];\n'
    : chrRam
      ? '    chr = ram;\n'
      : '    chr = "assets/tiles.chr";\n';
  let extraTop = '';
  let extraInit = '';
  let extraUpdate = '';
  if (chrRam) {
    extraTop += `
/// Pattern tables live in a switchable PRG bank and are copied to CHR RAM.
@bank(0) const TILES: [u8; 8192] = asset("assets/tiles.chr");
`;
    extraInit += `    video::upload_chr(0, TILES);\n`;
  }
  if (board === 'cnrom' || board === 'gxrom' || board === 'colordreams') {
    extraTop += `use std::mapper;\n`;
    extraUpdate += `    // B swaps between the two 8 KiB tile banks.
    if (input::held() & input::B) != 0 {
        mapper::chr_bank(0, 1);
    } else {
        mapper::chr_bank(0, 0);
    }
`;
  }
  if (board === 'mmc1' || board === 'mmc3' || board === 'uxrom' || board === 'axrom' || board === 'bnrom' || board === 'gxrom' || board === 'colordreams') {
    extraTop += `
/// Constants in a switchable bank are selected automatically when read.
@bank(${chrRam ? 1 : 0}) const MESSAGE: [u8; _] = "BANKED DATA OK";
`;
    extraInit += `    for i in 0..len(MESSAGE) {
        video::tile(9 + u8(i), 20, MESSAGE[i]);
    }
`;
  }
  if (board === 'mmc3') {
    extraTop += `use std::mapper;
use std::nes;

/// Every frame: arm the scanline counter for the middle of the screen.
fn vblank() {
    if !video::direct {
        mapper::irq_after(119);
    }
}

/// MMC3 IRQ at that scanline: tint the lower half of the screen blue
/// (the NMI restores the normal mask at the next vblank).
fn scanline() {
    nes::PPUMASK = video::mask | 0x80;
    mapper::irq_ack(false);
}
`;
  }
  return `${header}${chr}}
${COMMON}${extraTop}
fn init() {
    video::palettes([0x0f, 0x11, 0x21, 0x30, 0x0f, 0x16, 0x27, 0x30, 0x0f, 0x19, 0x29, 0x30, 0x0f, 0x00, 0x10, 0x30,
                     0x0f, 0x16, 0x27, 0x30, 0x0f, 0x11, 0x21, 0x30, 0x0f, 0x19, 0x29, 0x30, 0x0f, 0x00, 0x10, 0x30]);
${extraInit}    video::text(9, 4, "HELLO, NESTED!");
    video::text(8, 6, "${BOARDS.find((b) => b.id === board)?.name.toUpperCase()} STARTER");
    for col in 0..16 {
        video::metatile(col * 2, 26, ${BLOCK_TILE});
    }
}

fn update() {
    let held = input::held();
    if (held & input::LEFT) != 0 && x > 8 {
        x -= 2;
    }
    if (held & input::RIGHT) != 0 && x < 232 {
        x += 2;
    }
    if (held & input::UP) != 0 && y > 16 {
        y -= 2;
    }
    if (held & input::DOWN) != 0 && y < 192 {
        y += 2;
    }
    if (input::pressed() & input::A) != 0 {
        audio::sfx(1, BLIP);
    }
${extraUpdate}    sprite16(x, y, 0, 0);
}
`;
}

export async function templateProject(board: string, name: string): Promise<Project> {
  const files: ProjectFile[] = [{ path: 'main.nst', text: mainFor(board, name) }, { path: 'assets/tiles.chr', data: await starterTiles('tiles.chr') }];
  if (board === 'cnrom' || board === 'gxrom' || board === 'colordreams') files.push({ path: 'assets/tiles-alt.chr', data: await starterTiles('tiles-alt.chr') });
  return { id: newId(), name, mapper: board, files, updated: Date.now(), open: 'main.nst' };
}

export function blankModule(path: string): ProjectFile {
  const name = path.replace(/\.nst$/, '').split('/').pop() ?? 'module';
  return {
    path,
    text: `//! ${name}: describe what this module provides.

/// Items are private unless marked \`pub\`. Use it with \`use ${path.replace(/\.nst$/, '').replaceAll('/', '::')};\`.
pub fn hello() -> u8 {
    return 42;
}
`,
  };
}
