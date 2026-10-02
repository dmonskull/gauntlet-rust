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
`FUN_8005ff4c(id, snap)` fires every trigger with an id (and its chain)
from code — the tower's gates as it loads, snapped open, animated ones at
their last frame (`docs/items.md`, "Quest items and the tower's gates";
`Mechanics::fire`).

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

`FUN_8006437c` (the end of the item set-up, `FUN_80063fb0`, after the
drop) checks trigger ids are unique (`"%d triggers with id = %d"`,
`"%d special triggers ..."`) and links each trigger with a `next` id to
the trigger whose id matches (not flag 0x40): item `+0xE4` = that item,
which gets trigger flag `0x200` (`"Linked Triggers loop"`, `"Trigger id
%d, no next %d"`). A chained-to trigger can't be touched itself; it's set
by the one before it. The id byte `+0xE2` is read as a **signed** byte
(`lbz` + `extsb`): the uniqueness check takes only ids 1–127 and, for
each trigger in item order, clears the id (0) of every other trigger with
the same id and the same flag `0x40` — so the first one keeps it and no
chain, camera point or firing by id (`FUN_8005ff4c`) reaches the others
(C2's 441 shares 103 with 438, C3's 426 shares 1 with 417); ids 128–255
are never checked. A chain goes to the first trigger (other than itself)
with the id that isn't a quest gate: on the disc D3's 362, I2's 477, K4's
438 and the tower's 81, 149 and 150 (to the quest gate 104) chain to
nothing.

**Camera points**: the locator set-up (`FUN_80066258`, after the items)
links each transmitter of kind 9 through `FUN_80066c7c(slot, index)`: in
the tower (realm 13) indices 198, 170–183 and above 200 are the tower's
own cameras (slots for its scenes; `0xF0`, `0xDC`, `0xAA` are the
wizard's lookouts) and go no further; otherwise every trigger whose id,
as a signed byte, equals the index gets `+0xEE` = the point (`"Two
cameras link to trigger"` when it had one). An id of 128 and up never
matches, so the tower's pad 145 (id 240, at the wizard's pedestal) has no
cut, though the tower has a camera 240. `mechanics.rs` clears the shared
ids and links the points the same way (`clear_shared_ids`,
`camera_point`); it had linked every trigger whose id matched, cutting to
camera 240 each time the hero stepped onto the pedestal, and cutting on
C3's 426 too.

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
- **Node flag `0x2000000`** (the animated mode, below): no heights —
  without kind flag `0x20`, state `0x20` sets node flag `0x200000` (play
  on to the last frame) and clears `0x100000`, else the reverse (play back
  to the first); it has arrived — not moving — with `0x100000` and
  `0x400000` (at the first frame) or `0x200000` and `0x800000` (at the
  last). With kind flag `0x20`: while no player holds it, once at the
  last frame (`0x800000`) it clears state `0x30` and stops (`0x300000`);
  while held it sets state `0x30` and clears both (round and round).
  Stood on (without kind flag 8, `FUN_80063840` = 2) it stops where it
  is (`0x300000`) and clears state `0x10`, skipping the rest.
- **Everything else moves along Y**: toward the on height with state
  `0x20`, else the off height, at most **4 units/s** (`r2-0x6650` × frame
  time); within ±0.001 (`r2-0x6648`/`-0x6640`) it has arrived, and kind
  flag `0x20` then flips state `0x20` (it goes back). Instance Y = base +
  offset. Node flag `0x8000000` while moving.
  - **Without kind flag 8**, it only moves while no player stands on it
    (`FUN_80063840(node, 1)`: 2 = a live player's standing node `+0x8C4`
    is this node, 1 = a monster's `+0x298` is, 0 = none). It's the node
    itself, not a child: a platform whose floor is a child node (G1's
    swinging arm and lift) carries the hero whatever its flags. The same
    hold applies in the animated mode (both play flags set, not moving).
    ACTIVESW-style triggers (subtype `0x18`) always add flag 8, as do
    DOORPAD, DOORSW, LIFTPAD, LIFTSW and LIFTEND; ELEVPAD (`0x19`:
    `0x804`) and ELEVSW (`0x1A`: `0x02`) don't (nor the bridges', which
    don't move).
    `level_audit --ridden` lists every mover a ridden pad drives: on the
    disc only C2's six elevator switches (pads 40, 42, 50, 76, 89, 101)
    drive a node without flag 8 that the hero stands on itself — they move
    once the hero steps off — and two lift pads share their spots with an
    off switch on the same node (A4 256 with 259, J3 483 with 495): the
    triggers update in item order before the movers, so where both are
    touched the pad turns it on and the switch off again in the same
    update, and it stays put.
- While moving (or fading): state `|= 0x10`, node flag `0x20000000`, and
  without kind flag 8 the disable byte `+0x35` = 1 (the player's queries,
  mask 1, skip it). The byte is reset to 0 at the start of each update.
- Then, unless the kind flags have any of `0x47`, the state is cleared —
  kept only for kind `0x10` bridges while someone stands on the node
  (`FUN_80063840` ≠ 0). So pads have to be held; switches (2), lifts
  (4) and elevators keep their state.

Sounds (skipped while the sound byte is negative): bridges of kinds 0x14 /
0x16 play `DAT_801233fc` = `S_BRIDCL<realm>` when state `0x20` clears and
`DAT_801233c4` = `S_BRIDOP<realm>` when it sets (`FUN_8009c938` /
`FUN_8009c9a4`; realms A C D H I). Other movers use the sound byte:

- below 10: a loop while moving and a stop sound when it stops
  (`FUN_8009cecc(0 / 2, pos, v)`), one loop at a time for all movers
  (`FUN_8009cecc(−1)` when none moves). `FUN_800a0e44` fills the pairs
  per level from the sets at `0x80122aa4`: `MET ROPE CHAIN ICE STONE
  *ROCK` → `S_ELV<set><realm letter>` / `S_ELV<set>STP<letter>` (`…B` on
  boss levels), `*ROCK` → `S_ROCKROTATE` / `S_ROCKSTOP`;
- 11: a one-shot on turning on (with node flags `0xC00000`);
- above 10: a one-shot from row v − 10 of `0x80123354` (`0x38` per row,
  realm × 4; `FUN_8009ca10`) when it starts or stops: row 1 `S_TRAP<l>`,
  row 2 `S_QUAKEC` / `S_ELVCNNK`, rows 3/4 `S_BRIDOP<l>` / `S_BRIDCL<l>`.

On the disc nearly all parameters are −1..4. `mechanics.rs` plays all of
these (the node-flag check for 11 is left out) where the game does, at
0xE0 ([audio-format.md](audio-format.md), "Positional sounds"): the
one-shots panned at the mover's node as it is that tick
(`audio::PlaySoundAt`), the loop following the first mover moving
(`audio::LoopSoundAt`), the rotators' grinding following the last one
grinding on; a mover's stop sound only if its set's loop was the one
playing (the one asked for the tick before), as the game's `FUN_800163c4`
test has it.

## Animated objects

The world file's table ([worlds-format.md](worlds-format.md), "Animated
objects") lists nodes the level animates itself, one track each. The
loader flags each one's node — and, through `FUN_800aafb0`, every node
under it — `0x2000000`; the item set-up flags every trigger's target
`0x100000` as it registers the mover. Each frame, before the items
(`FUN_80056748` → `FUN_80055d08`, unless a camera cut has 11 to 99,999
fields of its hold left: the 30-field delay and most of a trigger's cut
pass with everything still, then the object plays while the camera
watches), `FUN_800a7ff8` runs each object by its node's flags:

| `0x100000` | `0x200000` | plays |
| --- | --- | --- |
| set | set | not at all: held (node flag `0x8000000` cleared) |
| set | — | back to frame 0, then holds |
| — | set | on to the last frame, then holds |
| — | — | forward round and round |

Playing, it poses the node at its frame (`FUN_8000f72c`, the characters'
track sampler): the angles become the instance's rotation (`FUN_800bd448`,
or `FUN_800bd548` with flag `0x8000`) when the track has any, the
translation is added to the node's own (`+0x1C`) when it has any, a scale
is set (instance `+0x40`, flag 8) when it has any. Then — unless time is
stopped (`r13-0x731C`) and it goes round — node flags `0x400000`/
`0x800000` clear, `0x8000000` sets and the frame moves 30 a second
(`r2-0x5040` × the frame time): on, until its whole part reaches the last
frame (that frame held, `0x8000000` cleared, or round to 0), then
`0x800000`; back, until it goes below 0 (0 held), then `0x400000`. So
every trigger's target starts at its first frame and holds there (and
non-targets go round), a switch plays it on, and its cut — which holds
while the node has `0x8000000` — ends when it gets there. Going round,
a node of type `0x50000` (`flags & 0x100F0000`: H1's fire, the I realm's
minecarts, all top-level nodes) bursts where it is (`FUN_80055e04` with the
node's world position): `FUN_80055e60` makes an effect — in the ice and
sky realms (9, 11) the realm items' `WORLD_EXP` (effect `0x5E`, looked up
by name in the realm's and the level's item banks when the effects load, `FUN_800972dc`, into `0x80284E60`; EXPLOSION when it isn't there),
else EXPLOSION (`0x16`) — through `FUN_800933f8`, gives it a blast
(`FUN_80093768`: damage 50, radius 6 and kind `0x800` in realms 9/11, 5
and `0x21` elsewhere, no owner) and scale (`FUN_800940a0`: 1 × 1 × 1 in
9/11, 1.5 × 1 × 1.5 elsewhere), and sounds the realm's burst
(`0x8012386C`: only the ice realm's `S_MINECAREXPLO`, at `0x7F`, faded);
then it and its parents hide (instance flag 2, node flag `0x10000000`)
until the next lap's frame is below 2.

On A2 the triggers' objects are its drawbridges — `A2SUPPORTA`–`D`, each
carrying a floor (`A2FLOOR#n`, moving collision), stand raised 70–80° at
their first frame and swing down into place — the plank by the start
(`A2PLANKPIECEA`, which falls 18 units), the diving board
(`A2DIVEBOARD`, sliding 8.4 back) and `A2NSXLSHOWER` (dropping 32);
B2's snakes rise and sink. Most of the rest go round: B3's rolling and
falling rocks, B5's lava balls, C1's fish.

## Trigger touch and update

### Touch (`FUN_8005d71c` case 5)

The player update's item query runs it for each touched trigger:

- Trigger flag `0x100`: counts only if the player stands on the target
  node or one of its direct children (walks `+0x2E` first child /
  `+0x2C` next sibling from the target) — lift pads ride on the lift.
  The node the player stands on (`+0x8C4`) is set by `FUN_8008764c` only
  when the floor check finds a floor (`FUN_800878a0` returns above 0); the
  floor check clears it only when it finds no floor and the player isn't
  moving across (under 0.001), so a hero walking off a lift still counts
  as on it until it lands elsewhere or drops straight down (the rewrite's
  `LevelCollision::player_floor` does the same).
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
    class 4 within `r2-0x67a8`): its `+0xE4 |= 1`. Here `tick` lists the
    trigger in `Mechanics::woken` and `critters.rs` wakes the statue
    ([critters.md](critters.md));
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
without kind `0x800`: an item whose armour `+0xCF` isn't −1 has damage −
armour (at least 1) taken off its hit points `+0xD0` (type `+0x44`); 0 is
"dead". A `0x800` blow does no damage (−2 when above 2.0, `r2-0x67f0`).

- **Containers** (class 2): dead with type flag `0x200` (barrels `0x206`)
  and not yet used → used, contents released (`FUN_8005e8f8`); a barrel
  (0x2B) plays the realm's break sound (`0x801231cc`: `S_BARREL_WOODA/B/C/
  D/G/H/I/J/K`) and hint `0x1B`. Otherwise explosions blow chests apart
  (below, "Blows on items").
- **Obstacles** (class 10), keyed on the **type's** subtype (not the
  placement's override): damaged, not dead and not 0x29 → hit flash
  (`+0xE0` = 1: the next item update draws the root object in the
  level's `AAAWHITE` and the model without its lightmap, once —
  [rendering.md](rendering.md), "Texture overrides"); then
  - 0x2B BARREL and the rest: not dead → `S_WEAPONHITWOOD` (`0x3C`); dead →
    used, break sound; stays (broken);
  - 0x29 SAFEROCK: `FUN_80063d9c` steps its model down by its hit points
    (`docs/critters.md`, "Safe rocks");
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
- A hero's melee blow (`FUN_8008615c`, from the player update; missiles
  call `FUN_8005c1c8` directly) that did damage (the routine returns the
  hit points left, ≥ 0) also shows `FUN_8002f400` (effect) and, on an
  obstacle whose type has no name (type `+0x28` = 0: the secret walls),
  hint `0x14`.

A barrel broken: the used flag steps its atree (`IDLE`, `ACTIVE` 24
frames, `DONE`) through; at state 2 it is walked through (touch test).

### Blows on items (`FUN_8005c1c8`, classes 1 and 2)

What reaches a powerup or a chest:

- **The hero's blows** don't: the hit search (above) and the attack
  search (`FUN_800864b0`, which keeps an item only with armour `+0xCF`
  ≥ 0) find neither.
- **Missiles** (flag 2, `FUN_8005ed30` with the filter `FUN_8005ee04`)
  stop at containers, potions (not being taken or held in a chest),
  doors, generators with strength, placed monsters, hit switches, damage
  tiles of subtype 5 in state 1–2 and most obstacles; the effect item
  test (below) then decides the damage, and a missile of kind `0x800`
  does none to items.
- **Blasts** (area effects with flag 2, `FUN_80094418`): every item within
  their reach + the item type's `+0x0C` that the effect item test lets
  through, then not again until its cooldown `+0xD8`.

The effect item test `FUN_8009682c(item, effect flags, kind)` gives 0
(hit), 1 (passed by) or 2 (no damage): a powerup is passed by effects
with flag `0x100` (gold and food get 2 from those with kind `0x800000`);
a barrel container (0x2B) by effects with `0x1000`; and a powerup or
container of armour −1 by any kind without `0x200` (magic) or `0x400`
(explosion). A hero above level 24's magic (kind `0x800000`) also calls
`FUN_8005ba08` on the items it reaches (not traced). On the disc
(every level's types) chests — CHESTEXP (0x2C), CHEST (0x2E), CHESTG0–5
(0x2F), CHESTS (0x30) — treasure, keys, timed powerups and the quest
pieces have armour −1; food −2 (1 hit point for fruit, 2 for meat), so
any blast reaches it; potions 0; the barrel container BAROBJ armour 1
and 5 hit points.

Then `FUN_8005c1c8`, by class; "explosive" is kind `0x400` with a blow
(after armour) of 5 or more (`r2-0x67d8`):

- **Powerups** (class 1), by subtype:
  - 4 potion: at exactly 0 hit points it goes off ([effects.md](
    effects.md));
  - 2 key, 10–16 (runestones, the boss's key, the obelisk, legendary
    items, scrolls, gems, gargoyle pieces): nothing;
  - 1 treasure, explosive: effects `0x20` (CHESTDEST) and `0x21`
    (DESTSMOKE) at it (`FUN_80094120`), its animation becomes
    `TREAS_JUNK` (`POWERUPS`) and it's worth 10 (`+0xE0`);
  - 3 food, a `0x800` blow above 2: spoiled — with 2 hit points (meat)
    its animation becomes `BADMEAT` (`r2-0x67e8`), worth −100, else
    `GAPPLE` (`r2-0x67e0`), worth −50 — and hint `0x88` GASPOISON
    ("POISON GAS SPOILS FOOD", `S_GASFOODBAD`);
  - the rest (food, timed powerups, TIMEBOMB), explosive: effects `0x20`
    and `0x21`, the model `ITEMEXP0` (the realm's items bank) put in its
    place (`FUN_800b85c0(name, item model, its parent, 0x80800)`), the
    item freed, hint `0x87` EXPDESTROY ("EXPLOSIONS DESTROY ITEMS",
    `S_EXPDSTITMS`).
- **Containers** (class 2):
  - magic (`0x200`) on a container other than 0x2B with a Death inside
    (`FUN_80051f44` of the contents' name = `0x1E`): the Death is killed —
    the contents become an APPLE (`r2-0x6828`), `FUN_800a03a8` at it, and
    `+0xDE` = 3 × the blow (`r2-0x67d4`) makes the chest shake while
    closed (`FUN_800606e8` case 2); it also writes subtype 1 into the
    container's **type**;
  - otherwise, unless its hit points just ran out and it breaks open,
    explosive: a CHESTEXP (0x2C) is set off — `+0xC8` and `+0xCA` = 2,
    used, released (`S_TICKY`, flag `0x40`), so it explodes on its next
    update (below); any other container lets out a Death it holds
    (`FUN_8005e8f8`), plays effects `0x1F` (CHESTDEST) and `0x21`
    (DESTSMOKE), leaves `CHESTSEXP0` (the silver chest, 0x30) or
    `CHESTGEXP0` in its place and is freed: anything else inside is
    lost, and there is no hint.

Nothing in these paths frees the left models (`ITEMEXP0`, the chests').
The hints are shown once (mode 3, priority 50). A hit on an item whose
armour isn't −1 also plays its hit effect (`FUN_80093c78`).

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
`+0x650`: 12 (`r2-0x5658`) for an exploding barrel (effect `0x18`) and
CHESTEXP (`0x1D`), 6.5 (`r2-0x563c`) poison (6, `r2-0x566c`, only for
other ids). The damage reaches players
through `FUN_80094418` → `FUN_80078560` with a falloff not traced here.

**CHESTEXP** (container 0x2C, locked): opened with a key (or set off
by an explosion, above) it ticks while its `ACTIVE` action (61 frames)
plays; at state 2 (`FUN_800606e8` case 2) it explodes,
`FUN_80092bf4(50 × level +0xDC, pos, 0x1D)` (`r2-0x6728`), with the
realm's barrel explosion sound (`FUN_8009d210`: `S_BARREL_EXPLO<letter>`)
and hint `0x89` CHESTSEXPL ("SOME CHESTS MAY EXPLODE WHEN OPENED",
`S_CHESTSEXPL`), and is freed. Its listed contents (`TIMEBOMB`) aren't
released. Effect `0x1D` is EXPLOSION's model (`0x16`) drawn 2.5 × 1 ×
2.5 (`r2-0x5670`, `r2-0x5710`) and 3 higher (`r2-0x5650`), blasting out
to 12 (`r2-0x5658`) with kind `0x421`, plus the effect's own `EXPCHEST`
(`DAT_80284b54`) at the chest.

## Level record `+0xD8`, `+0xDC`

Two more item scales in the level record ([monsters.md](monsters.md)):

| offset | use | values |
| --- | --- | --- |
| `+0xD8` | × damage tiles' off time | 0.5–1.25 (A1 0.8) |
| `+0xDC` | × hazard damage: damage tiles, explosions | 0.5–1.0 (A1 0.65) |

The forest realm and some secret/test levels have 0 here as in the other
scales.

## In this rewrite

[`mechanics.rs`](../crates/gdl-game/src/mechanics.rs) runs, each 30 Hz tick
after the hero moves:

- **Touches**: the hero's feet against each trigger's touch shape (the
  placement's radius as a cylinder, or the type's shape; LIFTPAD doubled),
  marking it and the triggers chained after it (not quest ones).
  Chained-to triggers and hit switches can't be touched.
- **Trigger update** exactly as above: flags 1 / 2 / 4 / plain pad, the
  lift timer (120 fields), stand-on-target (0x100), all-players (0x400,
  one player), the touches cleared unless flags 0xC0. Pads play OFF / ONA /
  ON / OFFA.
- **Movers**: one per target node — every trigger's, made for the party
  or not, as the game registers them (the first registration's kind,
  flags, heights and sound; later ones fill heights still 0 and a sound
  still ≤ 0; lift kinds merged) — starting at the off height; only the
  party's triggers ever change them. Toward on / off at 4 units/s,
  arrival within 0.001, kind flag 0x20 turning round, kind flag 8 or no
  player on it to move, the disable byte 1 while moving (0xFF for hidden
  bridges), state kept by kinds 0x47 (bridges while someone stands on
  them); on a node in the animated mode, the play flags instead of a
  height.
- **Rotators** subtypes 0 (always) and 2 (once touched, to the limit,
  grinding with `S_ROCKROTATE` in realm A or `S_METLROTATE` in realm I
  and stopping with `S_ROCKSTOP` / `S_METLROTATESTO`, `FUN_8009d01c`'s
  tables `0x801232e4` / `0x8012331c`).
- **Poses**: each moving root's world pose is its own move, then its
  moving parent's. Every node in its subtree gets it in the collision
  (`LevelCollision::set_pose`), and `world.rs` draws the subtree's model
  placements as one `MovingGroup` entity posed each frame (interpolated
  between ticks).
- **Riding**: every item the level's drop put on a moving node's floor
  (lift pads, pickups on platforms) moves its touch shape and model with
  that node's pose ([level-population.md](level-population.md),
  "Placements"); a trigger for every player dropped onto its target counts
  only stood on there. The hero standing on a moving node is carried by
  the node's change of pose (feet, floor and facing).

`GDL_WARP="x,y,z"` starts the hero somewhere else for testing (on
levelA4 `-132.1,21,-1` is the LIFTPAD of `A4ELEV8`: the hero rides it down
13 units, waits 2 s, and back up while it's held).

Bridges fade in and out at the game's 8 alpha steps a field (the shader's
fade, `uv_offset.w`; opaque parts blend while fading).

Animated objects (`mechanics.rs`): their nodes are moving roots too, posed
from their tracks each tick as above — the movers aimed at them drive
the play flags, the cut that shows one waits for it (`node_moving`), the
damaging walls read the run-time flags (`Mechanics::node_flags`). Their
scale is drawn (about the node) but their collision doesn't scale (not
confirmed either way). A bursting one (type `0x50000`) hides for the tick
its lap ends and shows the realm's explosion (`EffectAt`) and the ice
realm's sound where it burst; the explosion carries its blast
(`effects::WorldBurst`): 50 damage out to 6 (kind `0x800`) in the ice and
sky realms, out to 5 (`0x21`) elsewhere, no one's, hurting heroes, monsters
and items (flags `0x2B`) for as long as the explosion plays.

Stand-ins and gaps: triggers run on or off screen; subtype 1 rotators;
only the hero (not monsters) holds a mover by standing on it. Hazards and
breakables are below.

[`hazards.rs`](../crates/gdl-game/src/hazards.rs) runs the damage tiles
and damaging walls:

- Tiles cycle as decoded: OFF waits the type's `+0x48` (or −3 × the
  placement's `+0x32`) × 2 fields × level `+0xD8`, a negative time random
  in |v|..3|v|; the other actions last their animation. Out (state 2 or 4)
  a tile with type flag 1 hurts a hero standing in its shape: damage
  (placement `+0x30` or type `+0x40`) × level `+0xDC`, less armour, flags
  = value | 0x80, pushed out along the tile's −Z for values with 0x30, the
  realm's tile sound, once per phase. Levitating heroes are spared.
- Walls: the last wall the hero's move hit, with its node's flags OR'ed up
  the parents: 15 (0x30000–0x50000, knockdown and push), 10 (0x20000,
  knockback and push) or 5, less armour, once a second.
- On levelA1, `GDL_WARP="-46.8,0.25,7.5"` stands the hero in a force
  field: 11.5 damage each time it comes on.

Stand-ins: active phases last their animation (the game's own timer for
them isn't confirmed). Walls on nodes in the animated mode hurt only
while they animate (or a mover moves them).

[`breakables.rs`](../crates/gdl-game/src/breakables.rs): each hittable
item (armour byte ≠ −1; barrel containers 0x2B–0x2D before they break,
obstacles but safe rocks, hit switches) gets a `TargetKind::Breakable`
target at its touch centre, kept there as the item rides a moving floor
(G2's dropping plank, G3's rising floor; `breakables::follow` — the game
searches its items where they are). A blow takes damage − armour (at least 1,
rounded) off its hit points (type `+0x44`), none for kind 0x800 blows. At
0: containers flagged 0x200 break open (`USED`, the realm's
`S_BARREL_WOOD<letter>` for barrels) and release their contents as a new
item where they stood, with its model (`population::ContentModels`, built
at level load for everything containers hold) and a 30-field pickup delay,
keys as many as the container's `+0x34`; exploding barrels blast 30 ×
level `+0xDC` over 6 units and poison barrels 10 × over 6.5 (the missiles'
falloff, hurting monsters and the hero at once), with
`S_BARREL_EXPLO<letter>` / `S_BARREL_GAS<letter>`; other obstacles break
(`S_WEAPONHITWOOD` until then); shootable walls are freed with
`S_SECRETWALL`; hit switches press down their chain every time.

On levelA1: `GDL_WARP="-5.5,-2.5,-23"` with `GDL_BUTTONS=attack` breaks
the barrel behind the start (a blue potion falls out);
`GDL_WARP="-13.5,-2.5,-3.8"` sets off an exploding barrel (17.5 to the
hero).

A monster inside (the 84 Deaths) comes out where the container stood, at
tier 1 (its model is loaded with the level; on levelA2
`GDL_WARP="9.69,2.5,-50.8"` + attack breaks one open).

The barrels' blasts are the game's explosions (`effects.rs`,
`ExplosionAt`): `FUN_80092bf4(damage, position, effect)` makes an area
effect (flags `0x2B`: players, items, monsters) owned by nobody —
effect `0x18` (EXPLOSION) for an exploding barrel, kind `0x421`, radius
12 (`r2-0x5658`), drawn 1.75 × wide (`r2-0x5648`) and 2 higher
(`r2-0x5660`); `0x19` (POISONEXP1 → POISONEXP2 held 4 s, `r2-0x5640` →
POISONEXP3) for a poison barrel, kind `0x800`, radius 6.5 (`r2-0x563c`),
drawn 3.5 × wide (`r2-0x5638`); `0x1D` (EXPLOSION's model with EXPCHEST)
for CHESTEXP, radius 12, drawn 2.5 × wide and 3 higher. (A default of 6
and 1.5 × is for other ids.) So a barrel's blast sets off the barrels
and floor potions near it, and earns nobody experience.

Blasts reach what blows can't through `BlastItem` (`breakables.rs`;
`blast_reaches` is the effect item test for items of armour below 0):
explosive ones blow chests apart (`CHESTDEST` and `DESTSMOKE`; a Death
inside comes out, anything else is lost), set CHESTEXPs off, turn
treasure to junk (worth 10) and blow other powerups up; poison gas spoils
food (bad meat −100, a green apple −50, eaten as poison). A barrel
container blown apart without breaking open goes the same way. A CHESTEXP
opened with a key ticks (`S_TICKY`) instead of giving its contents, and
once open explodes: 50 × level `+0xDC`, `EXPCHEST`, the realm's
`S_BARREL_EXPLO<letter>`, and it's freed.

The blasts (`effects.rs::tick_blasts`) send a `BlastItem` for every
item in their reach that `blast_reaches` lets through, each spared as
long as their targets are (the game's item cooldown `+0xD8` isn't
traced). A CHESTEXP goes off as effect `0x1D` (`Exploder::Chest`: the
fireball out to 12, drawn 2.5 × wide and 3 higher, and `EXPCHEST` turned
as the chest), and the hints `0x87`, `0x88`, `0x89` show.

Stand-ins: the explosions' lights; the junk, spoiled food and left models
(`TREAS_JUNK`, `BADMEAT`, `GAPPLE`, `ITEMEXP0`, `CHESTSEXP0`,
`CHESTGEXP0`) show only if the level built them (`ContentModels`); a
released monster starts
right away (the game wakes a placed-monster item); a shootable wall's in-between hits are silent;
safe rocks aren't hittable; walls' own collision (item shape 4) isn't
ported, so shootable walls never blocked the hero in the first place.
