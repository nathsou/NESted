# Writing a NESted cartridge

Files use `.nst`. Comments are `//` or nested `/* ... */`. Identifiers are ASCII.
Strings are literal-only assets or text; game text currently supports uppercase
ASCII glyphs supplied by the game's CHR. Integer literals accept decimal,
hexadecimal (`0x...`), binary (`0b...`) and underscores.

```nested
cartridge "My game" {
    mapper = mmc3;
    region = ntsc;
    mirroring = vertical;
    chr = "assets/game.chr";
    palette = "assets/game.pal";
    screen = "assets/title.map";
}
const TRACK: [u16; 4] = [213, 169, 142, 169];
var ticks: u8 = 0;
var player_x: u8 = 32;

fn init() { tone(0, TRACK[0], 6); }
fn update() {
    ticks += 1;
    if (buttons() & 128) != 0 { player_x += 1; }
    sprite(0, player_x, 160, 148, 0);
}
```

Compile with `nested build game.nst -o game.nes --emit artifacts/game`.
The asset paths resolve relative to the source file in the CLI. Embedded users
supply those keys and bytes through `Assets` instead.

## Values and control flow

`u8`, `i8`, `u16`, `i16` and `bool` have fixed widths. All integer arithmetic wraps
at the operand width. Mixed-width operands require casts. `u8(300)` produces 44;
`i8(255)` produces -1. `bool(x)` means nonzero. Signed division truncates toward
zero, and signed remainder has the dividend's sign. Division and remainder
require a nonzero compile-time divisor. Oversized unsigned shifts produce zero;
signed right shifts extend the sign, including oversized shifts.

Global `const` values reside in ROM or become immediate constants. Global `var`
values reside in RAM and require compile-time initializers. Arrays accept
lists, repetitions (`[0; 64]`) or constant byte assets (`asset("path")`). Their
length is fixed. Functions accept and return scalar values. Locals use `let`
(immutable) or `var` (mutable), with optional inferred types.

`if/else`, `while`, `for i in 0..64`, `loop`, `break`, `continue`, and `return`
are available. There is no recursion or shadowing. Function-local names must be
unique even across separate blocks. Range ends are checked each iteration.
Ordinary function arguments and compound-assignment indices evaluate once in
source order. `&&` and `||` short-circuit.

An index must be statically safe from a constant, a loop interval, a mask or a
guard. Loop mutation, helper calls and wrapping arithmetic invalidate unsafe
facts. Use `table[raw index]` for a programmer-owned indexing contract. A known
constant out-of-bounds index is an error even with `raw`.

## Match

A selector is evaluated exactly once. Value arms return one common scalar type;
block arms perform statements, with arm-local bindings and ordinary `return`,
`break` and `continue` behavior. Do not mix value and block arms.

```nested
fn direction(pad: u8) -> u8 {
    return match pad {
        16 | 32 => 1,
        64 | 128 => 2,
        _ => 0,
    };
}

fn update() {
    match buttons() & 3 {
        1 => { tone(0, 213, 5); }
        2 => { silence(0); }
        _ => {}
    }
}
```

Patterns are typed compile-time scalar constants, alternatives separated by
`|`, or inclusive ranges such as `0..=7`. Negative signed limits are valid,
including `-128` and `-32768`. Patterns must not overlap. Every match must cover
the selector's entire integer/bool domain or end with `_`. Boolean matches
can cover `false` and `true` without a wildcard. Guards and pattern bindings
are not supported. A range arm also supplies a bounds proof for that selector
inside its block. Side effects retain their usual bounds-invalidation rules.

## Formatting

`nested fmt game.nst` formats files with four-space indentation and readable
blocks, operators and array lists. `nested fmt --check games/*.nst` checks them
without writing. VS Code and the playground use the same Rust formatter through
LSP; editor tab/space preferences are respected. Strings retain their spelling,
comments retain their content, and assembly is indented without rewriting its
instructions. Unterminated lexical input is left unchanged.

## Inline assembly

```nested
fn rotate_with_carry(x: u8) -> u8 {
    return asm(a = x) {
        asl a
        adc #$00
    };
}
```

Inputs can bind `a`, `x`, and `y`; the expression returns A as `u8`. Local/global
names and function call names become assembler symbols. Assembly-local labels
are made unique for each block. All official 6502 instructions and addressing
modes are supported, as are `.byte`, `.word`, `.res`, and `.align`.

Assembly blocks are opaque to optimization. Registers/flags and arbitrary
memory are conservatively clobbered. Raw assembly is responsible for balanced
stack use, hardware sequencing and any manually accessed shared memory. Avoid
hidden indirect calls into language functions: static-frame allocation needs an
analyzable call graph. The assembler rejects indirect `jmp` through `$xxFF`,
which triggers the original 6502 page-wrap hardware bug.

## Placement and interrupts

`@zp var x: u8 = 0;` requires zero page. `@ram var x: u8 = 0;` forces ordinary
RAM. Large arrays use ordinary RAM automatically. Exhausting a region is a
compile-time error. `@bank(1) const MAP: [u8; 1440] = asset("map.bin");` places
constant data in a mapper's switchable window; reading it explicitly selects
that bank. Bank state is observable. Banked code and far calls are not supported.

`@nmi fn vblank() { ... }` and `@irq fn scanline() { ... }` use separate static
frames from main code and each other. Cross-context helpers require separate
functions. Interrupt callbacks may use scalar operations, assembly and
constant-address `peek`/`poke`; ordinary game builtins and software multiply/divide use main scratch and are rejected there.
Cross-context global storage requires `@shared` on a single byte. Its accesses
remain observable; this does not make a multi-instruction update atomic.

`@reset fn boot() { ... }` selects custom boot mode. The runtime performs basic
hardware/RAM initialization and calls that function, then halts if it returns.
Use assembly when you need your own main loop, interrupt enable policy or PPU
sequencing. In ordinary mode `init` runs once and `update` prepares each frame.
An ordinary `@nmi` callback runs when a prepared frame is published.

`@export` retains an otherwise unreachable function. `@noinline` prevents
source-level inlining. `@inline` and `@cold` are currently accepted hints;
inlining remains governed by the small pure-function heuristic.

`@budget(500)` checks an acyclic worst-case **unstalled instruction-cycle** bound,
including direct callees and conservative branch costs. Loops must disappear
through unrolling, otherwise verification fails. It excludes DMA, interrupts,
DMC stalls and PPU phase. Assembly listings show per-instruction min/max costs.

## Runtime builtins

| Function | Behavior |
|---|---|
| `buttons() -> u8`, `pressed() -> u8` | Held and rising-edge input: A=1, B=2, Select=4, Start=8, Up=16, Down=32, Left=64, Right=128. |
| `tile(x,y,tile)` | Writes a nametable tile. Coordinates clip to 32×30. Queued during rendering; direct while rendering is disabled. |
| `text(x,y,"TEXT")` | Literal ASCII tile indices; at most 255 bytes. |
| `sprite(slot,x,y,tile,attributes)` | Writes one of 64 shadow OAM entries. Tile IDs refer to the game's pattern table. |
| `hide_sprites()` | Hides all shadow OAM entries. |
| `palette(index,color)` | Direct while rendering is disabled, queued during rendering. |
| `render(bool)` | Disabling clears pending video writes and NMI. Enabling waits for the next vblank, copies OAM and restores scroll/rendering/NMI. |
| `screen("asset.map")` | Copies a 1024-byte nametable while rendering is disabled. |
| `screen_bank(bank,offset:u16)` | Copies a nametable from banked PRG data, leaving that bank selected. |
| `tone(channel,period:u16,volume)` | Pulse channels 0/1, triangle channel 2. Timer periods and volume are hardware values. |
| `noise(period,volume)` | Noise instrument on channel 3. |
| `silence(channel)` | Silences an APU channel. |
| `rand() -> u8` | Deterministic 16-bit LFSR; not a source of external entropy. |
| `peek(address:u16) -> u8`, `poke(address:u16,value)` | Volatile CPU bus access, including hardware side effects. |
| `bank(bank)` | Selects a PRG bank. NROM rejects this operation. |
| `bank_peek(bank,offset:u16) -> u8` | Selects and reads a banked byte. |

The queue holds **40 tile/palette writes per prepared frame**. Overflow drops
writes and increments RAM `$037F`; the workbench's memory/debug API and game tests
can inspect it. The shipped games are tested for zero overflow. Copy whole
screens with rendering disabled rather than submitting them as a frame queue.
OAM DMA and a full standard queue fit NTSC vblank with DMC disabled; an extra
user callback consumes additional time and is the programmer's timing contract.

## Memory and boards

| CPU address | Reservation |
|---|---|
| `$0000–$001F` | Runtime scratch/input/publication state |
| `$0020–$00FF` | Compiler zero-page globals and static frames |
| `$0100–$01FF` | Hardware stack |
| `$0200–$02FF` | Shadow OAM |
| `$0300–$0377` | Forty three-byte VRAM queue entries |
| `$037E`, `$037F` | Hardware frame counter; dropped queue write count |
| `$0400–$07FF` | Compiler RAM globals/arrays and static frames |

NROM uses a 32-KiB fixed PRG image and 8-KiB CHR ROM. MMC1 and UxROM have a fixed
last 16-KiB PRG window plus 16-KiB data banks. UxROM uploads the declared 8-KiB CHR
asset into CHR RAM at boot and uses bus-conflict-safe bank writes. MMC3 keeps the
last two 8-KiB PRG windows fixed and selects data at `$8000`. MMC1/MMC3 currently
use one fixed 8-KiB CHR-ROM set. Supported PRG capacities are checked at link time.
Battery-backed saves, CHR animation banking, mapper variants and PAL are deferred.

See the four game sources for complete programs. See [design.md](design.md) for
why these constraints were selected and what a future feature must improve.
