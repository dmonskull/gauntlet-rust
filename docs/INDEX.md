# Implementation notes

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header, DOL layout, FST filesystem, RVZ images |
| [objects-ngc-format.md](objects-ngc-format.md) | Models: header, names, texture bindings, PS2 VIF geometry |
| [textures-ngc-format.md](textures-ngc-format.md) | Texture formats, palettes, lightmaps |
| [worlds-format.md](worlds-format.md) | Level scene graph and model placement |
| [rendering.md](rendering.md) | How a level is drawn: diffuse × colour × lightmap |

## Confirmed and implemented

- Disc image: boot header, `main.dol` layout, FST — every game file read
  straight from the `.iso` (all 2,481 match an extracted copy byte-for-byte).
- Dolphin's RVZ compressed images (Zstandard, junk packing), read on the
  fly — the whole disc matches the ISO byte-for-byte.
- Locating a user's copy from a disc image (`.iso`/`.gcm`/`.rvz`), extracted
  folder or `main.dol` (`gdl-install`).
- Models, textures, lightmaps and world placement for all 67 levels.

## Reverse engineering setup

Ghidra project: `~/ghidra-projects/projects/GauntletDarkLegacy` (`main.dol`,
GameCube loader + Gekko/Broadway language). Drive it with `analyzeHeadless`
and small Java `GhidraScript`s, or the Cerberus RE bridge
(`~/start-cerberus-bridge.sh`).

A full decompilation of every function lives at
`~/ghidra-projects/exports/GauntletDarkLegacy-main.dol.c` (2,962 functions,
one `//// FUNCTION <addr> <name>` header each). Grepping it is the fastest
way in: for strings, constants, callers (`FUN_xxxxxxxx(`), and GPU FIFO
writes (`DAT_cc008000`).

Globals are addressed off two base registers set in the entry stub at
`0x80005310`: **`r2 = 0x8034D100`** (read-only constants: `unaff_r2 + -0x...`)
and **`r13 = 0x8034B4E0`** (mutable globals: `unaff_r13 + -0x...`). Map an
address to a file offset with the DOL section table
([disc-format.md](disc-format.md)).

The retail binary kept its debug strings (asserts, error messages, file
names), which is how most systems here were found.

## Not reverse engineered yet

- Characters: `PLAYERS/`, `MONSTERS/`, `CRITTER/` models, and `ANIM.PS2`
  (animation) — models likely reuse the `objects.ngc` format.
- Gameplay: the main loop, entity update, combat, items, co-op.
- `WDATA/*.WAD` per-realm resources (loaded by `FUN_8005a094`, parsed by
  `FUN_80058074`: cameras, audio, 14 named realm types).
- The rest of `WORLDS.PS2` (collision/grid tables) and audio.
