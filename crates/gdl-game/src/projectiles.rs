//! Missiles: the weapon a hero throws or shoots when a throw or strafe
//! attack ends, and the arrows, bombs and fireballs the throwing monsters
//! let go of (`docs/projectiles.md` has the game's code and tables behind
//! every number here).
//!
//! A missile leaves with a velocity lobbed to come down a set distance
//! ahead (heroes: 15–35 units by how long the throw was wound up; monsters:
//! at the player), falls under its type's gravity and flies for at most
//! three seconds. Each 30 Hz tick it sweeps the segment it moved along
//! against, in the game's order, the players (monster missiles), the
//! monsters (hero missiles), generators and breakables (hero missiles) and
//! the level's collision, and stops at the first thing it meets. Bombs then
//! burst, hurting everything their growing blast reaches the less the
//! further out it is. Monsters are hurt through [`Hit`] messages, players
//! through [`DamagePlayer`].
//!
//! [`HeroShot`] and [`MonsterShot`] are how `player.rs` and `monsters.rs`
//! ask for a missile.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::anim::AnimFile;
use gdl_formats::collision::{self, node_flags};
use gdl_formats::pdata::PlayerStats;
use gdl_formats::{LevelCollision, ModelFile, enemy};

use crate::actions::Strike;
use crate::audio::PlaySound;
use crate::character::{self, CharacterData, CharacterModel};
use crate::combat::{self, CritterAim, Hit, TargetKind, Targetable};
use crate::effects::{BlastAt, PotionBurst, StrikePotion, is_floor_potion};
use crate::items::LevelItems;
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::monsters::{Monster, MonsterLevel, MonsterTick};
use crate::player::{Player, PlayerChoice};
use crate::player_state::{DamagePlayer, EnemyScale, PlayerState, SpendPower, power};
use crate::population::LevelPopulation;
use crate::world::{LevelEntity, LevelGround};

pub struct ProjectilesPlugin;

impl Plugin for ProjectilesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<HeroShot>()
            .add_message::<MonsterShot>()
            .init_resource::<MissileModels>()
            .init_resource::<PlayerGuard>()
            .add_systems(FixedUpdate, (launch_hero, launch_monster, fly).chain().after(MonsterTick))
            .add_systems(
                Update,
                (setup_level.run_if(resource_exists_and_changed::<LevelPopulation>), interpolate).chain(),
            );
        if let Some(spec) = ThrowerSpec::from_env() {
            app.insert_resource(spec).add_systems(FixedUpdate, spawn_thrower.before(MonsterTick));
        }
    }
}

/// `GDL_THROWER=<distance>[,<ai>[,<tier>]]`: a grunt (the level must load
/// grunts at that tier; default tier 4, AI 0x17) placed that far in front
/// of the hero at each level start, facing it — to watch the throwing AIs.
#[derive(Resource, Clone, Copy)]
struct ThrowerSpec {
    distance: f32,
    ai: i16,
    tier: i32,
    /// The level it was placed on.
    done: Option<Entity>,
}

impl ThrowerSpec {
    fn from_env() -> Option<Self> {
        let spec = std::env::var("GDL_THROWER").ok()?;
        let mut parts = spec.split(',').map(str::trim);
        let distance = parts.next()?.parse().ok()?;
        let int = |s: &str| i64::from_str_radix(s.trim_start_matches("0x"), if s.starts_with("0x") { 16 } else { 10 }).ok();
        let ai = parts.next().and_then(int).unwrap_or(0x17) as i16;
        let tier = parts.next().and_then(int).unwrap_or(4) as i32;
        Some(Self { distance, ai, tier, done: None })
    }
}

fn spawn_thrower(
    mut commands: Commands,
    mut spec: ResMut<ThrowerSpec>,
    level: Option<ResMut<MonsterLevel>>,
    ground: Option<Res<LevelGround>>,
    players: Query<(Entity, &Player)>,
) {
    let (Some(mut level), Ok((hero, player))) = (level, players.single()) else { return };
    if spec.done == Some(hero) {
        return;
    }
    spec.done = Some(hero);
    let (feet, facing) = (Vec3::from(player.start.0), player.start.1);
    let mut at = feet + Vec3::new(facing.sin(), 0.0, facing.cos()) * spec.distance;
    if let Some(y) = ground.as_ref().and_then(|g| g.0.floor_height((at + Vec3::Y * 2.0).to_array())) {
        at.y = y;
    }
    let new = crate::monsters::NewMonster {
        enemy: 4,
        tier: spec.tier,
        ai: spec.ai,
        position: at.to_array(),
        facing: locomotion::wrap(facing + PI),
        generator: None,
        placed: true,
        awareness: None,
        freeze: 0.0,
        throw_rate: 1.0,
    };
    let made = crate::monsters::spawn_monster(&mut level, new, &mut commands);
    info!("thrower (AI {:#x}, tier {}) at {at:?}: {made:?}", spec.ai, spec.tier);
}

/// A player's collision centre above its feet (the class record's centre
/// offset; the same for every class).
pub const PLAYER_CENTRE: f32 = 2.5;
/// A player's half height for missiles (half the class record's height).
const PLAYER_HALF_HEIGHT: f32 = 2.5;
/// Missiles vanish after this many seconds (two for the multi-shot
/// spreads).
const LIFETIME: f32 = 3.0;
const SPREAD_LIFETIME: f32 = 2.0;

/// Weapon power bits the hero's missiles act on: three and five missiles
/// fanned out, piercing (the crossbow), bouncing off walls (reflect).
pub mod shot_kind {
    pub const MULTI: u32 = 0x8_0000;
    pub const MULTI5: u32 = 0x40_0000;
    pub const PIERCE: u32 = 0x10_0000;
    pub const BOUNCE: u32 = 0x20_0000;
    /// The power throw.
    pub const POWER: u32 = 0x200_0000;
}

/// The spread's turns (cosine, sine): straight, ±15°, ±30° (the game's
/// two tables of them, `docs/projectiles.md`). Three missiles with the
/// multi-shot, five with the five-way one.
const SPREAD: [(f32, f32); 5] = [(1.0, 0.0), (0.966, 0.259), (0.966, -0.259), (0.866, 0.5), (0.866, -0.5)];

/// How many missiles a throw of `kind` makes.
fn spread_count(kind: u32) -> usize {
    if kind & shot_kind::MULTI5 != 0 {
        5
    } else if kind & shot_kind::MULTI != 0 {
        3
    } else {
        1
    }
}

/// A crossbow power's throws fly as bolts (the game's special missile
/// record): straight, pointing along their flight, a radius of 5, hitting
/// heavily (`0x20`).
const BOLT: MissileType = missile(0x20, 0.0, 0.0, 5.0, 0.0, [0.0; 3], 0.0);
/// Skorne's gauntlets' shots (the other two special records): the left
/// one's lightning (2), the right one's acid (4), a radius of 2, no fall
/// or spin.
const LIGHTNING_SHOT: MissileType = missile(2, 0.0, 0.0, 2.0, 0.0, [0.0; 3], 0.0);
const ACID_SHOT: MissileType = missile(4, 0.0, 0.0, 2.0, 0.0, [0.0; 3], 0.0);
/// The gauntlets' special bits: every throw while one is worn is its shot.
const LEFT_GAUNTLET: u32 = 0x8000;
const RIGHT_GAUNTLET: u32 = 0x4000;
/// A crossbow bolt with a use of the power does twice the damage (1.5
/// times on a boss level).
const BOLT_DAMAGE: f32 = 2.0;
const BOSS_BOLT_DAMAGE: f32 = 1.5;
/// A missile doing more than its damage is drawn that much bigger, and
/// 1.2 times again above level 98.
const TOP_LEVEL: u32 = 98;
const TOP_LEVEL_SIZE: f32 = 1.2;
/// A bounce keeps this much of any upward speed.
const BOUNCE_RISE: f32 = 0.4;
/// A throw wound up for longer than this goes further, up to 0.1 s more.
const WIND_UP: f32 = 0.27;
const WIND_UP_MAX: f32 = 0.1;
/// The power throw always counts as wound up this long.
const POWER_WIND_UP: f32 = 0.06;
/// A throw comes down 15 units ahead, plus 200 per second of wind-up.
const REACH: f32 = 15.0;
const REACH_PER_SECOND: f32 = 200.0;
/// ...and 0.5 below where it was let go.
const REACH_DROP: f32 = -0.5;
/// A hero's missile starts this far ahead of the hand; the wall check
/// before it's let go starts this far behind.
const START_AHEAD: f32 = 2.0;
const CHECK_BEHIND: f32 = 3.0;
/// A hero only aims at a target within 30° of the facing, and never
/// steeper than 60° up.
const AIM_CONE: f32 = 0.866;
/// Hero missiles are sped up by the throwing stat: 20–60 units a second.
const SPEED_MIN: f32 = 20.0;
const SPEED_MAX: f32 = 60.0;
/// The power throw's missile is 1.8 times the size (and drawn twice as big).
const POWER_SIZE: f32 = 1.8;
/// Once a missile hurts a player for more than 2, that player can't be hurt
/// by missiles for a quarter of a second.
const PLAYER_GUARD: f32 = 0.25;
const GUARD_ABOVE: f32 = 2.0;
/// The wall test uses half the missile's radius.
const WALL_RADIUS: f32 = 0.5;
/// Below this far under the level's lowest point a missile is gone.
const BELOW_LEVEL: f32 = 25.0;
/// A blast grows from a third of its radius to all of it while its damage
/// falls from full to nothing.
const BLAST_START: f32 = 0.33;
const BLAST_FALLOFF: f32 = 1.5;
/// Monster missiles: aimed up to ±2.5 units (× the level's spread) off,
/// arrows 3.5 and bombs 5.5 lower; let go 2.5 above the monster's centre
/// (fireballs from it) and 3 units ahead; only when the target is within
/// 45° of the monster's facing.
const MONSTER_SPREAD: f32 = 2.5;
const ARROW_DROP: f32 = -3.5;
const BOMB_DROP: f32 = -5.5;
const MONSTER_LIFT: f32 = 2.5;
const MONSTER_AHEAD: f32 = 3.0;
const MONSTER_CONE: f32 = 0.707;

/// One missile type: the game's `0x30`-byte missile records.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MissileType {
    /// Damage kind bits it hits with (the low four: its element).
    pub kind: u32,
    /// Monster missiles: the damage and speed they're thrown with.
    pub damage: f32,
    pub speed: f32,
    pub radius: f32,
    /// A bomb's blast radius (0: no blast).
    pub blast: f32,
    /// Turns per second about its own X, Y, Z (radians); none: it points
    /// along its flight.
    pub spin: [f32; 3],
    /// Downward acceleration, units/s².
    pub gravity: f32,
}

const SPIN: f32 = 6.0 * PI;

const fn missile(kind: u32, damage: f32, speed: f32, radius: f32, blast: f32, spin: [f32; 3], gravity: f32) -> MissileType {
    MissileType { kind, damage, speed, radius, blast, spin, gravity }
}

/// The heroes' missiles, by class (the secret classes mirror the first
/// eight). The warrior's axe, valkyrie's sword, dwarf's hammer and
/// knight's mace tumble end over end; arrows, bolts and bombs don't.
pub const HERO_MISSILES: [MissileType; 8] = [
    missile(0, 0.0, 0.0, 1.0, 0.0, [SPIN, 0.0, 0.0], 12.0),
    missile(0, 0.0, 0.0, 1.0, 0.0, [SPIN, 0.0, 0.0], 8.0),
    missile(0, 0.0, 0.0, 1.2, 0.0, [0.0; 3], 8.0),
    missile(0, 0.0, 0.0, 0.7, 0.0, [0.0; 3], 8.0),
    missile(0, 0.0, 0.0, 1.0, 0.0, [SPIN, 0.0, 0.0], 20.0),
    missile(0, 0.0, 0.0, 1.0, 0.0, [SPIN, 0.0, 0.0], 8.0),
    missile(0, 0.0, 0.0, 1.2, 0.0, [0.0; 3], 8.0),
    missile(0, 0.0, 0.0, 0.7, 2.0, [0.0; 3], 8.0),
];

/// The three things a monster can throw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissileKind {
    Arrow,
    Bomb,
    Fireball,
}

impl MissileKind {
    /// What an AI throws: 0x10 and 0x17 arrows, 0x11 and 0x1A bombs,
    /// anything else a fireball.
    pub fn of_ai(ai: i16) -> Self {
        match ai {
            0x10 | 0x17 => MissileKind::Arrow,
            0x11 | 0x1A => MissileKind::Bomb,
            _ => MissileKind::Fireball,
        }
    }

    /// The second half of the model's atree name (`GRU_ARROW`).
    fn name(self) -> &'static str {
        match self {
            MissileKind::Arrow => "ARROW",
            MissileKind::Bomb => "BOMB",
            MissileKind::Fireball => "FBALL",
        }
    }
}

/// The AIs that stand (or back off) and throw instead of walking into the
/// player: 0x10, 0x11, 0x17, 0x1A.
pub fn throws(ai: i16) -> bool {
    matches!(ai, 0x10 | 0x11 | 0x17 | 0x1A)
}

const ARROW: MissileType = missile(0, 10.0, 25.0, 0.5, 0.0, [0.0; 3], 30.0);
const BOMB: MissileType = missile(0x10, 10.0, 20.0, 0.2, 3.0, [0.0, 1.0, 0.0], 35.0);

/// A monster type's missile of a kind, if it has one.
pub fn monster_missile(enemy: i32, kind: MissileKind) -> Option<MissileType> {
    use MissileKind::*;
    Some(match (enemy, kind) {
        // tro gru sor liz zom pla ice ske imp war
        (1 | 4 | 7 | 10 | 13 | 14 | 16 | 19 | 23 | 24, Arrow) => ARROW,
        (1 | 4 | 7 | 10 | 13 | 14 | 16 | 19 | 23 | 24, Bomb) => BOMB,
        // Worms spit all three, slowly and nearly straight.
        (17, Arrow) => missile(0, 5.0, 15.0, 0.3, 0.0, [0.0; 3], 1.0),
        (17, Bomb) => missile(0, 10.0, 20.0, 0.3, 0.0, [0.0; 3], 1.0),
        (17, Fireball) => missile(0, 15.0, 25.0, 0.3, 0.0, [0.0; 3], 1.0),
        // dem, gho; pla
        (2 | 20, Fireball) => missile(1, 15.0, 25.0, 0.3, 0.0, [0.0; 3], 1.0),
        (14, Fireball) => missile(0, 15.0, 25.0, 0.3, 0.0, [0.0; 3], 1.0),
        // sor, war
        (7 | 24, Fireball) => missile(3, 20.0, 20.0, 0.2, 0.0, [0.0; 3], 1.0),
        // grm
        (27, Fireball) => missile(2, 25.0, 80.0, 2.0, 0.0, [0.0; 3], 0.0),
        _ => return None,
    })
}

/// The hero's thrown weapon for a class index: its name in the weapon
/// table and, by player level ÷ 10, which of its models it uses (0: the
/// class's own `…_THROW0`; otherwise `…_THROW<n>` among its effects).
const WEAPONS: [(&str, &[u8; 10]); 10] = [
    ("AXE", b"0000000000"),
    ("SWD", b"0000000000"),
    ("STF", b"1112223333"),
    ("BOW", b"1112223333"),
    ("HAM", b"0000000000"),
    ("MAC", b"0000000000"),
    ("WND", b"1112223333"),
    ("BOM", b"1111112233"),
    ("MIN", b"1111111111"),
    ("FAL", b"1111111111"),
];

/// Missile type index for a class: the secret classes (8 and up) share the
/// class record values of the class eight before them, and this table too
/// (stand-in: the game's per-class index for them isn't traced).
fn missile_class(class: usize) -> usize {
    class % HERO_MISSILES.len()
}

/// A hero let go of a throw (`player.rs`, when a throw or strafe attack
/// hands over to the next action).
#[derive(Message, Clone, Copy, Debug)]
pub struct HeroShot {
    pub hero: Entity,
    pub feet: Vec3,
    pub facing: f32,
    /// Toward what the search found (unit vector), or where the hero is
    /// heading when it found nothing or is strafing.
    pub aim: Vec3,
    /// Whether the search found a target at all.
    pub targeted: bool,
    /// The throw and strafe bits (`Strike::SHOT`, `Strike::POWER_THROW`).
    pub strike: Strike,
    /// Seconds since the attack began.
    pub wound_up: f32,
}

/// A throwing monster let go of its missile (`monsters.rs`).
#[derive(Message, Clone, Copy, Debug)]
pub struct MonsterShot {
    pub monster: Entity,
    pub enemy: i32,
    pub ai: i16,
    /// The monster's centre, and the point it throws at.
    pub from: Vec3,
    pub at: Vec3,
    pub facing: f32,
    /// 0..1, for the aim's spread.
    pub random: f32,
}

/// Who threw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Hero(Entity),
    Monster(Entity),
}

impl Owner {
    fn entity(self) -> Entity {
        match self {
            Owner::Hero(e) | Owner::Monster(e) => e,
        }
    }
}

/// Armour bits that turn monster missiles back (the reflect shield;
/// `0x1000000`, which no power-up sets, does too).
const REFLECTS: u32 = 0x102_0000;
/// Shrunk (the shrink power), monsters' missiles do this much of their
/// damage.
const SHRUNK_MISSILE: f32 = 0.5;
/// A reflected missile does at most this.
const REFLECTED_MOST: f32 = 15.0;
/// The ricochet sound, at most this often (seconds).
const RICOCHET: &str = "S_RICOCHET";
const RICOCHET_EVERY: f64 = 1.0;

/// A missile in flight.
#[derive(Component, Debug)]
pub struct Projectile {
    pub owner: Owner,
    pub position: Vec3,
    previous: Vec3,
    pub velocity: Vec3,
    pub gravity: f32,
    pub radius: f32,
    pub damage: f32,
    pub kind: u32,
    pub blast: f32,
    spin: [f32; 3],
    /// Heading it left along (for the spinning ones).
    yaw: f32,
    age: f32,
    /// Seconds it flies before it's gone (a missile with a blast bursts
    /// just before).
    lifetime: f32,
    scale: f32,
    /// A thrown potion: the magic blast it becomes where it lands.
    pub potion: Option<PotionBurst>,
    /// What a piercing missile has hit already (it hits each once).
    pierced: Vec<Entity>,
    /// The hero whose reflect shield turned it back: it flies on at
    /// monsters (and past that hero).
    reflected: Option<Entity>,
}

/// What a missile was let go with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    /// Where the wall check before the release starts, and where the
    /// missile starts.
    pub check: Vec3,
    pub start: Vec3,
    pub velocity: Vec3,
}

/// The game's aim for a hero's throw: toward the target when it's within
/// 30° of the facing (a little steeper up, half as steep down, and never
/// flatter than the floor's slope), else straight along the facing tilted
/// by the floor's slope — always, for a `bolt` (kind `0x100000`).
/// `elevation` is the sine of the floor's slope along the facing (the
/// dwarf throws 0.2 higher).
pub fn hero_aim(facing: f32, aim: Vec3, targeted: bool, elevation: f32, dwarf: bool, bolt: bool) -> Vec3 {
    let mut e = elevation.min(0.707);
    if dwarf {
        e += 0.2;
    }
    let f = Vec3::new(facing.sin(), 0.0, facing.cos());
    let mut a = aim;
    let across = Vec2::new(a.x, a.z).length();
    if bolt || f.x * a.x + f.z * a.z < AIM_CONE * across {
        a = Vec3::new(f.x, e, f.z);
    } else if !targeted {
        let s = (1.0 - e * e).max(0.0).sqrt();
        a = Vec3::new(a.x * s, e, a.z * s);
    } else {
        if a.y > 0.0 {
            a.y *= 1.2;
        }
        if a.y.abs() < e.abs() {
            a.y = 0.5 * (a.y + e);
        }
    }
    if a.y < 0.0 {
        a.y *= 0.5;
    }
    let a = a.normalize_or(f);
    if a.y > AIM_CONE { f } else { a }
}

/// A lob: a direction with unit horizontal part and the slope that, at
/// `speed` along the ground, arrives `rise` above the start `across`
/// horizontal units away (`to` gives the direction) under `gravity`.
pub fn lob(to: Vec2, rise: f32, speed: f32, gravity: f32) -> Vec3 {
    let across = to.length();
    let s = if across <= 0.001 { 1.0 } else { 1.0 / across };
    let slope = (0.5 * gravity * across / speed + rise * speed * s) / speed;
    Vec3::new(to.x * s, slope, to.y * s)
}

/// How far ahead a throw comes down: 15 units, plus 200 per second it was
/// wound up past 0.27 s (at most 0.1 s more).
pub fn reach(wound_up: f32) -> f32 {
    REACH + REACH_PER_SECOND * (wound_up - WIND_UP).clamp(0.0, WIND_UP_MAX)
}

/// A hero missile's damage and speed from its class's throwing stat
/// (strength; magic for the wizard and sorceress).
pub fn hero_missile_power(stat: f32) -> (f32, f32) {
    let damage = 5.0 + 0.001 * stat * 15.0;
    let speed = SPEED_MIN + 0.001 * stat * (SPEED_MAX - SPEED_MIN);
    (damage, speed.clamp(1.0, 100.0))
}

/// A monster's throw pause as a throw action starts: `owed` is its throw
/// rate × the level's timing plus what was carried; every whole unit over
/// one becomes a second of pause, plus the new clip's length at 30 frames a
/// second. Returns the pause (None: unchanged) and the new carry.
pub fn throw_pause(owed: f32, frames: u32) -> (Option<f32>, f32) {
    if owed <= 0.0 {
        return (None, owed);
    }
    let (mut whole, mut carry) = (0.0, owed);
    while carry > 1.0 {
        whole += 1.0;
        carry -= 1.0;
    }
    let pause = if whole >= 1.0 { frames as f32 / 30.0 + whole } else { whole };
    (Some(pause), carry)
}

/// The share of a bomb's damage that reaches something whose surface is
/// `distance` from the burst: the blast front gets there when it has grown
/// to it, by then weaker. None when it never does.
pub fn blast_share(distance: f32, blast: f32) -> Option<f32> {
    if blast <= 0.0 {
        return None;
    }
    let f = (1.0 + BLAST_START - distance / blast).min(1.0);
    (f > BLAST_START).then_some(BLAST_FALLOFF * (f - BLAST_START))
}

/// Where a segment from `a` to `b` first enters an upright cylinder
/// (centre, radius, half height), as a fraction of the way; 0 if it starts
/// inside.
pub fn cylinder_hit(a: Vec3, b: Vec3, centre: Vec3, radius: f32, half: f32) -> Option<f32> {
    let d = b - a;
    let p = Vec2::new(a.x - centre.x, a.z - centre.z);
    let v = Vec2::new(d.x, d.z);
    // Across: |p + v t| ≤ radius.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let vv = v.dot(v);
    let c = p.dot(p) - radius * radius;
    if vv < 1e-12 {
        if c > 0.0 {
            return None;
        }
    } else {
        let bq = p.dot(v);
        let disc = bq * bq - vv * c;
        if disc < 0.0 {
            return None;
        }
        let r = disc.sqrt();
        lo = lo.max((-bq - r) / vv);
        hi = hi.min((-bq + r) / vv);
    }
    // Up and down: |a.y + d.y t − centre.y| ≤ half.
    let y = a.y - centre.y;
    if d.y.abs() < 1e-12 {
        if y.abs() > half {
            return None;
        }
    } else {
        let (t0, t1) = ((-half - y) / d.y, (half - y) / d.y);
        lo = lo.max(t0.min(t1));
        hi = hi.min(t0.max(t1));
    }
    (lo <= hi).then_some(lo)
}

/// The game's generic ray test against the level (every surface), with the
/// missile's half radius.
fn wall(collision: &LevelCollision, from: Vec3, to: Vec3, radius: f32) -> Option<collision::Hit> {
    let q = collision::Query {
        radius,
        node_mask: node_flags::ANY,
        disable_mask: 0,
        normal_y: [-2.0, 2.0],
        prefer_crossing: false,
    };
    collision.cast(from.to_array(), to.to_array(), &q)
}

/// A hero's throw: where it starts and how fast it goes, given the class's
/// release offset (in the hero's frame, from its centre), how long it
/// counts as wound up, the missile's speed and gravity. A bolt (kind
/// `0x100000`) isn't lobbed: it flies along the facing.
pub fn hero_launch(shot: &HeroShot, class: usize, offset: Vec3, wound_up: f32, speed: f32, gravity: f32, bolt: bool) -> Launch {
    let aim = hero_aim(shot.facing, shot.aim, shot.targeted, 0.0, class == 4, bolt);
    let hand = shot.feet + Vec3::Y * PLAYER_CENTRE + Quat::from_rotation_y(shot.facing) * offset;
    let dir = if bolt {
        aim
    } else {
        let target = aim * reach(wound_up);
        lob(Vec2::new(target.x, target.z), target.y + REACH_DROP, speed, gravity)
    };
    Launch { check: hand - aim * CHECK_BEHIND, start: hand + aim * START_AHEAD, velocity: dir * speed }
}

/// Which release a strike is (the game's order: a gauntlet's shot, the
/// power throw, the crossbow's bolt, a throw) and how it goes: how long
/// it counts as wound up, the damage multiplier, where it's let go, the
/// kind it adds, the radius it's made bigger by.
#[derive(Debug)]
struct Release {
    wound_up: f32,
    mult: f32,
    hand: Hand,
    kind: u32,
    size: f32,
}

/// Where a release leaves from: the class's throwing hand, its power
/// throw's, or the hero's centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hand {
    Throw,
    Power,
    Centre,
}

fn release(shot: &HeroShot, crossbow_use: bool, boss_level: bool) -> Release {
    let ev = shot.strike.0;
    let fixed = WIND_UP + POWER_WIND_UP;
    if ev & (Strike::GAUNTLET_LEFT | Strike::GAUNTLET_RIGHT) != 0 {
        Release { wound_up: fixed, mult: 1.0, hand: Hand::Centre, kind: 0, size: 1.0 }
    } else if ev & Strike::POWER_THROW != 0 {
        Release { wound_up: fixed, mult: 2.0, hand: Hand::Power, kind: 0x200_0010, size: POWER_SIZE }
    } else if ev & Strike::CROSSBOW != 0 {
        let (mult, kind) = match (crossbow_use, boss_level) {
            (false, _) => (1.0, 0),
            (true, false) => (BOLT_DAMAGE, shot_kind::PIERCE),
            (true, true) => (BOSS_BOLT_DAMAGE, shot_kind::PIERCE),
        };
        Release { wound_up: WIND_UP, mult, hand: Hand::Centre, kind, size: 1.0 }
    } else {
        Release { wound_up: shot.wound_up, mult: 1.0, hand: Hand::Throw, kind: 0, size: 1.0 }
    }
}

/// The sound a hero's release makes: a gauntlet's own; else the super
/// shot with the crossbow or a multi-shot; else the weapon element's;
/// else the class's throw (the secret classes the class eight before
/// theirs — a stand-in, the game's index for them isn't traced).
fn throw_sound(special: u32, weapon: u32, class: usize) -> String {
    if special & LEFT_GAUNTLET != 0 {
        return "S_GAUNTLET1".into();
    }
    if special & RIGHT_GAUNTLET != 0 {
        return "S_GAUNTLET2".into();
    }
    if weapon & (shot_kind::MULTI | shot_kind::MULTI5 | shot_kind::PIERCE) != 0 {
        return "S_SUPERSHOT".into();
    }
    match weapon & 0xF {
        1 => "S_AMULETFIRE".into(),
        2 => "S_AMULETLIGHTNI".into(),
        3 => "S_AMULETLIGHT".into(),
        4 => "S_AMULETACID".into(),
        _ => format!("S_{}THROW", ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"][missile_class(class)]),
    }
}

/// A monster's throw at `shot.at`, or None when the target is off to the
/// side (more than 45° from its facing).
pub fn monster_launch(shot: &MonsterShot, kind: MissileKind, t: &MissileType, speed_scale: f32, spread: f32) -> Option<(Launch, f32)> {
    let speed = t.speed * speed_scale;
    if speed <= 0.0 {
        return None;
    }
    let d = shot.at - shot.from;
    let mut dir = match kind {
        MissileKind::Fireball => d.normalize_or_zero(),
        _ => {
            let drop = if kind == MissileKind::Bomb { BOMB_DROP } else { ARROW_DROP };
            let off = spread * (-MONSTER_SPREAD + 2.0 * MONSTER_SPREAD * shot.random) + drop;
            lob(Vec2::new(d.x, d.z), d.y + off, speed, t.gravity)
        }
    };
    dir.y = dir.y.max(0.0);
    let n = dir.normalize_or_zero();
    if n.x * shot.facing.sin() + n.z * shot.facing.cos() < MONSTER_CONE {
        return None;
    }
    // Where each type lets go, relative to its centre.
    let mut lift = if kind == MissileKind::Fireball { 0.0 } else { MONSTER_LIFT };
    let mut ahead = 0.0;
    match (shot.enemy, kind) {
        (4, MissileKind::Arrow) | (7 | 24, MissileKind::Fireball) | (14, MissileKind::Arrow) => lift = 1.5,
        (13, MissileKind::Arrow) | (14, MissileKind::Fireball) => lift = 1.0,
        (17, _) => {
            lift = 2.0;
            ahead = 2.0;
        }
        (23, MissileKind::Arrow) => lift = 0.0,
        (23, MissileKind::Bomb) => {
            lift = 0.0;
            ahead = -2.5;
        }
        (27, _) => lift = 0.0,
        _ => {}
    }
    let release = Vec3::new(shot.from.x + dir.x * ahead, shot.from.y + lift, shot.from.z + dir.z * ahead);
    let launch = Launch { check: release, start: release + dir * MONSTER_AHEAD, velocity: dir * speed };
    Some((launch, t.damage))
}

/// Per-level missile setup: the hero's missile and release offsets, and
/// the models.
#[derive(Resource)]
struct HeroMissile {
    class: usize,
    kind: MissileType,
    throw_offset: Vec3,
    power_offset: Vec3,
    damage: f32,
    speed: f32,
    model: Option<Arc<CharacterModel>>,
    /// What the hero throws as the Pojo: the phoenix's fireball.
    pojo_model: Option<Arc<CharacterModel>>,
    /// The crossbow's bolt and the gauntlets' shots.
    bolt_model: Option<Arc<CharacterModel>>,
    lightning_model: Option<Arc<CharacterModel>>,
    acid_model: Option<Arc<CharacterModel>>,
}

/// The Pojo (special `0x400`) throws the phoenix's fireball (`WEAPONS`)
/// in place of the class's weapon, from here in the hero's frame (from its
/// centre) whatever the throw — still the class's missile otherwise: its
/// size, fall, spin and damage.
const POJO: u32 = 0x400;
const POJO_MISSILE: &str = "PHOENIX_FBALL";
const BOLT_MISSILE: &str = "SUPERARROW";
const LIGHTNING_MISSILE: &str = "BOSSG_ELEC";
const ACID_MISSILE: &str = "BOSSG_ACID";
const POJO_HAND: Vec3 = Vec3::new(0.0, -0.5, -1.25);

/// Monster missile models, loaded the first time a type throws one.
#[derive(Resource, Default)]
struct MissileModels(HashMap<(i32, MissileKind), Option<Arc<CharacterModel>>>);

/// When each player can next be hurt by a missile (fixed-clock seconds).
#[derive(Resource, Default)]
struct PlayerGuard(HashMap<Entity, f64>);

/// Reads an atree (by name) with its folder's models and textures.
pub(crate) fn load_atree(game: &mut LoadedGame, folder: &str, atree: &str) -> Option<CharacterData> {
    let anim = AnimFile::parse(&game.install.read(&format!("{folder}/ANIM.PS2")).ok()?).ok()?;
    let tree = anim.atrees.into_iter().find(|a| a.name.eq_ignore_ascii_case(atree))?;
    let model = ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?;
    let textures = game.install.read(&format!("{folder}/textures.ngc")).ok()?;
    Some(CharacterData {
        name: format!("{folder}/{atree}"),
        class: String::new(),
        colour: String::new(),
        clips: Arc::new(tree.clone()),
        skeleton: tree,
        model,
        textures,
    })
}

#[allow(clippy::too_many_arguments)]
fn setup_level(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    choice: Option<Res<PlayerChoice>>,
    state: Option<Res<PlayerState>>,
    mut models: ResMut<MissileModels>,
    mut guard: ResMut<PlayerGuard>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    models.0.clear();
    guard.0.clear();
    let Some(choice) = choice else { return };
    let Some(class) = character::class_index(&choice.class) else { return };
    let level = state.as_ref().map_or(1, |s| s.level.max(1));
    let stats = game
        .install
        .read(&format!("PDATA/{}.WAD", choice.class))
        .ok()
        .and_then(|b| PlayerStats::parse(&b).ok().flatten());
    let magic = matches!(missile_class(class), 2 | 6);
    let stat = stats.map_or(400.0, |s| {
        let st = if magic { s.magic } else { s.strength };
        locomotion::stat_at_level(st.start, st.max, level, 0.0)
    });
    let (damage, speed) = hero_missile_power(stat);

    // The model: `<weapon>_THROW<n>`, the class's own for n = 0, else
    // among its effects (`SFX<colour>`), else `<weapon>_THROW1` of its own.
    let colour: String = choice.variant.chars().take(3).collect::<String>().to_ascii_uppercase();
    let base = &["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"][missile_class(class)];
    let model = WEAPONS.get(class).and_then(|(weapon, digits)| {
        let digit = digits[(level as usize / 10).min(9)] as char;
        let own = format!("PLAYERS/{}/{}", choice.class, choice.variant);
        let effects = [format!("PLAYERS/{}/SFX{colour}", choice.class), format!("PLAYERS/{base}/SFX{colour}")];
        let data = if digit == '0' {
            load_atree(&mut game, &own, &format!("{weapon}_THROW0"))
        } else {
            let name = format!("{weapon}_THROW{digit}");
            effects.iter().find_map(|f| load_atree(&mut game, f, &name))
        };
        let data = data.or_else(|| load_atree(&mut game, &own, &format!("{weapon}_THROW1")));
        if data.is_none() {
            warn!("no thrown-weapon model for {} ({weapon}, {digit})", choice.class);
        }
        data.map(|d| Arc::new(CharacterModel::build(&d, &mut meshes, &mut materials, &mut images)))
    });
    let mut weapon = |name: &str| {
        let model = load_atree(&mut game, "WEAPONS", name)
            .map(|d| Arc::new(CharacterModel::build(&d, &mut meshes, &mut materials, &mut images)));
        if model.is_none() {
            warn!("no {name} in WEAPONS");
        }
        model
    };
    let pojo_model = weapon(POJO_MISSILE);
    let bolt_model = weapon(BOLT_MISSILE);
    let lightning_model = weapon(LIGHTNING_MISSILE);
    let acid_model = weapon(ACID_MISSILE);
    let v = |a: [f32; 3]| Vec3::from(a);
    info!("{} throws: {damage:.1} damage at {speed:.1} units/s", choice.class);
    commands.insert_resource(HeroMissile {
        class,
        kind: HERO_MISSILES[missile_class(class)],
        throw_offset: stats.map_or(Vec3::Y, |s| v(s.throw_offset)),
        power_offset: stats.map_or(Vec3::Y, |s| v(s.power_throw_offset)),
        damage,
        speed,
        model,
        pojo_model,
        bolt_model,
        lightning_model,
        acid_model,
    });
}

/// A monster or object as a missile sees it: an upright cylinder.
pub(crate) struct Body {
    pub entity: Entity,
    pub kind: TargetKind,
    pub centre: Vec3,
    pub radius: f32,
    pub half: f32,
    /// Part of a critter: it counts once (`combat::one_per_critter`).
    pub aim: Option<CritterAim>,
}

/// The monsters and objects this tick. A monster is its radius and, as
/// half height, its step, around its centre (feet + centre height; the
/// game's `+0x54` point, taken to be that as for players); a critter's
/// body its cylinder's radius and, as half height, its height around its
/// centre, a hit sphere its radius both ways around its own (the game's
/// swept cylinder tests); anything else targetable as a monster or object
/// (the practice dummy) is its own extent.
pub(crate) fn bodies(targets: &Query<(Entity, &GlobalTransform, &Targetable, Option<&Monster>)>) -> Vec<Body> {
    targets
        .iter()
        .filter(|(_, _, t, _)| matches!(t.kind, TargetKind::Monster | TargetKind::Object))
        .map(|(entity, g, t, m)| match m {
            Some(m) => Body {
                entity,
                kind: t.kind,
                centre: Vec3::from(m.position) + Vec3::Y * enemy::enemy_stats(m.enemy).map_or(0.0, |s| s.center_height),
                radius: m.stats.radius,
                half: m.stats.step,
                aim: None,
            },
            None if t.critter.is_some() => Body {
                entity,
                kind: t.kind,
                centre: g.translation(),
                radius: t.radius,
                half: t.height,
                aim: t.critter,
            },
            None => Body {
                entity,
                kind: t.kind,
                centre: g.translation() + Vec3::Y * (0.5 * t.height),
                radius: t.radius,
                half: 0.5 * t.height,
                aim: None,
            },
        })
        .collect()
}

/// The first potion lying on the floor a segment meets (hero missiles
/// only): its upright cylinder grown by the missile's radius (stand-in for
/// the game's item touch test, as for [`item_hit`]).
fn potion_hit(a: Vec3, b: Vec3, radius: f32, items: &LevelItems) -> Option<(f32, usize)> {
    items
        .views()
        .filter(is_floor_potion)
        .filter_map(|v| {
            let centre = Vec3::from(v.shape.centre);
            cylinder_hit(a, b, centre, radius + v.shape.radius, radius + v.shape.reach).map(|s| (s, v.placement))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
}

/// The first generator or breakable (hero missiles only) a segment meets:
/// the item's upright extent around its feet grown by the missile's
/// radius (stand-in for the game's item touch test).
fn item_hit<'a>(
    a: Vec3,
    b: Vec3,
    radius: f32,
    items: impl Iterator<Item = (Entity, Vec3, &'a Targetable)>,
) -> Option<(f32, Entity, TargetKind)> {
    items
        .filter(|(_, _, t)| matches!(t.kind, TargetKind::Generator | TargetKind::Breakable))
        .filter_map(|(e, feet, t)| {
            let centre = feet + Vec3::Y * (0.5 * t.height);
            cylinder_hit(a, b, centre, t.radius + radius, 0.5 * t.height + radius).map(|s| (s, e, t.kind))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
}

#[allow(clippy::too_many_arguments)]
fn spawn_projectile(
    commands: &mut Commands,
    model: Option<&CharacterModel>,
    owner: Owner,
    launch: &Launch,
    t: &MissileType,
    radius: f32,
    damage: f32,
    kind: u32,
    scale: f32,
) -> Entity {
    let p = Projectile {
        owner,
        position: launch.start,
        previous: launch.start,
        velocity: launch.velocity,
        gravity: t.gravity,
        radius,
        damage,
        kind: kind | t.kind,
        blast: t.blast,
        spin: t.spin,
        yaw: launch.velocity.x.atan2(launch.velocity.z),
        age: 0.0,
        lifetime: LIFETIME,
        scale,
        potion: None,
        pierced: Vec::new(),
        reflected: None,
    };
    let transform = Transform::from_translation(launch.start).with_rotation(orientation(&p)).with_scale(Vec3::splat(scale));
    let entity = match model {
        Some(m) => m.spawn(transform, commands),
        None => commands.spawn((transform, Visibility::default())).id(),
    };
    commands.entity(entity).insert((p, LevelEntity));
    entity
}

/// A thrown potion (`effects.rs`): it tumbles (10π rad/s), falls at 100
/// units/s², hits like a hero's missile (the burst's damage) and bursts
/// into a magic blast where it stops, or when its 0.667 s are up.
pub fn spawn_potion(
    commands: &mut Commands,
    model: Option<&CharacterModel>,
    hero: Entity,
    start: Vec3,
    velocity: Vec3,
    burst: PotionBurst,
) -> Entity {
    let t = missile(burst.kind, burst.damage, velocity.length(), POTION_RADIUS, 0.0, [POTION_SPIN, 0.0, 0.0], POTION_GRAVITY);
    let launch = Launch { check: start, start, velocity };
    let e = spawn_projectile(commands, model, Owner::Hero(hero), &launch, &t, POTION_RADIUS, burst.damage, 0, 1.0);
    commands.queue(move |world: &mut World| {
        if let Some(mut p) = world.get_mut::<Projectile>(e) {
            p.lifetime = POTION_LIFETIME;
            p.potion = Some(burst);
        }
    });
    e
}

const POTION_RADIUS: f32 = 0.5;
const POTION_SPIN: f32 = 10.0 * PI;
const POTION_GRAVITY: f32 = 100.0;
const POTION_LIFETIME: f32 = 0.667;
/// A missile with a blast bursts this long before its time is up.
const BURST_EARLY: f32 = 1.0 / 15.0;

/// Whether a wall (any surface) lies between two points, for a sphere of
/// `radius`.
pub fn wall_between(collision: &LevelCollision, from: Vec3, to: Vec3, radius: f32) -> bool {
    wall(collision, from, to, radius).is_some()
}

/// A shot a hero's familiar or phoenix spits (`familiars.rs`): it flies
/// and hits like the hero's own missiles — monsters, generators,
/// breakables, potions on the floor, the level — from `start` at
/// `velocity`, falling at `gravity`, pointing along its flight, for 3 s,
/// with its own model and numbers.
#[allow(clippy::too_many_arguments)]
pub fn spawn_hero_missile(
    commands: &mut Commands,
    model: Option<&CharacterModel>,
    hero: Entity,
    start: Vec3,
    velocity: Vec3,
    gravity: f32,
    radius: f32,
    damage: f32,
    kind: u32,
) -> Entity {
    let t = missile(0, damage, velocity.length(), radius, 0.0, [0.0; 3], gravity);
    let launch = Launch { check: start, start, velocity };
    spawn_projectile(commands, model, Owner::Hero(hero), &launch, &t, radius, damage, kind, 1.0)
}

/// A critter's missile (`critters.rs`): it flies and hits like a
/// monster's, from `start` at `velocity`, with the critter's own model
/// (drawn at `scale`) and numbers.
#[allow(clippy::too_many_arguments)]
pub fn spawn_critter_missile(
    commands: &mut Commands,
    model: Option<&CharacterModel>,
    critter: Entity,
    start: Vec3,
    velocity: Vec3,
    gravity: f32,
    radius: f32,
    damage: f32,
    kind: u32,
    scale: f32,
) -> Entity {
    let t = missile(0, damage, velocity.length(), radius, 0.0, [0.0; 3], gravity);
    let launch = Launch { check: start, start, velocity };
    spawn_projectile(commands, model, Owner::Monster(critter), &launch, &t, radius, damage, kind, scale)
}

#[allow(clippy::too_many_arguments)]
fn launch_hero(
    mut commands: Commands,
    mut shots: MessageReader<HeroShot>,
    hero: Option<Res<HeroMissile>>,
    (state, level): (Option<Res<PlayerState>>, Option<Res<MonsterLevel>>),
    ground: Option<Res<LevelGround>>,
    targets: Query<(Entity, &GlobalTransform, &Targetable)>,
    mut hits: MessageWriter<Hit>,
    (items, mut struck): (Option<Res<LevelItems>>, MessageWriter<StrikePotion>),
    (mut sounds, mut spent): (MessageWriter<PlaySound>, MessageWriter<SpendPower>),
    players: Query<&Player>,
) {
    let boss_level = level.as_ref().is_some_and(|l| l.boss >= 0);
    let top_level = state.as_ref().is_some_and(|s| s.level > TOP_LEVEL);
    for shot in shots.read() {
        let Some(hero) = hero.as_deref() else { continue };
        let (weapon, special) = players.get(shot.hero).map_or((0, 0), |p| (p.weapon, p.special_bits));
        // The crossbow's bolt spends a use of it; with none left it's the
        // class's missile.
        let crossbow_use = shot.strike.0 & Strike::CROSSBOW != 0 && weapon & shot_kind::PIERCE != 0;
        if crossbow_use {
            spent.write(SpendPower { subtype: power::WEAPON, bits: shot_kind::PIERCE });
        }
        let r = release(shot, crossbow_use, boss_level);
        // The throw starts from the hero's weapon bits.
        let kind = r.kind | weapon;
        let bolt = kind & shot_kind::PIERCE != 0;
        // The missile: a gauntlet's while one is worn, a bolt with the
        // crossbow (not the power throw), else the class's.
        let (t, model) = if special & LEFT_GAUNTLET != 0 {
            (LIGHTNING_SHOT, &hero.lightning_model)
        } else if special & RIGHT_GAUNTLET != 0 {
            (ACID_SHOT, &hero.acid_model)
        } else if bolt && kind & shot_kind::POWER == 0 {
            (BOLT, &hero.bolt_model)
        } else {
            (hero.kind, &hero.model)
        };
        // The Pojo throws from its own hand, the phoenix's fireball unless
        // a gauntlet's shot.
        let pojo = special & POJO != 0;
        let model = if pojo && special & (LEFT_GAUNTLET | RIGHT_GAUNTLET) == 0 { &hero.pojo_model } else { model };
        let offset = match r.hand {
            _ if pojo => POJO_HAND,
            Hand::Power => hero.power_offset,
            Hand::Throw => hero.throw_offset,
            Hand::Centre => Vec3::ZERO,
        };
        sounds.write(PlaySound(throw_sound(special, weapon, hero.class)));
        let mut launch = hero_launch(shot, missile_class(hero.class), offset, r.wound_up, hero.speed, t.gravity, bolt);
        let radius = t.radius * r.size;
        let damage = hero.damage * r.mult;
        // A wall between the hand and the start: it breaks there (a
        // bouncing one starts from behind the hand instead; a bolt goes
        // through).
        if !bolt
            && let Some(g) = ground.as_deref()
            && wall(&g.0, launch.check, launch.start, radius).is_some()
        {
            if kind & shot_kind::BOUNCE == 0 {
                debug!("throw blocked by a wall at release");
                continue;
            }
            launch.start = launch.check;
        }
        // Something breakable right there takes it at once; a potion lying
        // there goes off. The throw ends there, a bolt's goes on.
        let found = item_hit(launch.check, launch.start, radius, targets.iter().map(|(e, g, t)| (e, g.translation(), t)));
        let potion = items.as_deref().and_then(|i| potion_hit(launch.check, launch.start, radius, i));
        if let Some((_, placement)) = potion.filter(|(s, _)| found.is_none_or(|f| *s < f.0)) {
            struck.write(StrikePotion { placement, by: Some(shot.hero) });
            if !bolt {
                continue;
            }
        } else if let Some((s, target, target_kind)) = found {
            let at = launch.check.lerp(launch.start, s);
            hits.write(Hit { target, attacker: shot.hero, damage, kind, push: Vec3::ZERO, at, target_kind, ranged: true });
            if !bolt {
                continue;
            }
        }
        info!(
            "hero throws: {damage:.1} damage, {:.1} units/s, from {:?} along {:?}",
            launch.velocity.length(),
            launch.start,
            launch.velocity
        );
        // Drawn bigger by a damage multiplier above 1, and at level 99.
        let scale = r.mult.max(1.0) * if top_level { TOP_LEVEL_SIZE } else { 1.0 };
        let count = spread_count(kind);
        for &(c, s) in &SPREAD[..count] {
            let v = launch.velocity;
            let turned = Launch { velocity: Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c), ..launch };
            let e = spawn_projectile(&mut commands, model.as_deref(), Owner::Hero(shot.hero), &turned, &t, radius, damage, kind, scale);
            if count > 1 {
                commands.entity(e).entry::<Projectile>().and_modify(|mut p| p.lifetime = SPREAD_LIFETIME);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn launch_monster(
    mut commands: Commands,
    mut shots: MessageReader<MonsterShot>,
    mut game: ResMut<LoadedGame>,
    level: Option<Res<MonsterLevel>>,
    ground: Option<Res<LevelGround>>,
    mut models: ResMut<MissileModels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    enemies: Res<EnemyScale>,
) {
    for shot in shots.read() {
        let Some(level) = level.as_deref() else { continue };
        let kind = MissileKind::of_ai(shot.ai);
        let Some(t) = monster_missile(shot.enemy, kind) else {
            debug!("enemy {} has no {:?}", shot.enemy, kind);
            continue;
        };
        let Some((launch, mut damage)) =
            monster_launch(shot, kind, &t, level.tuning.missile_speed, level.tuning.missile_spread)
        else {
            continue;
        };
        // Shrunk monsters' missiles do half.
        if enemies.shrunk() {
            damage *= SHRUNK_MISSILE;
        }
        if let Some(g) = ground.as_deref()
            && wall(&g.0, launch.check, launch.start, t.radius).is_some()
        {
            continue;
        }
        let model = models
            .0
            .entry((shot.enemy, kind))
            .or_insert_with(|| {
                let name = enemy::enemy_name(shot.enemy)?.to_ascii_uppercase();
                let atree = format!("{name}_{}", kind.name());
                let mut folders = level.enemies.folders(shot.enemy);
                folders.push(name.clone());
                let data = folders.iter().find_map(|f| load_atree(&mut game, &format!("MONSTERS/{f}"), &atree));
                if data.is_none() {
                    warn!("no model {atree} for enemy {}", shot.enemy);
                }
                data.map(|d| Arc::new(CharacterModel::build(&d, &mut meshes, &mut materials, &mut images)))
            })
            .clone();
        debug!("enemy {} throws a {kind:?}: {damage} damage from {:?}", shot.enemy, launch.start);
        spawn_projectile(&mut commands, model.as_deref(), Owner::Monster(shot.monster), &launch, &t, t.radius, damage, 0, 1.0);
    }
}

/// What a missile ran into this tick.
enum Stop {
    /// A player, a monster, an item or a wall, at this point.
    At(Vec3),
    /// Out of time, or fell out of the level: it just goes.
    Gone,
}

#[allow(clippy::too_many_arguments)]
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    ground: Option<Res<LevelGround>>,
    mut projectiles: Query<(Entity, &mut Projectile)>,
    mut players: Query<(Entity, &mut Player)>,
    targets: Query<(Entity, &GlobalTransform, &Targetable, Option<&Monster>)>,
    mut guard: ResMut<PlayerGuard>,
    mut hits: MessageWriter<Hit>,
    mut damage: MessageWriter<DamagePlayer>,
    mut potions: MessageWriter<BlastAt>,
    (items, mut struck): (Option<Res<LevelItems>>, MessageWriter<StrikePotion>),
    (mut sounds, mut ricochet): (MessageWriter<PlaySound>, Local<f64>),
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    let bottom = ground.as_ref().map_or(f32::MIN, |g| g.0.bounds[0][1] - BELOW_LEVEL);
    let bodies = bodies(&targets);
    for (entity, mut p) in &mut projectiles {
        let p = &mut *p;
        let from = p.position;
        let mut to = from + p.velocity * dt;
        p.previous = from;
        p.velocity.y -= p.gravity * dt;
        p.age += dt;
        let r = p.radius;
        let hero_owned = matches!(p.owner, Owner::Hero(_));
        let mut stop = None;

        // Players (monster missiles): the first cylinder the move enters.
        if !hero_owned {
            let hit = players
                .iter()
                .filter(|(e, _)| p.reflected != Some(*e))
                .filter_map(|(e, pl)| {
                    let centre = Vec3::from(pl.mover.position) + Vec3::Y * PLAYER_CENTRE;
                    cylinder_hit(from, to, centre, r + pl.radius, r + PLAYER_HALF_HEIGHT).map(|s| (s, e))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let reflects = hit.is_some_and(|(_, e)| players.get(e).is_ok_and(|(_, pl)| pl.armour_bits & REFLECTS != 0));
            if let Some((s, player)) = hit.filter(|_| reflects) {
                // A reflect shield turns it back unhurt: it hits monsters
                // now, at most 15, and moves on at once (`S_RICOCHET` at
                // most once a second).
                if now >= *ricochet + RICOCHET_EVERY {
                    sounds.write(PlaySound(RICOCHET.into()));
                    *ricochet = now;
                }
                p.velocity = -p.velocity;
                p.damage = p.damage.min(REFLECTED_MOST);
                p.reflected = Some(player);
                to = from.lerp(to, s) + p.velocity * dt;
                info!("a missile glances off the hero's reflect shield");
            } else if let Some((s, player)) = hit {
                to = from.lerp(to, s);
                let until = guard.0.get(&player).copied().unwrap_or(f64::MIN);
                if until <= now
                    && let Ok((_, mut pl)) = players.get_mut(player)
                {
                    // Pushing along its flight.
                    let amount = pl.take_blow(p.damage, p.kind, p.velocity.normalize_or_zero());
                    if amount != 0.0 {
                        damage.write(DamagePlayer { amount });
                    }
                    if p.damage > GUARD_ABOVE {
                        guard.0.insert(player, now + PLAYER_GUARD as f64);
                    }
                    info!("a missile hits the hero for {amount:.1}");
                }
                stop = Some(Stop::At(to));
            }
        }
        // Monsters and objects (hero missiles, and reflected ones).
        if stop.is_none() && (hero_owned || p.reflected.is_some()) {
            // A critter's spheres before its body.
            let met: Vec<(f32, &Body)> = bodies
                .iter()
                .filter(|b| !p.pierced.contains(&b.entity))
                .filter_map(|b| cylinder_hit(from, to, b.centre, r + b.radius, r + b.half).map(|s| (s, b)))
                .collect();
            let hit = combat::one_per_critter(met, |(_, b)| b.aim).into_iter().min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((s, b)) = hit {
                let at = from.lerp(to, s);
                info!("missile hits {:?} {:?} for {:.1}", b.kind, b.entity, p.damage);
                hits.write(Hit {
                    target: b.entity,
                    attacker: p.owner.entity(),
                    damage: p.damage,
                    kind: p.kind,
                    push: Vec3::ZERO,
                    at,
                    target_kind: b.kind,
                    ranged: true,
                });
                // A piercing one flies on (each thing it passes, once).
                if p.kind & shot_kind::PIERCE != 0 {
                    p.pierced.push(b.entity);
                } else {
                    to = at;
                    stop = Some(Stop::At(to));
                }
            }
        }
        // Generators, breakables and potions on the floor (hero missiles;
        // the monsters' pass them): the nearest along the way.
        let potion = items.as_deref().filter(|_| stop.is_none() && hero_owned).and_then(|i| potion_hit(from, to, r, i));
        if stop.is_none() && hero_owned {
            let found = item_hit(from, to, r, targets.iter().map(|(e, g, t, _)| (e, g.translation(), t)).filter(|(e, ..)| !p.pierced.contains(e)));
            if let Some((s, placement)) = potion.filter(|(s, _)| found.is_none_or(|f| *s < f.0)) {
                to = from.lerp(to, s);
                info!("missile strikes a potion (placement {placement})");
                struck.write(StrikePotion { placement, by: Some(p.owner.entity()) });
                stop = Some(Stop::At(to));
            } else if let Some((s, target, target_kind)) = found {
                let at = from.lerp(to, s);
                info!("missile hits {target_kind:?} {target:?} for {:.1}", p.damage);
                hits.write(Hit {
                    target,
                    attacker: p.owner.entity(),
                    damage: p.damage,
                    kind: p.kind,
                    push: Vec3::ZERO,
                    at,
                    target_kind,
                    ranged: true,
                });
                if p.kind & shot_kind::PIERCE != 0 {
                    p.pierced.push(target);
                } else {
                    to = at;
                    stop = Some(Stop::At(to));
                }
            }
        }
        // The level.
        if stop.is_none()
            && let Some(g) = ground.as_deref()
            && let Some(h) = wall(&g.0, from, to, WALL_RADIUS * r)
        {
            let n = Vec3::from(h.normal).normalize_or_zero();
            if p.kind & shot_kind::BOUNCE == 0 {
                to = Vec3::from(h.point);
                debug!("missile hits the level at {to:?}");
                stop = Some(Stop::At(to));
            } else if p.velocity.dot(n) < 0.0 {
                // Reflected off the surface, keeping less of any rise (one
                // already leaving the surface it touches flies on).
                to = Vec3::from(h.point);
                p.velocity -= 2.0 * p.velocity.dot(n) * n;
                if p.velocity.y > 0.0 {
                    p.velocity.y *= BOUNCE_RISE;
                }
                debug!("missile bounces off the level at {to:?}");
            }
        }
        let bursts = p.blast > 0.0 || p.potion.is_some();
        if stop.is_none() && bursts && p.age >= p.lifetime - BURST_EARLY {
            stop = Some(Stop::At(to));
        }
        if stop.is_none() && (p.age >= p.lifetime || to.y < bottom) {
            stop = Some(Stop::Gone);
        }
        p.position = to;
        match stop {
            None => {}
            Some(Stop::Gone) => {
                commands.entity(entity).despawn();
            }
            Some(Stop::At(at)) => {
                if let Some(b) = p.potion {
                    potions.write(BlastAt { owner: p.owner.entity(), at, kind: b.kind, damage: b.damage, radius: b.radius });
                } else if p.blast > 0.0 {
                    burst(p, at, now, &mut players, &bodies, &mut guard, &mut hits, &mut damage);
                }
                commands.entity(entity).despawn();
            }
        }
    }
}

/// A bomb bursts at `at`: every monster (and, for a monster's bomb, every
/// player) the growing blast reaches takes its share of the damage, once.
#[allow(clippy::too_many_arguments)]
fn burst(
    p: &Projectile,
    at: Vec3,
    now: f64,
    players: &mut Query<(Entity, &mut Player)>,
    bodies: &[Body],
    guard: &mut PlayerGuard,
    hits: &mut MessageWriter<Hit>,
    damage: &mut MessageWriter<DamagePlayer>,
) {
    info!("bomb bursts at {at:?} (blast {:.1})", p.blast);
    // A critter once: its first sphere in reach, else its body.
    let reached: Vec<(f32, &Body)> = bodies
        .iter()
        .filter_map(|b| blast_share((b.centre - at).length() - b.radius, p.blast).map(|share| (share, b)))
        .collect();
    for (share, b) in combat::one_per_critter(reached, |(_, b)| b.aim) {
        hits.write(Hit {
            target: b.entity,
            attacker: p.owner.entity(),
            damage: p.damage * share,
            kind: p.kind,
            push: Vec3::ZERO,
            at,
            target_kind: b.kind,
            ranged: true,
        });
    }
    if matches!(p.owner, Owner::Monster(_)) {
        for (e, mut pl) in players.iter_mut() {
            let feet = Vec3::from(pl.mover.position);
            let centre = feet + Vec3::Y * PLAYER_CENTRE;
            let d = (centre - at).length() - pl.radius;
            let Some(share) = blast_share(d, p.blast) else { continue };
            if guard.0.get(&e).is_none_or(|&until| until <= now) {
                let blow = p.damage * share;
                let (kind, push) = crate::effects::blast_on_hero(blow, p.kind, at, feet);
                let amount = pl.take_blow(blow, kind, push);
                if amount != 0.0 {
                    damage.write(DamagePlayer { amount });
                }
                if blow > GUARD_ABOVE {
                    guard.0.insert(e, now + PLAYER_GUARD as f64);
                }
            }
        }
    }
}

/// How a missile is turned: along its flight, or tumbling from the heading
/// it left along.
fn orientation(p: &Projectile) -> Quat {
    if p.spin == [0.0; 3] {
        let v = p.velocity;
        let across = Vec2::new(v.x, v.z).length();
        Quat::from_rotation_y(v.x.atan2(v.z)) * Quat::from_rotation_x(-v.y.atan2(across))
    } else {
        let [x, y, z] = p.spin;
        Quat::from_rotation_y(p.yaw)
            * Quat::from_rotation_x(-x * p.age)
            * Quat::from_rotation_y(y * p.age)
            * Quat::from_rotation_z(z * p.age)
    }
}

fn interpolate(fixed: Res<Time<Fixed>>, mut projectiles: Query<(&Projectile, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (p, mut transform) in &mut projectiles {
        transform.translation = p.previous.lerp(p.position, t);
        transform.rotation = orientation(p);
        transform.scale = Vec3::splat(p.scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_throw_reaches_further_the_longer_it_is_wound_up() {
        assert_eq!(reach(0.0), 15.0);
        assert_eq!(reach(0.27), 15.0);
        assert!((reach(0.32) - 25.0).abs() < 1e-4);
        assert!((reach(5.0) - 35.0).abs() < 1e-4);
        // The power throw counts as 0.06 s.
        assert!((reach(WIND_UP + POWER_WIND_UP) - 27.0).abs() < 1e-4);
    }

    #[test]
    fn missile_power_follows_the_stat() {
        assert_eq!(hero_missile_power(0.0), (5.0, 20.0));
        let (d, s) = hero_missile_power(1000.0);
        assert!((d - 20.0).abs() < 1e-4 && (s - 60.0).abs() < 1e-4);
    }

    /// A lob lands where it was aimed: after `across / speed` seconds of
    /// falling it's `rise` above the start.
    #[test]
    fn a_lob_comes_down_where_aimed() {
        for (across, rise, speed, g) in [(35.0, -0.5, 36.0, 8.0), (15.0, 2.0, 20.0, 30.0), (27.0, -4.0, 25.0, 35.0)] {
            let dir = lob(Vec2::new(0.0, across), rise, speed, g);
            let v = dir * speed;
            let t = across / v.z;
            let y = v.y * t - 0.5 * g * t * t;
            assert!((y - rise).abs() < 1e-3, "{across} {rise} {speed} {g}: {y}");
        }
    }

    #[test]
    fn heroes_aim_at_targets_in_front_only() {
        // Nothing found: along the facing, level.
        let a = hero_aim(0.0, Vec3::Z, false, 0.0, false, false);
        assert!((a - Vec3::Z).length() < 1e-6);
        // A target 20° off and a little up: steeper by 1.2.
        let n = Vec3::new(20f32.to_radians().sin(), 0.1, 20f32.to_radians().cos()).normalize();
        let a = hero_aim(0.0, n, true, 0.0, false, false);
        assert!(a.x > 0.3 && a.y > n.y);
        // A bolt goes along the facing whatever it found.
        assert!((hero_aim(0.0, n, true, 0.0, false, true) - Vec3::Z).length() < 1e-6);
        // 40° off: ignored.
        let off = Vec3::new(40f32.to_radians().sin(), 0.0, 40f32.to_radians().cos());
        assert!((hero_aim(0.0, off, true, 0.0, false, false) - Vec3::Z).length() < 1e-6);
        // Downhill targets count half; the dwarf throws a little up.
        let down = Vec3::new(0.0, -0.4, 0.9).normalize();
        assert!(hero_aim(0.0, down, true, 0.0, false, false).y > down.y);
        assert!(hero_aim(0.0, Vec3::Z, false, 0.0, true, false).y > 0.19);
    }

    fn shot(strike: u32, wound_up: f32) -> HeroShot {
        let hero = Entity::from_raw_u32(1).unwrap();
        HeroShot { hero, feet: Vec3::ZERO, facing: 0.0, aim: Vec3::Z, targeted: false, strike: Strike(strike), wound_up }
    }

    #[test]
    fn releases_in_the_games_order() {
        // A gauntlet's shot: from the centre, reach 27, no extra damage.
        let r = release(&shot(Strike::GAUNTLET_LEFT, 0.0), false, false);
        assert_eq!((r.hand, r.mult, r.kind), (Hand::Centre, 1.0, 0));
        assert!((reach(r.wound_up) - 27.0).abs() < 1e-3);
        // The power throw: from its own offset, twice the damage, bigger.
        let r = release(&shot(Strike::POWER_THROW | Strike::SHOT, 0.0), false, false);
        assert_eq!((r.hand, r.mult, r.kind, r.size), (Hand::Power, 2.0, 0x200_0010, POWER_SIZE));
        // The crossbow's bolt: twice (1.5 times on a boss level) with a
        // use of it, else the class's plain missile.
        let r = release(&shot(Strike::CROSSBOW, 0.0), true, false);
        assert_eq!((r.hand, r.mult, r.kind), (Hand::Centre, BOLT_DAMAGE, shot_kind::PIERCE));
        assert_eq!(release(&shot(Strike::CROSSBOW, 0.0), true, true).mult, BOSS_BOLT_DAMAGE);
        assert_eq!(release(&shot(Strike::CROSSBOW, 0.0), false, false).kind, 0);
        // A throw: from the hand, as long as it was wound up.
        let r = release(&shot(Strike::SHOT, 0.31), false, false);
        assert_eq!((r.hand, r.wound_up), (Hand::Throw, 0.31));
    }

    #[test]
    fn bolts_fly_straight() {
        let s = shot(Strike::CROSSBOW, 0.0);
        let l = hero_launch(&s, 0, Vec3::ZERO, WIND_UP, 40.0, 0.0, true);
        assert!((l.velocity - Vec3::Z * 40.0).length() < 1e-4);
        assert!((l.start - Vec3::new(0.0, PLAYER_CENTRE, START_AHEAD)).length() < 1e-4);
        // A gauntlet's shot is lobbed (with no fall it comes down 0.5
        // lower 27 units ahead).
        let l = hero_launch(&s, 0, Vec3::ZERO, WIND_UP + POWER_WIND_UP, 40.0, 0.0, false);
        assert!(l.velocity.y < 0.0 && (l.velocity.y / l.velocity.z + 0.5 / 27.0).abs() < 1e-4);
    }

    #[test]
    fn throw_sounds_by_power() {
        assert_eq!(throw_sound(LEFT_GAUNTLET | RIGHT_GAUNTLET, shot_kind::PIERCE, 0), "S_GAUNTLET1");
        assert_eq!(throw_sound(RIGHT_GAUNTLET, 0, 0), "S_GAUNTLET2");
        assert_eq!(throw_sound(0, shot_kind::PIERCE | 1, 3), "S_SUPERSHOT");
        assert_eq!(throw_sound(0, shot_kind::MULTI, 3), "S_SUPERSHOT");
        assert_eq!(throw_sound(0, 2, 3), "S_AMULETLIGHTNI");
        assert_eq!(throw_sound(0, 4, 3), "S_AMULETACID");
        assert_eq!(throw_sound(0, 0, 3), "S_ARCTHROW");
        assert_eq!(throw_sound(0, 5, 12), "S_DWFTHROW");
    }

    #[test]
    fn segments_enter_cylinders() {
        let c = Vec3::new(0.0, 2.5, 0.0);
        let s = cylinder_hit(Vec3::new(-5.0, 2.5, 0.0), Vec3::new(5.0, 2.5, 0.0), c, 1.0, 2.5).unwrap();
        assert!((s - 0.4).abs() < 1e-5);
        assert!(cylinder_hit(Vec3::new(-5.0, 2.5, 2.0), Vec3::new(5.0, 2.5, 2.0), c, 1.0, 2.5).is_none());
        assert!(cylinder_hit(Vec3::new(-5.0, 6.0, 0.0), Vec3::new(5.0, 6.0, 0.0), c, 1.0, 2.5).is_none());
        // Dropping in from above.
        let s = cylinder_hit(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, 0.0, 0.0), c, 1.0, 2.5).unwrap();
        assert!((s - 0.5).abs() < 1e-5);
        assert_eq!(cylinder_hit(c, c + Vec3::X, c, 1.0, 2.5), Some(0.0));
    }

    #[test]
    fn blasts_weaken_as_they_grow() {
        assert!((blast_share(0.0, 3.0).unwrap() - 1.5 * 0.67).abs() < 1e-4);
        assert!(blast_share(-1.0, 3.0).unwrap() <= 1.5 * 0.67 + 1e-6);
        // Two thirds of the way out: 1.5 × (0.663 − 0.33).
        let mid = blast_share(2.0, 3.0).unwrap();
        assert!((mid - 1.5 * (1.33 - 2.0 / 3.0 - 0.33)).abs() < 1e-4);
        assert!(blast_share(3.0, 3.0).is_none_or(|s| s < 1e-4));
        assert!(blast_share(1.0, 0.0).is_none());
    }

    #[test]
    fn throw_pauses_come_every_whole_unit() {
        // Rate × timing 1: the first throw is free, the rest pause a second
        // plus the clip.
        let (p, c) = throw_pause(1.0, 30);
        assert_eq!((p, c), (Some(0.0), 1.0));
        let (p, c) = throw_pause(1.0 + c, 30);
        assert_eq!((p, c), (Some(2.0), 1.0));
        // Half: every other throw.
        let (p, c) = throw_pause(0.5, 15);
        assert_eq!((p, c), (Some(0.0), 0.5));
        let (p, _) = throw_pause(0.5 + c, 15);
        assert_eq!(p, Some(0.0));
        assert_eq!(throw_pause(0.0, 15), (None, 0.0));
    }

    #[test]
    fn monsters_throw_only_ahead() {
        let shot = |at: Vec3| MonsterShot {
            monster: Entity::PLACEHOLDER,
            enemy: 4,
            ai: 0x17,
            from: Vec3::new(0.0, 3.0, 0.0),
            at,
            facing: 0.0,
            random: 0.5,
        };
        let t = monster_missile(4, MissileKind::Arrow).unwrap();
        let (l, damage) = monster_launch(&shot(Vec3::new(0.0, 2.5, 20.0)), MissileKind::Arrow, &t, 1.0, 1.0).unwrap();
        assert_eq!(damage, 10.0);
        // Grunts let go 1.5 above their centre, 3 ahead.
        assert!((l.check.y - 4.5).abs() < 1e-5);
        assert!(l.start.z > 2.9 && (Vec2::new(l.velocity.x, l.velocity.z).length() - 25.0).abs() < 1e-3);
        assert!(monster_launch(&shot(Vec3::new(20.0, 2.5, 0.0)), MissileKind::Arrow, &t, 1.0, 1.0).is_none());
        assert_eq!(MissileKind::of_ai(0x11), MissileKind::Bomb);
        assert!(monster_missile(3, MissileKind::Arrow).is_none());
    }
}
