# `objects.ngc` — models

Implemented in [`crates/gdl-formats/src/model.rs`](../crates/gdl-formats/src/model.rs).
Verified: all 67 levels parse completely — 51,007 objects, 79,252
submeshes, ~598K triangles, every packet size agreeing with its DMA tag.

## Byte order and version

Little-endian throughout (shared with the PS2 build). `FUN_800b7534` byte
swaps the header and tables after load and warns `"model %d %s has a bad
version (%08x) (want %08x)"` unless the version at `0x40` is `0xF00B000D`.
`levelL2` is `0xF00B000C` — the retail game flags it too; its strip table
entries are 6 bytes instead of 8 (see below). Bytes `0x00..0x20` are the
build-machine path as text; `0x20..0x40` are zero.

## Header (`0x40..0x80`)

| offset | field |
| --- | --- |
| 0x40 | version |
| 0x44 | object count |
| 0x48 | texture binding count |
| 0x4C | object name count (= object count) |
| 0x50 | texture name count |
| 0x54 | objects offset (`0x40`-byte records) |
| 0x58 | texture bindings offset (`0x40`-byte records) |
| 0x5C | object names offset (`0x18`-byte records) |
| 0x60 | texture names offset (`0x24`-byte records) |
| 0x64 | strip table offset (extra submesh descriptors) |
| 0x68 | geometry offset (VIF packets) |
| 0x6C–0x7E | not named yet |

## Object record (`0x40` bytes)

| offset | field |
| --- | --- |
| +0x04 | bounding value (a float; duplicated in `WORLDS.PS2` nodes) |
| +0x08 | flags |
| +0x0C | submesh count |
| +0x10 | first submesh descriptor (8 bytes, below) |
| +0x18 | offset of the remaining descriptors in the strip table |
| +0x1C | offset of the first submesh's packet |

Submesh packets are back to back; each descriptor gives its size.

## Submesh descriptor

From the draw loop `FUN_800c3bbc`:

| offset | field |
| --- | --- |
| +0 | packet size in 16-byte quadwords, including the DMA tag |
| +2 | **diffuse** binding → `FUN_800c3d60` → `FUN_800c6a78` → GX texture map 0 |
| +4 | **lightmap** binding → `FUN_800c6bf0` → GX texture map 1; 0 = none |
| +6 | signed parameter passed with the lightmap (not named yet) |

Legacy `0xF00B000C` strip entries are 6 bytes: size, diffuse, lightmap.

## Names

Object names (`0x18`): `name[16]`, bounding value (u32), object index (u16),
pad. Sorted by name — `FUN_800b8684` (`MBOX_FindObject`) binary-searches it.

Texture names (`0x24`): `name[0x1E]`, binding index, width, height. Used for
looking textures up by name (animated/scrolling textures).

## Texture binding (`0x40` bytes on disk)

The load path compacts these to `0x10` bytes in memory; only these on-disk
fields are read (`FUN_800b7534`, `FUN_800c7510`, `FUN_800c70c4`,
`FUN_800c6d0c`):

| on disk | meaning |
| --- | --- |
| +0x00 (u8) | format selector — see [textures-ngc-format.md](textures-ngc-format.md) |
| +0x08 (u16) | runtime flags; bit `0x100` = untextured |
| +0x0C (u32) | offset into `textures.ngc` (palette first, if any) |
| +0x16 (u16) | width; 0 = untextured |
| +0x18 (u16) | height; 0 = untextured |

## Geometry: PS2 VIF packets

Each packet is a PS2 DMA tag (qwc in the low 16 bits) followed by VIF
`UNPACK` streams — the PS2 vector-unit upload format. The GameCube build
walks them in software in **`FUN_800c48c0`**; `decode_packet` follows it
exactly. Per batch (one triangle strip), in words:

| VIF write | contents |
| --- | --- |
| `V4-32` @0 | vertex count *N*, 0, 1.0, −1.0 |
| positions | cmd `0x69` s16×3, `0x6A` s8×3, else s32×3; *N*+1 entries (last is a trailer) |
| `V4-5` @2 | normal: 5 bits/axis, `(v − 15) / 15`; **bit 15 = strip restart** |
| `V4-5` @3 | optional prelit colour, present iff this write targets VU address 3 |
| UVs | cmd `0x6D` u16×4, `0x66` u8×2, else u16×2 |
| `MSCAL`/`MSCNT` | ends the batch |

Scale constants, read from the small-data area (`r2 = 0x8034D100`):
positions ÷128 (`r2−0x481c`), UVs ÷32768 (`r2−0x4830`) then ×256 (the
default texture transform `r2−0x4800` × `0x80127b68`), so ÷128 overall.
With `0x6D`, components 3–4 are the lightmap UV, ÷32768.

Strips split where a vertex has the restart flag and at least two vertices
precede it since the last split; the new strip starts at the previous
vertex.

## Not reverse engineered yet

- Header fields `0x6C`–`0x7E`; object `+0x08` flags and `+0x20..0x2C`.
- The descriptor's `+6` lightmap parameter.
- Binding fields other than the five above.
