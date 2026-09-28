# `objects.ngc` — model file header

Status: **header confirmed**, per-entry array layouts are hypotheses. Found by
decompiling the fixup routine at `0x800b7534` in `main.dol` (Ghidra project
`GauntletDarkLegacy`), which every level's `objects.ngc` passes through right
after loading — see `docs/INDEX.md` for how to re-run that decompile.

## Byte order

The whole file is **little-endian on disk** (shared with the PS2 build,
which is little-endian MIPS). `0x800b7534` byte-swaps every multi-byte header
field and every per-entry array element after load, once it confirms the
version field turns into the expected magic after exactly one 32-bit swap.
Confirmed directly against real files: `levelE2/objects.ngc` offset `0x40` is
literally the bytes `0d 00 0b f0`, i.e. `0xF00B000D` read little-endian.

## Header layout (offset from file start)

Bytes `0x00..0x20` are a NUL-terminated ASCII path comment (build-machine
path, e.g. `"...jects/GPS2/Disk/levels/levelE2/"`), not used by the game.
`0x20..0x40` is zero-padded. The real header starts at `0x40`:

| offset | size | field | notes |
| --- | --- | --- | --- |
| 0x40 | u32 | `version` | magic. Observed `0xF00B000D` (most levels) and `0xF00B000C` (`levelL2`) — treat as "known versions", not a single hardcoded constant |
| 0x44 | u32 | `num_objects` | always observed equal to the field at `0x4c` across every level checked |
| 0x48 | u32 | `num_b` | drives a second `0x40`-byte-entry array at `objects_b_offset`, later compacted **in place** down to `0x10` bytes/entry (material→texture remap table?) |
| 0x4c | u32 | `num_objects_dup` | see `0x44`; used as the count for the array at `bounds_offset` (`0x18` bytes/entry) |
| 0x50 | u32 | `num_d` | array at `d_offset`, `0x24` bytes/entry; trailing entries whose first byte is `0` get trimmed from the count after byte-swapping |
| 0x54 | u32→ptr | `objects_offset` | array of `num_objects` entries, `0x40` bytes each — the main per-object/mesh record |
| 0x58 | u32→ptr | `objects_b_offset` | array of `num_b` entries, `0x40` bytes each on disk, compacted to `0x10` bytes/entry after load |
| 0x5c | u32→ptr | `bounds_offset` | array of `num_objects` entries, `0x18` bytes each; entry `+0x14` (u16) indexes back into the `objects_offset` array (sets that object's `+0x2c` back-pointer to this entry) |
| 0x60 | u32→ptr | `d_offset` | array of `num_d` entries, `0x24` bytes each |
| 0x64 | u32→ptr | unnamed | relocated the same way (`+= file_base`) |
| 0x68 | u32→ptr | unnamed | relocated the same way |
| 0x6c | u32→ptr | unnamed | relocated the same way |
| 0x70 | u32→ptr | unnamed | relocated from a *different* base: `file_base + *(u32*)(header_table + entry*0x10 + 8)`, not from this header directly |
| 0x74 | u32→ptr | unnamed | relocated from `file_base + that_table_entry[2] + that_table_entry[3]` |
| 0x78 | u32→ptr | unnamed | relocated the same way as `0x64..0x6c` |
| 0x7c | u16 | unnamed | byte-swapped only, not relocated |
| 0x7e | u16 | unnamed | byte-swapped only, not relocated |

All `offset` fields are file-relative on disk and become absolute pointers
(`+= file_base`) once loaded into memory — for our purposes (reading from a
file, not emulating GC memory), they stay file-relative offsets.

## Per-`objects_offset` entry validation (confirmed from `0x800b7534`)

An entry is considered invalid (zeroes out its "count" field at `+0xc`) unless
**all** of:
- `*(i32*)(entry+0xc) >= 1`
- `*(i16*)(entry+0x10) != 0`
- `*(i32*)(entry+0x1c) != 0`
- if `*(i32*)(entry+0xc) > 1`, also `*(i32*)(entry+0x18) != 0`

This strongly suggests `entry+0xc` is a sub-mesh/strip count, `entry+0x10` a
flags or material-index field, and `entry+0x18`/`entry+0x1c` are pointers to
per-strip data present only when the count allows it.

## `objects_b_offset` array — material/texture bindings (partially confirmed)

Confirmed from `FUN_800c7510` (`main.dol`), called right after both a
model *and* its texture finish loading (from `FUN_800b7344`). It walks
`num_b` entries at the *compacted* `objects_b_offset` array and binds each
to texture data. Cross-referencing the compaction copy in `0x800b7534`
(which shrinks each on-disk `0x40`-byte entry down to `0x10` bytes) resolves
which on-disk bytes back each compacted field:

| on-disk offset (within 0x40-byte entry) | compacted offset | meaning |
| --- | --- | --- |
| `+0x00` (u8) | `+0x00` | copied through, meaning unconfirmed |
| `+0x08` (u16) | `+0x02` | becomes the runtime "flags" field; bit `0x100` = untextured |
| `+0x0C` (u32) | `+0x04` | texture data offset, used only when textured |
| `+0x12` (u16) | `+0x08` | copied through, meaning unconfirmed |
| `+0x16` (u16) | `+0x0A` | if `0`, this binding is marked untextured |
| `+0x18` (u16) | `+0x0C` | if `0` (or already untextured), also marked untextured |

Compacted `+0x0E` and `+0x01` are **not** written by the compaction copy —
they're leftover memory, unconditionally overwritten at runtime (`0xFFFF`
and a model index respectively), so they carry no on-disk meaning. The rest
of each `0x40`-byte on-disk entry (most of it) is still unconfirmed — likely
a material name and additional texture/UV info, given the array's job.

Cross-checked against the `texidx` lookup path (`FUN_800ba314`
`"MBRomTexPtr"`, `FUN_800ba278`, `FUN_800ba1ac`): all three independently
index this exact same array (`model_base + 0x58`, stride `0x10`, by the
low 16 bits of a `texidx`), confirming it's the canonical texture-binding
table used throughout the game, not something local to the one fixup
routine.

Implemented as `gdl_formats::MaterialBinding` — `flags_raw` (`+0x08`),
`texture_offset` (`+0x0C`), and the two "untextured" check fields
(`+0x16`/`+0x18`). **Verified**: across all 67 real `objects.ngc` files,
every one of the 8,387 bindings whose checks mark it "textured" has a
`texture_offset` that lands inside its level's actual `textures.ngc` file
size (`cargo test -p gdl-formats textured_bindings_point_inside`) — strong
evidence `texture_offset` really is a file-relative offset into
`textures.ngc`.

## Not yet reverse engineered

- Most of the `objects_b_offset` entry (only 4 of ~16 four-byte words
  confirmed — see above).
- The exact meaning of the `d_offset` array (`0x24` bytes/entry) — likely
  collision or trigger geometry given the trailing-empty-entry trimming.
- The unnamed pointers at `0x64`–`0x78`.
- Vertex/index/strip data layout inside the `objects_offset` entries once
  `entry+0xc` submesh count is followed.

`textures.ngc` does **not** appear to share this pattern: real files (e.g.
`levelE2/textures.ngc`) have no ASCII path comment and no obvious version
magic in the first `0x40` bytes — byte patterns from `0x40` onward already
look like GX `CMPR`-style compressed texture block data (repeating
plausible RGB565 colour shorts), suggesting little or no file-level header.
Not yet confirmed against decompiled code — the `objects.ngc`/`textures.ngc`
load path in `FUN_800b7344` doesn't call an equivalent byte-swap routine for
textures, which would make sense if GX-compressed texel data is stored as
opaque byte blocks that don't need endian conversion.

Next step to make progress on `objects.ngc`: decompile `0x800b7fec`/
`0x800b719c`'s caller chain forward into whatever actually walks
`objects_offset` entries for rendering, to name the remaining fields. For
`textures.ngc`: find the function that actually builds a GX `GXTexObj` from
the loaded data (search for code indexing off the texture pointer stored at
`objects.ngc`'s material-binding `+0x04` field) to confirm the container
format (TPL-style multi-image table vs. one texture per file).
