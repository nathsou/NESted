# Playground and cartridge refinement audit

This pass uses the compact toolbars and pane-based layouts of
[nathsou/goldl](https://github.com/nathsou/goldl) and
[nathsou/yodl](https://github.com/nathsou/yodl) as references. The playground
continues to compile and execute real ROMs locally.

## Root causes corrected

| Finding | Correction | Regression evidence |
|---|---|---|
| A global `paint_state` shared a generated name with `paint(state)`'s parameter, corrupting Nonogram cells. Compiler temporaries and function end labels had related collision risks. | Globals, length-encoded locals, temporaries, branch labels and function ends use separate symbol namespaces. The assembler rejects duplicate equates as well as labels. | Optimized/unoptimized execution with deliberately colliding user names; every Nonogram cell checked on every test frame. |
| Full-screen writes resumed PPU rendering mid-frame and could retain old queued writes. | Rendering disables NMI and discards the old queue, then resumes at the next vertical blank with prepared OAM and scroll. Scene transitions hide old actors. | Controller-driven puzzle, room, menu and floor transitions; screenshots from executing ROMs. |
| Large rhythm notes, two-sprite receptor flashes, decorative platformer particles and crowded dungeon rows could exhaust scanline sprites. | Single-sprite note heads/flashes, non-overlapping sustain flashes, fewer particles and at most three enemies per row. | Every rendered frame in cartridge tests checks the eight-sprite scanline limit. |
| Pause/resume text could compete with rhythm HUD updates in the 40-write queue. | Pause transitions get their own presentation frame; gameplay resumes on the following update. | Pause/frozen-note/resume tests; video overflow checked every frame. |
| Catch-up emulation discarded all but the last frame's audio. Old cartridge responses could update a new scene. | Preserve every frame's audio and tag requests/responses by cartridge generation. Use the NTSC frame accumulator; clear held buttons on focus loss. | Production browser switching, stepping, rebuilding and audio activation. |
| Formatting/completion could observe the previous source during the editor's debounce interval. | Flush document changes before LSP queries. | Formatting immediately after an edit, without waiting for debounce. |
| Assembly semicolon comments containing braces or Unicode confused the language lexer. | Assembly comments remain opaque to parsing and formatting. | Formatting preserves identical ROM bytes with a brace/Unicode assembly comment. |
| VS Code's existing assembly snippet contained an invalid JSON escape. | Encode the snippet's literal dollar escape as valid JSON. | Parse every extension JSON asset and package the VSIX. |

## Playground

A 44-pixel dark header replaces the large hero and cartridge gallery. A native
select chooses cartridges. Draggable/keyboard-resizable panes, Workbench/Play/
Code layouts, integer NES canvas scaling and mobile Source/Game/Output tabs
keep the tools accessible. Source files can be opened locally; browser drafts
persist, and resetting an example can restore the previous draft. Loading
failures release the cartridge picker; worker startup errors/timeouts surface
in the workbench.

Browser checks cover all four cartridges, relative deployment paths, LSP,
diagnostics, rebuilds, audio activation, header/picker dimensions, integer
canvas scale, pane resizing, layout switching, 390-pixel mobile panes, fresh
formatting and draft restoration. Screenshots are captured from current ROMs.

## Games

- **Nonogram:** plain white grid, black fills, pencil crosses and a red cursor;
  completed line clues fade. Sixteen puzzles, including a gentle first puzzle,
  are independently checked for one solution and solvability through line
  deductions without guessing. Held-button painting and a 64-edit undo ring
  support quick input. Every puzzle is solved through controller inputs, next
  and previous navigation wrap, and undo is exercised.
- **Starstring:** readable black playfield, compact diamond notes, sustained
  tails, progress, ghost-tap penalties and pause/resume. The melody's lead-in
  matches note travel time for each scroll speed. Controller tests complete
  the entire 192-event song on all three difficulties without misses.
- **Skythread:** clear stone terrain and a contrasting climber, slower run
  animation, a dash trail following the last position, accurate grounded
  presentation and a background dash indicator. Room/death transitions reset
  the gravity phase. Controller routes complete all six rooms without deaths;
  separate tests exercise jump, dash and restart.
- **Emberkeep:** restrained stone palettes, wall-blocked lantern rays,
  persistent exploration, nearby redraw priority, delayed actor display until
  its floor is ready, combat/item messages, pause, capped healing and traps on
  later floors. Enemy tells remain visible; damage is capped at one hit per
  player turn. Tests verify connected floors and win a six-floor run through
  controller navigation, combat and potions.

## Language and checks

`match` evaluates its selector once and supports exhaustive integer/bool
patterns, alternatives, inclusive ranges, scalar results and statement blocks.
Overlap, reversed ranges, nonconstant patterns, missing cases and incompatible
arm types are diagnosed. Range arms contribute bounds facts. Comparisons are
lowered to typed branches; jump tables, guards and pattern bindings remain
future work.

The shared Rust formatter expands blocks, spaces operators, wraps arrays and
respects LSP tabs/spaces. It preserves literal spelling and assembly content;
representative formatted programs compile to byte-identical ROMs in optimized
and unoptimized modes. CLI `fmt --check` enforces example presentation in CI.

Validation uses Rust formatting/lint checks, release workspace tests,
byte-for-byte native/WASM cartridge parity, finite non-silent APU samples,
production browser checks, extension packaging and GitHub Pages deployment.
Per-frame queue/sprite checks cover the exercised routes and fixtures; they are
not a proof for every possible input sequence or cartridge board variant.
