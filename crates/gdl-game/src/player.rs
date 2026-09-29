//! The player: one hero walking and fighting through the current level from
//! its start point. Everything runs on the game's fixed 30 Hz tick: the
//! controls become the game's logical buttons and an intent, the target
//! search and action chaining pick the next action (`combat.rs`,
//! `actions.rs`), blows are resolved into [`Hit`] messages, and movement
//! (`locomotion.rs`) is resolved against the level's collision and
//! interpolated for drawing so it looks smooth at any frame rate.
//!
//! Controls (the game's default GameCube scheme, `docs/combat.md`):
//!
//! | game | pad | keyboard |
//! | --- | --- | --- |
//! | move | left stick | WASD / arrows (Shift walks) |
//! | attack (A) | south | J |
//! | power attack (Y) | north | L |
//! | turbo / defend (B) | west | H |
//! | magic (X) | east | U |
//! | charge (L) | left trigger | P |
//! | strafe (R) | right trigger | O |
//! | combo move (Z) | right bumper | G |
//!
//! `GDL_STICK="x,y"` holds the stick; `GDL_BUTTONS="attack@10-12,power"`
//! holds buttons (`attack`, `power`, `turbo`, `magic`, `charge`, `strafe`,
//! `combo`), each for the whole run or for a range of ticks since the hero
//! appeared.

use bevy::prelude::*;
use gdl_formats::{PlayerCollision, PlayerGround};
use gdl_formats::pdata::PlayerStats;

use crate::actions::{self, Action, ActionState, Env};
use crate::camera::FreeLook;
use crate::character::{self, Animator, CharacterData};
use crate::combat::{self, Buttons, Hit, Intent, TargetKind, Targetable, button};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion::{self, Mover, Stick, wrap};
use crate::play_camera::PlayCamera;
use crate::player_state::PlayerState;
use crate::population::LevelPopulation;
use crate::world::{LevelEntity, LevelGround};

/// The hero's radius if the class data can't be read (every class has 1.5).
const DEFAULT_RADIUS: f32 = 1.5;
/// The game's "attack aim" and "walk-into attack" pad options, both on by
/// default: attacking in place turns the hero toward the target, and walking
/// into a monster attacks it.
const ATTACK_AIM: bool = true;
const WALK_INTO_ATTACK: bool = true;
/// Stand-in: the turbo meter isn't modelled, so it stays empty (no turbo
/// attacks, charge or co-op combos).
const TURBO_METER: f32 = 0.0;
/// Lunges and strafe attacks drift forward at this stick magnitude when the
/// stick is released.
const DRIFT: f32 = 0.5;

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
    /// The class's stats, for re-deriving them when the level rises.
    stats: Option<PlayerStats>,
    armor: f32,
    data: CharacterData,
    speed: f32,
    strength: f32,
    radius: f32,
    class: Option<usize>,
}

#[derive(Component)]
pub struct Player {
    pub mover: Mover,
    /// The floor being followed and the node stood on, between ticks.
    pub ground: PlayerGround,
    /// Where the player (re)starts the level.
    pub start: ([f32; 3], f32),
    /// Position and facing before the latest tick, for interpolation.
    previous: ([f32; 3], f32),
    /// The action playing and what the chaining keeps between ticks.
    pub actions: ActionState,
    /// Last tick's requested action.
    request: Action,
    /// The playing clip and its frame at the last tick, to see a looping
    /// clip come round.
    last_clip: (usize, f32),
    /// Movement factor of the action the chaining last saw playing; it
    /// scales the next tick's step.
    move_factor: f32,
    /// Damage per blow.
    pub strength: f32,
    /// Derived armour, 0–5: taken off every blow that armour stops.
    pub armor: f32,
    /// Collision radius: reach and range bands are measured from it.
    pub radius: f32,
    /// The game's class index.
    class: Option<usize>,
}

/// The hero's working stats at a character level: strength (5–20),
/// armour (0–5) and speed (units/s), each from the class stat plus 5 per
/// level up to its maximum.
fn derived_stats(stats: Option<&PlayerStats>, level: u32) -> (f32, f32, f32) {
    let at = |s: gdl_formats::pdata::Stat| locomotion::stat_at_level(s.start, s.max, level, 0.0);
    let strength = combat::strength(stats.map_or(400.0, |s| at(s.strength)));
    let armor = (0.005 * stats.map_or(0.0, |s| at(s.armor))).clamp(0.0, 5.0);
    let speed = locomotion::move_speed(stats.map_or(400.0, |s| at(s.speed)), 0.0);
    (strength, armor, speed)
}

/// Re-derives the hero's stats when its level changes.
fn level_stats(
    hero: Option<Res<Hero>>,
    state: Option<Res<PlayerState>>,
    mut applied: Local<u32>,
    mut players: Query<&mut Player>,
) {
    let (Some(hero), Some(state)) = (hero, state) else { return };
    let fresh = players.iter_mut().any(|p| p.is_added());
    if state.level == *applied && !fresh {
        return;
    }
    *applied = state.level;
    let (strength, armor, speed) = derived_stats(hero.stats.as_ref(), state.level);
    for mut p in &mut players {
        p.strength = strength;
        p.armor = armor;
        p.mover.speed = speed;
    }
    info!("level {}: strength {strength:.1}, armour {armor:.2}, speed {speed:.2}", state.level);
}

impl Player {
    /// Moves the hero instantly (no interpolation smear), standing on a
    /// fresh floor.
    pub fn teleport(&mut self, at: [f32; 3], facing: f32) {
        self.mover.position = at;
        self.mover.facing = facing;
        self.ground = PlayerGround::new(at[1]);
        self.previous = (at, facing);
    }
}

/// Player movement; the play camera ticks after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerTick;

/// Spawning the hero on a new level; things placed relative to it go after.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerSpawn;

/// Controls between ticks: last tick's buttons (for presses), scripted
/// buttons, and ticks since the hero appeared.
#[derive(Resource, Default)]
struct Controls {
    held: u32,
    script: Vec<(u32, Option<(u64, u64)>)>,
    ticks: u64,
}

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(locomotion::TICK_HZ))
            .insert_resource(Controls { script: button_script(), ..default() })
            .add_systems(Startup, load_hero)
            .add_systems(FixedUpdate, tick.in_set(PlayerTick))
            .add_systems(Update, level_stats)
            .add_systems(
                Update,
                (spawn_player.run_if(resource_exists_and_changed::<LevelPopulation>).in_set(PlayerSpawn), interpolate)
                    .chain(),
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
    let at_level_1 = |s: gdl_formats::pdata::Stat| locomotion::stat_at_level(s.start, s.max, 1, 0.0);
    let speed_stat = stats.map_or(400.0, |s| at_level_1(s.speed));
    let speed = locomotion::move_speed(speed_stat, 0.0);
    let strength = combat::strength(stats.map_or(400.0, |s| at_level_1(s.strength)));
    let radius = stats.map_or(DEFAULT_RADIUS, |s| s.body.radius);
    // Armour works like the other derived stats: 0.001 × stat across 0–5.
    let armor = (0.005 * stats.map_or(0.0, |s| at_level_1(s.armor))).clamp(0.0, 5.0);
    info!(
        "player {}: speed stat {speed_stat} -> {speed:.2} units/s, strength {strength:.1}, armour {armor:.2}, radius {radius}",
        data.name
    );
    let class = character::class_index(&data.class);
    commands.insert_resource(Hero { stats, data, speed, strength, armor, radius, class });
}

#[allow(clippy::too_many_arguments)]
fn spawn_player(
    mut commands: Commands,
    hero: Option<Res<Hero>>,
    population: Res<LevelPopulation>,
    ground: Option<Res<LevelGround>>,
    mut controls: ResMut<Controls>,
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
    let player = Player {
        mover: Mover::new(feet, facing, hero.speed),
        ground: PlayerGround::new(feet[1]),
        start: (feet, facing),
        previous: (feet, facing),
        actions: ActionState::default(),
        request: Action::READY,
        last_clip: (0, 0.0),
        move_factor: 1.0,
        strength: hero.strength,
        armor: hero.armor,
        radius: hero.radius,
        class: hero.class,
    };
    commands.entity(root).insert((player, LevelEntity));
    controls.ticks = 0;
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

/// The logical buttons a named control (for `GDL_BUTTONS`) holds.
fn named_button(name: &str) -> Option<u32> {
    Some(match name {
        "attack" | "a" => button::QUICK,
        "power" | "y" => button::POWER,
        "turbo" | "defend" | "b" => button::TURBO | button::DEFEND,
        "magic" | "x" => button::MAGIC,
        "charge" | "l" => button::CHARGE,
        "strafe" | "r" => button::STRAFE,
        "combo" | "z" => button::COMBO_MOVE,
        _ => return None,
    })
}

/// `GDL_BUTTONS`: `name` (held throughout) or `name@from-to` (held for
/// those ticks, inclusive), comma-separated.
fn button_script() -> Vec<(u32, Option<(u64, u64)>)> {
    let Ok(spec) = std::env::var("GDL_BUTTONS") else { return Vec::new() };
    spec.split(',')
        .filter_map(|item| {
            let item = item.trim().to_ascii_lowercase();
            let (name, range) = match item.split_once('@') {
                Some((name, range)) => {
                    let (a, b) = range.split_once('-').unwrap_or((range, range));
                    (name.to_string(), Some((a.parse().ok()?, b.parse().ok()?)))
                }
                None => (item.clone(), None),
            };
            let bits = named_button(&name);
            if bits.is_none() {
                warn!("GDL_BUTTONS: unknown button {name:?}");
            }
            Some((bits?, range))
        })
        .collect()
}

/// The logical buttons held now, from the keyboard, pads and script. The
/// game's default scheme puts turbo and defend on the same button.
fn read_buttons(keys: &ButtonInput<KeyCode>, pads: &Query<&Gamepad>, controls: &Controls) -> u32 {
    const KEYS: [(KeyCode, u32); 7] = [
        (KeyCode::KeyJ, button::QUICK),
        (KeyCode::KeyL, button::POWER),
        (KeyCode::KeyH, button::TURBO | button::DEFEND),
        (KeyCode::KeyU, button::MAGIC),
        (KeyCode::KeyP, button::CHARGE),
        (KeyCode::KeyO, button::STRAFE),
        (KeyCode::KeyG, button::COMBO_MOVE),
    ];
    // By position on the pad: A south, B west, X east, Y north.
    const PAD: [(GamepadButton, u32); 8] = [
        (GamepadButton::South, button::QUICK),
        (GamepadButton::North, button::POWER),
        (GamepadButton::West, button::TURBO | button::DEFEND),
        (GamepadButton::East, button::MAGIC),
        (GamepadButton::LeftTrigger2, button::CHARGE),
        (GamepadButton::LeftTrigger, button::CHARGE),
        (GamepadButton::RightTrigger2, button::STRAFE),
        (GamepadButton::RightTrigger, button::COMBO_MOVE),
    ];
    let mut held = 0;
    for (key, bits) in KEYS {
        if keys.pressed(key) {
            held |= bits;
        }
    }
    for pad in pads {
        for (b, bits) in PAD {
            if pad.pressed(b) {
                held |= bits;
            }
        }
    }
    for &(bits, range) in &controls.script {
        if range.is_none_or(|(a, b)| (a..=b).contains(&controls.ticks)) {
            held |= bits;
        }
    }
    held
}

/// The clip an action plays: its own, or for the low power finisher the
/// close finisher's if the class has none, else the first clip.
fn clip_for(animator: &Animator, action: Action) -> usize {
    let find = |name: &str| animator.clips.actions.iter().position(|a| a.name == name);
    find(action.name())
        .or_else(|| match action {
            Action::ATTPWRALOW => find(Action::ATTPWRACLOSE.name()),
            Action::ATTPWRALOWR => find(Action::ATTPWRACLOSER.name()),
            _ => None,
        })
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    free_look: Res<FreeLook>,
    play_camera: Option<Res<PlayCamera>>,
    ground: Option<Res<LevelGround>>,
    mut controls: ResMut<Controls>,
    mut players: Query<(Entity, &mut Player, &mut Animator)>,
    targets: Query<(Entity, &GlobalTransform, &Targetable)>,
    mut hits: MessageWriter<Hit>,
    state: Option<Res<PlayerState>>,
) {
    // A dead hero lies still until it's revived.
    if state.is_some_and(|s| !s.alive) {
        return;
    }
    let dt = time.delta_secs();
    controls.ticks += 1;
    let raw = if free_look.0 { Vec2::ZERO } else { read_stick(&keys, &pads) };
    let held = if free_look.0 { 0 } else { read_buttons(&keys, &pads, &controls) };
    let buttons = Buttons::from_held(held, controls.held);
    controls.held = held;
    // Stick up moves the way the play camera faces.
    let yaw = play_camera.map_or(0.0, |c| c.rig.yaw);
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let dir = right * raw.x + forward * raw.y;
    let stick = Stick { heading: dir.x.atan2(dir.z), magnitude: raw.length().min(1.0) };
    let body = PlayerCollision::default();
    let candidates = || targets.iter().map(|(e, t, target)| (e, t.translation(), target));

    for (entity, mut player, mut animator) in &mut players {
        let p = &mut *player;
        p.previous = (p.mover.position, p.mover.facing);
        let position = Vec3::from(p.mover.position);
        let facing = p.mover.facing;
        let current = p.actions.action;

        // What the controls ask for. The action playing may hold the stick
        // back (magic, defending); lunges drift on without it.
        let magnitude = stick.magnitude * actions::stick_scale(current);
        let mut intent = combat::classify(buttons, stick.magnitude, wrap(stick.heading - facing), TURBO_METER);
        p.actions.observe_buttons(buttons.held);
        let keeps_facing = intent.keeps_facing();
        let drive = if magnitude == 0.0 && !keeps_facing && actions::drifts_forward(current) { DRIFT } else { magnitude };
        let wanted = if magnitude > 0.0 && !keeps_facing { stick.heading } else { facing };

        // The target the hero is heading for, and whether it walked into it.
        let found = combat::search(position, wanted, candidates());
        let mut walked_into = false;
        if WALK_INTO_ATTACK
            && matches!(intent, Intent::Walk | Intent::Run)
            && p.actions.edges == 0
            && !(0x27..=0x72).contains(&current.0)
            && found.is_some_and(|f| f.distance < combat::WALK_INTO + p.radius && f.attacked_by_walking_into())
        {
            intent = Intent::Quick;
            walked_into = true;
        }
        p.actions.range = combat::range(found.as_ref(), p.radius, intent.is_attack() && !walked_into);
        let strafing = held & (button::DEFEND | button::STRAFE) != 0
            || (0x09..=0x10).contains(&current.0)
            || (0x47..=0x4E).contains(&current.0);
        let aim = match found {
            _ if strafing || !ATTACK_AIM => wanted,
            Some(f) => combat::heading_of(f.direction),
            None => facing,
        };
        p.actions.target_angle = wrap(aim - facing);

        let requested = combat::request(intent, p.actions.range, stick.magnitude, walked_into, p.actions.combo, p.request);
        if requested == Action::ATTSTEP1 {
            p.actions.target_angle = wrap(wanted - facing);
        }
        p.request = requested;

        // The chaining. A looping clip counts as ended each time it comes
        // round (a stand-in: the game's end flag for loops isn't traced).
        let clips = animator.clips.clone();
        let env = Env {
            frame: animator.frame,
            class: p.class,
            has_low2: clips.actions.iter().any(|a| a.name == Action::ATTLOW2.name()),
        };
        let next = p.actions.next(requested, &env);
        let loops = clips.actions.get(animator.action).is_some_and(|a| a.loops());
        let wrapped = loops && p.last_clip.0 == animator.action && animator.frame < p.last_clip.1;
        let ended = animator.finished() || wrapped;
        let (move_factor, turn_factor) = actions::factors(current, p.class.unwrap_or(0));
        let clip = clip_for(&animator, next.action);
        if next.switch.applies(clip != animator.action, ended) {
            if clip != animator.action {
                animator.play_blended(&clips.actions[clip].name, next.blend);
            } else {
                animator.play(clip);
            }
            let strike = p.actions.switched(next.action, p.class);
            debug!(
                "action {} -> {} (asked {}, clip {})",
                current.name(),
                next.action.name(),
                requested.name(),
                animator.action_name()
            );
            if strike.melee() {
                let blow = strike_blow(entity, p, strike, position, facing, &targets, ground.as_deref());
                if let Some(hit) = blow {
                    info!(
                        "{} lands on {:?}: {:.1} damage, kind {:#x}",
                        current.name(),
                        hit.target_kind,
                        hit.damage,
                        hit.kind
                    );
                    hits.write(hit);
                }
            }
            if strike.projectile() {
                debug!("{} would release a projectile (not implemented)", current.name());
            }
        }
        p.last_clip = (animator.action, animator.frame);

        // Facing: toward the stick, or held while strafing and defending;
        // attacking in place turns toward the target.
        let category = p.actions.action.category().0;
        let mut face = (magnitude > 0.0 && !keeps_facing).then_some(stick.heading);
        if ATTACK_AIM && (1..=10).contains(&category) && category != 7 && !strafing && drive == 0.0 {
            face = Some(aim);
        }

        // Movement: this tick's step uses the movement factor from the last
        // tick's chaining, turning uses this tick's.
        let heading = if drive > 0.0 && magnitude == 0.0 { facing } else { stick.heading };
        p.mover.factors = (p.move_factor, turn_factor);
        let d = p.mover.step(Stick { heading, magnitude: drive }, face, dt);
        p.move_factor = move_factor;
        let feet = p.mover.position;
        let d = match &ground {
            Some(g) => g.0.move_player(feet, d, &body, &mut p.ground).delta,
            None => d,
        };
        p.mover.position = std::array::from_fn(|i| feet[i] + d[i]);
        // Fell out of the level: back to the start (the game kills the
        // player here; lives aren't implemented yet).
        if ground.as_ref().is_some_and(|g| p.mover.position[1] <= g.0.kill_height()) {
            let (at, facing) = p.start;
            p.teleport(at, facing);
        }
        trace!("player at {:?}", p.mover.position);
    }
}

/// Resolves a melee blow: the nearest target in the facing direction within
/// reach, in sight for monsters and objects.
fn strike_blow(
    attacker: Entity,
    p: &Player,
    strike: actions::Strike,
    position: Vec3,
    facing: f32,
    targets: &Query<(Entity, &GlobalTransform, &Targetable)>,
    ground: Option<&LevelGround>,
) -> Option<Hit> {
    let found = combat::search(position, facing, targets.iter().map(|(e, t, target)| (e, t.translation(), target)))?;
    if found.distance >= combat::REACH + p.radius {
        return None;
    }
    let sighted = matches!(found.kind, TargetKind::Monster | TargetKind::Object);
    if sighted && ground.is_some_and(|g| g.0.wall(position.to_array(), found.position.to_array(), combat::SIGHT_RADIUS).is_some()) {
        return None;
    }
    let (damage, kind) = combat::blow(strike, p.strength, &found);
    let push = if sighted { combat::push(facing, damage) } else { Vec3::ZERO };
    let at = position + Vec3::new(facing.sin(), 0.0, facing.cos()) * (combat::REACH + p.radius);
    Some(Hit { target: found.entity, attacker, damage, kind, push, at, target_kind: found.kind })
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
