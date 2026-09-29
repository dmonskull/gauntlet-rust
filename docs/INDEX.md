# Implementation notes

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header, DOL layout, FST filesystem |
| [objects-ngc-format.md](objects-ngc-format.md) | Models: header, names, texture bindings, PS2 VIF geometry |
| [textures-ngc-format.md](textures-ngc-format.md) | Texture formats, palettes, lightmaps |
| [worlds-format.md](worlds-format.md) | Level scene graph and model placement |
| [animation-format.md](animation-format.md) | Skeletons, actions and keyframed animation |
| [chunk-files.md](chunk-files.md) | The `.WAD`/`.ROM` tagged-chunk container, and game text |
| [rendering.md](rendering.md) | How a level is drawn: diffuse × colour × lightmap |
| [player-movement.md](player-movement.md) | Stats → speed, stick → walk/run, per-tick movement and turning |

## Confirmed and implemented

- Disc image: boot header, `main.dol` layout, FST — every game file read
  straight from the `.iso` (all 2,481 match an extracted copy byte-for-byte).
- Locating a user's copy from a disc image, extracted folder or `main.dol`
  (`gdl-install`).
- Models, textures, lightmaps and world placement for all 67 levels.
- Skeletons and animation for every player class and monster, skeletal and
  flipbook (`--viewer`).
- The `.WAD`/`.ROM` container every data file uses, and all game text.
- Player class stats, and walking/running a hero around a level at the
  game's 30 Hz tick with its speeds and turn rate.

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

- Monster stats (`CRITTER/*.WAD`), hand/effect glows, blending between
  actions.
- Gameplay: the main loop, entity update, combat, items, co-op.
- `WDATA/*.WAD` per-realm resources (loaded by `FUN_8005a094`, parsed by
  `FUN_80058074`: cameras, audio, 14 named realm types).
- The rest of `WORLDS.PS2` (collision/grid tables) and audio.
