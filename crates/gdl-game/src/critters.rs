//! Critters: the game's scripted monsters, run from their `CRITTER/*.WAD`
//! data (`docs/critters.md` has the game's code behind all of this). This
//! runs the placed golem and the level's boss (one-part bosses so far).
//!
//! A golem stands as a statue until a wake trigger by it comes on; a boss
//! is made at the level's boss locator and sleeps (playing INIT) until two
//! seconds have passed and its targets are within its wake distance. Then
//! every 30 Hz tick a critter
//!
//! 1. scores the players against its type's target condition: the golem
//!    keeps the best, a boss every one that passes (up to four); players a
//!    critter hit in the last quarter second score worse;
//! 2. picks a move: the forced ones first (INIT → START, DEATH, a move's
//!    follow-up, a boss's READY after START, then a knockdown, knockback,
//!    roar or flinch for the blows it took), else a block when its target
//!    is attacking, else the next step of a running pattern or the least
//!    recently used pattern or attack whose condition a target meets, else
//!    the movement move whose condition scores its target best, else a
//!    taunt (unhurt) or READY — every move only when its cooldown since it
//!    last ended has run out;
//! 3. switches to it the way the move's transition allows (at once, or
//!    when the animation playing ends); a boss holds a move at least its
//!    hold time;
//! 4. lands the move's blows on their frames — a sphere on the move's node
//!    swept from its last position, a breath cone along the node, a
//!    missile (with the critter's own effect model, aimed with an arc at
//!    its target, faster the angrier it is) or a ring on the ground — and
//!    plays its sounds; a player a critter hit can't be hit by one again
//!    for 0.25 s;
//! 5. walks at the move's speed in the move's direction plus its knockback
//!    (a golem on the level's walls and floor, stopping short of players;
//!    a boss kept within its leash of home) and turns toward its target at
//!    the move's rate.
//!
//! Blows from the hero reach it through `damage.rs` ([`Critter::take_hit`]):
//! critters whose type has hit spheres are hit on those ([`CritterSphere`],
//! each with its share of hit points and damage scale), others on the
//! body; a blocking critter takes a quarter, its armour comes off each
//! blow, the hero earns a share of its experience per blow and a fifth of
//! it for the kill; the blows' kinds pick its hit reaction and push it
//! (bosses aren't pushed). At 0 hit points it plays DEATH and is removed
//! when that ends (a boss when its hold after it ends).
//!
//! A statue only wakes when a wake trigger (flag 0x2000) next to it comes
//! on, as in the game — most placed golems are never woken.
//! `GDL_WAKE_STATUES=<range>` (a testing aid) also wakes them when the
//! hero comes that close.
//!
//! A hero who brings the realm's legendary item gets the boss intro
//! (`GDL_LEGENDARY=1`, a testing aid, pretends so): once START is over
//! the level darkens, the boss idles 1–3 s, roars, and fights (the
//! chimera only after a missile hits it); a missile in the dark freezes
//! the dragon or stuns the djinn and cuts the intro short.
//!
//! Stand-ins (see the doc): a woken statue goes straight to its ACTIVE
//! animation and its critter appears when that ends; ground rings (a
//! damaging effect in the game) hurt players in their radius at once;
//! critter missiles live the missiles' three seconds; the chimera's wake
//! timer starts at once. The darkening isn't drawn; the heroes' side of
//! the intro, the boss camera, the boss key, parts (the chimera's heads),
//! breaking nodes, the health meter, effects and fading, its blows on
//! other monsters and pushing players aside aren't done.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::Affine3A;
use bevy::prelude::*;
use gdl_formats::anim::AnimFile;
use gdl_formats::audio::AudioCatalog;
use gdl_formats::collision::{node_flags, push_out};
use gdl_formats::critter::{self, CritterDamage, CritterFile, CritterMove, Condition, class, kind};
use gdl_formats::population::{ItemClass, LocatorKind, PlacementParams, REALM_LETTERS, rotation_matrix};
use gdl_formats::{LevelCollision, ModelFile};

use crate::audio::PlaySound;
use crate::character::{Animator, CharacterData, CharacterModel, advance_clip, clip_end};
use crate::combat::{TargetKind, Targetable};
use crate::damage::after_armor;
use crate::effects::effect_life;
use crate::exits::ChangeLevelTo;
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::mechanics::Mechanics;
use crate::monsters::{MonsterLevel, MonsterTick};
use crate::player::Player;
use crate::player_state::{DamagePlayer, PlayerState};
use crate::population::LevelPopulation;
use crate::projectiles::{cylinder_hit, load_atree, spawn_critter_missile};
use crate::world::{LevelEntity, LevelGround};

pub struct CrittersPlugin;

impl Plugin for CrittersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (tick_critters.after(MonsterTick), run_victory.after(tick_critters)))
            .add_systems(
            Update,
            (setup_level.run_if(resource_added::<MonsterLevel>), interpolate).chain(),
        );
    }
}

/// The end of a boss level, once the boss is gone (the game's end
/// sequence; its steps as the game numbers them):
///
/// - 0: the heroes win the realm; the key shows where the boss was made
///   (not after the skornes or garm), with its sound, its second model
///   taking over for 30 s when its clip is done;
/// - 1–2: 5 s (10 s after the first skorne and garm);
/// - 3–4: the wizard appears halfway between the boss's spot and the
///   heroes, 3 above them (at a fixed spot for the skornes), and fades in;
/// - 5: he speaks; half a second after, 6: his second speech (how many of
///   the realm's runestones the heroes hold), and a second after that
/// - 8: the last countdown, 2 s, ends the level.
///
/// Stand-ins: his messages aren't shown, so each speech's length stands in
/// for its pages; he doesn't fade; the heroes' teleport-out effect isn't
/// drawn; the next level is the tower's first.
#[allow(clippy::too_many_arguments)]
fn run_victory(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    mut state: Option<ResMut<PlayerState>>,
    players: Query<&Player>,
    mut animators: Query<&mut Animator>,
    mut sounds: MessageWriter<PlaySound>,
    mut change: MessageWriter<ChangeLevelTo>,
) {
    let Some(mut level) = level else { return };
    let level = &mut *level;
    let Some(mut v) = level.victory.take() else { return };
    let now = level.now;
    let spawn = |model: &Arc<CharacterModel>, at: [f32; 3], commands: &mut Commands| {
        let e = model.spawn(Transform::from_translation(Vec3::from(at)), commands);
        commands.entity(e).insert(LevelEntity);
        e
    };

    // The key: its clip, then the second model for 30 s.
    if let Some((e, until, second)) = v.key
        && now >= until
    {
        commands.entity(e).try_despawn();
        v.key = match (&level.end.key_after, second) {
            (Some(m), false) => Some((spawn(m, v.key_at, &mut commands), now + KEY_AFTER, true)),
            _ => None,
        };
        debug!("the boss key {}", if v.key.is_some() { "turns to its second model" } else { "goes" });
    }

    match v.step {
        0 => {
            if let Some(s) = state.as_deref_mut() {
                s.realms_beaten |= 1 << level.realm_id;
            }
            info!("realm {} beaten", level.realm_id);
            if let Some((m, life)) = &level.end.key {
                v.key = Some((spawn(m, v.key_at, &mut commands), now + life, false));
                sounds.write(PlaySound(format!("S_BOSSKEY{}", level.realm)));
                info!("the boss key shows at {:?} for {life:.2} s", v.key_at);
            }
            v.step = 1;
        }
        1 => {
            let wait = if matches!(level.boss_type, SKORNE | GARM) { WIZARD_DELAY_LONG } else { WIZARD_DELAY };
            v.timer = now + wait;
            v.step = 2;
        }
        2 if now >= v.timer => v.step = 3,
        3 => {
            let at = if matches!(level.boss_type, SKORNE | SKORNE2) {
                WIZARD_SKORNE_SPOT
            } else {
                let heroes: Vec<[f32; 3]> = if state.as_ref().is_none_or(|s| s.alive) {
                    players.iter().map(|p| p.mover.position).collect()
                } else {
                    Vec::new()
                };
                let mut at = wizard_spot(level.boss_spot, &heroes);
                at[1] += WIZARD_RISE;
                at
            };
            v.wizard = level.end.wizard.as_ref().map(|m| spawn(m, at, &mut commands));
            info!("the wizard appears at {at:?}");
            v.fade = WIZARD_FADE;
            v.step = 4;
        }
        4 => {
            v.fade -= WIZARD_FADE_STEP;
            if v.fade <= 0 {
                let length = speak(level, 0, &mut sounds);
                v.timer = now + length + AFTER_FIRST_SPEECH;
                v.step = 5;
            }
        }
        5 if now >= v.timer => {
            let runes = state.as_ref().map_or(0, |s| runes_held(level.realm_id, &s.runestones));
            let length = if level.boss_type < SKORNE { speak(level, runes + 1, &mut sounds) } else { 0.0 };
            v.timer = now + length + AFTER_SECOND_SPEECH;
            v.step = 6;
        }
        6 if now >= v.timer => {
            v.countdown = COUNTDOWN;
            v.step = 8;
        }
        8 | 10 => {
            v.countdown -= DT;
            if v.step == 8 && v.countdown <= TELEPORT_LEFT {
                debug!("the heroes teleport out (the effect isn't drawn)");
                v.step = 10;
            }
            if v.countdown <= 0.0 {
                info!("the boss level is over: to {AFTER_BOSS_LEVEL}");
                change.write(ChangeLevelTo(AFTER_BOSS_LEVEL.into()));
                // The key and the wizard go with the level.
                return;
            }
        }
        _ => {}
    }
    // The wizard plays his first clip over and over.
    if let Some(mut a) = v.wizard.and_then(|w| animators.get_mut(w).ok())
        && a.finished()
    {
        a.play(0);
    }
    level.victory = Some(v);
}

/// Plays the wizard's speech `n`: its length (0 when there's none).
fn speak(level: &CritterLevel, n: usize, sounds: &mut MessageWriter<PlaySound>) -> f32 {
    match level.end.speeches.get(n).cloned().flatten() {
        Some((name, length)) => {
            info!("the wizard says {name} ({length:.1} s)");
            sounds.write(PlaySound(name));
            length
        }
        None => 0.0,
    }
}

/// Where the wizard stands: the mean of the boss's spot and the heroes,
/// at the first hero's height (the game counts it twice in place of the
/// spot's).
fn wizard_spot(spot: [f32; 3], heroes: &[[f32; 3]]) -> [f32; 3] {
    let mut sum = spot;
    for (i, h) in heroes.iter().enumerate() {
        sum = add(sum, *h);
        if i == 0 {
            sum[1] = 2.0 * h[1];
        }
    }
    scale(sum, 1.0 / (1 + heroes.len()) as f32)
}

/// Seconds per 30 Hz tick: critters keep time in seconds.
const DT: f32 = 1.0 / 30.0;
/// Enemy type of the golem, and the first boss type.
const GOLEM: i32 = 0x1D;
const FIRST_BOSS: i32 = 0x22;
/// Boss types the intro and the end treat apart.
const DRAGON: i32 = 0x22;
const CHIMERA: i32 = 0x23;
const DJINN: i32 = 0x24;
const DRIDER: i32 = 0x25;
const PBOSS: i32 = 0x26;
const YETI: i32 = 0x27;
const WRAITH: i32 = 0x28;
const LICH: i32 = 0x29;
const SKORNE: i32 = 0x2A;
const SKORNE2: i32 = 0x2B;
const GARM: i32 = 0x2C;
/// Bosses up to this type have an intro (not the second skorne or garm).
const LAST_INTRO_BOSS: i32 = 0x2A;
/// The pickup kind of the realms' legendary items (its amount is the realm).
const LEGENDARY: i32 = 13;

/// The boss intro's states (the game's `r13-0x725c`, `docs/critters.md`
/// "The boss intro").
mod intro {
    /// No intro: no hero brought the realm's legendary item.
    pub const NONE: i32 = 0;
    /// Until the boss's START ends.
    pub const START: i32 = 1;
    /// A wait while the level darkens.
    pub const WAIT: i32 = 2;
    /// The boss roars (still dark).
    pub const ROAR: i32 = 3;
    /// The roar is over; the level update moves 4 on to 5 at once.
    pub const ROARED: i32 = 4;
    pub const AFTER: i32 = 5;
    pub const FIGHT: i32 = 6;
    /// The boss is dead.
    pub const OVER: i32 = 99;
}
/// The intro's wait: 1 s for the chimera, the lich and the first skorne,
/// 3 s for the rest.
const INTRO_WAIT_SHORT: f32 = 1.0;
const INTRO_WAIT: f32 = 3.0;
/// Seconds in state 5 before the djinn, P-boss, yeti, wraith and first
/// skorne go on to 6.
const INTRO_AFTER: f32 = 29.0;
/// A missile hitting the dragon in states 2–3 freezes it, the djinn and the
/// P-boss are stunned (1200, 1800 and 18000 fields at 60 a second).
const DRAGON_FREEZE: f32 = 20.0;
const DJINN_STUN: f32 = 30.0;
const PBOSS_STUN: f32 = 300.0;
/// A stunned critter turns at this share of its rate.
const STUNNED_TURN: f32 = 0.1;
/// The level light's darkening in states 2–3: each tick asks for this
/// offset for this long; the offset moves at most these steps a tick; an
/// expired target decays by this factor and snaps to 0 below the last.
const DARKEN_TO: f32 = -0.8;
const DARKEN_HOLD: f32 = 0.1;
const DARKEN_STEP: f32 = -0.25;
const BRIGHTEN_STEP: f32 = 0.05;
const DARKEN_DECAY: f32 = 0.6;
const DARKEN_SNAP: f32 = 0.05;

/// The end of a boss level: the second key model shows this long; the
/// wizard appears 5 s after the boss goes (10 s after the first skorne and
/// garm), 3 above the heroes' centre (at a fixed spot for the skornes),
/// fading in 4 of 255 a tick; half a second after his first speech and a
/// second after his second the last countdown starts; the heroes teleport
/// out 35 fields before its end.
const KEY_AFTER: f32 = 30.0;
const WIZARD_DELAY: f32 = 5.0;
const WIZARD_DELAY_LONG: f32 = 10.0;
const WIZARD_RISE: f32 = 3.0;
const WIZARD_SKORNE_SPOT: [f32; 3] = [0.0, -12.0, 6.0];
const WIZARD_FADE: i32 = 255;
const WIZARD_FADE_STEP: i32 = 4;
const AFTER_FIRST_SPEECH: f32 = 0.5;
const AFTER_SECOND_SPEECH: f32 = 1.0;
const COUNTDOWN: f32 = 2.0;
const TELEPORT_LEFT: f32 = 35.0 / 60.0;
/// A speech missing from the sound catalog counts as this long.
const SPEECH_GUESS: f32 = 4.5;
/// Where the heroes go after a boss (stand-in: the game picks the first
/// level of world 13 flagged for it).
const AFTER_BOSS_LEVEL: &str = "levelL1";

/// A wake trigger wakes the nearest statue within this (horizontally,
/// less the statue item's radius).
const WAKE_REACH: f32 = 10.0;
/// A boss sleeps at least this long before it may wake.
const BOSS_WAKE_DELAY: f32 = 2.0;
/// `TYPE +0x5C` flags: hit spheres, a boxed leash, a wake timer started by
/// the floor, free turning.
const TYPE_SPHERES: u32 = 0x2;
const TYPE_BOX_LEASH: u32 = 0x20;
const TYPE_FLOOR_WAKE: u32 = 0x80;
const TYPE_FREE_TURN: u32 = 0x400;

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
/// Bosses track at most this many players.
const MAX_TARGETS: usize = 4;
/// A blocking critter takes this share of a blow.
const BLOCKED: f32 = 0.25;
/// Share of its experience every player earns for the kill.
const KILL_EXPERIENCE: f32 = 0.2;
/// A boss takes blows × this for 0..4 players (outside its intro).
const BOSS_DAMAGE_BY_PLAYERS: [f32; 5] = [1.0, 1.0, 0.5, 0.3, 0.2];
/// The roar damage is × this for 0..4 players.
const ROAR_BY_PLAYERS: [f32; 5] = [1.0, 1.0, 1.5, 2.0, 2.0];
/// Players: only one hero runs here.
const PLAYERS: usize = 1;
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
/// Ticks after its event before `GDL_CRITTER_SHOT` saves its screenshot.
const SHOT_DELAY: u32 = 8;
/// When a move that has never run "ended".
const NEVER: f32 = -1.0e6;
/// A critter drops at most this fast (units/s).
const MAX_DROP: f32 = 16.0;
/// Missiles: the span of the launch speed range anger reaches (from 0.5
/// to 1.5 anger), and the launch direction's drop when it aims at nothing.
const SPEED_SPAN: f32 = 0.75;
const ANGER_CAP: f32 = 1.5;
const UNAIMED_DROP: f32 = -0.5;

/// Blow kind bits.
const KIND_STRONG: u32 = 0x10;
const KIND_KNOCKDOWN: u32 = 0x20;
const KIND_HEAVY: u32 = 0x100;
const KIND_REACTIONS: u32 = 0x130;
/// Blow kinds that don't land on a hit sphere (they hit the body).
const KIND_BODY: u32 = 0x100320;

/// One critter file loaded for the level, with a model per body.
pub struct CritterKind {
    pub file: CritterFile,
    bodies: Vec<Option<Body>>,
    /// The statue a placed one stands as until it wakes.
    statue: Option<CharacterModel>,
    /// Missile models: effect atrees named by its `SFXX` records.
    effects: HashMap<usize, Arc<CharacterModel>>,
}

/// A body's model and what its moves animate.
struct Body {
    model: CharacterModel,
    /// Per move (within the type): its action (a missing one plays the
    /// first, as the game does), and its node.
    actions: Vec<usize>,
    nodes: Vec<Option<usize>>,
    /// Per `NODE` record of the type: its skeleton node (none: the root).
    spheres: Vec<Option<usize>>,
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

/// One of a critter's hit spheres (its type's `NODE` records): the hero
/// hits these instead of the body.
#[derive(Component, Clone, Copy, Debug)]
pub struct CritterSphere {
    pub critter: Entity,
    pub node: usize,
}

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
    /// The boss, once made; whether it has died (its DEATH has played:
    /// the game's `r13-0x7784`).
    pub boss: Option<Entity>,
    pub boss_dead: bool,
    /// The level's boss type (`LEVL +0x44`, −1 for none) and its realm's
    /// number.
    boss_type: i32,
    realm_id: u32,
    /// The boss intro: its state ([`intro`]) and timer, and whether the
    /// legendary item that started it has been used up.
    pub intro: i32,
    intro_timer: f32,
    legendary_used: bool,
    /// The scene light's darkening during the intro.
    light: Darkening,
    /// The boss locator's position; the end's models; the end, once the
    /// boss is gone.
    boss_spot: [f32; 3],
    end: EndModels,
    victory: Option<Victory>,
    /// Events this tick, for `GDL_CRITTER_SHOT_ON`.
    events: Vec<&'static str>,
    /// Stand-in look for critter missiles: their effect models are drawn by
    /// the effects system (textures it supplies), which isn't done.
    glow: (Handle<Mesh>, Handle<StandardMaterial>),
    rng: u32,
}

impl CritterLevel {
    /// The offset the game adds to the scene's brightness (0 normally, down
    /// to −0.8 in the boss intro; the brightness is clamped to 0..1). The
    /// renderer doesn't apply it yet.
    pub fn light_offset(&self) -> f32 {
        self.light.offset
    }

    fn random(&mut self) -> f32 {
        // xorshift32; the game has its own generator.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The scene-light offset (the game's `r13-0x7170`): a caller asks for a
/// target for a while; every frame the offset moves toward the target,
/// quickly down and slowly up, and a target nobody asks for any more
/// decays to 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Darkening {
    offset: f32,
    target: f32,
    until: f32,
}

impl Darkening {
    fn ask(&mut self, now: f32, hold: f32, target: f32) {
        let until = now + hold + DT;
        if until > self.until {
            self.until = until;
            self.target = target;
        }
    }

    fn step(&mut self, now: f32) {
        if self.target != 0.0 && self.until < now {
            self.target *= DARKEN_DECAY;
            if self.target.abs() < DARKEN_SNAP {
                self.target = 0.0;
            }
        }
        self.offset += (self.target - self.offset).clamp(DARKEN_STEP, BRIGHTEN_STEP);
    }
}

/// The models of a boss level's end, from the level's own item set
/// (`ITEMS/<level>`): the key (and its clip's length), its second model,
/// the wizard; and the wizard's speeches for this boss with their lengths.
#[derive(Default)]
struct EndModels {
    key: Option<(Arc<CharacterModel>, f32)>,
    key_after: Option<Arc<CharacterModel>>,
    wizard: Option<Arc<CharacterModel>>,
    speeches: Vec<Option<(String, f32)>>,
}

/// The end of a boss level, from the boss's removal: the game's end
/// sequence, its steps numbered as the game numbers them.
struct Victory {
    step: u8,
    timer: f32,
    /// Where the key shows: the boss's spawn position plus `TYPE +0xD0`.
    key_at: [f32; 3],
    /// The key's model, until when it shows, and whether it's the second.
    key: Option<(Entity, f32, bool)>,
    wizard: Option<Entity>,
    fade: i32,
    countdown: f32,
}

/// The game's instance state (0 new, 1 dying, 3 active).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CritterState {
    New,
    Dying,
    Active,
}

/// A target the critter tracks (the game keeps up to four).
#[derive(Clone, Copy, Debug)]
struct Tracked {
    player: Entity,
    /// Horizontal distance from its centre, and the unit direction.
    distance: f32,
    direction: [f32; 2],
    position: [f32; 3],
    score: f32,
    /// Scales the score (the game weighs damage dealt against taken).
    weight: f32,
    /// The player's centre (missiles aim here).
    centre: [f32; 3],
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

    /// Moves the clip on at its rate (900 / rate frames a second): a loop
    /// has ended on the tick it starts over, any other clip once it's past
    /// its last frame.
    fn advance(&mut self, dt: f32) {
        let wrapped = advance_clip(&mut self.frame, dt, self.frames, self.rate, self.loops);
        self.ended = if self.loops { wrapped } else { self.frame >= clip_end(self.frames) };
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
    /// Root position (feet, plus its hover height), and where it was made.
    pub position: [f32; 3],
    spawned_at: [f32; 3],
    pub yaw: f32,
    home_yaw: f32,
    home: [f32; 3],
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
    /// When each move last ended; when each pattern last started.
    ends: Vec<f32>,
    pattern_starts: Vec<f32>,
    /// The pattern running and its step, and the one just chosen.
    pattern: Option<(usize, usize)>,
    chosen_pattern: Option<usize>,
    /// A boss holds its move until this time.
    hold_until: f32,
    /// A sleeping boss may wake from this time.
    wake_at: f32,
    /// Blows and sounds done this move (bits 1, 2).
    blows_done: u8,
    sounds_done: u8,
    tracked: Vec<Tracked>,
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
    /// Per hit sphere: damage taken (it stops counting at its share of hit
    /// points), and its entity.
    sphere_damage: Vec<f32>,
    spheres: Vec<Entity>,
    blows_dealt: u32,
    /// Hit points at the start of the last tick.
    hp_before: f32,
    /// The critter clock at its last tick (seconds since the level began).
    now: f32,
    /// A missile hit it since its last tick (the intro reacts).
    missile_hit: bool,
    /// Seconds it stays frozen (its animation and moves stop) and stunned
    /// (it turns slowly): the intro's reactions to missiles.
    frozen: f32,
    stunned: f32,
}

impl Critter {
    fn moves(&self) -> &[CritterMove] {
        self.kind.file.type_moves(self.ty)
    }

    fn body(&self) -> &Body {
        self.kind.bodies[self.ty].as_ref().expect("critters are only made with a body")
    }

    fn class(&self) -> i16 {
        self.kind.file.desc.class
    }

    fn move_kind(&self, i: Option<usize>) -> Option<i32> {
        i.and_then(|i| self.moves().get(i)).map(|m| m.kind)
    }

    /// A blow from the hero, on hit sphere `sphere` or the body (`ranged`:
    /// a missile or thrown weapon): returns the experience it earns.
    /// `sounds` gets the names of the sounds to play.
    #[allow(clippy::too_many_arguments)]
    pub fn take_hit(
        &mut self,
        damage: f32,
        kind_bits: u32,
        push: [f32; 3],
        sphere: Option<usize>,
        ranged: bool,
        level: Option<&CritterLevel>,
        sounds: &mut Vec<String>,
    ) -> u32 {
        if self.state != CritterState::Active || self.hit_points <= 0.0 {
            return 0;
        }
        let (realm, intro) = level.map_or(('A', intro::NONE), |l| (l.realm, l.intro));
        self.missile_hit |= ranged;
        let ty = self.kind.file.types[self.ty].clone();
        let boss = self.class() == class::BOSS;
        let (mut damage, mut kind_bits) = (damage, kind_bits);
        if self.move_kind(self.current) == Some(kind::BLOCK) {
            kind_bits &= !KIND_REACTIONS;
            damage *= BLOCKED;
        }
        damage = after_armor(damage, ty.armor);
        self.damage_taken += damage;
        // Bosses take less with more players, except in their intro's
        // first four states.
        if boss && !(intro::START..=intro::ROARED).contains(&intro) {
            damage *= BOSS_DAMAGE_BY_PLAYERS[PLAYERS];
        }
        let share = damage.clamp(0.0, self.hit_points.max(0.0)) / (1.0 + self.full_hit_points);
        let mut xp = (share * ty.experience) as u32;
        if boss {
            xp *= PLAYERS as u32;
        }
        // A blow on a hit sphere that still has hit points is scaled by it,
        // up to what it has left; once a sphere is spent, blows on it land
        // on the body in full (the game skips the sphere then).
        if kind_bits & KIND_BODY == 0
            && let Some(n) = sphere
            && let Some(node) = self.kind.file.type_nodes(self.ty).get(n)
        {
            let full = node.hit_points * self.full_hit_points;
            let taken = self.sphere_damage[n];
            if taken < full {
                damage = (damage * node.damage_scale).min(full - taken);
                self.sphere_damage[n] = taken + damage;
            }
        }
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
            sound_chain(&self.kind.file, s, realm, sounds);
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

/// Loads the critter files the level's enemy slots name (the golem and the
/// boss for now), their models; places the statues and makes the boss.
#[allow(clippy::too_many_arguments)]
fn setup_level(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    monsters: Res<MonsterLevel>,
    population: Option<Res<LevelPopulation>>,
    ground: Option<Res<LevelGround>>,
    state: Option<Res<PlayerState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(population), Some(ground)) = (population, ground) else { return };
    let realm_id = REALM_LETTERS.iter().find(|(l, _)| *l == monsters.realm).map_or(1, |r| r.1);
    let realm_items = format!("level{}", monsters.realm);
    let mut kinds = HashMap::new();
    let wanted = monsters.enemies.loaded.iter().map(|e| e.0).filter(|&e| e == GOLEM || e >= FIRST_BOSS);
    for enemy in wanted {
        if kinds.contains_key(&enemy) {
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
        let mut build = |d: &CharacterData| CharacterModel::build(d, &mut meshes, &mut materials, &mut images);
        let bodies = (0..file.types.len())
            .map(|ty| {
                let owner = file.types[ty].parent.unwrap_or(ty);
                let d = data(&file.atree_name(owner))?;
                let moves = file.type_moves(ty);
                let actions = moves
                    .iter()
                    .map(|m| d.clips.actions.iter().position(|a| a.name == m.anim).unwrap_or(0))
                    .collect();
                let node = |name: &str| (!name.is_empty()).then(|| d.skeleton.node_index(name)).flatten();
                let nodes = moves.iter().map(|m| node(&m.node)).collect();
                let spheres = file.type_nodes(ty).iter().map(|n| node(&n.name)).collect();
                let clips = d.clips.actions.iter().map(|a| (a.frames, a.rate, a.loops())).collect();
                Some(Body { model: build(&d), actions, nodes, spheres, clips })
            })
            .collect::<Vec<_>>();
        if bodies.first().is_none_or(Option::is_none) {
            warn!("{folder}: no atree {}", file.atree_name(0));
            continue;
        }
        let statue = data("GOL_STATUE").map(|d| build(&d));
        // Missile models: the effect a projectile blow starts names an
        // atree of the critter's own folder.
        let mut effects = HashMap::new();
        for d in &file.damage {
            if !matches!(d.kind, 1 | 8) {
                continue;
            }
            let Ok(e) = usize::try_from(d.effects[0]) else { continue };
            let Some(name) = file.sounds.get(e).map(|s| s.effect.clone()) else { continue };
            if effects.contains_key(&e) || name.is_empty() {
                continue;
            }
            match data(&name) {
                Some(md) => {
                    debug!(
                        "{folder}: missile model {name}: nodes {:?}, objects {:?}",
                        md.skeleton.nodes.iter().map(|n| (n.name.as_str(), n.has_model(), n.hidden(), n.render_flags)).collect::<Vec<_>>(),
                        md.model
                            .objects
                            .iter()
                            .filter(|o| o.name.starts_with(&name))
                            .map(|o| (o.name.as_str(), o.submeshes.len(), o.flags))
                            .collect::<Vec<_>>()
                    );
                    effects.insert(e, Arc::new(build(&md)));
                }
                None => debug!("{folder}: no missile model {name}"),
            }
        }
        info!(
            "critter {} ({path}) from {folder}: {} moves, statue {}, {} missile models",
            file.desc.name,
            file.moves.len(),
            statue.is_some(),
            effects.len()
        );
        kinds.insert(enemy, Arc::new(CritterKind { file, bodies, statue, effects }));
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
        if kind.file.desc.class == class::BOSS {
            continue;
        }
        let mut position = p.position;
        if let Some(y) = ground.0.floor_height(position) {
            position[1] = y;
        }
        let m = rotation_matrix(p.rotation);
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
    let mut level = CritterLevel {
        kinds,
        statues,
        now: 0.0,
        realm: monsters.realm,
        hit_point_scale: t.monster_hit_points * testing_scale,
        speed_scale: t.monster_speed,
        damage_scale: t.monster_damage,
        guard: HashMap::new(),
        boss: None,
        boss_dead: false,
        boss_type: monsters.boss,
        realm_id,
        intro: intro::NONE,
        intro_timer: 0.0,
        legendary_used: false,
        light: Darkening::default(),
        boss_spot: [0.0; 3],
        end: EndModels::default(),
        victory: None,
        events: Vec::new(),
        glow: (
            meshes.add(Sphere::new(1.0)),
            standard.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.55, 0.1),
                emissive: LinearRgba::rgb(4.0, 1.6, 0.2),
                unlit: true,
                ..default()
            }),
        ),
        rng: 0x2545_F491,
    };

    // The boss appears at the level's boss locator, dropped onto the floor
    // below it.
    if monsters.boss >= FIRST_BOSS
        && let Some(kind) = level.kinds.get(&monsters.boss).cloned()
        && let Some(spot) = pop.locators.iter().find(|l| l.kind == LocatorKind::Boss)
    {
        let m = rotation_matrix(spot.rotation);
        let yaw = m[6].atan2(m[8]);
        let mut at = spot.position;
        if let Some(h) = ground.0.floor_probe(at, 4.0, -1000.0, 5.0, 2) {
            at[1] = h.point[1];
        }
        level.boss = spawn_critter(&level, &kind, at, yaw, &mut commands);
        level.boss_spot = spot.position;
        level.end = load_end_models(&mut game, &population.level, monsters.boss, monsters.realm, &mut meshes, &mut materials, &mut images);
        info!("boss {} at {at:?} facing {:.0}°", kind.file.desc.name, yaw.to_degrees());
        // The intro runs when a hero brings the realm's legendary item
        // (`GDL_LEGENDARY=1`, a testing aid, pretends one does); the second
        // skorne and garm have none.
        let carried = state.as_ref().is_some_and(|s| s.treasures.contains(&(LEGENDARY, realm_id as i32)));
        let pretend = std::env::var("GDL_LEGENDARY").is_ok_and(|v| v == "1");
        if (0..=LAST_INTRO_BOSS).contains(&monsters.boss) && (carried || pretend) {
            level.intro = intro::START;
            info!("the boss intro will run (the hero brings the legendary item{})", if carried { "" } else { ": GDL_LEGENDARY" });
        }
    }
    commands.insert_resource(level);
}

fn read_folder(game: &mut LoadedGame, folder: &str) -> Option<(AnimFile, ModelFile, Vec<u8>)> {
    let anim = AnimFile::parse(&game.install.read(&format!("{folder}/ANIM.PS2")).ok()?).ok()?;
    let model = ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?;
    let textures = game.install.read(&format!("{folder}/textures.ngc")).ok()?;
    Some((anim, model, textures))
}

/// Loads the models of the boss level's end from the level's own item set:
/// the key (bosses before the first skorne) and the wizard; and the lengths
/// of the wizard's speeches (from the sound catalog).
fn load_end_models(
    game: &mut LoadedGame,
    level: &str,
    boss_type: i32,
    letter: char,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> EndModels {
    let folder = format!("ITEMS/{level}");
    let mut build = |name: &str| -> Option<(Arc<CharacterModel>, f32)> {
        let data = load_atree(game, &folder, name)?;
        // A clip without frames plays 30.
        let life = data.clips.actions.first().map_or(1.0, |a| effect_life(if a.frames == 0 { 30 } else { a.frames }, a.rate));
        debug!(
            "{folder}/{name}: {} actions {:?}",
            data.clips.actions.len(),
            data.clips.actions.iter().map(|a| (a.name.as_str(), a.frames, a.rate, a.loops())).collect::<Vec<_>>()
        );
        Some((Arc::new(CharacterModel::build(&data, meshes, materials, images)), life))
    };
    let has_key = boss_type < SKORNE;
    let key = if has_key { build("BOSSKEY") } else { None };
    let key_after = if has_key { build("BOSSKEY2").map(|m| m.0) } else { None };
    let wizard = build("WIZARD").map(|m| m.0);
    let catalog = game.install.read("AUDIO/AUDATPS2.ROM").ok().and_then(|b| AudioCatalog::parse(&b).ok());
    let speeches = (0..5)
        .map(|n| {
            let name = wizard_speech(boss_type, letter, n)?;
            let length = catalog.as_ref().and_then(|c| c.find_sound(&name)).map_or(SPEECH_GUESS, |s| s.length.max(0.0));
            Some((name, length))
        })
        .collect();
    info!(
        "boss end from {folder}: key {}, second key {}, wizard {}",
        key.is_some(),
        key_after.is_some(),
        wizard.is_some()
    );
    EndModels { key, key_after, wizard, speeches }
}

/// The wizard's speech `n` after the boss of this type falls, in realm
/// `letter` (the game's table by boss type): 0 when he appears, then 1 +
/// how many of the realm's runestones the heroes hold ([`runes_held`]).
/// The skornes and garm have one each.
fn wizard_speech(boss_type: i32, letter: char, n: usize) -> Option<String> {
    let name = match (boss_type, n) {
        (DRAGON..=LICH, 0) => format!("S_DEFEATVOX{letter}"),
        (DRAGON..=LICH, 1) => format!("S_RUNEVOX0{letter}"),
        (DRAGON..=LICH, 2) => format!("S_RUNEVOX1{letter}"),
        (DRAGON..=DRIDER, 3 | 4) => format!("S_RUNEVOX1{letter}"),
        (PBOSS..=LICH, 3 | 4) => format!("S_RUNEVOX2{letter}"),
        (SKORNE, 0) => "S_E2VOXA".into(),
        (SKORNE2, 0) => "S_ENDVOX".into(),
        (GARM, 0) => "S_GRMDESTVOX".into(),
        _ => return None,
    };
    Some(name)
}

/// How many of the realm's runestones the heroes hold (`held`: their stone
/// numbers): 0 none, 1 some, 2 all, 3 all of a realm with two or more (the
/// game's realm table: which stones, how many).
fn runes_held(realm_id: u32, held: &[i32]) -> usize {
    let (mask, count) = match realm_id {
        1 => (0x1, 1),
        2 => (0x8, 1),
        3 => (0x40, 1),
        4 => (0x200, 1),
        7 => (0x180, 2),
        8 => (0x1000, 1),
        9 => (0x30, 2),
        10 => (0xC00, 2),
        11 => (0x6, 2),
        _ => (0, 0),
    };
    let bits = held.iter().filter(|&&n| (0..32).contains(&n)).fold(0u32, |b, &n| b | 1 << n);
    if bits & mask == mask {
        if count < 2 { 2 } else { 3 }
    } else if bits & mask == 0 {
        0
    } else {
        1
    }
}

/// Makes a critter of `kind` standing at `position` facing `yaw`: its
/// hit points, its home (the type's, or here), its hit spheres.
fn spawn_critter(level: &CritterLevel, kind: &Arc<CritterKind>, position: [f32; 3], yaw: f32, commands: &mut Commands) -> Option<Entity> {
    let ty = 0;
    let body = kind.bodies[ty].as_ref()?;
    let t = &kind.file.types[ty];
    let hp = t.hit_points * level.hit_point_scale;
    let position = [position[0], position[1] + t.hover, position[2]];
    let transform = Transform::from_translation(Vec3::from(position)).with_rotation(Quat::from_rotation_y(yaw));
    let root = body.model.spawn(transform, commands);
    let moves = kind.file.type_moves(ty).len();
    let nodes = kind.file.type_nodes(ty);
    let spheres: Vec<Entity> = if t.flags & TYPE_SPHERES != 0 {
        nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let target = Targetable::new(TargetKind::Object, n.radius, n.radius);
                let sphere = CritterSphere { critter: root, node: i };
                commands.spawn((Transform::from_translation(Vec3::from(position)), target, sphere, LevelEntity)).id()
            })
            .collect()
    } else {
        Vec::new()
    };
    let critter = Critter {
        kind: kind.clone(),
        ty,
        state: CritterState::New,
        hit_points: hp,
        full_hit_points: hp,
        position,
        spawned_at: position,
        yaw,
        home_yaw: yaw,
        home: t.fixed_home().unwrap_or(position),
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
        pattern_starts: vec![NEVER; kind.file.type_patterns(ty).len()],
        pattern: None,
        chosen_pattern: None,
        hold_until: 0.0,
        wake_at: 0.0,
        blows_done: 0,
        sounds_done: 0,
        tracked: Vec::new(),
        anger: ANGER_BASE,
        damage_taken: 0.0,
        kinds: 0,
        push: [0.0; 3],
        last_blow: 0.0,
        knock: [0.0; 3],
        clock: Clock { action: 0, frame: 0.0, frames: 1, rate: 30, loops: false, ended: true },
        node_at: None,
        node_was: None,
        sphere_damage: vec![0.0; nodes.len()],
        blows_dealt: 0,
        hp_before: hp,
        now: level.now,
        spheres: spheres.clone(),
        missile_hit: false,
        frozen: 0.0,
        stunned: 0.0,
    };
    commands.entity(root).insert((critter, LevelEntity));
    if spheres.is_empty() {
        commands.entity(root).insert(Targetable::new(TargetKind::Object, t.radius, t.height));
    }
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

/// A blow for a player: damage, kind bits, push.
type Blow = (Entity, f32, u32, [f32; 3]);

#[allow(clippy::too_many_arguments)]
fn tick_critters(
    mut commands: Commands,
    level: Option<ResMut<CritterLevel>>,
    ground: Option<Res<LevelGround>>,
    mechanics: Option<ResMut<Mechanics>>,
    population: Option<Res<LevelPopulation>>,
    mut state: Option<ResMut<PlayerState>>,
    mut players: Query<(Entity, &mut Player)>,
    mut critters: Query<(Entity, &mut Critter, &mut Animator), Without<StatueModel>>,
    mut statues: Query<&mut Animator, With<StatueModel>>,
    mut spheres: Query<&mut Transform, (With<CritterSphere>, Without<Critter>)>,
    bones: Query<&GlobalTransform>,
    mut hurt: MessageWriter<DamagePlayer>,
    mut sounds: MessageWriter<PlaySound>,
    mut death_shot: Local<Option<u32>>,
) {
    let (Some(mut level), Some(ground)) = (level, ground) else { return };
    let level = &mut *level;
    level.now += DT;
    let now = level.now;
    update_intro(level);
    // The hero who brought the legendary item uses it up once the level
    // darkens (the game clears its bit in the player's record).
    if matches!(level.intro, intro::WAIT | intro::ROAR) && !level.legendary_used {
        level.legendary_used = true;
        let item = (LEGENDARY, level.realm_id as i32);
        if state.as_deref().is_some_and(|s| s.treasures.contains(&item))
            && let Some(s) = state.as_deref_mut()
        {
            s.treasures.retain(|t| *t != item);
            info!("the hero's legendary item is used up");
        }
    }
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

    let intro_before = level.intro;
    let mut blows: Vec<Blow> = Vec::new();
    let mut to_play: Vec<String> = Vec::new();
    for (entity, mut c, mut animator) in &mut critters {
        let c = &mut *c;
        c.now = now;
        c.previous = (c.position, c.yaw);
        c.switched = false;
        let ty = type_info(&c.kind.file, c.ty);
        let boss = ty.class == class::BOSS;

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
        track(c, &ty, centre, &heroes, level);
        c.anger = (1.0 - c.hit_points.max(0.0) / (1.0 + c.full_hit_points)) * ANGER_SPAN + ANGER_BASE;
        if c.state == CritterState::New {
            if !boss {
                c.state = CritterState::Active;
            } else if wake_boss(c, &ty, now) {
                c.state = CritterState::Active;
                info!("the boss {} wakes", c.kind.file.desc.name);
            }
        }
        intro_reactions(c, level);

        // Dead and done: a golem when DEATH ends, a boss when its hold does
        // (it counts as dead from the end of DEATH).
        if boss && level.boss == Some(entity) && c.move_kind(c.current) == Some(kind::DEATH) && c.clock.ended && !level.boss_dead {
            level.boss_dead = true;
        }
        if c.move_kind(c.current) == Some(kind::DEATH) && c.clock.ended && (!boss || now >= c.hold_until) {
            debug!("critter {entity:?} is gone");
            if level.boss == Some(entity) {
                let key_at = add(c.spawned_at, c.kind.file.types[c.ty].key_offset);
                level.victory = Some(Victory { step: 0, timer: 0.0, key_at, key: None, wizard: None, fade: 0, countdown: 0.0 });
                info!("the boss is gone");
            }
            for s in &c.spheres {
                commands.entity(*s).try_despawn();
            }
            commands.entity(entity).try_despawn();
            continue;
        }

        // Choose, switch.
        c.next = None;
        c.pick = None;
        c.chosen_pattern = None;
        forced(c, &ty, now, level.intro, level.boss_type);
        if c.state == CritterState::Active {
            if c.next.is_none() {
                choose_block(c, now, &heroes);
            }
            if c.next.is_none() {
                choose_attack(c, now);
            }
            let attacking = cur_kind.is_some_and(|k| k >= kind::ATTACK_FIRST);
            if c.next.is_none() && !(boss && attacking) {
                choose_movement(c, centre, now);
            }
            if c.next.is_none() {
                let taunt = if c.anger < TAUNT_ANGER { find(c, kind::TAUNT, Find::Ready, now) } else { None };
                c.next = taunt.or_else(|| find(c, kind::READY, Find::Nearest, now));
            }
        }
        if c.next.is_none() && !boss {
            c.next = c.current;
        }
        let was = c.current;
        // Frozen, it keeps its move and frame.
        if c.frozen <= 0.0 {
            switch(c, now, &mut animator, &mut level.intro);
        }
        // The intro also moves on once the boss's START (without a
        // follow-up) or ROAR has played out, before anything replaces it.
        if level.boss == Some(entity)
            && c.clock.ended
            && let Some(m) = c.current.map(|i| &c.moves()[i])
        {
            if m.kind == kind::ROAR && level.intro == intro::ROAR {
                level.intro = intro::ROARED;
            } else if m.kind == kind::START && m.next < 0 && level.intro == intro::START {
                level.intro = intro::WAIT;
            }
        }
        if c.switched && c.move_kind(c.current) == Some(kind::DEATH) {
            level.events.push("death");
        }
        if c.hit_points < c.hp_before {
            level.events.push("hurt");
        }
        c.hp_before = c.hit_points;
        if c.state == CritterState::Dying {
            commands.entity(entity).try_remove::<Targetable>();
            for s in &c.spheres {
                commands.entity(*s).try_remove::<Targetable>();
            }
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
        trace!(
            "critter {entity:?} {} frame {:.1}/{} ended {} next {:?} hold {:.2}",
            mv.name,
            c.clock.frame,
            c.clock.frames,
            c.clock.ended,
            c.next.map(|n| c.moves()[n].name.clone()),
            c.hold_until - now
        );

        // A boss holds a move its hold time past the animation's end.
        if mv.hold <= 0.0 {
            c.hold_until = 0.0;
        } else if !c.clock.ended || c.hold_until == 0.0 {
            c.hold_until = now + mv.hold;
        }

        // The move's target and node.
        if c.move_target.is_none() || c.switched {
            c.move_target = c.pick.or_else(|| best_target(c, &mv.condition, true));
        }
        if c.switched {
            c.blows_done = 0;
            c.sounds_done = 0;
            c.node_was = None;
        }
        let root = Affine3A::from_rotation_translation(Quat::from_rotation_y(c.yaw), Vec3::from(c.position));
        let bone_matrix = |n: Option<usize>| -> Affine3A {
            n.and_then(|n| animator.bone(n)).and_then(|b| bones.get(b).ok()).map_or(root, |g| g.affine())
        };
        let node_matrix = bone_matrix(c.body().nodes[cur]);
        c.node_was = c.node_at.filter(|_| c.node_was.is_some() || !c.switched);
        c.node_at = Some(node_matrix.translation.into());
        if c.node_was.is_none() {
            c.node_was = c.node_at;
        }
        // The hit spheres follow their nodes.
        for (i, s) in c.spheres.iter().enumerate() {
            let Some(n) = c.kind.file.type_nodes(c.ty).get(i) else { continue };
            let m = bone_matrix(c.body().spheres[i]);
            if let Ok(mut t) = spheres.get_mut(*s) {
                t.translation = m.transform_point3(Vec3::from(n.offset));
            }
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
            deal(c, entity, &dmg, first, node_matrix, &heroes, level, &mut blows, &mut commands);
        }
        for (k, (s, at)) in mv.sounds.iter().enumerate() {
            let bit = 1 << k;
            if c.sounds_done & bit == 0 && *s >= 0 && i32::from(*at) <= frame {
                c.sounds_done |= bit;
                sound_chain(&c.kind.file, *s as usize, level.realm, &mut to_play);
            }
        }

        // Walk and turn.
        if boss {
            walk_leashed(c, &mv, &ty, level.speed_scale);
        } else {
            walk(c, &mv, &ty, &ground.0, &heroes, level.speed_scale);
        }
        turn(c, &mv, &heroes);
        if c.frozen > 0.0 {
            c.frozen = (c.frozen - DT).max(0.0);
        } else {
            c.clock.advance(DT);
        }
        c.stunned = (c.stunned - DT).max(0.0);
    }

    if level.intro != intro_before {
        info!("boss intro {intro_before} → {} at {now:.2} s", level.intro);
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
    // few ticks after the first critter event named by
    // `GDL_CRITTER_SHOT_ON` (`death`, the default; `missile`; `hurt`).
    let wanted = std::env::var("GDL_CRITTER_SHOT_ON").unwrap_or_else(|_| "death".into());
    if death_shot.is_none() && level.events.iter().any(|e| *e == wanted) {
        let delay = std::env::var("GDL_CRITTER_SHOT_DELAY").ok().and_then(|v| v.parse().ok());
        *death_shot = Some(delay.unwrap_or(SHOT_DELAY).max(1));
    }
    level.events.clear();
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

/// A sleeping boss wakes once its two seconds are up and every player it
/// tracks is within its wake distance (at once when that's 0). With the
/// type's flag 0x80 (the chimera) the game starts the two seconds only
/// when the boss stands on floor whose node has flag 0x10 in its byte
/// `+0x16`; that byte isn't read here, so they start at once (stand-in).
fn wake_boss(c: &mut Critter, ty: &TypeInfo, now: f32) -> bool {
    if c.wake_at == 0.0 {
        c.wake_at = now + BOSS_WAKE_DELAY;
        if ty.flags & TYPE_FLOOR_WAKE != 0 {
            debug!("boss wake timer started at once (stand-in for its floor's flag)");
        }
        return false;
    }
    if now < c.wake_at {
        return false;
    }
    let furthest = c.tracked.iter().map(|t| t.distance).fold(0.0f32, f32::max);
    let furthest = if furthest <= 0.0 { REJECTED } else { furthest };
    ty.wake_distance <= 0.0 || furthest < ty.wake_distance
}

/// The level's side of the boss intro, each tick before the critters move
/// (the game's level update): 99 once the boss is dead; in 2 a wait (from
/// the first tick) then 3; 4 goes on to 5 at once, noting the time; after
/// 29 s in 5 the djinn, P-boss, yeti, wraith and first skorne go on to 6
/// (the dragon, chimera, drider and lich stay in 5; a missile moves the
/// chimera on). The level darkens through 2–3.
fn update_intro(level: &mut CritterLevel) {
    let now = level.now;
    let was = level.intro;
    if level.boss_dead {
        level.intro = intro::OVER;
    }
    match level.intro {
        intro::ROARED => {
            level.intro = intro::AFTER;
            level.intro_timer = now;
        }
        intro::AFTER => {
            if now - level.intro_timer >= INTRO_AFTER && matches!(level.boss_type, DJINN | PBOSS | YETI | WRAITH | SKORNE) {
                level.intro = intro::FIGHT;
            }
        }
        intro::WAIT => {
            if level.intro_timer == 0.0 {
                let wait = if matches!(level.boss_type, CHIMERA | LICH | SKORNE) { INTRO_WAIT_SHORT } else { INTRO_WAIT };
                level.intro_timer = now + wait;
            } else if level.intro_timer <= now {
                level.intro = intro::ROAR;
                level.intro_timer = 0.0;
            }
            level.light.ask(now, DARKEN_HOLD, DARKEN_TO);
        }
        intro::ROAR => level.light.ask(now, DARKEN_HOLD, DARKEN_TO),
        _ => {}
    }
    level.light.step(now);
    if level.intro != was {
        info!("boss intro {was} → {} at {now:.2} s (light {:+.2})", level.intro, level.light_offset());
    }
}

/// What a missile hitting the boss does in its intro (the game checks it
/// where effects hit critters): in 2–3 it ends the intro for the dragon
/// (frozen) and the djinn (stunned), and stuns the P-boss; in 5 it starts
/// the chimera's fight. The P-boss's stun ends with its intro.
fn intro_reactions(c: &mut Critter, level: &mut CritterLevel) {
    if std::mem::take(&mut c.missile_hit)
        && let Some((next, freeze, stun)) = missile_reaction(level.intro, level.boss_type)
    {
        level.intro = next;
        if freeze > 0.0 {
            c.frozen = freeze;
        }
        if stun > 0.0 {
            c.stunned = stun;
        }
        info!("a missile hits the boss in its intro: now {next} (frozen {:.0} s, stunned {:.0} s)", c.frozen, c.stunned);
    }
    if level.boss_type == PBOSS && level.intro >= intro::FIGHT {
        c.stunned = 0.0;
    }
}

/// A missile hit in the intro: the next state, and the seconds it freezes
/// and stuns the critter hit.
fn missile_reaction(state: i32, boss_type: i32) -> Option<(i32, f32, f32)> {
    match (state, boss_type) {
        (intro::WAIT | intro::ROAR, DRAGON) => Some((intro::ROARED, DRAGON_FREEZE, 0.0)),
        (intro::WAIT | intro::ROAR, DJINN) => Some((intro::ROARED, 0.0, DJINN_STUN)),
        (intro::WAIT | intro::ROAR, PBOSS) => Some((state, 0.0, PBOSS_STUN)),
        (intro::AFTER, CHIMERA) => Some((intro::FIGHT, 0.0, 0.0)),
        _ => None,
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
    leash: f32,
    wake_distance: f32,
    flags: u32,
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
        leash: t.leash,
        wake_distance: t.wake_distance,
        flags: t.flags,
    }
}

/// The blows taken turn into knockback (not for bosses): the push × a
/// factor by the blows' kinds (less for the golem), capped.
fn knockback(c: &mut Critter) {
    let class = c.class();
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

/// Whether the critter's anger is outside a condition's range.
fn anger_fails(anger: f32, cond: &Condition) -> bool {
    anger < cond.min_anger || (cond.min_anger < cond.max_anger && cond.max_anger <= anger)
}

/// The game's target score for a point: the horizontal distance over the
/// cosine between the (turned) facing and the direction, or twice the
/// distance when that's 60° or more off; a failed condition scores 1e21 or
/// more.
fn score(c: &Critter, cond: &Condition, centre: [f32; 3], point: [f32; 3]) -> (f32, f32, [f32; 2]) {
    let (dx, dy, dz) = (point[0] - centre[0], point[1] - centre[1], point[2] - centre[2]);
    let distance = (dx * dx + dz * dz).sqrt();
    let dir = if distance > 0.0 { [dx / distance, dz / distance] } else { [0.0, 1.0] };
    if anger_fails(c.anger, cond) {
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

/// The players it tracks by its type's target condition, best first. A
/// golem keeps the best whatever it scores; a boss up to four that pass
/// (weighted 1: the game weighs damage dealt against taken, which only
/// matters with several players). Players a critter hit in the last
/// quarter second count a thousand times worse.
fn track(c: &mut Critter, ty: &TypeInfo, centre: [f32; 3], heroes: &[Hero], level: &CritterLevel) {
    let mut found: Vec<Tracked> = Vec::new();
    for h in heroes {
        let (mut s, distance, direction) = score(c, &ty.target, centre, h.feet);
        if level.guard.get(&h.entity).is_some_and(|&until| level.now < until) {
            s *= RECENTLY_HIT;
        }
        if ty.class == class::BOSS && s >= REJECTED {
            continue;
        }
        let centre = [h.feet[0], h.feet[1] + h.half, h.feet[2]];
        found.push(Tracked { player: h.entity, distance, direction, position: h.feet, score: s, weight: 1.0, centre });
    }
    found.sort_by(|a, b| a.score.total_cmp(&b.score));
    found.truncate(if ty.class == class::BOSS { MAX_TARGETS } else { 1 });
    c.tracked = found;
}

/// The tracked target that best meets `cond` (with `fallback`, the best one
/// anyway).
fn best_target(c: &Critter, cond: &Condition, fallback: bool) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for t in &c.tracked {
        let s = if anger_fails(c.anger, cond) {
            1.2e21
        } else if t.distance < cond.min_distance {
            1.01e21
        } else if cond.max_distance > 0.0 && cond.max_distance < t.distance {
            1.02e21
        } else {
            let h = c.yaw - cond.angle;
            if h.sin() * t.direction[0] + h.cos() * t.direction[1] < cond.min_cos { 1.1e21 } else { t.distance * t.weight }
        };
        if best.is_none_or(|(b, _)| s < b) {
            best = Some((s, t.player));
        }
    }
    match best {
        Some((s, p)) if s < REJECTED || fallback => Some(p),
        _ => None,
    }
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

/// The moves forced on it: INIT when new (a sleeping boss keeps playing
/// it), START after INIT, DEATH when dying, a move's follow-up; else, by
/// the boss intro's state, READY in 1–2, ROAR in 3, READY for the chimera
/// in 3–5, and otherwise a boss's READY after START; then reactions to the
/// blows taken.
fn forced(c: &mut Critter, ty: &TypeInfo, now: f32, intro: i32, boss_type: i32) {
    let cur = c.current.map(|i| c.moves()[i].clone());
    let chimera_waits = (intro::ROAR..=intro::AFTER).contains(&intro) && boss_type == CHIMERA;
    let next = match (&cur, c.state) {
        (None, _) | (_, CritterState::New) => find(c, kind::INIT, Find::Nearest, now),
        (Some(m), _) if m.kind == kind::INIT => find(c, kind::START, Find::Ready, now),
        (_, CritterState::Dying) => find(c, kind::DEATH, Find::Nearest, now),
        (Some(m), _) if m.next >= 0 => Some(m.next as usize),
        _ if (intro::START..=intro::WAIT).contains(&intro) => find(c, kind::READY, Find::Nearest, now),
        (Some(m), _) if intro == intro::ROAR && m.kind != kind::ROAR => find(c, kind::ROAR, Find::Nearest, now),
        _ if chimera_waits => find(c, kind::READY, Find::Ready, now),
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
    if c.next.is_none() && c.damage_taken >= ROAR_DAMAGE * ROAR_BY_PLAYERS[PLAYERS] {
        c.next = find(c, kind::ROAR, Find::Ready, now);
    }
    if c.next.is_none() && c.kinds & KIND_STRONG != 0 {
        c.next = find(c, kind::FLINCH, Find::Ready, now);
    }
    c.kinds &= !KIND_REACTIONS;
    if c.next.is_some() {
        // A forced move ends a pattern.
        c.pattern = None;
    }
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

/// The next step of a running pattern; else the least recently used
/// pattern or attack whose condition finds a target.
fn choose_attack(c: &mut Critter, now: f32) {
    let patterns = c.kind.file.type_patterns(c.ty).to_vec();
    if let Some((p, step)) = c.pattern
        && let Some(&m) = patterns[p].moves.get(step + 1)
        && m >= 0
    {
        c.next = Some(m as usize);
        return;
    }
    let mut best_time = 999_999.0f32;
    let mut pattern_pick: Option<usize> = None;
    for (i, p) in patterns.iter().enumerate() {
        if Some(i) == c.pattern.map(|p| p.0) || p.flags & 0x1000 != 0 || c.pattern_starts[i] + p.cooldown > now {
            continue;
        }
        let Some(t) = best_target(c, &p.condition, false) else { continue };
        if c.pattern_starts[i] < best_time {
            best_time = c.pattern_starts[i];
            pattern_pick = Some(i);
            c.pick = Some(t);
        }
    }
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
            pattern_pick = None;
            c.pick = Some(t);
        } else if pattern_pick.is_none() && chosen.is_none_or(|b| transition(&c.moves()[b], &m) > 1) {
            chosen = Some(i);
            c.pick = Some(t);
        }
    }
    match pattern_pick {
        Some(p) => {
            c.chosen_pattern = Some(p);
            c.next = usize::try_from(patterns[p].moves[0]).ok();
        }
        None => c.next = chosen,
    }
}

/// The movement move that scores its target best.
fn choose_movement(c: &mut Critter, centre: [f32; 3], now: f32) {
    let Some(t) = c.tracked.first().copied() else { return };
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

/// Switches to the chosen move the way the transition allows (not before
/// a boss's hold on its move runs out, unless the new one outranks
/// everything), and starts its animation; a switch records when the move
/// will end (its cooldown runs from then), and steps or starts a pattern.
/// Leaving START (without a follow-up) moves the boss intro 1 → 2, leaving
/// ROAR 3 → 4.
fn switch(c: &mut Critter, now: f32, animator: &mut Animator, intro: &mut i32) {
    let cur = c.current.map(|i| c.moves()[i].clone());
    let next = c.next;
    let (target, mode) = match (&cur, next) {
        (_, None) => (c.current, 0u8),
        (None, Some(n)) => (Some(n), 3),
        (Some(m), Some(n)) => {
            let nm = &c.moves()[n];
            if Some(n) != c.current && nm.priority >= 0xF00 && m.transition != 0 {
                (Some(n), 3)
            } else if c.hold_until > now {
                (c.current, 0)
            } else {
                (Some(n), transition(m, nm))
            }
        }
    };
    let mode = if target != c.current && mode == 0 && c.hold_until <= now && c.clock.ended { 1 } else { mode };
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
    if let Some(m) = &cur {
        if m.kind == kind::ROAR && *intro == intro::ROAR {
            *intro = intro::ROARED;
        } else if m.kind == kind::START && m.next < 0 && *intro == intro::START {
            *intro = intro::WAIT;
        }
    }
    let clip = c.body().clips.get(action).copied().unwrap_or((1, 30, false));
    c.clock.start(action, clip);
    animator.play(action);
    c.switched = true;
    c.current = Some(t);
    c.hold_until = 0.0;
    // Patterns: a new one starts; a running one steps on (or ends).
    if let Some(p) = c.chosen_pattern.take() {
        c.pattern_starts[p] = now;
        c.pattern = Some((p, 0));
    } else if let Some((p, step)) = c.pattern {
        let patterns = c.kind.file.type_patterns(c.ty);
        let step = step + 1;
        c.pattern = patterns[p].moves.get(step).filter(|&&m| m >= 0 && m as usize == t).map(|_| (p, step));
    } else {
        c.ends[t] = now + FRAME_TIME * (f32::from(clip.0) - 2.0);
    }
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

/// A blow, by its `DAMG` kind:
///
/// - 0: a sphere on the move's node, swept from where it was last tick,
///   against each player's cylinder;
/// - 1, 2, 8: a missile from the node (its offset in the critter's space),
///   aimed with an arc at its target (1: flag 1) or along its facing, as
///   fast as the critter's anger picks from the speed range, turned by the
///   blow's yaw and spread;
/// - 3: a ring on the ground: players within its radius (stand-in for the
///   game's damaging effect, at once);
/// - 4: a breath cone along the node's forward axis (turned by its yaw
///   and pitch): players between its reach and length, within its
///   thickness.
///
/// Players a critter hit can't be hit again for a quarter second.
#[allow(clippy::too_many_arguments)]
fn deal(
    c: &mut Critter,
    me: Entity,
    d: &CritterDamage,
    first: bool,
    node: Affine3A,
    heroes: &[Hero],
    level: &mut CritterLevel,
    blows: &mut Vec<Blow>,
    commands: &mut Commands,
) {
    let damage = d.damage * level.damage_scale;
    if matches!(d.kind, 1 | 2 | 8) {
        if first {
            launch(c, me, d, damage, level, commands);
            level.events.push("missile");
        }
        return;
    }
    let offset = node.matrix3 * Vec3::from(d.offset);
    let at = Vec3::from(c.node_at.unwrap_or(c.position)) + offset;
    let was = Vec3::from(c.node_was.unwrap_or(c.position)) + offset;
    // The breath's direction: the node's forward axis, turned.
    let forward = turn_dir(Vec3::from(node.matrix3.z_axis).normalize_or_zero(), d.yaw, d.pitch);
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
            4 => {
                let v = centre - at;
                let across = Vec2::new(v.x, v.z).length();
                across >= d.min_range
                    && across <= d.radius
                    && cylinder_hit(at, at + forward * d.radius, centre, h.radius + d.life, h.half + d.life).is_some()
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
    if first && !matches!(d.kind, 0 | 3 | 4) {
        debug!("critter {me:?}: damage kind {} not done yet", d.kind);
    }
}

/// Turns a direction by `yaw` about the vertical and tilts it by `pitch`
/// about the horizontal axis across it.
fn turn_dir(dir: Vec3, yaw: f32, pitch: f32) -> Vec3 {
    let d = Quat::from_rotation_y(yaw) * dir;
    let across = Vec3::new(d.z, 0.0, -d.x).normalize_or_zero();
    if across == Vec3::ZERO { d } else { Quat::from_axis_angle(across, pitch) * d }
}

/// The launch direction that brings a missile of `speed` under `gravity`
/// from `from` to `to` on the flatter arc (straight at it when it can't
/// reach).
fn lob_toward(from: Vec3, to: Vec3, speed: f32, gravity: f32) -> Vec3 {
    let d = to - from;
    let flat = Vec2::new(d.x, d.z);
    let h = flat.length();
    if gravity <= 0.0 || h < 1e-3 || speed <= 0.0 {
        return d.normalize_or_zero();
    }
    let s2 = speed * speed;
    let disc = s2 * s2 - gravity * (gravity * h * h + 2.0 * d.y * s2);
    if disc < 0.0 {
        return d.normalize_or_zero();
    }
    let angle = ((s2 - disc.sqrt()) / (gravity * h)).atan();
    let f = flat / h;
    Vec3::new(f.x * angle.cos(), angle.sin(), f.y * angle.cos())
}

/// A missile: from the move's node, at the critter's target (flag 1) or
/// along its facing, as fast as its anger picks from the speed range, with
/// its own effect model.
fn launch(c: &Critter, me: Entity, d: &CritterDamage, damage: f32, level: &mut CritterLevel, commands: &mut Commands) {
    let (s, co) = c.yaw.sin_cos();
    let o = d.offset;
    let offset = Vec3::new(o[0] * co + o[2] * s, o[1], -o[0] * s + o[2] * co);
    let start = Vec3::from(c.node_at.unwrap_or(c.position)) + offset;
    let t = ((c.anger.clamp(ANGER_BASE, ANGER_CAP) - ANGER_BASE) * SPEED_SPAN).min(1.0);
    let speed = d.speed[0] + t * (d.speed[1] - d.speed[0]);
    if speed <= 0.0 {
        debug!("critter {me:?}: a still effect (not done)");
        return;
    }
    let forward = Vec3::new(s, 0.0, co);
    let target = c.move_target.and_then(|p| c.tracked.iter().find(|t| t.player == p));
    let mut dir = match target {
        Some(t) if d.flags & 1 != 0 => {
            let to = Vec3::from(t.centre);
            if d.flags & 8 == 0 { lob_toward(start, to, speed, d.gravity) } else { (to - start).normalize_or_zero() }
        }
        _ if d.flags & 4 != 0 => forward,
        _ => Vec3::new(forward.x, UNAIMED_DROP, forward.z).normalize_or_zero(),
    };
    if d.kind == 1 {
        let spread = if d.spread > 0.0 { (level.random() - 0.5) * d.spread } else { 0.0 };
        dir = turn_dir(dir, d.yaw + spread, if d.flags & 8 != 0 { d.pitch } else { 0.0 });
    }
    let model = usize::try_from(d.effects[0]).ok().and_then(|e| c.kind.effects.get(&e)).map(|m| m.as_ref());
    info!("critter {me:?} launches a missile: {damage:.0} damage at {speed:.0}/s from {start:?}");
    let size = d.life.max(0.1);
    let e = spawn_critter_missile(commands, model, me, start, dir * speed, d.gravity, d.radius, damage, d.blow, size);
    // Stand-in glow (half its hit radius), since the effect model draws
    // nothing without the effects system.
    let (mesh, material) = level.glow.clone();
    let glow = Transform::from_scale(Vec3::splat(0.5 * d.radius / size));
    commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), glow, ChildOf(e)));
}

/// Walks at the move's speed (× the level's monster speed) in its
/// direction, plus the knockback, on the level's collision; stops short of
/// players.
fn walk(c: &mut Critter, m: &CritterMove, ty: &TypeInfo, collision: &LevelCollision, heroes: &[Hero], speed_scale: f32) {
    let s = m.speed * speed_scale * DT;
    let (dx, dz) = move_direction(c.yaw, m.kind, s);
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

/// A move's ground step for a critter facing `yaw`: forward, back, to
/// either side or diagonally.
fn move_direction(yaw: f32, k: i32, s: f32) -> (f32, f32) {
    let f = [yaw.sin(), yaw.cos()];
    match k {
        kind::BACK => (-s * f[0], -s * f[1]),
        kind::WALK_RIGHT => (s * f[1], -s * f[0]),
        kind::WALK_LEFT => (-s * f[1], s * f[0]),
        kind::WALK_DIAGONAL => (-s * f[1] + s * f[0], s * f[1] + s * f[0]),
        _ => (s * f[0], s * f[1]),
    }
}

/// A boss walks without the level's collision, kept within its leash of
/// home (a box with the type's flag), at its hover height.
fn walk_leashed(c: &mut Critter, m: &CritterMove, ty: &TypeInfo, speed_scale: f32) {
    if ty.leash <= 0.0 {
        return;
    }
    let (dx, dz) = move_direction(c.yaw, m.kind, m.speed * speed_scale * DT);
    let mut off = [c.position[0] + dx - c.home[0], c.position[2] + dz - c.home[2]];
    let dist = (off[0] * off[0] + off[1] * off[1]).sqrt();
    if ty.flags & TYPE_BOX_LEASH != 0 {
        off = [off[0].clamp(-ty.leash, ty.leash), off[1].clamp(-ty.leash, ty.leash)];
    } else if dist > ty.leash {
        off = [off[0] * ty.leash / dist, off[1] * ty.leash / dist];
    }
    c.position = [c.home[0] + off[0], c.position[1], c.home[2] + off[1]];
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
            if ty.flags & TYPE_FREE_TURN != 0 {
                g
            } else {
                let d = locomotion::wrap(g - c.home_yaw).clamp(-ty.max_turn, ty.max_turn);
                c.home_yaw + d
            }
        })
    };
    let Some(goal) = goal else { return };
    let rate = m.turn * DT * if c.stunned > 0.0 { STUNNED_TURN } else { 1.0 };
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
        // Four frames at rate 30 (30 a second): over half a frame after the
        // last one comes up.
        let mut c = Clock { action: 0, frame: 0.0, frames: 1, rate: 30, loops: false, ended: true };
        c.start(1, (4, 30, false));
        assert!(!c.ended);
        for _ in 0..3 {
            c.advance(DT);
        }
        assert!(!c.ended && c.frame == 3.0);
        c.advance(DT);
        assert!(c.ended && c.frame == 3.5);
        // Rate 60 is 15 frames a second: four frames take seven ticks.
        c.start(1, (4, 60, false));
        let mut ticks = 0;
        while !c.ended {
            c.advance(DT);
            ticks += 1;
        }
        assert_eq!(ticks, 7);
        // A loop has ended on the tick it starts over.
        c.start(2, (4, 30, true));
        for _ in 0..3 {
            c.advance(DT);
        }
        assert!(!c.ended);
        c.advance(DT);
        assert!(c.ended && c.frame < 1.0);
        c.advance(DT);
        assert!(!c.ended);
    }

    #[test]
    fn lobs_land_on_the_target() {
        let (from, to) = (Vec3::ZERO, Vec3::new(30.0, -10.0, 0.0));
        let (speed, gravity) = (40.0, 30.0);
        let v = lob_toward(from, to, speed, gravity) * speed;
        // Where it is when it has come 30 across.
        let t = 30.0 / v.x;
        let y = v.y * t - 0.5 * gravity * t * t;
        assert!((y - to.y).abs() < 1e-3, "{y}");
        // Too far to reach: straight at it.
        assert_eq!(lob_toward(from, Vec3::new(1000.0, 0.0, 0.0), 10.0, 30.0), Vec3::X);
    }

    fn level_with(boss_type: i32, state: i32) -> CritterLevel {
        CritterLevel {
            kinds: HashMap::new(),
            statues: Vec::new(),
            now: 0.0,
            realm: 'B',
            hit_point_scale: 1.0,
            speed_scale: 1.0,
            damage_scale: 1.0,
            guard: HashMap::new(),
            boss: None,
            boss_dead: false,
            boss_type,
            realm_id: 2,
            intro: state,
            intro_timer: 0.0,
            legendary_used: false,
            light: Darkening::default(),
            boss_spot: [0.0; 3],
            end: EndModels::default(),
            victory: None,
            events: Vec::new(),
            glow: (Handle::default(), Handle::default()),
            rng: 1,
        }
    }

    /// Ticks the level's side of the intro until the state changes (or a
    /// minute passes): the seconds it took.
    fn until_change(l: &mut CritterLevel) -> f32 {
        let (from, start) = (l.intro, l.now);
        while l.intro == from && l.now - start < 60.0 {
            l.now += DT;
            update_intro(l);
        }
        l.now - start
    }

    #[test]
    fn the_intro_waits_then_roars_in_the_dark() {
        let mut l = level_with(DRAGON, intro::WAIT);
        let waited = until_change(&mut l);
        assert_eq!(l.intro, intro::ROAR);
        // The timer is set on the first tick: 3 s and a tick or two.
        assert!(waited > INTRO_WAIT && waited < INTRO_WAIT + 2.5 * DT, "{waited}");
        assert!((l.light_offset() - DARKEN_TO).abs() < 1e-5);
        let mut chimera = level_with(CHIMERA, intro::WAIT);
        let waited = until_change(&mut chimera);
        assert!(waited > INTRO_WAIT_SHORT && waited < INTRO_WAIT_SHORT + 2.5 * DT, "{waited}");
    }

    #[test]
    fn after_the_roar_some_bosses_settle_into_the_fight() {
        let mut l = level_with(DJINN, intro::ROARED);
        until_change(&mut l);
        assert_eq!(l.intro, intro::AFTER);
        let settled = until_change(&mut l);
        assert_eq!(l.intro, intro::FIGHT);
        assert!((settled - INTRO_AFTER).abs() < 0.05, "{settled}");
        // The dragon stays in 5, its light back to normal, until it dies.
        let mut l = level_with(DRAGON, intro::ROARED);
        until_change(&mut l);
        until_change(&mut l);
        assert_eq!(l.intro, intro::AFTER);
        assert_eq!(l.light_offset(), 0.0);
        l.boss_dead = true;
        until_change(&mut l);
        assert_eq!(l.intro, intro::OVER);
    }

    #[test]
    fn missiles_cut_some_intros_short() {
        assert_eq!(missile_reaction(intro::WAIT, DRAGON), Some((intro::ROARED, DRAGON_FREEZE, 0.0)));
        assert_eq!(missile_reaction(intro::ROAR, DJINN), Some((intro::ROARED, 0.0, DJINN_STUN)));
        assert_eq!(missile_reaction(intro::ROAR, PBOSS), Some((intro::ROAR, 0.0, PBOSS_STUN)));
        assert_eq!(missile_reaction(intro::AFTER, CHIMERA), Some((intro::FIGHT, 0.0, 0.0)));
        assert_eq!(missile_reaction(intro::AFTER, DRAGON), None);
        assert_eq!(missile_reaction(intro::START, CHIMERA), None);
    }

    #[test]
    fn the_light_darkens_fast_and_comes_back_slowly() {
        let mut d = Darkening::default();
        let mut now = 0.0;
        for _ in 0..4 {
            now += DT;
            d.ask(now, DARKEN_HOLD, DARKEN_TO);
            d.step(now);
        }
        assert!((d.offset - DARKEN_TO).abs() < 1e-5, "{d:?}");
        let mut ticks = 0;
        while d.offset < 0.0 && ticks < 100 {
            now += DT;
            d.step(now);
            ticks += 1;
        }
        // At most 0.05 a tick back up.
        assert!((16..30).contains(&ticks), "{ticks}");
    }

    #[test]
    fn turned_directions_keep_their_length() {
        let d = turn_dir(Vec3::Z, std::f32::consts::FRAC_PI_2, 0.0);
        assert!((d - Vec3::X).length() < 1e-5, "{d}");
        let d = turn_dir(Vec3::Z, 0.0, 0.3);
        assert!((d.length() - 1.0).abs() < 1e-5 && d.y.abs() > 0.2);
    }
}
