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

- What `step` and radius each character type uses (type data not decoded).
- Header words 4 and 6; node `+0x34`.
- The swept-sphere distance helpers were ported for their result (closest
  points between segments), not instruction by instruction.
