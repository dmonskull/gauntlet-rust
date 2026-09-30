# Audio: sound banks, the sound catalog, music streams

Everything audible on the disc is Nintendo DSP-ADPCM, played by the audio
DSP through the SDK's AX voice library. The game's own code is a port of
Midway's PS2 "DCS" sound driver: the game side sends numbered commands to a
dispatcher that, on PS2, was an IOP server (`FUN_800d4e9c`: 4 = load bank,
8 = open stream, 10 = play stream, 0xC = stop stream, 0x11 = batched voice
commands). File names and debug strings still say PS2 (`AUDATPS2.ROM`,
`"Bank type %d.%03d made with VAG..."`, `VAGp` headers), but every sample
is GameCube ADPCM.

Implemented in `crates/gdl-formats/src/audio/` (`dsp.rs`, `bank.rs`,
`stream.rs`, `catalog.rs`) and `crates/gdl-formats/src/world_data.rs`;
played by `crates/gdl-game/src/audio.rs`.

| file | what | endianness |
| --- | --- | --- |
| `AUDIO/AUDATPS2.ROM` | catalog: modes → groups → banks → named sounds | little |
| `AUDIO/*.VBK` (65) | sound banks: calls + DSP-ADPCM samples | big |
| `STREAMS/*.ads` (111) | music: interleaved DSP-ADPCM | big |
| `WDATA/*.WAD` | per-realm world data; the `AUDS` chunk picks each level's bank and music | little |

Other files in `AUDIO/` (`ConvertVAG.exe`, `sound3.dsp`/`.txt`,
`temp.txt`, `siren.adpcm`, `*.pcm8`, `*.pcm16`, `SOUND3.WAV`) are tooling
leftovers the game never opens. `sound3.txt` is the text dump Nintendo's
`DSPADPCM` tool writes, and `sound3.dsp` is the matching 0x60-byte header —
the same header the banks and streams use.

## Codec: DSP-ADPCM

Evidence that it's ADPCM on the DSP and not something decoded on the CPU:

- `FUN_800d36a8` (start a bank sample on a voice) calls `FUN_800ef270`
  with the sample's 16 coefficients + gain + predictor/scale + history, and
  `FUN_800ef160` with format 0, loop flag 0. These two are the SDK's
  `AXSetVoiceAdpcm` (copies exactly that 0x28-byte block into the voice)
  and `AXSetVoiceAddr` (format 10 = PCM16 and 0x19 = PCM8 get the fixed
  gains 0x0800/0x0100; 0 = ADPCM keeps the coefficients). The retail binary
  also has the `"DSPInit(): Build Date: %s %s"` SDK string.
- Nothing in `main.dol` decodes ADPCM; the frames go to ARAM untouched.

So the decoder is the hardware's: 8-byte frames, first byte
`predictor << 4 | scale_exponent`, then 14 signed 4-bit residuals;
`s = ((r << scale) << 11 + 1024 + c1·s[-1] + c2·s[-2]) >> 11`, clamped to
i16, with `(c1, c2)` = coefficient pair `predictor`. Verified on the data:
every decoded bank sample and stream is smooth (mean |step| / RMS ≈ 0.07–0.4
for music; random coefficients give 0.8+), frame-boundary steps match
in-frame steps (ratio 1.00), and lengths match the headers exactly.

### The 0x60-byte DSP header (big-endian)

| off | type | field |
| --- | --- | --- |
| 0x00 | u32 | samples |
| 0x04 | u32 | nibbles |
| 0x08 | u32 | sample rate |
| 0x0C | u16 | loop flag (0 everywhere) |
| 0x0E | u16 | format (0 = ADPCM) |
| 0x10 | u32 | loop start |
| 0x14 | u32 | loop end (the tool wrote the sample count here) |
| 0x18 | u32 | current address (2) |
| 0x1C | 16 × i16 | coefficients |
| 0x3C | u16 | gain |
| 0x3E | u16 | initial predictor/scale |
| 0x40 | i16 ×2 | history 1, 2 |
| 0x44 | u16, i16 ×2 | loop predictor/scale, loop history |

## Sound banks: `AUDIO/*.VBK`

Loaded by `FUN_800d2b50`: header `FUN_800d40e4`, call list `FUN_800d2fdc`,
samples `FUN_800d2d5c` → `FUN_800d441c`.

```text
0x00  "KNBV"   ("VBNK" = byte-swapped variant, not on the disc)
0x04  u32 call-list size in bytes
0x08  u32 version: low 16 bits 0x106 (or 0x100, not on the disc)
0x0C  u32 calls (must be < 0x500)
0x10  u32 samples           (0x106 only)
0x14  call list
      samples back to back:
        0x30 "VAGp" header: +0x04 u32 0x28 (= DSP header follows),
             +0x0C u32 data bytes, +0x10 u32 sample rate, +0x20 name[16]
        0x60 DSP header
        ADPCM data
```

- `FUN_800d40e4` compares the magic against `r2-0x44b0` "VBNK" / `r2-0x44a8`
  "KNBV" (the first means swap), requires calls < 0x500, and version 0x106
  (reads the extra sample-count word) or 0x100 (16-byte calls; size must be
  calls × 16).
- `FUN_800d441c` accepts "pGAV" (`r2-0x4498`, swap) or "VAGp" (`r2-0x4490`);
  takes the data size from +0x0C, sets the pitch from +0x10
  (`rate << 12 / 48000`), and when +0x04 == 0x28 reads the 0x60-byte DSP
  header and keeps its coefficients and +0x3E predictor/scale.
- VAG rate and DSP-header rate agree on all 2,044 samples. Rates on disc:
  2000–44100 Hz, mostly 12000 and 18000.

### Calls

The game plays *calls*, not samples. The 0x106 call list is a u16 stream:

```text
step*  : u16  bits 0-11 sample index, 0x4000 loop start, 0x2000 loop back,
              0x8000 last step
then   : u16 volume (0..0x7F), u16 duck, u16 priority
```

- `FUN_800d2fdc` walks each call to the step with bit 15 set and skips 3
  more words; errors if the walk doesn't end exactly at the list size
  (`"BANK call list size mismatches"`). `FUN_800d33a4` relocates the 12-bit
  sample indices when a bank's samples land in the global sample table.
- `FUN_800d2698` (play) skips to the last step and reads volume
  (`requested × vol / 0x7F`), duck (subtracted from every other voice's
  volume while it plays) and priority (voice stealing, `FUN_800d350c`):
  the voice keeps `requested priority << 16 | call priority`, the
  requested one being the third word of the play command (each caller's
  own: 2 for the announcer's queued lines, `0x42`–`0x6E` for the heroes',
  10 for the tower's chimes). With no free voice (no call, no duck, the AX
  voice stopped; searched round-robin from `r13-0x68c0`) the new sound
  takes the voice whose key is the highest met so far on that walk while
  still ≤ its requested priority — in effect only voices started at
  priority 0 — and otherwise isn't played (the reply is −2).
  `docs/frontend.md`, "The voice queues".
- `FUN_800d36a8` plays the current step's sample, then: `0x2000` → back to
  the nearest `0x4000` step at or before it; else `0x8000` → stop; else next
  step. Samples themselves never hardware-loop (voice loop flag 0, start
  nibble `aram·2+2`, end `(aram+size)·2−1`).

On disc: 65 banks, 2,247 calls (181 loop), 2,044 samples, ~66 minutes.
Most calls are one step; `0xE000` is a single looping sample. The catalog
length field (below) equals the sum of a call's samples for every
non-looping call, which also confirms multi-step calls play in sequence.

## Sound catalog: `AUDIO/AUDATPS2.ROM`

Loaded and byte-swapped by `FUN_800169e0` (`"audio"`, `"audatps2.rom"`).

```text
0x00  u32 modes, u32 banks, u32 sounds
0x0C  u32 offsets from file start: modes, banks, sounds
mode  0x2494: name[16], u32 groups, 32 × group
group 0x124:  name[16], u32, u32, u32 bank count, 64 × u32 bank index,
              u32, u32 (runtime)
bank  0x2C:   file[16], name[16], u32 (size, PS2?), u16 sound count,
              u16 first sound, u16 slot, u16 handle (runtime)
sound 0x1C:   name[16], u32 id = bank << 16 | call, f32 length,
              f32 start (runtime, fields)
```

- `FUN_800168bc` finds a mode by name (`"AUDIO: UNABLE TO FIND MODE %s"`)
  and loads its groups; `FUN_80016ee8` loads one named bank of a named
  group via `FUN_800171e0`, which opens `audio/<file>.vbk` (`r2-0x7d90`
  "audio", `r2-0x7d88` "%s.vbk").
- `FUN_8001801c` finds a sound: for each bank, for its sounds, `strncmp` of
  15 characters; returns the id (`"UNABLE TO FIND SOUND: %s"`).
- `FUN_80015cac` plays an id: bank = `id >> 16`, call handle = bank's
  handle + `(id & 0xFFF)` (`"AUDIO: BANK %s NOT LOADED. SOUND:%s"`).
- `FUN_80017fe8` reads the f32; it's the call length in seconds (1/640 s
  resolution) or −1 for looping calls — matches all 2,228 sounds. The
  voice queues hold a line for it × 60 fields, and a started sound counts
  as playing for as long (`docs/frontend.md`, "The voice queues").
- `+0x18` is written when a line is queued (when it will start) and when
  a sound starts (`FUN_80015f24`: now); nothing reads it.

Retail catalog: one mode `ALL` with groups COMMON, VOICE1, VOICE2,
PLAYER1–4 (the 8 character banks), LEVELS (48 banks); 60 banks, 2,228
sounds. Every bank's sound count equals its `.VBK` call count.

## Music: `STREAMS/*.ads`

Opened by `FUN_800174b8` (`"streams"` + name, `"Audio Stream bad file"`),
header read by `FUN_800d7948` (0x28 bytes) → `FUN_800d7de0`, voices set up
by `FUN_800d71d4`, data de-interleaved into per-channel ARAM buffers by
`FUN_800d6830`.

```text
0x00  "dhSS"          ("SShd" = swapped variant)
0x04  u32 0x18
0x08  u32 codec: 0x20 = DSP-ADPCM; anything else plays as PCM16 (none on disc)
0x0C  u32 sample rate
0x10  u32 channels: must be 1 or 2
0x14  u32 interleave (0x20 on every file)
0x18  u32 ×2 0xFFFFFFFF
0x20  "dbSS", u32 data bytes (all channels)
0x28  channels × 0x60 DSP header   (read only when codec == 0x20)
      data: interleave bytes of ch0, ch1, ch0, ch1, …
```

- `FUN_800d7de0` checks "dhSS"/"dbSS" (`r2-0x4458`/`-0x4450`) or the swapped
  forms, rejects channel counts other than 1/2, and reads
  `channels × 0xC0 / 2` bytes of DSP headers when codec == 0x20.
- `FUN_800d71d4`: codec 0x20 → `AXSetVoiceAdpcm` with each channel's
  header, format 0; otherwise format 10 (PCM16).
- `FUN_800d6830` copies `interleave` (stream `+0x68` = header `+0x14`) bytes
  per channel in turn. `FUN_800d7948` requires the per-channel buffer to be
  a multiple of it.
- Looping (`FUN_800d6ae4`): at end of file with the loop flag set, seek back
  to `0x28 + channels × 0x60` and keep streaming.
- All 111 files: data size = file size − header; every channel's data is
  padded to the interleave; per-channel sample counts in the DSP headers
  match the data exactly. 80 stereo, 31 mono; rates 12000, 18000, 24000
  (most), 44100 (`tower`, `DREAM6C`). ~173 minutes.

## Which music a level plays

`WDATA/<realm>.WAD` (loaded by `FUN_80058074`) is a chunk file:

```text
0x00  u32 directory offset, u32 chunks
dir   16 bytes each: u32 tag, u32 offset, u32 count, u32
```

`FUN_800bed78` swaps the directory, `FUN_800becec` finds a chunk by tag
(tags at `r2-0x6a68`: WRLD LEVL ENMY CAMS AUDS SNDS MAPS BCAM).

- `LEVL`: 0x10C bytes per level. `+0x08` is the level name; the level folder
  is `"levels\level%s"` of it (`FUN_8005638c`). `FUN_80059cb0` turns
  `+0x5A` (i16, negative → 0) into a pointer to `AUDS + index × 0x3C`
  stored at `+0x64` (`"World Data %s has no audio"`).
- `AUDS`: 0x3C bytes. `+0x00` bank name — `FUN_800a0a18` loads it from the
  catalog's `LEVELS` group (also each player's character bank, `VOICE2`, and
  `TOWAMB` in the town realm). `+0x18` stream name, `+0x28` i16 tracks,
  `+0x2A` i16 flag passed to the stream start: 0 (or a mono console
  setting) plays a stereo stream mixed to the centre, 1 keeps the channels
  apart (`FUN_800d708c`); it isn't a channel count (`sky2` has 1 with mono
  files). `+0x2C` 8 × i16 parts per track. `+0x10..0x18` unknown.

`FUN_800a0b38` builds the stream name (`r2-0x5288` "%s", `-0x5240` "%s%c",
`-0x5238` "%s_%d", `-0x5284` ".ads"):

```text
name = stream                     if tracks == 1
       stream + 'a' + track       otherwise
name += "_" + (part + 1)          if parts[track] >= 2
```

e.g. `castle1.ads`; `castle2` → `CASTLE2A`/`CASTLE2B`; `castle6` →
`castle6_1`, `castle6_2`; `desert2` → `DESERT2A`, `desert2b_1..`,
`DESERT2C`. It checks the file exists (`"Audio stream does not exist"`).
Level start (`FUN_800a097c`) sets track 0, part 0. When a part ends,
`FUN_80017658` advances the part; the last part is started with the loop
flag (single-track) or replayed when it ends (multi-track), so the last
part loops. Gameplay switches tracks (the track index at `r13-0x720c`) —
not traced yet; the runtime stays on track 0.

Every retail level's streams exist. Only the leftover `TEST.WAD` realm
names missing ones (its record describes an older `dream1`).

## Positional sounds

The game's sound calls go through a few wrappers of `FUN_80015cac(−1, id,
volume, pos, pan, priority)`: `FUN_800157ec(id, volume, priority)` plays
centred; the rest don't play while `r13-0x7380 & 0x8000` is set (unless
`r13-0x7854`): `FUN_80015a30(id, pan, volume, priority)` with a given
pan; `FUN_80015a94` and `FUN_80015694(id, pos, volume, priority)` pan by
`pos`; and `FUN_80015828(id, pos, volume, priority)` also fades with
distance: the
volume × `clamp(1.4 − d / 50, 0, 1)` (`r2-0x7db0`, `-0x7da8`), `d` being
the distance from `pos` to the nearest hero in play (`FUN_80063658`), and
nothing plays at 0. The pan: 127.5 + 127.5 × (the unit offset of `pos`
from the camera's focus `0x8023F1BC`, flat, · the camera's right
`0x8023F094`) × `min(|offset| / 20, 1)`, negated when `right.x · off.z <
right.z · off.x`, clamped to −256…255; 127 (centre) with no position or
while `r13-0x77f8` is set. The heroes' footsteps use `FUN_80015828`
([player-movement.md](player-movement.md), "Footsteps").

## Not done / unconfirmed

- Track switching during gameplay, ducking, priorities, and the
  positional pan and fade above: the runtime plays a level's track 0 and
  effects at their call volume, nothing more.
- Bank version 0x100 and the byte-swapped variants (`VBNK`, `pGAV`,
  `SShd`) exist in the loader but not on the disc, so aren't implemented.
- PCM16 streams (codec ≠ 0x20): same.
- `SNDS` chunk (per-level sound names, e.g. `S_ENTERING1A`) and the rest of
  the WAD.
