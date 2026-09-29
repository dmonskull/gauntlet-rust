# The level's working parts: triggers, moving geometry, hazards, breakables

What drives a level's moving parts: trigger pads and switches, the world
nodes they move (lifts, bridges, doors, trap walls), rotators, damage
tiles, damaging walls, and items that break when hit. Addresses are in
`main.dol`; `r2`/`r13` as in [INDEX.md](INDEX.md). Record layouts are in
[level-population.md](level-population.md) (placements, item types),
[items.md](items.md) (the `0xF0`-byte item record) and
[collision.md](collision.md) (world nodes, collision queries).

**Status**: decoded, mostly not built. See "In this rewrite" at the end
for what exists.

Constants below were read from the DOL at `r2`-relative addresses; where
the code reads one as a double it's given as such.

## Triggers (item class 5)

### Placement bytes

Read by the item constructor `FUN_800646e4` (class 5 case):

| offset | field |
| --- | --- |
| `+0x30` | i16 target `WORLDS.PS2` node (checked against the node count `DAT_8028c4cc`, `"Item has wobjidx > max"`; −1 / −2 = none; a node without an instance logs `"ITEM TARGET WOBJ %s: OBJECT NOT FOUND"`) → item `+0xDC` (node pointer) |
| `+0x32` | u16 flags; the high byte is kept, the low byte comes from the subtype (below) → item `+0xE0` |
| `+0x34` | u8 **touch radius**, not a time: item `+0xE8` = 0.5 (`r2-0x6790`, double) × byte; 0xFF → 0.01 (`r2-0x6610`) |
| `+0x35` | i8 mover sound parameter → `DAT_80257b30` (below) |
| `+0x36` | u8 id → item `+0xE2` |
| `+0x37` | u8 next id → item `+0xE3` |
| `+0x38` | i16 off height × 0.1 (`r2-0x67f8`, double) |
| `+0x3A` | i16 on height × 0.1 |

The touch test (`FUN_8005f0e0`, class 5) uses a cylinder of radius
`+0xE8` when it's positive; otherwise the type's own shape, with LIFTPAD
(0x1B) doubling its radius (× `r2-0x6850` = 2). Quest triggers (flag
0x40, id < 100, `FUN_800a1928` true) double it too. Item flag 0x400 and
trigger flag 0x200 (chained-to, below) make a trigger untouchable.

### Default flags by subtype

The trigger flags' low byte (item `+0xE0`) is set from the subtype; the
placement's high byte is OR'ed on:

| subtype | name | flags |
| --- | --- | --- |
| 0x14 | BRIDGEPAD | 0x10 |
| 0x15 | DOORPAD | 0x08 |
| 0x16 | BRIDGESW | 0x12 |
| 0x17 | DOORSW | 0x0A |
| 0x18 (and any other) | ACTIVESW | placement `+0x32` \| 8 |
| 0x19 | ELEVPAD | 0x804 |
| 0x1A | ELEVSW | 0x02 |
| 0x1B | LIFTPAD | 0x80C |
| 0x1C | LIFTSW | 0x09 |
| 0x1D | LIFTEND | 0x0A |

ACTIVESW is by far the most common on the disc (flags 0x2 on 860 of
1,700-odd triggers). Subtype 0x1F is a switch that's hit rather than
touched (below).

The target node gets `+0x14` = subtype \| (flags & 0xFF) << 8 — its "kind"
byte and kind flags — and `+0x16` / `+0x17` (state, previous state) = 0,
and node flag `0x100000`. A target that is a particle-system node (flag
`0x800`) also gets `0x10000000` (left out of the moving-node collision
pass).

### Chains

`FUN_8006437c` checks trigger ids are unique (`"%d triggers with id =
%d"`, `"%d special triggers ..."`) and links each trigger with a `next`
id to the trigger whose id matches (not flag 0x40): item `+0xE4` = that
item, which gets trigger flag `0x200` (`"Linked Triggers loop"`,
`"Trigger id %d, no next %d"`). A chained-to trigger can't be touched
itself; it's set by the one before it.

## Movers: the nodes triggers move

### Registration (`FUN_80065e14`)

`FUN_80065e14(node, kind, off, on, sound)` adds the target node to the
list `DAT_8025e3f0` (count `r13-0x71cc`, at most 0x95: `"TOO MANY ITEM
WOBJS"`) with parallel float arrays:

| array | meaning |
| --- | --- |
| `DAT_80257428` | base Y: the node instance's matrix Y (`+0x34`) at registration |
| `DAT_802571d0` | current offset (starts at the off height) |
| `DAT_80257680` | off height (0.1 × placement `+0x38`) |
| `DAT_802578d8` | on height (0.1 × placement `+0x3A`) |
| `DAT_80257b30` | sound parameter (placement `+0x35`) |

A node already in the list keeps its entry: heights and sound are only
filled if still unset; two triggers with different kinds log `"ITEM WOBJ
%s TRIGGER TYPES DIFFER"`, except lift kinds 0x1B–0x1D, which merge to
0x1B. A node without an instance logs `"TARGET WOBJ %s HAS NO NODE"`.

Placement instances hang under their parent's (`FUN_800aafb0`): instance
`+0x30` is the node's *local* translation, and only static nodes have
their `+0x1C` turned into a world position (`"World obj with dynamic
parent"`). Moving the target's instance moves every descendant with it.

### Update (`FUN_800629ec`, end of the item update `FUN_800606e8`)

Per mover each frame, with the node's state byte `+0x16`: bits 0–3 = the
players holding it, `0x10` = moving, `0x20` = on; `uVar4` = the kind flags
(node `+0x14` >> 8):

- **Particle nodes** (node flag `0x800`): switched on/off by state `0x20`
  (`FUN_800aae2c` / `FUN_800aadd4`).
- **Bridges** (kind flag `0x10`): they don't move, they **fade**. Hidden
  while inactive — `(state & 0xF) == 0`, or with kind flag `0x20`,
  `(state & 0x2F) == 0` — when the node's collision disable byte `+0x35`
  = 0xFF (every player and monster query skips it) and its instance fades
  out by 8 alpha steps per field (`FUN_800ba9b0`, instance `+0x53`, flag
  `0x200` translucent, `2` hidden); active, it fades in and collides.
- **Node flag `0x2000000`**: a separate mode toggling node flags
  `0x100000`/`0x200000` (ends `0x400000`/`0x800000`). No trigger target on
  the disc uses it.
- **Everything else moves along Y**: toward the on height with state
  `0x20`, else the off height, at most **4 units/s** (`r2-0x6650` × frame
  time); within ±0.001 (`r2-0x6648`/`-0x6640`) it has arrived, and kind
  flag `0x20` then flips state `0x20` (it goes back). Instance Y = base +
  offset. Node flag `0x8000000` while moving.
  - **Without kind flag 8**, it only moves while no player stands on it
    (`FUN_80063840(node, 1)`: 2 = a live player's standing node `+0x8C4`
    is this node, 1 = a monster's `+0x298` is, 0 = none).
- While moving (or fading): state `|= 0x10`, node flag `0x20000000`, and
  without kind flag 8 the disable byte `+0x35` = 1 (the player's queries,
  mask 1, skip it). The byte is reset to 0 at the start of each update.
- Then, unless the kind flags have any of `0x47`, the state is cleared —
  kept only for kind `0x10` bridges while someone stands on the node
  (`FUN_80063840` ≠ 0). So pads have to be held; switches (2), lifts
  (4) and elevators keep their state.

Sounds: bridges play `DAT_801233fc` (appear) / `DAT_801233c4` (vanish) by
realm (`FUN_8009c938` / `FUN_8009c9a4`) when state `0x20` changes. Other
movers use the sound parameter: below 10 a start/stop loop pair from the
runtime table `DAT_8028aff0` (`FUN_8009cecc`, filled per level — not
traced); 11 on arrival; above 10 a one-shot from
`0x80123354 + (v − 10) × 0x38 + realm × 4` (`FUN_8009ca10`). On the disc
nearly all parameters are −1..4.

## Trigger touch and update

### Touch (`FUN_8005d71c` case 5)

The player update's item query runs it for each touched trigger:

- Trigger flag `0x100`: counts only if the player stands on the target
  node or one of its direct children (walks `+0x2E` first child /
  `+0x2C` next sibling from the target) — lift pads ride on the lift.
- Flag `0x400` (all players needed) with more than one player: hints
  `0x7E` / `0x7F`.
- Sets the player's bit in `+0xCE` on this trigger and every trigger down
  its chain (`+0xE4`), or just this one with flag `0x40`.

### Hit switches

`FUN_8005c1c8` (below) on a trigger of subtype 0x1F: `+0xCE |= 0xF` down
the chain — all players at once.

### Update (`FUN_800606e8` case 5)

Only for triggers on screen (item flag `0x4000`) or flagged `0x40`, and not
item flag `0x400`. `m` = `+0xCE` (with flag `0x100` masked to players
still standing on the target or a child of it, `+0x8C4` / its parent
`+0x18`); `node` = the target:

- Timer `+0xEC` counts down by fields.
- Flag `0x40` (quest triggers): id < 100 → `FUN_800a1928` /
  `FUN_800a200c`, id ≥ 101 → `FUN_800a1728` / `FUN_800a1f88` (quest
  state; not traced), then `+0xCE` copied down the chain.
- Flag `0x400`: unless `m` equals every player present (`r13-0x7318`),
  `m` = 0 and node flag `0x4000000` cleared (else set).
- No node: action `+0xCA` = 2 when `m` (with flag 4: 2 or 0 by `m`).
- **Flag 1 (off switch, LIFTSW)**: pressed → if the node is on and still
  (`state & 0xF0 == 0x20`) it's turned off (`state &= 0xF`); action shows
  the inverse of on.
- **Flag 2 (switch)**: pressed → if `state & 0xF0 == 0` the node is
  turned on (`state = state & 0xF | 0x20`); a one-shot latch. DOORSW
  (0x17) shows hint 5 unless `r13-0x7380 & 0x8000`.
- **Flag 4 (lifts, elevators)**: pressed → players added to the state;
  when the node isn't moving and the timer has run out, state `0x20`
  flips and the timer = 120 fields (while it moves the timer is held at
  120) — a held lift pad sends it up, waits 2 s, sends it down… Released:
  action 0, and flag `0x800` sets the timer to (players − 1) × 60 fields
  (`r13-0x7394` players).
- **Plain pad** (none of 1/2/4): pressed → state = `m | 0x20`; the mover
  clears it again each frame unless its kind keeps it, so a DOORPAD door
  is open while the pad is held.
- Action otherwise mirrors state `0x20` (2 on, 0 off).
- **On a new activation** (action becomes non-zero):
  - flag `0x1000`: `FUN_800277ec(10.0, 0, 0, 0xB4, 100)` — probably a
    camera shake (not traced);
  - flag `0x2000`: wakes the nearest placed monster (`FUN_80062fdc` for
    class 4 within `r2-0x67a8`): its `+0xE4 |= 1`;
  - `+0xEE` ≥ 0 and `m`: camera cut to the linked transmitter
    (`FUN_8001be98`; locator kind 9, [level-population.md](level-population.md)).
- Finally, unless `r13-0x774c` or flags `0xC0`, the touches `+0xCE` are
  cleared for the next frame.

The pad's own animation (item update's first loop): its atree has `OFF`,
`ONA`, `ON`, `OFFA`; going from state 0 to action 2 it plays 1, from 2 to
0 it plays 3, mode 1; the state follows when the switch reports.

`FUN_8000eb70` (behind `FUN_80011104`) returns bit 1 when it starts the
requested action, 2 when the previous one had ended, 4 when the requested
one is playing and has ended — the item code's `(r & 3) != 0` is "just
started".

## Rotators (item class 12)

Constructor: `+0xDC` = target node, `+0xE0` = placement `+0x34` read as a
**float angle per field** (e.g. 0.0087 rad), `+0xE4` = placement `+0x38`
(limit), `+0xE8` = 0 (accumulated). Update (`FUN_800606e8` case 0xC):

- **Subtype 0**: turns every frame by angle × fields (unless
  `r13-0x7600` and the node is a floor, flag 4): the node's flag 1
  (translation-only collision) is cleared and its instance turned about
  its own Y axis by `FUN_800be7e8` (X axis toward Z for a positive angle).
- **Subtype 2**: once touched (touch handler case 0xC sets `+0xC4 |= 1`),
  turns until the accumulated angle reaches ±limit, then sets `0x1000`.
  Sounds `FUN_8009d01c`: start/stop by realm, `0x801232e4` /
  `0x8012331c` — `S_ROCKROTATE`/`S_ROCKSTOP` (mountain),
  `S_METLROTATE`/`S_METLROTATESTO` (ice).
- **Subtype 1**: `FUN_8002c450` / `FUN_8002c4f0` depending on whether a
  monster stands on it (with more than one player) — not traced.

Only 8 rotators exist on the disc.

## Moving collision and carrying the hero

`FUN_8000dfec` tests a moving node's triangles in its frame: node flag
`0x1000` → the instance's translation (`+0x30`), inverse matrix unless
node flag 1; `0x1000000` → the full world matrix from `FUN_800bb904`
(walks the parent instances, `+0x74`). So moving a node's instance moves
its collision; see [collision.md](collision.md).

**Carrying**: after the player's floor check, `FUN_8008764c` →
`FUN_800877c0`: standing on a node with an instance and flag `0x1000`,
the player's instance (`+0x74`) is **re-parented under the node's
instance** (`FUN_800bb084`), so the platform carries the hero as it moves
or turns; on anything else it goes back under the world root
(`r13-0x6fcc`). `FUN_800781f4` refuses (and zeroes the move's X/Z) when
another player stands on a different `0x1000` node flagged `0x4000`.

## Damage tiles (item class 8)

No TRAP (class 6) placements exist on the disc: every hazard is a damage
tile. Types (item type records):

| name | subtype | shape | extents | value (`+0x3C`, kind flags) | `+0x40` damage | `+0x48` |
| --- | --- | --- | --- | --- | --- | --- |
| SPIKES | 0 | box | 3.5, 5, 2.0 × 0.4 | 0x2000 | 20 | −40 |
| FLAMEV | 1 | cylinder | 0.75, 5 | 1 | 20 | −40 |
| FLAMEH | 1 | box | 4.25, 3, 0.6 × 3.0 | 1 | 20 | −40 |
| FORCEF, FORCEF_S | 2 | box | 4.5 / 3.5, 5 | 2 | 20 | −40 |
| SAWBLADE | 3 | box | 3.5, 5, 2.0 × 0.1 | 0x2000 | 20 | −40 |
| TENTACLE | 4 | box | 9, 5, 4 × 1 | 0x20 | 40 | −100 |
| TENTWALL | 5 | box | 6, 5, 1 × 2 | 0x20 | 20 | −100 |

Type flags `0x5`: used (1) and cycling (4). Their atrees have `OFF`, `ONA`
(rising), `ON`, `ONB` (going away).

**Construction** (`FUN_800646e4` case 8): damage `+0xDC` = placement
`+0x30` (i16) or else the type's `+0x40`, × level record `+0xDC`;
placement `+0x32` ≠ 0 overwrites the type's `+0x48` with it × −3; hit
points × level `+0xCC`; the timer starts at `+0x48` × 2 fields × level
`+0xD8`.

**Cycle** (item update, first loop, flags 1 and 4): when the timer runs
out the action steps on (wrapping at the action count `+0x7C`; flag 2
would clamp). As each action **starts**, the state `+0xC8` becomes it and
the timer is set: action 0 → type `+0x48` × 2 fields × level `+0xD8`;
others → type `+0x4A` × 2 (0 for every tile), which falls back to a value
from the animation (`0.5 + +0x80 × +0x9C`, × 2 — probably the action's
length in fields; not confirmed). A negative time is random:
`n = −2v`, `n/2 + random(n)`, i.e. |v|..3|v| fields. Tiles with more than
4 actions and a negative time use 0.

**Hurting** (`FUN_80086e44` skips tiles unless state is 2 or 4 **and**
flag 1 is set; then `FUN_8005d71c` case 8), unless the hero levitates
(`+0x124 & 1`) or its hazard cooldown `+0x8E8` is in the future (game
seconds `r13-0x756c`):

- `FUN_80078560(damage, player, kind, flags, push)`: kind 3 for
  subtypes 0, 3, 4, else 2; flags = type value `| 0x80`; with value
  `& 0x30` a push of −1 (`r2-0x6770`, double) × the item's Z axis (so
  tentacles knock the hero down).
- Sound `FUN_8009e8a4(pos, subtype)`: table `0x80122FF4 + realm × 0x1C +
  subtype × 4` — castle `S_SPIKEA`, `S_FFIELDZAPA` (2), `S_BUZZSAW` (3),
  `S_TENTACLES` (4, 5); mountain `S_FIREHOLE` (1); desert `S_SPIKEC`,
  `S_FIREHOLEC`, `S_FFIELDZAPC`, `S_SPIKEGATE` (6); forest `S_LOGSPIKE`,
  `S_FIREHOLED`, `S_SWINGBLADE`, `S_TENTACLESD`; hell, town, battle,
  ice, dream, sky `S_FIREHOLEF/G/H/I/J/K` (1).
- Cooldown `+0x8E8` = now + (timer + 1) × 1/30 s (`r2-0x6760`, double):
  once per active phase.
- Hint `0x15`.

Also: with `r13-0x731c` set, tiles are reset to off (state and action 0,
timer 30); a tentacle in state 1 plays a sound when a player is near
(`FUN_8009d1dc`).

## Damaging walls

`FUN_80085b14(player, node, …)`, called when the player's walls test hits
(`FUN_80087f4c`): with the node's flags OR'ed up its parents
(`FUN_800aacac`):

- nothing unless `& 0xF0000`; nodes with `0x2000000` only while moving
  (`0x8000000`); not while the player's `+0x204` > 0x1E; not before the
  cooldown `+0x8EC`;
- `0x30000`, `0x40000`, `0x50000`: 15 damage (`r2-0x5a80`), kind 1, flags
  `0x20` (knockdown), push along the X/Z line from the hit to the player,
  sound `FUN_8009c088`;
- `0x20000`: 10 damage (`r2-0x5b24`), flags `0x10` (knockback), push;
- any other: 5 damage (`r2-0x5b44`), flags 0;
- cooldown 1 s (`r2-0x5be0`, double).

On the disc these are mostly moving nodes: minecarts (ice), spinners
(hell), lava balls (mountain), crows (desert), rolling rocks. Static
floors with `0x10000` (town, battle, ice) are only reached if the walls
test hits them.

## Breakables

### The hit search (`FUN_8005b260`)

Items the hero's blows can find: live, not picked up (`0x8100`), armour
byte `+0xCF` (type `+0x42`) ≠ −1, in the game, on screen; class 3
generators; class 2 containers of subtype 0x2B–0x2D; class 10 obstacles
except 0x29; class 5 triggers of subtype 0x1F; barrels (0x2B–0x2D) only
before they break (state < 1). Within 2 (`r2-0x6850`) × the type's
`+0x10` vertically; distance |v| × 0.9 (`r2-0x6848`) for 0x2C/0x2D, × 1.2
(`r2-0x6840`) for other non-generators, − min(radius, 5)
(`r2-0x6838`); [combat.md](combat.md) has the cone.

### A hit (`FUN_8008615c` → `FUN_8005c1c8(damage, item, kind, player)`)

Generators first get the level-versus-player scale. Then, for blows
without kind `0x800`: damage − armour `+0xCF` (at least 1) is taken off
the hit points `+0xD0` (type `+0x44`); 0 is "dead". A `0x800` blow does
no damage (−2 when above 2.0, `r2-0x67f0`).

- **Containers** (class 2): dead with type flag `0x200` (barrels `0x206`)
  and not yet used → used, contents released (`FUN_8005e8f8`); a barrel
  (0x2B) plays the realm's break sound (`0x801231cc`: `S_BARREL_WOODA/B/C/
  D/G/H/I/J/K`) and hint `0x1B`. Blows with kind `0x400` of 5 or more
  (`r2-0x67d8`) blow chests apart (`CHESTSEXP0` / `CHESTGEXP0` effects).
- **Obstacles** (class 10), keyed on the **type's** subtype (not the
  placement's override): not dead → hit flash (`+0xE0` = 1); then
  - 0x2B BARREL and the rest: not dead → `S_WEAPONHITWOOD` (`0x3C`); dead →
    used, break sound; stays (broken);
  - 0x29 SAFEROCK: `FUN_80063d9c` (breaks into its count of pieces);
  - 0x2A WALL (shootable walls, shape 4): `FUN_8009c128` — the level's
    hit sound, or `S_SECRETWALL` at 0 — and freed at 0 hit points;
  - 0x2C EXP BARREL: dead → used, explosion `FUN_80092bf4(30 × level
    +0xDC, pos, 0x18)` (`r2-0x67b8`), `S_BARREL_EXPLOA/B/C/D`
    (`0x8012323c`);
  - 0x2D POI BARREL: dead → used, poison cloud `FUN_80092bf4(10 × level
    +0xDC, pos, 0x19)` (`r2-0x67a8`), `S_BARREL_GASA/B`… (`0x80123204`).
  Every BAREXP and BARPOI placement on the disc overrides its subtype to
  0x2B, but these paths read the type's, so they still explode or gas.
- **Triggers** 0x1F: switched (above).
- The hit also shows `FUN_8002f400` (effect) and hint `0x14` for
  obstacles.

A barrel broken: the used flag steps its atree (`IDLE`, `ACTIVE` 24
frames, `DONE`) through; at state 2 it is walked through (touch test).

### Releasing contents (`FUN_8005e8f8`)

A random contents type is resolved like a placement's. CHEST GOLD (0x2F)
and SILVER (0x30, one-choice) keep the amount as their own gold; CHESTEXP
(0x2C) sets `0x40` and plays `S_TICKY` (`0x2D`); otherwise a new item of
the contents type is made at the container (`FUN_800642b4`,
`FUN_800646e4`), dropped to the floor (`FUN_80064140`), with a 30-field
pickup delay and keys taking the container's placement `+0x34` count; a
monster (class 4) inside is woken (`+0xE4 |= 1`). A chest with a mount
node (`+0xE4`) holds the item inside, grown from nothing as it opens, and
the chest is taken with it.

Containers on the disc hold keys, potions, food (some poisoned), treasure,
powerups, scrolls — and in 84 cases a Death (class 4, `DEATH`).

### Explosions

`FUN_80092bf4` makes a blast through the projectile/effect records
(`0x802855a8`, [projectiles.md](projectiles.md)): damage `+0x644`, flags
`+0x64C` (`0x421` for explosions, `0x800` for the poison cloud), radius
`+0x650`: 6 (`r2-0x566c`) for an exploding barrel, 6.5 (`r2-0x563c`)
poison, 12 (`r2-0x5658`) for type `0x1D`. The damage reaches players
through `FUN_80094418` → `FUN_80078560` with a falloff not traced here.

**CHESTEXP** (container 0x2C, locked): opened with a key it ticks while
its `ACTIVE` action (61 frames) plays; at state 2 (`FUN_800606e8` case 2)
it explodes, `FUN_80092bf4(50 × level +0xDC, pos, 0x1D)` (`r2-0x6728`),
with hint `0x89`, and is freed. Its listed contents (`TIMEBOMB`) aren't
released.

## Level record `+0xD8`, `+0xDC`

Two more item scales in the level record ([monsters.md](monsters.md)):

| offset | use | values |
| --- | --- | --- |
| `+0xD8` | × damage tiles' off time | 0.5–1.25 (A1 0.8) |
| `+0xDC` | × hazard damage: damage tiles, explosions | 0.5–1.0 (A1 0.65) |

The forest realm and some secret/test levels have 0 here as in the other
scales.

## In this rewrite

Built so far:

- `NodePose` and `LevelCollision::{set_pose, pose, poses}`
  ([`collision.rs`](../crates/gdl-formats/src/collision.rs)): each node's
  rigid move from where the file puts it; queries against a moved moving
  node are turned into its rest frame and the hit turned back out. Tested
  with a lifted floor and a turned wall.
- `LevelData::{nodes, placement_nodes}`
  ([`level.rs`](../crates/gdl-game/src/level.rs)): the scene graph and
  each model placement's node.
- `LevelItems` API ([`items.rs`](../crates/gdl-game/src/items.rs)):
  `ItemView`, `view`, `views`, `realm`, `play`, `set_state`, `set_flags`,
  `free`, `release` (new items from `RELEASED_BASE`, amount override,
  pickup delay; no model yet); `USED` is public; used obstacles step
  through their actions like opened chests.

Still to build:

- `mechanics.rs`: triggers (touch, chains, update), movers, rotators,
  node world poses (own move, then the parent's), carrying the hero (apply
  the standing node's change of pose to the hero's feet, floor and
  facing).
- Drawing moving nodes: `world.rs` splits instances of trigger/rotator
  targets and their descendants out of the merged meshes into entities
  posed each frame. Bridge fading as show/hide (a stand-in for the alpha
  fade).
- `hazards.rs`: damage tiles, damaging walls, a stand-in one-off blast for
  exploding barrels and CHESTEXP until the effect system is ported.
- `breakables.rs`: a `Targetable` (kind `Breakable`) entity per hittable
  item, hits applied as above, contents released through
  `LevelItems::release`, with models pre-built for container contents by
  `population.rs`.
- Not planned yet: quest triggers (flag 0x40), camera shake and cuts,
  mover sound loops (`DAT_8028aff0`), obstacle falls (0x28, 0x31, 0x33–0x35),
  safe-rock pieces, node flag `0x2000000` animation.
