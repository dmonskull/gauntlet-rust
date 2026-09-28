# Implementation notes

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header and DOL executable layout |
| [objects-ngc-format.md](objects-ngc-format.md) | Model file (`objects.ngc`) header, byte order, and material/texture bindings |
| [textures-ngc-format.md](textures-ngc-format.md) | Texture file (`textures.ngc`) — status: not yet reverse engineered, what's known |

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
- `objects.ngc`'s `objects_b_offset` array (material/texture bindings): 4 of
  its fields, enough to resolve which bytes of the sibling `textures.ngc`
  each binding's texture lives at. Verified against all 8,387 textured
  bindings across every level.

## Corrections to earlier notes

The `"World Data %s has no cameras"` / `"No world data file: %s"` style
strings turned out to belong to a *different*, global per-world-type
`"gar_%s.wad"` resource system (14 named types, e.g. `"castle"`), reached
from `FUN_8005a094`/`FUN_80058074`/`FUN_800a8dfc` — **not** the per-level
`WORLDS.PS2` file sitting in each `LEVELS/levelXX/` folder. The real
`WORLDS.PS2` loader hasn't been located yet.

## Not started yet

- `objects.ngc` per-entry array layouts (mesh/strip data, the `num_b` array
  beyond its 4 confirmed fields, and the `num_d` array) — see "Not yet
  reverse engineered" in `objects-ngc-format.md`
- `textures.ngc` — no header found yet; width/height/GX-format still
  unknown (`textures-ngc-format.md`)
- `WORLDS.PS2` (the actual per-level file — not yet located in the binary;
  see "Corrections" above)
- `ANIM.PS2` (animation data)
- Actor/monster data (`MONSTERS/`, `CRITTER/`)
- Main game loop / entity update
- Co-op and combat systems
