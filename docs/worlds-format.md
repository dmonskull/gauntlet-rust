# `WORLDS.PS2` — level scene graph

Implemented in [`crates/gdl-formats/src/world.rs`](../crates/gdl-formats/src/world.rs).
Verified: all 67 levels parse and walk without cycles; all 50,402 model
placements resolve by name to their level's `objects.ngc`.

## Loading path

`FUN_800a8dfc` loads a level's models and records the level folder;
`FUN_800a8bf4` then streams the file named `"worlds"` (`r2−0x5010`; `.PS2`
is appended by the file layer) and, on completion, `FUN_800a8f4c` swaps the
header. `FUN_800a964c` swaps the tables and wires up the world struct.
Little-endian.

## Header (30 × u32)

| word | field |
| --- | --- |
| 0 | node count |
| 1 | nodes offset (`0x3C`-byte records) |
| 2 | collision triangle count (`0x28` bytes, [collision.md](collision.md)) |
| 3 | collision triangles offset |
| 4 | not read by the wiring function |
| 5 | collision grid cells offset (u32 each) |
| 6 | not read (0 in every level) |
| 7 | collision grid cell lists offset (u16 each) |
| 8 | collision grid rows offset (8 bytes, one per grid row) |
| 9–14 | level bounding box min / max (f32) |
| 15 | grid cell size (f32; 3.0 in retail levels) |
| 16, 17 | grid width, depth in cells — they cover the bounding box's X and Z |
| 18–23 | three gameplay tables (`0x50`, `0x3C`, `0x1C` bytes) — not decoded yet |
| 24 | version: `0xF00BAB00` \| revision (retail: 2) |
| 25 | animated objects: clips header offset (below) |
| 26 | animated objects: count |
| 27 | animated objects: table offset (`0x10`-byte records) |
| 28, 29 | revision-gated extras, not decoded |

## Node (`0x3C` bytes)

| offset | field |
| --- | --- |
| +0x00 | name, 16 bytes |
| +0x10 | flags (`0x1001000` = dynamic; `0x800`, `0x8000`, `0x400` change instancing) |
| +0x18 | parent pointer (runtime only) |
| +0x1C | translation relative to the parent (3 × f32) |
| +0x28 | 1 = draws the model with its name (runtime: replaced by the instance) |
| +0x2C | next sibling (i16, −1 = none) |
| +0x2E | first child (i16, −1 = none) |
| +0x30 | bounding value — equals the model record's `+0x04`; also bounds the node's collision triangles |
| +0x35 | collision disable bits (0 in every level) |
| +0x36 | collision triangle count (i16) |
| +0x38 | first collision triangle (i32, −1 = none) |

## Placement

`FUN_800aacd0` walks the tree from node 0 (node 0 and its siblings are the
roots), recursing into children. For each node `FUN_800aafb0` creates an
instance under the parent's instance with the node's local translation —
there's no rotation or scale — so a node's world position is the sum of the
translations up its ancestor chain. `FUN_800b8610` then attaches the model
found by `FUN_800b8684`: an exact 16-byte name match, binary-searched in
each loaded model file's sorted name table, falling back to `AAANULLOBJ`.

Coordinates are Y-up, and triangle winding is counter-clockwise in a
right-handed frame, matching Bevy directly.

## Animated objects (words 25–27)

With a revision (`version & 0xF00BAB00 == 0xF00BAB00`, low byte ≠ 0) and
word 25 set, `FUN_800a964c` reads the level's animated objects
(`world::object_animations`, carried in `Population::animations`):

- word 25: a clips header — the same seven words as an `ANIM.PS2` atree's
  ([animation-format.md](animation-format.md): delta tables, key data,
  track table, one action, one bone per object), swapped and relocated by
  `FUN_8000e994`;
- words 26/27: the objects, `0x10` bytes each: node index (i16), frame
  count (i16), a word marking the record relocated (`0x8000`), the play
  flags (u16; set to `0x101` as the level loads), the current frame (f32),
  and the file offset of the object's track entry in the clips' table
  (bone *k* for object *k* on every level).

As it loads, every object's node gets flag `0x2000000`, and
`FUN_800aafb0` passes the flag down to every node built under one: the
animated mode ([mechanics.md](mechanics.md), "Animated objects"). 1,608
objects on the disc (68 worlds), 131,266 keys; 418 tracks move only (no
rotation or scale, no delta keys), which the game plays (its test for "no
animation" is `flags & 0x0FFF == 0`).

A node's flags word holds text on 30 nodes (`"trig"`, `0x67697274`, on
fifteen of A2's rock piles and trims — an exporter's tag); the game reads
it as flags like any other (`0x1000` and `0x1000000` moving collision,
`0x2000000` the animated mode, `0x90000` a damaging wall while it
animates) but none of them is in the table or a trigger's target, so
nothing moves them and they never hurt.
