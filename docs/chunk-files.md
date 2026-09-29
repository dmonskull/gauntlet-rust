# Chunk files: `.WAD` and `.ROM`

Implemented in [`chunk.rs`](../crates/gdl-formats/src/chunk.rs) and
[`text.rs`](../crates/gdl-formats/src/text.rs). Verified: all 55 `.WAD`
and text `.ROM` files on the disc parse (`AUDIO/AUDATPS2.ROM` is audio, a
different format); all 5 text ROMs yield 1,887 strings.

## Container

The game finds records by tag with `FUN_800becec(file, tag, &count)`.

| offset | field |
| --- | --- |
| 0x0 | directory offset |
| 0x4 | chunk count |
| directory | count × `{tag[4], u32 offset, u32 count, u32 count}` |

Tags are stored byte-reversed: the game builds them as big-endian u32
constants (`r2` constants assembled byte by byte in the loaders) and the
files are little-endian, so `YMNE` on disk is `ENMY`. Chunk sizes aren't
stored; a chunk runs to the next chunk's offset or the directory.

Tags seen on the disc:

| file | chunks |
| --- | --- |
| `WDATA/<realm>.WAD` | `ENMY`, `BCAM`, `CAMS`, `SNDS`, `AUDS`, `MAPS`, `LEVL`, `WRLD` — per-level tables (6 entries for the 6 castle levels) |
| `PDATA/<class>.WAD` | `SFXX`, `DAMG`, `PDAT` (player stats; loaded by `FUN_8008a160`) |
| `CRITTER/<monster>.WAD` | `SFXX`, `DAMG`, `DESC`, `ADDA`, `NODE`, `MOVE`, `PTRN`, `TYPE` |
| `SHPDATA/SHOP.WAD` | `ITEM` |
| `TEXT/*.ROM` | `FONT`, `TEXT`, `TOFF`, `STRS`, `LOFF`, `LIST`, `DEFS`, `SDEF`, `LDEF` |

## Text (`TEXT/*.ROM`, loaded as `"%s_e.rom"` by `FUN_8001ffa8`)

| chunk | contents |
| --- | --- |
| `TEXT` | pool of NUL-terminated strings |
| `TOFF` | u32 offsets into `TEXT` |
| `STRS` | 20-byte group records: `u32 count, u32 first string, u32 font, f32 scale x, f32 scale y` |
| `DEFS` / `SDEF` | group names: pool + u32 offsets (`PLAYER_CLASS`, `ARC_RANK`, `DRIDER_SPEECH`, …) |
| `LIST` / `LOFF` / `LDEF` | named lists of groups (menus); 8-byte records — not decoded yet |
| `FONT` | 20-byte font entries (`8Hi_fonts5`, `font32`, …) |

`ENGLISH.ROM` is the GameCube text, `PS2ENGLISH.ROM`/`BAKENGLISH.ROM` are
leftovers; `HINTS_E.ROM` and `SCROLL_E.ROM` hold hints and scroll text.
