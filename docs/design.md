# Language design: decisions and tradeoffs

NESted is designed for the Ricoh 2A03 CPU, the PPU, and an explicitly selected
cartridge board. A language that merely makes C prettier would miss the NES's
most useful opportunities: whole-program allocation, cartridge placement,
byte arithmetic and predictable publication of video work.

## The direction

Use modern, familiar expressions and control flow over a small statically
allocated core. Make widths, unchecked indexing, volatile hardware operations
and cartridge banking visible. Let the compiler own ordinary instruction
selection and static frames; let assembly own the exceptional hardware trick.
Keep the compiler independent of both the game framework and the editor host.

| Decision | Benefit | Cost and rejected alternative |
|---|---|---|
| One target: NTSC NES | Every lowering can exploit 6502 addressing and real cartridge layout. | No portable backend. PAL is a future timing profile, not a renamed region flag. |
| `u8`, `i8`, `u16`, `i16`, `bool` | Byte values stay cheap; widening is deliberate. | Word arithmetic is slower. An implicit machine-sized integer would disguise that cost. |
| Modular arithmetic, explicit casts | Position counters, phases and bit masks have defined behavior in all builds. | Overflow does not trap; bounds proofs must account for wrapping. C's signed overflow model is unsuitable here. |
| Fixed arrays and compile-time assets | Memory use is known before a ROM runs. | No heap, growing collection, runtime allocation or garbage collector. |
| `let` and `var`; no shadowing | Bindings and static frame reports stay easy to identify. | Names must be unique across a function, including disjoint blocks. Lexical visibility still applies. |
| Named scalar functions | Calls are convenient and analyzable across the whole cartridge. | No indirect functions or closures in v0.1. Dynamic call graphs complicate overlays and bank lifetimes. |
| No recursion | Locals can occupy static, overlaid frames instead of a software stack. | Recursive algorithms must use fixed work queues. The 256-byte hardware stack still carries return addresses. |
| No structs or references yet | Small compiler, explicit layouts and straightforward alias analysis. | Parallel arrays are verbose. A future struct design must expose AoS versus SoA and byte layout before adding syntax. |
| Half-open ranges, ordinary loops | Familiar game logic; short loops can unroll. | Dynamic loops have no automatic cycle bound. Range end expressions are evaluated at each iteration in v0.1. |
| Exhaustive scalar `match`, literal alternatives and inclusive ranges | Game states and input dispatch are readable; overlap and missing cases are checked, and the selector executes once. | Linear typed comparison chains are predictable but can be larger than hand-built jump tables. Pattern bindings, guards and jump-table heuristics are deferred. |
| Short-circuit `&&` and `||` | Guards and hardware conditions execute in source order. | Optimizations must preserve side effects and interrupt-visible reads. |
| Compile-time nonzero division divisor | The compiler can diagnose division by zero and reduce powers of two. | Dynamic division is deferred. Non-power-of-two division uses a software routine and costs real cycles. |
| Proven indexing, `[raw index]` escape | Constant mistakes and unsafe intervals produce useful editor errors. | Conservative proofs sometimes need an explicit raw access. Raw indexing has no runtime bounds check. |
| Byte `@shared` storage | Individual accesses are atomic with respect to instruction-boundary interrupts. | A read/modify/write sequence is not a concurrency protocol. Multi-byte publication requires explicit assembly. |
| Distinct main/NMI/IRQ frames | An NMI may preempt an IRQ without corrupting compiler temporaries. | Helpers called across contexts require distinct definitions. Implicit reentrant software stacks would consume scarce RAM. |
| Inline official 6502 assembly | Exact bus sequences and specialized raster effects remain possible. | Opaque barriers reduce optimization; users own raw addresses, stack discipline, and timing assumptions. |
| Typed A/X/Y assembly inputs; A-byte result | Most small custom instructions integrate directly into expressions. | Register outputs/clobber annotations are deferred; conservative barriers are the current contract. |
| Cartridge configuration in source | A mapper is a compilation/linking decision, including fixed windows and CHR RAM. | Board variants are deliberately limited. An arbitrary mapper number does not imply support. |
| Banked data; fixed code in v0.1 | Assets can exceed the fixed window while execution remains stable. | No banked functions/far calls yet. A useful future design needs explicit mapper-state capabilities. |
| Managed frame runtime as an optional entry mode | Normal games prepare OAM and a bounded VRAM queue without hand-writing boot code. | It consumes reserved RAM and a standard NMI. `@reset` provides custom boot mode when those choices do not fit. |
| Instruction budgets are explicit | Acyclic contracts can be checked against assembled branch costs and callees. | DMA, preemption, bus stalls and PPU phase need separate contracts. A source annotation cannot promise raster timing by itself. |
| Shared Rust compiler/LSP in native and WASM | Diagnostics and semantics agree in VS Code and the playground. | Raw WASM ABI and host transports require careful buffer ownership and error handling. |
| Caller-owned assets; no compiler I/O | Other game tools can embed compilation without adopting our CLI or emulator. | Hosts must resolve files and decide their own asset policy. |

## What “optimizing” means on this machine

The current pipeline performs typed constant folding, algebraic simplification,
power-of-two strength reduction, small pure-function inlining, constant branch
removal, unreachable statement elimination, short-loop unrolling, and dead
function removal. Lowering selects immediate arithmetic and comparisons,
constant offsets, shifts/adds for sparse constant multipliers, and zero-page
instructions. Locals and temporary arenas share space across call paths that
cannot coexist. Main, NMI and IRQ have separate pools.

Machine passes remove redundant branches, invert branches to use fall-through,
remove dead instructions after terminating transfers, remove repeated immediate
index loads, and remove selected redundant RAM loads only when flags are dead.
The assembler relaxes branches beyond the 6502's signed 8-bit displacement.

Banked reads can change mapper state and therefore cannot be discarded as pure.
Shared byte reads and raw bus reads remain observable. Inline assembly is opaque.
Optimizations are exposed as inspectable passes and counters; a transformation
must survive reference/differential execution tests before being relied upon.
This is a custom backend, not LLVM, and v0.1 is not a claim that every program
matches a skilled assembly programmer's best possible result.

## Deliberately deferred

A future version can add explicit struct layouts/SoA containers, fixed-point
numbers with declared scaling, module imports, bank-lifetime types, verified
bounded loops, and user-defined compile-time transforms. Each should earn its
complexity through a real game. Heap allocation, exceptions, inheritance,
thread abstractions and portable integer defaults do not fit the current goal.
