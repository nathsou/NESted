# nested-compiler

The NESted compiler, standard library, formatter and language server, as a
Rust library with **no dependencies and no I/O**. It builds on stable Rust
for native targets and for `wasm32-unknown-unknown`.

```toml
[dependencies]
nested-compiler = { git = "https://github.com/nathsou/NESted" }
```

## Compiling

A `Project` is a map of source files and a map of assets, keyed by
project-relative paths. You decide where they come from: a directory, an
archive, an editor's buffers or bytes baked into your program.

```rust
use nested_compiler::{compile, Options, Project};

let mut project = Project::single("use counter; fn update() { counter::tick(); }");
project.sources.insert("counter.nst".into(), "var n: u8 = 0; pub fn tick() { n += 1; }".into());

match compile(&project, Options::default()) {
    Ok(out) => {
        std::fs::write("game.nes", &out.rom)?;  // a complete iNES image
        // out.passes: MIR, optimized MIR, 6502, layout and the annotated listing
        // out.linked: symbols with addresses and sizes, RAM and bank usage
        // out.warnings: non-fatal diagnostics
    }
    Err(failure) => eprintln!("{}", failure.render()),
}
```

`Project::single(src)` makes a project whose `main.nst` is `src`. The
standard library (`std::video`, `std::audio`, ...) is embedded in the crate.
`Failure` carries diagnostics with spans into its `SourceMap`, and the
passes produced before the error.

`check::analyze(&project)` runs the front end only: names, types, proofs and
diagnostics, without generating code. It is what the language server uses
on every keystroke.

## Formatting

```rust
let pretty = nested_compiler::fmt::format(src, &nested_compiler::fmt::Style::default());
```

## Language server

`lsp::LanguageServer` handles one JSON-RPC message at a time and returns the
responses and notifications to send; the host owns the transport.
`lsp::serve_stdio()` is the standard stdio loop used by `nested lsp`.
Documents are identified by URI; give the server binary assets with
`set_asset(uri, bytes)`. It supports diagnostics, hover, completion,
signature help, definition, references, rename, formatting, document
symbols, semantic tokens and inlay hints.

## Example

`cargo run -p nested-compiler --example embed` compiles a two-file project
held in memory and prints its passes and function sizes.
`cargo run -p nested-compiler --example dump -- path/to/main.nst MIR` prints
one pass of a project on disk.
