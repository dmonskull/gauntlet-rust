//! The player: one hero walking the current level from its start point.
//! Movement runs on the game's fixed 30 Hz tick (`locomotion.rs`), resolved
//! against the level's collision, and is interpolated for drawing so it
//! looks smooth at any frame rate.
//!
//! Controls: left stick or WASD/arrows to move (Shift walks).

use bevy::prelude::*;
use gdl_formats::MoveParams;
use gdl_formats::pdata::PlayerStats;

use crate::camera::FreeLook;
use crate::character::{self, Animator, CharacterData};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion::{self, Mover, Stick, Switch};
use crate::play_camera::PlayCamera;
use crate::population::LevelPopulation;
use crate::world::{LevelEntity, LevelGround};

/// Stand-ins until the player's own collision values are decoded: wall
/// radius and floor step for the generic actor mover.
const PLAYER_RADIUS: f32 = 1.0;
const PLAYER_STEP: f32 = 2.0;
/// The actor mover drops at most 16 units per second.
const MAX_DROP_PER_SECOND: f32 = 16.0;

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
}

#[derive(Component)]
pub struct Player {
    pub mover: Mover,
    /// Position and facing before the latest tick, for interpolation.
    previous: ([f32; 3], f32),
}

/// Player movement; the play camera ticks after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerTick;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(locomotion::TICK_HZ))
            .add_systems(Startup, load_hero)
            .add_systems(FixedUpdate, tick.in_set(PlayerTick))
            .add_systems(
                Update,
                (spawn_player.run_if(resource_exists_and_changed::<LevelPopulation>), interpolate).chain(),
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
    commands.insert_resource(Hero { data, speed });
}

#[allow(clippy::too_many_arguments)]
fn spawn_player(
    mut commands: Commands,
    hero: Option<Res<Hero>>,
    population: Res<LevelPopulation>,
    ground: Option<Res<LevelGround>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(hero), Some(ground)) = (hero, ground) else { return };
    let collision = &ground.0;
    let (mut feet, facing) = match population.player_start() {
        Some(s) => (s.position, s.yaw),
        None => {
            let [lo, hi] = collision.bounds;
            (std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0), 0.0)
        }
    };
    // Stand on the floor under the start.
    if let Some(y) = collision.floor_height(feet).or_else(|| collision.top_floor(feet[0], feet[2])) {
        feet[1] = y;
    }
    let transform = Transform::from_translation(Vec3::from(feet)).with_rotation(Quat::from_rotation_y(facing));
    let (root, _, _) =
        character::spawn_character(&hero.data, transform, &mut commands, &mut meshes, &mut materials, &mut images);
    let mover = Mover::new(feet, facing, hero.speed);
    commands.entity(root).insert((Player { mover, previous: (feet, facing) }, LevelEntity));
    info!("player starts at {feet:?} facing {:.0} deg", facing.to_degrees());
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

#[allow(clippy::too_many_arguments)]
fn tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    free_look: Res<FreeLook>,
    play_camera: Option<Res<PlayCamera>>,
    ground: Option<Res<LevelGround>>,
    mut players: Query<(&mut Player, &mut Animator)>,
) {
    let dt = time.delta_secs();
    let raw = if free_look.0 { Vec2::ZERO } else { read_stick(&keys, &pads) };
    // Stick up moves the way the play camera faces.
    let yaw = play_camera.map_or(0.0, |c| c.rig.yaw);
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let dir = right * raw.x + forward * raw.y;
    let stick = Stick { heading: dir.x.atan2(dir.z), magnitude: raw.length().min(1.0) };
    let params = MoveParams::new(PLAYER_RADIUS, PLAYER_STEP, MAX_DROP_PER_SECOND * dt);

    for (mut player, mut animator) in &mut players {
        player.previous = (player.mover.position, player.mover.facing);
        let d = player.mover.step(stick, dt);
        let feet = player.mover.position;
        let d = match &ground {
            Some(g) => g.0.move_actor(feet, d, &params).delta,
            None => d,
        };
        player.mover.position = std::array::from_fn(|i| feet[i] + d[i]);
        trace!("player at {:?}", player.mover.position);

        // The game's action chaining; the new action's factors apply from
        // the next tick.
        let current = animator.action_name().to_string();
        let t = locomotion::transition(&current, player.mover.gait);
        let switch = match t.switch {
            Switch::Now => t.action != current,
            Switch::AtEnd => t.action != current && animator.finished(),
        };
        if switch && animator.play_blended(t.action, t.blend) {
            player.mover.factors = locomotion::action_factors(t.action);
            trace!("action {current} -> {}", t.action);
        }
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
