//! Material for level geometry: the diffuse texture and lightmap pairing the
//! game's draw loop binds to GX texture maps 0 and 1 (`docs/rendering.md`).

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Lightmaps modulate at 2x, PS2-style (mid-level = unchanged). Chosen from
/// the data — lightmap texels cluster at alpha level 3 of 7, which 2x maps
/// to ~0.86 brightness and 1x to ~0.43 — not yet traced to the game's TEV
/// setup.
pub const LIGHTMAP_SCALE: f32 = 2.0;

pub struct LevelMaterialPlugin;

impl Plugin for LevelMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "level.wgsl");
        app.add_plugins(MaterialPlugin::<LevelMaterial>::default());
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct LevelMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub diffuse: Option<Handle<Image>>,
    #[texture(2)]
    #[sampler(3)]
    pub lightmap: Option<Handle<Image>>,
    /// x: lightmap enabled, y: alpha cutoff, z: lightmap scale.
    #[uniform(4)]
    pub params: Vec4,
    pub alpha_mode: AlphaMode,
}

impl LevelMaterial {
    pub fn new(diffuse: Option<Handle<Image>>, lightmap: Option<Handle<Image>>, alpha_mode: AlphaMode) -> Self {
        let cutoff = match alpha_mode {
            AlphaMode::Mask(c) => c,
            _ => 0.0,
        };
        let lit = if lightmap.is_some() { 1.0 } else { 0.0 };
        Self { diffuse, lightmap, params: Vec4::new(lit, cutoff, LIGHTMAP_SCALE, 0.0), alpha_mode }
    }
}

impl Material for LevelMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://gdl_game/level.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}
