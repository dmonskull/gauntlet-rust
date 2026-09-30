# `ANIM.PS2` — skeletons and animation

Implemented in [`crates/gdl-formats/src/anim.rs`](../crates/gdl-formats/src/anim.rs),
played by [`crates/gdl-game/src/character.rs`](../crates/gdl-game/src/character.rs).
Verified: all 712 `ANIM.PS2` files on the disc parse — 2,984 atrees, 81,920
tracks, 1.29M keys, all finite — and characters pose and move believably
in the viewer (`cargo run -p gdl-game -- --viewer --character KNI`).

## Where it's loaded

Levels load `"anim"` (`r2−0x5018`) beside `"worlds"`; players load
`"players/%s/anim"`. Every loaded file goes through `FUN_8001267c`, which
byte-swaps and relocates it and registers each atree (up to 24, `"Too many
Atrees"`). Little-endian.

## File header

| offset | field |
| --- | --- |
| 0x0 (u16) | atree count |
| 0x2 (u16) | version; ≥ 8 adds the pair at 0x10 |
| 0x4 | atree table offset (`0x24`-byte entries: name[0x20], data offset) |
| 0x8 / 0xC | count / offset of `0x58`-byte records (attached effects; not decoded) |
| 0x10 / 0x14 | count / offset of `0x138`-byte records (particle systems; same parser the world uses) |

## Atree (offsets relative to the atree)

| offset | field |
| --- | --- |
| 0x00 | actions offset (`0x30`-byte records) |
| 0x04 | clips offset → header handed to `FUN_8000e994` |
| 0x08 | object-animation list (`FUN_80018f00`) |
| 0x0C | skeleton nodes offset (`0x3C`-byte records) |
| 0x10 | node count (max 0x200) |
| 0x14 | action count |
| 0x18 | name — the model prefix (`ARC_BLU`), or class name for shared clips |

**Node** (`FUN_80012f9c` instantiates the atree, `FUN_80013480` each node):

| offset | field |
| --- | --- |
| +0x00 | name[0x20] |
| +0x20 | rest offset from the parent (3 × f32) |
| +0x2C (u16) | kind: 0 static, **1 skeletal**, **2 flipbook**, 3 (a texture modifier: `+0x34` is the byte offset from the action table to its record in the file's list; see `docs/rendering.md`), **4 particle system** (below) |
| +0x2E (u16) | bit 0: no model of its own |
| +0x30 (u32) | render flags set on the node's instance (bit 0 = hidden; glows use `0x4c01880`) |
| +0x34 (i32) | **byte offset**: skeletal → from the clips header to this bone's run of track entries; flipbook → from the list header to its first entry |
| +0x38 | parent index (−1 = root; parents precede children; one root) |

**Particle-system nodes** (kind 4, `FUN_80013480` → `FUN_80018304` →
`FUN_800cede8`): the node's `+0x34` is a byte offset from the atree's
action table (`+0x00`) to a `0x138`-byte particle record — the same format
as the levels' (`docs/rendering.md`); `WEAPONS/ANIM.PS2` keeps its records
(letters M, N, O… for BLOODFX1, P, Q, R… for BLOODFX2) between atrees. The
node has no name: its name field's words `+0x14..+0x1F` are three floats
handed to the record's set-up (`FUN_800ceeb8`; (0, 0.5, 0.866) on the blood
sprays — taken as the spray's direction), and the node's own object is
hidden (instance flag 1). `anim::Atree::particles`.

A node's model is the object named `<atree name><node name>`, found with
the same name lookup the world uses — so segmented characters are one rigid
object per bone. `DUMMY` nodes are hidden (the instantiator compares the
name against `"DUMMY"` at `r2−0x7ec8`).

## Flipbooks (object animation, `FUN_80018f00`)

Regular monsters (grunts, demons, ghosts…) aren't skinned: every frame of
every action is a separate pre-posed mesh, named e.g. `DEM1WALK2F03` (tier,
action, part, frame). The atree's list is `{u32 offset from here, u32
count}` then `0x28`-byte entries: first frame's object name[0x20], (runtime
object), u16 frame count, u16 parameter. A flipbook node's entries run one
per action from its `+0x34` offset; frame *k* of an action is the *k*-th
object after the named one (objects are stored sorted by name). All 687
flipbook nodes on the disc have an entry for every action.

**Action**: name[0x20], frame count (u16 `+0x20`), rate (u16 `+0x22`: time
per frame, see below; 30 almost everywhere, 60/45/40/15… for some), params
(`+0x24` bit 0 = loops; `+0x26` bit 0 = move the object by the root's offset
when the action ends; `+0x28` = how many texture modifiers the action runs;
`+0x2A` bit 0 = its flipbook frames and texture modifiers play backwards),
and `+0x2C` the index of its first texture modifier in the file's list (−1
none; a pointer after `FUN_8001267c` loads the atree — see
`docs/rendering.md`, "Texture animation"). No monster action sets the
`+0x26` or `+0x2A` bits.

## Playing an action (`FUN_8000ed70`, `FUN_8000ef18`)

An object's anim instance times its action against the game clock
`r13-0x757c`, in seconds (`FUN_8002eff4` adds fields / 60 each frame, 1/30
at the game's 30 fps):

- starting (`FUN_8000ed70`): frame length `+0x2C` = rate × 1/30
  (`r2-0x7f60`) × the instance's speed `+0x28` (1.0, `FUN_8000e910`; nothing
  in the player or monster code changes it); a rate below 1 uses the speed
  alone. Start time `+0x20` = clock − start frame × length × 1/30
  (`r13-0x7958`, set by `FUN_8000e8e8`).
- each frame (`FUN_8000ef18`): t = (clock − start) / (length / 30); the
  frame shown is floor(t + 0.5) (`r2-0x7f48`), or t itself where the
  instance interpolates and t is more than 0.125 from it. Once that
  reaches frames − 1 + 0.5 the action is over: it holds its last frame, or
  a looping one restarts from 0 at that tick.

So each frame lasts **rate / 900 s**: rate 30 plays at 30 frames a second,
60 at 15, 45 at 20, 40 at 22.5, 15 at 60. A clip is done half a frame after
its last frame comes up; a 30-rate loop of *n* frames comes round every *n*
ticks, a 60-rate one every 2*n* − 1. (`character::clip_fps`, `clip_end`,
`loop_length`; the rewrite had played the rate as frames per second, so
60-rate clips — most monster attacks, some walks — ran four times too
fast.)

A player class splits these: `PLAYERS/<class>/<variant>/ANIM.PS2` holds the
variant's skeleton (named `ARC_BLU` etc., no clips), and
`PLAYERS/<class>/ANIM/ANIM.PS2` the class's shared skeleton, 139 actions and
clips (named `ARC`). Bones match by name.

## Clips header (7 words, `FUN_8000e994`)

Offsets relative to the header: rotation delta table, translation delta
table, scale delta table (256 × f32 each; offset 0 = absent), key data,
track table; then action count, bone count.

The track table is **bone-major**: entry `bone × actions + action`, 8 bytes:
`u16 flags, u16 channel count, u32 offset into key data`. A bone whose flags
have no low-nibble bits and no high byte isn't animated by that action
(`FUN_8000f2d8`): it stays at its rest offset with identity rotation.

## Tracks (`FUN_8000f7f4`)

Flag bits (masks at `0x80117b40`):

| bits | channels | default |
| --- | --- | --- |
| `0x001 0x002 0x004` | rotation x y z (radians) | 0 |
| `0x010 0x020 0x040` | translation x y z (added to the rest offset) | 0 |
| `0x100 0x200 0x400` | scale x y z | 1 |
| `0x080` | use the alternate Euler order | |
| `0x2000` | keys after the first are delta bytes | |
| `0x4000` | single static key, no bitmap | |

Data: a keyframe bitmap (`ceil(frames / 32)` little-endian u32 words, bit
*i* = frame *i*; `FUN_80010904` finds the next key), then the first key as
floats in channel order (rotation, translation, scale). Each later key is
either full floats again, or — with `0x2000` — one byte per channel, each
added to the running value through that group's delta table.

Sampling: linear interpolation between the surrounding keys; a rotation
channel whose step is ≥ π/2 in magnitude holds the earlier key instead
(`r2−0x7ef8`/`−0x7ef0`); within 0.125 frames of the next key that key is
used as-is (`r2−0x7f00`); rotations are finally wrapped to (−π, π].

## Bone matrix

`FUN_800bd448` (default) and `FUN_800bd548` (flag `0x080`) build the
rotation from the three angles with explicit formulas (ported verbatim in
`rotation_matrix`), row-major for row vectors with the translation in the
last row. Angles are radians — the sine routine at `FUN_800bcbf8` is a
polynomial over [−π, π], cosine is sine shifted by π/2.

## Not decoded yet

- Action param 2 (`+0x26`) beyond bit 0; the `0x138` records; the
  object-animation list.
- Blending between actions (the state machine around `FUN_8000eb70`).
- Hand-glow effects (nodes with glow render flags are hidden for now).

## Weapons (`FUN_8007ae84`)

The player setup builds `WEAP_<colour>_HD<n>` (`n` = 1/2/3 from a player
stat: < 10, < 50, else) — or `WEAP_HOLD` for classes 8+ — and parents it to
the class's hand bone from the table at `0x8011f90c`/`0x8011f94c`: `R_WRIST`
(WAR VAL WIZ ARC MIN FAL JAC TIG), `RIGHTHAN` (DWF KNI SOR OGR UNI MED),
`RHEND` (JES HYE). Class order (table at `0x8011f878`): WAR VAL WIZ ARC DWF
KNI SOR JES MIN FAL JAC TIG OGR UNI MED HYE SUM. `SHADOWL1` is the blob
shadow.
