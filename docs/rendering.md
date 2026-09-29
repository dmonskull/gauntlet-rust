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
