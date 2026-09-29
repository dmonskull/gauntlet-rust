//! The player: one hero walking the current level. Movement runs on the
//! game's fixed 30 Hz tick (`locomotion.rs`) and is interpolated for
//! drawing, so it looks smooth at any frame rate. The camera follows unless
//! free look is on (`C` toggles).
//!
//! Controls: left stick or WASD/arrows to move (Shift walks), `C` free look.

use bevy::prelude::*;
use gdl_formats::pdata::PlayerStats;

use crate::camera::FreeLook;
use crate::character::{self, Animator, CharacterData};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion::{self, Mover, Stick};
use crate::world::{CurrentLevelStats, LevelEntity};

pub struct PlayerPlugin;

/// Which hero to play, from the command line.
#[derive(Resource, Clone)]
pub struct PlayerChoice {
    pub class: String,
    pub variant: String,
}

/// The loaded hero, spawned again on every level.
#[derive(Resource)]
struct Hero {
    data: CharacterData,
    speed: f32,
    /// Rest-pose height, for the camera.
    height: f32,
}

#[derive(Component)]
pub struct Player {
    pub mover: Mover,
    /// Position and facing before the latest tick, for interpolation.
    previous: ([f32; 3], f32),
}

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(locomotion::TICK_HZ))
            .insert_resource(FreeLook(false))
            .add_systems(Startup, load_hero)
            .add_systems(FixedUpdate, tick)
            .add_systems(
                Update,
                (
                    spawn_player.run_if(resource_exists_and_changed::<CurrentLevelStats>),
                    toggle_camera,
                    (interpolate, follow_camera).chain(),
                ),
            );
    }
}

fn load_hero(mut commands: Commands, mut game: ResMut<LoadedGame>, choice: Res<PlayerChoice>) {
    let install = &mut game.install;
    let data = match character::load_player(install, &choice.class, &choice.variant) {
        Ok(data) => data,
        Err(why) => {
            error!("can't load player {}/{}: {why}", choice.class, choice.variant);
            return;
        }
    };
    let stats = install
        .read(&format!("PDATA/{}.WAD", choice.class))
        .ok()
        .and_then(|b| PlayerStats::parse(&b).ok().flatten());
    let speed_stat = stats.map_or(400.0, |s| locomotion::stat_at_level(s.speed.start, s.speed.max, 1, 0.0));
    let speed = locomotion::move_speed(speed_stat, 0.0);
    info!("player {}: speed stat {speed_stat} -> {speed:.2} units/s", data.name);
    commands.insert_resource(Hero { data, speed, height: 0.0 });
}

#[allow(clippy::too_many_arguments)]
fn spawn_player(
    mut commands: Commands,
    hero: Option<ResMut<Hero>>,
    level: Res<CurrentLevelStats>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut hero) = hero else { return };
    let Some((min, max)) = level.bounds else { return };
    // Until the level's own player starts are wired in: the middle of the
    // level, facing +Z.
    let start = (min + max) / 2.0;
    let transform = Transform::from_translation(start);
    let (root, lo, hi) = character::spawn_character(&hero.data, transform, &mut commands, &mut meshes, &mut materials, &mut images);
    hero.height = (hi.y - lo.y).max(1.0);
    let mover = Mover::new(start.to_array(), 0.0, hero.speed);
    commands.entity(root).insert((Player { mover, previous: (start.to_array(), 0.0) }, LevelEntity));
}

/// The stick, in the camera's frame: +Y away from the camera, +X right.
fn read_stick(keys: &ButtonInput<KeyCode>, pads: &Query<&Gamepad>) -> Vec2 {
    if let Some(v) = std::env::var("GDL_STICK").ok().and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
    }) {
        return v;
    }
    for pad in pads {
        let v = pad.left_stick();
        if v.length() > 0.0 {
            return v.clamp_length_max(1.0);
        }
    }
    let axis = |pos: [KeyCode; 2], neg: [KeyCode; 2]| {
        (pos.iter().any(|k| keys.pressed(*k)) as i32 - neg.iter().any(|k| keys.pressed(*k)) as i32) as f32
    };
    let v = Vec2::new(
        axis([KeyCode::KeyD, KeyCode::ArrowRight], [KeyCode::KeyA, KeyCode::ArrowLeft]),
        axis([KeyCode::KeyW, KeyCode::ArrowUp], [KeyCode::KeyS, KeyCode::ArrowDown]),
    )
    .normalize_or_zero();
    // Keyboard runs; Shift walks.
    if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) { v * 0.5 } else { v }
}

fn tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    free_look: Res<FreeLook>,
    camera: Query<&Transform, (With<Camera3d>, Without<Player>)>,
    mut players: Query<(&mut Player, &mut Animator)>,
) {
    let raw = if free_look.0 { Vec2::ZERO } else { read_stick(&keys, &pads) };
    let (forward, right) = camera.single().map_or((Vec3::Z, Vec3::NEG_X), |t| {
        let f = Vec3::new(t.forward().x, 0.0, t.forward().z).normalize_or(Vec3::Z);
        (f, Vec3::new(-f.z, 0.0, f.x))
    });
    let dir = right * raw.x + forward * raw.y;
    let stick = Stick { heading: dir.x.atan2(dir.z), magnitude: raw.length().min(1.0) };

    for (mut player, mut animator) in &mut players {
        player.previous = (player.mover.position, player.mover.facing);
        let d = player.mover.step(stick, time.delta_secs());
        let p = &mut player.mover.position;
        for (a, b) in p.iter_mut().zip(d) {
            *a += b;
        }
        animator.play_named(player.mover.gait.action());
    }
}

fn interpolate(fixed: Res<Time<Fixed>>, mut players: Query<(&Player, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (player, mut transform) in &mut players {
        let (p0, f0) = player.previous;
        let (p1, f1) = (player.mover.position, player.mover.facing);
        transform.translation = Vec3::from(p0).lerp(Vec3::from(p1), t);
        transform.rotation = Quat::from_rotation_y(f0 + locomotion::wrap(f1 - f0) * t);
    }
}

fn toggle_camera(keys: Res<ButtonInput<KeyCode>>, mut free_look: ResMut<FreeLook>) {
    if keys.just_pressed(KeyCode::KeyC) {
        free_look.0 = !free_look.0;
    }
}

/// Looks down on the hero from behind and above, like the game's default
/// view, easing after it.
fn follow_camera(
    time: Res<Time>,
    free_look: Res<FreeLook>,
    hero: Option<Res<Hero>>,
    players: Query<&Transform, With<Player>>,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
) {
    if free_look.0 {
        return;
    }
    let (Some(hero), Ok(player), Ok(mut cam)) = (hero, players.single(), camera.single_mut()) else { return };
    let target = player.translation + Vec3::Y * hero.height * 0.5;
    let eye = target + Vec3::new(0.0, 0.8, -0.6).normalize() * hero.height * 9.0;
    let k = 1.0 - (-8.0 * time.delta_secs()).exp();
    cam.translation = cam.translation.lerp(eye, k);
    cam.look_at(target, Vec3::Y);
}
