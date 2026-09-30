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

**The calls.** Every sound effect goes through `FUN_80015cac(jukebox, id,
volume, pos, pan, priority)` by way of a few wrappers (argument order
theirs):

| wrapper | args | placed | notes |
| --- | --- | --- | --- |
| `FUN_800157ec` | id, volume, priority | centred | |
| `FUN_80015a30` | id, pan, volume, priority | the pan given | every caller passes 0x7F or a player's pan `0x80122A90[p]`, which is 0x7F for all four: centred |
| `FUN_80015a94` | id, pos, volume, priority | panned by `pos` | nothing for id < 0 |
| `FUN_80015694` | id, pos, volume, priority | panned by `pos` | the same without the id check; the loops' |
| `FUN_80015828` | id, pos, volume, priority | panned and faded by `pos` | nothing for id < 0 or at a fade of 0 |

The last four don't play while `r13-0x7380 & 0x8000` is set (unless
`r13-0x7854`). `FUN_80015cac` scales the volume by the options' effects
volume (`r13-0x7FB8` / 256), forces the pan to 0x7F in mono
(`r13-0x77F8`, set by `FUN_80017ebc(0)`: "Mono"), and whenever it's
given a position recomputes the pan from it with the same law
(`FUN_800167a4`); the priority is the driver's voice-stealing key (above,
"Calls"), nothing to do with placing.

**The pan** (`FUN_800167a4`, and inline in the wrappers). The ear is
camera 0's focus `0x8023F1BC` (its target: `FUN_8001c42c` copies the
camera's `+0xA4`; one special camera, `FUN_8001bc3c`, puts its eye there),
the axis camera 0's view matrix row 0 `0x8023F094` — built by
`FUN_800227d8` as up × forward, `(fz, 0, −fx)`: the screen's right. With
`o` the level offset of `pos` from the focus (Y dropped) and `u = o /
|o|` (0 at the focus):

```text
pan = trunc(127.5 + 127.5 × (u · right) × min(|o| / 20, 1))
pan = −pan   if right.x · u.z < right.z · u.x   (u · forward < 0: behind the focus)
pan clamped to −256 … 255;  0x7F without a position (or in mono)
```

So 0 is full left, 127 dead ahead, 255 full right, and anything nearer
the camera than its focus goes negative; within 20 of the focus the pan
narrows toward the centre. (`r2-0x7DB8` 127.5, `r2-0x7DC0` 20.)

**The fade** (`FUN_80015828` only): `d` is the distance in 3D from `pos` to
the feet (`+0x44`) of the nearest player in play (state `+0xE8` = 1;
`FUN_80063658`, 1000 with none; the attract loop's demo, mode `0x8008`,
measures from the camera instead), and the requested volume is multiplied
by `clamp(1.4 − d / 50, 0, 1)` and truncated (`r2-0x7DB0` 1.4,
`r2-0x7DA8` 50): full within 20, silent from 70 — and at 0 it isn't
played at all.

**Once, as it starts.** The pan and the fade are worked out when the call
is made, from the camera and the heroes then. The started-voice table
(`0x8023D4E8`) keeps the position pointer, but nothing reads it back: a
one-shot isn't re-panned as it plays. A **loop** that follows something is
re-panned by its owner every frame: it finds its voices by their priority
— each such loop has its own (`FUN_8001630c`) — and sets their pan
(`FUN_800160a8`, driver command `0x55AC`); the level items' ambient loops
(`FUN_800a00ec`) also set their volume that way every frame
(`FUN_800161fc`, `0x55AB`) from their own distance factor — and that new
volume is taken as it is (× the effects volume / 256, less the ducking),
not × the call's own / 127 as the voice's start was. Each such command
moves the voice's pan or volume toward the new value by at most 8
(`FUN_800d22f4`, `FUN_800d24ac`): with the owners sending one every game
tick, 8 a tick (30 a second). Such loops — each but the items' started,
panned by its position (`FUN_80015694`), whenever it isn't playing
(`FUN_800163c4`), and stopped by its sound (`FUN_80016558`):

- the exit's flame (`S_EXITFLAME`, `FUN_8009ce48`, 0xE0): at the first
  hero standing in an exit, its `+0x54` (`FUN_8007692c`);
- the movers' (`FUN_8009cecc`, 0xE0): the level's table `0x8028AFF0` by the
  mover's set, at the mover's node (its matrix's `+0x30`). Only the first
  mover moving each tick calls it (`FUN_800629ec`); a mover stopping stops
  it and, if it was playing, plays the set's stop sound, panned at 0xE0;
- the rotators' (`FUN_8009d01c`, 0xE0): the realm's `0x801232E4`, at the
  node. Every rotator grinding calls it each tick, so the last one's pan
  holds; one reaching its end stops it and plays the realm's
  `0x8012331C`, panned at 0xE0 (`FUN_800606e8`);
- the hourglass (`S_HOURGLASS`, `FUN_8009fee8`, 0x7F): at the first hero
  holding it (in play with `+0x960` set, [powers.md](powers.md)), its
  `+0x54`;
- Death's drain (`S_DEATHSUCK`, `FUN_800a045c`, 0x7F): at the draining
  Death's feet `+0x34` (its AI, `FUN_800460a8`), and at the hero's feet
  `+0x44` while its halo drains a Death (`FUN_8007c4f0`) — one voice for
  both (priority 0x6F);
- the items' ambient sounds (`FUN_800a00ec`), re-volumed as well.

**The volume.** The driver sets a voice's volume to requested × the call's
own (the bank's `vol`, 0–127) / 127, less the ducking (`FUN_800d2698`),
and its AX volume to that × 0x3FFF / 255 (`FUN_800d3d48`): linear. So 0x7F
plays a call at its own volume, 0xB4 at 1.4 ×, 0xE0 at 1.76 ×, 0xFF at 2 ×.

**The mixer** (`FUN_800d3d48` as the voice starts, `FUN_800d22f4` on
changes). The pan is an angle, 512 to the turn: the side pan `|0x100 −
((pan + 0x100) & 0x1FF)| / 2` (= |pan| / 2) goes to `MIXSetPan`
(`FUN_80102504`: 0 left … 127 right) and the surround pan `|0x100 −
((pan + 0x180) & 0x1FF)| / 2` to `MIXSetSPan` (`FUN_801025a0`: 127 in
front … 0 behind). So 0 is left, ±128 ahead or behind, 255 right: 127 →
side 63, surround 127; −127 → 63, 0. The mixer's table (`0x8023AEBC`,
tenths of a dB) is 10·log10((127 − k) / 127), −90.4 dB at 127 — a
constant-power law, −3 dB each at the centre (checked against all 128
entries); `0x8023B0BC` is the same backwards. A channel gets l =
T[side], r = T[127 − side], f = T[127 − surround], b = T[surround], and
in the mixer's mode 1 (what `MIXInit` sets; the game never changes it)
`L = fader + l + f`, `R = fader + r + f`, `S = fader + b`, turned into AX
volumes by `0x8023AE40` (0x8000 at 0 dB).

**Who plays what** (players' `+0x44` are the feet, `+0x54` the top point
4.4 above; monsters' `+0x34` the feet, `+0x44`, `+0x54` the bump point;
items' `+0x34` the place, `+0x54` the centre, the blows on items taking it
2 higher; "fade" is `FUN_80015828`, "pan" `FUN_80015a94`/`FUN_80015694`):

| sounds | function | call | where | volume |
| --- | --- | --- | --- | --- |
| footsteps | `FUN_8009e804` | fade | the hero's feet | 0x7F |
| the hero's throw, amulet or super-shot sounds | `FUN_8009ee70` | fade | the hero's feet | 0x7F |
| turbo attacks, `S_POJOTURBO` | `FUN_8009ed88` | pan | the hero's feet | 0xE0 |
| `S_PLAYERDIES` and the class's death cry | `FUN_8009ea88` | pan | the hero's feet | 0x7F / 0xE0 |
| a blow on the hero: `S_PLYRDMG`, `S_PLYRDMG2`, `S_PLYRDMG3` by its kind | `FUN_8009eb14` | pan | the hero's feet | 0x7F |
| the class's death line `S_<CLS>DIE1` | `FUN_8009f198` | pan | the hero's feet | 0xE0 |
| potions `S_POTION1`–`4`, shields `S_SHIELD1`–`4` | `FUN_8009e860` | pan | the hero's feet | 0x7F |
| damage tiles (`0x80122FF4` by realm and tile; `S_FIREHOLE2` for `S_FIREHOLE` on the dragon's level, boss 0x22) | `FUN_8009e8a4` | pan | the hero's feet | 0x7F (`S_TENTACLES`, `S_TENTACLESD` 0xB4) |
| `S_TUNNEL` (going out through an exit) | `FUN_8009ca90` | pan | the hero's feet | 0x7F |
| `S_HALO`, `S_THUNDERHAMMER`, `S_MASK`, `S_HORNS`, `S_GAUNTLET1`/`2`, the breaths (`FUN_8009ed08`) | `FUN_8009e990`, `FUN_8009cce8`, `FUN_8009ec88`, `FUN_8009ecc8`, `FUN_8009ec48`, `FUN_8009ec08` | pan | the hero's top | 0xE0 |
| `S_XRAY` | `FUN_8009e950` | pan | the hero's top | 0x7F |
| `S_DEATHDIE` while the halo drains a Death, started again whenever it isn't playing | `FUN_800a035c` from `FUN_80080d3c` | pan | the Death's `+0x54` | 0x7F |
| `S_LEVITATEDOWN`, `S_UNGROW`, `S_UNPOJO` | `FUN_8009cd28`, `FUN_8009cd98`, `FUN_8009cdd8` | pan | the hero's top | 0xE0 |
| `S_UNSHRINK` | `FUN_8009cd68` | centred | — | 0xE0 |
| `S_WARN` | `FUN_8009e9d0` | centred | — | by health: 0xCA at 10 or less, 0xB1 below 25, 0x98 below 100, else 0x7F |
| `S_TURBODEFENSE` | `FUN_8009ebc8` | pan | the hero's `+0x64` | 0x7F |
| pickups | `FUN_8009c630`, `FUN_8009c670`, `FUN_8009c718`, `FUN_8009c7e0`, `FUN_8009c870` | centred | — | 0x7F (powers by value) |
| `S_CHEST`, the doors' sounds | `FUN_8009c8b0`, `FUN_8009c8e0` | fade | the item's centre | 0x7F |
| `S_TRANSPORT<realm>` | `FUN_8009c1c4` | fade | the transporter gone to | 0x7F |
| `S_TICKY` (a timed chest) | `FUN_8009d330` | centred | — | 0xE0 |
| a generator hurt / destroyed | `FUN_8009bfac` / `FUN_8009c010` | fade | its centre, 2 up | 0xB4 / 0x7F |
| barrels: wood, explosive, gas (`S_BARREL_…<realm>`) | `FUN_8009d2b0`, `FUN_8009d210`, `FUN_8009d260` | fade | its centre, 2 up | 0xE0 |
| `S_SECRETWALL`, an obstacle's own hit sound | `FUN_8009c128` | fade | its centre, 2 up | 0x7F / its record's |
| `S_WEAPONHITWOOD` (blows on wood) | `FUN_8009e784` | fade | its centre, 2 up | 0x7F |
| a crumbling floor starting to fall (`0x801232AC` by realm, and `FUN_8009d154`'s) | `FUN_8009d104`, `FUN_8009d154` | fade | the item | 0xE0 |
| bridges `S_BRIDCL<r>`/`S_BRIDOP<r>`; movers by kind (`0x80123354`) | `FUN_8009c938`, `FUN_8009c9a4`; `FUN_8009ca10` | pan | the mover | 0xE0 |
| a monster's hit and death sounds | `FUN_8009d6c0`, `FUN_8009d7b4` | fade | its feet | 0xE0 |
| `S_SUICIDE_YELL`, `S_SUICIDE_BOMB` | `FUN_8009d5a4`, `FUN_8009d300` | fade | the runner's `+0x44` / `+0x54` | 0xE0 |
| `S_ENEMYARROW`, `S_ENEMYFIREBALL`, a thrown weapon's (`FUN_8009d51c`) | `FUN_8009d664`, `FUN_8009d634` | fade | where the monster's missile starts | 0x7F |
| `S_DEATHLAUGH`, `S_DEATHSHATTER`; `S_DEATHDIE` (Death killed) | `FUN_800a03d8`, `FUN_800a0408`; `FUN_800a03a8` | pan | Death's feet | 0xE0; 0x7F |
| critters' `SFXX` sounds | `FUN_8009bb64` from `FUN_8003d6f8` | fade (pan alone during its DEATH move, kind `0x11`) | the critter's root `+0x3C` | 0xE0 |
| `S_BOSSKEY<realm>` | `FUN_8009eb78` | pan | where the key appears (the boss's spawn point) | 0xE0 |
| `S_RICOCHET` | `FUN_8009e7b4` | fade, at most once a second (`r2-0x5310`) | the missile | 0x7F |
| `S_SPLASH`, effects' own sounds (`FUN_8009d35c`) | `FUN_8009ce18`, `FUN_8009d35c` | fade | the effect | 0xB4; the record's |
| the tower's chimes and knocks (`S_STNDGLASS`, `S_RUNEHIT`, `S_RUNEFALL`, `S_SHRD8`, `S_SHRDS127`) | `FUN_8009bc98` | centred (`pos` 0) | — | 0xFF |

The voice queues' lines carry a pan too: the heroes' (queue 0) are panned
from the hero's position as they're queued (`FUN_800167a4`); the
announcer's are centred.

Here (`audio.rs`): `PlaySoundAt { name, at, volume, fade }` —
`panned`, `faded`, `centred` — does the pan and the fade above as the
sound starts: the ear is the play camera's current view (its target, and
its right from eye to target), the heroes in play are the living hero not
out of the level (its feet), and the call's gain is the call's volume ×
volume / 127 (samples are floats, so a call asked for louder isn't
clipped by the decoder). The pan becomes left and right gains from the
side pan through the mixer's table, relative to the centre's (a centred
sound plays exactly as a `PlaySound`): stereo here, so the surround pan —
in front of the focus or behind it — isn't applied, and a sound behind
pans by its side pan like one in front. There's no mono option.

`LoopSoundAt { key, name, at, volume, follow_volume }` is a loop that
follows something: one per channel `key`, sent by its owner every tick
with where the thing is now (`LoopSoundAt::at`; `stop` ends it). It starts
panned from `at` at the call's volume × `volume` / 127; while it plays,
each frame its pan slides toward the one for where `at` is now by at most
8 a game tick (240 a second, the short way round the 512 to the turn), and
with `follow_volume` its volume toward `volume` itself the same way (the
items' quirk above). A call that doesn't loop and has ended is started
again by the next request, as the game's owners do. Only the last request
for a channel in a frame counts (two ticks in one frame don't start a
loop twice; the same for `LoopSound`, which still plays centred).

Placed so far: the footsteps; the x-ray; the tower's chimes (centred, at
0xFF); the powers' ends and `S_WARN` (`player_state.rs`); the damage tiles
(`hazards.rs`); the bridges', movers' and rotators' one-shots and their two
loops (`mechanics.rs`); a monster's hit and death sounds, a generator's
hurt and destroyed, a critter's hit sounds, `S_DEATHDIE` when Death is
killed (`damage.rs`); the runner's yell, Death's laugh and Death's drain
loop (`monsters.rs`); the hourglass's loop (`game_hud.rs`). Stand-ins: the
runner's yell is at its centre (the game's `+0x44` point, which the port
also uses for its effects), a generator's sounds at its place raised 2
(the game's are at its centre raised 2). Not done: the voice queues' pans;
`S_EXITFLAME` and `S_TUNNEL` aren't played; nor are the hero's own cries
(`S_PLYRDMG*`, `S_PLAYERDIES`, the death lines) and the items' ambient
loops.

## Not done / unconfirmed

- Track switching during gameplay, ducking and priorities: the runtime
  plays a level's track 0; effects at their call volume, or as a
  positional call asks ("Positional sounds").
- Bank version 0x100 and the byte-swapped variants (`VBNK`, `pGAV`,
  `SShd`) exist in the loader but not on the disc, so aren't implemented.
- PCM16 streams (codec ≠ 0x20): same.
- `SNDS` chunk (per-level sound names, e.g. `S_ENTERING1A`) and the rest of
  the WAD.
