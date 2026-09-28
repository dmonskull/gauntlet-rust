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

## Not yet reverse engineered

- What `num_b`'s compacted `0x10`-byte entries actually hold (material name?
  texture index + UV info?).
- The exact meaning of the `d_offset` array (`0x24` bytes/entry) — likely
  collision or trigger geometry given the trailing-empty-entry trimming.
- The unnamed pointers at `0x64`–`0x78`.
- Vertex/index/strip data layout inside the `objects_offset` entries once
  `entry+0xc` submesh count is followed.

Next step to make progress here: decompile `0x800b7fec`/`0x800b719c`'s
caller chain forward into whatever actually walks `objects_offset` entries
for rendering (not just the fixup routine), to name the remaining fields.
