# `textures.ngc` — texture pixel data

Implemented in [`crates/gdl-formats/src/texture.rs`](../crates/gdl-formats/src/texture.rs).
Verified: 8,315 of the disc's textured bindings decode (72 use selectors the
game's own tables don't cover — see below).

No file header. Each [model binding](objects-ngc-format.md#texture-binding-0x40-bytes-on-disk)
gives an offset, width, height and a format selector byte. Data is in
GameCube-native form: GX tiled layout, big-endian.

## Format selector

Decoded exactly as `FUN_800c70c4` does when it builds GX texture objects.
The selector names **PS2 pixel-storage modes** (`PSMT4 = 0x14`, `PSMT8 =
0x13`, `PSMCT16 = 0x02`) through three 5-entry tables at `0x80127bb0`
(format), `0x80127bc4` (palette bytes to skip) and `0x80127bd8` (direct
formats), which the game maps to GX formats:

| selector | format | palette | pixels start at |
| --- | --- | --- | --- |
| `0x0?` (low 3 bits 0–1) | GX `RGB5A3` | — | offset |
| `0x1?` | GX `CI4` | 16 × RGB5A3 | offset + 32 |
| `0x2?` | GX `CI4` | 16 × RGB5A3 | offset + 64 |
| `0x3?` | GX `CI8` | 256 × RGB5A3 | offset + 512 |
| `0x4?` | GX `CI8` | 256 × RGB5A3 | offset + 1024 |
| `0x8?` | GX `CI8` | shared (below) | offset |
| `0x9?`–`0xF?` | GX `CI4` | shared (below) | offset |

Selectors `0x5?`–`0x7?` index past the end of the game's 5-entry tables, and
direct formats other than PSMCT16 leave the texel size uninitialised — the
retail game can't draw these correctly either, so they're reported as
unsupported. All textures wrap (repeat) and have no mipmaps.

## Shared palettes

`FUN_800c76dc` builds two global palettes at runtime instead of reading
them: RGB5A3 `(i × 0x800) | 0xFFF` (16 entries) and `(i × 0x80) | 0xFFF`
(256 entries) — white with a 3-bit alpha ramp. Lightmaps (256×256 `0x92`
textures) use these, so their intensity lives in alpha, in 8 levels.

## Tiling

GX tiles are row-major, texels row-major within a tile, dimensions padded to
whole tiles: `CI4` 8×8, `CI8` 8×4, `RGB5A3` 4×4. `CI4` stores the high nibble
first. RGB5A3: top bit set → opaque RGB555, clear → ARGB 3-4-4-4.
