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
| `LIST` / `LOFF` / `LDEF` | named lists of groups: `LIST` has 8-byte records `{u32 count, u32 first}` into `LOFF`, a table of u32 group indices; `LDEF` holds each list's name (u32 offsets into the `DEFS` pool) |
| `FONT` | 20-byte records `{name[16], u32 slot}`: the fonts a group's `font` number indexes; `slot` is 0 on disc and filled by the loader |

The lists are not menus. `ENGLISH.ROM` has four: `CLASS_RANK` (the 16
`*_RANK` groups in class order), `CLASS_TURBO` (8 `*_TURBO`),
`LEGEND_ITEMS` (11 `LEGEND_ITEMS0nn`) and `CONTROLS_DESC` (`CONTROLS1..4`);
`HINTS_E.ROM` has four more, `SCROLL_E.ROM` none. The menus themselves are
tables in `main.dol` ([frontend.md](frontend.md)).

`FONT` names are matched to the game's 13 font slots with a plain,
case-sensitive `strcmp` (`FUN_800e76a4`) at the end of the text ROM loader
`FUN_8001ffa8`, over the slot names at `0x80118538`; a name that matches nothing leaves
slot 0. `ENGLISH.ROM`'s fonts are `8Hi_fonts5` (matches nothing → slot 0,
`font8x8`), `font32` (slot 6) and `initials` (slot 7). Fonts themselves:
[frontend.md](frontend.md).

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

**The heroes' attack records** (found 2026-10-01, `tools/hdamg.py <class>`
dumps them). `+0x00` / `+0x02` are the file's `SFXX` and `DAMG` counts
(WAR: 15, 14); after loading, `+0x04` / `+0x08` point at those tables
(`DAMG` records here are `0x58` bytes, not the critters' `0x50`). The i16s
at `+0x0C`–`+0x22` (not sound indices, as once noted) are `DAMG` indices for
the actions the player update hits with through `FUN_80088b88` (−1 none):

| `PDAT` | action | WAR |
| --- | --- | --- |
| `+0x0C` | ATTPWRACLOSE `0x23` | 0 |
| `+0x0E` | ATTPWRALOW `0x54` | 1 |
| `+0x10` | ATTPWRAMED `0x25` | 2 |
| `+0x12` | ATT360 `0x3C` | 3 |
| `+0x14` | ATTPWRATHROW `0x63` | 4 |
| `+0x16` | ATTPWRB `0x56` (turbo) | 5 |
| `+0x18`, `+0x1A` | ATTPWRC `0x57` (full turbo; two records) | 6, 7 |
| `+0x1C` | COMBOACT1 `0x58` (co-op combo) | 8 |
| `+0x1E` | COMBOACT3 `0x5A` | −1 |
| `+0x20` | ? | 12 |
| `+0x22` | action `0x7B` | 13 |

A hero `DAMG` record as `FUN_80088b88` (record, the action's time now and
last tick) reads it — the rest isn't traced:
`+0x00` i16 kind (0 and 5 nothing; 2, 3, 4 and others a hit through
`FUN_80089114`, its third argument 0 for kind 2; 10 an area through
`FUN_80030094`, stepped `+0x14` apart along the facing out to `+0x20`,
scaled by `+0x2C/+0x30/+0x34`, effects' damage × `+0x44`); `+0x02` flags
(`0x2000`, `0x20`, `0x10`: a screen shake of three strengths,
`FUN_80067acc`, on both linked heroes; `0x400` a node glow while it runs;
`0x80` no lasting phase; `0x300` the reach grows (or shrinks, `0x200`) over
the run; `0x1000` stops the slots it hits); `+0x38` f32 damage (WAR's turbo
B 50, its combo 10) — a record with damage also pays the pending turbo cost
(`+0x910`) as it lands; `+0x50` i16 start frame and `+0x52` end frame (−1:
one frame); `+0x54` i16 a hint raised as it starts (`FUN_800a4268`).
The ported turbo attacks still land as finishers (a stand-in: their real
blows are these records).
value derived from this stat; the derivation isn't traced yet.
