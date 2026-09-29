# Level collision

Implemented in [`crates/gdl-formats/src/collision.rs`](../crates/gdl-formats/src/collision.rs)
(tables parsed as part of `WorldFile`, queries on `LevelCollision`), with a
debug overlay in [`crates/gdl-game/src/collision_debug.rs`](../crates/gdl-game/src/collision_debug.rs)
(`K` toggles it; `GDL_DEBUG_COLLISION=1` starts with it on).

Verified over all 68 `WORLDS.PS2` files on disc (67 levels plus
`ORIGlevelL1`): 153,360 collision triangles, 618,535 grid cell entries,
2,199 moving nodes — see "Checks" below.

## Where it lives

Collision is **in `WORLDS.PS2`**, not in a separate file and not the render
meshes. It's a simplified triangle soup (levelA1: 1,612 collision triangles
against tens of thousands of render triangles), owned by the scene-graph
nodes, plus an X/Z grid to find them.

`colworlds` is a dead end: the streamer `FUN_800a8bf4` would load a file
named `"colworlds"` (`s_colworlds_80114c34`) into a second world struct at
`0x8028c3c8` when the buffer pointer `r13−0x6d58` is set, and
`FUN_800a86fc` would wire it with the same `FUN_800a964c` — but the level
loader `FUN_800a8dfc` always stores 0 there, and no level folder on the disc
has such a file.

## Tables (wired by `FUN_800a964c`)

`FUN_800a964c(world, file)` fills the world struct at `0x8028c46c`. The
fields that matter here, as addresses:

| struct field | from header | what |
| --- | --- | --- |
| `0x8028c470` | word 1 | nodes (`0x3C` bytes) |
| `0x8028c474` | word 3, count word 2 | collision triangles (`0x28` bytes) |
| `0x8028c478` | word 8, count word 17 | grid rows (8 bytes) |
| `0x8028c47c` | word 5, (word 8 − word 5) / 4 entries | grid cells (u32) |
| `0x8028c480` | word 7, (word 5 − word 7) / 2 entries | cell lists (u16) |
| `0x8028c484..` | words 9–14 | level bounds min / max |
| `0x8028c4b4` | word 15 | cell size (3.0 in every level) |
| `0x8028c4b8` | 1 / word 15 | inverse cell size |
| `0x8028c4bc`, `0x8028c4c0` | words 16, 17 | grid columns (X), rows (Z) |
| `0x8028c4c4` | — | query stamp, 1..255 |
| `0x8028c4c8` | — | one "visited" byte per triangle (allocated here) |

Header word 4 (varies per level) and word 6 (always 0) aren't read by the
wiring function. Words 18–23 are three gameplay tables (`0x50`-,
`0x3C`- and `0x1C`-byte records, used by `FUN_8005ba08`, `FUN_80063fb0`,
`FUN_80066258` …) — not collision, not decoded yet.

### Collision triangle (`0x28` bytes)

| offset | type | field |
| --- | --- | --- |
| `+0x00` | i16 | lowest corner height × 64 |
| `+0x02` | i16 | highest corner height × 64 |
| `+0x04` | f32 | frame scale `1/sqrt(1 − ny²)` (∞ for flat triangles, unused there) |
| `+0x08` | 3 × f32 | unit normal; only the side it points to collides |
| `+0x14` | 3 × f32 | first corner (the frame origin) |
| `+0x20` | 2 × i16 | second corner, in-plane X, Z, × 1/64 |
| `+0x24` | 2 × i16 | third corner, in-plane X, Z, × 1/64 |

The plane frame is `FUN_80021a9c` (world → plane) and `FUN_80021b60`
(plane → world), with the scale `s` from `+0x04`:

```
plane.x = s·(nx·p.z − nz·p.x)
plane.y = n·p                              (height above the plane)
plane.z = s·((1 − ny²)·p.y − ny·(nx·p.x + nz·p.z))
```

and the identity (or `(x, −y, −z)`) when `|ny| > 0.999999`
(`r2−0x79a0`/`−0x7998`). The i16 corners are scaled by `r2−0x79c0` = 1/64,
the height range by `r2−0x7fb0` = 64. Decoded this way, every non-sliver
triangle's corners wind counter-clockwise around its stored normal, and
every height range contains its corners.

### Node fields

| offset | field |
| --- | --- |
| `+0x10` | flags; which queries see the node (below) |
| `+0x1C` | at runtime the node's **world** position: `FUN_800aafb0` adds the parent's `+0x1C` in place while placing (the file holds the local offset) |
| `+0x30` | bounding radius — also bounds its collision triangles |
| `+0x35` | disable bits, OR'ed with the parent's; a query skips a node sharing a bit with its mask (0 in every retail file) |
| `+0x36` | i16 collision triangle count |
| `+0x38` | i32 first collision triangle (−1 = none) |

Triangle runs are contiguous and never shared (149,524 of the 153,360
triangles belong to a node; the rest are unreferenced). Triangles of static
nodes are in world space. Nodes with flag `0x1000` or `0x1000000` move
(doors, lifts, traps): their triangles are relative to the node, and
`FUN_8000dfec` transforms the query into node space instead (translation only
when flag `1` is also set, else the instance's inverse matrix via
`FUN_800bde10`).

Flag bits used by the queries: `0x2`, `0x100` walls only; `0x4`, `0x200`
floors only; `0x8`, `0x10`, `0x20` both; `0x800` (particle anchors) neither.
`0x40` skips the normal and height filters; `0x200` hits are kept apart
from the nearest-hit result (`DAT_8023c32c`); `0x38` hits don't push movers
out; `0x10000000` excludes a moving node.

### Grid

- Row `z` (8 bytes): u16 first column, u16 last column, u32 index of its
  first cell. Empty rows are `0xFFFF, 0xFFFE`.
- Cell (u32): entry count in the top 10 bits, byte offset into the lists in
  the low 22.
- Entry: i16 node, i16 count, then `count` i16 triangle indices relative to
  the node's first triangle.
- **Cell 0** is never referenced by a row: it's a plain list of the moving
  nodes. `FUN_800442c0` builds a runtime grid of 10-unit cells for them
  (rebuilt as they move); `FUN_800439e0` walks it and tests all of a moving
  node's triangles.

The level tool's grid is loose: 88,816 listings are in cells nowhere near
their triangle (axis-aligned and diagonal walls smeared along whole rows,
columns or diagonals), and 366 of 434,572 corner/centre probes find a
triangle missing from a cell it covers. The game uses the data as is.

## Queries

`FUN_8000d578(radius, from, to, result, node_mask, disable_mask)` is the one
segment query everything goes through, with a mode word at `r13−0x7978` and
a normal-Y window at `r13−0x796c`/`−0x7970` set by the caller:

1. Clip the segment's X/Z box (grown by the radius) to the level bounds and
   walk the cells along it (`FUN_8000dd00`, `"GRID ERROR"`); bump the stamp.
2. For each entry whose node passes `flags & node_mask`, the disable byte
   and has triangles, `FUN_8000dfec`: reject by bounding sphere (node
   position `+0x1C`, radius `+0x30` + query radius) against the segment,
   move the segment into node space, quantise its height range ×64, then
   `FUN_8000e3b8` over the entry's triangles.
3. `FUN_8000e3b8` skips triangles already stamped this query, and (unless
   node flag `0x40`) those outside the normal-Y window or height range, then
   runs the swept-sphere test `FUN_80021394` (or the plain line crossing
   `FUN_80021050` in any-hit mode `0x20`).
4. Scoring: squared distance from the start to the hit point; in mode
   `0x10` a hit that doesn't cross the triangle scores 10000 × its squared
   distance instead; × 0.95 when `|n · dir| < 0.25`. Lowest wins.
5. Then the moving nodes (`FUN_800439e0`), and the result: node (`+0x44`),
   score (`+0x40`), hit point (`+0x30`, mode `2`), and a frame built from
   the hit normal by `FUN_8000e674` (mode `4`; row 1 is the normal).

`FUN_80021394`, the swept sphere against one triangle, in its plane frame:
the start must be on the front (`y ≥ 0`) and the end no further out. If the
segment crosses the plane, the crossing point inside the triangle is a hit
at distance 0; outside, the nearest edge it's outside of (segment–segment
distance, `FUN_80021c00` → `FUN_80021dfc` / `FUN_8002253c`) must be within
the radius. If it doesn't cross, at least one end must be within the radius
of the plane; the nearer end projecting inside is a hit at distance 0,
otherwise the nearest edge within the radius. We port this; for segments
vertical in X/Z the game uses the horizontal distance to the edge plus
(when not crossing) the nearer end's height, which we do too.

Wrappers (constants from `r2`):

| function | mode | nodes | normal Y | notes |
| --- | --- | --- | --- | --- |
| `FUN_8000d1e0` | 7 | `0x13A` | ±0.866 | **walls**, disable mask 2 |
| `FUN_8000d274` | 7 | `0x13A` | ±0.866 | walls, disable mask 1 |
| `FUN_8000d308` | `0x20` | `0x13A` | ±0.866 | any-hit walls, radius 0.1 |
| `FUN_8000d4b8` | 7 (+`0x10`) | `0x23C` | 0.5 … 2 | **floor probe**: vertical segment from `y+up` to `y+down` |
| `FUN_8000d3c4` | `0x17` | `0x23C` | 0.5 … 2 | **floor height**: from `y+4` to `y−10`; returns the hit Y or the default |
| `FUN_8000cfa0` | 7 | `0x23E` | ±2 | generic ray |
| `FUN_8000cf40` | 3 | `0x3E` | ±2 | radius 0.1, line of sight |

So walls are faces at least 30° from horizontal and floors at most 60°; the
band in between is both. Items are dropped onto the floor with
`FUN_8000d3c4` (radius 1.0, `r2−0x6804`) plus 0.1 in `FUN_80064140`, which
logs `"Bad Item floor pos"` when nothing is found.

### Moving an actor

`FUN_80035320` (called from the movement update `FUN_8003a8dc`), per move:

1. Wall test (`FUN_8000d1e0`) from the position to position + move, radius
   from the actor's type (`+0x7C`). On a hit whose node isn't `0x38`,
   `FUN_8000d034` pushes the destination back out along the wall normal in
   X/Z to exactly the radius: each axis is corrected when it's the minor axis
   of the move or the move goes into the wall along it; a correction larger
   than the move plus half the radius zeroes that axis and blocks the whole
   horizontal move.
2. Floor probe (`FUN_8000d4b8`, radius 1.0, from `step` above to
   `step + 3` below, `step` from the type's `+0x78`) at the leading edge,
   position + direction × (move length + radius). The floor is accepted if
   its height is within 2 × (radius + length) of the feet; if it's more
   than 0.1 × length away it's re-probed at the actual destination.
   Otherwise the horizontal move is cancelled (and the Y follows the floor
   under the current position). No floor ahead at all also cancels it.
3. Y moves to the floor, but down by at most 16 × frame time
   (`r2−0x7288`, `r13−0x7570`).

Monsters (`FUN_800453f0` → `FUN_80045b98`, 0x394-byte records at
`0x802515e8`) do the same with a probe radius of half the actor radius,
`+5` extra depth and their own step value.

`LevelCollision::move_actor` implements `FUN_80035320`'s version; the actor
type values (radius, step) aren't decoded yet, so callers pass them.

### Moving a player

Players don't use `FUN_80035320`. The player update `FUN_80080d3c` (called
per player from `FUN_8007692c`; players are the 4 × `0x335C`-byte records
at `0x802754c0`) resolves the tick's displacement with its own chain.
Implemented as `LevelCollision::move_player` with `PlayerCollision` (shape)
and `PlayerGround` (the floor state carried between ticks).

**The player's shape** comes from the class's `PDAT` record
([chunk-files.md](chunk-files.md)), copied when the player joins
(`FUN_80079ed8`; `DAT_80282310[player]` points at the loaded `PDAT`):

| player field | value | from | use |
| --- | --- | --- | --- |
| `+0x850` | 1.5 | `PDAT+0x4C` | collision radius (walls, floors, other actors) |
| `+0x854` | 2.5 | `PDAT+0x48` × 0.5 (`r2−0x5ea8`) | half-height: floor probe depth, actor-vs-actor vertical reach |
| `+0x838` | (0, 4.4, 0) | `PDAT+0x50` | offset of the top point `+0x54` |
| `+0x844` | (0, 2.5, 0) | `PDAT+0x54` | offset of the collision centre `+0x64` |

All 16 classes on the disc have the same four values. `FUN_8005a334`
(`FUN_8005a658` / `FUN_8005a584`, run for every player by `FUN_8007692c`
before the update) sets `+0x64` = feet (`+0x44`, the matrix translation at
`+0x14` + `0x30`) + the rotated `+0x844`, and `+0x54` likewise from
`+0x838`. So the collision tests start from the **centre, 2.5 above the
feet**. The radius at `+0x850` is confirmed by player-vs-player
(`FUN_80087068` adds the other player's `+0x850`) and the half-height by the
cylinder test `FUN_8002fa24` (`|dy| ≤ h₁ + h₂` with both `+0x854`s).

**Order in `FUN_80080d3c`** (the plain-walking path; `c` is `+0x64`, `d`
the displacement from [player-movement.md](player-movement.md), whose Y is
clamped to ≥ 0 first):

1. `FUN_80086e44(r, h, …)` — other entities (the `0xF0`-byte objects at
   `r13−0x71a8`) — and `FUN_80087068` — other players' cylinders, pushing
   `d` so it ends touching. Actor collision, **not ported**.
2. **Walls**, `FUN_80087f4c(r, …)` with `c.y` raised by 1.0 (`r2−0x5be0`)
   for the call: `FUN_8000d274`, the walls query (`0x13A`, normal Y ±0.866)
   with disable mask **1**, from `c′` to `c′ + d`, radius `r`.
   - A hit on a node with flags `0x38` changes nothing (returns 2, or 1
     in action `0x8F`).
   - A hit on a moving node (own flag `0x1000`) slides: `d −= n (n·d)`, in
     3D.
   - Otherwise `FUN_8000d034` pushes `d` out (see "Moving an actor"; its
     return value is ignored here).
   - Then the hit triangle's visited byte is bumped (so the next query
     skips it) and the walls are tested again from `c′` to `c′ + d` with
     radius 0.95 r (`r2−0x5a38`). A second wall more than 60° away from
     the first (normals' dot **below 0.5**, `r2−0x5cb0`) — a corner —
     zeroes all of `d`.
   - Returns 0 (no wall), 1, or 2. A hit also runs `FUN_80085b14`
     (damaging surfaces, node flags `0xF0000`) — not ported.
3. **Floor**, `FUN_800878a0(r, h, …, wall result)`: probe `FUN_8000d4b8`
   at `p = c + d`, from `p` (up 0.0) down `3 + h` (`r2−0x5c88`), radius
   `r`, crossing-preferring (mode `0x10`), disable mask 1; result at
   `0x80282248`. So floors up to the centre (2.5 above the feet — the
   highest step) and down to 3 below the feet are found. With `F` the hit
   height, `F₀` the floor being followed (`+0x8b4`) and `|dxz|` the
   horizontal length of `d`:
   - A standing node (`+0x8c4`, flags inherited up the parents by
     `FUN_800aacac`) with flags `0x0C000000` and `0x20000000` can't be left
     for another node: `d` = 0, returns −1 (−2 if no floor). Not ported.
   - **No floor**: if `|dxz|` < 0.001 forget the standing node; unless
     airborne (`+0x8d4 & 0x8000`) zero `d.xz`; `F₀` = the kill height
     `r13−0x7278` (level bounds min Y − 4.5, set in `FUN_80057020`);
     returns −2.
   - Airborne: while `feet.y − F ≥ 0.2` return 0, else clear the flag.
   - **Standing still** (`|dxz|` < 0.001 and `+0xEC`, last tick's state,
     is 1 = playing): `F₀ = F` only if `|F − F₀| > 3` or the node moves;
     returns 1 if `|F − F₀| > 0.001`, else 0.
   - `|F − F₀|` above 3 (`r2−0x5bb8`; 4 when the hit is the node already
     stood on) **refuses the move**: `d.xz` = 0, `F₀` = feet Y, returns 0.
   - If the wall result was 2 and the floor isn't that wall's node: push
     `d` out of that wall (`FUN_8000d034`) and re-probe (below).
   - Otherwise let `o = p − hit` (how far the hit is from straight below
     `p`), `a = d · o`, `s = |o.xz|`. If `s` < 0.001 and there was no wall,
     also probe a radius further on, at `p + r·normalize(d)`:
     nothing there → `o` = that `r·dir`, `a` = 0, `s` = r; a hit on a node
     with flag `0x8` within 4 (`r2−0x5c08`) of `F` → `s` = 0; any other
     hit → `o`, `a`, `s` from that probe.
   - `s` < 0.001 or `a` < 0 (the floor is right below, or the move heads
     towards it): **accept**, `F₀ = F`, return 1 (2 when `s` ≥ 0.001 and
     `a` ≥ −0.25, `r2−0x5a40`).
   - Else **pull back**: `d.xz −= o.xz` and re-probe at `c + d`
     (not crossing-preferring): a floor → `F₀` = its height, return 2;
     none → `d.xz` = 0, return 0. This holds a player's centre a radius
     back from a drop it can't follow, and over a ledge's edge when the
     probe only grazes it.
4. **Follow the floor** (back in `FUN_80080d3c`): when the result is > 0,
   or 0 and the standing node doesn't move (own flag `0x1000`), or −2 and
   there's no standing node (or airborne):
   `d.y += max(F₀ − feet.y, −16 × tick)` (`r2−0x5bd8`, `r13−0x7570`). Up
   is immediate; down is at most 16 units/s.
5. `FUN_8001bf88` — keeping players on screen together (camera bounds,
   `FUN_80025ca0` / `FUN_8001c084`) — may clip `d.xz`. Not ported.
6. **Climbing slows the walk**: if `d.y > 0.1 × step` (`step` = the stick
   step, factor × tick × speed × magnitude) and `hypot(step, d.y) > 0.01`,
   `d.xz *= step / hypot(step, d.y)` (`FUN_800bce38` is an approximate
   hypot).
7. If the floor result is > 0, `FUN_8008764c`: the same node lock as
   above, then `FUN_800877c0` records the floor node as the standing node
   `+0x8c4` and attaches to moving platforms (zeroing `d.xz`), and a slope
   value `+0x8bc`. Only the standing node is ported.
8. Actor collision again with 0.9 r (`r2−0x5bc0`): entities
   (`FUN_80086e44`), then teleporters (`FUN_80086ab8`, which can replace
   `d`), players (`FUN_80087068`), and monsters/generators
   (`FUN_80087258`, radius `r`, or 3.0 / `r2−0x5c9c` for actions `0x8F` /
   `0x89`); each clips `d` per axis with `FUN_800859d4` so it stops
   heading into the other actor. Actions `0x89` and `0x8F` (charges /
   being carried) also knock the other actor back. **Not ported.**
9. `+0x44 += d` (after the attack/shove handling, which can zero `d`).

`FUN_80087ec0` (in one game mode, `r13−0x7380 == 0x4010`) probes the same
floor from the feet instead of the centre and sets the standing node.

**What `move_player` does differently**: no actors, hazards, lifts,
platform locks, airborne state or on-screen clipping; the climb
slow-down's `step` is the horizontal length of the requested move (the
game's stick step whenever there's no knockback); exact `hypot`.

**Checks** (`collision::tests::players_*`): synthetic walls, corners,
ledges, steps and drops, plus
`players_walk_every_real_level_without_falling_through`: from every
level's entry-0 player start (on the floor under it), running
(12.5 × 1.3 units/s) 45 ticks in 8 directions: 24,480 moves, 6,118 touching
walls, 16 held back by the floor. The centre's path never crosses a wall,
the floor is never lost, and the feet never end more than one tick's drop
below a floor. They do dip for a tick 21 times, both by the game's rules:
at seams where a near miss 0.02 away on a lower floor outranks the floor
right below (10000 × d² < the crossing's distance², levelE1), and where
sliding along a moving node's downward-facing wall pushes `d` down
(levelK2).

## What we do differently

- Cells are gathered from the segment's whole X/Z box rather than walked
  along the line — a superset, so the same nearest hit.
- Moving nodes are tested at rest (their world position), all triangles,
  with no runtime grid. Nothing animates them yet.
- `0x200` hits are dropped rather than reported separately.
- `top_floor(x, z)` (a probe from the top of the level down) is ours, for
  placing things with no starting height.

## Checks

`collision::tests::every_real_level_collision_is_consistent` over every
`WORLDS.PS2`: normals are unit; every non-sliver triangle's corners wind
counter-clockwise around its normal (371 slivers skipped); frame scales
match the normal; every corner is inside its node's bounding sphere and its
height range; static triangles are inside the level bounds; triangle runs
don't overlap; the grid references only real nodes and triangles; and the
floor query run from just above each of the 27,402 static floor triangles
finds a floor every time — that triangle or one above it in 27,332 cases.
The other 70 are the game's own ranking: a floor the probe misses by a hair
(10000 × a squared distance of ~0.001) beats a real crossing a few units
further down. 34,390 of 53,300 placed models have a floor within the item
query's reach (the rest are in the air: torches, ceilings, scenery).

## Not confirmed

- What `step` and radius each monster/actor type uses (type data not
  decoded). Players: see "Moving a player".
- For players: the airborne flag `+0x8d4 & 0x8000` (who sets it), the
  node lock flags `0x0C000000`/`0x20000000`, and whether any level's
  moving nodes actually take the 3D wall slide at rest positions (we test
  them un-animated).
- Header words 4 and 6; node `+0x34`.
- The swept-sphere distance helpers were ported for their result (closest
  points between segments), not instruction by instruction.
