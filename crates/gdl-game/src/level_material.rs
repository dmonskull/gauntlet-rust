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

/// The level's light in shader form (`docs/rendering.md` "Lighting").
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct SceneLight {
    pub dir: Vec4,
    pub color: Vec4,
}

/// The game draws unlit objects with object colour 0x80 (1.0 on the PS2).
const OBJECT_COLOR: f32 = 128.0 / 255.0;

impl SceneLight {
    pub fn new(light: &gdl_formats::LevelLight) -> Self {
        // The game negates and normalizes the direction: this points at the
        // light.
        let toward = -Vec3::from(light.direction).normalize_or(Vec3::NEG_Y);
        Self {
            dir: toward.extend(light.ambient),
            color: (Vec3::from(light.color) * light.intensity).extend(OBJECT_COLOR),
        }
    }
}

impl Default for SceneLight {
    /// What every retail level but one uses.
    fn default() -> Self {
        Self::new(&gdl_formats::LevelLight { ambient: 0.8, direction: [-1.0, -6.0, 2.0], color: [1.0; 3], intensity: 1.0 })
    }
}

pub struct LevelMaterialPlugin;

impl Plugin for LevelMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "level.wgsl");
        app.add_plugins(MaterialPlugin::<LevelMaterial>::default())
            .init_resource::<SceneLight>()
            .add_systems(PostUpdate, apply_scene_light);
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
    /// x: lightmap enabled, y: alpha cutoff, z: stage 0 scale, w: lit by
    /// the level light (no prelit colours).
    #[uniform(4)]
    pub params: Vec4,
    /// xyz: unit vector toward the light, w: ambient.
    #[uniform(5)]
    pub light_dir: Vec4,
    /// rgb: light colour × intensity, a: object colour.
    #[uniform(6)]
    pub light_color: Vec4,
    /// xy: scroll of the diffuse texture.
    #[uniform(7)]
    pub uv_offset: Vec4,
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
        let light = SceneLight::default();
        Self {
            diffuse,
            lightmap,
            params: Vec4::new(lit, cutoff, STAGE0_SCALE, 0.0),
            light_dir: light.dir,
            light_color: light.color,
            // z: additive (the shader premultiplies and leaves the
            // destination's alpha alone, which Bevy's `Add` expects).
            uv_offset: Vec4::new(0.0, 0.0, if alpha_mode == AlphaMode::Add { 1.0 } else { 0.0 }, 0.0),
            alpha_mode,
            depth_test: true,
            depth_write: true,
        }
    }

    /// Lit by the level light rather than prelit vertex colours.
    pub fn dynamic(mut self, on: bool) -> Self {
        self.params.w = if on { 1.0 } else { 0.0 };
        self
    }

    pub fn is_dynamic(&self) -> bool {
        self.params.w > 0.5
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

/// Keeps every dynamically lit material on the current level light (new
/// materials start on the default).
fn apply_scene_light(
    light: Res<SceneLight>,
    mut events: MessageReader<AssetEvent<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
) {
    let added: Vec<AssetId<LevelMaterial>> =
        events.read().filter_map(|e| if let AssetEvent::Added { id } = e { Some(*id) } else { None }).collect();
    let ids: Vec<AssetId<LevelMaterial>> = if light.is_changed() { materials.ids().collect() } else { added };
    for id in ids {
        let stale = materials
            .get(id)
            .is_some_and(|m| m.is_dynamic() && (m.light_dir != light.dir || m.light_color != light.color));
        if stale && let Some(m) = materials.get_mut(id) {
            m.light_dir = light.dir;
            m.light_color = light.color;
        }
    }
}
