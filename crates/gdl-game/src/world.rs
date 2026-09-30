//! Turns a level's parsed models, world placements and textures into Bevy
//! meshes, and switches levels at runtime.

use bevy::prelude::*;

use std::collections::{HashMap, HashSet};

use gdl_formats::texmod::{FirstFrame, TexModKind};

use crate::billboard::Billboard;
use crate::texanim::{LevelTexAnims, TexAnim};
use crate::camera::FlyCamera;
use crate::collision_debug::{self, CollisionOverlay};
use crate::level::{LevelData, LoadedGame};
use crate::level_material::LevelMaterial;
use crate::mechanics;
use crate::particles;
use crate::quest;
use crate::model_mesh::{self, TextureCache};
use crate::population::{self, LevelPopulation, PopulationView};

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChangeLevel>()
            .add_systems(Startup, |mut w: MessageWriter<ChangeLevel>| {
                w.write(ChangeLevel(0));
            })
            .add_systems(Update, (level_keys, change_level).chain())
            .add_systems(Update, prewarm);
    }
}

/// Moves `n` levels forward (negative: backward). `0` reloads the current one.
#[derive(Message)]
pub struct ChangeLevel(pub isize);

/// The current level's collision, for anything that moves.
#[derive(Resource, Clone)]
pub struct LevelGround(pub std::sync::Arc<gdl_formats::LevelCollision>);

/// Drawn regardless of the view for its first few frames, so every
/// material's pipeline is built while the level-start shot shows rather
/// than with a hitch the first time the camera turns to it.
#[derive(Component)]
struct Prewarm(u8);

const PREWARM_FRAMES: u8 = 3;

fn prewarm(mut commands: Commands, mut meshes: Query<(Entity, &mut Prewarm)>) {
    for (e, mut p) in &mut meshes {
        if p.0 == PREWARM_FRAMES {
            commands.entity(e).insert(bevy::camera::visibility::NoFrustumCulling);
        }
        if p.0 == 0 {
            commands.entity(e).remove::<(Prewarm, bevy::camera::visibility::NoFrustumCulling)>();
        } else {
            p.0 -= 1;
        }
    }
}

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

/// `]`/Page Down and `[`/Page Up step through the levels.
/// `GDL_TOUR=<seconds>` (testing: memory across level changes) moves on
/// every that many seconds, by `GDL_TOUR_STEP` levels (default 1; 0 reloads
/// the same level).
fn level_keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut tour: Local<Option<(f32, f32, isize)>>,
    mut w: MessageWriter<ChangeLevel>,
) {
    let tour = tour.get_or_insert_with(|| {
        let env = |k: &str| std::env::var(k).ok();
        let every = env("GDL_TOUR").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let step = env("GDL_TOUR_STEP").and_then(|v| v.parse::<isize>().ok()).unwrap_or(1);
        (every, every, step)
    });
    if tour.0 > 0.0 {
        tour.1 -= time.delta_secs();
        if tour.1 <= 0.0 {
            tour.1 = tour.0;
            w.write(ChangeLevel(tour.2));
        }
    }
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
    // The realm last played outside the tower (where the heroes come back).
    mut last_realm: Local<Option<u32>>,
) {
    let Some(step) = requests.read().map(|r| r.0).reduce(|a, b| a + b) else {
        return;
    };
    let count = game.levels.len() as isize;
    game.current = (game.current as isize + step).rem_euclid(count) as usize;

    for e in &old {
        commands.entity(e).despawn();
    }
    // The old level's triggers and movers point at its nodes: gone before
    // the new nodes arrive, so the fixed tick waits for the new level's
    // (built when its population is in).
    commands.remove_resource::<mechanics::Mechanics>();

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
            let entry = population::start_entry(&level.name, *last_realm);
            if let Some((realm, _)) = crate::quest::level_of(&level.name)
                && realm != 13
            {
                *last_realm = Some(realm);
            }
            let start = level.population.player_start(entry);
            if let Ok((mut transform, mut fly)) = camera.single_mut() {
                *fly = match start {
                    Some(start) if camera_at_start() => {
                        FlyCamera::behind(&population::start_transform(&start), &mut transform)
                    }
                    _ => FlyCamera::looking_at_bounds(built.min, built.max, &mut transform),
                };
            }
            commands.insert_resource(LevelGround(std::sync::Arc::new(level.collision)));
            if entry != 0 {
                info!("{}: arriving at start {entry}", level.name);
            }
            commands.insert_resource(LevelPopulation { level: level.name.clone(), population: level.population, entry });
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
    // `GDL_LIST_NEAR="x,y,z"` logs the model placements within 12 units.
    if let Some(v) = std::env::var("GDL_LIST_NEAR").ok().and_then(|s| {
        let v: Vec<f32> = s.split(',').filter_map(|x| x.parse().ok()).collect();
        (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
    }) {
        for (i, &(object, at, flags)) in level.placements.iter().enumerate() {
            let o = &level.model.objects[object];
            let d = Vec3::from(at).distance(v);
            if d < 12.0 {
                let tex: Vec<String> = o.submeshes.iter().map(|m| level.model.texture_names.iter().find(|t| t.binding == m.descriptor.texture).map_or(String::from("?"), |t| t.name.clone())).collect();
                let node = level.placement_nodes.get(i).map(|&n| level.nodes[n].flags);
                info!("near {d:.1}: {} flags {flags:#x} node flags {node:x?} at {at:?} textures {tex:?}", o.name);
            }
        }
    }
    // What triggers and rotators move (`mechanics.rs`) is drawn apart,
    // one entity per moving group, so it can be posed.
    let nodes = mechanics::LevelNodes::new(level.nodes.clone());
    let roots = mechanics::moving_roots(&level.population);
    let group_of = |i: usize| level.placement_nodes.get(i).and_then(|&n| nodes.group_of(n, &roots));
    // Particle-system nodes only mark where effects come from.
    let emitter = |i: usize| {
        level.placement_nodes.get(i).is_some_and(|&n| level.nodes[n].flags & gdl_formats::collision::node_flags::PARTICLES != 0)
    };
    let (facing, fixed): (Vec<_>, Vec<_>) = level
        .placements
        .iter()
        .enumerate()
        .filter(|&(i, _)| !emitter(i))
        .partition(|&(i, &(_, _, flags))| group_of(i).is_none() && Billboard::from_flags(flags).is_some());
    let (moving, fixed): (Vec<_>, Vec<_>) = fixed.into_iter().partition(|&(i, _)| group_of(i).is_some());
    // The tower's exit glows (`L1NSNC<realm><n>_ACTIVE`) are drawn apart
    // too, so a shut exit can hide its own (`quest.rs`).
    let glows: HashMap<usize, quest::ExitGlow> =
        level.nodes.iter().enumerate().filter_map(|(n, node)| quest::ExitGlow::named(&node.name).map(|g| (n, g))).collect();
    let glow_roots: HashSet<usize> = glows.keys().copied().collect();
    let glow_of = |i: usize| level.placement_nodes.get(i).and_then(|&n| nodes.group_of(n, &glow_roots));
    let (glowing, fixed): (Vec<_>, Vec<_>) = fixed.into_iter().partition(|&(i, _)| glow_of(i).is_some());
    let instances = fixed.iter().map(|&(_, &(object, at, flags))| (object, Vec3::from(at), flags));
    let mut built = model_mesh::build_flagged(&level.model, &mut cache, instances, meshes, materials, images, &mut bounds);
    let mut triangles: usize = built.iter().map(|b| b.triangles).sum();
    let mut count = built.len();
    for b in &built {
        commands.spawn((Mesh3d(b.mesh.clone()), MeshMaterial3d(b.material.clone()), LevelEntity, Prewarm(PREWARM_FRAMES)));
    }
    let mut groups: HashMap<usize, Vec<(usize, Vec3, u32)>> = HashMap::new();
    for &(i, &(object, at, flags)) in &moving {
        groups.entry(group_of(i).unwrap()).or_default().push((object, Vec3::from(at), flags));
    }
    for (root, instances) in groups {
        debug!(
            "moving group {} ({}): {:?}",
            root,
            nodes.nodes[root].name,
            instances.iter().map(|&(o, _, f)| format!("{} {f:#x}", level.model.objects[o].name)).collect::<Vec<_>>()
        );
        let parts = model_mesh::build_flagged(&level.model, &mut cache, instances, meshes, materials, images, &mut bounds);
        let group = commands.spawn((mechanics::MovingGroup::new(root), Transform::IDENTITY, Visibility::default(), LevelEntity)).id();
        for p in &parts {
            triangles += p.triangles;
            count += 1;
            commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(group)));
        }
        built.extend(parts);
    }
    let mut glow_groups: HashMap<usize, Vec<(usize, Vec3, u32)>> = HashMap::new();
    for &(i, &(object, at, flags)) in &glowing {
        glow_groups.entry(glow_of(i).unwrap()).or_default().push((object, Vec3::from(at), flags));
    }
    for (root, instances) in glow_groups {
        let parts = model_mesh::build_flagged(&level.model, &mut cache, instances, meshes, materials, images, &mut bounds);
        let group = commands.spawn((glows[&root], Transform::IDENTITY, Visibility::default(), LevelEntity)).id();
        for p in &parts {
            triangles += p.triangles;
            count += 1;
            commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(group)));
        }
        built.extend(parts);
    }
    let facing: Vec<_> = facing.into_iter().map(|(_, p)| p).collect();
    let origins = nodes.origin.clone();
    commands.insert_resource(nodes);
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

    // The particle systems its `PSYS` nodes run.
    let emitters = level.nodes.iter().enumerate().filter(|(_, n)| {
        n.flags & gdl_formats::collision::node_flags::PARTICLES != 0 && n.name.contains("PSYS")
    });
    let mut emitters: Vec<(String, Vec3)> = emitters.map(|(i, n)| (n.name.clone(), Vec3::from(origins[i]))).collect();
    // `GDL_PARTICLE_TEST=<letter>`: that record's system 6 units in front of
    // the player start, in the open (test with `GDL_LOOK_AT` on it).
    if let (Ok(letter), Some(start)) = (std::env::var("GDL_PARTICLE_TEST"), level.population.player_start(0)) {
        let ahead = Vec3::new(start.yaw.sin(), 0.0, start.yaw.cos()) * 6.0;
        let at = Vec3::from(start.position) + ahead + Vec3::Y * 3.0;
        info!("GDL_PARTICLE_TEST: record {letter} at {at}");
        emitters.push((format!("TESTPSYS{letter}"), at));
    }
    let find = |model: &gdl_formats::ModelFile, name: &str| {
        model.texture_names.iter().find(|t| t.name.eq_ignore_ascii_case(name)).map(|t| t.binding)
    };
    let texture = |name: &str| -> Option<Handle<Image>> {
        if let Some(b) = find(&level.model, name) {
            return cache.get(b, images).map(|(i, _)| i);
        }
        let (model, _) = common?;
        let b = find(model, name)?;
        shared_cache.as_mut()?.get(b, images).map(|(i, _)| i)
    };
    let made = particles::spawn_emitters(&level.particles, emitters, texture, commands, meshes, materials);
    info!("{made} particle systems ({} records)", level.particles.len());

    Built { meshes: count, triangles, min: bounds.0, max: bounds.1 }
}
