//! Drives the Bevy camera from the game's play camera (`camera_rig.rs`):
//! built from the level's camera points and its `WDATA` camera record when
//! the level loads, ticked at 30 Hz after the player, and interpolated
//! between ticks. `C` toggles the free-fly camera instead.

use std::collections::HashMap;

use bevy::prelude::*;
use gdl_formats::population::LocatorKind;
use gdl_formats::{LevelCamera, LevelLight, WorldData};

use crate::camera::{FlyCamera, FreeLook};
use crate::camera_rig::{CameraPoint, CameraRig};
use crate::level::LoadedGame;
use crate::level_material::SceneLight;
use crate::player::{Player, PlayerTick};
use crate::population::LevelPopulation;
use crate::world::LevelGround;

/// The game's field of view: 60° across a 4:3 picture, i.e. this much
/// vertically. Kept vertical so wider screens see more to the sides.
fn vertical_fov() -> f32 {
    2.0 * (0.75 * 30f32.to_radians().tan()).atan()
}

pub struct PlayCameraPlugin;

impl Plugin for PlayCameraPlugin {
    fn build(&self, app: &mut App) {
        // GDL_FREE_CAMERA=1 starts in the free camera (level overviews).
        let free = std::env::var("GDL_FREE_CAMERA").is_ok_and(|v| !v.is_empty() && v != "0");
        app.insert_resource(FreeLook(free))
            .add_systems(Startup, (load_level_cameras, set_fov))
            .add_systems(FixedUpdate, tick.after(PlayerTick))
            .add_systems(
                Update,
                (
                    start.run_if(resource_exists_and_changed::<LevelPopulation>),
                    toggle,
                    place.run_if(resource_exists::<PlayCamera>),
                )
                    .chain(),
            );
    }
}

/// Each level's camera record and light, by lower-case level folder
/// (`levela1`).
#[derive(Resource, Default)]
struct LevelCameras(HashMap<String, (LevelCamera, LevelLight)>);

#[derive(Resource)]
pub struct PlayCamera {
    pub rig: CameraRig,
    /// Eye and target before the latest tick, for interpolation.
    previous: ([f32; 3], [f32; 3]),
}

fn load_level_cameras(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let mut cameras = LevelCameras::default();
    let wads: Vec<String> = game
        .install
        .files()
        .iter()
        .filter(|f| {
            let f = f.to_ascii_uppercase();
            f.starts_with("WDATA/") && f.ends_with(".WAD")
        })
        .cloned()
        .collect();
    for path in wads {
        match game.install.read(&path).map_err(|e| e.to_string()).and_then(|b| WorldData::parse(&b).map_err(|e| e.to_string())) {
            Ok(world) => {
                for level in &world.levels {
                    cameras
                        .0
                        .insert(level.folder().to_ascii_lowercase(), (world.cameras[level.camera].clone(), level.light));
                }
            }
            Err(e) => warn!("{path}: {e}"),
        }
    }
    commands.insert_resource(cameras);
}

fn set_fov(mut projections: Query<&mut Projection, With<Camera3d>>) {
    for mut p in &mut projections {
        if let Projection::Perspective(p) = p.as_mut() {
            p.fov = vertical_fov();
        }
    }
}

fn start(
    mut commands: Commands,
    population: Res<LevelPopulation>,
    ground: Option<Res<LevelGround>>,
    cameras: Option<Res<LevelCameras>>,
    mut scene_light: ResMut<SceneLight>,
) {
    let Some(ground) = ground else { return };
    let records = cameras.and_then(|c| c.0.get(&population.level.to_ascii_lowercase()).cloned());
    // The level's light lights everything that isn't prelit.
    *scene_light = records.as_ref().map_or_else(SceneLight::default, |(_, light)| SceneLight::new(light));
    let record = records
        .map(|(c, _)| c)
        .unwrap_or(LevelCamera { mode: 0, pitch_limit: 0.35, bounds: None, near: 24.0, far: 32.0 });
    // Only plain camera points are picked by distance; starting, intro and
    // trigger cameras are chosen by events.
    let points: Vec<CameraPoint> = population
        .population
        .locators
        .iter()
        .filter(|l| l.kind == LocatorKind::Transmitter(2))
        .map(|l| CameraPoint { position: l.position, yaw: l.rotation[1], pitch: l.rotation[0] })
        .collect();
    let [lo, hi] = ground.0.bounds;
    let bounds = record.target_bounds(lo, hi);
    let focus = population.player_start().map_or([0.0; 3], |s| s.position);
    let rig = CameraRig::new(points, bounds, record.near, focus);
    let previous = (rig.eye(), rig.target);
    commands.insert_resource(PlayCamera { rig, previous });
}

fn tick(camera: Option<ResMut<PlayCamera>>, players: Query<&Player>) {
    let (Some(mut camera), Ok(player)) = (camera, players.single()) else { return };
    camera.previous = (camera.rig.eye(), camera.rig.target);
    camera.rig.tick(player.mover.position);
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut free_look: ResMut<FreeLook>,
    mut camera: Query<(&Transform, &mut FlyCamera)>,
) {
    if keys.just_pressed(KeyCode::KeyC) {
        free_look.0 = !free_look.0;
        if let Ok((transform, mut fly)) = camera.single_mut() {
            fly.sync(transform);
        }
    }
}

fn place(
    fixed: Res<Time<Fixed>>,
    free_look: Res<FreeLook>,
    play: Res<PlayCamera>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    if free_look.0 {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else { return };
    let t = fixed.overstep_fraction();
    let eye = Vec3::from(play.previous.0).lerp(Vec3::from(play.rig.eye()), t);
    let target = Vec3::from(play.previous.1).lerp(Vec3::from(play.rig.target), t);
    *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}
