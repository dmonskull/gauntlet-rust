# Level population — items, generators, monsters, starts

Implemented in [`crates/gdl-formats/src/population.rs`](../crates/gdl-formats/src/population.rs),
shown by [`crates/gdl-game/src/population.rs`](../crates/gdl-game/src/population.rs)
(`I` cycles models + markers / models / markers / hidden; `GDL_POPULATION`
= `all` | `models` (default) | `markers` | `off`; `GDL_CAMERA=start` starts the
camera behind the player start).

Verified: all 68 `WORLDS.PS2` files parse — 21,111 placements, 5,742
locators. 99.7% of placements lie inside their level's bounding box (the
rest are mostly in the `levelT*` test levels); every level has exactly one
entry-0 player start inside its bounds; every exit names an existing level;
every transporter's destination exists; container contents, random-type
choices, placement type indices and all 1,441 trigger/rotator target node
indices are in range (1,102 are model nodes, the rest groups such as
`B5PLAT1`, `L1DRAWB66`, `C2NSNWBRIDGE` — lifts, drawbridges, bridges);
all 600 named obstacles are objects in the level's `objects.ngc`.

## Where it lives

Not in `WDATA/*.WAD` (those are per-realm: enemy sets, cameras, sounds,
per-level records) — the population is **in each level's `WORLDS.PS2`**,
in three tables the header points at:

| header words | table | stride |
| --- | --- | --- |
| 18, 19 | item types (count, offset) | `0x50` |
| 20, 21 | placements | `0x3C` |
| 22, 23 | locators | `0x1C` |

`FUN_800a964c` (the world loader, see [worlds-format.md](worlds-format.md))
stores them in the world struct at `0x8028c46c`: `+0x68` item types
(`DAT_8028c4d4`), `+0x6C` placements (`DAT_8028c4d8`), `+0x70` locators
(`DAT_8028c4dc`), counts at `+0x74/+0x78/+0x7C` (`DAT_8028c4e0/e4/e8`).
It byte-swaps them field by field — which is how the field sizes below are
known; the placement parameters are swapped per class of the placement's
item type.

Level setup (`FUN_80057020`) then calls `FUN_80063fb0` (items) and
`FUN_80066258` (locators).

## Item types (`0x50` bytes)

| offset | field |
| --- | --- |
| +0x00 | class (i32): −1 random, 1 POWERUP, 2 CONTAINER, 3 GENERATOR, 4 ENEMYINFO, 5 TRIGGER, 6 TRAP, 7 DOOR, 8 DAMAGETILE, 9 EXIT, 10 OBSTACLE, 11 TRANSPORTER, 12 ROTATOR, 13 SOUND — names from the game's table at `0x801185b0` |
| +0x04 | subtype (i32); names at `0x801185e8`: 1 GOLD 2 KEY 3 FOOD 4 POTION 5 WEAPON 6 ARMOR 7 SPEED 8 MAGIC 9 SPECIAL 10 RUNESTONE, 20–29 trigger kinds (BRIDGEPAD…LIFTEND), 40–49 obstacle kinds (FALLING, SAFEROCK, WALL, BARREL, EXP BARREL, POI BARREL, CHEST…) |
| +0x08 | u16 collision shape (0 none, 1 cylinder, 2 sphere, 3 box, 4 walls), u16 bit 0 keeps height ([items.md](items.md)) |
| +0x0C..+0x1C | 4 × f32 extents: touch radius, vertical reach, box half widths along X and Z; the item's visibility radius is twice the larger of the first two |
| +0x1C | vec3: centre offset, turned with the item (`FUN_8005a400`) |
| +0x28 | name[16]: model/atree name, or the monster for generators/monsters |
| +0x38 | i32, not named |
| +0x3C | i32 value: potion colour 1–4, or the bit of the power granted |
| +0x40 | i16 amount: gold value, food health, key count |
| +0x42 | i16 → item `+0xCF` (−1/−2 on most) |
| +0x44 | i16 hit points (generators multiply by strength; debug `"%s (HP=%d)"`) |
| +0x46 | u16 item flags → item `+0xC4` |
| +0x48 | i16 (damage tiles: damage-related, `×−3`) |
| +0x4A | i16 duration, seconds (→ float) |
| +0x4C | 0 on disc; the atree pointer, filled in at load by `FUN_80067338` |

**Random** types (class −1): `+0x04` is a count, `+0x08` up to 16 i16
type indices. `FUN_800646e4` loops while the class is −1, picking
`index = (seed >> 5 + item slot) % count`.

## Placements (`0x3C` bytes)

| offset | field |
| --- | --- |
| +0x00 | i16 item type index (`"NewItem: bad index"` if negative) |
| +0x02 | player count gate (`FUN_80065d84`): 0 always; 1–10 at least n players; >10 exactly n−10 |
| +0x03 | flags: bit 0 → item flag `0x40` (the item updates off screen too); bit 1 → no model; bit 2 → model flag `0x80000` |
| +0x04, +0x06 | i16 × 2 → item `+0xC0/+0xC2` |
| +0x08 | name[16]: overrides the type's model name (walls name a level object; sounds name the sound) |
| +0x18 | position (vec3) |
| +0x24 | rotation, Euler radians (vec3) |
| +0x30..+0x3C | class-specific, below |

`FUN_80063fb0` builds each placement's matrix: identity, then
`FUN_800be93c` (X), `FUN_800be9e8` (Y), `FUN_800be894` (Z) — each
post-multiplies, so for row vectors the result is Rx·Ry·Rz — then sets the
translation, and calls `FUN_800646e4(item, placement, type, matrix)`.
`FUN_80064140` then drops each item onto the floor below it (`"Bad Item
floor pos"` when there's none).

Parameters by class (from `FUN_800646e4` and the debug display
`FUN_8002e650`, which prints `"ITEM %02X (%dP)"` and a line per class):

| class | +0x30 | +0x32 | +0x34 | +0x36 | +0x38 |
| --- | --- | --- | --- | --- | --- |
| POWERUP | i16 count (KEY with >1 becomes KEYRING) | | | | |
| CONTAINER | i16 contents: item type index | | i16 | | |
| GENERATOR | i16 strength 1–3 | i16 AI (default per monster, `0x8011b7a0`) | i16 max (0 → 10/5/2 by strength) | i16 rate (0 → 5/10/15): waits 6 × rate video fields between monsters ([monsters.md](monsters.md)) | |
| ENEMYINFO | i16 level (tier) | i16 AI | f32 awareness range | | i16 |
| TRIGGER | i16 `WORLDS.PS2` node it moves (−1 none) | u16 flags (high byte kept) | u8 time (0xFF = none) | u8 id, u8 next id (chains) | i16, i16 |
| EXIT | i32: 0 = go to the code at +0x34 | | char[8] level code (`A6`, `g1`) | | |
| OBSTACLE | i16 subtype override | i16 count | | | |
| TRANSPORTER | i32 id | | i32 destination id | | |
| ROTATOR | i32 node index | | i32 | | f32 |
| SOUND | f32 radius (`"RAD=%d"`) | | i32 | | i16, i16 |
| DAMAGETILE | i16 damage override | i16 | | | |

Generator debug line: `"%s (%s-%d) Lv%d Max=%d"` = class, monster, AI,
strength, max; model `"GEN_%s%d"` (monster code + strength), or
`"GEN_SPECIAL%d"`; a type named `BOSSGEN` generates the level's boss.
Strength < 1 prints `"Generator at %.1f %.1f %.1f has strength %d"`.

Exit codes (`FUN_80057b2c`): the letter is matched against the realm table
at `0x8011bf08` (`0x2C`-byte entries: realm id, name `castle`…, letter at
`+0x0F`: A castle, B mount, C desert, D forest, E temple, F hell, G town,
H battle, I ice, J dream, K sky, L tower (id 13), S secret, T test) and the
digit gives the level: `realm << 8 | digit − '1'`.

Transporters: `FUN_80064600` links each to the transporter whose id is its
destination (`"Transporter id %d no dest %d"`). Triggers: `FUN_8006437c`
checks ids are unique and chains `next` ids (`"Linked Triggers loop"`).

## Monster names

Item type names for generators/monsters match (case-insensitively,
`FUN_800c83e8`) the first name in the `0x24`-byte table at `0x8011a9a8`
(`FUN_80051f44`): id, name[16] (`sco`, `tro`, `dem`, `rat`, `gru`…), code
(`SCO`…) used in `GEN_<code><n>`. 44 entries; ids skip 28. Names outside it
(e.g. `GARGOYLE`, `ICM`, `LOW` in the shared type list) resolve to −1 —
they're unused in the levels that carry them. Levels only name placeholder
monsters (`gru`, `kni`, `rat`), which the realm's enemy list replaces
([monsters.md](monsters.md)). The display-name table at
`0x8011afd8` (`SCORPION`, `TROLL`…) is indexed by the same id.

## Locators (`0x1C` bytes)

`u8 kind, u8 param, i16 index, vec3 position, vec3 rotation (Euler)`.
`FUN_80066258`:

| kind | meaning |
| --- | --- |
| 1, 2 | "transmitter" (camera point, `"> MAX_TRANSMITTERS"`, 256); 1 is also the starting camera for entry `index` |
| 3, 4 | transmitter, second kind (X angle negated); 3 is remembered specially |
| 5 | milestone (`"> MAX_MILESTONES"`, 128) |
| 6 | boss spawn: builds a matrix and spawns monster set 4 there (`FUN_8001bb54`); present once in each boss level only |
| 7 | **player start**: `index` is the entry; position; `rotation.y` is the facing handed to the players (`r13−0x6fe0`) |
| 8, 10 | lookout (`"> %d LOOKOUTS"`, 20) |
| 9 | transmitter linked to the trigger with id `index` (`"Two cameras link to trigger"`) |

Outside the tower realm (13) the start index is forced to 0, and every
level but the two hub levels (`levelL1`, `levelL3`: 12 entries each) has a
single start. `FUN_800669d4(entry)` selects a start (falling back to 0);
`FUN_800668e4` picks the nearest one.

## Item models

`FUN_8006776c` loads `"items/%s"` (the realm's folder — named in its WAD,
e.g. `levelA` for CASTLE) and `"items/level%s"` (the level's own, if its
`objects.ngc` exists); `"powerups"` is loaded too. `FUN_80067338` looks
each type's name up as an atree (`FUN_800674e0`: realm items, powerups,
level items; generators also every loaded monster), and `FUN_80065b4c`
builds it from the atree, or else finds a plain object by name, then name
+ `L1`, then name + `ROOT` (`FUN_800b8684`). Generator models are plain
objects `GEN_<code><n>L1` in `MONSTERS/<code>/objects.ngc`.

The runtime uses `ITEMS/level<realm letter>` — every realm WAD names the
folder of its own letter — and shows atrees at their rest pose until the
item's actions play (`items.rs`). Flipbook nodes (the barrels `BAROBJ`,
`BAREXP`, `BARPOI`: idle, breaking and broken frames, `BAROBJIDLE15F`,
`BAROBJACTI16F`…) draw their action's frame, the frame objects in a row
from each action's first. Only placements the player count allows are
made (`+0x02`, `FUN_80065d84`): on levelA1 233 of 486 are for 2–4
players — extra barrels and pickups that a one-player game doesn't have.

## Not decoded yet

- Item type `+0x08`, `+0x38`, `+0x42`, `+0x48`; the `extent` floats beyond
  "radius from the first two".
- Placement `+0x04/+0x06`; ENEMYINFO `+0x38`. (The generator rate, the
  ENEMYINFO range and what generators and monsters do with them:
  [monsters.md](monsters.md).)
- Which model axis is a player's "forward" (the game derives items' yaw
  from where +Z goes: `atan2(m20, m22)`).
- Transmitter kinds 1–4/9 in detail (camera behaviour), milestones.
- Items are dropped to the floor by collision at run time; markers use the
  stored positions.
