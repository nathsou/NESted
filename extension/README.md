# NESted for VS Code

Syntax highlighting, snippets, completion, diagnostics, hover, definitions,
references, rename, signature help, semantic tokens, formatting, and type hints
for `.nst` files. The client speaks standard JSON-RPC/LSP to `nested lsp` using
Node and VS Code's built-in APIs; it has no npm runtime dependencies.

Build the CLI with `cargo build --release -p nested-cli`, then put `nested` on
PATH or set `nested.serverPath`. A host-specific package can bundle the binary.

Run **NESted: Build NES ROM** (Ctrl/Cmd+Enter) to build the project the
active file belongs to (the nearest folder with a `main.nst`) and write every
compiler pass into its `.nested/` folder.
**NESted: Check Cartridge** runs the same compiler without writing a ROM.
**NESted: Open Playground** opens the configurable playground URL.

From the repository, `node scripts/package-extension.mjs` produces an installable
VSIX with the current host's native CLI. Install with VS Code's **Extensions:
Install from VSIX** command. Module and asset paths are relative to the
project folder.
