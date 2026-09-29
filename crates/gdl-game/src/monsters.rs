//! Monsters: the regular enemies generators make and levels place, run the
//! way the game's per-frame monster update does (`docs/monsters.md`) — pick
//! the nearest player they're aware of and walk at it (the chasers) or
//! wander until one comes close (the small ones), at their type's speed and
//! turn rate, against the level's collision and each other, and attack when
//! they bump into a player. Stats are the game's per-type tables
//! (`gdl_formats::enemy`) scaled by the level's tuning record.
//!
//! Runs on the 30 Hz fixed tick after the player, interpolated for drawing.
//! Attacks don't hurt anyone yet: each landed hit is a [`MonsterHit`]
//! message for the health code to read.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_6, PI};
use std::sync::Arc;

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;
use gdl_formats::anim::AnimFile;
use gdl_formats::collision::{node_flags, push_out};
use gdl_formats::enemy::{self, ACTION_NAMES, EnemyInstance, EnemyScales, FIELDS_PER_TICK, LevelEnemies};
use gdl_formats::{LevelCollision, LevelTuning, ModelFile, WorldData};
use gdl_install::GameInstall;

use crate::character::{Animator, CharacterData, CharacterModel};
use crate::generators::{self, Generator};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::play_camera::PlayCamera;
use crate::player::{Player, PlayerTick};
use crate::population::LevelPopulation;
use crate::world::{LevelEntity, LevelGround};

/// Stand-ins for the player's collision cylinder (radius and height), which
/// the monsters' bump test reads from the player record and which isn't
/// decoded yet.
pub const PLAYER_RADIUS: f32 = 1.0;
pub const PLAYER_HEIGHT: f32 = 5.0;

/// The monster mover drops at most this far per second.
const MAX_DROP_PER_SECOND: f32 = 16.0;
/// Falling more than this below the floor it stood on kills a monster.
const FALL_LIMIT: f32 = 5.0;
/// Steering offsets tried in turn when a wall or another monster is in the
/// way: 0, π/8 … 7π/8.
const AVOID_OFFSETS: [f32; 8] =
    [0.0, PI / 8.0, PI / 4.0, 3.0 * PI / 8.0, PI / 2.0, 5.0 * PI / 8.0, 3.0 * PI / 4.0, 7.0 * PI / 8.0];

pub struct MonstersPlugin;

impl Plugin for MonstersPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MonsterHit>()
            .add_systems(Startup, load_level_tunings)
            .add_systems(
                FixedUpdate,
                (generators::tick_generators, generators::tick_placed, tick_monsters).chain().after(PlayerTick),
            )
            .add_systems(
                Update,
                (setup_level.run_if(resource_exists_and_changed::<LevelPopulation>), interpolate, log_hits).chain(),
            );
    }
}

/// A monster's attack landed on a player. Nothing applies the damage yet.
#[derive(Message, Debug, Clone, Copy)]
pub struct MonsterHit {
    pub monster: Entity,
    pub player: Entity,
    pub damage: f32,
    /// The third attack, which hits half as hard again.
    pub strong: bool,
}

/// Each level's tuning record and the enemy types it loads, by lower-case
/// level folder (`levela1`).
#[derive(Resource, Default)]
struct LevelTunings(HashMap<String, (LevelTuning, LevelEnemies)>);

fn load_level_tunings(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let mut tunings = LevelTunings::default();
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
        let parsed = game.install.read(&path).map_err(|e| e.to_string());
        match parsed.and_then(|b| WorldData::parse(&b).map_err(|e| e.to_string())) {
            Ok(world) => {
                for level in &world.levels {
                    let loaded = world.level_enemies(level).iter().map(|e| (e.enemy, e.subtype)).collect();
                    tunings.0.insert(level.folder().to_ascii_lowercase(), (level.tuning, LevelEnemies { loaded }));
                }
            }
            Err(e) => warn!("{path}: {e}"),
        }
    }
    commands.insert_resource(tunings);
}

/// Model and action lookup for one monster type at one tier.
pub struct MonsterModel {
    pub model: CharacterModel,
    /// The atree action for each of the game's action indices, if present.
    actions: [Option<usize>; ACTION_NAMES.len()],
    /// Whether each present action loops.
    loops: [bool; ACTION_NAMES.len()],
}

impl MonsterModel {
    fn has(&self, action: u8) -> bool {
        self.actions.get(action as usize).is_some_and(Option::is_some)
    }
}

/// The current level's monster state: tuning, models, and bookkeeping the
/// game keeps in globals.
#[derive(Resource)]
pub struct MonsterLevel {
    pub scales: EnemyScales,
    /// Most monsters alive at once.
    pub slots: usize,
    /// Per (enemy type, tier): the model, if one could be found.
    models: HashMap<(i32, i32), Option<Arc<MonsterModel>>>,
    /// 30 Hz ticks since the level started.
    pub tick: u32,
    rng: u32,
    /// Monsters created so far (each gets the next number: it staggers
    /// their target searches like the game's slot index does).
    created: u32,
}

impl MonsterLevel {
    /// The game's `random(n)`: 0..n.
    pub fn random(&mut self, n: u32) -> u32 {
        // xorshift32; the game has its own generator.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        if n == 0 { 0 } else { x % n }
    }

    pub fn model(&self, enemy: i32, tier: i32) -> Option<Arc<MonsterModel>> {
        self.models.get(&(enemy, tier)).cloned().flatten()
    }
}

/// What stopped a monster's last move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Block {
    #[default]
    None,
    Wall,
    /// Another monster.
    Actor,
    /// A player (it attacks).
    Player,
}

/// A live monster.
#[derive(Component)]
pub struct Monster {
    pub enemy: i32,
    pub tier: i32,
    pub ai: i16,
    pub stats: EnemyInstance,
    pub hit_points: f32,
    /// Feet.
    pub position: [f32; 3],
    /// Which way the body faces (turns at the type's rate).
    pub facing: f32,
    /// Which way it's trying to move.
    pub heading: f32,
    /// The generator that made it.
    pub generator: Option<Entity>,
    /// Placed in the level rather than generated (recycled last).
    pub placed: bool,
    pub target: Option<Entity>,
    pub target_distance: f32,
    /// Has noticed a player (stays set).
    pub aware: bool,
    /// Its bounds are on screen, with a margin.
    pub near_screen: bool,
    floor: f32,
    previous: ([f32; 3], f32),
    /// Frozen for this many video fields (just placed).
    freeze: f32,
    model: Arc<MonsterModel>,
    /// The game's action index it's playing, and the one it asks for.
    action: u8,
    request: u8,
    /// Hits landed; every 8th attack is the stronger third one.
    attacks: u16,
    /// The player the current attack is aimed at.
    strike: Option<Entity>,
    blocked: Block,
    avoid_side: i32,
    avoid_step: usize,
    /// Video fields left to keep the current avoiding heading.
    avoid_timer: f32,
    stuck: u8,
    last_heading: f32,
    /// Turns a wanderer has made since it last swapped direction.
    wander_turns: u8,
    number: u32,
}

const READY: u8 = 0;
const START: u8 = 1;
const WALK: u8 = 3;
const RUN: u8 = 4;
const ATTACK1: u8 = 0xC;
const ATTACK2: u8 = 0xE;
const ATTACK3: u8 = 0x10;
const RUNATTACK1: u8 = 0x16;
const RUNATTACK2: u8 = 0x17;

/// Everything needed to create a monster.
pub struct NewMonster {
    pub enemy: i32,
    pub tier: i32,
    pub ai: i16,
    pub position: [f32; 3],
    pub facing: f32,
    pub generator: Option<Entity>,
    pub placed: bool,
    /// Overrides the awareness range (placed monsters can set one).
    pub awareness: Option<f32>,
    /// Video fields to stand still first.
    pub freeze: f32,
}

/// Creates a monster: its stats, its model (playing START) and its entity.
pub fn spawn_monster(level: &mut MonsterLevel, new: NewMonster, commands: &mut Commands) -> Option<Entity> {
    let stats = enemy::enemy_stats(new.enemy)?;
    let model = level.model(new.enemy, new.tier)?;
    let mut instance = stats.instance(new.tier, new.ai, &level.scales);
    if let Some(a) = new.awareness {
        instance.awareness = a;
    }
    let transform =
        Transform::from_translation(Vec3::from(new.position)).with_rotation(Quat::from_rotation_y(new.facing));
    let root = model.model.spawn(transform, commands);
    level.created += 1;
    let monster = Monster {
        enemy: new.enemy,
        tier: new.tier,
        ai: new.ai,
        hit_points: instance.hit_points,
        stats: instance,
        position: new.position,
        facing: new.facing,
        heading: new.facing,
        generator: new.generator,
        placed: new.placed,
        target: None,
        target_distance: f32::MAX,
        aware: false,
        near_screen: true,
        floor: new.position[1],
        previous: (new.position, new.facing),
        freeze: new.freeze,
        action: START,
        request: READY,
        attacks: 0,
        strike: None,
        blocked: Block::None,
        avoid_side: 0,
        avoid_step: 0,
        avoid_timer: 0.0,
        stuck: 0,
        last_heading: new.facing,
        wander_turns: 0,
        number: level.created,
        model: model.clone(),
    };
    commands.entity(root).insert((monster, LevelEntity));
    Some(root)
}

/// Loads what the new level's generators and placed monsters need.
#[allow(clippy::too_many_arguments)]
fn setup_level(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    population: Res<LevelPopulation>,
    tunings: Option<Res<LevelTunings>>,
    ground: Option<Res<LevelGround>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(ground) = ground else { return };
    let entry = tunings.and_then(|t| t.0.get(&population.level.to_ascii_lowercase()).cloned());
    let (raw, enemies) = match entry {
        Some((t, e)) => (Some(t), e),
        None => (None, LevelEnemies::default()),
    };
    let tuning = level_tuning(raw);
    let scales = EnemyScales {
        hit_points: tuning.monster_hit_points,
        speed: tuning.monster_speed,
        awareness: tuning.monster_awareness,
        damage: tuning.monster_damage,
    };
    let (gens, placed) = generators::from_population(&population.population, &ground.0, &tuning, &enemies);

    let mut wanted: Vec<(i32, i32)> =
        gens.iter().map(|g| (g.enemy, g.tier)).chain(placed.iter().map(|p| (p.enemy, p.tier))).collect();
    wanted.sort();
    wanted.dedup();
    let mut folders = FolderCache::default();
    let mut models = HashMap::new();
    for &(id, tier) in &wanted {
        let model = match load_enemy(&mut game.install, &mut folders, &enemies, id, tier) {
            Ok(data) => Some(Arc::new(monster_model(&data, &mut meshes, &mut materials, &mut images))),
            Err(why) => {
                warn!("monster {id} tier {tier}: {why}");
                None
            }
        };
        models.insert((id, tier), model);
    }

    info!(
        "monsters: {} generators, {} placed, {} slots, types {:?} from {:?} (tuning {:?})",
        gens.len(),
        placed.len(),
        tuning.monster_slots,
        wanted,
        enemies.loaded,
        tuning
    );
    for g in gens {
        commands.spawn((g, Transform::default(), LevelEntity));
    }
    commands.insert_resource(generators::PlacedMonsters(placed));
    commands.insert_resource(MonsterLevel {
        slots: tuning.monster_slots.max(1) as usize,
        scales,
        models,
        tick: 0,
        rng: 0x1234_5678,
        created: 0,
    });
}

/// The level's tuning with unset (zero) scales read as 1. The forest realm
/// and a few secret/test levels leave them at 0; what the game makes of
/// that isn't confirmed — taken literally, their monsters would have no
/// hit points and never notice anyone. Stand-in.
fn level_tuning(raw: Option<LevelTuning>) -> LevelTuning {
    let mut t = raw.unwrap_or(LevelTuning {
        monster_slots: 15,
        monster_hit_points: 1.0,
        monster_speed: 1.0,
        monster_awareness: 1.0,
        monster_damage: 1.0,
        generator_hit_points: 1.0,
        generator_rate: 1.0,
        generator_max: 1.0,
    });
    for v in [
        &mut t.monster_hit_points,
        &mut t.monster_speed,
        &mut t.monster_awareness,
        &mut t.monster_damage,
        &mut t.generator_hit_points,
        &mut t.generator_rate,
        &mut t.generator_max,
    ] {
        if *v <= 0.0 {
            *v = 1.0;
        }
    }
    t
}

/// A monster folder's model, texture bytes and atrees.
struct MonsterFolder {
    model: ModelFile,
    textures: Vec<u8>,
    anim: AnimFile,
}

/// Monster folders read so far (`None`: missing or unreadable).
#[derive(Default)]
struct FolderCache(HashMap<String, Option<Arc<MonsterFolder>>>);

impl FolderCache {
    fn get(&mut self, install: &mut GameInstall, folder: &str) -> Option<Arc<MonsterFolder>> {
        self.0
            .entry(folder.to_string())
            .or_insert_with(|| {
                let dir = format!("MONSTERS/{folder}");
                let model = ModelFile::parse(&install.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
                let textures = install.read(&format!("{dir}/textures.ngc")).ok()?;
                let anim = AnimFile::parse(&install.read(&format!("{dir}/ANIM.PS2")).ok()?).ok()?;
                Some(Arc::new(MonsterFolder { model, textures, anim }))
            })
            .clone()
    }
}

/// The model for an enemy type at a tier: the atree `<NAME><tier>` in the
/// folders the level loads for the type (`MONSTERS/<name>`, `<name>aux`,
/// `<name><n>`, by the realm's enemy records). If the tier's atree isn't
/// there, the nearest tier that is stands in (the game would draw a plain
/// `<NAME><tier>L1` object).
fn load_enemy(
    install: &mut GameInstall,
    folders: &mut FolderCache,
    enemies: &LevelEnemies,
    id: i32,
    tier: i32,
) -> Result<CharacterData, String> {
    let name = enemy::enemy_name(id).ok_or("unknown enemy type")?.to_ascii_uppercase();
    let mut tiers = vec![tier];
    for d in 1..=3 {
        tiers.extend([tier - d, tier + d].into_iter().filter(|t| (1..=3).contains(t)));
    }
    let mut search = enemies.folders(id);
    if search.is_empty() {
        search.push(name.clone());
    }
    for t in tiers {
        let Some(atree) = enemy::atree_name(id, t) else { continue };
        for folder in &search {
            let Some(files) = folders.get(install, folder) else { continue };
            let Some(tree) = files.anim.atrees.iter().find(|a| a.name == atree) else { continue };
            return Ok(CharacterData {
                name: format!("{folder}/{atree}"),
                class: String::new(),
                colour: String::new(),
                skeleton: tree.clone(),
                clips: Arc::new(tree.clone()),
                model: files.model.clone(),
                textures: files.textures.clone(),
            });
        }
    }
    Err(format!("no atree for {name} tier {tier}"))
}

fn monster_model(
    data: &CharacterData,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> MonsterModel {
    let mut actions = [None; ACTION_NAMES.len()];
    let mut loops = [false; ACTION_NAMES.len()];
    for (i, name) in ACTION_NAMES.iter().enumerate() {
        if let Some(a) = data.clips.actions.iter().position(|a| a.name == *name) {
            actions[i] = Some(a);
            loops[i] = data.clips.actions[a].loops();
        }
    }
    MonsterModel { model: CharacterModel::build(data, meshes, materials, images), actions, loops }
}

/// What the game's play camera sees: its view from eye to target with the
/// game's 60° × 45° (4:3) field of view — whatever the window's shape or
/// the free camera. Before the play camera exists nothing is on screen.
pub fn game_view(camera: Option<&PlayCamera>) -> Option<Frustum> {
    let rig = &camera?.rig;
    let (eye, target) = (Vec3::from(rig.eye()), Vec3::from(rig.target));
    if eye.distance_squared(target) < 1e-6 {
        return None;
    }
    let fov = 2.0 * (0.75 * 30f32.to_radians().tan()).atan();
    let clip = Mat4::perspective_rh(fov, 4.0 / 3.0, 0.5, 2000.0) * Mat4::look_at_rh(eye, target, Vec3::Y);
    Some(Frustum::from_clip_from_world(&clip))
}

/// The on-screen test the game makes against its camera: a sphere around a
/// point.
pub fn on_screen(view: Option<&Frustum>, at: [f32; 3], radius: f32) -> bool {
    view.is_some_and(|f| f.intersects_sphere(&Sphere { center: Vec3::from(at).into(), radius }, true))
}

/// A player as the monsters see it.
#[derive(Clone, Copy)]
pub struct Target {
    pub entity: Entity,
    pub feet: [f32; 3],
}

/// Monster positions at the start of the tick, for bump tests.
#[derive(Clone, Copy)]
pub struct Body {
    pub entity: Entity,
    pub feet: [f32; 3],
    pub radius: f32,
    pub step: f32,
}

#[allow(clippy::too_many_arguments)]
fn tick_monsters(
    mut commands: Commands,
    time: Res<Time>,
    level: Option<ResMut<MonsterLevel>>,
    ground: Option<Res<LevelGround>>,
    players: Query<(Entity, &Player)>,
    camera: Option<Res<PlayCamera>>,
    mut monsters: Query<(Entity, &mut Monster, &mut Animator)>,
    mut generators: Query<&mut Generator>,
    mut hits: MessageWriter<MonsterHit>,
) {
    let (Some(mut level), Some(ground)) = (level, ground) else { return };
    level.tick = level.tick.wrapping_add(1);
    let collision = &ground.0;
    let dt = time.delta_secs();
    let view = game_view(camera.as_deref());
    let frustum = view.as_ref();
    let targets: Vec<Target> = players.iter().map(|(e, p)| Target { entity: e, feet: p.mover.position }).collect();
    let bodies: Vec<Body> = monsters
        .iter()
        .map(|(e, m, _)| Body { entity: e, feet: m.position, radius: m.stats.radius, step: m.stats.step })
        .collect();

    for (entity, mut m, mut animator) in &mut monsters {
        let m = &mut *m;
        m.previous = (m.position, m.facing);
        let r = m.stats.radius;
        m.near_screen = on_screen(frustum, m.position, 2.0 * r + 15.0);
        select_target(m, &targets, level.tick);

        // Asleep unless a player is in range or it's near the screen.
        if m.target_distance > m.stats.awareness && !m.near_screen {
            continue;
        }
        let target = m.target.and_then(|t| targets.iter().find(|p| p.entity == t)).copied();
        if m.target.is_some() && m.target_distance <= m.stats.awareness {
            m.aware = true;
        }

        m.request = READY;
        let mut velocity = [0.0f32; 3];
        let direct = target.map(|t| heading_to(m.position, t.feet));
        let turn_to = match (m.ai, direct) {
            // The wanderers go their own way until a player comes close.
            (2 | 4, _) => Some(wander(m, direct)),
            (_, Some(direct)) if m.aware => {
                let player = target.map_or(m.position, |t| t.feet);
                Some(steer(m, direct, player, collision, &bodies, entity))
            }
            _ => None,
        };
        if let Some(h) = turn_to {
            // Walk at the heading; only a walking or running body moves.
            m.request = WALK;
            let moving = matches!(m.action, WALK | RUN | RUNATTACK1 | RUNATTACK2) && m.model.has(m.action);
            if moving {
                let s = m.stats.speed_per_tick;
                velocity = [h.sin() * s, 0.0, h.cos() * s];
            }
        }
        if m.freeze > 0.0 {
            m.freeze -= FIELDS_PER_TICK;
            velocity = [0.0; 3];
        }
        if let Some(h) = turn_to {
            let rate = m.stats.turn_per_tick * if m.action == RUN { 3.0 } else { 1.0 };
            m.facing = turn_toward(m.facing, h, rate);
        }

        // Walls and floor, then players and other monsters in the way.
        let moved = monster_move(collision, m, velocity, MAX_DROP_PER_SECOND * dt);
        let delta = moved.delta;
        if moved.wall {
            blocked(m, None, target.map(|t| t.feet));
        }
        let to = add(m.position, delta);
        let mut bumped_player = None;
        for p in &targets {
            let reach = r + 0.5 + PLAYER_RADIUS;
            if bumps(m.position, to, p.feet, reach, m.stats.step + PLAYER_HEIGHT) {
                bumped_player = Some(*p);
                break;
            }
        }
        if let Some(p) = bumped_player {
            // It stops and swings at the player it walked into.
            m.blocked = Block::Player;
            m.strike = Some(p.entity);
            m.request = if m.attacks & 7 == 7 { ATTACK3 } else { ATTACK1 };
            m.position[1] += delta[1];
        } else if let Some(other) = bodies
            .iter()
            .find(|b| b.entity != entity && bumps(m.position, to, b.feet, r + b.radius, m.stats.step + b.step))
        {
            blocked(m, Some(other.feet), target.map(|t| t.feet));
            m.position[1] += delta[1];
        } else {
            if !moved.wall {
                m.blocked = Block::None;
            }
            m.position = to;
        }
        if moved.fell {
            despawn_monster(&mut commands, entity, m, &mut generators);
            continue;
        }

        if let Some(hit) = animate(m, &mut animator) {
            if let Some(player) = m.strike {
                hits.write(MonsterHit {
                    monster: entity,
                    player,
                    damage: m.stats.damage * if hit { 1.5 } else { 1.0 },
                    strong: hit,
                });
            }
            m.attacks = m.attacks.wrapping_add(1);
        }
    }
}

/// Removes a monster and frees its generator's slot.
pub fn despawn_monster(commands: &mut Commands, entity: Entity, m: &Monster, generators: &mut Query<&mut Generator>) {
    if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
        g.alive = g.alive.saturating_sub(1);
    }
    commands.entity(entity).despawn();
}

/// The game re-picks a monster's target on one tick in eight (staggered
/// by monster), or whenever it has none: the nearest player within its
/// awareness range. Its distance to the target is kept every tick.
fn select_target(m: &mut Monster, targets: &[Target], tick: u32) {
    let keep = m.target.is_some() && tick & 7 != m.number & 7;
    let current = m.target.and_then(|t| targets.iter().find(|p| p.entity == t));
    if keep && let Some(p) = current {
        m.target_distance = distance(m.position, p.feet);
        return;
    }
    m.target = None;
    m.target_distance = f32::MAX;
    for p in targets {
        let d = distance(m.position, p.feet);
        if d <= m.stats.awareness && d < m.target_distance {
            m.target = Some(p.entity);
            m.target_distance = d;
        }
    }
}

/// The heading a chasing monster (AI 7) takes toward the player: straight
/// at it, or off to one side by the current avoiding step while a wall or
/// another monster is in the way. A heading whose next step would hit a
/// wall or monster isn't taken; after 10 such ticks it goes straight at the
/// player again. While an avoid timer runs it keeps its heading.
fn steer(
    m: &mut Monster,
    direct: f32,
    player: [f32; 3],
    collision: &LevelCollision,
    bodies: &[Body],
    me: Entity,
) -> f32 {
    if m.avoid_timer > 0.0 {
        m.avoid_timer -= FIELDS_PER_TICK;
        return m.heading;
    }
    let offset = AVOID_OFFSETS[m.avoid_step.min(AVOID_OFFSETS.len() - 1)];
    let mut h = match m.blocked {
        Block::Wall => {
            if m.avoid_side == 0 {
                m.avoid_side = nearer_side(m.position, m.facing, player);
            }
            if m.avoid_side < 1 { direct - offset } else { direct + offset }
        }
        Block::Actor => {
            if m.avoid_side < 1 {
                m.heading - offset
            } else {
                m.heading + offset
            }
        }
        Block::None | Block::Player => direct,
    };
    h = locomotion::wrap(h);
    let s = m.stats.speed_per_tick / FIELDS_PER_TICK;
    let ahead = [m.position[0] + h.sin() * s, m.position[1], m.position[2] + h.cos() * s];
    let turned = (locomotion::wrap(m.heading - m.last_heading)).abs() > 0.0349
        && (locomotion::wrap(h - m.last_heading)).abs() <= 0.0349;
    let blocked_ahead = turned || look_ahead_blocked(m, ahead, collision, bodies, me);
    if blocked_ahead {
        m.stuck = m.stuck.saturating_add(1);
    } else {
        m.stuck = 0;
    }
    if m.stuck > 10 {
        m.heading = direct;
        return direct;
    }
    if !blocked_ahead {
        m.last_heading = m.heading;
        m.heading = h;
    }
    h
}

/// The wanderers (AIs 2 and 4, the small monsters): walk straight on; when
/// a wall or monster held them up, turn 45° (AI 2 left, AI 4 right) once
/// the 20-field wait is over, swapping the turn after four turns. A player
/// within 8 units makes them chase (AI 0; our chase is AI 7's). Walking
/// into a player turns them at it.
fn wander(m: &mut Monster, direct: Option<f32>) -> f32 {
    if direct.is_some() && m.target_distance <= 8.0 {
        m.ai = 0;
        return m.heading;
    }
    if m.avoid_timer > 0.0 {
        m.avoid_timer -= FIELDS_PER_TICK;
        if m.avoid_timer < 1.0 {
            let turn = if m.ai == 2 { PI / 4.0 } else { -PI / 4.0 };
            m.heading = locomotion::wrap(m.heading + turn);
            m.wander_turns += 1;
            if m.wander_turns > 3 {
                m.wander_turns = 0;
                m.ai = if m.ai == 2 { 4 } else { 2 };
            }
        }
    }
    if let (Block::Player, Some(d)) = (m.blocked, direct) {
        m.heading = d;
    }
    m.heading
}

/// The game's look-ahead: another monster overlapping the next step, or a
/// wall between here and there (its thin any-hit wall test).
fn look_ahead_blocked(m: &Monster, ahead: [f32; 3], collision: &LevelCollision, bodies: &[Body], me: Entity) -> bool {
    let r = 0.1 + m.stats.radius;
    let height = 0.1 + m.stats.step;
    if bodies.iter().any(|b| b.entity != me && bumps(m.position, ahead, b.feet, r + b.radius, height + b.step)) {
        return true;
    }
    let lift = [0.0, 1.0, 0.0];
    collision.wall(add(m.position, lift), add(ahead, lift), 0.1).is_some()
}

/// Which side to go round a wall: +1 if a unit step 30° to the left of the
/// facing ends nearer the player than one 30° to the right, else −1.
fn nearer_side(at: [f32; 3], facing: f32, player: [f32; 3]) -> i32 {
    let d = |a: f32| {
        let (x, z) = (at[0] + a.sin() - player[0], at[2] + a.cos() - player[2]);
        x * x + z * z
    };
    if d(facing + FRAC_PI_6) < d(facing - FRAC_PI_6) { 1 } else { -1 }
}

/// How an AI steps round what blocks it: fields to hold each avoiding
/// step, fields to go straight when giving up, and the steps before the
/// side flips. Only the chasers step round; the rest just wait 20 fields.
fn avoid_rule(ai: i16, wall: bool) -> Option<(f32, f32, usize)> {
    match (ai, wall) {
        (7, true) => Some((10.0, 60.0, 6)),
        (7, false) => Some((15.0, 50.0, 6)),
        (0, true) => Some((5.0, 60.0, 8)),
        _ => None,
    }
}

/// A wall (or, with `other`, another monster) stopped the move: a chaser
/// steps its avoiding angle round (the monster case first picks the side
/// away from the other monster), flipping side after its step limit and
/// giving up — straight at the player for a while — once the side has
/// flipped twice. Other AIs wait 20 fields (AI 0 60 for a monster).
fn blocked(m: &mut Monster, other: Option<[f32; 3]>, target: Option<[f32; 3]>) {
    m.blocked = if other.is_some() { Block::Actor } else { Block::Wall };
    if let Some(o) = other
        && (m.avoid_side == 0 || m.avoid_side.abs() > 2)
    {
        m.avoid_side = side_of(m.position, o);
        m.avoid_step = 0;
    }
    let Some((hold, give_up, limit)) = avoid_rule(m.ai, other.is_none()) else {
        if m.avoid_timer < 1.0 {
            m.avoid_timer = if m.ai == 0 && other.is_some() { 60.0 } else { 20.0 };
        }
        return;
    };
    if m.avoid_side.abs() < 3 {
        m.avoid_step += 1;
        if m.avoid_timer <= 0.0 {
            m.avoid_timer = hold;
        }
    } else {
        m.avoid_timer = give_up;
        if let Some(t) = target {
            m.heading = heading_to(m.position, t);
        }
        m.avoid_step = 0;
        m.avoid_side = 0;
    }
    if m.avoid_step > limit {
        m.avoid_side *= -2;
        m.avoid_step = 0;
    }
}

/// The game's side pick against another monster: along the axis they're
/// most apart on, +1 or −1 by which way the other one lies.
fn side_of(me: [f32; 3], other: [f32; 3]) -> i32 {
    if (me[0] - other[0]).abs() < (me[2] - other[2]).abs() {
        if other[0] <= me[0] { 1 } else { -1 }
    } else if other[2] <= me[2] {
        -1
    } else {
        1
    }
}

/// Plays the actions the monster asks for, the way the game's action
/// chooser does for regular monsters: START and attacks play out before
/// anything else; an attack finishing is when it lands, and its recovery
/// (`…R`) follows; walking and standing switch at once. Missing actions
/// fall back as the game's do (RUN ↔ WALK, any attack → ATTACK1, else
/// READY). Returns `Some(strong)` on the tick an attack lands.
fn animate(m: &mut Monster, animator: &mut Animator) -> Option<bool> {
    let model = m.model.clone();
    let finished = |m: &Monster, animator: &Animator| -> bool {
        let Some(a) = model.actions[m.action as usize] else { return true };
        if model.loops[m.action as usize] || animator.action != a {
            return animator.action != a;
        }
        let frames = animator.clips.actions.get(a).map_or(1, |x| x.frames) as f32;
        animator.frame >= frames - 1.0
    };
    let mut landed = None;
    let next = match m.action {
        START => finished(m, animator).then_some(m.request),
        ATTACK1 | ATTACK2 | 0x12 | 0x14 => finished(m, animator).then(|| {
            landed = Some(false);
            m.action + 1
        }),
        ATTACK3 => finished(m, animator).then(|| {
            landed = Some(true);
            m.action + 1
        }),
        0xD => finished(m, animator).then(|| if model.has(ATTACK2) { ATTACK2 } else { m.request }),
        0xF | 0x11 | 0x13 | 0x15 => finished(m, animator).then_some(m.request),
        _ => Some(m.request),
    };
    if let Some(mut next) = next {
        next = match next {
            WALK if !model.has(WALK) => RUN,
            RUN if !model.has(RUN) => WALK,
            ATTACK2 | ATTACK3 | 0x12 | 0x14 if !model.has(next) => ATTACK1,
            n => n,
        };
        let restart = next != m.action;
        m.action = next;
        match model.actions[next as usize].or(model.actions[READY as usize]) {
            Some(a) if restart || animator.action != a => animator.play(a),
            _ => {}
        }
    }
    landed
}

struct MonsterMove {
    delta: [f32; 3],
    /// A wall stopped or deflected the move.
    wall: bool,
    /// Dropped more than the fall limit: the game kills it.
    fell: bool,
}

/// The game's monster mover: like the players' (`LevelCollision::
/// move_actor`), but the wall test starts 2 units up with 1.5 × the
/// monster's radius, the floor probe has half its radius and searches
/// from its step above to step + 5 below, at the leading edge
/// (position + direction × (radius + move)).
fn monster_move(collision: &LevelCollision, m: &mut Monster, velocity: [f32; 3], max_drop: f32) -> MonsterMove {
    let r = m.stats.radius;
    let step = m.stats.step;
    let mut v = velocity;
    let mut wall = false;
    let start = [m.position[0], m.position[1] + 2.0, m.position[2]];
    let horizontal = (v[0] * v[0] + v[2] * v[2]).sqrt();
    if horizontal > 0.0 {
        let wall_r = 1.5 * r;
        if let Some(hit) = collision.wall(start, add(start, v), wall_r)
            && collision.nodes[hit.node].flags & node_flags::NO_PUSH == 0
        {
            wall = true;
            if push_out(wall_r, start, &mut v, hit.point, hit.normal) {
                v[0] = 0.0;
                v[2] = 0.0;
            }
        }
    }
    let len = (v[0] * v[0] + v[2] * v[2]).sqrt();
    let probe = |at: [f32; 3]| collision.floor_probe(at, step, -step - 5.0, 0.5 * r, 2);
    if len > 0.0 {
        let dir = [v[0] / len, 0.0, v[2] / len];
        let allowance = 2.0 * (0.1 + r + len);
        let edge = add(start, scale(dir, r + len));
        let mut ok = false;
        if let Some(hit) = probe(edge) {
            let rise = (hit.point[1] - m.floor).abs();
            if rise <= allowance {
                ok = true;
                m.floor = hit.point[1];
                if 0.1 * len < rise {
                    match probe(add(start, v)) {
                        Some(h) => m.floor = h.point[1],
                        None => ok = false,
                    }
                }
            }
        }
        if !ok {
            v[0] = 0.0;
            v[2] = 0.0;
        }
    }
    if v[0] == 0.0 && v[2] == 0.0 {
        // Standing: follow the floor under it.
        if let Some(h) = probe(start) {
            m.floor = h.point[1];
        }
    }
    let dy = m.floor - m.position[1];
    let fell = dy < -FALL_LIMIT;
    v[1] = dy.max(-max_drop);
    MonsterMove { delta: v, wall, fell }
}

/// The game's bump test: does moving from `from` to `to` bring this body
/// within `reach` (horizontally) and `height` (vertically) of `other`?
/// Already overlapping, only moving further in counts.
pub fn bumps(from: [f32; 3], to: [f32; 3], other: [f32; 3], reach: f32, height: f32) -> bool {
    let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
    let (ox, oz) = (other[0] - from[0], other[2] - from[2]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 > 0.0 { ((ox * dx + oz * dz) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cz) = (from[0] + dx * t - other[0], from[2] + dz * t - other[2]);
    if cx * cx + cz * cz > reach * reach || (from[1] + (to[1] - from[1]) * t - other[1]).abs() > height {
        return false;
    }
    let start2 = ox * ox + oz * oz;
    if start2 <= reach * reach {
        if len2 == 0.0 {
            return false;
        }
        // Moving toward it?
        return ox * dx + oz * dz > 0.0;
    }
    true
}

/// Heading from `from` toward `to` (the game's angle: +Z is 0, +X is π/2).
pub fn heading_to(from: [f32; 3], to: [f32; 3]) -> f32 {
    (to[0] - from[0]).atan2(to[2] - from[2])
}

fn turn_toward(facing: f32, goal: f32, rate: f32) -> f32 {
    let d = locomotion::wrap(goal - facing);
    locomotion::wrap(facing + d.clamp(-rate, rate))
}

pub fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Logs landed hits (`RUST_LOG=gdl_game::monsters=debug`) until something
/// applies them.
fn log_hits(mut hits: MessageReader<MonsterHit>, monsters: Query<&Monster>) {
    for hit in hits.read() {
        let Ok(m) = monsters.get(hit.monster) else { continue };
        debug!(
            "enemy {} tier {} (AI {}, {:.1} HP) hits player {:?} for {:.1}{}",
            m.enemy,
            m.tier,
            m.ai,
            m.hit_points,
            hit.player,
            hit.damage,
            if hit.strong { " (strong)" } else { "" }
        );
    }
}

fn interpolate(fixed: Res<Time<Fixed>>, mut monsters: Query<(&Monster, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (m, mut transform) in &mut monsters {
        let (p0, f0) = m.previous;
        transform.translation = Vec3::from(p0).lerp(Vec3::from(m.position), t);
        transform.rotation = Quat::from_rotation_y(f0 + locomotion::wrap(m.facing - f0) * t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_test_is_a_swept_cylinder() {
        let o = [0.0; 3];
        assert!(bumps([-5.0, 0.0, 0.0], [5.0, 0.0, 0.0], o, 1.0, 1.0));
        assert!(!bumps([-5.0, 0.0, 2.0], [5.0, 0.0, 2.0], o, 1.0, 1.0));
        assert!(!bumps([-5.0, 3.0, 0.0], [5.0, 3.0, 0.0], o, 1.0, 1.0));
        // Overlapping already: only moving in counts.
        assert!(bumps([0.5, 0.0, 0.0], [0.4, 0.0, 0.0], o, 1.0, 1.0));
        assert!(!bumps([0.5, 0.0, 0.0], [0.6, 0.0, 0.0], o, 1.0, 1.0));
    }

    #[test]
    fn headings_follow_the_games_angle() {
        assert_eq!(heading_to([0.0; 3], [0.0, 0.0, 1.0]), 0.0);
        assert!((heading_to([0.0; 3], [1.0, 0.0, 0.0]) - PI / 2.0).abs() < 1e-6);
        assert!((turn_toward(0.0, 1.0, 0.1) - 0.1).abs() < 1e-6);
        // The short way round, across ±π.
        assert!((turn_toward(3.0, -3.0, 0.1) - 3.1).abs() < 1e-5);
        assert!((turn_toward(3.0, -3.0, 0.5) + 3.0).abs() < 1e-5);
        assert_eq!(side_of([0.0; 3], [0.0, 0.0, 5.0]), 1);
        assert_eq!(side_of([0.0; 3], [5.0, 0.0, 0.0]), -1);
    }
}
