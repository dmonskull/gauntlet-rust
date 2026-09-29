//! Turns a level's parsed models, world placements and textures into Bevy
//! meshes, and switches levels at runtime.

use bevy::prelude::*;

use std::collections::HashMap;

use gdl_formats::texmod::{FirstFrame, TexModKind};

use crate::billboard::Billboard;
use crate::texanim::{LevelTexAnims, TexAnim};
use crate::camera::FlyCamera;
use crate::collision_debug::{self, CollisionOverlay};
use crate::level::{LevelData, LoadedGame};
use crate::level_material::LevelMaterial;
use crate::model_mesh::{self, TextureCache};
use crate::population::{self, LevelPopulation, PopulationView};

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

/// The current level's collision, for anything that moves.
#[derive(Resource, Clone)]
pub struct LevelGround(pub std::sync::Arc<gdl_formats::LevelCollision>);

/// Tags everything belonging to the level currently shown.
#[derive(Component)]
pub struct LevelEntity;

/// What's currently on screen, for the HUD.
#[derive(Resource, Default)]
pub struct CurrentLevelStats {
    pub name: String,
    pub meshes: usize,
    pub triangles: usize,
    pub collision_triangles: usize,
    pub error: Option<String>,
    /// World-space bounds of the level geometry.
    pub bounds: Option<(Vec3, Vec3)>,
    /// What populates the level, counted by kind.
    pub population: String,
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
    mut marker_materials: ResMut<Assets<StandardMaterial>>,
    view: Res<PopulationView>,
    overlay: Res<CollisionOverlay>,
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
            // Some level textures animate through frames kept in the always
            // loaded WEAPONS model file (torches).
            let shared = shared_textures(&mut game.install);
            let built = spawn_level(&level, shared.as_ref(), &mut commands, &mut meshes, &mut materials, &mut images);
            collision_debug::spawn_overlay(
                &level.collision,
                overlay.visible,
                &mut commands,
                &mut meshes,
                &mut marker_materials,
            );
            stats.collision_triangles = level.collision.triangles.len();
            stats.meshes = built.meshes;
            stats.triangles = built.triangles;
            stats.bounds = (built.min.x <= built.max.x).then_some((built.min, built.max));
            let spawned = population::spawn(
                &level,
                &mut game.install,
                *view,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut marker_materials,
                &mut images,
            );
            stats.population = spawned.summary;
            let start = level.population.player_start(0);
            if let Ok((mut transform, mut fly)) = camera.single_mut() {
                *fly = match start {
                    Some(start) if camera_at_start() => {
                        FlyCamera::behind(&population::start_transform(&start), &mut transform)
                    }
                    _ => FlyCamera::looking_at_bounds(built.min, built.max, &mut transform),
                };
            }
            commands.insert_resource(LevelGround(std::sync::Arc::new(level.collision)));
            commands.insert_resource(LevelPopulation { level: level.name.clone(), population: level.population });
        }
        Err(why) => stats.error = Some(why),
    }
    if let Ok(mut window) = windows.single_mut() {
        window.title = format!("Gauntlet: Dark Legacy - {}", stats.name);
    }
    info!("level {}: {} meshes, {} triangles", stats.name, stats.meshes, stats.triangles);
    commands.insert_resource(stats);
}

/// `GDL_CAMERA=start` starts the camera behind the player start instead of
/// over the whole level.
fn camera_at_start() -> bool {
    std::env::var("GDL_CAMERA").is_ok_and(|v| v.eq_ignore_ascii_case("start"))
}

struct Built {
    meshes: usize,
    triangles: usize,
    min: Vec3,
    max: Vec3,
}

/// Places every model the world file references and merges them into a few
/// hundred meshes (one per diffuse/lightmap pair).
/// `WEAPONS/objects.ngc` + `textures.ngc`: textures every level can use.
fn shared_textures(install: &mut gdl_install::GameInstall) -> Option<(gdl_formats::ModelFile, Vec<u8>)> {
    let model = gdl_formats::ModelFile::parse(&install.read("WEAPONS/objects.ngc").ok()?).ok()?;
    Some((model, install.read("WEAPONS/textures.ngc").ok()?))
}

fn spawn_level(
    level: &LevelData,
    common: Option<&(gdl_formats::ModelFile, Vec<u8>)>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> Built {
    let mut bounds = (Vec3::MAX, Vec3::MIN);
    let mut cache = TextureCache::new(&level.model, &level.textures);
    // Static instances merge into shared meshes; ones that turn toward the
    // camera stay separate entities (their meshes shared per object).
    let (facing, fixed): (Vec<_>, Vec<_>) =
        level.placements.iter().partition(|&&(_, _, flags)| Billboard::from_flags(flags).is_some());
    let instances = fixed.iter().map(|&&(object, at, flags)| (object, Vec3::from(at), flags));
    let built = model_mesh::build_flagged(&level.model, &mut cache, instances, meshes, materials, images, &mut bounds);
    let mut triangles: usize = built.iter().map(|b| b.triangles).sum();
    let mut count = built.len();
    for b in &built {
        commands.spawn((Mesh3d(b.mesh.clone()), MeshMaterial3d(b.material.clone()), LevelEntity));
    }
    let mut shared: HashMap<(usize, u32), Vec<model_mesh::BuiltMesh>> = HashMap::new();
    for &&(object, at, flags) in &facing {
        let parts = shared.entry((object, flags)).or_insert_with(|| {
            let mut local = (Vec3::MAX, Vec3::MIN);
            model_mesh::build_flagged(&level.model, &mut cache, [(object, Vec3::ZERO, flags)], meshes, materials, images, &mut local)
        });
        let at = Vec3::from(at);
        bounds.0 = bounds.0.min(at);
        bounds.1 = bounds.1.max(at);
        let root = commands
            .spawn((Transform::from_translation(at), Visibility::default(), Billboard::from_flags(flags).unwrap(), LevelEntity))
            .id();
        for p in parts.iter() {
            triangles += p.triangles;
            count += 1;
            commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(root)));
        }
    }
    // Texture animations: every material drawing a modified texture, and
    // the images a flipbook steps through.
    let mut by_binding: HashMap<u16, Vec<Handle<LevelMaterial>>> = HashMap::new();
    for b in built.iter().chain(shared.values().flatten()) {
        by_binding.entry(b.diffuse).or_default().push(b.material.clone());
    }
    let mut shared_cache = common.map(|(model, textures)| TextureCache::new(model, textures));
    let anims: Vec<TexAnim> = level
        .texmods
        .iter()
        .filter_map(|m| {
            let materials = by_binding.get(&m.binding)?.clone();
            let n = m.count.unsigned_abs();
            let frames = match &m.kind {
                TexModKind::Frames(FirstFrame::Binding(first)) => {
                    (0..n).map(|k| cache.get(first + k, images).map(|(image, _)| image)).collect()
                }
                TexModKind::Frames(FirstFrame::Named(name)) => {
                    let find = |model: &gdl_formats::ModelFile| model.texture_names.iter().find(|t| &t.name == name).map(|t| t.binding);
                    match (find(&level.model), common.and_then(|(model, _)| find(model))) {
                        (Some(first), _) => (0..n).map(|k| cache.get(first + k, images).map(|(i, _)| i)).collect(),
                        (None, Some(first)) => {
                            let sc = shared_cache.as_mut()?;
                            (0..n).map(|k| sc.get(first + k, images).map(|(i, _)| i)).collect()
                        }
                        (None, None) => return None,
                    }
                }
                _ => Vec::new(),
            };
            Some(TexAnim::new(m.clone(), materials, frames))
        })
        .collect();
    info!("{} of {} texture animations attached", anims.len(), level.texmods.len());
    commands.insert_resource(LevelTexAnims::new(anims));

    Built { meshes: count, triangles, min: bounds.0, max: bounds.1 }
}
