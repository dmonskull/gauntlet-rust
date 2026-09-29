//! Critters: the game's scripted monsters, run from their `CRITTER/*.WAD`
//! data (`docs/critters.md` has the game's code behind all of this). This
//! runs the placed golem: it stands as a statue until a wake trigger by it
//! comes on, then every 30 Hz tick it
//!
//! 1. scores the players against its type's target condition and keeps
//!    the best (players it hit in the last quarter second score worse);
//! 2. picks a move: the forced ones first (INIT → START, DEATH, a move's
//!    follow-up, then a knockdown, knockback, roar or flinch for the blows
//!    it took), else a block when its target is attacking, else the least
//!    recently used attack whose condition its target meets, else the
//!    movement move whose condition scores its target best, else a taunt
//!    (unhurt) or READY — every move only when its cooldown since it last
//!    ended has run out;
//! 3. switches to it the way the move's transition allows (at once, or
//!    when the animation playing ends);
//! 4. lands the move's blows on their frames: a sphere on the move's node,
//!    swept from its last position, against each player (a hit player
//!    can't be hit by a critter again for 0.25 s); plays its sounds;
//! 5. walks at the move's speed in the move's direction plus its knockback,
//!    against the level's walls and floor, stopping short of players, and
//!    turns toward its target at the move's rate.
//!
//! Blows from the hero reach it through `damage.rs` ([`Critter::take_hit`]):
//! a blocking critter takes a quarter, its armour comes off each blow, the
//! hero earns a share of its experience per blow and a fifth of it for the
//! kill; the blows' kinds pick its hit reaction and push it. At 0 hit
//! points it plays DEATH and is removed when that ends.
//!
//! A statue only wakes when a wake trigger (flag 0x2000) next to it comes
//! on, as in the game — most placed golems are never woken.
//! `GDL_WAKE_STATUES=<range>` (a testing aid) also wakes them when the
//! hero comes that close.
//!
//! Stand-ins (see the doc): a woken statue goes straight to its ACTIVE
//! animation and its critter appears when that ends;
//! the golem's stomp ring (a damaging effect in the game) hurts players in
//! its radius at once; projectiles, the health meter, effects, breakable
//! nodes, its blows on other monsters and pushing players aside aren't
//! done. Bosses, the gargoyle and the general aren't run yet.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::anim::AnimFile;
use gdl_formats::collision::{node_flags, push_out};
use gdl_formats::critter::{self, CritterDamage, CritterFile, CritterMove, Condition, class, kind};
use gdl_formats::population::{ItemClass, PlacementParams, REALM_LETTERS};
use gdl_formats::{LevelCollision, ModelFile};

use crate::audio::PlaySound;
use crate::character::{Animator, CharacterData, CharacterModel};
use crate::combat::{TargetKind, Targetable};
use crate::damage::after_armor;
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::mechanics::Mechanics;
use crate::monsters::{MonsterLevel, MonsterTick};
use crate::player::Player;
use crate::player_state::{DamagePlayer, PlayerState};
use crate::population::LevelPopulation;
use crate::projectiles::cylinder_hit;
use crate::world::{LevelEntity, LevelGround};

pub struct CrittersPlugin;

impl Plugin for CrittersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, tick_critters.after(MonsterTick)).add_systems(
            Update,
            (setup_level.run_if(resource_added::<MonsterLevel>), interpolate).chain(),
        );
    }
}

/// Seconds per 30 Hz tick: critters keep time in seconds.
const DT: f32 = 1.0 / 30.0;
/// Enemy type of the golem.
const GOLEM: i32 = 0x1D;

/// A wake trigger wakes the nearest statue within this (horizontally,
/// less the statue item's radius).
const WAKE_REACH: f32 = 10.0;

/// Anger: 0.5 at full health up to 5 near death.
const ANGER_SPAN: f32 = 4.5;
const ANGER_BASE: f32 = 0.5;
/// Below this anger an idle critter taunts.
const TAUNT_ANGER: f32 = 0.8;
/// Damage taken lately that makes it roar (× the player-count table, 1 for
/// one player).
const ROAR_DAMAGE: f32 = 50.0;
/// Damage taken is forgotten this long after the last blow.
const DAMAGE_MEMORY: f32 = 3.0;
/// A player a critter hit can't be hit by one again for this long, and
/// scores × [`RECENTLY_HIT`] as a target meanwhile.
const HIT_GUARD: f32 = 0.25;
const RECENTLY_HIT: f32 = 1000.0;
/// Scores at or above this mean a condition failed.
const REJECTED: f32 = 1.0e21;
/// A blocking critter takes this share of a blow.
const BLOCKED: f32 = 0.25;
/// Share of its experience every player earns for the kill.
const KILL_EXPERIENCE: f32 = 0.2;
/// Knockback per unit of push by blow kind, the golem's reduction, cap,
/// decay per tick, stop threshold and upward fall per second.
const KNOCK_HEAVY: f32 = 10.0;
const KNOCK_KNOCKDOWN: f32 = 7.5;
const KNOCK_STRONG: f32 = 5.0;
const KNOCK_DEAD: f32 = 20.0;
const KNOCK_GOLEM: f32 = 5.0;
const KNOCK_MAX: f32 = 40.0;
const KNOCK_DECAY: f32 = 0.8;
const KNOCK_STOP: f32 = 0.01;
const KNOCK_FALL: f32 = 100.0;
/// Seconds per animation frame when recording when a move ends.
const FRAME_TIME: f32 = 1.0 / 30.0;
/// Ticks into a death before `GDL_CRITTER_SHOT` saves its screenshot.
const DEATH_SHOT_DELAY: u32 = 8;
/// When a move that has never run "ended".
const NEVER: f32 = -1.0e6;
/// A critter drops at most this fast (units/s).
const MAX_DROP: f32 = 16.0;

/// Blow kind bits.
const KIND_STRONG: u32 = 0x10;
const KIND_KNOCKDOWN: u32 = 0x20;
const KIND_HEAVY: u32 = 0x100;
const KIND_REACTIONS: u32 = 0x130;

/// One critter file loaded for the level, with a model per body.
pub struct CritterKind {
    pub file: CritterFile,
    bodies: Vec<Option<Body>>,
    /// The statue a placed one stands as until it wakes.
    statue: Option<CharacterModel>,
}

/// A body's model and what its moves animate.
struct Body {
    model: CharacterModel,
    /// Per move (within the type): its action (a missing one plays the
    /// first, as the game does), and its node.
    actions: Vec<usize>,
    nodes: Vec<Option<usize>>,
    /// Per action: frames, rate, loops.
    clips: Vec<(u16, u16, bool)>,
}

/// A placed critter waiting as a statue.
struct Statue {
    placement: usize,
    enemy: i32,
    position: [f32; 3],
    yaw: f32,
    /// The item type's radius (the wake reach is measured to its edge).
    radius: f32,
    entity: Option<Entity>,
    waking: bool,
    done: bool,
}

#[derive(Component)]
struct StatueModel;

/// The level's critter state.
#[derive(Resource)]
pub struct CritterLevel {
    kinds: HashMap<i32, Arc<CritterKind>>,
    statues: Vec<Statue>,
    /// Seconds since the level started.
    now: f32,
    realm: char,
    hit_point_scale: f32,
    speed_scale: f32,
    damage_scale: f32,
    /// Per player: until when critter blows can't hit it.
    guard: HashMap<Entity, f32>,
}

/// The game's instance state (0 new, 1 dying, 3 active).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CritterState {
    New,
    Dying,
    Active,
}

/// The target the critter tracks (the game keeps up to four).
#[derive(Clone, Copy, Debug)]
struct Tracked {
    player: Entity,
    /// Horizontal distance from its centre, and the unit direction.
    distance: f32,
    direction: [f32; 2],
    position: [f32; 3],
}

/// A critter's own animation clock (the game's controller: frame, whether
/// the clip has come to its end).
#[derive(Clone, Copy, Debug)]
struct Clock {
    action: usize,
    frame: f32,
    frames: u16,
    rate: u16,
    loops: bool,
    ended: bool,
}

impl Clock {
    fn start(&mut self, action: usize, clip: (u16, u16, bool)) {
        *self = Clock { action, frame: 0.0, frames: clip.0, rate: clip.1, loops: clip.2, ended: false };
    }

    fn advance(&mut self, dt: f32) {
        let last = self.frames.saturating_sub(1) as f32;
        if last <= 0.0 {
            self.ended = true;
            return;
        }
        self.frame += dt * self.rate.max(1) as f32;
        if self.frame >= last {
            self.ended = true;
            self.frame = if self.loops { self.frame % last } else { last };
        } else if self.loops {
            self.ended = false;
        }
    }
}

/// A live critter.
#[derive(Component)]
pub struct Critter {
    kind: Arc<CritterKind>,
    ty: usize,
    pub state: CritterState,
    pub hit_points: f32,
    pub full_hit_points: f32,
    /// Root position (feet, plus its hover height).
    pub position: [f32; 3],
    pub yaw: f32,
    home_yaw: f32,
    previous: ([f32; 3], f32),
    floor: f32,
    /// Current and chosen move (within the type).
    current: Option<usize>,
    next: Option<usize>,
    /// The attack choice's target for the chosen move.
    pick: Option<Entity>,
    /// The current move's target.
    move_target: Option<Entity>,
    switched: bool,
    /// When each move last ended.
    ends: Vec<f32>,
    /// Blows and sounds done this move (bits 1, 2).
    blows_done: u8,
    sounds_done: u8,
    tracked: Option<Tracked>,
    anger: f32,
    /// Damage taken lately, its kinds, summed push, time of the last blow.
    damage_taken: f32,
    kinds: u32,
    push: [f32; 3],
    last_blow: f32,
    knock: [f32; 3],
    clock: Clock,
    /// The move node's world position this tick and last.
    node_at: Option<[f32; 3]>,
    node_was: Option<[f32; 3]>,
    blows_dealt: u32,
    /// The critter clock at its last tick (seconds since the level began).
    now: f32,
}

impl Critter {
    fn moves(&self) -> &[CritterMove] {
        self.kind.file.type_moves(self.ty)
    }

    fn body(&self) -> &Body {
        self.kind.bodies[self.ty].as_ref().expect("critters are only made with a body")
    }

    fn move_kind(&self, i: Option<usize>) -> Option<i32> {
        i.and_then(|i| self.moves().get(i)).map(|m| m.kind)
    }

    /// A blow from the hero: returns the experience it earns. `sound` gets
    /// the hit sound's name to play.
    pub fn take_hit(&mut self, damage: f32, kind_bits: u32, push: [f32; 3], now_sound: &mut Vec<String>, realm: char) -> u32 {
        if self.state != CritterState::Active || self.hit_points <= 0.0 {
            return 0;
        }
        let ty = &self.kind.file.types[self.ty];
        let (mut damage, mut kind_bits) = (damage, kind_bits);
        if self.move_kind(self.current) == Some(kind::BLOCK) {
            kind_bits &= !KIND_REACTIONS;
            damage *= BLOCKED;
        }
        damage = after_armor(damage, ty.armor);
        self.damage_taken += damage;
        let xp = (damage.min(self.hit_points) / (1.0 + self.full_hit_points) * ty.experience) as u32;
        if damage <= 0.0 {
            return xp;
        }
        self.kinds |= kind_bits;
        self.push = add(self.push, push);
        self.last_blow = self.now;
        self.hit_points -= damage;
        if self.hit_points <= 0.0 {
            self.state = CritterState::Dying;
            return xp + (KILL_EXPERIENCE * ty.experience) as u32;
        }
        if let Ok(s) = usize::try_from(ty.hit_effects[0]) {
            sound_chain(&self.kind.file, s, realm, now_sound);
        }
        xp
    }
}

/// The names of an `SFXX` record's sounds and those chained after it.
fn sound_chain(file: &CritterFile, first: usize, realm: char, out: &mut Vec<String>) {
    let mut next = Some(first);
    let mut guard = 0;
    while let Some(i) = next {
        let Some(s) = file.sounds.get(i) else { break };
        if !s.sound.is_empty() {
            out.push(s.sound.replace("%c", &realm.to_string()));
        }
        next = usize::try_from(s.next).ok();
        guard += 1;
        if guard > 8 {
            break;
        }
    }
}

/// Loads the critter files the level's enemy slots name (the golem for
/// now), their models, and places the statues.
#[allow(clippy::too_many_arguments)]
fn setup_level(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    monsters: Res<MonsterLevel>,
    population: Option<Res<LevelPopulation>>,
    ground: Option<Res<LevelGround>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(population), Some(ground)) = (population, ground) else { return };
    let realm_id = REALM_LETTERS.iter().find(|(l, _)| *l == monsters.realm).map_or(1, |r| r.1);
    let realm_items = format!("level{}", monsters.realm);
    let mut kinds = HashMap::new();
    for &(enemy, _) in &monsters.enemies.loaded {
        if enemy != GOLEM || kinds.contains_key(&enemy) {
            continue;
        }
        let Some(file_name) = critter::file_for_enemy(enemy, realm_id, "") else { continue };
        let path = format!("CRITTER/{file_name}");
        let file = match game.install.read(&path).map_err(|e| e.to_string()).and_then(|b| CritterFile::parse(&b).map_err(|e| e.to_string())) {
            Ok(f) => f,
            Err(e) => {
                warn!("{path}: {e}");
                continue;
            }
        };
        let folder = format!("MONSTERS/{}", critter::model_folder(&file.desc, &realm_items, ""));
        let Some((anim, model, textures)) = read_folder(&mut game, &folder) else {
            warn!("{folder}: can't read the critter's models");
            continue;
        };
        let data = |name: &str| -> Option<CharacterData> {
            let tree = anim.atrees.iter().find(|a| a.name.eq_ignore_ascii_case(name))?;
            Some(CharacterData {
                name: format!("{folder}/{name}"),
                class: String::new(),
                colour: String::new(),
                skeleton: tree.clone(),
                clips: Arc::new(tree.clone()),
                model: model.clone(),
                textures: textures.clone(),
            })
        };
        let bodies = (0..file.types.len())
            .map(|ty| {
                let owner = file.types[ty].parent.unwrap_or(ty);
                let d = data(&file.atree_name(owner))?;
                let moves = file.type_moves(ty);
                let actions = moves
                    .iter()
                    .map(|m| d.clips.actions.iter().position(|a| a.name == m.anim).unwrap_or(0))
                    .collect();
                let nodes = moves.iter().map(|m| (!m.node.is_empty()).then(|| d.skeleton.node_index(&m.node)).flatten()).collect();
                let clips = d.clips.actions.iter().map(|a| (a.frames, a.rate, a.loops())).collect();
                let model = CharacterModel::build(&d, &mut meshes, &mut materials, &mut images);
                Some(Body { model, actions, nodes, clips })
            })
            .collect::<Vec<_>>();
        if bodies.first().is_none_or(Option::is_none) {
            warn!("{folder}: no atree {}", file.atree_name(0));
            continue;
        }
        let statue = data("GOL_STATUE").map(|d| CharacterModel::build(&d, &mut meshes, &mut materials, &mut images));
        info!("critter {} ({path}) from {folder}: {} moves, statue {}", file.desc.name, file.moves.len(), statue.is_some());
        kinds.insert(enemy, Arc::new(CritterKind { file, bodies, statue }));
    }

    // Placed critters stand as statues.
    let pop = &population.population;
    let mut statues = Vec::new();
    for (placement, p) in pop.placements.iter().enumerate() {
        let ty = pop.resolved_type(p);
        if ty.class != ItemClass::EnemyInfo || !p.active_for(1) {
            continue;
        }
        let Some(enemy) = ty.enemy() else { continue };
        let Some(kind) = kinds.get(&enemy) else { continue };
        let mut position = p.position;
        if let Some(y) = ground.0.floor_height(position) {
            position[1] = y;
        }
        let m = gdl_formats::population::rotation_matrix(p.rotation);
        let yaw = m[6].atan2(m[8]);
        let range = match p.params(ty.class) {
            PlacementParams::Enemy { range, .. } => range,
            _ => 0.0,
        };
        let entity = kind.statue.as_ref().map(|s| {
            let t = Transform::from_translation(Vec3::from(position)).with_rotation(Quat::from_rotation_y(yaw));
            let e = s.spawn(t, &mut commands);
            commands.entity(e).insert((StatueModel, LevelEntity));
            e
        });
        debug!("critter {enemy:#x} statue at placement {placement} {position:?} (range {range})");
        statues.push(Statue {
            placement,
            enemy,
            position,
            yaw,
            radius: 0.5 * ty.extent[0].max(ty.extent[1]),
            entity,
            waking: false,
            done: false,
        });
    }
    info!("critters: {} kinds, {} statues", kinds.len(), statues.len());
    let t = &monsters.tuning;
    // `GDL_CRITTER_HP=<scale>`: a testing aid that scales critters' hit
    // points (to see one die sooner).
    let testing_scale = std::env::var("GDL_CRITTER_HP").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    commands.insert_resource(CritterLevel {
        kinds,
        statues,
        now: 0.0,
        realm: monsters.realm,
        hit_point_scale: t.monster_hit_points * testing_scale,
        speed_scale: t.monster_speed,
        damage_scale: t.monster_damage,
        guard: HashMap::new(),
    });
}

impl CritterLevel {
    /// The level's realm letter (critter sound names use it).
    pub fn realm(&self) -> char {
        self.realm
    }
}

fn read_folder(game: &mut LoadedGame, folder: &str) -> Option<(AnimFile, ModelFile, Vec<u8>)> {
    let anim = AnimFile::parse(&game.install.read(&format!("{folder}/ANIM.PS2")).ok()?).ok()?;
    let model = ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?;
    let textures = game.install.read(&format!("{folder}/textures.ngc")).ok()?;
    Some((anim, model, textures))
}

/// Makes a critter of `kind` standing at `position` facing `yaw`.
fn spawn_critter(level: &CritterLevel, kind: &Arc<CritterKind>, position: [f32; 3], yaw: f32, commands: &mut Commands) -> Option<Entity> {
    let ty = 0;
    let body = kind.bodies[ty].as_ref()?;
    let t = &kind.file.types[ty];
    let hp = t.hit_points * level.hit_point_scale;
    let position = [position[0], position[1] + t.hover, position[2]];
    let transform = Transform::from_translation(Vec3::from(position)).with_rotation(Quat::from_rotation_y(yaw));
    let root = body.model.spawn(transform, commands);
    let moves = kind.file.type_moves(ty).len();
    let critter = Critter {
        kind: kind.clone(),
        ty,
        state: CritterState::New,
        hit_points: hp,
        full_hit_points: hp,
        position,
        yaw,
        home_yaw: yaw,
        previous: (position, yaw),
        floor: position[1] - t.hover,
        current: None,
        next: None,
        pick: None,
        move_target: None,
        switched: false,
        // Never used yet: every move is ready (the game's clock has run since
        // boot, ours since the level began).
        ends: vec![NEVER; moves],
        blows_done: 0,
        sounds_done: 0,
        tracked: None,
        anger: ANGER_BASE,
        damage_taken: 0.0,
        kinds: 0,
        push: [0.0; 3],
        last_blow: 0.0,
        knock: [0.0; 3],
        clock: Clock { action: 0, frame: 0.0, frames: 1, rate: 30, loops: false, ended: true },
        node_at: None,
        node_was: None,
        blows_dealt: 0,
        now: level.now,
    };
    commands.entity(root).insert((critter, Targetable::new(TargetKind::Object, t.radius, t.height), LevelEntity));
    Some(root)
}

/// A player as the critters see it.
#[derive(Clone, Copy)]
struct Hero {
    entity: Entity,
    feet: [f32; 3],
    radius: f32,
    half: f32,
    attacking: bool,
}

#[allow(clippy::too_many_arguments)]
fn tick_critters(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    ground: Option<Res<LevelGround>>,
    mechanics: Option<ResMut<Mechanics>>,
    population: Option<Res<LevelPopulation>>,
    state: Option<Res<PlayerState>>,
    mut players: Query<(Entity, &mut Player)>,
    mut critters: Query<(Entity, &mut Critter, &mut Animator), Without<StatueModel>>,
    mut statues: Query<&mut Animator, With<StatueModel>>,
    bones: Query<&GlobalTransform>,
    mut hurt: MessageWriter<DamagePlayer>,
    mut sounds: MessageWriter<PlaySound>,
    mut death_shot: Local<Option<u32>>,
) {
    let (Some(mut level), Some(ground)) = (level, ground) else { return };
    let level = &mut *level;
    level.now += DT;
    let now = level.now;
    let alive = state.as_ref().is_none_or(|s| s.alive);
    let (radius, half) = state.as_ref().map_or((1.5, 2.5), |s| (s.radius, s.half_height));
    let heroes: Vec<Hero> = if alive {
        players
            .iter()
            .map(|(e, p)| {
                let c = p.actions.action.category().0;
                Hero { entity: e, feet: p.mover.position, radius, half, attacking: (1..=12).contains(&c) }
            })
            .collect()
    } else {
        Vec::new()
    };

    wake_statues(level, mechanics, population.as_deref(), &heroes, &mut statues, &mut commands);

    let mut blows: Vec<(Entity, f32, u32, [f32; 3])> = Vec::new();
    let mut to_play: Vec<String> = Vec::new();
    for (entity, mut c, mut animator) in &mut critters {
        let c = &mut *c;
        c.now = now;
        c.previous = (c.position, c.yaw);
        c.switched = false;
        let ty = type_info(&c.kind.file, c.ty);

        // The blows taken become knockback; old damage is forgotten.
        knockback(c);
        let cur_kind = c.move_kind(c.current);
        if (c.last_blow > 0.0 && now - c.last_blow > DAMAGE_MEMORY)
            || cur_kind == Some(kind::ROAR)
            || cur_kind.is_some_and(|k| (0x40..0x7F).contains(&k))
        {
            c.damage_taken = 0.0;
            c.kinds = 0;
            c.last_blow = 0.0;
        }

        // Targets and anger.
        let centre = centre_of(c, &ty);
        c.tracked = best_player(c, &ty, centre, &heroes, level);
        c.anger = (1.0 - c.hit_points.max(0.0) / (1.0 + c.full_hit_points)) * ANGER_SPAN + ANGER_BASE;
        if c.state == CritterState::New {
            c.state = CritterState::Active;
        }

        // Dead and done.
        if c.move_kind(c.current) == Some(kind::DEATH) && c.clock.ended {
            debug!("critter {entity:?} is gone");
            commands.entity(entity).try_despawn();
            continue;
        }

        // Choose, switch.
        c.next = None;
        c.pick = None;
        forced(c, &ty, now);
        if c.state == CritterState::Active {
            if c.next.is_none() {
                choose_block(c, now, &heroes);
            }
            if c.next.is_none() {
                choose_attack(c, now);
            }
            if c.next.is_none() {
                choose_movement(c, centre, now);
            }
            if c.next.is_none() {
                let taunt = if c.anger < TAUNT_ANGER { find(c, kind::TAUNT, Find::Ready, now) } else { None };
                c.next = taunt.or_else(|| find(c, kind::READY, Find::Nearest, now));
            }
        }
        if c.next.is_none() {
            c.next = c.current;
        }
        let was = c.current;
        switch(c, now, &mut animator);
        if c.switched && c.move_kind(c.current) == Some(kind::DEATH) && death_shot.is_none() {
            *death_shot = Some(DEATH_SHOT_DELAY);
        }
        if c.state == CritterState::Dying {
            commands.entity(entity).try_remove::<Targetable>();
        }
        if c.current != was && let Some(i) = c.current {
            debug!(
                "critter {entity:?} {} → {} (anger {:.2}, {:.0} hp)",
                was.map_or("-", |w| c.moves()[w].name.as_str()),
                c.moves()[i].name,
                c.anger,
                c.hit_points
            );
        }
        let Some(cur) = c.current else { continue };
        let mv = c.moves()[cur].clone();

        // The move's target and node.
        if c.move_target.is_none() || c.switched {
            c.move_target = c.pick.or_else(|| best_target(c, &mv.condition, true));
        }
        if c.switched {
            c.blows_done = 0;
            c.sounds_done = 0;
            c.node_was = None;
        }
        let node = c.body().nodes[cur];
        let node_matrix = match node.and_then(|n| animator.bone(n)).and_then(|b| bones.get(b).ok()) {
            Some(g) => g.affine(),
            None => bevy::math::Affine3A::from_rotation_translation(Quat::from_rotation_y(c.yaw), Vec3::from(c.position)),
        };
        c.node_was = c.node_at.filter(|_| c.node_was.is_some() || !c.switched);
        c.node_at = Some(node_matrix.translation.into());
        if c.node_was.is_none() {
            c.node_was = c.node_at;
        }

        // Blows and sounds on their frames.
        let frame = c.clock.frame as i32;
        let bits = blow_bits(&mv, frame, c.blows_done);
        for (slot, bit) in [(0usize, 1u8), (1, 2)] {
            if bits & bit == 0 {
                continue;
            }
            let first = c.blows_done & bit == 0;
            c.blows_done |= bit;
            let Ok(d) = usize::try_from(mv.damage[slot]) else { continue };
            let Some(dmg) = c.kind.file.damage.get(d).cloned() else { continue };
            deal(c, entity, &dmg, first, node_matrix, &heroes, level, &mut blows);
        }
        for (k, (s, at)) in mv.sounds.iter().enumerate() {
            let bit = 1 << k;
            if c.sounds_done & bit == 0 && *s >= 0 && i32::from(*at) <= frame {
                c.sounds_done |= bit;
                sound_chain(&c.kind.file, *s as usize, level.realm, &mut to_play);
            }
        }

        // Walk and turn.
        walk(c, &mv, &ty, &ground.0, &heroes, level.speed_scale);
        turn(c, &mv, &heroes);
        c.clock.advance(DT);
    }

    for (player, amount, kind_bits, push) in blows {
        let Ok((_, mut p)) = players.get_mut(player) else { continue };
        let amount = after_armor(amount, p.armor);
        if amount > 0.0 {
            p.queue_hit(amount, kind_bits, Vec3::from(push));
            hurt.write(DamagePlayer { amount });
        }
        info!("a critter hits the hero for {amount:.1} (kind {kind_bits:#x})");
    }
    // `GDL_CRITTER_SHOT=<png>`: a testing aid that saves a screenshot a
    // few ticks into the first critter death.
    if let Some(left) = death_shot.as_mut()
        && *left > 0
    {
        *left -= 1;
        if *left == 0
            && let Some(path) = std::env::var_os("GDL_CRITTER_SHOT")
        {
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(std::path::PathBuf::from(path)));
        }
    }
    for s in to_play {
        sounds.write(PlaySound(s));
    }
}

/// Wakes statues: from the level's wake triggers (the nearest statue
/// within [`WAKE_REACH`] of each), and with `GDL_WAKE_STATUES` when the
/// hero comes that close; a woken statue plays ACTIVE, then its critter
/// takes its place.
fn wake_statues(
    level: &mut CritterLevel,
    mechanics: Option<ResMut<Mechanics>>,
    population: Option<&LevelPopulation>,
    heroes: &[Hero],
    statues: &mut Query<&mut Animator, With<StatueModel>>,
    commands: &mut Commands,
) {
    if let (Some(mut mech), Some(pop)) = (mechanics, population) {
        for trigger in std::mem::take(&mut mech.woken) {
            let Some(at) = pop.population.placements.get(trigger).map(|p| p.position) else { continue };
            let nearest = level
                .statues
                .iter_mut()
                .filter(|s| !s.waking && !s.done)
                .map(|s| (horizontal(at, s.position) - s.radius, s))
                .filter(|(d, _)| *d < WAKE_REACH)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, s)) = nearest {
                info!("trigger {trigger} wakes the statue at placement {}", s.placement);
                s.waking = true;
            }
        }
    }
    if let Some(range) = std::env::var("GDL_WAKE_STATUES").ok().and_then(|v| v.parse::<f32>().ok()) {
        for s in level.statues.iter_mut().filter(|s| !s.waking && !s.done) {
            if heroes.iter().any(|h| distance(h.feet, s.position) < range) {
                info!("the hero wakes the statue at placement {} (GDL_WAKE_STATUES)", s.placement);
                s.waking = true;
            }
        }
    }
    let mut ready = Vec::new();
    for (i, s) in level.statues.iter_mut().enumerate() {
        if !s.waking || s.done {
            continue;
        }
        let finished = match s.entity.and_then(|e| statues.get_mut(e).ok()) {
            Some(mut a) => {
                if a.action_name() != "ACTIVE" && a.frame == 0.0 && a.play_named("ACTIVE") {
                    false
                } else {
                    a.action_name() != "ACTIVE" || a.finished()
                }
            }
            None => true,
        };
        if finished {
            s.done = true;
            if let Some(e) = s.entity.take() {
                commands.entity(e).try_despawn();
            }
            ready.push(i);
        }
    }
    for i in ready {
        let (enemy, position, yaw) = (level.statues[i].enemy, level.statues[i].position, level.statues[i].yaw);
        let Some(kind) = level.kinds.get(&enemy).cloned() else { continue };
        if let Some(e) = spawn_critter(level, &kind, position, yaw, commands) {
            info!("critter {} awakes at {position:?} ({e:?})", kind.file.desc.name);
        }
    }
}

/// The bits of `TYPE` the update reads, copied so the critter can be
/// borrowed mutably meanwhile.
#[derive(Clone, Copy)]
struct TypeInfo {
    class: i16,
    target: Condition,
    center: [f32; 3],
    radius: f32,
    height: f32,
    hover: f32,
}

fn type_info(file: &CritterFile, ty: usize) -> TypeInfo {
    let t = &file.types[ty];
    TypeInfo {
        class: file.desc.class,
        target: t.target,
        center: t.center,
        radius: t.radius,
        height: t.height,
        hover: t.hover,
    }
}

/// The blows taken turn into knockback (not for bosses): the push × a
/// factor by the blows' kinds (less for the golem), capped.
fn knockback(c: &mut Critter) {
    let class = c.kind.file.desc.class;
    if class == class::BOSS {
        return;
    }
    let mut factor = if c.hit_points <= 0.0 {
        KNOCK_DEAD
    } else if c.kinds & 0x10140 != 0 {
        KNOCK_HEAVY
    } else if c.kinds & KIND_KNOCKDOWN != 0 {
        KNOCK_KNOCKDOWN
    } else if c.kinds & KIND_STRONG != 0 {
        KNOCK_STRONG
    } else {
        0.0
    };
    if class == class::GOLEM {
        factor -= KNOCK_GOLEM;
    }
    if factor > 0.0 {
        c.knock = add(c.knock, scale(c.push, factor));
        let speed = length(c.knock);
        if speed > KNOCK_MAX {
            c.knock = scale(c.knock, KNOCK_MAX / speed);
        }
        c.push = [0.0; 3];
    }
}

/// Its centre: the root plus the type's centre offset, turned with it.
fn centre_of(c: &Critter, ty: &TypeInfo) -> [f32; 3] {
    let (s, co) = c.yaw.sin_cos();
    let o = ty.center;
    [c.position[0] + o[0] * co + o[2] * s, c.position[1] + o[1], c.position[2] - o[0] * s + o[2] * co]
}

/// The game's target score for a point: the horizontal distance over the
/// cosine between the (turned) facing and the direction, or twice the
/// distance when that's 60° or more off; a failed condition scores 1e21 or
/// more.
fn score(c: &Critter, cond: &Condition, centre: [f32; 3], point: [f32; 3]) -> (f32, f32, [f32; 2]) {
    let (dx, dy, dz) = (point[0] - centre[0], point[1] - centre[1], point[2] - centre[2]);
    let distance = (dx * dx + dz * dz).sqrt();
    let dir = if distance > 0.0 { [dx / distance, dz / distance] } else { [0.0, 1.0] };
    if c.anger < cond.min_anger || (cond.min_anger < cond.max_anger && cond.max_anger <= c.anger) {
        return (1.2e21, distance, dir);
    }
    if distance < cond.min_distance {
        return (1.01e21, distance, dir);
    }
    if cond.max_distance > 0.0 && cond.max_distance < distance {
        return (1.02e21, distance, dir);
    }
    if cond.max_height > 0.0 && cond.max_height < dy.abs() {
        return (1.03e21, distance, dir);
    }
    let h = c.yaw - cond.angle;
    let cos = h.sin() * dir[0] + h.cos() * dir[1];
    if cos < cond.min_cos {
        return (1.1e21, distance, dir);
    }
    let s = if cos <= 0.5 { distance * 2.0 } else { distance / cos.abs() };
    (s, distance, dir)
}

/// The best player by the type's target condition; players a critter hit
/// in the last quarter second count a thousand times worse.
fn best_player(c: &Critter, ty: &TypeInfo, centre: [f32; 3], heroes: &[Hero], level: &CritterLevel) -> Option<Tracked> {
    let mut best: Option<(f32, Tracked)> = None;
    for h in heroes {
        let (mut s, distance, direction) = score(c, &ty.target, centre, h.feet);
        if level.guard.get(&h.entity).is_some_and(|&until| level.now < until) {
            s *= RECENTLY_HIT;
        }
        if best.is_none_or(|(b, _)| s < b) {
            best = Some((s, Tracked { player: h.entity, distance, direction, position: h.feet }));
        }
    }
    best.map(|b| b.1)
}

/// The tracked target, if it meets `cond` (or anyway, with `fallback`).
fn best_target(c: &Critter, cond: &Condition, fallback: bool) -> Option<Entity> {
    let t = c.tracked?;
    let failed = c.anger < cond.min_anger
        || (cond.min_anger < cond.max_anger && cond.max_anger <= c.anger)
        || t.distance < cond.min_distance
        || (cond.max_distance > 0.0 && cond.max_distance < t.distance)
        || {
            let h = c.yaw - cond.angle;
            h.sin() * t.direction[0] + h.cos() * t.direction[1] < cond.min_cos
        };
    (!failed || fallback).then_some(t.player)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Find {
    /// Only a move off cooldown.
    Ready,
    /// The one nearest to ready.
    Nearest,
}

/// The game's move lookup by kind: skips disabled moves; the one whose
/// cooldown runs out soonest (only ready ones for [`Find::Ready`]).
fn find(c: &Critter, k: i32, mode: Find, now: f32) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (i, m) in c.moves().iter().enumerate() {
        if m.flags & 4 != 0 || m.kind != k {
            continue;
        }
        let left = if m.cooldown <= 0.0 { 0.0 } else { c.ends[i] + m.cooldown - now };
        if (left <= 0.0 || mode == Find::Nearest) && best.is_none_or(|(b, _)| left < b) {
            best = Some((left, i));
        }
    }
    match best {
        Some((_, i)) => Some(i),
        None if mode == Find::Nearest && k != kind::READY => {
            warn!("critter can't find move kind {k:#x}");
            find(c, kind::READY, Find::Nearest, now)
        }
        None => None,
    }
}

fn ready(c: &Critter, i: usize, now: f32) -> bool {
    let m = &c.moves()[i];
    m.cooldown <= 0.0 || c.ends[i] + m.cooldown <= now
}

/// The moves forced on it: INIT when new, START after INIT, DEATH when
/// dying, a move's follow-up; then reactions to the blows taken.
fn forced(c: &mut Critter, ty: &TypeInfo, now: f32) {
    let cur = c.current.map(|i| c.moves()[i].clone());
    let next = match (&cur, c.state) {
        (None, _) | (_, CritterState::New) => find(c, kind::INIT, Find::Nearest, now),
        (Some(m), _) if m.kind == kind::INIT => find(c, kind::START, Find::Ready, now),
        (_, CritterState::Dying) => find(c, kind::DEATH, Find::Nearest, now),
        (Some(m), _) if m.next >= 0 => Some(m.next as usize),
        (Some(m), _) if m.kind == kind::START && ty.class == class::BOSS => find(c, kind::READY, Find::Ready, now),
        _ => None,
    };
    c.next = next;
    if c.next.is_none() && c.kinds & 0x120 != 0 {
        if c.kinds & KIND_HEAVY != 0 {
            c.next = find(c, kind::KNOCKDOWN, Find::Ready, now);
        }
        if c.next.is_none() {
            c.next = find(c, kind::KNOCKBACK, Find::Ready, now);
        }
    }
    if c.next.is_none() && c.damage_taken >= ROAR_DAMAGE {
        c.next = find(c, kind::ROAR, Find::Ready, now);
    }
    if c.next.is_none() && c.kinds & KIND_STRONG != 0 {
        c.next = find(c, kind::FLINCH, Find::Ready, now);
    }
    c.kinds &= !KIND_REACTIONS;
}

/// Whether a move's node (and its follow-up's) are there.
fn has_nodes(c: &Critter, i: usize) -> bool {
    let body = c.body();
    let m = &c.moves()[i];
    body.nodes[i].is_some() && (m.next < 0 || body.nodes.get(m.next as usize).copied().flatten().is_some())
}

/// A block when the target it would block is attacking.
fn choose_block(c: &mut Critter, now: f32, heroes: &[Hero]) {
    for i in 0..c.moves().len() {
        let m = c.moves()[i].clone();
        if m.kind != kind::BLOCK || m.flags & 4 != 0 || (m.flags & 0x10 != 0 && !has_nodes(c, i)) || !ready(c, i, now) {
            continue;
        }
        if let Some(t) = best_target(c, &m.condition, false)
            && heroes.iter().any(|h| h.entity == t && h.attacking)
        {
            c.next = Some(i);
        }
    }
}

/// The least recently used attack whose condition finds a target.
fn choose_attack(c: &mut Critter, now: f32) {
    let mut best_time = 999_999.0f32;
    let mut chosen: Option<usize> = None;
    for i in 0..c.moves().len() {
        let m = c.moves()[i].clone();
        if Some(i) == c.current || !m.is_attack() || m.flags & 4 != 0 {
            continue;
        }
        if m.flags & 0x10 != 0 && !has_nodes(c, i) {
            continue;
        }
        if !ready(c, i, now) {
            continue;
        }
        let Some(t) = best_target(c, &m.condition, false) else { continue };
        if c.ends[i] < best_time {
            best_time = c.ends[i];
            chosen = Some(i);
            c.pick = Some(t);
        } else if chosen.is_none_or(|b| transition(&c.moves()[b], &m) > 1) {
            chosen = Some(i);
            c.pick = Some(t);
        }
    }
    c.next = chosen;
}

/// The movement move that scores its target best.
fn choose_movement(c: &mut Critter, centre: [f32; 3], now: f32) {
    let Some(t) = c.tracked else { return };
    let mut best = REJECTED;
    for i in 0..c.moves().len() {
        let m = &c.moves()[i];
        if !m.is_movement() || m.flags & 4 != 0 || (m.kind == kind::GO_TO_POINT) || !ready(c, i, now) {
            continue;
        }
        let (s, _, _) = score(c, &m.condition, centre, t.position);
        if s < best {
            best = s;
            c.next = Some(i);
        }
    }
}

/// How a move may give way to the next (0 when its animation ends and the
/// move differs, 1 when it ends, 2 at once), by the playing move's
/// transition and the two priorities.
fn transition(cur: &CritterMove, next: &CritterMove) -> u8 {
    let (a, b) = (cur.priority, next.priority);
    match cur.transition {
        0 => 0,
        0x14 => {
            if (b & !0xFF) <= (a & !0xFF) {
                1
            } else {
                2
            }
        }
        0x3C => {
            if b < a {
                1
            } else {
                2
            }
        }
        0x50 => {
            if b < 1 {
                1
            } else {
                2
            }
        }
        0x5A => 2,
        _ => {
            if a < b {
                2
            } else {
                1
            }
        }
    }
}

/// Switches to the chosen move the way the transition allows, and starts
/// its animation; a switch records when the move will end (its cooldown
/// runs from then).
fn switch(c: &mut Critter, now: f32, animator: &mut Animator) {
    let cur = c.current.map(|i| c.moves()[i].clone());
    let next = c.next;
    let (target, mode) = match (&cur, next) {
        (_, None) => (c.current, 0u8),
        (None, Some(n)) => (Some(n), 3),
        (Some(m), Some(n)) => {
            let nm = &c.moves()[n];
            if Some(n) != c.current && nm.priority >= 0xF00 && m.transition != 0 {
                (Some(n), 3)
            } else {
                (Some(n), transition(m, nm))
            }
        }
    };
    let mode = if target != c.current && mode == 0 && c.clock.ended { 1 } else { mode };
    let Some(t) = target else { return };
    let action = c.body().actions[t];
    let differs = action != c.clock.action || c.current.is_none();
    let go = match mode {
        0 => c.clock.ended && differs,
        1 => c.clock.ended,
        2 => c.clock.ended || differs,
        _ => true,
    };
    if !go {
        if next.is_none() && c.clock.ended {
            c.current = None;
        }
        return;
    }
    let clip = c.body().clips.get(action).copied().unwrap_or((1, 30, false));
    c.clock.start(action, clip);
    animator.play(action);
    c.switched = true;
    c.current = Some(t);
    c.ends[t] = now + FRAME_TIME * (f32::from(clip.0) - 2.0);
}

/// Which of the move's two blows land this frame (bits 1, 2): sweeps land
/// every frame of their windows, the rest once from their frames.
fn blow_bits(m: &CritterMove, frame: i32, done: u8) -> u8 {
    if m.hit_frames[0] < 0 {
        return 0;
    }
    let mut bits = 0;
    match m.kind {
        kind::SWEEP | kind::SWEEP_2 | kind::SWEEP_3 => {
            if m.hit_frames[0] <= frame && frame <= i32::from(m.hit_ends[0]) {
                bits |= 1;
            }
            if m.hit_frames[1] >= 0 && m.hit_frames[1] <= frame && frame <= i32::from(m.hit_ends[1]) {
                bits |= 2;
            }
        }
        _ => {
            if done & 1 == 0 && m.hit_frames[0] <= frame {
                bits |= 1;
            }
            if m.hit_frames[1] >= 0 && done & 2 == 0 && m.hit_frames[1] <= frame {
                bits |= 2;
            }
        }
    }
    bits
}

/// A blow: kind 0 is a sphere on the move's node, swept from where it was
/// last tick, against each player's cylinder; kind 3 (the golem's stomp
/// ring) hurts players within its radius at once (stand-in for the game's
/// damaging effect). Other kinds need the effects and projectiles and
/// aren't done.
#[allow(clippy::too_many_arguments)]
fn deal(
    c: &mut Critter,
    me: Entity,
    d: &CritterDamage,
    first: bool,
    node: bevy::math::Affine3A,
    heroes: &[Hero],
    level: &mut CritterLevel,
    blows: &mut Vec<(Entity, f32, u32, [f32; 3])>,
) {
    let damage = d.damage * level.damage_scale;
    let offset = node.matrix3 * Vec3::from(d.offset);
    let at = Vec3::from(c.node_at.unwrap_or(c.position)) + offset;
    let was = Vec3::from(c.node_was.unwrap_or(c.position)) + offset;
    for h in heroes {
        if level.guard.get(&h.entity).is_some_and(|&until| level.now < until) {
            continue;
        }
        let centre = Vec3::from(h.feet) + Vec3::Y * h.half;
        let hit = match d.kind {
            0 => cylinder_hit(was, at, centre, h.radius + d.radius, h.half + d.radius).is_some(),
            3 if first => {
                let v = centre - at;
                Vec2::new(v.x, v.z).length() <= d.radius + h.radius && v.y.abs() <= h.half + d.radius
            }
            _ => false,
        };
        if !hit {
            continue;
        }
        let towards = Vec3::new(h.feet[0] - c.position[0], 1.0, h.feet[2] - c.position[2]).normalize_or_zero();
        let push = 0.5 * ((at - was) + towards);
        blows.push((h.entity, damage, d.blow, push.to_array()));
        level.guard.insert(h.entity, level.now + HIT_GUARD);
        c.blows_dealt += 1;
        debug!("critter {me:?} blow kind {} lands for {damage:.1}", d.kind);
    }
    if first && !matches!(d.kind, 0 | 3) {
        debug!("critter {me:?}: damage kind {} not done yet", d.kind);
    }
}

/// Walks at the move's speed (× the level's monster speed) in its
/// direction, plus the knockback, on the level's collision; stops short of
/// players.
fn walk(c: &mut Critter, m: &CritterMove, ty: &TypeInfo, collision: &LevelCollision, heroes: &[Hero], speed_scale: f32) {
    let s = m.speed * speed_scale * DT;
    let f = [c.yaw.sin(), c.yaw.cos()];
    let (dx, dz) = match m.kind {
        kind::BACK => (-s * f[0], -s * f[1]),
        kind::WALK_RIGHT => (s * f[1], -s * f[0]),
        kind::WALK_LEFT => (-s * f[1], s * f[0]),
        kind::WALK_DIAGONAL => (-s * f[1] + s * f[0], s * f[1] + s * f[0]),
        _ => (s * f[0], s * f[1]),
    };
    let mut v = [dx + c.knock[0] * DT, c.knock[1] * DT, dz + c.knock[2] * DT];
    for (i, k) in c.knock.iter_mut().enumerate() {
        *k *= KNOCK_DECAY;
        if k.abs() < KNOCK_STOP {
            *k = 0.0;
        }
        if i == 1 && *k > 0.0 {
            *k = (*k - KNOCK_FALL * DT).max(0.0);
        }
    }
    // Players stop it.
    let from = [c.position[0], c.position[1], c.position[2]];
    let to = [from[0] + v[0], from[1], from[2] + v[2]];
    for h in heroes {
        let reach = ty.radius + h.radius;
        let (ax, az) = (to[0] - h.feet[0], to[2] - h.feet[2]);
        let (bx, bz) = (from[0] - h.feet[0], from[2] - h.feet[2]);
        if ax * ax + az * az < reach * reach && ax * ax + az * az < bx * bx + bz * bz {
            v[0] = 0.0;
            v[2] = 0.0;
        }
    }
    // Walls from its centre, then the floor at the leading edge.
    let feet_y = c.position[1] - ty.hover;
    let start = [c.position[0], feet_y + 2.0, c.position[2]];
    if (v[0] != 0.0 || v[2] != 0.0)
        && let Some(hit) = collision.wall(start, add(start, [v[0], 0.0, v[2]]), ty.radius)
        && collision.nodes[hit.node].flags & node_flags::NO_PUSH == 0
    {
        let mut d = [v[0], 0.0, v[2]];
        if push_out(ty.radius, start, &mut d, hit.point, hit.normal) {
            d = [0.0; 3];
        }
        v[0] = d[0];
        v[2] = d[2];
    }
    let probe = |at: [f32; 3]| collision.floor_probe(at, ty.height, -ty.height - 3.0, 1.0, 2);
    let len = (v[0] * v[0] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        let edge = [start[0] + v[0] / len * (ty.radius + len), feet_y, start[2] + v[2] / len * (ty.radius + len)];
        let mut ok = false;
        if let Some(hit) = probe(edge) {
            let rise = (hit.point[1] - c.floor).abs();
            if rise <= 2.0 * (ty.radius + len) {
                ok = true;
                c.floor = hit.point[1];
                if rise > 0.1 * len {
                    match probe([from[0] + v[0], feet_y, from[2] + v[2]]) {
                        Some(h) => c.floor = h.point[1],
                        None => ok = false,
                    }
                }
            }
        }
        if !ok {
            v[0] = 0.0;
            v[2] = 0.0;
        }
    } else if let Some(h) = probe([from[0], feet_y, from[2]]) {
        c.floor = h.point[1];
    }
    let dy = (c.floor - feet_y).max(-MAX_DROP * DT) + v[1].max(0.0);
    c.position = [from[0] + v[0], c.position[1] + dy, from[2] + v[2]];
}

/// Turns toward the move's target at the move's rate (toward home for
/// moves flagged so); without the free-turning flag, no further than the
/// type allows from the way it was made facing.
fn turn(c: &mut Critter, m: &CritterMove, heroes: &[Hero]) {
    let ty = &c.kind.file.types[c.ty];
    let goal = if m.flags & 0x20 != 0 {
        Some(c.home_yaw)
    } else {
        c.move_target.and_then(|t| heroes.iter().find(|h| h.entity == t)).map(|h| {
            let g = (h.feet[0] - c.position[0]).atan2(h.feet[2] - c.position[2]);
            if ty.flags & 0x400 != 0 {
                g
            } else {
                let d = locomotion::wrap(g - c.home_yaw).clamp(-ty.max_turn, ty.max_turn);
                c.home_yaw + d
            }
        })
    };
    let Some(goal) = goal else { return };
    let rate = m.turn * DT;
    let d = locomotion::wrap(goal - c.yaw).clamp(-rate, rate);
    c.yaw = locomotion::wrap(c.yaw + d);
}

fn interpolate(fixed: Res<Time<Fixed>>, mut critters: Query<(&Critter, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (c, mut transform) in &mut critters {
        let (p0, f0) = c.previous;
        transform.translation = Vec3::from(p0).lerp(Vec3::from(c.position), t);
        transform.rotation = Quat::from_rotation_y(f0 + locomotion::wrap(c.yaw - f0) * t);
    }
}

fn horizontal(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    length([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(kind: i32, priority: i32, transition: i16, hit_frames: [i32; 2], hit_ends: [i16; 2]) -> CritterMove {
        CritterMove {
            kind,
            flags: 0,
            priority,
            name: String::new(),
            anim: String::new(),
            node: String::new(),
            hit_frames,
            damage: [-1, -1],
            hit_ends,
            next: -1,
            transition,
            sounds: [(-1, -1); 2],
            condition: Condition::default(),
            cooldown: 0.0,
            speed: 0.0,
            turn: 0.0,
            hold: 0.0,
        }
    }

    #[test]
    fn transitions_follow_the_priorities() {
        let walk = mv(kind::WALK, 0x210, 0x14, [-1, -1], [-1, -1]);
        let attack = mv(kind::SWEEP, 0x220, 0x14, [6, -1], [7, -1]);
        let knockback = mv(kind::KNOCKBACK, 0xD00, 0x14, [-1, -1], [-1, -1]);
        let start = mv(kind::START, 0xFFF, 0, [-1, -1], [-1, -1]);
        // Same priority band: wait for the animation to end.
        assert_eq!(transition(&walk, &attack), 1);
        // A higher band cuts in at once.
        assert_eq!(transition(&walk, &knockback), 2);
        // START gives way only when it's done.
        assert_eq!(transition(&start, &walk), 0);
    }

    #[test]
    fn sweeps_land_every_frame_of_their_window_others_once() {
        let sweep = mv(kind::SWEEP, 0x220, 0x14, [6, -1], [7, -1]);
        assert_eq!(blow_bits(&sweep, 5, 0), 0);
        assert_eq!(blow_bits(&sweep, 6, 0), 1);
        assert_eq!(blow_bits(&sweep, 7, 1), 1);
        assert_eq!(blow_bits(&sweep, 8, 1), 0);
        let stomp = mv(0x82, 0x220, 0x14, [10, -1], [-1, -1]);
        assert_eq!(blow_bits(&stomp, 12, 0), 1);
        assert_eq!(blow_bits(&stomp, 13, 1), 0);
        let idle = mv(kind::READY, 0x200, 0x14, [-1, -1], [-1, -1]);
        assert_eq!(blow_bits(&idle, 3, 0), 0);
    }

    #[test]
    fn the_clock_ends_clips_and_loops_them() {
        let mut c = Clock { action: 0, frame: 0.0, frames: 1, rate: 30, loops: false, ended: true };
        c.start(1, (4, 30, false));
        assert!(!c.ended);
        for _ in 0..3 {
            c.advance(DT);
        }
        assert!(c.ended && c.frame == 3.0);
        c.start(2, (4, 30, true));
        for _ in 0..3 {
            c.advance(DT);
        }
        assert!(c.ended);
        c.advance(DT);
        assert!(!c.ended && c.frame < 3.0);
    }
}
