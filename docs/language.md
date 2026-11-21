# The NESted language

NESted is a small, statically allocated language for one machine: the NES.
A program is a **project**: a directory with `main.nst`, any other modules,
and the assets they reference. The compiler turns it into a complete iNES ROM.

```nested
// main.nst
cartridge {
    title = "Hello";
    mapper = nrom;
    mirroring = vertical;
    chr = "assets/tiles.chr";
}

use std::video;
use std::input;

var x: u8 = 120;

fn init() {
    video::palettes([0x0f, 0x11, 0x21, 0x30]);
    video::text(10, 12, "HELLO, NES!");
}

fn update() {
    if (input::held() & input::RIGHT) != 0 {
        x += 1;
    }
    video::sprite(x, 100, 0x01, 0);
}
```

```sh
nested build hello            # → hello.nes
nested run hello 300 --shot hello.png
nested fmt hello/*.nst
```

The rest of this guide covers the language, then the standard library, then
the hardware contracts the compiler relies on.

## Lexical structure

Comments are `//`, `///` (documentation shown on hover) and nested `/* */`.
`//!` at the top of a file documents the module. Integer literals are decimal,
hexadecimal `0x2000`, binary `0b1010_0000` or characters `'A'`; underscores
separate digits. Strings are byte arrays: `"SCORE"` has type `[u8; 5]`, and
may span lines (each newline is kept).

## Types

| Type | Meaning |
|---|---|
| `u8`, `i8`, `u16`, `i16` | Fixed-width integers. Arithmetic wraps at the width. |
| `bool` | `true` or `false`, one byte. |
| `[T; N]` | A fixed array. `[T; _]` takes its length from the initializer. |
| `&[T]` | A read-only slice: a pointer and a 16-bit length. |
| `struct S { ... }` | Named fields of scalars, arrays or other structs. |
| `enum E { ... }` | Named values, `u8` by default (`enum E: u16 { ... }`). |
| `type R = u8 in 0..8` | A *range type*: a `u8` the compiler proves stays in `0..8`. |

There are no pointers, references to mutable data, heap or closures. Every
variable has a fixed address chosen by the linker.

### Conversions

Widths never change implicitly, except that a value always fits a wider type
of the same signedness (`u8` → `u16`). Otherwise convert explicitly:

- `u16(x)`, `i8(x)` and friends wrap or sign-extend: `u8(300)` is 44.
- `R(x)` for a range type must be provable (see *Proofs*); `x as R` is the
  unchecked form when you know better.
- `bool(x)` is `x != 0`. `u8(true)` is 1.
- `lo(w)` and `hi(w)` take a word's bytes.

Literals take the type their context expects; an unsuffixed literal that does
not fit is an error rather than a silent wrap.

### Structs and enums

```nested
enum Kind { Bat, Slime = 4, Ghost }
struct Enemy { x: u8, y: u8, kind: Kind, hp: u8 }

var enemies: [Enemy; 8] = [Enemy { x: 0, y: 0, kind: Kind::Bat, hp: 3 }; 8];

fn hurt(i: Slot) {
    enemies[i].hp -= 1;
}
```

Structs are values: they can be locals, globals, array elements, parameters
and results. The compiler stores them **field by field**: an array of structs
becomes one array per field (structure of arrays), and a `u16` array becomes
separate low-byte and high-byte tables. `enemies[i].hp` is then a single
`lda hp,x`. Nothing in the language depends on this layout, so you get the
fast 6502 form without writing parallel arrays by hand.

### Range types

```nested
type Slot = u8 in 0..8;        // half-open
type Lane = u8 in 0..=3;       // inclusive
```

A range type is its base type plus a promise. Indexing a `[T; 8]` with a
`Slot` needs no check. Assigning to a `Slot` requires a proof that the value
is in range; arithmetic on it (`slot += 1`) is accepted when the compiler can
prove the result stays inside, for example under `if slot < 7`.

### Slices

A `&[T]` views an array of bytes (`u8`, `i8`, `bool` or a one-byte enum):
a constant table, a string literal, `asset("file.bin", 16..48)` or a
global `var` array. Local arrays can't be viewed; keep a global buffer
instead. An array becomes a slice wherever one is
expected (an argument, a `let s: &[u8]`, a returned value), so
`video::text(1, 1, "READY")` passes a constant string. `len(s)` is a `u16`.

## Items

```nested
const SPEED: u8 = 3;                       // a compile-time value
const LEVEL: [u8; _] = asset("level1.bin");// bytes in ROM
var score: u16 = 0;                        // RAM, initialized at reset
pub fn add(points: u8) { score += u16(points); }
```

- `const` values are computed at compile time. Arrays and strings live in
  ROM; scalars become immediates.
- `var` globals live in RAM and are initialized at power-on.
- `fn` takes scalars, structs and slices and may return one value (a scalar,
  a struct, or an array from a compile-time function). Arrays are passed as
  slices or by index; copying them by value would hide a large cost. Locals are `let` (immutable) or `var` (mutable). Names may not be
  shadowed inside a function.
- Items are private to their module unless marked `pub`.

### Compile-time evaluation

Any function without hardware access can run at compile time. A `const`
initializer may call one, so tables are computed rather than pasted:

```nested
fn squares() -> [u16; 16] {
    var t: [u16; 16] = [0; 16];
    for i in 0..16 {
        t[i] = u16(i) * u16(i);
    }
    return t;
}
const SQUARES: [u16; 16] = squares();
```

The standard library's `math::SIN` table is built this way. `if` with a
constant condition only checks and compiles the branch taken, and
`defined(path)` tests whether a name exists, so libraries can adapt to the
program and the board (`if board::MMC3 { ... }`).

### Assets

`asset("path")` is the bytes of a project file at compile time;
`asset("path", 16..48)` takes a range of it. Paths are relative to the
project root. Use them for CHR, nametables, levels and music.

## Modules

`use std::video;` makes `video::text` available. `use enemies;` loads
`enemies.nst` from the project, `use world::map;` loads `world/map.nst`.
`use a::b as c;` renames, and `use video::text;` imports a single item.
`main::` names the root module from anywhere; the runtime calls the hooks
`main::init`, `main::update`, `main::vblank` and `main::scanline` if they
exist.

## Statements and expressions

`if/else`, `while`, `loop`, `for i in a..b` (and `a..=b`), `break`,
`continue` and `return`. `if` and `match` are also expressions (`let d = if left { 0 } else { 1 };`). `&&` and
`||` short-circuit. Compound assignment (`+=`, `<<=`, ...) evaluates its
target once.

Comparisons don't chain: `a < b < c` is an error; write `(a < b) == c` if you
mean it.

```nested
let points = match kind {
    Kind::Bat => 10,
    Kind::Slime | Kind::Ghost => 25,
};
let band = match y {
    0..=63 => 0,
    64..=127 => 1,
    _ => 2,
};
```

`match` must be exhaustive: the compiler names a missing variant or value.
Patterns are literals, ranges, enum variants (the `Kind::` prefix may be
omitted), `|` alternatives and `_`.

Arithmetic wraps. Signed division truncates toward zero and the remainder has
the dividend's sign. Shifts by the width or more give 0 (or the sign for
signed `>>`). Multiplication, division and remainder by non-constants call
small runtime routines; by constants they become shifts and adds.

### Proofs and indexing

Every array index must be **provably in bounds**. The checker tracks integer
intervals through constants, masks (`i & 15`), modulo, shifts, loop counters,
`if` guards (`if i < len(t)`), early returns and range types. Assigning to a
variable, calling a function that may write it, or wrapping arithmetic
forgets what was known.

When you hold the invariant yourself, write `t[raw i]`: no proof and no
runtime check. A constant out-of-range index is an error even with `raw`.
`t[raw x + 3]` folds the offset into the instruction's address.

## Attributes

| Attribute | On | Effect |
|---|---|---|
| `@zp` | `var` | Place in zero page (faster, smaller instructions). The linker already promotes the most-used variables. |
| `@ram` | `var` | Keep out of zero page. |
| `@at(0x300)` | `var` | Fixed address. |
| `@io(0x4016)` | `var` | A hardware register: every read and write happens, in order. |
| `@shared` | `var` | Accessed by both main code and an interrupt; never cached in registers. |
| `@bank(n)` | `const`, `fn` | Place in switchable PRG bank `n`. Reads and calls switch banks automatically. |
| `@inline` / `@noinline` | `fn` | Override the inliner (small functions are inlined automatically). |
| `@nmi`, `@irq`, `@reset` | `fn` | Interrupt handlers. Defining `@reset` replaces the standard runtime. |
| `@recursive(n)` | `fn` | Allow recursion up to depth `n`, saving locals on a software stack. |
| `@budget(n)` | `fn` | Verify the function takes at most `n` CPU cycles (see below). |
| `@export` | `fn` | Keep the function even if nothing calls it (for tools and assembly). |

### Interrupts and contexts

Code reachable from an interrupt handler is compiled separately for that
context, with its own locals and arithmetic helpers, so an NMI can call the
same function main code was in the middle of. Globals that both sides use
must be `@shared` (or read and written with care); the compiler keeps them
out of registers.

### Recursion

Functions are not reentrant by default: their locals have fixed addresses,
which lets the linker overlay the frames of functions that are never active
together. `@recursive(n)` opts in; the compiler saves the live locals on a
shared stack before each recursive call. The chess engine's search uses this.

### Cycle budgets

`@budget(n)` asks the linker to prove a worst case: the sum of the slowest
timing of every instruction in the function (each runs at most once when the
code has no loops) plus the bound of each function it calls. Loops,
recursion, indirect jumps, banked calls and runtime arithmetic helpers can't
be bounded this way and are reported. Use it for vblank code that must
finish before rendering starts. DMA stalls and interrupt latency are not
included.

### Inline assembly

```nested
asm {
    lda {count}
    clc
    adc #1
    sta {count}
}
```

`{name}` is the address of a variable or constant in scope. Labels are local
to the block. Assembly is a barrier: the optimizer assumes it may read and
write anything.

## The cartridge

The root module declares the hardware:

```nested
cartridge {
    title = "Sprout";
    mapper = mmc3;            // nrom cnrom uxrom axrom bnrom gxrom colordreams mmc1 mmc3
    mirroring = vertical;     // horizontal vertical single four
    chr = "assets/tiles.chr"; // or a list of files, or `ram`
    prg = 128;                // KiB; default: as much as the code needs
    battery = false;
    queue = 96;               // bytes of background updates per frame
    sprites = true;           // false frees the 256-byte sprite page
    stack = 64;               // bytes reserved for the hardware stack
}
```

| Board | PRG banking | CHR | Notes |
|---|---|---|---|
| `nrom` | none (32 KiB) | 8 KiB ROM | |
| `cnrom` | none | 8 KiB banks (`mapper::chr_bank`) | |
| `uxrom` | 16 KiB + fixed 16 KiB | RAM | |
| `axrom` | 32 KiB | RAM | single-screen mirroring |
| `bnrom` | 32 KiB | RAM | |
| `gxrom` | 32 KiB | 8 KiB banks | |
| `colordreams` | 32 KiB | 8 KiB banks | |
| `mmc1` | 16 KiB + fixed 16 KiB | 4 KiB banks | switchable mirroring |
| `mmc3` | 8 KiB + fixed | 1 and 2 KiB banks | scanline IRQ |

On 32 KiB boards, the code shared by every bank (the runtime, interrupt
handlers, bank trampolines) is replicated into each bank. On boards with
CHR RAM, upload patterns in `init` with `video::upload_chr`.

`use std::board;` gives compile-time facts about the chosen board, such as
`board::MMC3` and `board::QUEUE`.

## The standard library

`std::runtime` is linked unless you define `@reset`. It initializes the
console, calls `main::init()` once with rendering off, then every frame
reads the controllers, calls `main::update()` and publishes the frame in the
next vertical blank. `main::vblank()` runs in every NMI (even when `update`
ran long), and `main::scanline()` handles the MMC3 IRQ.

| Module | Provides |
|---|---|
| `std::video` | `sprite`, `metasprite`, `flicker`; background `tile`, `metatile`, `text`, `number`, `fill`, `put`, `write`, `attribute`; `palette`, `palettes`; `scroll`; `load_nametable`, `load_rle`, `clear`, `upload_chr`; `show`, `off`, `on`, `pattern_tables`, `tall_sprites` |
| `std::input` | `held`, `pressed`, `released` (and `held2`, `pressed2`) with the `A B SELECT START UP DOWN LEFT RIGHT` masks |
| `std::audio` | A music and sound-effect driver: `play(p1, p2, tri, noise)`, `sfx(channel, stream)`, `stop`; direct `tone`, `noise`, `silence`. Streams are bytes: notes `C1`–`B7` with lengths, `REST`, `INSTRUMENT`, `LOOP`, `END`. |
| `std::math` | `rand`, `below`, `stir`, `min`, `max`, `abs`, `diff`, `sin`, `cos` and the compile-time `SIN` table |
| `std::mapper` | `chr_bank`, `mirroring`, `irq_after`, `irq_ack`, `irq_off` (no-ops on boards without the feature) |
| `std::nes` | The hardware registers (`PPUCTRL`, `PPUMASK`, `OAMDMA`, `JOY1`, ...) as `@io` variables |

Hover any name in the playground or VS Code for its documentation; the
library is ordinary NESted (`crates/compiler/src/std/`).

### Frames and the video queue

Drawing calls in `update` don't touch the PPU. Sprites go to a shadow page
copied by DMA, and background writes go to a queue of `queue` bytes that the
NMI copies during the next vertical blank, so the player never sees half a
frame. If `update` takes longer than a frame, the previous frame stays on
screen and the game slows down rather than tearing. `video::room()` tells you
how much queue is left; writes that don't fit are dropped and counted in
`video::dropped`, which the cartridge tests require to stay 0.

While rendering is off (`init`, or between `video::off()` and
`video::on()`) background writes go straight to the PPU with no limit.

## Tools

- `nested build <project> [-o out.nes] [--emit dir] [--no-opt]` writes the
  ROM; `--emit` also writes every compiler pass (MIR, optimized MIR, 6502,
  layout and the annotated listing).
- `nested check <project>` reports diagnostics only.
- `nested run <project|rom> [frames] [--input a@60,start@90] [--hold right] [--shot out.png]`
  runs headlessly.
- `nested fmt [--check] files...` formats code.
- `nested lsp` is the language server for editors.
