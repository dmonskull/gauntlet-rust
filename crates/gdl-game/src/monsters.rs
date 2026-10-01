//! Monsters: the regular enemies generators make and levels place, run the
//! way the game's per-frame monster update does (`docs/monsters.md`) — pick
//! the nearest player they're aware of and walk at it (the chasers) or
//! wander until one comes close (the small ones), at their type's speed and
//! turn rate, against the level's collision and each other, and attack when
//! they bump into a player. Stats are the game's per-type tables
//! (`gdl_formats::enemy`) scaled by the level's tuning record.
//!
//! Runs on the 30 Hz fixed tick after the player, interpolated for drawing.
//! Each landed hit is a [`MonsterHit`] message for the health code to read.
//! The throwing AIs stand (or back off) and throw; each missile they release
//! is a [`MonsterShot`] for `projectiles.rs` (`docs/projectiles.md`).

use gdl_formats::detmath::Det;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_6, PI};
use std::sync::Arc;

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;
use gdl_formats::anim::AnimFile;
use gdl_formats::collision::{node_flags, push_out};
use gdl_formats::enemy::{self, ACTION_NAMES, EnemyInstance, EnemyScales, FIELDS_PER_TICK, LevelEnemies};
use gdl_formats::texmod::TexMod;
use gdl_formats::{LevelCollision, LevelTuning, ModelFile, WorldData};
use gdl_install::GameInstall;

use crate::audio::{CALL_VOLUME, LoopSoundAt, PlaySoundAt};
use crate::combat::{TargetKind, Targetable};
use crate::character::{Animator, CharacterData, CharacterModel};
use crate::deaths::{self, DeathSet, DeathTextures, Dissolve};
use crate::flash::{self, Flash, FlashColours};
use bevy::mesh::MeshTag;
use crate::effects::{EffectAt, EffectOn, Exploder, ExplosionAt};
use crate::fade::Fade;
use crate::hints::{Hint, ShowHint};
use crate::generators::{self, Generator};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::play_camera::PlayCamera;
use crate::player::{Player, PlayerTick};
use crate::player_state::{EnemyScale, TimeStop, power};
use crate::population::LevelPopulation;
use crate::projectiles::{self, MonsterShot};
use crate::texanim::{LevelTexAnims, TexAnim};
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

/// The suicide runners' AI (`docs/monsters.md`, "Suicide runners").
const SUICIDE: i16 = 0x12;
/// The unaware AIs, 5 and 6 (`docs/monsters.md`, "Unaware monsters"):
/// they walk their heading and turn a quarter away from what blocks them,
/// 5 one way and 6 the other, then again after 20 fields.
const UNAWARE: i16 = 5;
const UNAWARE_TURN: f32 = FRAC_PI_2;
const UNAWARE_HOLD: f32 = 20.0;
/// Their look ahead for walls: this far past their radius.
const UNAWARE_LOOK: f32 = 0.5;
/// The AIs that hand a tick with no aware target over to the unaware ones
/// (5 or 6 by the monster's slot); the game puts each monster's own AI
/// back after its tick. (The suicide runner does too, once it runs.)
const GOES_UNAWARE: [i16; 12] = [0, 1, 3, 7, 8, 10, 0xD, 0xE, 0x13, 0x14, 0x15, 0x16];
/// The AIs that run from a charging suicide runner near them (the level's
/// "leader"): within 10 units of it, while its player is within their
/// awareness.
const FLEES_RUNNER: [i16; 15] = [0, 1, 2, 4, 5, 6, 7, 8, 10, 0xC, 0xD, 0xE, 0xF, 0x10, 0x16];
const FLEE_REACH_SQ: f32 = 100.0;
/// They run (RUN at twice their speed) straight away from it.
const FLEE_SPEED: f32 = 2.0;
/// The AI a fleeing monster runs for the tick.
const FLEE: i16 = 0x18;
/// The wanderers (AI 2/4) chase a player within this distance.
const WANDER_CHASE: f32 = 8.0;
/// Once a player is within its awareness a runner waits this many video
/// fields, then gets up (READYTOWALK) and runs.
const SUICIDE_WAIT: f32 = 60.0;
/// Fields it runs before it blows up anyway.
const SUICIDE_RUN: f32 = 240.0;
/// Its run: RUN at 1.5 × its speed.
const SUICIDE_SPEED: f32 = 1.5;
/// While it keeps running into things it tries these heading offsets in
/// turn: ±5°, ±10° … ±40°.
const SUICIDE_OFFSETS: [f32; 16] = {
    let mut o = [0.0; 16];
    let mut i = 0;
    while i < 16 {
        let a = (i / 2 + 1) as f32 * 5.0 * PI / 180.0;
        o[i] = if i % 2 == 0 { a } else { -a };
        i += 1;
    }
    o
};
/// Out of offsets, it only runs on while its heading is at least 6° off
/// the one it had when the bumping began.
const SUICIDE_TURN: f32 = 6.0 * PI / 180.0;
/// Its blast: 50 × the level's damage scale.
const SUICIDE_DAMAGE: f32 = 50.0;
/// Blowing itself up is a fire blow.
const SUICIDE_KIND: u32 = 1;
/// The realms whose runners leave a poison cloud instead (7 and 11).
const POISON_REALMS: [char; 2] = ['G', 'K'];
const SUICIDE_YELL: &str = "S_SUICIDE_YELL";
/// The runner's yell: faded, at this requested volume.
const RUNNER_VOLUME: u8 = 0xE0;
/// The effect a level's runners take from the first of its (non-special)
/// monster folders that has it.
const SUICIDE_EFFECT: &str = "SUICIDEEXP";

pub struct MonstersPlugin;

impl Plugin for MonstersPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MonsterHit>()
            .add_message::<DeathDrain>()
            .add_systems(Startup, load_level_tunings)
            .add_systems(
                FixedUpdate,
                (generators::tick_generators, generators::tick_placed, tick_monsters)
                    .chain()
                    .after(PlayerTick)
                    .in_set(MonsterTick),
            )
            .add_systems(
                Update,
                (setup_level.run_if(resource_exists_and_changed::<LevelPopulation>), interpolate, log_hits).chain(),
            );
    }
}

/// The monsters' 30 Hz tick (generators, placed monsters, the monsters).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MonsterTick;

/// A monster's third attack hits half as hard again; shrunk, every blow
/// does half.
const STRONG_BLOW: f32 = 1.5;
const SHRUNK_BLOW: f32 = 0.5;

/// A monster's attack landed on a player.
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
pub(crate) struct LevelTunings(HashMap<String, (LevelTuning, LevelEnemies, MonsterSoundTable)>);

impl LevelTunings {
    /// The enemy types a level loads (its realm's swap for the
    /// placeholders), by its folder name.
    pub(crate) fn enemies(&self, level: &str) -> Option<&LevelEnemies> {
        self.0.get(&level.to_ascii_lowercase()).map(|e| &e.1)
    }
}

/// Each enemy type's hit and death sounds on a level.
pub type MonsterSoundTable = HashMap<i32, Arc<enemy::MonsterSounds>>;

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
                    let realm_enemies = world.level_enemies(level);
                    let mut loaded: Vec<(i32, i32)> = realm_enemies.iter().map(|e| (e.enemy, e.subtype)).collect();
                    // Every level without a boss loads Death too, in a
                    // free slot.
                    if level.tuning.boss_enemy < 0 && !loaded.iter().any(|(t, _)| *t == DEATH_TYPE) {
                        loaded.push((DEATH_TYPE, DEATH_SLOT));
                    }
                    let realm = level.name.chars().next().unwrap_or('A').to_ascii_uppercase();
                    let sounds = realm_enemies
                        .iter()
                        .filter_map(|e| {
                            let s = enemy::monster_sounds(e.enemy, e.subtype, &e.name, level.tuning.boss_enemy, realm)?;
                            Some((e.enemy, Arc::new(s)))
                        })
                        .collect();
                    let gargoyle = realm_enemies.iter().find(|e| e.enemy == enemy::GARGOYLE).map(|e| e.variant.clone()).unwrap_or_default();
                    tunings.0.insert(level.folder().to_ascii_lowercase(), (level.tuning, LevelEnemies { loaded, gargoyle }, sounds));
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
    /// The level's tuning record (zero scales read as 1).
    pub tuning: LevelTuning,
    /// The enemy types the level loads, and from which folders.
    pub enemies: LevelEnemies,
    /// Most monsters alive at once.
    pub slots: usize,
    /// The level's experience level and scale (`LevelTuning`).
    pub experience: (f32, f32),
    /// Each enemy type's hit and death sounds.
    pub sounds: MonsterSoundTable,
    /// The level folder's realm letter (`A` for `levelA1`).
    pub realm: char,
    /// The level's boss type, or -1.
    pub boss: i32,
    /// The monster folder holding the level's SUICIDEEXP effect.
    pub suicide_folder: Option<String>,
    /// Per (enemy type, tier): the model, if one could be found.
    models: HashMap<(i32, i32), Option<Arc<MonsterModel>>>,
    /// 30 Hz ticks since the level started.
    pub tick: u32,
    rng: u32,
    /// Monsters created so far (each gets the next number: it staggers
    /// their target searches like the game's slot index does).
    created: u32,
    /// The leader (`r13-0x73b8`): the first suicide runner seen charging
    /// near the screen, kept until it's gone.
    leader: Option<Entity>,
    /// A Death drained last tick (`S_DEATHSUCK` loops).
    death_sucking: bool,
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

    /// What the machines compare online (`online.rs`).
    pub fn sync_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.tick, self.rng, self.created, self.death_sucking).hash(&mut h);
        h.finish()
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
    /// 1–3 by its hit points when it was made against its type's full
    /// ones (the tier; special variants count as 2). Picks its sounds.
    pub strength: i16,
    /// Blows that have done damage.
    pub hits: i16,
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
    /// This tick's AI (the record's `+0x310`): its own, or one it hands
    /// the tick over to — unaware, or fleeing a charging runner.
    frame_ai: i16,
    number: u32,
    /// Blows taken since the last tick: damage, kind bits, push.
    pending: (f32, u32, [f32; 3]),
    /// Knockback velocity, units per second.
    knock: [f32; 3],
    /// The throwing AIs: video fields before the first throw, a random
    /// 0–9 when the AI starts.
    throw_timer: f32,
    /// Seconds during which attacks and throws are refused, and the
    /// fraction carried between throws (`docs/projectiles.md`).
    throw_pause: f32,
    throw_carry: f32,
    /// What each throw adds to the pause (1; placed monsters can set it).
    throw_rate: f32,
    /// A kiting thrower backing away from a player that came too close.
    retreat: bool,
    /// A suicide runner's progress (AI 0x12).
    suicide: Suicide,
    /// Killed: playing out its death (`deaths.rs`).
    pub dying: Option<Dying>,
    /// Its hit flash (`flash.rs`).
    flash: Flash,
    /// Death's own state (type 30 only).
    pub death: DeathState,
}

/// Death (`docs/monsters.md`, "Death"): the drain's timer and contact
/// delay, what it drained and who it runs from, and its leaving.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeathState {
    /// Video fields to its next drain (`+0x208`).
    timer: f32,
    /// Contacts it lets pass before it drains (`+0x2D4`: 1 when placed).
    delay: u8,
    /// The hero it's draining or drained last (`+0x284`).
    pub drained: Option<Entity>,
    /// It drank its fill and leaves (`+0x320`).
    pub left: bool,
    /// The nearest hero with the halo, which it runs from (`+0x328`).
    halo_hero: Option<Entity>,
    /// Its run's nudges while blocked (`+0x324`).
    nudge: usize,
    /// Leaving: how see-through it is (`+0x388`, of 255).
    fade: f32,
    /// Its drain effect is on (`+0x1E0`).
    drain_effect: bool,
}

/// A Death drains a hero it touches (`damage.rs` applies it): `amount`
/// of health, or of experience steps when `experience`.
#[derive(Message, Clone, Copy, Debug)]
pub struct DeathDrain {
    pub death: Entity,
    pub hero: Entity,
    pub amount: f32,
    pub experience: bool,
}

/// Marks a Death's entity (the halo's drain looks for one): whether it's
/// a tier-2 Death, which drains (and gives up) experience.
#[derive(Component)]
pub struct DeathMonster {
    pub experience: bool,
}

/// Death's type, and the armour bit of the halo that beats it.
pub const DEATH_TYPE: i32 = enemy::DEATH;
/// The slot subtype Death is loaded in (none of the numbered slots, which
/// the placeholder monsters are swapped by).
const DEATH_SLOT: i32 = 0;
const HALO: u32 = 0x8_0000;
/// Video fields between its drains.
const DEATH_DRAIN_FIELDS: f32 = 3.0;
/// Its run from a haloed hero, at its walk × this; the heading nudges it
/// tries in turn while the run is blocked (degrees).
const DEATH_RUN: f32 = 0.9;
const DEATH_NUDGES: [f32; 8] = [5.0, -5.0, 10.0, -10.0, 15.0, -15.0, 20.0, -20.0];
/// Leaving, it rises this fast (units a second) and grows see-through by
/// this much of 255 a video field.
const DEATH_RISE: f32 = 10.0;
const DEATH_FADE: f32 = 4.0;
/// Its sounds: the drain's loop, the laugh as it leaves full.
const DEATH_SUCK: &str = "S_DEATHSUCK";
const DEATH_SUCK_LOOP: &str = "death_suck";
const DEATH_LAUGH: &str = "S_DEATHLAUGH";
/// Its laugh plays panned at its feet at this requested volume; the drain
/// follows it at the call's own.
const DEATH_LAUGH_VOLUME: u8 = 0xE0;
/// Its drain effects (`MONSTERS/DEATH`): health, experience.
const DEATH_BANK: &str = "MONSTERS/DEATH";
const DEATH_ARC: &str = "DEATH_ARC";
const DEATH_EXP: &str = "DEATH_EXP";

/// A suicide runner's progress toward blowing up.
#[derive(Clone, Copy, Debug, Default)]
struct Suicide {
    /// 0 waiting for a player, 1 about to get up, 2 running.
    stage: u8,
    /// Video fields left before it gets up.
    wait: f32,
    /// Video fields it has run.
    running: f32,
    /// The next heading offset to try while it keeps bumping into things,
    /// and its heading when the bumping began.
    step: usize,
    saved: f32,
}

/// A killed monster's death under way.
#[derive(Clone, Copy, Debug)]
pub struct Dying {
    /// The death texture its body goes through; none: it goes at once.
    pub set: Option<DeathSet>,
    /// The killing blow's kind (its element picks the die effect), and
    /// where it landed.
    pub kind: u32,
    pub blow: [f32; 3],
    /// The death texture's counter (`deaths::DEATH_START`, a step a tick).
    pub step: f32,
    /// The die effect and the texture are on.
    started: bool,
    /// Nobody killed it (a suicide runner blowing itself up): no die
    /// effect.
    quiet: bool,
}

impl Monster {
    /// A blow lands: hit points go at once; the reaction (flinch or
    /// knockdown, and the push) follows on its next tick.
    pub fn take_hit(&mut self, damage: f32, kind: u32, push: [f32; 3]) {
        self.hit_points -= damage;
        self.pending.0 += damage;
        self.pending.1 |= kind;
        self.pending.2 = add(self.pending.2, push);
    }

    /// The killing blow (of `kind`, landing at `blow`): it starts dying.
    /// Death rises and fades instead of dissolving, with no effect.
    pub fn die(&mut self, kind: u32, blow: [f32; 3]) {
        let death = self.enemy == DEATH_TYPE;
        let set = if death { None } else { deaths::death_set(self.enemy, self.stats.step, kind) };
        self.dying = Some(Dying { set, kind, blow, step: deaths::DEATH_START, started: false, quiet: death });
    }

    /// Its centre: feet plus its type's centre height.
    pub fn centre(&self) -> Vec3 {
        Vec3::from(self.position) + Vec3::Y * enemy::enemy_stats(self.enemy).map_or(0.0, |s| s.center_height)
    }

    /// The death texture frame a dying body shows, once it's started.
    pub fn death_frame(&self) -> Option<usize> {
        self.dying.filter(|d| d.step >= 0.0).map(|d| d.step as usize)
    }
}

/// Hit kinds that knock a monster down (HIT2 and a big push), and the one
/// that pushes a flinch a little (the `Hit::kind` bits).
const KNOCKDOWN_KINDS: u32 = 0x10160;
const BIG_HIT_KIND: u32 = 0x200;
const BIG_HIT_DAMAGE: f32 = 10.0;
const STRONG_KIND: u32 = 0x10;
/// Push multipliers: knocked down (a big monster gets half), a strong
/// flinch; knockback speed is capped at 40, keeps 0.8 of itself each tick
/// (components under 0.01 stop) and upward speed falls at 100/s.
const KNOCKDOWN_PUSH: f32 = 40.0;
const KNOCKDOWN_PUSH_BIG: f32 = 20.0;
const FLINCH_PUSH: f32 = 8.0;
const KNOCK_MAX: f32 = 40.0;
const KNOCK_DECAY: f32 = 0.8;
const KNOCK_STOP: f32 = 0.01;
const KNOCK_FALL: f32 = 100.0;
/// Monsters whose floor step (`+0x23C`) is above this are big: they take
/// half the knockdown push, hit heroes harder and dissolve when they die.
pub const BIG_STEP: f32 = 2.0;
/// The knockdown push of the golem (`0x1D`) and the acid blob (`0x15`).
const KNOCKDOWN_PUSH_GOLEM: f32 = 2.0;
const GOLEM: i32 = 0x1D;
const ACID_BLOB: i32 = 0x15;
const KNIGHT: i32 = 5;
const TREE: i32 = 0xB;

const HIT1: u8 = 0x1C;
const HIT2: u8 = 0x1D;

/// Turns the blows a monster took into its reaction: the action it plays
/// and the push added to its knockback. One it lives through flashes it.
fn react(m: &mut Monster, animator: &mut Animator) {
    let Some(action) = take_blows(m) else { return };
    if m.hit_points > 0.0 {
        m.flash.start();
    }
    let action = if m.model.has(action) { action } else if m.model.has(HIT1) { HIT1 } else { return };
    m.action = action;
    if let Some(a) = m.model.actions[action as usize] {
        animator.play(a);
    }
    debug!("monster {} reacts: {} with push {:?} ({:.1} hp left)", m.number, ACTION_NAMES[action as usize], m.knock, m.hit_points);
}

/// Adds the blows taken since the last tick to the knockback (capped) and
/// says which reaction they call for (none for less than a point).
fn take_blows(m: &mut Monster) -> Option<u8> {
    let (damage, kind, push) = std::mem::take(&mut m.pending);
    if damage < 1.0 {
        return None;
    }
    let (action, factor) = reaction(damage, kind, m.stats.step, m.enemy);
    m.knock = add(m.knock, scale(push, factor));
    let speed = (m.knock[0] * m.knock[0] + m.knock[1] * m.knock[1] + m.knock[2] * m.knock[2]).sqrt();
    if speed > KNOCK_MAX {
        m.knock = scale(m.knock, KNOCK_MAX / speed);
    }
    Some(action)
}

/// A killed monster's death begins: its die effect where it stands, and
/// its body onto its death texture (none: it's gone next).
fn start_death(
    entity: Entity,
    m: &Monster,
    textures: Option<&DeathTextures>,
    commands: &mut Commands,
    effects: &mut MessageWriter<EffectAt>,
) {
    let Some(d) = m.dying else { return };
    if !d.quiet
        && let Some(name) = deaths::die_effect(m.enemy, d.kind)
    {
        let scale = deaths::die_effect_scale(m.enemy, m.stats.step);
        let at = deaths::effect_origin(m.stats.step, m.centre(), Vec3::from(d.blow));
        effects.write(EffectAt { name, bank: None, at, facing: 0.0, scale });
    }
    let frames = d.set.and_then(|s| textures.and_then(|t| t.frames(s)));
    if let Some(frames) = frames {
        commands.entity(entity).try_insert(Dissolve::new(frames));
    }
    debug!("monster {} dies: {:?}", m.number, d.set);
}

/// A dying monster's tick (the game's monster state 8): the killing blow's
/// push, the slide on its knockback, DEATH — or its HIT2 knock-down, or
/// READY's animation, when the body has none — and the death texture's
/// counter. True once it's gone: when the counter runs out, at once with
/// no death texture, or when it falls off the level.
fn die_tick(m: &mut Monster, animator: &mut Animator, collision: &LevelCollision, dt: f32) -> bool {
    let Some(d) = m.dying.as_mut() else { return true };
    if d.set.is_none() {
        return true;
    }
    d.step += deaths::DEATH_STEP * FIELDS_PER_TICK / 2.0;
    if d.step >= deaths::DEATH_STEPS {
        return true;
    }
    take_blows(m);
    let knock = scale(m.knock, dt);
    settle_knock(m, dt);
    let moved = monster_move(collision, m, knock, MAX_DROP_PER_SECOND * dt);
    m.position = add(m.position, moved.delta);
    if m.action != DEATH {
        m.action = DEATH;
        let model = &m.model;
        let clip = model.actions[DEATH as usize].or(model.actions[HIT2 as usize]).or(model.actions[READY as usize]);
        if let Some(a) = clip
            && animator.action != a
        {
            animator.play(a);
        }
    }
    moved.fell
}

/// The reaction to `damage` of `kind` on a monster of type `enemy` with
/// this floor step: the action and how much of the blow's push it takes.
fn reaction(damage: f32, kind: u32, step: f32, enemy: i32) -> (u8, f32) {
    let knockdown = kind & KNOCKDOWN_KINDS != 0 || (damage > BIG_HIT_DAMAGE && kind & BIG_HIT_KIND != 0);
    if knockdown {
        let push = match enemy {
            GOLEM => KNOCKDOWN_PUSH_GOLEM,
            ACID_BLOB => 0.0,
            _ if step > BIG_STEP => KNOCKDOWN_PUSH_BIG,
            _ => KNOCKDOWN_PUSH,
        };
        (HIT2, push)
    } else if kind & STRONG_KIND != 0 {
        (HIT1, FLINCH_PUSH)
    } else {
        (HIT1, 0.0)
    }
}

/// Knockback decays each tick.
fn settle_knock(m: &mut Monster, dt: f32) {
    for (i, v) in m.knock.iter_mut().enumerate() {
        *v *= KNOCK_DECAY;
        if v.abs() < KNOCK_STOP {
            *v = 0.0;
        }
        if i == 1 && *v > 0.0 {
            *v = (*v - KNOCK_FALL * dt).max(0.0);
        }
    }
}

/// Stand-in height of a generator for the hit search.
const GENERATOR_HEIGHT: f32 = 4.0;

const READY: u8 = 0;
const START: u8 = 1;
const WALK: u8 = 3;
const RUN: u8 = 4;
const ATTACK1: u8 = 0xC;
const ATTACK2: u8 = 0xE;
const ATTACK3: u8 = 0x10;
const RUNATTACK1: u8 = 0x16;
const RUNATTACK2: u8 = 0x17;
const THROW1: u8 = 0x18;
const THROW2: u8 = 0x19;
const THROWF: u8 = 0x1A;
const ATTTOREADY: u8 = 0x1B;
const DEATH: u8 = 0x20;
const READYTOWALK: u8 = 9;

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
    /// What each throw adds to the throwing AIs' pause (1 unless placed
    /// with a rate).
    pub throw_rate: f32,
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
        strength: match new.tier {
            ..=1 => 1,
            2 | 3 => new.tier as i16,
            _ => 2,
        },
        hits: 0,
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
        frame_ai: new.ai,
        number: level.created,
        pending: (0.0, 0, [0.0; 3]),
        knock: [0.0; 3],
        model: model.clone(),
        throw_timer: level.random(10) as f32,
        throw_pause: 0.0,
        throw_carry: 0.0,
        throw_rate: new.throw_rate,
        retreat: false,
        suicide: Suicide::default(),
        dying: None,
        flash: Flash::default(),
        // A placed Death lets one contact pass before it drains.
        death: DeathState { delay: u8::from(new.placed && new.enemy == DEATH_TYPE), ..DeathState::default() },
    };
    // Hittable: its radius, and (a stand-in for the game's height test)
    // twice its centre height.
    let target = Targetable::new(TargetKind::Monster, instance.radius, 2.0 * stats.center_height).with_size(stats.step());
    commands.entity(root).insert((monster, target, LevelEntity));
    if new.enemy == DEATH_TYPE {
        commands.entity(root).insert(DeathMonster { experience: new.tier == 2 });
    }
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
    mut deaths: ResMut<DeathTextures>,
    texanims: Option<ResMut<LevelTexAnims>>,
) {
    let Some(ground) = ground else { return };
    let entry = tunings.and_then(|t| t.0.get(&population.level.to_ascii_lowercase()).cloned());
    let (raw, enemies, sounds) = match entry {
        Some((t, e, s)) => (Some(t), e, s),
        None => (None, LevelEnemies::default(), MonsterSoundTable::default()),
    };
    let tuning = level_tuning(raw);
    let scales = EnemyScales {
        hit_points: tuning.monster_hit_points,
        speed: tuning.monster_speed,
        awareness: tuning.monster_awareness,
        damage: tuning.monster_damage,
    };
    let realm = crate::quest::level_of(&population.level).map_or(0, |(r, _)| r);
    let (gens, placed) = generators::from_population(&population.population, &ground.0, &tuning, &enemies, realm, population.players);

    let mut wanted: Vec<(i32, i32)> =
        gens.iter().flat_map(|g| g.makes()).chain(placed.iter().map(|p| (p.enemy, p.tier))).collect();
    // Monsters hiding in containers (Deaths in barrels) come out at tier 1.
    let pop = &population.population;
    for p in &pop.placements {
        let ty = pop.resolved_type(p);
        if let gdl_formats::population::PlacementParams::Container { contents: Some(c), .. } = p.params(ty.class)
            && c < pop.item_types.len()
            && let Some(id) = pop.resolve(c).enemy()
        {
            wanted.push((id, 1));
        }
    }
    wanted.sort();
    wanted.dedup();
    let mut folders = FolderCache::default();
    let mut models = HashMap::new();
    let mut animated = Vec::new();
    for &(id, tier) in &wanted {
        let model = match load_enemy(&mut game.install, &mut folders, &enemies, id, tier) {
            Ok((data, folder)) => {
                let (model, anims) = monster_model(&data, &folder.texmods, &mut meshes, &mut materials, &mut images);
                animated.extend(anims);
                Some(Arc::new(model))
            }
            Err(why) => {
                warn!("monster {id} tier {tier}: {why}");
                None
            }
        };
        models.insert((id, tier), model);
    }
    if let Some(mut texanims) = texanims {
        info!("{} texture animations on the level's monsters", animated.len());
        texanims.extend(animated);
    }
    // Death textures: WEAPONS' (once), and the level's DEATHALT from the
    // trees' folders, else the knights'.
    deaths.load_shared(&mut game.install, &mut images);
    let alt: Vec<Arc<MonsterFolder>> = [TREE, KNIGHT]
        .iter()
        .filter(|&&e| wanted.iter().any(|w| w.0 == e))
        .flat_map(|&e| enemies.folders(e))
        .filter_map(|f| folders.get(&mut game.install, &f))
        .collect();
    deaths.set_alt(alt.iter().map(|f| (&f.model, f.textures.as_slice(), f.texmods.as_slice())), &mut images);
    // The runners' SUICIDEEXP: from the first of the level's monster
    // folders that has one, the special variants' excepted.
    let runners = gens.iter().any(|g| g.ai == SUICIDE) || placed.iter().any(|p| p.ai == SUICIDE);
    let suicide_folder = if runners {
        enemies.loaded.iter().filter(|&&(_, subtype)| subtype != 4).filter_map(|&(id, subtype)| enemy::folder(id, subtype)).find(
            |f| folders.get(&mut game.install, f).is_some_and(|files| files.anim.atrees.iter().any(|a| a.name == SUICIDE_EFFECT)),
        )
    } else {
        None
    };

    info!(
        "monsters: {} generators, {} placed, {} slots, types {:?} from {:?} (tuning {:?})",
        gens.len(),
        placed.len(),
        tuning.monster_slots,
        wanted,
        enemies.loaded,
        tuning
    );
    for p in &placed {
        debug!("placed enemy {} tier {} AI {:#x} at {:.1?}", p.enemy, p.tier, p.ai, p.position);
    }
    for g in gens {
        // Hittable where it stands. Its height is a stand-in (generator
        // models are about this tall; at most 3.5 counts as low).
        let target = Targetable::new(TargetKind::Generator, g.screen_radius / 4.0, GENERATOR_HEIGHT);
        commands.spawn((Transform::from_translation(Vec3::from(g.position)), target, g, LevelEntity));
    }
    commands.insert_resource(generators::PlacedMonsters(placed));
    commands.insert_resource(MonsterLevel {
        slots: tuning.monster_slots.max(1) as usize,
        experience: (tuning.experience_level, tuning.experience_scale),
        scales,
        sounds,
        realm: population.level.strip_prefix("level").and_then(|l| l.chars().next()).unwrap_or('?').to_ascii_uppercase(),
        boss: tuning.boss_enemy,
        suicide_folder,
        tuning,
        enemies,
        models,
        tick: 0,
        rng: 0x1234_5678,
        created: 0,
        leader: None,
        death_sucking: false,
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
        throw_timing: 1.0,
        missile_speed: 1.0,
        missile_spread: 1.0,
        generator_hit_points: 1.0,
        generator_rate: 1.0,
        generator_max: 1.0,
        experience_level: 0.0,
        experience_scale: 1.0,
        boss_enemy: -1,
        tile_time: 1.0,
        hazard_damage: 1.0,
    });
    for v in [
        &mut t.monster_hit_points,
        &mut t.monster_speed,
        &mut t.monster_awareness,
        &mut t.monster_damage,
        &mut t.throw_timing,
        &mut t.missile_speed,
        &mut t.missile_spread,
        &mut t.generator_hit_points,
        &mut t.generator_rate,
        &mut t.generator_max,
        &mut t.experience_scale,
        &mut t.tile_time,
        &mut t.hazard_damage,
    ] {
        if *v <= 0.0 {
            *v = 1.0;
        }
    }
    t
}

/// A monster folder's model, texture bytes, atrees and texture modifiers.
struct MonsterFolder {
    model: ModelFile,
    textures: Vec<u8>,
    anim: AnimFile,
    texmods: Vec<TexMod>,
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
                let bytes = install.read(&format!("{dir}/ANIM.PS2")).ok()?;
                let anim = AnimFile::parse(&bytes).ok()?;
                let texmods = TexMod::parse_all(&bytes).unwrap_or_default();
                Some(Arc::new(MonsterFolder { model, textures, anim, texmods }))
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
) -> Result<(CharacterData, Arc<MonsterFolder>), String> {
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
            let data = CharacterData {
                name: format!("{folder}/{atree}"),
                class: String::new(),
                colour: String::new(),
                skeleton: tree.clone(),
                clips: Arc::new(tree.clone()),
                model: files.model.clone(),
                textures: files.textures.clone(),
            };
            return Ok((data, files));
        }
    }
    Err(format!("no atree for {name} tier {tier}"))
}

/// A monster's model, with its folder's running texture animations.
fn monster_model(
    data: &CharacterData,
    texmods: &[TexMod],
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> (MonsterModel, Vec<TexAnim>) {
    let mut actions = [None; ACTION_NAMES.len()];
    let mut loops = [false; ACTION_NAMES.len()];
    for (i, name) in ACTION_NAMES.iter().enumerate() {
        if let Some(a) = data.clips.actions.iter().position(|a| a.name == *name) {
            actions[i] = Some(a);
            loops[i] = data.clips.actions[a].loops();
        }
    }
    let (model, anims) = CharacterModel::build_animated(data, texmods, meshes, materials, images);
    (MonsterModel { model, actions, loops }, anims)
}

/// What the game's cameras see (`PlayCamera::game_views`: the play
/// camera's; online each hero's own): each view from eye to target with the
/// game's 60° × 45° (4:3) field of view — whatever the window's shape or
/// the free camera. Before the play camera exists nothing is on screen.
pub struct Views(Vec<Frustum>);

pub fn game_view(camera: Option<&PlayCamera>) -> Views {
    let Some(camera) = camera else { return Views(Vec::new()) };
    let fov = 2.0 * (0.75 * 30f32.to_radians().dtan()).datan();
    let frustum = |(eye, target): ([f32; 3], [f32; 3])| {
        let (eye, target) = (Vec3::from(eye), Vec3::from(target));
        if eye.distance_squared(target) < 1e-6 {
            return None;
        }
        let clip = Mat4::perspective_rh(fov, 4.0 / 3.0, 0.5, 2000.0) * Mat4::look_at_rh(eye, target, Vec3::Y);
        Some(Frustum::from_clip_from_world(&clip))
    };
    Views(camera.game_views().into_iter().filter_map(frustum).collect())
}

/// The on-screen test the game makes against its camera: a sphere around a
/// point (in any of the views).
pub fn on_screen(view: &Views, at: [f32; 3], radius: f32) -> bool {
    view.0.iter().any(|f| f.intersects_sphere(&Sphere { center: Vec3::from(at).into(), radius }, true))
}

/// A player as the monsters see it.
#[derive(Clone, Copy)]
pub struct Target {
    pub entity: Entity,
    pub feet: [f32; 3],
    /// Invisible: not picked.
    pub hidden: bool,
    /// Wears the halo: never Death's target, and Death runs from it.
    pub halo: bool,
}

/// Monster positions at the start of the tick, for bump tests.
#[derive(Clone, Copy)]
pub struct Body {
    pub entity: Entity,
    pub feet: [f32; 3],
    pub radius: f32,
    pub step: f32,
}

/// What Death's part of the monster tick sends and changes.
type DeathWriters<'w, 's> = (
    MessageWriter<'w, DeathDrain>,
    MessageWriter<'w, LoopSoundAt>,
    MessageWriter<'w, ShowHint>,
    Query<'w, 's, &'static mut Fade>,
    MessageWriter<'w, EffectOn>,
);

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
    mut shots: MessageWriter<MonsterShot>,
    death_textures: Option<Res<DeathTextures>>,
    mut effects: MessageWriter<EffectAt>,
    (mut explosions, mut sounds): (MessageWriter<ExplosionAt>, MessageWriter<PlaySoundAt>),
    (colours, mut tags, stop, enemies): (Res<FlashColours>, Query<&mut MeshTag>, Res<TimeStop>, Res<EnemyScale>),
    (mut drains, mut loops, mut hints, mut fades, mut riding): DeathWriters,
) {
    let (Some(mut level), Some(ground)) = (level, ground) else { return };
    level.tick = level.tick.wrapping_add(1);
    // Where the Death draining a hero this tick is (its drain's sound
    // follows it).
    let mut sucking: Option<Vec3> = None;
    let collision = &ground.0;
    let dt = time.delta_secs();
    let view = game_view(camera.as_deref());
    let frustum = &view;
    let targets: Vec<Target> = players
        .iter()
        .map(|(e, p)| Target {
            entity: e,
            feet: p.mover.position,
            hidden: p.special_bits & power::INVISIBLE != 0,
            halo: p.armour_bits & HALO != 0,
        })
        .collect();
    // The dying don't get in anyone's way.
    let bodies: Vec<Body> = monsters
        .iter()
        .filter(|(_, m, _)| m.dying.is_none())
        .map(|(e, m, _)| Body { entity: e, feet: m.position, radius: m.stats.radius, step: m.stats.step })
        .collect();
    // The leader: the first suicide runner charging near the screen takes
    // over; the last one stays until it's gone. Its place and its
    // player's distance, for the monsters that run from it.
    let charging = monsters
        .iter()
        .filter(|(_, m, _)| m.dying.is_none() && m.ai == SUICIDE && m.near_screen && (m.action == RUN || m.request == RUN))
        .min_by_key(|(_, m, _)| m.number)
        .map(|(e, _, _)| e);
    if charging.is_some() {
        level.leader = charging;
    }
    let leader = level.leader.and_then(|e| {
        let (_, m, _) = monsters.get(e).ok().filter(|(_, m, _)| m.dying.is_none())?;
        Some((e, m.position, m.target_distance))
    });
    if leader.is_none() {
        level.leader = None;
    }

    for (entity, mut m, mut animator) in &mut monsters {
        let m = &mut *m;
        m.previous = (m.position, m.facing);
        if let Some(d) = m.dying {
            // The dying play on whatever the time.
            if animator.hold {
                animator.hold = false;
            }
            if !d.started {
                if let Some(d) = m.dying.as_mut() {
                    d.started = true;
                }
                // The death texture takes the flash's place.
                if m.flash.stop() {
                    flash::tag_body(&animator, |_| true, 0, &mut tags, &mut commands);
                }
                start_death(entity, m, death_textures.as_deref(), &mut commands, &mut effects);
                // However it died, a suicide runner goes off.
                if m.ai == SUICIDE {
                    explosions.write(ExplosionAt {
                        owner: entity,
                        at: m.centre(),
                        damage: SUICIDE_DAMAGE * level.scales.damage,
                        poison: POISON_REALMS.contains(&level.realm),
                        folder: level.suicide_folder.clone(),
                        by: Exploder::Monster,
                    });
                }
            }
            if m.enemy == DEATH_TYPE {
                if leave_tick(m, entity, &mut fades, &mut hints, &mut commands, dt) {
                    commands.entity(entity).try_despawn();
                }
                continue;
            }
            if die_tick(m, &mut animator, collision, dt) {
                commands.entity(entity).try_despawn();
            }
            continue;
        }
        let r = m.stats.radius;
        m.near_screen = on_screen(frustum, m.position, 2.0 * r + 15.0);
        // Time stopped, it stands frozen, its clip too: no target, move or
        // blow; only the blows it takes are still turned into reactions.
        if animator.hold != stop.0 {
            animator.hold = stop.0;
        }
        if stop.0 {
            react(m, &mut animator);
            if m.flash.step() {
                let tag = if m.flash.on() { colours.body().unwrap_or(0) } else { 0 };
                flash::tag_body(&animator, |_| true, tag, &mut tags, &mut commands);
            }
            continue;
        }
        select_target(m, &targets, level.tick);

        // Blows taken: flinch or knockdown. While the reaction plays the
        // monster only slides on its knockback.
        react(m, &mut animator);
        if m.flash.step() {
            let tag = if m.flash.on() { colours.body().unwrap_or(0) } else { 0 };
            flash::tag_body(&animator, |_| true, tag, &mut tags, &mut commands);
        }
        let knock = scale(m.knock, dt);
        settle_knock(m, dt);
        if matches!(m.action, HIT1 | HIT2) {
            if animator.finished() {
                m.action = READY;
            } else {
                let moved = monster_move(collision, m, knock, MAX_DROP_PER_SECOND * dt);
                m.position = add(m.position, moved.delta);
                if moved.fell {
                    despawn_monster(&mut commands, entity, m, &mut generators);
                }
                continue;
            }
        }

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
        // Death's atrees have only READY and START: every action plays
        // READY's animation, and it moves as its AI says whatever it's
        // playing — pressing into a hero it drains, every tick.
        let moving = |m: &Monster| {
            m.enemy == DEATH_TYPE || (matches!(m.action, WALK | RUN | RUNATTACK1 | RUNATTACK2) && m.model.has(m.action))
        };
        // A suicide runner's bump wait as its AI finds it (the last move's).
        let bumping_before = m.avoid_timer;
        // This tick's AI: its own, unless it runs from a charging runner
        // or, with no aware target, walks unaware (the game switches for the
        // tick and puts the monster's own AI back after it).
        m.frame_ai = m.ai;
        let flee_from = leader.and_then(|(runner, at, player)| {
            let d = [at[0] - m.position[0], at[1] - m.position[1], at[2] - m.position[2]];
            let near = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < FLEE_REACH_SQ;
            let flees = FLEES_RUNNER.contains(&m.ai) && runner != entity && !m.placed && m.avoid_timer < 1.0;
            (flees && near && player <= m.stats.awareness).then_some(at)
        });
        if flee_from.is_some() {
            m.frame_ai = FLEE;
        } else if GOES_UNAWARE.contains(&m.ai) && !(m.aware && target.is_some()) {
            m.frame_ai = unaware_ai(m.number);
        }
        if m.frame_ai != m.ai || matches!(m.ai, 5 | 6) {
            trace!("monster {} AI {:#x} ticks as {:#x} at {:?}, heading {:.2}", m.number, m.ai, m.frame_ai, m.position, m.heading);
        }
        let turn_to = if let Some(at) = flee_from {
            // Straight away from the runner, at a run.
            let h = locomotion::wrap(heading_to(m.position, at) + PI);
            m.heading = h;
            m.request = RUN;
            if moving(m) {
                let s = FLEE_SPEED * m.stats.speed_per_tick;
                velocity = [h.dsin() * s, 0.0, h.dcos() * s];
            }
            Some(h)
        } else if let Some(from) = (m.enemy == DEATH_TYPE && !(m.aware && target.is_some()))
            .then(|| m.death.halo_hero.and_then(|h| targets.iter().find(|t| t.entity == h)))
            .flatten()
        {
            // Death runs from a haloed hero, nudged aside while blocked.
            let nudge = if m.blocked != Block::None && m.death.nudge < DEATH_NUDGES.len() {
                m.death.nudge += 1;
                DEATH_NUDGES[m.death.nudge - 1].to_radians()
            } else {
                if m.blocked == Block::None {
                    m.death.nudge = 0;
                }
                0.0
            };
            let h = locomotion::wrap(heading_to(m.position, from.feet) + PI + nudge);
            m.heading = h;
            m.request = WALK;
            if moving(m) {
                let s = DEATH_RUN * m.stats.speed_per_tick;
                velocity = [h.dsin() * s, 0.0, h.dcos() * s];
            }
            Some(h)
        } else if m.ai == SUICIDE {
            let (heading, run, yell) = suicide_ai(m, target);
            if yell {
                // At its `+0x44` point: its centre here, as its effects.
                sounds.write(PlaySoundAt::faded(SUICIDE_YELL, m.centre(), RUNNER_VOLUME));
            }
            if m.frame_ai != SUICIDE {
                // It lost its player: an unaware walk this tick.
                let h = unaware(m, collision, &bodies, entity);
                m.request = WALK;
                if moving(m) {
                    let s = m.stats.speed_per_tick;
                    velocity = [h.dsin() * s, 0.0, h.dcos() * s];
                }
                Some(h)
            } else {
                if run {
                    m.request = RUN;
                    if moving(m) {
                        let s = SUICIDE_SPEED * m.stats.speed_per_tick;
                        velocity = [heading.dsin() * s, 0.0, heading.dcos() * s];
                    }
                }
                Some(heading)
            }
        } else if projectiles::throws(m.ai) {
            // The throwers face the player and throw; the kiting ones back
            // off while they throw.
            let (face, away) = throw_ai(m, target);
            if let Some(h) = away
                && moving(m)
            {
                let s = RETREAT_SPEED * m.stats.speed_per_tick;
                velocity = [h.dsin() * s, 0.0, h.dcos() * s];
            }
            Some(face)
        } else {
            // The wanderers chase a player within 8 for the tick (AI 0),
            // or walk unaware if they haven't noticed it.
            if matches!(m.frame_ai, 2 | 4) && direct.is_some() && m.target_distance <= WANDER_CHASE {
                m.frame_ai = if m.aware { 0 } else { unaware_ai(m.number) };
            }
            let turn_to = match (m.frame_ai, direct) {
                (5 | 6, _) => Some(unaware(m, collision, &bodies, entity)),
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
                if moving(m) {
                    let s = m.stats.speed_per_tick;
                    velocity = [h.dsin() * s, 0.0, h.dcos() * s];
                }
            }
            turn_to
        };
        if m.freeze > 0.0 {
            m.freeze -= FIELDS_PER_TICK;
            velocity = [0.0; 3];
        }
        if let Some(h) = turn_to {
            let rate = m.stats.turn_per_tick * if m.action == RUN { 3.0 } else { 1.0 };
            m.facing = turn_toward(m.facing, h, rate);
        }

        // Walls and floor, then players and other monsters in the way.
        let velocity = add(velocity, knock);
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
        if !bumped_player.is_some_and(|p| m.enemy == DEATH_TYPE && !p.halo) {
            // Touching no hero, Death's drain effect goes.
            m.death.drain_effect = false;
        }
        if let Some(p) = bumped_player {
            // It stops and swings at the player it walked into (a suicide
            // runner blows up instead, below; Death drains it).
            m.blocked = Block::Player;
            if m.enemy == DEATH_TYPE {
                if death_touch(m, entity, &p, &mut drains, &mut riding) {
                    // It pays with its own hit points; full, it leaves
                    // (laughing), else its drain's sound goes on at it —
                    // each Death draining pans the one loop, the last's
                    // pan holding.
                    m.hit_points -= m.stats.damage;
                    if m.hit_points >= 0.0 {
                        sucking = Some(Vec3::from(m.position));
                    } else {
                        m.hit_points = 0.0;
                        m.death.left = true;
                        sounds.write(PlaySoundAt::panned(DEATH_LAUGH, Vec3::from(m.position), DEATH_LAUGH_VOLUME));
                        if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
                            g.alive = g.alive.saturating_sub(1);
                        }
                        m.die(0, m.position);
                        commands.entity(entity).try_remove::<Targetable>();
                        info!("Death has drunk its fill and leaves");
                    }
                }
            } else if m.ai != SUICIDE {
                m.strike = Some(p.entity);
                m.request = if m.attacks & 7 == 7 { ATTACK3 } else { ATTACK1 };
            }
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
        // A suicide runner that reached a player, or ran out of run, blows
        // itself up; one that has just started bumping into things notes
        // where it was heading.
        if m.ai == SUICIDE {
            if m.suicide.running >= SUICIDE_RUN || bumped_player.is_some() {
                blow_up(&mut commands, entity, m, &mut generators);
                continue;
            }
            if bumping_before < 1.0 && m.avoid_timer > 0.0 {
                m.suicide.saved = m.heading;
                m.suicide.step = 0;
            }
        }

        // A throw pause refuses attacks and throws.
        if m.throw_pause > 0.0 {
            m.throw_pause -= dt;
            if matches!(m.request, ATTACK1..=0x15 | THROW1..=THROWF) {
                m.request = READY;
            }
        }
        let was = m.action;
        let event = animate(m, &mut animator, level.tuning.throw_timing);
        if projectiles::throws(m.ai) && was != m.action {
            debug!(
                "thrower {entity:?}: {} -> {} (asked {}, pause {:.2}, distance {:.1})",
                ACTION_NAMES[was as usize],
                ACTION_NAMES[m.action as usize],
                ACTION_NAMES[m.request as usize],
                m.throw_pause,
                m.target_distance
            );
        }
        match event {
            // The fireball AIs let fly on their attack blows instead of
            // hitting.
            Some(Event::Blow(_)) if matches!(m.ai, 0x1C | 0x1D | 0x1F) => {
                let centre = enemy::enemy_stats(m.enemy).map_or(0.0, |s| s.center_height);
                let from = Vec3::from(m.position) + Vec3::Y * centre;
                let at = match target {
                    Some(t) => Vec3::from(t.feet) + Vec3::Y * projectiles::PLAYER_CENTRE,
                    None => from + 20.0 * Vec3::new(m.facing.dsin(), 0.0, m.facing.dcos()),
                };
                let random = level.random(1000) as f32 / 1000.0;
                shots.write(MonsterShot { monster: entity, enemy: m.enemy, ai: m.ai, from, at, facing: m.facing, random });
                m.attacks = m.attacks.wrapping_add(1);
            }
            Some(Event::Blow(hit)) => {
                if let Some(player) = m.strike {
                    // Shrunk, half its damage (and never the strong
                    // third's half again).
                    let damage = if enemies.shrunk() {
                        m.stats.damage * SHRUNK_BLOW
                    } else {
                        m.stats.damage * if hit { STRONG_BLOW } else { 1.0 }
                    };
                    hits.write(MonsterHit { monster: entity, player, damage, strong: hit });
                }
                m.attacks = m.attacks.wrapping_add(1);
            }
            Some(Event::Throw) => {
                // At the target player's centre, or 20 units ahead.
                let centre = enemy::enemy_stats(m.enemy).map_or(0.0, |s| s.center_height);
                let from = Vec3::from(m.position) + Vec3::Y * centre;
                let at = match target {
                    Some(t) => Vec3::from(t.feet) + Vec3::Y * projectiles::PLAYER_CENTRE,
                    None => from + 20.0 * Vec3::new(m.facing.dsin(), 0.0, m.facing.dcos()),
                };
                let random = level.random(1000) as f32 / 1000.0;
                shots.write(MonsterShot { monster: entity, enemy: m.enemy, ai: m.ai, from, at, facing: m.facing, random });
            }
            None => {}
        }
    }
    // The drain's sound stops on a tick no Death drained.
    match sucking {
        Some(at) => {
            loops.write(LoopSoundAt::at(DEATH_SUCK_LOOP, DEATH_SUCK, at, CALL_VOLUME));
        }
        None if level.death_sucking => {
            loops.write(LoopSoundAt::stop(DEATH_SUCK_LOOP));
        }
        None => {}
    }
    level.death_sucking = sucking.is_some();
}

/// Death touches a hero (the game's bump routine, Death's branch): nothing
/// while it's frozen or the hero wears the halo; otherwise, after its
/// contact delay, it drains the hero every 3 video fields — health, or
/// experience for a tier-2 Death — its drain effect on it. True when a
/// drain landed.
fn death_touch(
    m: &mut Monster,
    entity: Entity,
    hero: &Target,
    drains: &mut MessageWriter<DeathDrain>,
    riding: &mut MessageWriter<EffectOn>,
) -> bool {
    if m.freeze >= 1.0 || hero.halo {
        return false;
    }
    if m.death.delay > 0 {
        m.death.delay -= 1;
        return false;
    }
    m.request = ATTACK1;
    m.death.timer -= FIELDS_PER_TICK;
    if m.death.timer > 0.0 {
        return false;
    }
    m.death.timer += DEATH_DRAIN_FIELDS;
    m.death.drained = Some(hero.entity);
    let experience = m.strength == 2;
    drains.write(DeathDrain { death: entity, hero: hero.entity, amount: m.stats.damage, experience });
    if !m.death.drain_effect {
        m.death.drain_effect = true;
        let name = if experience { DEATH_EXP } else { DEATH_ARC };
        riding.write(EffectOn { name, bank: Some(DEATH_BANK), on: entity, scale: 1.0 });
    }
    true
}

/// A Death leaving or killed (the game's dying frame for it): no death
/// texture or effect — it rises 10 units a second and grows see-through
/// by 4 of 255 a video field; once gone, a Death that left full tells the
/// hero it drained. True when it's gone.
fn leave_tick(
    m: &mut Monster,
    entity: Entity,
    fades: &mut Query<&mut Fade>,
    hints: &mut MessageWriter<ShowHint>,
    commands: &mut Commands,
    dt: f32,
) -> bool {
    m.position[1] += DEATH_RISE * dt;
    m.death.fade = (m.death.fade + DEATH_FADE * FIELDS_PER_TICK).min(255.0);
    let amount = m.death.fade / 255.0;
    match fades.get_mut(entity) {
        Ok(mut f) => f.amount = amount,
        Err(_) => {
            commands.entity(entity).try_insert(Fade::new(amount));
        }
    }
    if m.death.fade < 255.0 {
        return false;
    }
    if m.death.left && m.death.drained.is_some() {
        hints.write(ShowHint::to(0, if m.strength == 2 { Hint::DeathLeftAfterExperience } else { Hint::DeathLeftAfterHealth }));
    }
    true
}

/// Backing away, a kiting thrower moves at this fraction of its speed.
const RETREAT_SPEED: f32 = 0.8;

/// The throwing AIs (`docs/projectiles.md`): 0x11 and 0x17 turn to face
/// their player and throw whenever it's near the screen, within their
/// awareness and within 10 units up or down; 0x10 and 0x1A do the same but
/// back away (running, still facing it, still throwing) once it comes
/// within 0.6 of their awareness, until it's beyond 0.8. The first throw
/// waits a random 0–9 fields. Returns the heading to turn to and, when
/// backing away, the heading to move along.
fn throw_ai(m: &mut Monster, target: Option<Target>) -> (f32, Option<f32>) {
    let Some(t) = target else { return (m.heading, None) };
    m.heading = heading_to(m.position, t.feet);
    let kites = matches!(m.ai, 0x10 | 0x1A);
    let in_band = (-10.0..=10.0).contains(&(m.position[1] - t.feet[1]));
    let in_range = kites || m.target_distance <= m.stats.awareness;
    let mut away = None;
    if m.near_screen && in_band && in_range {
        if kites {
            let aware = m.stats.awareness;
            if !m.retreat {
                m.retreat = m.target_distance <= 0.6 * aware;
            } else if m.target_distance > 0.8 * aware {
                m.retreat = false;
            }
        }
        if m.throw_timer < 1.0 {
            if m.retreat {
                m.request = RUNATTACK1;
                away = Some(locomotion::wrap(m.heading + PI));
            } else {
                m.request = THROW1;
            }
        } else {
            m.throw_timer -= FIELDS_PER_TICK;
        }
    }
    (m.heading, away)
}

/// Removes a monster and frees its generator's slot.
/// A suicide runner (AI 0x12): it faces the player it's after; once one is
/// within its awareness it waits 60 fields, gets up (READYTOWALK) and, as
/// it starts running, yells and runs at the player at 1.5 × its speed.
/// While it keeps running into walls or monsters it tries heading offsets
/// of ±5° … ±40° in turn. Losing its player makes it unaware (AI 5/6).
/// Returns the heading to turn to, whether it runs along it, and whether
/// it has just started running (the yell).
fn suicide_ai(m: &mut Monster, target: Option<Target>) -> (f32, bool, bool) {
    if let Some(t) = target {
        m.heading = heading_to(m.position, t.feet);
    }
    let mut yell = false;
    match m.suicide.stage {
        0 => {
            if target.is_some() && m.target_distance <= m.stats.awareness {
                m.suicide = Suicide { stage: 1, wait: SUICIDE_WAIT, ..Suicide::default() };
                debug!("suicide runner {} sees a player {:.1} away", m.number, m.target_distance);
            }
            return (m.heading, false, false);
        }
        1 => {
            if m.action != RUN {
                if target.is_some() {
                    m.suicide.wait -= FIELDS_PER_TICK;
                    if m.suicide.wait < 1.0 {
                        m.request = READYTOWALK;
                    }
                }
                return (m.heading, false, false);
            }
            m.suicide.stage = 2;
            yell = true;
            debug!("suicide runner {} runs", m.number);
        }
        _ => {}
    }
    m.suicide.running += FIELDS_PER_TICK;
    if target.is_none() || !m.aware {
        m.frame_ai = unaware_ai(m.number);
        return (m.heading, false, yell);
    }
    // Held up by another monster it counts the wait down itself; while
    // held up it tries the next offset.
    if m.blocked == Block::Actor && m.avoid_timer > 0.0 {
        m.avoid_timer -= FIELDS_PER_TICK;
    }
    if m.avoid_timer > 0.0 && m.suicide.step < SUICIDE_OFFSETS.len() {
        m.heading = locomotion::wrap(m.heading + SUICIDE_OFFSETS[m.suicide.step]);
        m.suicide.step += 1;
        m.avoid_timer = 0.0;
    }
    let run = m.avoid_timer < 1.0 || locomotion::wrap(m.heading - m.suicide.saved).abs() >= SUICIDE_TURN;
    if run {
        m.avoid_timer = 0.0;
    }
    (m.heading, run, yell)
}

/// A suicide runner blows itself up: a fire blow of more than it has kills
/// it with no die effect (nobody killed it), and its death sets off the
/// explosion.
fn blow_up(commands: &mut Commands, entity: Entity, m: &mut Monster, generators: &mut Query<&mut Generator>) {
    m.hit_points = 0.0;
    if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
        g.alive = g.alive.saturating_sub(1);
    }
    m.die(SUICIDE_KIND, m.centre().to_array());
    if let Some(d) = m.dying.as_mut() {
        d.quiet = true;
    }
    commands.entity(entity).try_remove::<Targetable>();
    debug!("suicide runner {} blows up after {:.0} fields", m.number, m.suicide.running);
}

pub fn despawn_monster(commands: &mut Commands, entity: Entity, m: &Monster, generators: &mut Query<&mut Generator>) {
    if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
        g.alive = g.alive.saturating_sub(1);
    }
    commands.entity(entity).try_despawn();
}

/// The game re-picks a monster's target on one tick in eight (staggered
/// by monster), or whenever it has none: the nearest player within its
/// awareness range, not an invisible one. Its distance to the target is
/// kept every tick.
fn select_target(m: &mut Monster, targets: &[Target], tick: u32) {
    let keep = m.target.is_some() && tick & 7 != m.number & 7;
    let current = m.target.and_then(|t| targets.iter().find(|p| p.entity == t));
    if keep && let Some(p) = current {
        m.target_distance = distance(m.position, p.feet);
        return;
    }
    m.target = None;
    m.target_distance = f32::MAX;
    // Death never picks a hero with the halo; it notes the nearest one.
    let death = m.enemy == DEATH_TYPE;
    m.death.halo_hero = None;
    let mut halo_distance = f32::MAX;
    for p in targets.iter().filter(|p| !p.hidden) {
        let d = distance(m.position, p.feet);
        if death && p.halo {
            if d <= m.stats.awareness && d < halo_distance {
                m.death.halo_hero = Some(p.entity);
                halo_distance = d;
            }
            continue;
        }
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
    let ahead = [m.position[0] + h.dsin() * s, m.position[1], m.position[2] + h.dcos() * s];
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
/// the 20-field wait is over, swapping the turn after four turns. Walking
/// into a player turns them at it. (A player within 8 units makes them
/// chase for the tick: `tick_monsters`.)
fn wander(m: &mut Monster, direct: Option<f32>) -> f32 {
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

/// The unaware AI a monster hands a tick over to: 5 or 6 by its slot.
fn unaware_ai(number: u32) -> i16 {
    UNAWARE + (number & 1) as i16
}

/// An unaware monster (`docs/monsters.md`, "Unaware monsters"): it walks
/// its heading, turning a quarter — AI 5 one way, 6 the other — when a
/// wall is within its radius + 0.5 ahead or its next step is blocked, and
/// again when the 20 fields it then waits run out.
fn unaware(m: &mut Monster, collision: &LevelCollision, bodies: &[Body], me: Entity) -> f32 {
    let turn = if m.frame_ai == UNAWARE { -UNAWARE_TURN } else { UNAWARE_TURN };
    if m.avoid_timer > 0.0 {
        m.avoid_timer -= FIELDS_PER_TICK;
        if m.avoid_timer < 1.0 {
            m.heading = locomotion::wrap(m.heading + turn);
            m.wander_turns = (m.wander_turns + 1) % 4;
        }
    }
    let (sin, cos) = m.heading.dsin_cos();
    let r = m.stats.radius;
    let eye = [m.position[0], m.position[1] + 0.1 + r, m.position[2]];
    let reach = r + UNAWARE_LOOK;
    let wall = collision.wall(eye, [eye[0] + reach * sin, eye[1], eye[2] + reach * cos], 0.1).is_some();
    let step = m.stats.speed_per_tick;
    let ahead = [m.position[0] + step * sin, m.position[1], m.position[2] + step * cos];
    if wall || look_ahead_blocked(m, ahead, collision, bodies, me) {
        m.heading = locomotion::wrap(m.heading + turn);
        if m.avoid_timer < 1.0 {
            m.avoid_timer = UNAWARE_HOLD;
        }
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
        let (x, z) = (at[0] + a.dsin() - player[0], at[2] + a.dcos() - player[2]);
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
    let Some((hold, give_up, limit)) = avoid_rule(m.frame_ai, other.is_none()) else {
        if m.avoid_timer < 1.0 {
            m.avoid_timer = if m.frame_ai == 0 && other.is_some() { 60.0 } else { 20.0 };
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

/// What an action handing over to the next one does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    /// An attack lands (`true`: the stronger third one).
    Blow(bool),
    /// A throw lets go of its missile.
    Throw,
}

/// Plays the actions the monster asks for, the way the game's action
/// chooser does for regular monsters: START, attacks and throws play out
/// before anything else; an attack finishing is when it lands, and its
/// recovery (`…R`) follows; walking and standing switch at once. A throw
/// goes THROW1 → THROWF (again: THROWF → THROW2 → THROWF) and lets go as
/// THROWF starts; the running throw RUNATTACK1 → RUNATTACK2 lets go as its
/// second half starts; after the last THROWF comes ATTTOREADY. Missing
/// actions fall back as the game's do (RUN ↔ WALK, any attack → ATTACK1,
/// THROW2 → THROW1, ATTTOREADY → READY, else READY's animation). Each
/// throw action started adds to the throw pause (`timing`: the level's
/// scale).
fn animate(m: &mut Monster, animator: &mut Animator, timing: f32) -> Option<Event> {
    let model = m.model.clone();
    let finished = |m: &Monster, animator: &Animator| -> bool {
        let Some(a) = model.actions[m.action as usize] else { return true };
        if model.loops[m.action as usize] || animator.action != a {
            return animator.action != a;
        }
        animator.finished()
    };
    // A body without the throw asked for doesn't throw (stand-in: the game
    // would play READY's animation in its place).
    let request = match m.request {
        THROW1 if !model.has(THROW1) && !model.has(THROW2) => READY,
        RUNATTACK1 if !model.has(RUNATTACK1) && !model.has(RUNATTACK2) => READY,
        r => r,
    };
    let mut event = None;
    let next = match m.action {
        START => finished(m, animator).then_some(request),
        ATTACK1 | ATTACK2 | 0x12 | 0x14 => finished(m, animator).then(|| {
            event = Some(Event::Blow(false));
            m.action + 1
        }),
        ATTACK3 => finished(m, animator).then(|| {
            event = Some(Event::Blow(true));
            m.action + 1
        }),
        0xD => finished(m, animator).then(|| if model.has(ATTACK2) { ATTACK2 } else { request }),
        0xF | 0x11 | 0x13 | 0x15 => finished(m, animator).then_some(request),
        THROW1 | THROW2 => finished(m, animator).then(|| {
            event = Some(Event::Throw);
            THROWF
        }),
        THROWF => finished(m, animator).then_some(match request {
            THROW1 => THROW2,
            READY => ATTTOREADY,
            r => r,
        }),
        RUNATTACK1 => finished(m, animator).then(|| {
            if request == RUNATTACK1 {
                event = Some(Event::Throw);
                RUNATTACK2
            } else {
                request
            }
        }),
        RUNATTACK2 => finished(m, animator).then_some(request),
        ATTTOREADY => finished(m, animator).then_some(READY),
        // Getting up plays out, then it walks (runs, with no WALK), unless
        // a hit or its death comes first.
        READYTOWALK if request < HIT1 => {
            finished(m, animator).then_some(if model.has(WALK) { WALK } else { RUN })
        }
        _ => Some(request),
    };
    if let Some(mut next) = next {
        next = match next {
            WALK if !model.has(WALK) => RUN,
            RUN if !model.has(RUN) => WALK,
            ATTACK2 | ATTACK3 | 0x12 | 0x14 if !model.has(next) => ATTACK1,
            THROW2 if !model.has(THROW2) => THROW1,
            THROW1 if !model.has(THROW1) => THROW2,
            RUNATTACK1 if !model.has(RUNATTACK1) => RUNATTACK2,
            RUNATTACK2 if !model.has(RUNATTACK2) => RUNATTACK1,
            ATTTOREADY if !model.has(ATTTOREADY) => READY,
            n => n,
        };
        let restart = next != m.action;
        if restart && (THROW1..=THROWF).contains(&next) {
            let frames = model.actions[next as usize].and_then(|a| animator.clips.actions.get(a)).map_or(0, |a| a.frames);
            let (pause, carry) = projectiles::throw_pause(m.throw_rate * timing + m.throw_carry, frames.into());
            m.throw_carry = carry;
            if let Some(pause) = pause {
                m.throw_pause = pause;
            }
        }
        m.action = next;
        match model.actions[next as usize].or(model.actions[READY as usize]) {
            Some(a) if restart || animator.action != a => animator.play(a),
            _ => {}
        }
    }
    event
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
    (to[0] - from[0]).datan2(to[2] - from[2])
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

/// Places the monsters between ticks, drawn at the enemies' scale (the
/// shrink power's).
fn interpolate(fixed: Res<Time<Fixed>>, enemies: Res<EnemyScale>, mut monsters: Query<(&Monster, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (m, mut transform) in &mut monsters {
        let (p0, f0) = m.previous;
        transform.translation = Vec3::from(p0).lerp(Vec3::from(m.position), t);
        transform.rotation = Quat::from_rotation_y(f0 + locomotion::wrap(m.facing - f0) * t);
        transform.scale = Vec3::splat(enemies.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blows_flinch_or_knock_down_like_the_game() {
        assert_eq!(reaction(5.0, 0, 1.5, 4), (HIT1, 0.0));
        assert_eq!(reaction(5.0, 0x10, 1.5, 4), (HIT1, 8.0), "strong: flinch with a push");
        assert_eq!(reaction(5.0, 0x20, 1.5, 3), (HIT2, 40.0), "heavy: a small monster is knocked down");
        assert_eq!(reaction(5.0, 0x20, 3.0, 4), (HIT2, 20.0), "big monsters (step above 2) half as far");
        assert_eq!(reaction(5.0, 0x20, 3.0, GOLEM), (HIT2, 2.0));
        assert_eq!(reaction(5.0, 0x20, 1.5, ACID_BLOB), (HIT2, 0.0));
        assert_eq!(reaction(12.0, 0x200, 1.5, 4).0, HIT2);
        assert_eq!(reaction(8.0, 0x200, 1.5, 4).0, HIT1);
    }

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
