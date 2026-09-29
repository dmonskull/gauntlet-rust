//! Turns a level's parsed models, world placements and textures into Bevy
//! meshes, and switches levels at runtime.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use gdl_formats::texture::{self, TextureFormat as GameTextureFormat};

use crate::camera::FlyCamera;
use crate::level::{LevelData, LoadedGame};
use crate::level_material::LevelMaterial;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChangeLevel>()
            .add_systems(Startup, |mut w: MessageWriter<ChangeLevel>| {
                w.write(ChangeLevel(0));
            })
            .add_systems(Update, (level_keys, change_level).chain());
    }
}

/// Moves `n` levels forward (negative: backward). `0` reloads the current one.
#[derive(Message)]
pub struct ChangeLevel(pub isize);

/// Tags everything belonging to the level currently shown.
#[derive(Component)]
pub struct LevelEntity;

/// What's currently on screen, for the HUD.
#[derive(Resource, Default)]
pub struct CurrentLevelStats {
    pub name: String,
    pub meshes: usize,
    pub triangles: usize,
    pub error: Option<String>,
}

fn level_keys(keys: Res<ButtonInput<KeyCode>>, mut w: MessageWriter<ChangeLevel>) {
    if keys.just_pressed(KeyCode::BracketRight) || keys.just_pressed(KeyCode::PageDown) {
        w.write(ChangeLevel(1));
    }
    if keys.just_pressed(KeyCode::BracketLeft) || keys.just_pressed(KeyCode::PageUp) {
        w.write(ChangeLevel(-1));
    }
}

#[allow(clippy::too_many_arguments)]
fn change_level(
    mut requests: MessageReader<ChangeLevel>,
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    old: Query<Entity, With<LevelEntity>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut camera: Query<(&mut Transform, &mut FlyCamera)>,
    mut windows: Query<&mut Window>,
) {
    let Some(step) = requests.read().map(|r| r.0).reduce(|a, b| a + b) else {
        return;
    };
    let count = game.levels.len() as isize;
    game.current = (game.current as isize + step).rem_euclid(count) as usize;

    for e in &old {
        commands.entity(e).despawn();
    }

    let mut stats = CurrentLevelStats { name: game.current_name().to_string(), ..default() };
    match game.load_current() {
        Ok(level) => {
            let built = spawn_level(&level, &mut commands, &mut meshes, &mut materials, &mut images);
            stats.meshes = built.meshes;
            stats.triangles = built.triangles;
            if let Ok((mut transform, mut fly)) = camera.single_mut() {
                *fly = FlyCamera::looking_at_bounds(built.min, built.max, &mut transform);
            }
        }
        Err(why) => stats.error = Some(why),
    }
    if let Ok(mut window) = windows.single_mut() {
        window.title = format!("Gauntlet: Dark Legacy - {}", stats.name);
    }
    info!("level {}: {} meshes, {} triangles", stats.name, stats.meshes, stats.triangles);
    commands.insert_resource(stats);
}

struct Built {
    meshes: usize,
    triangles: usize,
    min: Vec3,
    max: Vec3,
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

/// Merges every placed submesh sharing a (diffuse, lightmap) pair into one
/// mesh, so a level is ~a hundred draw calls rather than tens of thousands.
fn spawn_level(
    level: &LevelData,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> Built {
    let mut groups: HashMap<(u16, u16), MeshBuffers> = HashMap::new();
    let (mut min, mut max) = (Vec3::MAX, Vec3::MIN);

    let placed = level.placements.iter().flat_map(|&(object, at)| {
        level.model.objects[object].submeshes.iter().map(move |s| (s, Vec3::from(at)))
    });
    for (submesh, offset) in placed {
        if submesh.triangles.is_empty() {
            continue;
        }
        let d = submesh.descriptor;
        let buf = groups.entry((d.texture, d.lightmap)).or_default();
        let base = buf.positions.len() as u32;
        for v in &submesh.vertices {
            let p = Vec3::from(v.position) + offset;
            min = min.min(p);
            max = max.max(p);
            buf.positions.push(p.to_array());
            buf.normals.push(v.normal);
            buf.uvs.push(v.uv);
            buf.lightmap_uvs.push(v.lightmap_uv.unwrap_or_default());
            // Raw 0..1 values: the shader multiplies in gamma space.
            buf.colors.push(match v.color {
                Some([r, g, b]) => [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
                None => [1.0; 4],
            });
        }
        // Both windings: the game draws this geometry without culling.
        for t in &submesh.triangles {
            buf.indices.extend(t.iter().map(|i| i + base));
            buf.indices.extend([t[0], t[2], t[1]].iter().map(|i| i + base));
        }
    }

    let mut textures: HashMap<u16, Option<(Handle<Image>, AlphaMode)>> = HashMap::new();
    let mut texture = |binding: u16, images: &mut Assets<Image>| {
        textures
            .entry(binding)
            .or_insert_with(|| {
                let b = level.model.bindings.get(binding as usize)?;
                let image = texture::decode(&level.textures, b).ok()?;
                let alpha = match GameTextureFormat::from_selector(b.format) {
                    Ok(GameTextureFormat::SharedPaletteCi4 | GameTextureFormat::SharedPaletteCi8) => {
                        AlphaMode::Blend
                    }
                    _ if image.has_transparency() => AlphaMode::Mask(0.5),
                    _ => AlphaMode::Opaque,
                };
                Some((images.add(to_bevy_image(image)), alpha))
            })
            .clone()
    };

    let mut triangles = 0;
    let mut count = 0;
    for ((diffuse, lightmap), buf) in groups {
        triangles += buf.indices.len() / 6;
        let (diffuse_image, alpha_mode) = match texture(diffuse, images) {
            Some((image, alpha)) => (Some(image), alpha),
            None => (None, AlphaMode::Opaque),
        };
        let lightmap_image = (lightmap != 0).then(|| texture(lightmap, images)).flatten().map(|(i, _)| i);

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, buf.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, buf.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, buf.uvs)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, buf.colors)
            .with_inserted_indices(Indices::U32(buf.indices));
        if lightmap_image.is_some() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, buf.lightmap_uvs);
        }

        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(LevelMaterial::new(diffuse_image, lightmap_image, alpha_mode))),
            LevelEntity,
        ));
        count += 1;
    }

    Built { meshes: count, triangles, min, max }
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
