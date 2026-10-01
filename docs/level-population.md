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
| +0x04, +0x06 | i16 × 2 → item `+0xC0/+0xC2`: a secret wall's (shape 4) first collision triangle and count |
| +0x08 | name[16]: overrides the type's model name (walls name a level object; sounds name the sound) |
| +0x18 | position (vec3) |
| +0x24 | rotation, Euler radians (vec3) |
| +0x30..+0x3C | class-specific, below |

`FUN_80063fb0` builds each placement's matrix: identity, then
`FUN_800be93c` (X), `FUN_800be9e8` (Y), `FUN_800be894` (Z) — each
post-multiplies, so for row vectors the result is Rx·Ry·Rz — then sets the
translation, and calls `FUN_800646e4(item, placement, type, matrix)`.
Once every item is made, `FUN_80063fb0` runs the mover update
(`FUN_800629ec`) once — every trigger's target now stands at its off
height — and `FUN_80064140` drops each item onto the floor below it. The
animated objects are at their first frame by then: the level load
(`FUN_80057020`) runs the world's update `FUN_80056748` once just before
`FUN_80063fb0`, and its animated-object pass (`FUN_80055d08` →
`FUN_800a7ff8`) poses each at its current frame, 0, before moving it on
(the mover update only gives the triggers' animated targets their play
flags). Every
item has an instance (its model, or a bare one), so all but the types that
keep their height (`+0x0A` bit 0) — those flagged to have no model too.
The floor is `FUN_8000d3c4`'s (4 above to 10 below, radius 1); the item
ends 0.1 above it, or 0.1 above where it was with none (`"Bad Item floor
pos"`). Then:

- a floor on a moving node (flag `0x1000`) takes the item's instance
  under its own (`FUN_800bb084`): the item rides it from then on — each
  frame its matrix is its instance's (`FUN_8005a334`), so its touch shape
  goes with it. A lift pad placed on its lowered lift rides it; one placed
  under a raised one stays where it is.
- a trigger for every player (flag `0x400`) whose floor is its target or a
  child of it becomes one that counts only stood on there (`0x100`).

The rewrite drops the items the same way (`items.rs`, with the start
poses from `mechanics::start_poses`: the movers at their off heights,
the animated objects at their first frame) and moves every rider with its
floor (`mechanics.rs`). It had dropped only items with a model, with the
movers at rest, and moved lift pads with their target's pose from rest —
which put a pad placed on a lowered lift that far below it (B5's 275, D4's
338 and 342); and it had dropped onto the animated objects at rest, so a
lift pad placed on an animated platform didn't ride it (G1's 420 on the
swinging arm's platform and 424 on the lowered lift stayed behind as the
platform left, and the hero standing on the platform at its other end
couldn't send it back).

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

That level id is an index into the realm WAD's level records, not a
folder: `FUN_8005638c` → `FUN_80058074(id)` takes record `id & 0xFF` of the
WAD's `LEVL` chunk (the first one when the index is past the last) as the
level (`r13-0x72bc`) and loads `"levels/level%s"` with its name (`+0x08`);
finishing a level marks that id. The records aren't all in the folders'
order: CASTLE.WAD lists A1, A6, A2, A3, A4, A5, DREAM.WAD J1, J2, J3, J6,
J4, J5 and SKY.WAD K2, K3, K4, K1, K5 (the other realms in order; each
realm's boss level last). So the tower's `a2` portal leads to `levelA6`,
`a6` to the boss level `levelA5`, `j4` to `levelJ6`, `k1` to `levelK2`
and `k4` to `levelK1`. The runtime maps ids and folders through the WADs
the same way (`gdl_formats::LevelOrder`, `exits::LevelIds`); it had loaded
the folder the code spelled (`levelA2` for `a2`) and marked a level
finished by its folder's digit. TOWER.WAD has only L1 and L2 and TEST.WAD
T1–T3, so `levelL3` (a stripped copy of the tower whose triggers move no
node) and `levelT4` (empty) can't be loaded, nor `DEMO1` (no `level`
prefix; no WAD names it) or `ORIGlevelL1`; no exit names L2, L3 or a test
level.

Transporters: `FUN_80064600` links each to the transporter whose id is its
destination (`"Transporter id %d no dest %d"`). Triggers: `FUN_8006437c`
checks ids are unique and chains `next` ids (`"Linked Triggers loop"`).

What the build leaves out or changes (`population.rs`, `items.rs`): an
obelisk (powerup subtype 12) is freed as it's made — never in a level; a
KEY placed with a count above 1 is made the level's `KEYRING` type; a
random type picks one of its choices (`((seed >> 5) + slot) % count`;
the rewrite takes the first); every trigger's target is registered as a
mover, for the party or not ([mechanics.md](mechanics.md)).

### Auditing the levels

`cargo run -p gdl-formats --example level_audit -- <game>/Gauntlet [level]
[--triggers] [--anim]` checks every level against these rules: movers
only triggers for more players register (held at their off height, or a
bridge hidden), trigger links (chains to missing ids, quest gates, odd
flags), placements built differently (obelisks, key rings, random types,
rotators), blocking items without a model, secret walls, and trigger
targets under an animated object with no animation of their own;
`--triggers` lists the party's triggers with a warp point on the floor
under each and what it moves (or plays), `--anim` where each animated
target's collision is at rest, at its first frame and at its last.
The link checks also flag triggers whose shared id the game clears, ids
of 128 and up that a camera point never reaches, and chains whose id only
a quest gate has ([mechanics.md](mechanics.md), "Chains"). `--reach`
looks for floors down lines from above the highest to below the lowest
an animated object's first or last frame takes its collision, not just
within the level's box (`WORLDS.PS2` header bounds): J4's flying
platforms start above it (J4A14 at 75, J4A40 at 103; the top is 65) or
below it (J4A18 at −59; the bottom is −48), which made J4's 335, 366,
422 and 429 read as unreachable.

The last round (A5, A6, J1–J6, K1–K5, L1–L3, S1–S9, T1–T4, DEMO1),
with `--triggers --anim --falls --walls --reach --ridden`:

- **A5** (the castle's boss): the nine ELEVSWs all lower `A5ELEVATOR`
  (eight chain to the first); three safe rocks (`SAFEROCK1`). **A6**: six
  triggers, all reached (the tour saw each arrive), three secret walls,
  four shot-down falls (`0x34`), two key rings.
- **J1, J2, J6, K2**: key rings and secret walls only. **J3**: two movers
  only triggers for more players register (`J3ELEV26` held 5 below,
  `J3PILLAR` at its first frame — the game registers every trigger's
  target); the lift pad 483 with the off switch 495 on `J3ELEV2` is the
  game's own clash ([mechanics.md](mechanics.md), "Movers"). **J4**:
  every trigger is reached once the platforms are counted where they
  start: 335 rides `J4A14`, which trigger 421 brings down from 75 to 37;
  366 and 429 ride `J4A40` (365 brings it from 103 down to 23); 422 rides
  `J4A18` (339 raises it from −59 to 26). The game drops them onto the
  platforms' first frames and they ride them, as the runtime does.
  **J5**: the dream's boss level (its WAD's last record): a turbo
  powerup, three more for more players.
- **K1**: `K1PAD8` held at its first frame (only a trigger for more
  players plays it). **K3** and **T1**: a node moved by an ACTIVESW and a
  hit switch (`K3ELEV13`, `T1ELEV1`): the game keeps the first
  registration's kind and flags ("TRIGGER TYPES DIFFER"), as the runtime
  does. **K4**: trigger 438 (the balloon) chains to id 100, which no
  trigger has: nothing more. K3, K4 and T2 have random types (the runtime
  takes the first choice, a stand-in). **K5** (the sky's boss): three
  safe rocks (`SAFEROCK3`).
- **L1** (the tower): 81 and 83, pads placed without a model with a touch
  radius of 25 at (0, −59.7, 81.3), are 21 below the lowered H4 platform
  `L1ELEV669` (its floor at −38.5) with no floor in their reach (the
  touch reaches 5.5 up and down): only a hero falling past them touches
  them — 81's chain (to the quest gate 104) leads nowhere, 83 calls
  `L1ELEV02` (id 199), as its own trigger 84 and the pads 85–93 and 148
  do. The pad 145 on the wizard's pedestal (id 240) has no camera cut
  ([mechanics.md](mechanics.md), "Chains"). **L3**: not a level the game
  loads (above, "Exit codes"); its triggers move nothing. **L2**: two
  exits only, and no exit leads there.
- **S1–S9**: no exits; the secret realm's timer sends the heroes back
  ([items.md](items.md), not in the runtime yet). S3: sixteen paired
  transporters and a rotator with no node. S4, S6, S7: secret walls;
  S7's `S7WELL` is held at its first frame (a trigger for more players).
- **T1–T3, DEMO1**: test levels the game doesn't reach (no exit names
  them; DEMO1 isn't in any WAD); T2 lays out the powerups (random types,
  key rings, an obelisk the game never makes). T4 is an empty folder.

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
| 9 | transmitter linked to the triggers whose id, read as a signed byte, is `index` (`"Two cameras link to trigger"`; the tower keeps 170–183, 198 and above 200 for its own cameras — [mechanics.md](mechanics.md) "Chains") |

Outside the tower realm (13) the start index is forced to 0, and every
level but the two hub levels (`levelL1`, `levelL3`: 12 entries each) has a
single start. `FUN_800669d4(entry)` selects a start (falling back to 0);
`FUN_800668e4` picks the nearest one.

Lookouts (kinds 8, 10) are kept in the `0x6C`-byte table at `0x80257e88`
(`FUN_80066258`, at most 20), the locator's param at `+0x6A`;
`FUN_80067194(param)` finds one ("CAN'T FIND LOOKPUT PARAM"). Their matrix
(`FUN_8006703c` with its third argument set) is built from the angles
(−x, y + π, z) by `FUN_800bd344`, an Euler builder of its own (sines
negated) whose yaw turns +Z toward (sin y, cos y) — the opposite sense to
placements'. The runtime's `population::lookout_transform` does the same.

Which entry (`FUN_800a2ba8`): a requested one (`r13-0x7cb8`, set by
`FUN_800a117c` to the start nearest a point), else the index of the realm
last played outside the tower (`r13-0x7230`, set as each non-tower level
loads, `0xD` at boot) in `0x801244dc`: 13, 7, 2, 1, 11, 4, 3, 9, 10, 5, 6,
8 — the tower's centre for a new game, then the starts beside the G, B,
A, K, D, C, I, J, E, F and H gates. Each entry has its starting camera
(kind-1 transmitter with that index) for the arrival shot. The runtime
does both (`population::start_entry`, `play_camera.rs`).

## Item models

`FUN_8006776c` loads `"items/%s"` (the realm's folder — named in its WAD,
e.g. `levelA` for CASTLE) and `"items/level%s"` (the level's own, if its
`objects.ngc` exists); `"powerups"` is loaded too. `FUN_80067338` looks
each type's name up as an atree (`FUN_800674e0`: realm items, powerups,
level items; generators also every loaded monster), and `FUN_80065b4c`
builds it from the atree, or else finds a plain object by name, then
with `L1` appended, then `ROOT` appended to that — name + `L1ROOT`
(`strcat`, `FUN_800e7814`; `FUN_800b8684` compares 15 characters). Generator models are plain
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
