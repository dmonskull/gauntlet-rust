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
   hardware, converted to linear at the end; textures upload as plain
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
| `0x800000` | additive: `GXSetBlendMode(BLEND, SRCALPHA, ONE)` (PS2 ALPHA `0x48`; normal is `0x44` = `SRCALPHA, INVSRCALPHA`) |
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
set from the level's record when it loads). Blending happens in linear
space here but in gamma space on the GameCube, so dark translucent layers
(blob shadows) come out lighter than the original.

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
