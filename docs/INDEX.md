# Implementation notes

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header and DOL executable layout |
| [objects-ngc-format.md](objects-ngc-format.md) | Model file (`objects.ngc`) header — counts, offsets, byte order |

## Ghidra project

`~/ghidra-projects/projects/GauntletDarkLegacy` — `main.dol` imported and
auto-analyzed (2962 functions, entry point `0x800051fc`). Driven headlessly
via `analyzeHeadless` and the Cerberus RE bridge
(`~/start-cerberus-bridge.sh`).

Debug strings survived in the retail binary (asserts, error prints, file
paths) — grepping defined strings for things like `.ngc`, `WORLDS`, `LEVELS`
and decompiling their referencing functions is the fastest way in so far;
see `objects-ngc-format.md` for how that found the model header.

## Confirmed so far

- GameCube disc boot header + `main.dol` layout (`disc-format.md`)
- `objects.ngc` header: version magic, object/sub-array counts and offsets
  (`objects-ngc-format.md`). Verified by parsing all 67 real `objects.ngc`
  files across every level — `cargo test -p gdl-formats`.

## Not started yet

- `objects.ngc` per-entry array layouts (mesh/strip data, the `num_b` and
  `num_d` arrays) — see "Not yet reverse engineered" in
  `objects-ngc-format.md`
- `textures.ngc` (GameCube native texture format)
- `WORLDS.PS2` (level/world layout — object placement, cameras, audio per
  the `"World Data %s has no cameras"`-style debug strings)
- `ANIM.PS2` (animation data)
- Actor/monster data (`MONSTERS/`, `CRITTER/`)
- Main game loop / entity update
- Co-op and combat systems
