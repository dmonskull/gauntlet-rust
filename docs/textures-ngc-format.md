# `textures.ngc` — status: not yet reverse engineered

No file-level header found: real files (e.g. `levelE2/textures.ngc`) have
no ASCII path comment and nothing that looks like a version magic in the
first `0x40` bytes, unlike `objects.ngc`. See
[objects-ngc-format.md](objects-ngc-format.md) for how `objects.ngc`'s
`MaterialBinding::texture_offset` was confirmed (via 8,387 real bindings
across every level) to be a valid file-relative offset into the sibling
`textures.ngc`.

## What looking at real offsets shows

Dumping the bytes at every confirmed `texture_offset` in `levelE2` (128
distinct offsets) shows:

- Several offsets start with a repeating `c2 10 c2 10 ...` byte pattern —
  plausibly a single solid-color GX `CMPR` block tiled across a whole
  texture (a `CMPR` block is 8 bytes: two RGB565 colors + 4×4 2-bit
  indices; a flat color naturally repeats).
- The gaps between consecutive sorted offsets (a rough proxy for one
  texture's size, since textures are presumably packed back-to-back) take
  only a handful of distinct values: `0x40`, `0x300`, `0x1200`, `0x2200`,
  `0x4200` bytes. Consistent round sizes, but not yet matched to specific
  width/height/GX-format combinations — `0x40` bytes is too small for any
  plausible `CMPR` texture above 8×16, so more than one GX format is likely
  in use (`CMPR`, `I4`/`I8`, `RGB565`, `RGB5A3` are all real possibilities
  for a GC game of this era).

## What's missing

Width, height and GX texture format aren't visible in `textures.ngc`
itself, and haven't been found in the confirmed part of `objects.ngc`'s
`MaterialBinding` either — they're probably in `MaterialBinding`'s
remaining ~13 unconfirmed 4-byte words, or built from a lookup elsewhere.

## Next step

Find where the game actually constructs a GX texture object (the
Nintendo SDK's `GXInitTexObj`-equivalent call) from a resolved
`MaterialBinding`/texture pointer — that call's arguments give width,
height and format directly. Not yet located; the two functions found so
far that touch resolved texture pointers (`FUN_800ba1ac`, `FUN_800ba278`)
only copy the raw `0x10`-byte binding record around (texture-scroll
animation), they don't build a texture object.
