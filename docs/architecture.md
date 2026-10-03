# NESted architecture

NESted is a NES-only, statically allocated systems language. The compiler is
implemented in Rust without third-party compiler, parser, assembler, JSON, LSP,
or WebAssembly-binding dependencies. The execution crate uses the pinned Nessy
emulator and its upstream dependencies. The web tooling uses Vite and the
TypeScript 7 native preview. VS Code integration uses the built-in VS Code API.

The same compiler and language-service implementations run natively and in WASM.
The browser talks JSON-RPC to the language service in a worker. ROM execution
runs in a separate worker. There is no JavaScript game implementation: every
example is compiled to a real iNES cartridge and executed by the emulator.

## Compiler pipeline

Source and declared binary assets -> lexer -> parser -> name/type analysis ->
constant evaluation and whole-program transformations -> byte-oriented lowering
-> 6502 machine optimization -> relaxing assembler -> mapper-aware linker.

Pass snapshots, source spans, optimization remarks, memory allocation, symbol
addresses, and instruction costs are retained for inspection. Estimated cycles
must be labeled as estimates; verified bounds require bounded control flow and
explicit hardware assumptions. Source-level timing is not inferred from NMI
membership. Inline assembly is an optimization barrier by default.

## Execution and memory

The standard game runtime initializes RAM, mapper state, the PPU, CHR RAM when
needed, palettes, and OAM. It calls `init` once and `update` once per published
frame. Main code prepares a bounded VRAM queue and OAM shadow; NMI consumes
published work. Runtime scratch, compiler storage, CPU stack, OAM, and the VRAM
queue have separate reservations. A custom entry-point mode is independent of
the standard game runtime.

Integers have fixed widths and defined modular arithmetic. Narrowing is explicit.
Recursion is rejected. Known out-of-bounds accesses are diagnosed. Dynamic raw
indexing and hardware accesses must remain visible in the language and reports.
Banked data and code must respect the declared cartridge's fixed and switched
windows; changing an iNES mapper number alone is not sufficient support.

## Games as integration tests

* **Bloom & Logic**: a Nonogram garden with multiple original puzzles.
* **Starstring**: a four-lane rhythm game with an original chiptune chart.
* **Skythread**: a precision platformer with jumping, dashing, and authored rooms.
* **Emberkeep**: a turn-based dungeon crawler with generated connected floors.

Pixel art is authored as deterministic tile/pattern source and compiled into
NES 2-bpp CHR assets. Music is composed as note, rhythm, and instrument data and
played by the NES APU. Screenshots are captured from executing compiled ROMs.
