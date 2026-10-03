# NESted

A NES-only language and Rust/WASM toolchain, with a language server, VS Code
integration, and a browser playground for source, compiler passes, assembly,
and playable cartridges.

Development is underway. [Architecture](docs/architecture.md) records the
implementation boundaries. Build instructions, game controls, verified features,
and emulator screenshots will be added alongside runnable milestones.

The compiler is dependency-free Rust. NES execution uses
[Nessy](https://github.com/nathsou/nessy), pinned to an exact commit.
