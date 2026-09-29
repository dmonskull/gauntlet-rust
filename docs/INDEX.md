# Implementation notes

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header, DOL layout, FST filesystem, RVZ images |
| [objects-ngc-format.md](objects-ngc-format.md) | Models: header, names, texture bindings, PS2 VIF geometry |
| [textures-ngc-format.md](textures-ngc-format.md) | Texture formats, palettes, lightmaps |
| [worlds-format.md](worlds-format.md) | Level scene graph and model placement |
| [animation-format.md](animation-format.md) | Skeletons, actions and keyframed animation |
| [chunk-files.md](chunk-files.md) | The `.WAD`/`.ROM` tagged-chunk container, and game text |
| [level-population.md](level-population.md) | Items, generators, monsters, exits and player starts in `WORLDS.PS2` |
| [rendering.md](rendering.md) | How a level is drawn: diffuse × colour × lightmap |
| [camera.md](camera.md) | The play camera: level camera points, per-level distance and bounds, smoothing |
| [player-movement.md](player-movement.md) | Stats → speed, stick → walk/run, per-tick movement and turning |
| [audio-format.md](audio-format.md) | DSP-ADPCM sound banks, sound catalog, music streams, level music |
| [collision.md](collision.md) | Level collision triangles, grid, floor/wall queries, actor movement |
| [monsters.md](monsters.md) | Monster stats and tiers, the realm's monster slots, generators, placed monsters, the monster AI and mover; `CRITTER` files |

## Confirmed and implemented

- Disc image: boot header, `main.dol` layout, FST — every game file read
  straight from the `.iso` (all 2,481 match an extracted copy byte-for-byte).
- Dolphin's RVZ compressed images (Zstandard, junk packing), read on the
  fly — the whole disc matches the ISO byte-for-byte.
- Locating a user's copy from a disc image (`.iso`/`.gcm`/`.rvz`), extracted
  folder or `main.dol` (`gdl-install`).
- Models, textures, lightmaps and world placement for all 67 levels.
- Skeletons and animation for every player class and monster, skeletal and
  flipbook (`--viewer`).
- The `.WAD`/`.ROM` container every data file uses, and all game text.
- Player class stats, and walking/running a hero around a level from its
  start point at the game's 30 Hz tick with its speeds and turn rate, on the
  level's collision, followed by the game's own play camera.
- Audio: all 65 sound banks, the sound catalog and all 111 music streams
  decode; each level plays its own music, sound effects play by name.
- What populates every level — item types, placements (pickups,
  generators, monsters, doors, triggers, exits, transporters) and locators
  (player starts, boss spawn, camera points) — shown in the level view.
- Level collision (triangles, grid) and the game's floor, wall and
  move-with-collision queries, including the player's own wall/floor chain
  and its size from `PDAT`.
- Monsters: the per-type stat tables, tiers, each realm's monster slots
  (the level data's `gru`/`rat` placeholders become the realm's own),
  generators spawning at the game's rate and limits, placed monsters, and
  the chase/wander AIs walking at the player on the level's collision and
  attacking (hits are messages; no health yet).

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

- The critter system (bosses, golem, gargoyles; `CRITTER/*.WAD` beyond
  hit points and table spans) and most monster AIs beyond chase/wander;
  hand/effect glows, blending between actions.
- Gameplay: the main loop, combat and health, item behaviour, co-op.
- `WDATA/*.WAD` per-realm resources (loaded by `FUN_8005a094`, parsed by
  `FUN_80058074`: cameras, enemies, maps, 14 named realm types). The chunk
  directory, level names, camera, audio and enemy records and the monster
  tuning are parsed so far.
- The rest of `WORLDS.PS2` (header words 4 and 6).
- Audio behaviour beyond playback: music track switching, ducking,
  positional sound.
- Monster/actor type data (collision radius, step height) that the actor
  movers read (players' is decoded), and actor-vs-actor collision.
