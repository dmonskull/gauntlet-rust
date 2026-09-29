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
| `CRITTER/<boss>.WAD` | `SFXX`, `DAMG`, `DESC`, `ADDA`, `NODE`, `MOVE`, `PTRN`, `TYPE` — bosses' behaviour ([monsters.md](monsters.md)) |
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

## Player stats (`PDATA/<class>.WAD`, chunk `PDAT`)

One 384-byte record. Decoded so far (`pdata.rs`): four `(start, max)` f32
pairs — strength `+0x28`, speed `+0x30`, armour `+0x38`, magic `+0x40`.
The order is fixed by the class archetypes among the eight original
classes (Dwarf strongest and slowest, Wizard/Sorceress most magic,
Knight/Valkyrie most armour); values run 100–650 at start, up to 999.

The body measurements follow, copied into the player record when a player
joins (`FUN_80079ed8`, reading through the `PDAT` pointer at
`DAT_80282310[player]`); every class has the same values:

| offset | value | player field | meaning |
| --- | --- | --- | --- |
| `+0x48` | 5.0 | `+0x854` = × 0.5 | height (half is the collision half-height) |
| `+0x4C` | 1.5 | `+0x850` | collision radius |
| `+0x50` | 4.4 | `+0x83C` (Y of offset `+0x838`) | top point above the feet (`+0x54`) |
| `+0x54` | 2.5 | `+0x848` (Y of offset `+0x844`) | collision centre above the feet (`+0x64`) |

See [collision.md](collision.md) "Moving a player". `+0x58` (1.0–1.3) and
`+0x5C..+0x7C` are further per-class tuning, not named yet. The player movement code multiplies stick magnitude by a speed
value derived from this stat; the derivation isn't traced yet.
