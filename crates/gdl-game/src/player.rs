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
//! `GDL_WARP="x,y,z"` starts the hero there; `GDL_STICK="x,y"` holds the stick; `GDL_BUTTONS="attack@10-12,power"`
//! holds buttons (`attack`, `power`, `turbo`, `magic`, `charge`, `strafe`,
//! `combo`), each for the whole run or for a range of ticks since the hero
//! appeared.

use bevy::mesh::MeshTag;
use bevy::prelude::*;
use gdl_formats::{PlayerCollision, PlayerGround};
use gdl_formats::enemy::FIELDS_PER_TICK;
use gdl_formats::pdata::PlayerStats;

use crate::actions::{self, Action, ActionState, Env, Strike};
use crate::camera::FreeLook;
use crate::character::{self, Animator, CharacterData};
use crate::combat::{self, Buttons, Hit, Intent, TargetKind, Targetable, button};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion::{self, Mover, Stick, wrap};
use crate::play_camera::PlayCamera;
use crate::player_state::{PlayerState, SpendPower, power};
use crate::population::LevelPopulation;
use crate::effects::{BreathAt, ChopAt, EffectAt, MAGIC_BUTTONS, MagicIntent, MagicState, UsePotion};
use crate::flash::{self, Flash, FlashColours};
use crate::fade::BodyLook;
use crate::hints::{Hint, ShowHint};
use crate::projectiles::{self, HeroShot};
use crate::world::{LevelEntity, LevelGround};

/// The hero's radius if the class data can't be read (every class has 1.5).
const DEFAULT_RADIUS: f32 = 1.5;
/// The game's "attack aim" and "walk-into attack" pad options, both on by
/// default: attacking in place turns the hero toward the target, and walking
/// into a monster attacks it.
const ATTACK_AIM: bool = true;
const WALK_INTO_ATTACK: bool = true;
/// The turbo meter (player `+0x828`): fills at 2 a second up to 100 while
/// the hero isn't in a turbo-class action, drains at 20 a second while
/// charging; a turbo attack needs 40 (ATTPWRB) or a full meter (ATTPWRC)
/// and pays for it when the blow lands.
const TURBO_MAX: f32 = 100.0;
const TURBO_REGEN: f32 = 2.0;
const TURBO_CHARGE_DRAIN: f32 = 20.0;
const TURBO_ATTACK: f32 = 40.0;
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
    /// The magic controls: the double tap, the lock after a use, the
    /// throw's wind-up.
    pub magic: MagicState,
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
    /// Speed at its level, before what speed powers add.
    base_speed: f32,
    /// Its weapon powers' bits (`PowerBits::weapon`): its blows and
    /// missiles carry them.
    pub weapon: u32,
    /// Its armour powers' bits (`PowerBits::armour`), which blows on it
    /// go through, and whether this is a boss level (their elements'
    /// factors).
    pub armour_bits: u32,
    boss_level: bool,
    /// Its special powers' bits (`PowerBits::special`).
    pub special_bits: u32,
    /// Its model's scale: the ogre's, grown, at level 99.
    model_scale: f32,
    /// The Hand of Death and the Health Vampire armed (the game's `+0xA1E`,
    /// `+0xA20`): each set while its power is on (the vampire's only
    /// while the hand's is off), both cleared once neither is.
    death_hand: [bool; 2],
    /// Blows taken since the last tick: damage, kind flags, summed push
    /// directions (the game's `+0x8D0`, `+0x8D4`, `+0x8DC`).
    pending_hit: (f32, u32, Vec3),
    /// Its left wrist's skeleton node (the lightning shield's spark
    /// leaves from it), and until when each target is spared the
    /// lightning shield's next blow.
    left_wrist: Option<usize>,
    shield_cooldowns: Vec<(Entity, f64)>,
    /// Its hit flash (`flash.rs`), the invulnerability's chrome in the
    /// same slot, and how its body is drawn for them ([`show_body_looks`]).
    flash: Flash,
    chrome: Flash,
    body_look: BodyLook,
    /// Turbo meter, 0–100.
    pub turbo: f32,
    /// What the turbo attack under way will cost when it lands.
    turbo_cost: f32,
    /// Collision radius: reach and range bands are measured from it.
    pub radius: f32,
    /// The game's class index.
    class: Option<usize>,
    /// When the latest attack or throw began (fixed-clock seconds): how
    /// long a throw was wound up decides how far it goes.
    attack_started: f64,
    /// The level node and point the last tick's move ran into, if a wall
    /// stopped it (damaging walls, `hazards.rs`).
    pub wall_hit: Option<(usize, [f32; 3])>,
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
        p.base_speed = speed;
        p.mover.speed = speed;
    }
    info!("level {}: strength {strength:.1}, armour {armor:.2}, speed {speed:.2}", state.level);
}

/// What the hero's powerups add up to this tick (`PowerBits`), put to
/// use: its weapon bits on its blows and missiles, its armour bits
/// against blows, speed powers on its speed (the game clamps the sum to its range), a turbo power's fill.
fn apply_powers(
    state: Option<Res<PlayerState>>,
    level: Option<Res<crate::monsters::MonsterLevel>>,
    mut players: Query<&mut Player>,
) {
    let Some(state) = state else { return };
    let b = state.bits;
    let boss_level = level.is_some_and(|l| l.boss >= 0);
    for mut p in &mut players {
        p.weapon = b.weapon;
        p.armour_bits = b.armour;
        p.special_bits = b.special;
        p.boss_level = boss_level;
        arm_death_hand(&mut p.death_hand, b.special);
        p.model_scale = if p.class == Some(OGRE) {
            OGRE_SCALE
        } else if b.special & power::GROW != 0 {
            GROWN_SCALE
        } else if state.level >= crate::player_state::MAX_LEVEL {
            TOP_LEVEL_SCALE
        } else {
            1.0
        };
        let speed = (p.base_speed + b.speed).clamp(locomotion::SPEED_MIN, locomotion::SPEED_MAX);
        if p.mover.speed != speed {
            p.mover.speed = speed;
        }
        if b.turbo > 0.0 {
            p.turbo = (p.turbo + b.turbo).min(TURBO_MAX);
        }
    }
}

/// The chrome and invisibility blink in their last this many seconds,
/// this many times a second (shown in the odd eighths).
const BLINKS_FROM: f32 = 3.0;
const BLINK_RATE: f32 = 8.0;
/// Invisibility: the body's transparency (of 255) wavers about this, by
/// this much, once a second.
const INVISIBLE: f32 = 160.0;
const INVISIBLE_WAVER: f32 = 16.0;

/// The longest-lasting of the hero's powers of `subtype` with any of
/// `bits`: its seconds left, negative for one that doesn't run out.
fn longest_power(state: &PlayerState, subtype: i32, bits: u32) -> Option<f32> {
    state
        .powers
        .iter()
        .filter(|p| p.subtype == subtype && p.value & bits != 0)
        .map(|p| p.time)
        .reduce(|a, b| if a < 0.0 || b < 0.0 { -1.0 } else { a.max(b) })
}

/// Whether a power with `t` seconds left shows its look: always, but in
/// its last three seconds only on the odd eighths.
fn blink_shows(t: f32) -> bool {
    t < 0.0 || t > BLINKS_FROM || (t * BLINK_RATE) as i32 % 2 == 1
}

/// The powers' looks on the hero's body (`docs/powers.md`), from the stats
/// routine:
/// - **Chrome** (invulnerability): the hero's timed texture effect is
///   re-armed with `CHROMEGOLD` (with the gold armour) or `CHROMESILVER`
///   every tick the longest invulnerability shows ([`blink_shows`]) and,
///   like a flash, it shows for the tick it's armed and the next. It takes
///   the slot from a hit flash.
/// - **Invisibility**: the body's transparency is 160 + 16 × sin(2π t)
///   while the longest invisibility shows, else none.
#[allow(clippy::type_complexity)]
fn show_body_looks(
    state: Option<Res<PlayerState>>,
    colours: Res<FlashColours>,
    mut players: Query<(&mut Player, &Animator)>,
    mut drawn: Query<&mut MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    (mut tags, mut commands): (Query<&mut MeshTag>, Commands),
) {
    let Some(state) = state else { return };
    let chrome_time = longest_power(&state, power::ARMOUR, crate::damage::resists::INVULNERABLE);
    let armed = chrome_time.is_some_and(blink_shows);
    let gold = state.bits.armour & crate::damage::resists::GOLD != 0;
    let fade = match longest_power(&state, power::SPECIAL, power::INVISIBLE) {
        Some(t) if blink_shows(t) => {
            let transparency = INVISIBLE + (INVISIBLE_WAVER * (std::f32::consts::TAU * t).sin()).trunc();
            transparency / 255.0
        }
        _ => 0.0,
    };
    for (mut p, animator) in &mut players {
        let p = &mut *p;
        if armed {
            p.chrome.start();
        }
        if p.chrome.step() {
            debug!("chrome {} (time {chrome_time:?})", if p.chrome.on() { "on" } else { "off" });
        }
        let texture = if p.chrome.on() { colours.chrome[usize::from(gold)].as_ref() } else { None };
        if texture.is_some() && p.flash.stop() {
            flash::tag_body(animator, |_| true, 0, &mut tags, &mut commands);
        }
        p.body_look.show(texture, fade, animator, &mut drawn, &mut materials);
    }
}

/// Armour bits: the fire wall and lightning shields, and all three
/// shields (with the reflect shield) — held on the left arm.
const FIRE_WALL: u32 = 0x20_0000;
const LIGHTNING_SHIELD: u32 = 0x40_0000;
const SHIELDS: u32 = 0x62_0000;
/// The fire wall burns what the hero walks into for 3 every tick; the
/// lightning shield strikes it for 20, heavy, once a second, with
/// `L_SHLD_ACTIVE` from the left wrist.
const FIRE_WALL_DAMAGE: f32 = 3.0;
const FIRE_WALL_KIND: u32 = 0x1;
const LIGHTNING_DAMAGE: f32 = 20.0;
const LIGHTNING_KIND: u32 = 0x22;
const LIGHTNING_COOLDOWN: f64 = 1.0;
const LIGHTNING_SPARK: &str = "L_SHLD_ACTIVE";

/// The hero's model scale: the ogre's (class 12) always; grown (the
/// special power `0x100`); at level 99.
const OGRE: usize = 12;
const OGRE_SCALE: f32 = 1.6;
const GROWN_SCALE: f32 = 1.3;
const TOP_LEVEL_SCALE: f32 = 1.2;
/// Grown, the hero's blows (the game's contact routine: melee, the
/// shields) do twice as much, and push with it.
const GROWN_BLOWS: f32 = 2.0;

/// The attack a power puts in place of the hero's own (the player
/// update's table, first that applies): Skorne's horns or mask breathe,
/// his left and right gauntlets fire (ATTFIREL, ATTFIRELR), the super
/// crossbow shoots (SSHOT1), the hammer chops (ATTCHOP), a breath power
/// breathes.
fn special_attack(special: u32, weapon: u32) -> Option<Action> {
    if special & 0x3000 != 0 {
        Some(Action::ATTBREATHE)
    } else if special & 0x8000 != 0 {
        Some(Action(0x67))
    } else if special & 0x4000 != 0 {
        Some(Action(0x68))
    } else if weapon & 0x10_0000 != 0 {
        Some(Action(0x6B))
    } else if weapon & HAMMER != 0 {
        Some(Action::ATTCHOP)
    } else if special & BREATHS != 0 {
        Some(Action::ATTBREATHE)
    } else {
        None
    }
}

/// The hammer's weapon bit.
const HAMMER: u32 = 0x1000_0000;
/// The special bits of the breath powers (fire, acid, lightning).
const BREATHS: u32 = 0x70;
/// A breath's reach, and the node it goes out from.
const BREATH_RADIUS: f32 = 20.0;
const HEAD: &str = "HEAD";

/// The breath the hero's powers give (first that applies): Skorne's horns
/// or mask (BOSS_BREATHE, 50 fire), fire (or the Pojo's), acid, lightning
/// (40, each with the heavy kind): effect, kind, damage, sound.
fn breath_of(special: u32) -> Option<(&'static str, u32, f32, Option<&'static str>)> {
    if special & 0x3000 != 0 {
        Some(("BOSS_BREATHE", 0x21, 50.0, Some(if special & 0x1000 != 0 { "S_HORNS" } else { "S_MASK" })))
    } else if special & 0x410 != 0 {
        // The Pojo alone has no breath sound of its own (it has its turbo's).
        Some(("FIREBREATHE", 0x21, 40.0, (special & 0x10 != 0).then_some("S_BREATHFIRE")))
    } else if special & 0x20 != 0 {
        Some(("ACIDBREATHE", 0x24, 40.0, Some("S_BREATHGAS")))
    } else if special & 0x40 != 0 {
        Some(("ELECBREATHE", 0x22, 40.0, Some("S_BREATHELEC")))
    } else {
        None
    }
}

/// A shield's blow on the target it touches.
fn shield_blow(hero: Entity, f: &combat::Found, damage: f32, kind: u32, facing: f32, grown: bool) -> Hit {
    let damage = if grown { damage * GROWN_BLOWS } else { damage };
    let sighted = matches!(f.kind, TargetKind::Monster | TargetKind::Object);
    let push = if sighted { combat::push(facing, damage) } else { Vec3::ZERO };
    Hit { target: f.entity, attacker: hero, damage, kind, push, at: f.position, target_kind: f.kind, ranged: false }
}

/// The action played with a shield on: SHIELD_READY for READY,
/// SHIELD_RUN for the walks and runs.
fn shield_action(action: Action) -> Action {
    match action.0 {
        0x00 => Action(0x15),
        0x11..=0x14 => Action(0x16),
        _ => action,
    }
}

/// Knockback speeds the hero's reactions add along the blow's push.
const STRONG_KNOCKBACK: f32 = 16.0;
const KNOCKDOWN_KNOCKBACK: f32 = 32.0;
/// The Pojo, being small, flies further when knocked down.
const POJO_KNOCKDOWN_KNOCKBACK: f32 = 80.0;
const FLING_KNOCKBACK: f32 = 100.0;
/// Blows of more than this flash the hero (and draw a reaction).
const HIT_FLASH_DAMAGE: f32 = 1.0;

/// The game's reaction class for the blows a hero took this tick (see
/// `docs/combat.md` "Blows that land on the hero"): 0 none, 1 a flinch,
/// 2 a stun (kind 0x80), 3 kind 0x2000, 10 a strong knockback, 20 a
/// knockdown, 30 fling; knockback classes gain 1 when the push comes from
/// behind the facing. Returns the class, the knockback speed, and the
/// heading to face (for the knockback classes).
fn hit_reaction(damage: f32, flags: u32, push: Vec3, facing: f32, pojo: bool) -> (u32, f32, Option<f32>) {
    if damage <= 1.0 {
        return (if flags & 0x80 != 0 { 2 } else { 0 }, 0.0, None);
    }
    let (mut class, speed) = if flags & 0x10000 != 0 {
        (30, FLING_KNOCKBACK)
    } else if flags & 0x40 != 0 {
        (20, FLING_KNOCKBACK)
    } else if flags & 0x120 != 0 {
        (20, if pojo { POJO_KNOCKDOWN_KNOCKBACK } else { KNOCKDOWN_KNOCKBACK })
    } else if flags & 0x10 != 0 {
        (10, STRONG_KNOCKBACK)
    } else if flags & 0x2000 != 0 {
        (3, 0.0)
    } else if flags & 0x80 != 0 {
        (2, 0.0)
    } else {
        (1, 0.0)
    };
    if class < 10 {
        return (class, speed, None);
    }
    let mut heading = push.x.atan2(push.z);
    if wrap(heading - facing).abs() > std::f32::consts::FRAC_PI_2 {
        class += 1;
        heading = wrap(heading + std::f32::consts::PI);
    }
    (class, speed, Some(heading))
}

/// The action a reaction class asks for (`None`: no override).
fn reaction_action(class: u32) -> Option<Action> {
    match class {
        10 | 11 => Some(Action(0x82)),
        20 => Some(Action(0x85)),
        21 => Some(Action(0x83)),
        30 | 31 => Some(Action(0x87)),
        _ => None,
    }
}

/// The Hand of Death's and the Health Vampire's special bits.
const HAND_OF_DEATH: u32 = 0x20_0000;
const HEALTH_VAMPIRE: u32 = 0x40_0000;

/// Arms the Hand of Death while its power is on, else the Health Vampire
/// while its is; clears both once neither is.
fn arm_death_hand(armed: &mut [bool; 2], special: u32) {
    if special & HAND_OF_DEATH != 0 {
        armed[0] = true;
    } else if special & HEALTH_VAMPIRE != 0 {
        armed[1] = true;
    } else {
        *armed = [false; 2];
    }
}

/// Blows of this or less don't knock the hero about.
const KNOCKLESS_DAMAGE: f32 = 2.0;

impl Player {
    /// Whether a monster's blow turns back on it (the Hand of Death or,
    /// first, the Health Vampire armed): `Some(vampire)`.
    pub fn turns_blows(&self) -> Option<bool> {
        match self.death_hand {
            [_, true] => Some(true),
            [true, false] => Some(false),
            _ => None,
        }
    }

    /// A blow lands on the hero (the game's hurt-player routine,
    /// `docs/combat.md`): through its armour and armour powers
    /// (`damage::resist`) — levitation dodging small monsters' blows —
    /// then queued for its next tick's reaction — a
    /// blow of 2 or less without its knockback, one armour stopped still
    /// with its stun. Returns the health it takes, for a [`DamagePlayer`]:
    /// negative for the gold armour's heal, which draws no reaction.
    ///
    /// [`DamagePlayer`]: crate::player_state::DamagePlayer
    pub fn take_blow(&mut self, damage: f32, kind: u32, push: Vec3) -> f32 {
        let mut kind = kind;
        let mut d = crate::damage::resist(damage, &mut kind, self.armor, self.armour_bits, self.boss_level);
        // Levitating, small monsters' blows don't reach it.
        if kind & combat::hit_kind::SMALL_MONSTER != 0 && self.special_bits & power::LEVITATE != 0 {
            d = 0.0;
        }
        if d >= 0.0 {
            if d <= KNOCKLESS_DAMAGE {
                kind &= !combat::hit_kind::KNOCKS;
            }
            self.pending_hit.0 += d;
            self.pending_hit.1 |= kind;
            self.pending_hit.2 += push;
        }
        d
    }

    /// Moves the hero instantly (no interpolation smear), standing on a
    /// fresh floor.
    pub fn teleport(&mut self, at: [f32; 3], facing: f32) {
        self.mover.position = at;
        self.mover.facing = facing;
        self.ground = PlayerGround::new(at[1]);
        self.previous = (at, facing);
    }
}

/// What the hero's tick sends: shots, potions, effects, breaths and spent
/// power uses.
type HeroWriters<'w> = (
    MessageWriter<'w, HeroShot>,
    MessageWriter<'w, UsePotion>,
    MessageWriter<'w, EffectAt>,
    MessageWriter<'w, BreathAt>,
    MessageWriter<'w, SpendPower>,
    MessageWriter<'w, ChopAt>,
);

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
            // Loaded again whenever the choice changes (the front end's
            // character select); the next level spawn uses it.
            .add_systems(Update, load_hero.run_if(resource_changed::<PlayerChoice>).before(PlayerSpawn))
            .add_systems(FixedUpdate, tick.in_set(PlayerTick))
            .add_systems(FixedUpdate, (apply_powers, show_body_looks).after(crate::player_state::PowersTick))
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
    // `GDL_WARP="x,y,z"`: start somewhere else (testing).
    let warped = std::env::var("GDL_WARP").ok().and_then(|s| {
        let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    });
    if let Some(at) = warped {
        feet = at;
    }
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
        base_speed: hero.speed,
        weapon: 0,
        armour_bits: 0,
        boss_level: false,
        special_bits: 0,
        model_scale: 1.0,
        death_hand: [false; 2],
        pending_hit: (0.0, 0, Vec3::ZERO),
        left_wrist: hero.data.skeleton.node_index(crate::power_looks::left_wrist(hero.class)),
        shield_cooldowns: Vec::new(),
        flash: Flash::default(),
        chrome: Flash::default(),
        body_look: BodyLook::default(),
        turbo: 0.0,
        turbo_cost: 0.0,
        radius: hero.radius,
        class: hero.class,
        attack_started: 0.0,
        magic: MagicState::default(),
        wall_hit: None,
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
        // Not on any GameCube scheme's buttons: for testing.
        "throwmagic" => button::THROW_MAGIC,
        "shieldmagic" => button::MAGIC_SHIELD,
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
    (free_look, boxes, scene): (Res<FreeLook>, Res<crate::message_box::MessageBox>, Res<crate::tower_scenes::Scene>),
    play_camera: Option<Res<PlayCamera>>,
    ground: Option<Res<LevelGround>>,
    mut controls: ResMut<Controls>,
    mut players: Query<(Entity, &mut Player, &mut Animator)>,
    targets: Query<(Entity, &GlobalTransform, &Targetable)>,
    mut hits: MessageWriter<Hit>,
    (mut shots, mut potions, mut effects, mut effects_breath, mut spent, mut chops): HeroWriters,
    mut hints: MessageWriter<ShowHint>,
    (colours, mut tags, mut commands): (Res<FlashColours>, Query<&mut MeshTag>, Commands),
    state: Option<Res<PlayerState>>,
    monster_level: Option<Res<crate::monsters::MonsterLevel>>,
    boss: (Option<Res<crate::critters::CritterLevel>>, Query<&GlobalTransform>),
) {
    let boss_level = monster_level.as_ref().is_some_and(|l| l.boss >= 0);
    // The boss intro's first wait (its state 2): the heroes stand still
    // and turn to face the boss (`docs/critters.md`).
    let (critters, bodies) = boss;
    let face_boss = critters
        .as_ref()
        .filter(|c| c.intro == 2)
        .and_then(|c| c.boss)
        .and_then(|b| bodies.get(b).ok())
        .map(|t| t.translation());
    // A dead hero lies still until it's revived.
    if state.as_ref().is_some_and(|s| !s.alive) {
        return;
    }
    let dt = time.delta_secs();
    // Blows during a camera cut do nothing (the game's damage routine
    // refuses them).
    let cut = play_camera.as_ref().is_some_and(|c| c.in_cut());
    controls.ticks += 1;
    // The pads aren't read during a camera cut (the game blocks them from
    // its start to its end), while the message box has them, or while the
    // tower's wizard announces something.
    let deaf = free_look.0 || cut || boxes.holds_input() || scene.holds_input();
    let raw = if deaf { Vec2::ZERO } else { read_stick(&keys, &pads) };
    let held = if deaf { 0 } else { read_buttons(&keys, &pads, &controls) };
    let buttons = Buttons::from_held(held, controls.held);
    controls.held = held;
    // Stick up moves the way the camera faces (the boss camera's on a boss
    // level); the game's heading is
    // the camera's yaw + the stick's angle, so right is +X facing +Z (on
    // the screen's right: the picture is mirrored, `camera.rs`).
    let yaw = play_camera.as_deref().map_or(0.0, PlayCamera::yaw);
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::new(forward.z, 0.0, -forward.x);
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
        let magnitude = if face_boss.is_some() { 0.0 } else { stick.magnitude * actions::stick_scale(current) };
        // Magic is ignored until let go after a use; while MAGICS or
        // THROWPOTIONS plays, it's watched for the double tap and the
        // throw's wind-up.
        let magic = p.magic.observe(held, current.0, FIELDS_PER_TICK);
        let unmagic = |b: u32| if magic.is_none() { b & !MAGIC_BUTTONS } else { b };
        let magic_buttons = Buttons { held: unmagic(buttons.held), pressed: unmagic(buttons.pressed) };
        let mut intent = combat::classify(magic_buttons, stick.magnitude, wrap(stick.heading - facing), p.turbo);
        let has_potions = state.as_ref().is_some_and(|s| !s.potions.is_empty());
        if intent == Intent::Magic && !has_potions {
            // No potion: the hint, and the hero moves as the stick says.
            hints.write(ShowHint(Hint::CollectMagicFirst));
            intent = combat::classify(Buttons::default(), stick.magnitude, 0.0, p.turbo);
        }
        p.actions.observe_buttons(buttons.held);
        let keeps_facing = intent.keeps_facing();
        let drive = if magnitude == 0.0 && !keeps_facing && actions::drifts_forward(current) { DRIFT } else { magnitude };
        let wanted = if magnitude > 0.0 && !keeps_facing { stick.heading } else { facing };

        // Blows taken: flinch, knockback or knockdown. A flinch only
        // interrupts standing and moving about; the rest override.
        let (mut hit_damage, mut hit_flags, mut hit_push) = std::mem::take(&mut p.pending_hit);
        // Invulnerable: no reaction at all.
        if cut || p.armour_bits & crate::damage::resists::INVULNERABLE != 0 {
            (hit_damage, hit_flags, hit_push) = (0.0, 0, Vec3::ZERO);
        }
        let pojo = p.special_bits & power::POJO != 0;
        let (reaction, knock, reaction_face) = hit_reaction(hit_damage, hit_flags, hit_push, facing, pojo);
        // A blow of more than a point flashes the hero.
        if hit_damage > HIT_FLASH_DAMAGE {
            p.flash.start();
        }
        if p.flash.step() {
            let tag = if p.flash.on() { colours.body().unwrap_or(0) } else { 0 };
            flash::tag_body(&animator, |_| true, tag, &mut tags, &mut commands);
        }
        if knock > 0.0 {
            let k = hit_push * knock;
            for (v, add) in p.mover.knockback.iter_mut().zip(k.to_array()) {
                *v += add;
            }
        }
        // The target the hero is heading for, and whether it walked into it
        // — or, with a fire wall or lightning shield, touched it with the
        // shield (not while a blow is making it react).
        let found = combat::search(position, wanted, candidates());
        let touching = found.filter(|f| reaction == 0 && f.distance < combat::WALK_INTO + p.radius);
        let shielded = touching.is_some() && p.armour_bits & (FIRE_WALL | LIGHTNING_SHIELD) != 0;
        let grown = p.special_bits & power::GROW != 0;
        if let Some(f) = touching.filter(|_| shielded) {
            if p.armour_bits & FIRE_WALL != 0 {
                // Standing counts as walking; walking or running into it
                // burns it every tick.
                if intent == Intent::Idle {
                    intent = Intent::Walk;
                }
                if matches!(intent, Intent::Walk | Intent::Run) {
                    hits.write(shield_blow(entity, &f, FIRE_WALL_DAMAGE, FIRE_WALL_KIND, facing, grown));
                }
            } else {
                let now = time.elapsed_secs_f64();
                p.shield_cooldowns.retain(|(_, until)| *until > now);
                if p.shield_cooldowns.iter().all(|(e, _)| *e != f.entity) {
                    hits.write(shield_blow(entity, &f, LIGHTNING_DAMAGE, LIGHTNING_KIND, facing, grown));
                    p.shield_cooldowns.push((f.entity, now + LIGHTNING_COOLDOWN));
                    // The shield's spark, from the left wrist toward it.
                    let wrist = p.left_wrist.and_then(|n| animator.bone(n)).and_then(|b| bodies.get(b).ok());
                    let at = wrist.map_or(position + Vec3::Y * body.centre_height, |t| t.translation());
                    let toward = combat::heading_of(f.position - at);
                    effects.write(EffectAt { name: LIGHTNING_SPARK, bank: None, at, facing: toward, scale: 1.0 });
                }
            }
        }
        let mut walked_into = false;
        if WALK_INTO_ATTACK
            && !shielded
            && reaction == 0
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

        let mut requested =
            combat::request(intent, p.actions.range, stick.magnitude, walked_into, p.actions.combo, p.request);
        // A power's own attack takes the place of every attack (not one
        // made by walking into something). Only the breath's and the
        // hammer's are done.
        if !walked_into
            && matches!(intent, Intent::Quick | Intent::Power | Intent::StrafeAttack(_))
            && let Some(a) = special_attack(p.special_bits, p.weapon).filter(|a| matches!(*a, Action::ATTBREATHE | Action::ATTCHOP))
        {
            requested = a;
        }
        if intent == Intent::Magic {
            requested = match magic {
                Some(MagicIntent::Throw) => Action::THROWPOTIONS,
                Some(MagicIntent::Shield) => {
                    p.magic.flags |= MagicState::SHIELD;
                    Action::MAGICS
                }
                _ => Action::MAGICS,
            };
        }
        // Turbo attacks: a full meter swings ATTPWRC, 40 or more ATTPWRB;
        // the Pojo breathes fire for 40.
        if intent == Intent::Turbo {
            if p.special_bits & power::POJO != 0 {
                if p.turbo >= TURBO_ATTACK {
                    requested = Action::ATTBREATHE;
                    p.turbo_cost = TURBO_ATTACK;
                }
            } else if p.turbo >= TURBO_MAX {
                requested = Action(0x57);
                p.turbo_cost = TURBO_MAX;
            } else if p.turbo >= TURBO_ATTACK {
                requested = Action(0x56);
                p.turbo_cost = TURBO_ATTACK;
            }
        }
        if let Some(a) = reaction_action(reaction) {
            requested = a;
        } else if reaction == 1 && matches!(intent, Intent::Idle | Intent::Walk | Intent::Run) {
            requested = Action::HITREACT;
        }
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
            magic_released: p.magic.flags & MagicState::RELEASED != 0,
        };
        let mut next = p.actions.next(requested, &env);
        // With a shield on its arm the hero stands and moves behind it.
        if p.armour_bits & SHIELDS != 0 {
            next.action = shield_action(next.action);
        }
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
            // A turbo attack pays for itself as it lands.
            if matches!(current.0, 0x56 | 0x57) {
                p.turbo = (p.turbo - p.turbo_cost).max(0.0);
                p.turbo_cost = 0.0;
            }
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
            // The game notes when an attack starts before it releases a
            // shot, so a strafe attack chaining into the next one throws
            // with no wind-up.
            if strike.0 & Strike::STARTED != 0 {
                p.attack_started = time.elapsed_secs_f64();
            }
            if next.action == Action::THROWPOTIONS {
                p.magic.charge = 0.0;
            }
            // A potion: the blast (the shield after a double tap), or the
            // throw (mode 3 with the throw button held). Not during a
            // camera cut.
            // The breath goes out as ATTBREATHE starts, and a use is spent.
            if strike.0 & Strike::BREATH != 0
                && let Some((fx, kind, damage, sound)) = breath_of(p.special_bits)
            {
                let head = animator.node(HEAD).and_then(|n| animator.bone(n));
                // The Pojo's breathes from its own head and pays its turbo.
                let pojo = p.special_bits & power::POJO != 0;
                if pojo {
                    p.turbo = (p.turbo - p.turbo_cost).max(0.0);
                    p.turbo_cost = 0.0;
                }
                let breath = BreathAt { hero: entity, head, fx, kind, damage, radius: BREATH_RADIUS, sound, pojo };
                effects_breath.write(breath);
                spent.write(SpendPower { subtype: power::SPECIAL, bits: BREATHS });
            }
            // The hammer comes down as its recovery starts, and a use is
            // spent.
            if strike.0 & Strike::CHOP != 0 {
                chops.write(ChopAt { hero: entity });
                spent.write(SpendPower { subtype: power::WEAPON, bits: HAMMER });
            }
            if strike.0 & (Strike::MAGIC | Strike::THROW_POTION) != 0 && !cut {
                let mode = if strike.0 & Strike::THROW_POTION != 0 {
                    if held & button::THROW_MAGIC != 0 { 3 } else { 2 }
                } else if p.magic.flags & MagicState::SHIELD != 0 {
                    1
                } else {
                    0
                };
                potions.write(UsePotion { hero: entity, feet: position, facing, mode, charge: p.magic.charge });
                p.magic.flags = MagicState::USED;
            }
            if strike.projectile() {
                // Aimed at what the search found, unless strafing or
                // defending (then along where the hero is heading).
                // The game aims from the release height at the target's
                // centre (a monster's `+0x54`), so the throw tilts up or down
                // to meet it.
                // On a boss level a throw looks much further out.
                let found = match found {
                    None if boss_level => combat::search_within(position, wanted, combat::BOSS_THROW_RANGE, candidates()),
                    f => f,
                };
                let aim = match found {
                    Some(f) if held & (button::DEFEND | button::STRAFE) == 0 => {
                        let from = position + Vec3::Y * projectiles::PLAYER_CENTRE;
                        let centre = f.position + Vec3::Y * (0.5 * f.height);
                        (centre - from).normalize_or(f.direction)
                    }
                    _ => Vec3::new(wanted.sin(), 0.0, wanted.cos()),
                };
                let wound_up = (time.elapsed_secs_f64() - p.attack_started) as f32;
                debug!("{} releases a projectile after {wound_up:.2} s", current.name());
                let targeted = found.is_some();
                shots.write(HeroShot { hero: entity, feet: position, facing, aim, targeted, strike, wound_up });
            }
        }
        p.last_clip = (animator.action, animator.frame);

        // The turbo meter: drains while charging, otherwise refills
        // (not during the turbo-class actions themselves).
        if p.actions.action.category().0 < 11 {
            if p.actions.action == Action::SHOVE {
                p.turbo = (p.turbo - TURBO_CHARGE_DRAIN * dt).max(0.0);
            } else {
                p.turbo = (p.turbo + TURBO_REGEN * dt).min(TURBO_MAX);
            }
        }

        // Facing: toward the stick, or held while strafing and defending;
        // attacking in place turns toward the target.
        let category = p.actions.action.category().0;
        let mut face = (magnitude > 0.0 && !keeps_facing).then_some(stick.heading);
        if ATTACK_AIM && (1..=10).contains(&category) && category != 7 && !strafing && drive == 0.0 {
            face = Some(aim);
        }
        if reaction_face.is_some() {
            face = reaction_face;
        }
        if let Some(b) = face_boss {
            face = Some((b.x - position.x).atan2(b.z - position.z));
        }

        // Movement: this tick's step uses the movement factor from the last
        // tick's chaining, turning uses this tick's.
        let heading = if drive > 0.0 && magnitude == 0.0 { facing } else { stick.heading };
        p.mover.factors = (p.move_factor, turn_factor);
        let d = p.mover.step(Stick { heading, magnitude: drive }, face, dt);
        p.move_factor = move_factor;
        let feet = p.mover.position;
        // Under the boss camera a step out of its view slides along it.
        let d = match play_camera.as_deref() {
            Some(c) => c.keep_in_view(feet, [feet[0], feet[1] + body.centre_height, feet[2]], d),
            None => d,
        };
        let d = match &ground {
            Some(g) => {
                let moved = g.0.move_player(feet, d, &body, &mut p.ground);
                p.wall_hit = moved.wall.map(|h| (h.node, h.point));
                moved.delta
            }
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
    let (mut damage, kind) = combat::blow(strike, p.strength, &found);
    if p.special_bits & power::GROW != 0 {
        damage *= GROWN_BLOWS;
    }
    let kind = kind | p.weapon;
    let push = if sighted { combat::push(facing, damage) } else { Vec3::ZERO };
    let at = position + Vec3::new(facing.sin(), 0.0, facing.cos()) * (combat::REACH + p.radius);
    Some(Hit { target: found.entity, attacker, damage, kind, push, at, target_kind: found.kind, ranged: false })
}

fn interpolate(fixed: Res<Time<Fixed>>, mut players: Query<(&Player, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (player, mut transform) in &mut players {
        let (p0, f0) = player.previous;
        let (p1, f1) = (player.mover.position, player.mover.facing);
        transform.translation = Vec3::from(p0).lerp(Vec3::from(p1), t);
        transform.rotation = Quat::from_rotation_y(f0 + locomotion::wrap(f1 - f0) * t);
        let scale = Vec3::splat(player.model_scale);
        if transform.scale != scale {
            transform.scale = scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_flinch_knock_back_or_down_like_the_game() {
        let from_front = Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(hit_reaction(0.5, 0, from_front, 0.0, false).0, 0, "a scratch does nothing");
        assert_eq!(hit_reaction(5.0, 0x4000_0000, from_front, 0.0, false).0, 1);
        let (class, speed, face) = hit_reaction(5.0, 0x10, from_front, 0.0, false);
        assert_eq!((class, speed), (11, 16.0), "pushed back by a blow from the front");
        assert!(face.unwrap().abs() < 1e-5, "turns to face the attacker");
        assert_eq!(hit_reaction(5.0, 0x20, -from_front, 0.0, false).0, 20);
        assert_eq!(reaction_action(21), Some(Action(0x83)));
        assert_eq!(hit_reaction(5.0, 0x20, -from_front, 0.0, true).1, 80.0, "the Pojo flies further");
    }

    #[test]
    fn powers_take_over_the_attacks_in_the_games_order() {
        assert_eq!(special_attack(0, 0), None);
        assert_eq!(special_attack(0x10, 0), Some(Action::ATTBREATHE));
        // The hammer comes before a breath, Skorne's horns before both.
        assert_eq!(special_attack(0x10, HAMMER), Some(Action::ATTCHOP));
        assert_eq!(special_attack(0x1010, HAMMER), Some(Action::ATTBREATHE));
        assert_eq!(special_attack(0x8000, 0), Some(Action(0x67)));
        assert_eq!(special_attack(0, 0x10_0000), Some(Action(0x6B)));
        // The Pojo's breath comes from its turbo, not the attacks.
        assert_eq!(special_attack(0x400, 0), None);
    }

    #[test]
    fn breaths_by_power() {
        assert_eq!(breath_of(0x20).map(|b| (b.0, b.1)), Some(("ACIDBREATHE", 0x24)));
        assert_eq!(breath_of(0x2040).map(|b| (b.0, b.2, b.3)), Some(("BOSS_BREATHE", 50.0, Some("S_MASK"))));
        assert_eq!(breath_of(0x400).map(|b| (b.0, b.3)), Some(("FIREBREATHE", None)));
        assert_eq!(breath_of(0), None);
    }

    #[test]
    fn the_hand_of_death_latches_before_the_vampire() {
        let mut p = [false; 2];
        arm_death_hand(&mut p, HAND_OF_DEATH | HEALTH_VAMPIRE);
        assert_eq!(p, [true, false]);
        arm_death_hand(&mut p, HEALTH_VAMPIRE);
        assert_eq!(p, [true, true], "the hand stays armed until neither is on");
        arm_death_hand(&mut p, 0);
        assert_eq!(p, [false, false]);
    }
}
