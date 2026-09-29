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
3. Draw with `LevelMaterial`: `diffuse(uv) × vertex colour × lightmap
   alpha(uv2) × 2`, in gamma space like the GameCube's TEV, converted to
   linear at the end. Textures are uploaded as plain `Rgba8Unorm` so their
   raw values reach the shader; tonemapping is off.
4. Emit both windings of every triangle (the game doesn't cull this
   geometry).

## Assumptions not yet traced to the binary

- **Lightmap scale 2×.** Lightmap texels cluster at alpha level 3 of 7; 2×
  puts that at ~0.86 brightness, 1× at ~0.43. The TEV configuration for
  map 1 hasn't been read yet to confirm.
- **Vertex colours at 1×.** `FUN_800c48c0` shifts them left 3 and adds an
  ambient term before clamping; the ambient term and TEV scale aren't
  confirmed.
- Alpha: `Mask(0.5)` for textures with transparency, blending for the
  shared-palette formats. The submesh/binding flags that really choose the
  blend mode aren't decoded yet.
