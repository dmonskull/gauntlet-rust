//! Builds Bevy meshes and materials from a parsed `objects.ngc` +
//! `textures.ngc`, for level geometry and characters alike.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use gdl_formats::ModelFile;
use gdl_formats::texture::{self, TextureFormat as GameTextureFormat};

use crate::level_material::LevelMaterial;

/// A model file's textures, decoded on first use and shared between meshes.
pub struct TextureCache<'a> {
    model: &'a ModelFile,
    textures: &'a [u8],
    decoded: HashMap<u16, Option<(Handle<Image>, AlphaMode)>>,
}

impl<'a> TextureCache<'a> {
    pub fn new(model: &'a ModelFile, textures: &'a [u8]) -> Self {
        Self { model, textures, decoded: HashMap::new() }
    }

    pub fn get(&mut self, binding: u16, images: &mut Assets<Image>) -> Option<(Handle<Image>, AlphaMode)> {
        let (model, textures) = (self.model, self.textures);
        self.decoded
            .entry(binding)
            .or_insert_with(|| {
                let b = model.bindings.get(binding as usize)?;
                let image = texture::decode(textures, b).ok()?;
                // The game alpha-blends everything (discarding alpha <= 2).
                // Blending is only needed where alpha is actually partial;
                // cut-outs keep a mask so they sort and depth-write like
                // opaque geometry.
                let partial = image.pixels.as_chunks::<4>().0.iter().filter(|p| (8..248).contains(&p[3])).count();
                let alpha = match GameTextureFormat::from_selector(b.format) {
                    Ok(GameTextureFormat::SharedPaletteCi4 | GameTextureFormat::SharedPaletteCi8) => {
                        AlphaMode::Blend
                    }
                    _ if partial * 50 > image.pixels.len() / 4 => AlphaMode::Blend,
                    _ if image.has_transparency() => AlphaMode::Mask(0.5),
                    _ => AlphaMode::Opaque,
                };
                Some((images.add(to_bevy_image(image)), alpha))
            })
            .clone()
    }
}

/// How an instance is drawn, from its render flags.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct DrawState {
    additive: bool,
    depth_test: bool,
    depth_write: bool,
    lightmap: bool,
}

impl DrawState {
    fn from_flags(flags: u32) -> Self {
        use gdl_formats::world::render_flags as f;
        Self {
            additive: flags & f::ADDITIVE != 0,
            depth_test: flags & f::NO_DEPTH_TEST == 0,
            depth_write: flags & f::NO_DEPTH_WRITE == 0,
            lightmap: flags & f::NO_LIGHTMAP == 0,
        }
    }
}

#[derive(Default)]
struct MeshBuffers {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    lightmap_uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

pub struct BuiltMesh {
    pub mesh: Handle<Mesh>,
    pub material: Handle<LevelMaterial>,
    pub triangles: usize,
}

/// Merges the given objects (each moved by its offset) into one mesh per
/// (diffuse, lightmap) pair, drawn the default way.
pub fn build(
    model: &ModelFile,
    cache: &mut TextureCache,
    instances: impl IntoIterator<Item = (usize, Vec3)>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
    bounds: &mut (Vec3, Vec3),
) -> Vec<BuiltMesh> {
    let instances = instances.into_iter().map(|(object, offset)| (object, offset, 0));
    build_flagged(model, cache, instances, meshes, materials, images, bounds)
}

/// Like [`build`], with each instance's render flags (`WorldNode::render_flags`)
/// choosing its blending and depth handling; one mesh per (diffuse,
/// lightmap, draw state).
pub fn build_flagged(
    model: &ModelFile,
    cache: &mut TextureCache,
    instances: impl IntoIterator<Item = (usize, Vec3, u32)>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
    bounds: &mut (Vec3, Vec3),
) -> Vec<BuiltMesh> {
    let mut groups: HashMap<(u16, u16, DrawState, bool), MeshBuffers> = HashMap::new();
    for (object, offset, flags) in instances {
        let state = DrawState::from_flags(flags);
        for submesh in &model.objects[object].submeshes {
            if submesh.triangles.is_empty() {
                continue;
            }
            let d = submesh.descriptor;
            let lightmap = if state.lightmap { d.lightmap } else { 0 };
            // Without prelit colours the game lights the vertices itself.
            let lit = submesh.vertices.first().is_some_and(|v| v.color.is_none());
            let buf = groups.entry((d.texture, lightmap, state, lit)).or_default();
            let base = buf.positions.len() as u32;
            for v in &submesh.vertices {
                let p = Vec3::from(v.position) + offset;
                bounds.0 = bounds.0.min(p);
                bounds.1 = bounds.1.max(p);
                buf.positions.push(p.to_array());
                buf.normals.push(v.normal);
                buf.uvs.push(v.uv);
                buf.lightmap_uvs.push(v.lightmap_uv.unwrap_or_default());
                // Raw 0..1 values: the shader multiplies in gamma space.
                // Unlit vertices are lit in the shader (the colour only
                // carries alpha then).
                buf.colors.push(match v.color {
                    Some([r, g, b]) => [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
                    None => [0.5, 0.5, 0.5, 1.0],
                });
            }
            // Both windings: the game draws this geometry without culling.
            for t in &submesh.triangles {
                buf.indices.extend(t.iter().map(|i| i + base));
                buf.indices.extend([t[0], t[2], t[1]].iter().map(|i| i + base));
            }
        }
    }

    let mut out = Vec::with_capacity(groups.len());
    for ((diffuse, lightmap, state, lit), buf) in groups {
        let triangles = buf.indices.len() / 6;
        let (diffuse_image, mut alpha_mode) = match cache.get(diffuse, images) {
            Some((image, alpha)) => (Some(image), alpha),
            None => (None, AlphaMode::Opaque),
        };
        if state.additive {
            alpha_mode = AlphaMode::Add;
        }
        let lightmap_image = (lightmap != 0).then(|| cache.get(lightmap, images)).flatten().map(|(i, _)| i);

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, buf.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, buf.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, buf.uvs)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, buf.colors)
            .with_inserted_indices(Indices::U32(buf.indices));
        if lightmap_image.is_some() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, buf.lightmap_uvs);
        }
        out.push(BuiltMesh {
            mesh: meshes.add(mesh),
            material: materials.add(LevelMaterial::new(diffuse_image, lightmap_image, alpha_mode).with_depth(state.depth_test, state.depth_write).dynamic(lit)),
            triangles,
        });
    }
    out
}

/// Uploaded as plain `Rgba8Unorm`: the shader does its maths on the raw
/// (gamma-space) values the way the GameCube does, then converts.
fn to_bevy_image(image: texture::RgbaImage) -> Image {
    let mut out = Image::new(
        Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        image.pixels,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    out.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    out
}
