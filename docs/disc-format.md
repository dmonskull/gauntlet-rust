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

## FST — the disc filesystem (at `fst_offset`)

Implemented in [`crates/gdl-formats/src/fst.rs`](../crates/gdl-formats/src/fst.rs).
Verified by reading all 2,481 game files off the disc and matching an
extracted copy byte-for-byte.

12-byte big-endian entries, then a NUL-terminated string table. Entry 0 is
the root directory; its third word is the total entry count.

| offset | file entry | directory entry |
| --- | --- | --- |
| 0x0 (u8) | 0 | 1 |
| 0x1 (u24) | name offset in string table | name offset |
| 0x4 (u32) | data offset on disc | parent entry index |
| 0x8 (u32) | data length | index one past the last child |

This disc's top level holds `Gauntlet/` (all game data), `carddemo/`,
`opening.bnr` and `check.txt`. The game builds paths relative to
`Gauntlet/` (`"levels/level%s"`, lowercase; lookups are case-insensitive).

## Disc magic and image containers

Images are identified by content, not extension
([`ImageKind::sniff`](../crates/gdl-formats/src/disc.rs)): a plain
`.iso`/`.gcm` has the GameCube magic `C2 33 9F 3D` at boot.bin offset 0x1C;
RVZ starts with `RVZ\x01`. WIA (`WIA\x01`), GCZ (`B10BC001` LE), CISO and
WBFS are recognised only to tell the user to convert them in Dolphin.

## RVZ (Dolphin's compressed image)

Implemented in [`crates/gdl-formats/src/rvz.rs`](../crates/gdl-formats/src/rvz.rs)
from Dolphin's public `docs/WiaAndRvz.md`, with the details that document
leaves open taken from Dolphin's reader (`Source/Core/DiscIO/WIABlob.cpp`,
`WIACompression.cpp`, `LaggedFibonacciGenerator.cpp`). Verified: the user's
`.rvz` reads back identical to the `.iso` of the same disc — all
1,459,978,240 bytes, and every one of the 2,485 FST files.

The user's file: version 1.0, GameCube (`disc_type` 1, no partitions),
Zstandard level 19, chunk size 0x20000 (128 KiB), one raw data entry
covering the whole disc, 11,139 groups — 1,423 stored uncompressed (Zstd
didn't help) and 1,704 with junk packing.

Layout, all big-endian:

- `0x00` `wia_file_head_t` (0x48 bytes): magic, version, compatible
  version, size of the disc struct, its SHA-1, `iso_file_size` (u64 @0x24),
  `wia_file_size` (u64 @0x2C), header SHA-1. The file size is checked (it
  catches truncated copies); hashes aren't.
- `0x48` `wia_disc_t`: `disc_type` @+0, `compression` @+4 (0 none, 1 purge,
  2 bzip2, 3 LZMA, 4 LZMA2, 5 Zstd), level @+8 (signed), `chunk_size` @+0xC,
  the disc's first 0x80 bytes @+0x10, partition count @+0x90, raw data
  count/offset/stored size @+0xB4/+0xB8/+0xC0, group count/offset/stored size
  @+0xC4/+0xC8/+0xD0.
- Raw data table (compressed as one Zstd frame): 0x18-byte entries — disc
  offset u64, size u64, first group u32, group count u32. The first entry
  says offset 0x80, but its groups start at 0: round the offset down to
  0x8000 and grow the size to match.
- Group table (one Zstd frame): 12-byte `rvz_group_t` — file offset / 4;
  stored size (top bit set = compressed with the file's method, clear =
  stored as is; 0 = the whole group is zero); `rvz_packed_size` (0 = not
  packed). Each group holds `chunk_size` disc bytes, the last one of an
  entry fewer.
- Packing, applied before compression: a run of `u32 size` headers; top
  bit clear = `size` literal bytes follow, set = a 68-byte seed follows and
  `size & 0x7FFFFFFF` bytes of junk are regenerated from it.

Junk is the padding the mastering tool put between files: a lagged
Fibonacci generator (xor, j = 32, k = 521). Seed 17 BE words, extend to 521
with `b[i] = b[i-17] << 23 ^ b[i-16] >> 9 ^ b[i-1]`, then "advance" 4 times
(`b[i] ^= b[i+489]` for i < 32, `b[i] ^= b[i-32]` for the rest), and output
each word as bytes `x>>24, x>>18, x>>8, x` (18, not 16), advancing again
after every 521 words. The stream is aligned to 0x8000-byte disc blocks: a
run starting at disc offset `o` first discards `o % 0x8000` bytes, then
simply continues. Known-answer tests against Dolphin's generator are in
`rvz.rs`.

Reading decompresses only the groups a read touches and keeps the last 4.
Zstandard decoding uses the pure-Rust `ruzstd` crate (already a Bevy
dependency). Not supported: bzip2/LZMA/LZMA2 RVZs, WIA, and Wii discs —
all refused with a message pointing at Dolphin's *Convert File…*.
