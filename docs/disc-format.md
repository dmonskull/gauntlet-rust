# GameCube disc boot header and DOL layout

Confirmed against Gauntlet: Dark Legacy's own disc image. Implemented in
[`crates/gdl-formats/src/disc.rs`](../crates/gdl-formats/src/disc.rs).

## boot.bin (offset 0x0, 0x440 bytes)

| offset | field | notes |
| --- | --- | --- |
| 0x00 | game ID | 6 ASCII bytes; `GUNE5D` for this disc |
| 0x06 | maker code | 2 ASCII bytes; empty on this disc |
| 0x18 | disc number | |
| 0x19 | disc version | |
| 0x20 | title | NUL-terminated ASCII, up to 0x40 bytes; `"Gauntlet - Dark Legacy"` |
| 0x420 | `dol_offset` | u32 BE; `0x1DA00` on this disc |
| 0x424 | `fst_offset` | u32 BE; `0x25B200` |
| 0x428 | `fst_size` | u32 BE; `0x11872` |
| 0x42C | `fst_max_size` | u32 BE |

## main.dol (at `dol_offset`, 0x100-byte header)

7 `.text` and 11 `.data` sections, each independently placed in memory —
there is no single stored file size; it's the max of every section's
`offset + size`. For this disc: offset `0x1DA00`, size `0x23D7C0`
(2,348,992 bytes), entry point `0x800051fc`.

| offset | field |
| --- | --- |
| 0x00 | 7× u32 BE text section file offsets |
| 0x1C | 11× u32 BE data section file offsets |
| 0x48 | 7× u32 BE text section memory addresses |
| 0x64 | 11× u32 BE data section memory addresses |
| 0x90 | 7× u32 BE text section sizes |
| 0xAC | 11× u32 BE data section sizes |
| 0xD8 | BSS memory address |
| 0xDC | BSS size |
| 0xE0 | entry point |

Image base for the whole disc is `0x80000000`, standard for GameCube.
