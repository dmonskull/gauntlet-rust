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
| 2 | second table count (`0x28` bytes: u16, u16, u32, two vec3, 4 × u16) |
| 3 | second table offset |
| 4–8 | further tables (u32 array, u16 array, 8-byte records) — not named yet |
| 9–14 | level bounding box min / max (f32) |
| 15 | grid cell size (f32; 3.0 in retail levels) |
| 16, 17 | grid width, depth in cells — they cover the bounding box's X and Z |
| 18–23 | further table counts/offsets — not named yet |
| 24 | version: `0xF00BAB00` \| revision (retail: 2) |
| 25–29 | revision-gated extras |

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
| +0x30 | bounding value — equals the model record's `+0x04` |

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
