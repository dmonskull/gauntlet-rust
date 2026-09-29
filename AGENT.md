# Working notes for whoever (human or agent) picks this up

Read `README.md` first, then `docs/INDEX.md` for what's been reverse
engineered, how, and what's next.

## Rules

- No game assets, ever, in this repository. The runtime reads the user's own
  copy (disc image, extracted folder or `main.dol`) through `gdl-install`;
  nothing ships. Game files are only ever read.
- No retail addresses or decompiler placeholder names (`FUN_...`, `DAT_...`,
  `0x80xxxxxx`) in `crates/`. They go in `docs/`; code describes what the
  game does and points at the doc. `cargo test -p gdl-formats --test
  no_retail_addresses` enforces the shapes.
- Don't write a parser for a format that hasn't been confirmed against the
  binary or the disc data. A guess goes in `docs/` as a hypothesis, not in
  `crates/` as code. Where code does rest on an assumption (e.g. the 2×
  lightmap scale), say so next to it and in `docs/`.
- Every format parser gets a test that runs it over all the real data on the
  user's machine and skips cleanly when that data isn't there.
- `GDL_SCREENSHOT=out.png cargo run -p gdl-game` renders a few frames, saves
  a screenshot and exits — use it to check visual changes.

## Reverse engineering

Ghidra project `~/ghidra-projects/projects/GauntletDarkLegacy`; full
decompile at `~/ghidra-projects/exports/GauntletDarkLegacy-main.dol.c`.
`docs/INDEX.md` has the register bases (`r2`, `r13`) and the workflow.
