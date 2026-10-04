# nested-compiler

An embeddable NES compiler, assembler and language service written in Rust with
**no third-party dependencies**. It emits complete iNES ROMs for NROM, MMC1,
UxROM and MMC3. It requires no filesystem, process, emulator, JavaScript host,
allocator protocol or installed assembler beyond Rust's standard library.

```rust
use nested_compiler::{compile, Assets, Options};

let source = "var ticks:u8=0; fn update(){ ticks += 1; }";
let cartridge = compile(source, &Assets::new(), Options::default())
    .expect("valid NES program");
assert_eq!(&cartridge.rom[..4], b"NES\x1a");
// Write cartridge.rom to disk, give it to an emulator, or serve it to a browser.
// cartridge.passes, cartridge.assembly, cartridge.generated and cartridge.json()
// expose intermediate code, source locations, timing, symbols and memory layout.
```

Add a Git dependency to another Rust project's `Cargo.toml`:

```toml
[dependencies]
nested-compiler = { git = "https://github.com/nathsou/NESted", package = "nested-compiler" }
```

Pin `rev` to a commit for reproducible builds. The public entry points are
`compile`, `asset_paths`, `Assets`, `Options`, `Compilation`, and
`lsp::LanguageServer`, `formatter::format_source` and `formatter::FormatOptions`. `frontend::analyze` supplies editor analysis without
assets or linking. The internal frontend/backend modules remain experimental.

Assets are a caller-owned `BTreeMap<String, Vec<u8>>`. Keys match source literals,
e.g. `assets/player.chr`. `asset_paths(source)` discovers declared asset paths;
the host decides whether to resolve them from a project directory, archive,
network service, editor buffer, or embedded bytes. `compile` performs no I/O
and never fetches a dependency or asset. Parallel calls use independent state.

Failures return `Vec<frontend::Diagnostic>` with UTF-8 byte spans. Use
`lsp::diagnostics(source, errors)` to convert them to UTF-16 editor ranges.

The same crate builds on native Rust and `wasm32-unknown-unknown`. Browser hosts
can use the separate `nested-wasm` crate's raw ABI. That adapter combines this
compiler with `nested-runtime`, the optional Nessy execution adapter. Native
projects that only need ROM generation depend solely on `nested-compiler`.

`lsp::LanguageServer::handle(Json)` accepts a JSON-RPC message and returns its
responses and notifications. The host owns transport, scheduling and document
storage policy. `nested-cli lsp` provides stdio framing; the playground supplies
a worker transport. Neither transport is required to embed the services.

Run `cargo run -p nested-compiler --example embed` for a complete small example.
The repository's `docs/language.md` describes language behavior and hardware
contracts; `docs/design.md` records the design decisions and tradeoffs.
