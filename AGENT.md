# Working notes for whoever (human or agent) picks this up

Read `README.md` first, then `docs/INDEX.md` for what's actually been
reverse engineered so far.

## Rules

- No game assets, ever, in this repository. The runtime reads a disc image
  (or extracted disc tree) the user points it at via env var; nothing ships.
- No retail addresses or decompiler placeholder names (`FUN_...`, `DAT_...`)
  in `crates/`. A finding from Ghidra becomes a named, documented Rust type
  in `gdl-formats` (or a note in `docs/`) before it's used anywhere else —
  the retail offset itself stays in the Ghidra project, not in shipped code.
- Don't write a parser for a format that hasn't actually been confirmed
  against the binary or disc data. A guess based on "other GC games do it
  this way" belongs in `docs/` as a hypothesis, not in `crates/` as code.
- Ghidra project: `~/ghidra-projects/projects/GauntletDarkLegacy`. Drive it
  headlessly (`analyzeHeadless` + a `GhidraScript`) or through the Cerberus
  RE bridge (`~/start-cerberus-bridge.sh`, then `cerberus-re ...`).
