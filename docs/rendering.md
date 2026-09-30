# Rendering a level

Implemented in [`crates/gdl-game/src/world.rs`](../crates/gdl-game/src/world.rs)
and [`level_material.rs`](../crates/gdl-game/src/level_material.rs) /
[`level.wgsl`](../crates/gdl-game/src/level.wgsl).

## What the game does

`FUN_800c3bbc` draws an instance: for each submesh it binds the diffuse
texture to GX texture map 0 and, when the lightmap binding is non-zero, the
lightmap to map 1, then `FUN_800c48c0` decodes the VIF packet and emits GX
triangle strips. Level geometry is lit by its lightmaps (or by prelit
vertex colours), not by dynamic lights.

## What we do

1. For every `WORLDS.PS2` node with a model, place that model's submeshes
   at the node's world position.
2. Merge everything sharing a (diffuse, lightmap) pair into one mesh — a
   level is a few hundred draw calls instead of tens of thousands.
3. Draw with `LevelMaterial`, which reproduces the game's TEV setup
   (`FUN_800c46f8`, programming `GXSetTevColorIn`/`GXSetTevColorOp` at
   `FUN_800ffbe0`/`FUN_800ffce4`):
   - stage 0, always: `clamp(texture × rasterized colour × 2)`
     (`GX_CC_TEXC × GX_CC_RASC`, `GX_CS_SCALE_2`)
   - stage 1, chosen by `FUN_800c48c0` when the submesh has a lightmap
     (render state flag 2): `stage 0 × lightmap alpha` at 1×
     (`GX_CC_CPREV × GX_CC_TEXA`, texture map 1 on texture coordinate 1).
   The rasterized colour of lightmapped geometry is its prelit vertex colour
   (every lightmapped submesh on the disc has one). In gamma space like the
   hardware, and written out in gamma space: the 3D camera renders into a
   float target, so every blend works on gamma values as the GameCube's
   frame buffer does, and `gamma.rs` turns the finished picture linear for
   the sRGB output (before the UI draws). Textures upload as plain
   `Rgba8Unorm`; tonemapping is off.
4. Emit both windings of every triangle (the game doesn't cull this
   geometry).

## Assumptions not yet traced to the binary

- **Dynamic lighting.** Geometry without prelit colours (about 10,000
  submeshes, and all characters) is lit by the game's lights in
  `FUN_800c48c0` (normal · light direction plus ambient); we use a neutral
  0.5 until those lights are implemented.
- The ambient/base term `FUN_800c48c0` adds to prelit colours before
  clamping (`DAT_802cc088`) is assumed 0 for level geometry.
- Alpha: `Mask(0.5)` for textures with transparency, blending for the
  shared-palette formats. The submesh/binding flags that really choose the
  blend mode aren't decoded yet.

## Handedness

The data is right-handed (Y up, counter-clockwise front faces), but the
game shows it through a left-handed view: its projection (built in
`FUN_800c87c8` into the view record at `+0x80`) puts `+1` in the w row, so
the camera looks along +Z in view space, with a negative Y scale; and the
models agree — a character faces +Z with its right side at +X (the
Warrior's `R_WRIST` at x +0.85, `L_WRIST` −0.96; the weapon hangs from
`R_WRIST`). A plain right-handed render is therefore the game's picture
mirrored left to right: on the tower's start the orange crystals (x
15–28) came out on the hero's right, where the game has them on his left.

We draw with `camera::MirroredPerspective` — Bevy's perspective with clip
space flipped left to right — so the picture is the game's. Everything
else stays in the game's own coordinates. Level materials take clockwise
triangles as front faces (the flip reverses winding on screen), the stick
maps right to +X when facing +Z (the game's heading is the camera's yaw +
the stick's angle), and the free camera's strafe and mouse look are
flipped to follow the screen. Camera-facing nodes need nothing: they turn
+Z to the camera as the game does, which the flip then shows the game's
way round.

## Blending and depth (instance render flags)

Each `WORLDS.PS2` node's `+0x18` holds the flags its instance is created
with: `FUN_800aafb0` reads the field, then overwrites it with the parent
pointer, and ORs the value into the instance's `+0x60` (`FUN_800ba658`).
Level artists' node-name codes line up with the bits (`XP` nodes are
additive, `FF` nodes carry `0x1000000`, `CF` nodes `0x4000000`).

`FUN_800c3bbc` draws an instance with `+0x60 & 0x1090D7C0`;
`FUN_800c5894`/`FUN_800c664c` turn the bits into GX state (they're the PS2
GS settings the data was authored for):

| bit | effect |
| --- | --- |
| `0x40` | depth compare always (`GXSetZMode` func 7 instead of 6) |
| `0x80` | no depth writes |
| `0x4000` | skip the lightmap stage |
| `0x800000` | additive: `GXSetBlendMode(BLEND, SRCALPHA, ONE)` (PS2 ALPHA `0x48`; normal is `0x44` = `SRCALPHA, INVSRCALPHA`). Bevy draws `AlphaMode::Add` as premultiplied alpha, so `level.wgsl` outputs colour × alpha with alpha 0 for these (without that, glows drew as dark discs) |
| `0x100`, `0x200`/`0x400`/`0x100000` | tint colour from the instance / fade alpha from instance `+0x53` (not implemented) |
| `0x8000` (→ `0x20000`), `0x10000000` | extra texture stages (not implemented) |

The draw traversal (`FUN_800c7d6c`) skips an instance's subtree on `0x2`
and its own draw on `0x1`. Bits `0x0F000000` pick a facing mode that
`FUN_800c81ac` applies to the instance matrix each frame, from the camera
position (`FUN_800b8f54`): `0x1000000` turns about Y so +Z faces the camera
(`FF` bushes and trees), `0x3000000` also tilts freely, `0x5000000`/
`0x6000000`/`0x7000000` tilt at most 15°/30°/45° (`r2-0x4780..`), and
`0x4000000` takes the camera's rotation (`CF` sprites and glows). We spawn
those instances as their own entities with a `Billboard` component
(`billboard.rs`); `0x2000000` and `0x8000000` (other routines) are unused
by the levels.

Everything else is alpha-blended with an alpha test of > 2. We keep a
cut-out mask for textures whose alpha is only ever 0 or 255, blend textures
with real partial alpha (fog cards, glass), and specialize the level
material's pipeline on the two depth switches. `FUN_800b4490` is the same
table for effects and 2D.

## Lighting (geometry without prelit colours)

Characters, items and any object whose vertices carry no colour are lit
in software by the model interpreter (`FUN_800c48c0`), per vertex, in byte
units (0x80 = 1.0, clamped to 255):

```
colour = objColour × ambient + max(0, N · L) × lightColour × objColour
       (+ each point light: min(1, N·D × strength × (1 − k·|D|²) / |D|²) × colour)
```

`L` is the scene light's direction negated and normalized (`FUN_800ad810`,
×`r2-0x4d68` = −1); `objColour` is the instance's colour (0x80 grey for
ordinary instances). The scene light comes from the level's `WDATA` `LEVL`
record (`FUN_800678f0` → `FUN_800b6568`/`FUN_800b64cc`): ambient `+0xEC`,
direction `+0xF0`, colour `+0xFC`, intensity `+0x108` — ambient 0.8 and a
white light along (−1, −6, 2) in every level except B6. Up to three lights
are supported (`+0x9C` count, `+0xBC` array of the lighting environment);
the levels use one. Prelit vertices are multiplied by the instance colour
instead. Point lights (the `r13-0x6a9c` list) aren't implemented yet.

We light per pixel in `level.wgsl` with the same formula (`SceneLight`,
set from the level's record when it loads).

## Colour space

The GameCube blends in its 8-bit frame buffer, on gamma-encoded colour.
Blending linear light instead (an sRGB target) changes every translucent
and additive layer: dim additive texels all but vanish — the tower's
brazier flames (`P_TORCH`, texels at most 0.3, eight or so overlapping)
drew as a faint glow instead of the flames the game shows — and dark
translucent layers such as blob shadows come out too light. So the level
shader writes gamma-space colour into a float target, blends happen on
those values, and `gamma.rs` decodes the finished picture to linear light
(clamped to 1 first, as the 8-bit buffer saturates). Stand-in left: the
float target doesn't saturate between one blend and the next as 8 bits
do. Bevy's own materials (debug markers) come out darker for it.

## Texture animation (`LEVELS/<level>/ANIM.PS2`)

Despite the name, a level's `ANIM.PS2` is its texture-modifier list
(`texmod.rs`), loaded as "anim" into `0x8028c4ec` (`FUN_800a8b2c`).
Header `{0, 0, count, offset}`, then 0x58-byte records; set up by
`FUN_800185b0` (message "TEXMOD with 0 texidx") and stepped every tick by
`FUN_80018788` (from `FUN_80010a4c`):

| offset | field |
| --- | --- |
| `+0x04` | texture name (32 bytes) |
| `+0x24` | first-frame texture name (16 bytes), used when `+0x48` is −1 |
| `+0x44` | binding of the modified texture in the level's `objects.ngc` |
| `+0x48` | ≥ 0: first frame's binding; −1: look it up by the `+0x24` name; −2 / −3: scroll U / V |
| `+0x4C` | i16: frames, or steps per whole scroll (negative scrolls backwards) |
| `+0x4E` | i16: scroll phase |
| `+0x50` | ticks per step (≤ 1: every tick; checked against the frame counter `r13-0x7580`) |
| `+0x54` | starting step |

A flipbook swaps the binding's texture for `first + step % frames`
(`FUN_800ba278`); a scroll writes `±(step − phase) / |count|` into the
scroll table at `0x802c7b08` (U at `+4`, V at `+0xC`), which the draw path
applies as a texture-matrix offset for bindings flagged `0x40`
(`FUN_800c6a78` → `FUN_800c68e4`). All 67 levels' files parse (623
modifiers) and point at real bindings. Torches (`TORCHA`…) take their
frames from `TORCH00`, which isn't in the level's own texture names — not
resolved yet. We evaluate scrolls between ticks so they glide.

## Particle-system nodes

World nodes with node flag `0x800` (named `…PSYSE_FLAME…`, `…PSYSF_SPARK…`)
are particle emitters: their model is only a placeholder shape (a white
pyramid with `AAAWHITE`), which the game doesn't draw; triggers switch
some on and off (`docs/mechanics.md`). `world.rs` leaves the shape out and
[`particles.rs`](../crates/gdl-game/src/particles.rs) runs the flames,
smoke, pool fires, mist, embers and fireflies.

**The records** (`gdl_formats::psys`): a level's `WORLDS.PS2` header words
28/29 are the count and offset of its `0x138`-byte particle records (the
world struct's `+0x9C`/`+0xA0`, `DAT_8028c508`/`DAT_8028c50c`); they end
the file. Fields, by word, with the bit of word 4 that says they're set
and what `FUN_800ceeb8` makes of them:

| words | bit | meaning |
| --- | --- | --- |
| 0 | — | kind, ≥ `0x100` |
| 1 (`+4` i16, `+6` byte) | 1 | built-in preset first (`0x80127fc4`, ids 0–7); the letter `PSYS<letter>` names |
| 2, 3 | — | flag values and mask: `0x80` → instance `0x800000` (additive), `0x100` → `0x40000000`, `0x200` → `0x800`, `0x400` → `0x40`, `0x800` → `0x80`; `1/2/4/0x20/0x40` → emitter flags |
| 5, 6, 7 | 2, 4, 8 | emitter counts (`+0x2E`: ring size, `+0x30`, `+0x32`) |
| 8, 9 | `0x10` | the two emitting phases' lengths, s (× 30 → frames; 999 on every record) |
| 10, 11 | `0x20` | particle life: shortest, plus a random extra, s |
| 14 | `0x40` | spray cone, degrees (half-angle π·v/360; ≥ 359 all round) |
| 15 | `0x8000` | `+0x5C` |
| 0x10–0x17 | `0x4000` | texture name (in the level's `objects.ngc`, or `WEAPONS`) |
| 0x18–0x1A | `0x80` | direction (default up) |
| 0x1B–0x1D | `0x100` | start box half-size |
| 0x1E–0x21 | `0x200` | emission rates, particles/s: phase A start and slope, phase B start and slope (`+0xD0..DC`) |
| 0x22 | `0x400` | rate jitter, % |
| 0x23 | `0x800` | buoyancy: × −32/900 per frame² (negative rises) |
| 0x24 | `0x1000` | `+0x9C` (× −1; not identified) |
| 0x25 | `0x2000` | start speed, units/s |
| 0x26–0x29 | `0x10000` / `0x20000` | four colour keys `0xAARRGGBB` (RGB / alpha) over the life |
| 0x2A–0x2D | `0x40000` | four size keys |
| 0x2E | `0x80000` | a start delay, s |

The library (`FUN_800cbf44`, per emitter per frame) keeps particles in
packed rings filled through emitter callbacks and states 0 (delay) → 2/3
(phase A: rate `+0xD0` + `+0xD4`·t) → 4/5 (phase B) → 6 (done) → 8
(freed); `particles.rs` simulates each particle directly from the record
instead (stand-in), with the phases taken as always on. Unconfirmed: whether
the library counts frames or video fields (twice as many: brighter, busier
torches). `GDL_PARTICLE_TEST=<letter>` with `GDL_LOOK_AT="0,0.5,-6,10"`
on levelA1 shows one record's system in the open. What's known of the
setup:

- `FUN_800aae??` (the world-instance setup, around `0x800aaeb0`): a node
  whose name contains `PSYS` (`r2-0x5000`) takes the letter after it
  (`PSYSE_FLAME` → `E`) and looks for the particle record whose `+0x06`
  byte is that letter in the level's table (`DAT_8028c508`, count
  `DAT_8028c50c`, `0x138` bytes each — the ANIM.PS2 `+0x10`/`+0x14`
  records, [animation-format.md](animation-format.md)); `"Unable to find
  world psys %c"` otherwise. A node with collision (`+0x36` > 0) passes
  its first collision point (× `r2-0x4ff8`) as an offset. It then creates
  the system (`FUN_800cede8` → `FUN_800d0af4`) on the node's instance and
  sets instance flag `0x800000`.
- `FUN_800ceeb8(system, emitter, record, offset)` applies a record: the
  word at `+0x00` must be ≥ 0x100 (`"setWorldParms: WORLDPSYS type is"`);
  `+0x10` is a bit set saying which fields follow — bit 1 first applies the
  built-in preset whose `+0x04` short matches (table `0x80127fc4`, `0x138`
  stride, ends at −1); bits 2/4/8 set emitter shorts `+0x2E`/`+0x30`/`+0x32`
  from words 5–7; 0x10 two clamped rates from words 8–9 (× a constant) to
  `+0x3A`/`+0x3C`; 0x20 two bytes from words 10–11; 0x40 a size from word
  14; 0x80000 a short from word 0x2E; … (not finished).
- `"TOO MANY PSYS OBJECTS"`, `"PSYS requires too many bits"` and
  `"Setting PSYS attribute after draw"` mark the library's other entry
  points (`0x800cc…`–`0x800d1…`).

## Item models

Items built from an atree draw each node with that node's render flags
(blending, depth, camera facing), like characters; hidden nodes and
`…GLOW` nodes (textured by the effects system at run time) are left out.
