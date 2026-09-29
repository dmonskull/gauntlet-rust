//! Material for level geometry: the diffuse texture and lightmap pairing the
//! game's draw loop binds to GX texture maps 0 and 1 (`docs/rendering.md`).

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

/// TEV stage 0 scales texture × rasterized colour by 2 (`GX_CS_SCALE_2`),
/// PS2-style: a colour of 0.5 leaves the texture unchanged. The lightmap
/// stage then multiplies by the lightmap's alpha at 1×. See
/// `docs/rendering.md`.
pub const STAGE0_SCALE: f32 = 2.0;

pub struct LevelMaterialPlugin;

impl Plugin for LevelMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "level.wgsl");
        app.add_plugins(MaterialPlugin::<LevelMaterial>::default());
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(LevelMaterialKey)]
pub struct LevelMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub diffuse: Option<Handle<Image>>,
    #[texture(2)]
    #[sampler(3)]
    pub lightmap: Option<Handle<Image>>,
    /// x: lightmap enabled, y: alpha cutoff, z: stage 0 scale.
    #[uniform(4)]
    pub params: Vec4,
    pub alpha_mode: AlphaMode,
    /// The game's per-instance depth switches (render flags 0x40, 0x80).
    pub depth_test: bool,
    pub depth_write: bool,
}

/// Pipeline variant: depth test and depth write on or off.
#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct LevelMaterialKey {
    depth_test: bool,
    depth_write: bool,
}

impl From<&LevelMaterial> for LevelMaterialKey {
    fn from(m: &LevelMaterial) -> Self {
        Self { depth_test: m.depth_test, depth_write: m.depth_write }
    }
}

impl LevelMaterial {
    pub fn new(diffuse: Option<Handle<Image>>, lightmap: Option<Handle<Image>>, alpha_mode: AlphaMode) -> Self {
        let cutoff = match alpha_mode {
            AlphaMode::Mask(c) => c,
            _ => 0.0,
        };
        let lit = if lightmap.is_some() { 1.0 } else { 0.0 };
        Self {
            diffuse,
            lightmap,
            params: Vec4::new(lit, cutoff, STAGE0_SCALE, 0.0),
            alpha_mode,
            depth_test: true,
            depth_write: true,
        }
    }

    pub fn with_depth(mut self, test: bool, write: bool) -> Self {
        self.depth_test = test;
        self.depth_write = write;
        self
    }
}

impl Material for LevelMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://gdl_game/level.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            if !key.bind_group_data.depth_write {
                depth.depth_write_enabled = false;
            }
            if !key.bind_group_data.depth_test {
                depth.depth_compare = CompareFunction::Always;
            }
        }
        Ok(())
    }
}
