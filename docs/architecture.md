# Architecture

## Crates

```
nested-compiler   no dependencies, no I/O: Project → ROM + reports, LSP, formatter
  ├─ nested-cli       the `nested` command (build, check, run, fmt, lsp) and cartridge tests
  ├─ nested-wasm      compiler + LSP + emulator behind a plain C ABI for the browser
  └─ chessbench       headless chess benchmarks and a UCI bridge
nested-emulator   the Nessy emulator (CPU, PPU, APU, mappers)
  └─ nested-runtime   frames, pixels, audio, RAM/PPU inspection, save states, PNG
```

The compiler takes a `Project` (source files and asset bytes keyed by path)
and never touches the filesystem, so the same code runs in the CLI, in a
language server and in WebAssembly.

## Compiler pipeline

| Stage | File | What it does |
|---|---|---|
| Lexing | `lexer.rs` | Tokens plus comments kept as trivia for the formatter and hover docs. Assembly bodies are kept raw. |
| Parsing | `parser.rs` | AST with error recovery, so the editor still gets completions in broken code. |
| Checking | `check.rs`, `check_expr.rs` | Loads modules on demand (`use a::b` → `a/b.nst`, `std::` from the embedded library), resolves names and visibility, checks types and coercions, and proves array indices and range-type assignments with interval facts that flow through guards, loops and early returns. Produces typed HIR. |
| Constant evaluation | `consteval.rs` | An interpreter for HIR used for `const` initializers and constant `if`. It is also the reference semantics in the differential tests. |
| MIR construction | `build_mir.rs` | Scalarizes structs and splits word arrays into byte tables, creates one instance of each function per context (main, NMI, IRQ), and lowers to a control-flow graph of byte and word operations. |
| Optimization | `opt.rs` | Bottom-up inlining, constant and copy propagation, forward substitution, dead code and dead store elimination, CFG simplification, liveness and interference for frame sharing, and recursion save sets. |
| Instruction selection | `codegen.rs` | Byte-oriented 6502 code using A, X and Y, addressing-mode selection for SoA access, runtime helpers for multiply and divide per context, bank trampolines and the recursion save stack. |
| Peephole | `peephole.rs` | Register and flag tracking (aware of volatile memory and labels), redundant load and compare removal, jump threading and tail calls. |
| Assembly | `asm.rs` | Opcode tables, the inline-assembly parser, encoding with branch relaxation and an annotated listing with cycle counts and source lines. |
| Linking | `link.rs` | Board layouts, RAM allocation with zero-page promotion by use count, frame overlays from the call graph, bank placement, shared-segment replication on 32 KiB boards, reset/NMI/IRQ vectors, `@budget` verification, the iNES header and the layout report. |

Every stage's output is available as a named pass (`--emit` in the CLI, the
Passes tab in the playground).

## Runtime model

`std::runtime` boots the console, clears RAM, initializes globals, calls
`main::init` with rendering off (where CHR RAM boards upload their tiles), and then runs
the frame loop: read the controllers, call `main::update`, mark the frame
ready, wait for the NMI. The NMI handler publishes a ready frame (sprite
DMA, the video queue, palettes, scroll and control registers), runs the
audio driver, and calls `main::vblank` if defined. The MMC3 IRQ calls
`main::scanline`. Programs that want full control define `@reset` instead.

## Playground

```
main thread            compiler worker                 emulator worker
  CodeMirror  ─LSP──►  nested-wasm: LanguageServer      nested-wasm: Emulator
  inspectors  ◄─────   nested-wasm: compile → ROM ───►  frames, audio, RAM, PPU
  audio worklet ◄──────────────────────────────────────  samples
```

Projects (files and assets) live in IndexedDB. The editor speaks JSON-RPC to
the Rust language server in the compiler worker, which also builds ROMs. The
emulator worker runs frames on demand from a 60.1 Hz accumulator, returns
pixels, audio and RAM, and PPU memory when the PPU inspector is open. Builds
and frames are tagged with a generation number so a stale reply never draws
over a new cartridge.

## Tests

- `crates/compiler/tests/exec.rs` compiles programs and runs them in the
  emulator and in the constant evaluator, optimized and unoptimized, and
  requires identical results; `fuzz.rs` does the same with random programs.
- `frontend.rs`, `format.rs` and `budget.rs` cover checking, formatting and
  cycle budgets.
- `crates/cli/tests/games.rs` plays the example cartridges with controller
  input and checks every frame for dropped video writes.
- `scripts/test-wasm.mjs` requires byte-identical ROMs from the native and
  WASM compilers; `scripts/test-browser.mjs` drives the production playground.
