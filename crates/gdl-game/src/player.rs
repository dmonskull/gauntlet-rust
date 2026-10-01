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
//! | move | left stick | WASD (Shift walks) |
//! | attack (A) | south | J |
//! | power attack (Y) | north | L |
//! | turbo / defend (B) | west | H |
//! | magic (X) | east | U |
//! | charge (L) | left trigger | P |
//! | strafe (R) | right trigger | O |
//! | combo move (Z) | right bumper | G |
//! | the power menu | D-pad | arrows |
//!
//! `GDL_WARP="x,y,z"` starts the hero there; `GDL_STICK="x,y"` holds the stick; `GDL_BUTTONS="attack@10-12,power"`
//! holds buttons (`attack`, `power`, `turbo`, `magic`, `charge`, `strafe`,
//! `combo`, `up`, `down`, `left`, `right`), each for the whole run or for a range of ticks since the hero
//! appeared; `GDL_HOPS="x,y,z;x,y,z"` moves the hero onto each point in
//! turn, every `GDL_HOP_TICKS` ticks (default 120), on the first level.

use gdl_formats::detmath::Det;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use gdl_formats::{PlayerCollision, PlayerGround};
use gdl_formats::enemy::FIELDS_PER_TICK;
use gdl_formats::pdata::PlayerStats;

use crate::actions::{self, Action, ActionState, Env, Strike};
use crate::camera::FreeLook;
use crate::audio::{CALL_VOLUME, LoopSoundAt, PlaySoundAt};
use crate::character::{self, Animator, CharacterData};
use crate::monsters::DeathMonster;
use crate::combat::{self, Buttons, Hit, Intent, TargetKind, Targetable, button};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion::{self, Mover, Stick, wrap};
use crate::play_camera::PlayCamera;
use crate::player_state::{Cry, HurtHero, PlayerState, SpendPower, power};
use crate::options::GameOptions;
use crate::party::{Inputs, MAX_PLAYERS, Party, SlotInput};
use crate::population::LevelPopulation;
use crate::effects::{BreathAt, ChopAt, EffectAt, EffectOn, MAGIC_BUTTONS, MagicIntent, MagicState, UsePotion};
use crate::flash::{self, Flash, FlashColours};
use crate::fade::BodyLook;
use crate::going_out::{DeathLight, GoingOut};
use crate::hints::{Hint, ShowHint};
use crate::projectiles::{self, HeroShot};
use crate::world::{LevelEntity, LevelGround};

/// The hero's radius if the class data can't be read (every class has 1.5).
const DEFAULT_RADIUS: f32 = 1.5;
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

/// Which hero a player plays: the class's three-letter code and colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerChoice {
    pub class: String,
    pub variant: String,
}

/// Each slot's loaded hero, spawned again on every level: the choice it
/// was loaded for, and the hero.
#[derive(Resource, Default)]
struct HeroModels {
    slots: [Option<(PlayerChoice, Hero)>; MAX_PLAYERS],
}

/// A loaded hero: its model and the class's stats.
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
    /// Its player's slot (`party.rs`): its record, controls and panel.
    pub slot: usize,
    /// The buttons it held last tick (for presses).
    held: u32,
    /// Held by a critter's grab: the node it hangs from (none: where the
    /// grab found it) and where its model hangs, in that node's space and
    /// in the world's (`GrabHero`).
    pub grabbed: Option<(Option<Entity>, Vec3, Vec3)>,
    /// Thrown by a grab: the blow that lands once it's on a floor, and how
    /// far along it is (`ThrowHero`).
    thrown: Option<(f32, Throw)>,
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
    /// The halo: it has drunk from a Death since it came on (`S_HALO`
    /// once), and it's draining one now.
    halo_drank: bool,
    halo_draining: bool,
    /// Blows taken since the last tick: damage, kind flags, summed push
    /// directions (the game's `+0x8D0`, `+0x8D4`, `+0x8DC`).
    pending_hit: (f32, u32, Vec3),
    /// A blow's stun (the game's `+0x898`): until when (fixed-clock
    /// seconds) it holds a standing hero in STUN2, and the one the latest
    /// blow since the last tick brings.
    stun_until: f64,
    pending_stun: Option<f32>,
    /// The playing clip has ended or come round since it started.
    came_round: bool,
    /// Its left wrist's skeleton node (the lightning shield's spark
    /// leaves from it), and until when each target is spared its next
    /// timed blow (the lightning shield's, the charge's: the game keeps
    /// these per attacker on the target).
    left_wrist: Option<usize>,
    blow_cooldowns: Vec<(Entity, f64)>,
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
    /// Moved instantly since the item touch test last ran (`items.rs`
    /// takes it): that test starts from here, not from where the hero was.
    pub teleported: bool,
    /// Going out through an exit (`going_out.rs`; `items.rs` starts it),
    /// and the death light on the body (the same slot as the flash and the
    /// chrome; it wins over both).
    pub going_out: Option<GoingOut>,
    pub light: Option<DeathLight>,
}

/// The hero's working stats at a character level: strength (5–20),
/// armour (0–5) and speed (units/s), each from the class stat plus 5 per
/// level up to its maximum.
fn derived_stats(stats: Option<&PlayerStats>, level: u32, bought: &crate::player_state::StatBonus) -> (f32, f32, f32) {
    let at = |s: gdl_formats::pdata::Stat, b: f32| locomotion::stat_at_level(s.start, s.max, level, b);
    let strength = combat::strength(stats.map_or(400.0 + bought.strength, |s| at(s.strength, bought.strength)));
    let armor = (0.005 * stats.map_or(bought.armour, |s| at(s.armor, bought.armour))).clamp(0.0, 5.0);
    let speed = locomotion::move_speed(stats.map_or(400.0 + bought.speed, |s| at(s.speed, bought.speed)), 0.0);
    (strength, armor, speed)
}

/// Re-derives each hero's stats when its level or its bought points change.
fn level_stats(
    models: Res<HeroModels>,
    party: Res<Party>,
    mut applied: Local<[(u32, crate::player_state::StatBonus); MAX_PLAYERS]>,
    mut players: Query<&mut Player>,
) {
    let fresh: Vec<usize> = players.iter_mut().filter(|p| p.is_added()).map(|p| p.slot).collect();
    for (slot, state) in party.states() {
        let Some((_, hero)) = models.slots.get(slot).and_then(Option::as_ref) else { continue };
        if (state.level, state.bought) == applied[slot] && !fresh.contains(&slot) {
            continue;
        }
        applied[slot] = (state.level, state.bought);
        let (strength, armor, speed) = derived_stats(hero.stats.as_ref(), state.level, &state.bought);
        for mut p in players.iter_mut().filter(|p| p.slot == slot) {
            p.strength = strength;
            p.armor = armor;
            p.base_speed = speed;
            p.mover.speed = speed;
        }
        info!("player {} level {}: strength {strength:.1}, armour {armor:.2}, speed {speed:.2}", slot + 1, state.level);
    }
}

/// What the hero's powerups add up to this tick (`PowerBits`), put to
/// use: its weapon bits on its blows and missiles, its armour bits
/// against blows, speed powers on its speed (the game clamps the sum to its range), a turbo power's fill.
fn apply_powers(party: Res<Party>, level: Option<Res<crate::monsters::MonsterLevel>>, mut players: Query<&mut Player>) {
    let boss_level = level.is_some_and(|l| l.boss >= 0);
    for mut p in &mut players {
        let Some(state) = party.state(p.slot) else { continue };
        let b = state.bits;
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
        .active_powers()
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
/// - **The death light** (going out, a boss level's end): `DEATHLIGHT`'s
///   frame, drawn as a dying monster's death texture; it takes the slot.
#[allow(clippy::type_complexity)]
fn show_body_looks(
    party: Res<Party>,
    (colours, deaths): (Res<FlashColours>, Option<Res<crate::deaths::DeathTextures>>),
    mut players: Query<(&mut Player, &Animator)>,
    mut drawn: Query<&mut MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    (mut tags, mut commands): (Query<&mut MeshTag>, Commands),
) {
    let light_frames = deaths.and_then(|d| d.frames(crate::deaths::LIGHT));
    for (mut p, animator) in &mut players {
        let p = &mut *p;
        let Some(state) = party.state(p.slot) else { continue };
        let chrome_time = longest_power(state, power::ARMOUR, crate::damage::resists::INVULNERABLE);
        let armed = chrome_time.is_some_and(blink_shows);
        let gold = state.bits.armour & crate::damage::resists::GOLD != 0;
        let fade = match longest_power(state, power::SPECIAL, power::INVISIBLE) {
            Some(t) if blink_shows(t) => {
                let transparency = INVISIBLE + (INVISIBLE_WAVER * (std::f32::consts::TAU * t).dsin()).trunc();
                transparency / 255.0
            }
            _ => 0.0,
        };
        if armed {
            p.chrome.start();
        }
        if p.chrome.step() {
            debug!("chrome {} (time {chrome_time:?})", if p.chrome.on() { "on" } else { "off" });
        }
        let light = p.light.as_ref().zip(light_frames.as_ref()).map(|(l, f)| f[l.frame().min(f.len() - 1)].clone());
        let texture = if p.chrome.on() && light.is_none() { colours.chrome[usize::from(gold)].as_ref() } else { None };
        if (texture.is_some() || light.is_some()) && p.flash.stop() {
            flash::tag_body(animator, |_| true, 0, &mut tags, &mut commands);
        }
        p.body_look.show(texture, light.as_ref(), fade, animator, &mut drawn, &mut materials);
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
/// The charge's blow on what it runs into, and how long each target is
/// spared another.
const CHARGE_DAMAGE: f32 = 3.0;
const CHARGE_COOLDOWN: f64 = 1.0;

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
        Some(Action::ATTFIREL)
    } else if special & 0x4000 != 0 {
        Some(Action::ATTFIRELR)
    } else if weapon & 0x10_0000 != 0 {
        Some(Action::SSHOT1)
    } else if weapon & HAMMER != 0 {
        Some(Action::ATTCHOP)
    } else if special & BREATHS != 0 {
        Some(Action::ATTBREATHE)
    } else {
        None
    }
}

/// The halo (armour bit): its drain of a Death, a point a tick, with
/// `S_HALO` the first time (panned at the hero's top, `0xE0`),
/// `S_DEATHDIE` going on at the Death, Death's drain sound at the hero's
/// feet (`S_DEATHSUCK`, looping) and Death's drain effect on the hero.
const HALO: u32 = 0x8_0000;
const HALO_DRAIN: f32 = 1.0;
const HALO_SOUND: &str = "S_HALO";
const HALO_VOLUME: u8 = 0xE0;
const HALO_LOOP: &str = "halo_drain";
const HALO_SUCK_LOOP: &str = "halo_suck";
const DEATH_DIES: &str = "S_DEATHDIE";
const DEATH_SUCK: &str = "S_DEATHSUCK";
/// A potion going up as the magic shield (defending): its sound at the
/// hero's collision centre.
const TURBO_DEFENSE: &str = "S_TURBODEFENSE";
const DEATH_BANK: &str = "MONSTERS/DEATH";
const DEATH_ARC: &str = "DEATH_ARC";
const DEATH_EXP: &str = "DEATH_EXP";

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
        0x00 => Action::SHIELD_READY,
        0x11..=0x14 => Action::SHIELD_RUN,
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
    let mut heading = push.x.datan2(push.z);
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

/// How long a blow that does damage stuns the hero, standing (seconds):
/// poison a second, Death's drain a fifteenth (both: the drain's).
fn blow_stun(kind: u32) -> Option<f32> {
    if kind & combat::hit_kind::DRAIN != 0 {
        Some(1.0 / 15.0)
    } else if kind & combat::hit_kind::POISON != 0 {
        Some(1.0)
    } else {
        None
    }
}

/// The game's reaction class once the action playing and a blow's stun
/// are counted (its reaction prologue; the knock-downs' own part is
/// `hit_reaction`'s and the chaining's): standing while a stun lasts is
/// 100 (STUN2) — not while falling or reeling from a knockback — else a
/// stunning blow's reaction goes on while it plays (3 HITREACT `0x81`, 2
/// STUN1).
fn stun_class(class: u32, current: Action, stunned_standing: bool) -> u32 {
    match current.0 {
        0x83 | 0x85 => class,
        0x82 if class < 10 => class,
        _ if stunned_standing => 100,
        0x81 if class < 1 => 3,
        0x7F if class < 1 => 2,
        _ => class,
    }
}

/// What the stuns ask for (the hero stands still meanwhile): STUN2; a
/// stunning blow's STUN1 or HITREACT, then standing while it plays.
fn stun_action(class: u32, current: Action) -> Option<Action> {
    match class {
        100 => Some(Action::STUN2),
        2 | 3 if matches!(current, Action::STUN1 | Action::STUNREACT) => Some(Action::READY),
        2 => Some(Action::STUN1),
        3 => Some(Action::STUNREACT),
        _ => None,
    }
}

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
        // Going out, no blow reaches it (the game hurts only heroes in
        // play).
        if self.going_out.is_some() {
            return 0.0;
        }
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
        if d > 0.0
            && let Some(stun) = blow_stun(kind)
        {
            self.pending_stun = Some(stun);
        }
        d
    }

    /// Moves the hero instantly (no interpolation smear), standing on a
    /// fresh floor; the item touch test starts again from there rather
    /// than sweeping the jump (`teleported`).
    pub fn teleport(&mut self, at: [f32; 3], facing: f32) {
        self.mover.position = at;
        self.mover.facing = facing;
        self.ground = PlayerGround::new(at[1]);
        self.previous = (at, facing);
        self.teleported = true;
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
    MessageWriter<'w, PlaySoundAt>,
    MessageWriter<'w, LoopSoundAt>,
    MessageWriter<'w, EffectOn>,
);

/// A critter's grab holds the hero (`critters.rs`; `docs/critters.md`,
/// "7 — grab"): its model hangs at `offset` in the node's space (at `at`
/// with no node), and it neither moves, attacks nor reacts, playing
/// GRABBED, until thrown or let go. Its own place doesn't change.
#[derive(Message, Clone, Copy, Debug)]
pub struct GrabHero {
    pub hero: Entity,
    pub node: Option<Entity>,
    pub offset: Vec3,
    pub at: Vec3,
}

/// The grab throws the hero it holds: let go where it was taken, knocked
/// along `push` (the knockback set to it, FALLDOWN), and `damage` lands
/// once it's within 0.2 of a floor, the tick after.
#[derive(Message, Clone, Copy, Debug)]
pub struct ThrowHero {
    pub hero: Entity,
    pub damage: f32,
    pub push: Vec3,
}

/// A grab lets the hero go without a throw (its critter dying).
#[derive(Message, Clone, Copy, Debug)]
pub struct ReleaseHero {
    pub hero: Entity,
}

/// A throw's course: the fall to start, in the air, landed (its blow
/// lands this tick).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Throw {
    Starts,
    Flies,
    Landed,
}

/// A thrown hero has landed this close to a floor.
const THROWN_LANDS: f32 = 0.2;

fn take_grabs(
    mut grabs: MessageReader<GrabHero>,
    mut throws: MessageReader<ThrowHero>,
    mut releases: MessageReader<ReleaseHero>,
    mut players: Query<&mut Player>,
) {
    for g in grabs.read() {
        if let Ok(mut p) = players.get_mut(g.hero) {
            p.grabbed = Some((g.node, g.offset, g.at));
            p.thrown = None;
        }
    }
    for t in throws.read() {
        if let Ok(mut p) = players.get_mut(t.hero) {
            p.grabbed = None;
            p.mover.knockback = t.push.to_array();
            p.thrown = Some((t.damage, Throw::Starts));
        }
    }
    for r in releases.read() {
        if let Ok(mut p) = players.get_mut(r.hero) {
            p.grabbed = None;
        }
    }
}

/// Player movement; the play camera ticks after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerTick;

/// Each hero's pad as the player's tick read it, by slot: the buttons
/// pressed since a reader last took them (with `std::mem::take`) — none
/// while the pads aren't read (cuts, the message box).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct HeroPad {
    pub pressed: [u32; MAX_PLAYERS],
}

/// Spawning the hero on a new level; things placed relative to it go after.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerSpawn;

/// Scripted buttons (`GDL_BUTTONS`, for the first player), and ticks since
/// the heroes appeared.
#[derive(Resource, Default)]
pub(crate) struct Controls {
    script: Vec<(u32, Option<(u64, u64)>)>,
    /// `GDL_STICK="x,y"`: the first player's stick, held throughout.
    stick: Option<Vec2>,
    ticks: u64,
}

impl Controls {
    /// The scripted stick and buttons for tick `tick`, over a player's own.
    fn apply_script(&self, input: &mut SlotInput, tick: u64) {
        if let Some(v) = self.stick {
            input.stick = v;
        }
        for &(bits, range) in &self.script {
            if range.is_none_or(|(a, b)| (a..=b).contains(&tick)) {
                input.held |= bits;
            }
        }
    }
}

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(locomotion::TICK_HZ))
            .insert_resource(Controls { script: button_script(), stick: script_stick(), ..default() })
            .init_resource::<HeroPad>()
            .init_resource::<HeroModels>()
            .init_resource::<Inputs>()
            // Loaded again whenever a player's choice changes (the front
            // end's character select); the next level spawn uses it.
            .add_systems(Update, load_heroes.run_if(resource_changed::<Party>).before(PlayerSpawn))
            .add_message::<GrabHero>()
            .add_message::<ThrowHero>()
            .add_message::<ReleaseHero>()
            .add_systems(
                FixedUpdate,
                (
                    (gather_inputs.run_if(crate::online::lockstep_off), hop).before(PlayerTick),
                    (take_grabs, tick).chain().in_set(PlayerTick),
                ),
            )
            // Online the controls come from the session's bundles; this
            // machine's go out every frame (`online.rs`).
            .add_systems(
                bevy::app::RunFixedMainLoop,
                sample_online
                    .run_if(crate::online::lockstep_on)
                    .in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop)
                    .before(crate::online::drive),
            )
            .add_systems(FixedUpdate, (apply_powers, show_body_looks).after(crate::player_state::PowersTick))
            .add_systems(Update, level_stats)
            .add_systems(
                Update,
                (spawn_player.run_if(resource_exists_and_changed::<LevelPopulation>).in_set(PlayerSpawn), interpolate)
                    .chain(),
            );
    }
}

/// Loads each player's hero when its choice changes (none for an empty
/// slot).
fn load_heroes(mut models: ResMut<HeroModels>, mut game: ResMut<LoadedGame>, party: Res<Party>) {
    for slot in 0..MAX_PLAYERS {
        let wanted = party.choice(slot);
        if wanted == models.slots[slot].as_ref().map(|(c, _)| c) {
            continue;
        }
        models.slots[slot] = wanted.and_then(|c| Some((c.clone(), load_hero(&mut game.install, c)?)));
    }
}

fn load_hero(install: &mut gdl_install::GameInstall, choice: &PlayerChoice) -> Option<Hero> {
    let data = match character::load_player(install, &choice.class, &choice.variant) {
        Ok(data) => data,
        Err(why) => {
            error!("can't load player {}/{}: {why}", choice.class, choice.variant);
            return None;
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
    Some(Hero { stats, data, speed, strength, armor, radius, class })
}

/// Each player's place in the game's start square, by slot, in steps of
/// half a unit more than the first hero's radius (turned by the start
/// camera's yaw); and the sixteen spots round the first hero tried when
/// that one has no floor.
const FORMATION: [(f32, f32); MAX_PLAYERS] = [(0.0, 0.0), (0.0, 2.0), (-2.0, 2.0), (-2.0, 0.0)];
const AROUND: [(f32, f32); 16] = [
    (-2.0, -2.0),
    (-2.0, 0.0),
    (-2.0, 2.0),
    (0.0, -2.0),
    (0.0, 2.0),
    (2.0, -2.0),
    (2.0, 0.0),
    (2.0, 2.0),
    (-2.0, -1.0),
    (-2.0, 1.0),
    (-1.0, -2.0),
    (-1.0, 2.0),
    (1.0, -2.0),
    (1.0, 2.0),
    (2.0, -1.0),
    (2.0, 1.0),
];
/// A spot's floor must be within this of the first hero's.
const START_FLOOR_REACH: f32 = 3.0;

/// Where a hero after the first starts: its place in the square beside the
/// first, else one of the sixteen spots round it, else on it.
fn start_spot(
    collision: &gdl_formats::LevelCollision,
    first: [f32; 3],
    radius: f32,
    first_slot: usize,
    slot: usize,
    yaw: f32,
) -> [f32; 3] {
    let step = 0.5 + radius;
    let floor_at = |x: f32, z: f32| {
        let y = collision.player_floor_height([x, first[1], z], PlayerCollision::default().radius)?;
        ((y - first[1]).abs() <= START_FLOOR_REACH).then_some([x, y, z])
    };
    let (mine, theirs) = (FORMATION[slot.min(MAX_PLAYERS - 1)], FORMATION[first_slot.min(MAX_PLAYERS - 1)]);
    let (dx, dz) = (step * (mine.0 - theirs.0), step * (mine.1 - theirs.1));
    let (sin, cos) = yaw.dsin_cos();
    floor_at(first[0] + dx * cos + dz * sin, first[2] - dx * sin + dz * cos)
        .or_else(|| AROUND.iter().find_map(|&(x, z)| floor_at(first[0] + step * x, first[2] + step * z)))
        .unwrap_or(first)
}

#[allow(clippy::too_many_arguments)]
fn spawn_player(
    mut commands: Commands,
    (party, models): (Res<Party>, Res<HeroModels>),
    population: Res<LevelPopulation>,
    ground: Option<Res<LevelGround>>,
    mut controls: ResMut<Controls>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut warp_used: Local<bool>,
) {
    let Some(ground) = ground else { return };
    let collision = &ground.0;
    let (mut feet, facing) = match population.player_start() {
        Some(s) => (s.position, s.yaw),
        None => {
            let [lo, hi] = collision.bounds;
            (std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0), 0.0)
        }
    };
    // `GDL_WARP="x,y,z"`: start somewhere else (testing) — on the first
    // level only: a later level (after dying, an exit) starts at its own
    // start.
    let warped = std::env::var("GDL_WARP").ok().filter(|_| !std::mem::replace(&mut *warp_used, true)).and_then(|s| {
        let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    });
    // Only onto a floor the player's own floor check stands on (the items'
    // probe also finds floors players fall through): with none under the
    // point the hero would fall out of the level, back to the point, for
    // good.
    let warped = warped.and_then(|at| {
        let y = collision.player_floor_height(at, PlayerCollision::default().radius);
        if y.is_none() {
            warn!("GDL_WARP: no floor a player stands on under {at:?}; starting at the level's start");
        }
        y.map(|y| [at[0], y, at[2]])
    });
    if let Some(at) = warped {
        feet = at;
    } else if let Some(y) = collision.floor_height(feet).or_else(|| collision.top_floor(feet[0], feet[2])) {
        // Stand on the floor under the start.
        feet[1] = y;
    }
    // The start square turns with the entry's starting camera (its yaw as
    // the camera table keeps it, the locator's + π).
    let entry = population.entry;
    let yaw = population
        .population
        .locators
        .iter()
        .find(|l| l.kind == gdl_formats::population::LocatorKind::Transmitter(1) && l.index == entry)
        .map_or(0.0, |l| l.rotation[1] + std::f32::consts::PI);
    let mut first: Option<([f32; 3], f32, usize)> = None;
    for (slot, _) in party.members() {
        let Some((_, hero)) = models.slots.get(slot).and_then(Option::as_ref) else { continue };
        let at = match first {
            None => feet,
            Some((at, radius, first_slot)) => start_spot(collision, at, radius, first_slot, slot, yaw),
        };
        first.get_or_insert((at, hero.radius, slot));
        spawn_hero(&mut commands, hero, slot, at, facing, (&mut meshes, &mut materials, &mut images));
        info!("player {} starts at {at:?} facing {:.0} deg", slot + 1, facing.to_degrees());
    }
    controls.ticks = 0;
}

fn spawn_hero(
    commands: &mut Commands,
    hero: &Hero,
    slot: usize,
    feet: [f32; 3],
    facing: f32,
    (meshes, materials, images): (&mut Assets<Mesh>, &mut Assets<LevelMaterial>, &mut Assets<Image>),
) {
    let transform = Transform::from_translation(Vec3::from(feet)).with_rotation(Quat::from_rotation_y(facing));
    let (root, _, _) = character::spawn_character(&hero.data, transform, commands, meshes, materials, images);
    let player = Player {
        slot,
        held: 0,
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
        halo_drank: false,
        halo_draining: false,
        pending_hit: (0.0, 0, Vec3::ZERO),
        grabbed: None,
        thrown: None,
        stun_until: 0.0,
        pending_stun: None,
        came_round: false,
        left_wrist: hero.data.skeleton.node_index(crate::power_looks::left_wrist(hero.class)),
        blow_cooldowns: Vec::new(),
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
        teleported: false,
        going_out: None,
        light: None,
    };
    commands.entity(root).insert((player, LevelEntity));
}

/// `GDL_HOPS` (testing: a tour of a level's triggers): the points, the ticks
/// between hops, the first level's name and the next hop.
type Hops = (Vec<[f32; 3]>, u64, String, usize);

/// Moves the hero onto the next `GDL_HOPS` point every `GDL_HOP_TICKS`
/// ticks since it appeared, on the first level only, standing on the floor
/// the player's own check finds under it.
fn hop(
    mut hops: Local<Option<Hops>>,
    controls: Res<Controls>,
    population: Option<Res<LevelPopulation>>,
    ground: Option<Res<LevelGround>>,
    mut players: Query<&mut Player>,
) {
    let (Some(population), Some(ground)) = (population, ground) else { return };
    let (points, every, level, next) = hops.get_or_insert_with(|| {
        let env = |name| std::env::var(name).unwrap_or_default();
        let points = env("GDL_HOPS")
            .split(';')
            .filter_map(|p| {
                let v: Vec<f32> = p.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() == 3).then(|| [v[0], v[1], v[2]])
            })
            .collect();
        let every = env("GDL_HOP_TICKS").parse().ok().filter(|&n| n > 0).unwrap_or(120);
        (points, every, population.level.clone(), 0)
    });
    if population.level != *level || controls.ticks < *every * (*next as u64 + 1) {
        return;
    }
    let Some(&at) = points.get(*next) else { return };
    let i = *next;
    *next += 1;
    let Some(y) = ground.0.player_floor_height(at, PlayerCollision::default().radius) else {
        warn!("GDL_HOPS: no floor a player stands on under hop {i} {at:?}");
        return;
    };
    for mut p in &mut players {
        let facing = p.mover.facing;
        p.teleport([at[0], y, at[2]], facing);
    }
    info!("hop {i} to {:?}", [at[0], y, at[2]]);
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
        "up" => button::DPAD_UP,
        "down" => button::DPAD_DOWN,
        "left" => button::DPAD_LEFT,
        "right" => button::DPAD_RIGHT,
        _ => return None,
    })
}

/// `GDL_STICK="x,y"` (testing): the first player's stick.
fn script_stick() -> Option<Vec2> {
    let s = std::env::var("GDL_STICK").ok()?;
    let (x, y) = s.split_once(',')?;
    Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
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

/// Each local player's controls this tick ([`Inputs`]): the keyboard and
/// mouse if they're the player's, and the player's pad — whichever is
/// moved, so a player can switch between them as they please. Alone, the
/// player has the keyboard and every pad nobody else holds. The first
/// player also gets the test scripts (`GDL_STICK`, `GDL_BUTTONS`).
/// Players online are filled from the network instead.
fn gather_inputs(
    (keys, mouse, options): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>, Res<GameOptions>),
    pads: Query<(Entity, &Gamepad)>,
    party: Res<Party>,
    mut controls: ResMut<Controls>,
    mut inputs: ResMut<Inputs>,
) {
    controls.ticks += 1;
    let locals: Vec<usize> = party.members().filter(|(_, m)| !m.devices.remote).map(|(i, _)| i).collect();
    let solo = locals.len() == 1;
    for slot in 0..MAX_PLAYERS {
        let Some(member) = party.get(slot) else {
            inputs.slots[slot] = SlotInput::default();
            continue;
        };
        if member.devices.remote {
            continue;
        }
        let mut input = read_devices(member.devices, solo, slot, (&keys, &mouse, &options), &pads, &party);
        if locals.first() == Some(&slot) {
            controls.apply_script(&mut input, controls.ticks);
        }
        inputs.slots[slot] = input;
    }
}

/// What a local player's devices give now: the keyboard and mouse, their
/// pad — alone on this machine, every pad nobody else holds — and their
/// own settings (`settings` is the slot whose options are theirs) riding
/// along with the buttons.
fn read_devices(
    devices: crate::party::Devices,
    solo: bool,
    settings: usize,
    (keys, mouse, options): (&ButtonInput<KeyCode>, &ButtonInput<MouseButton>, &GameOptions),
    pads: &Query<(Entity, &Gamepad)>,
    party: &Party,
) -> SlotInput {
    let mut input = SlotInput::default();
    let mine = options.player(settings);
    if devices.keyboard || solo {
        input.held |= crate::controls::held_keys(keys, mouse, options);
        input.stick = crate::controls::stick_keys(keys, options);
        if [KeyCode::Escape, KeyCode::Backspace, KeyCode::KeyH].iter().any(|&k| keys.pressed(k)) {
            input.held |= SlotInput::BACK;
        }
    }
    for (pad_entity, pad) in pads {
        let ours = devices.pad == Some(pad_entity) || (solo && party.slot_of_pad(pad_entity).is_none());
        if !ours {
            continue;
        }
        input.held |= crate::controls::held_pad(pad, mine.scheme, options);
        let (left, right) = crate::controls::pad_sticks(pad);
        if left.length() > 0.0 {
            input.stick = left;
        }
        if right.length() > 0.0 {
            input.c_stick = right;
        }
        // The GameCube's B (west); an Xbox pad's B (east).
        if pad.pressed(GamepadButton::West) || pad.pressed(GamepadButton::East) {
            input.held |= SlotInput::BACK;
        }
    }
    input.with_settings(mine)
}

/// Online: this machine's player's controls every frame, for the session
/// (`online.rs`). A menu open over play takes them; their settings are
/// player 1's on this machine. `GDL_BUTTONS`/`GDL_STICK` count online
/// ticks here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sample_online(
    (keys, mouse, options): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>, Res<GameOptions>),
    pads: Query<(Entity, &Gamepad)>,
    party: Res<Party>,
    controls: Res<Controls>,
    online: Option<Res<crate::online::Online>>,
    lock: Res<crate::online::Lockstep>,
    fe: Option<Res<crate::frontend::Frontend>>,
    mut local: ResMut<crate::online::LocalControls>,
) {
    let Some(me) = online.and_then(|o| o.me) else { return };
    let Some(member) = party.get(me) else {
        local.0 = SlotInput::default();
        return;
    };
    let mut input = read_devices(member.devices, true, 0, (&keys, &mouse, &options), &pads, &party);
    if fe.is_some_and(|f| f.menu_open()) {
        input = SlotInput::default().with_settings(options.player(0));
    }
    controls.apply_script(&mut input, u64::from(lock.tick) + 1);
    // A menu's command rides along for a few frames.
    if let Some((bits, frames)) = local.1 {
        input.held |= bits;
        local.1 = (frames > 1).then_some((bits, frames - 1));
    }
    local.0 = input;
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
    inputs: Res<Inputs>,
    (free_look, boxes, scene): (Res<FreeLook>, Res<crate::message_box::MessageBox>, Res<crate::tower_scenes::Scene>),
    play_camera: Option<Res<PlayCamera>>,
    ground: Option<Res<LevelGround>>,
    mut players: Query<(Entity, &mut Player, &mut Animator)>,
    targets: Query<(Entity, &GlobalTransform, &Targetable)>,
    mut hits: MessageWriter<Hit>,
    (mut shots, mut potions, mut effects, mut effects_breath, mut spent, mut chops, mut sounds, mut loops, mut riding): HeroWriters,
    mut hints: MessageWriter<ShowHint>,
    (colours, mut tags, mut commands, mut hero_pad, mut hurt): (Res<FlashColours>, Query<&mut MeshTag>, Commands, ResMut<HeroPad>, MessageWriter<HurtHero>),
    party: Res<Party>,
    monster_level: Option<Res<crate::monsters::MonsterLevel>>,
    boss: (Option<Res<crate::critters::CritterLevel>>, Query<&GlobalTransform>, Query<&DeathMonster>),
) {
    let boss_level = monster_level.as_ref().is_some_and(|l| l.boss >= 0);
    // The boss intro's first wait (its state 2): the heroes stand still
    // and turn to face the boss (`docs/critters.md`).
    let (critters, bodies, deaths) = boss;
    let face_boss = critters
        .as_ref()
        .filter(|c| c.intro == 2)
        .and_then(|c| c.boss)
        .and_then(|b| bodies.get(b).ok())
        .map(|t| t.translation());
    let dt = time.delta_secs();
    // Blows during a camera cut do nothing (the game's damage routine
    // refuses them).
    let cut = play_camera.as_ref().is_some_and(|c| c.in_cut());
    // The pads aren't read during a camera cut (the game blocks them from
    // its start to its end), while the message box has them, or while the
    // tower's wizard announces something.
    let deaf = free_look.0 || cut || boxes.holds_input() || scene.holds_input();
    // Stick up moves the way the camera faces (the boss camera's on a boss
    // level); the game's heading is
    // the camera's yaw + the stick's angle, so right is +X facing +Z (on
    // the screen's right: the picture is mirrored, `camera.rs`).
    // Online each hero's stick turns by its own camera (`play_camera.rs`).
    let axes = |slot: usize| {
        let yaw = play_camera.as_deref().map_or(0.0, |c| c.yaw_of(slot));
        let forward = Vec3::new(yaw.dsin(), 0.0, yaw.dcos());
        (forward, Vec3::new(forward.z, 0.0, -forward.x))
    };
    let body = PlayerCollision::default();
    let candidates = || targets.iter().map(|(e, t, target)| (e, t.translation(), target));

    for (entity, mut player, mut animator) in &mut players {
        let p = &mut *player;
        // A dead hero lies still until it's revived.
        let Some(state) = party.state(p.slot) else { continue };
        if !state.alive {
            continue;
        }
        // The player's own settings ride with their controls (`party.rs`).
        let mine = inputs.slots.get(p.slot).copied().unwrap_or_default();
        let input = if deaf { SlotInput::default() } else { mine };
        let (raw, held) = (input.stick, input.buttons());
        let buttons = Buttons::from_held(held, p.held);
        p.held = held;
        // For what else reads the pad (the power menu): presses wait until
        // they're taken.
        if let Some(pressed) = hero_pad.pressed.get_mut(p.slot) {
            *pressed |= buttons.pressed;
        }
        let (forward, right) = axes(p.slot);
        let dir = right * raw.x + forward * raw.y;
        let stick = Stick { heading: dir.x.datan2(dir.z), magnitude: raw.length().min(1.0) };
        // The Robotron style's right stick (the GameCube's C-stick).
        let c_raw = if !mine.robotron() { Vec2::ZERO } else { input.c_stick };
        let c_dir = right * c_raw.x + forward * c_raw.y;
        let c_stick = Stick { heading: c_dir.x.datan2(c_dir.z), magnitude: c_raw.length().min(1.0) };
        p.previous = (p.mover.position, p.mover.facing);
        if p.light.as_mut().is_some_and(|l| !l.step()) {
            p.light = None;
        }
        // Going out through an exit: no control and no blows; it sinks and
        // spins, then it's gone (`going_out.rs`).
        if let Some(out) = &mut p.going_out {
            let (mut feet, mut facing) = (p.mover.position, p.mover.facing);
            let was_there = !out.gone();
            if !out.tick(&mut feet, &mut facing, body.half_height, dt, &mut p.light) && was_there {
                commands.entity(entity).insert(Visibility::Hidden);
            }
            (p.mover.position, p.mover.facing) = (feet, facing);
            p.pending_hit = (0.0, 0, Vec3::ZERO);
            p.pending_stun = None;
            continue;
        }
        let position = Vec3::from(p.mover.position);
        let facing = p.mover.facing;
        let current = p.actions.action;

        // Held by a grab: GRABBED loops and the hero does nothing else (its
        // movement, floor check and attacks are skipped; blows don't make
        // it react).
        if p.grabbed.is_some() {
            p.pending_hit = (0.0, 0, Vec3::ZERO);
            p.mover.knockback = [0.0; 3];
            let clip = clip_for(&animator, Action::GRABBED);
            if animator.action != clip || animator.finished() {
                animator.play(clip);
                p.actions.switched(Action::GRABBED, p.class);
            }
            continue;
        }
        // A throw's blow lands the tick after the hero came down on a floor
        // (through the damage routine: kind 0, no push, the hurt sound).
        if let Some((damage, Throw::Landed)) = p.thrown {
            p.thrown = None;
            let amount = p.take_blow(damage, 0, Vec3::ZERO);
            if amount != 0.0 {
                hurt.write(HurtHero { slot: p.slot, amount, kind: 0, cry: Cry::Hurt });
            }
            info!("the thrown hero lands: {amount:.1}");
        }

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
        let has_potions = !state.potions.is_empty();
        if intent == Intent::Magic && !has_potions {
            // No potion: the hint, and the hero moves as the stick says.
            hints.write(ShowHint::to(p.slot, Hint::CollectMagicFirst));
            intent = combat::classify(Buttons::default(), stick.magnitude, 0.0, p.turbo);
        }
        // The Robotron style (`docs/combat.md`, "Intents"): after magic,
        // turbo and defend, the right stick pushed attacks toward it — the
        // power attack with its button held — or, with the left stick
        // pushed too, strafe-attacks to its side of the left stick.
        let mut c_aim = None;
        if c_stick.magnitude > 0.0 && !matches!(intent, Intent::Magic | Intent::Turbo | Intent::Defend) {
            intent = if stick.magnitude > 0.0 {
                Intent::StrafeAttack(combat::Side::from_offset(wrap(c_stick.heading - stick.heading)))
            } else if magic_buttons.held & button::POWER != 0 {
                Intent::Power
            } else {
                Intent::Quick
            };
            if stick.magnitude == 0.0 {
                c_aim = Some(c_stick.heading);
            }
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
        let now = time.elapsed_secs_f64();
        if let Some(stun) = p.pending_stun.take().filter(|_| !cut) {
            p.stun_until = now + f64::from(stun);
        }
        let pojo = p.special_bits & power::POJO != 0;
        let (reaction, knock, reaction_face) = hit_reaction(hit_damage, hit_flags, hit_push, facing, pojo);
        let reaction = stun_class(reaction, current, now < p.stun_until && intent == Intent::Idle);
        // Thrown: FALLDOWN at once (the reaction class 301, played as
        // class 21's).
        let reaction = match p.thrown {
            Some((damage, Throw::Starts)) => {
                p.thrown = Some((damage, Throw::Flies));
                21
            }
            _ => reaction,
        };
        let stunned = matches!(reaction, 2 | 3 | 100);
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
                p.blow_cooldowns.retain(|(_, until)| *until > now);
                if p.blow_cooldowns.iter().all(|(e, _)| *e != f.entity) {
                    hits.write(shield_blow(entity, &f, LIGHTNING_DAMAGE, LIGHTNING_KIND, facing, grown));
                    p.blow_cooldowns.push((f.entity, now + LIGHTNING_COOLDOWN));
                    // The shield's spark, from the left wrist toward it.
                    let wrist = p.left_wrist.and_then(|n| animator.bone(n)).and_then(|b| bodies.get(b).ok());
                    let at = wrist.map_or(position + Vec3::Y * body.centre_height, |t| t.translation());
                    let toward = combat::heading_of(f.position - at);
                    effects.write(EffectAt { name: LIGHTNING_SPARK, bank: None, at, facing: toward, scale: 1.0 });
                }
            }
        }
        // The halo drains a Death the hero faces (under 90° off its
        // heading): the hero stands and grabs it, a point a tick
        // (`docs/monsters.md`, "Death").
        let halo = p.armour_bits & HALO != 0;
        if !halo {
            p.halo_drank = false;
        }
        let drinking = found
            .filter(|f| halo && wrap(combat::heading_of(f.direction) - facing).abs() < std::f32::consts::FRAC_PI_2)
            .and_then(|f| deaths.get(f.entity).ok().map(|d| (f, d.experience)));
        if let Some((f, experience)) = drinking {
            intent = Intent::Idle;
            hits.write(Hit {
                target: f.entity,
                attacker: entity,
                damage: HALO_DRAIN,
                kind: 0,
                push: Vec3::ZERO,
                at: f.position,
                target_kind: f.kind,
                ranged: false,
            });
            if !p.halo_drank {
                p.halo_drank = true;
                sounds.write(PlaySoundAt::panned(HALO_SOUND, position + Vec3::Y * crate::player_state::DEFAULT_HEAD, HALO_VOLUME));
            }
            // Every tick: S_DEATHDIE at the Death — the game starts it
            // again, panned, whenever it isn't playing (stand-in: one that
            // follows it, at its centre for its `+0x54` point) — and the
            // drain's sound at the hero's feet.
            let death = f.position + Vec3::Y * gdl_formats::enemy::enemy_stats(crate::monsters::DEATH_TYPE).map_or(0.0, |s| s.center_height);
            loops.write(LoopSoundAt::at(HALO_LOOP, DEATH_DIES, death, CALL_VOLUME));
            loops.write(LoopSoundAt::at(HALO_SUCK_LOOP, DEATH_SUCK, position, CALL_VOLUME));
            if !p.halo_draining {
                let name = if experience { DEATH_EXP } else { DEATH_ARC };
                riding.write(EffectOn { name, bank: Some(DEATH_BANK), on: entity, scale: 1.0 });
            }
        } else if p.halo_draining {
            loops.write(LoopSoundAt::stop(HALO_LOOP));
            loops.write(LoopSoundAt::stop(HALO_SUCK_LOOP));
        }
        p.halo_draining = drinking.is_some();
        // Charging (SHOVE) into a monster or a critter other than a boss:
        // 3, heavy, once a second each — in place of walking into it.
        let boss = |f: &combat::Found| targets.get(f.entity).is_ok_and(|(_, _, t)| t.boss);
        let charged = !shielded
            && current == Action::SHOVE
            && touching.is_some_and(|f| matches!(f.kind, TargetKind::Monster | TargetKind::Object) && !boss(&f));
        if charged && let Some(f) = touching {
            let now = time.elapsed_secs_f64();
            p.blow_cooldowns.retain(|(_, until)| *until > now);
            if p.blow_cooldowns.iter().all(|(e, _)| *e != f.entity) {
                hits.write(shield_blow(entity, &f, CHARGE_DAMAGE, combat::hit_kind::HEAVY, facing, grown));
                p.blow_cooldowns.push((f.entity, now + CHARGE_COOLDOWN));
            }
        }
        // Bosses are walked into only on the two levels whose boss is
        // type 0x25 or 0x29.
        let boss_walkable = monster_level.as_ref().is_some_and(|l| matches!(l.boss, 0x25 | 0x29));
        let mut walked_into = false;
        // The game's "walk-into attack" pad option (Auto Attack): walking
        // into a monster attacks it.
        if mine.auto_attack()
            && !shielded
            && !charged
            && reaction == 0
            && matches!(intent, Intent::Walk | Intent::Run)
            && p.actions.edges == 0
            && !(0x27..=0x72).contains(&current.0)
            && found.is_some_and(|f| {
                f.distance < combat::WALK_INTO + p.radius
                    && f.attacked_by_walking_into()
                    && (!boss(&f) || boss_walkable)
            })
        {
            intent = Intent::Quick;
            walked_into = true;
        }
        p.actions.range = combat::range(found.as_ref(), p.radius, intent.is_attack() && !walked_into);
        let strafing = held & (button::DEFEND | button::STRAFE) != 0
            || (0x09..=0x10).contains(&current.0)
            || (0x47..=0x4E).contains(&current.0);
        let aim = match found {
            // The C-stick attacks where it points.
            _ if c_aim.is_some() => c_aim.unwrap_or(wanted),
            _ if strafing || !mine.auto_aim() => wanted,
            Some(f) => combat::heading_of(f.direction),
            None => facing,
        };
        p.actions.target_angle = wrap(aim - facing);

        let mut requested =
            combat::request(intent, p.actions.range, stick.magnitude, walked_into, p.actions.combo, p.request);
        if p.halo_draining {
            requested = Action::DEATHGRABS;
        }
        // A power's own attack takes the place of every attack (not one
        // made by walking into something).
        if !walked_into
            && matches!(intent, Intent::Quick | Intent::Power | Intent::StrafeAttack(_))
            && let Some(a) = special_attack(p.special_bits, p.weapon)
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
        if let Some(a) = reaction_action(reaction).or_else(|| stun_action(reaction, current)) {
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
        let loops = clips.actions.get(animator.action).is_some_and(|a| a.loops());
        let wrapped = loops && p.last_clip.0 == animator.action && animator.frame < p.last_clip.1;
        let ended = animator.finished() || wrapped;
        p.came_round |= ended;
        let env = Env {
            frame: animator.frame,
            class: p.class,
            has_low2: clips.actions.iter().any(|a| a.name == Action::ATTLOW2.name()),
            magic_released: p.magic.flags & MagicState::RELEASED != 0,
            came_round: p.came_round,
        };
        let mut next = p.actions.next(requested, &env);
        // With a shield on its arm the hero stands and moves behind it.
        if p.armour_bits & SHIELDS != 0 {
            let action = shield_action(next.action);
            // The shield run starts over at its end (the chooser's loop
            // flag; its clip's own is off), taking over from itself only
            // then.
            if action == Action::SHIELD_RUN && next.action != action {
                next.again = true;
                if current == Action::SHIELD_RUN {
                    next.switch = actions::Switch::AtEnd;
                }
            }
            next.action = action;
        }
        let (move_factor, turn_factor) = actions::factors(current, p.class.unwrap_or(0));
        let clip = clip_for(&animator, next.action);
        let again = next.again && ended && clip == animator.action;
        if again || next.switch.applies(clip != animator.action, ended) {
            if clip != animator.action {
                animator.play_blended(&clips.actions[clip].name, next.blend);
            } else {
                animator.play(clip);
            }
            p.came_round = false;
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
                spent.write(SpendPower { slot: p.slot, subtype: power::SPECIAL, bits: BREATHS });
            }
            // The hammer comes down as its recovery starts, and a use is
            // spent.
            if strike.0 & Strike::CHOP != 0 {
                chops.write(ChopAt { hero: entity });
                spent.write(SpendPower { slot: p.slot, subtype: power::WEAPON, bits: HAMMER });
            }
            if strike.0 & (Strike::MAGIC | Strike::THROW_POTION) != 0 && !cut {
                let mode = if strike.0 & Strike::THROW_POTION != 0 {
                    if held & button::THROW_MAGIC != 0 { 3 } else { 2 }
                } else if p.magic.flags & MagicState::SHIELD != 0 {
                    1
                } else {
                    0
                };
                if mode == 1 {
                    let centre = position + Vec3::Y * crate::projectiles::PLAYER_CENTRE;
                    sounds.write(PlaySoundAt::panned(TURBO_DEFENSE, centre, CALL_VOLUME));
                }
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
                    _ => Vec3::new(wanted.dsin(), 0.0, wanted.dcos()),
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
        let drive = if stunned { 0.0 } else { drive };
        let mut face = (magnitude > 0.0 && !keeps_facing && !stunned).then_some(stick.heading);
        // The "attack aim" option (Auto Aim): attacking in place turns the
        // hero toward the target.
        if mine.auto_aim() && (1..=10).contains(&category) && category != 7 && !strafing && drive == 0.0 {
            face = Some(aim);
        }
        if reaction_face.is_some() {
            face = reaction_face;
        }
        if let Some(b) = face_boss {
            face = Some((b.x - position.x).datan2(b.z - position.z));
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
        // A thrown hero is down once it's within 0.2 of a floor.
        if let Some((damage, Throw::Flies)) = p.thrown
            && p.mover.position[1] - p.ground.floor <= THROWN_LANDS
        {
            p.thrown = Some((damage, Throw::Landed));
        }
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
    let at = position + Vec3::new(facing.dsin(), 0.0, facing.dcos()) * (combat::REACH + p.radius);
    Some(Hit { target: found.entity, attacker, damage, kind, push, at, target_kind: found.kind, ranged: false })
}

fn interpolate(fixed: Res<Time<Fixed>>, mut players: Query<(&Player, &mut Transform)>, nodes: Query<&GlobalTransform, Without<Player>>) {
    let t = fixed.overstep_fraction();
    for (player, mut transform) in &mut players {
        // Held, it hangs from the critter's node (turning with it).
        if let Some((node, offset, at)) = player.grabbed {
            let hang = node.and_then(|n| nodes.get(n).ok()).map(|g| g.affine());
            let (_, rotation, place) = match hang {
                Some(a) => (a * bevy::math::Affine3A::from_translation(offset)).to_scale_rotation_translation(),
                None => (Vec3::ONE, transform.rotation, at),
            };
            transform.translation = place;
            transform.rotation = rotation;
            continue;
        }
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
    fn stuns() {
        // Poison stuns for a second, Death's drain for a fifteenth.
        assert_eq!(blow_stun(combat::hit_kind::POISON), Some(1.0));
        assert_eq!(blow_stun(combat::hit_kind::DRAIN | combat::hit_kind::POISON), Some(1.0 / 15.0));
        assert_eq!(blow_stun(0x10), None);
        // Standing while stunned: STUN2, even over a new blow — not while
        // falling.
        assert_eq!(stun_class(0, Action::READY, true), 100);
        assert_eq!(stun_class(20, Action::READY, true), 100);
        assert_eq!(stun_class(0, Action(0x85), true), 0);
        assert_eq!(stun_action(100, Action::READY), Some(Action::STUN2));
        // A stunning blow: STUN1 (a damage tile's) or HITREACT (0x2000),
        // then standing while it plays.
        assert_eq!(stun_action(hit_reaction(5.0, 0x80, Vec3::Z, 0.0, false).0, Action::WALK1), Some(Action::STUN1));
        assert_eq!(stun_action(hit_reaction(5.0, 0x2000, Vec3::Z, 0.0, false).0, Action::READY), Some(Action::STUNREACT));
        assert_eq!(stun_class(0, Action::STUN1, false), 2);
        assert_eq!(stun_action(2, Action::STUN1), Some(Action::READY));
        assert_eq!(stun_class(0, Action::STUNREACT, true), 100);
        assert_eq!(stun_class(0, Action::WALK1, false), 0);
    }

    #[test]
    fn powers_take_over_the_attacks_in_the_games_order() {
        assert_eq!(special_attack(0, 0), None);
        assert_eq!(special_attack(0x10, 0), Some(Action::ATTBREATHE));
        // The hammer comes before a breath, Skorne's horns before both.
        assert_eq!(special_attack(0x10, HAMMER), Some(Action::ATTCHOP));
        assert_eq!(special_attack(0x1010, HAMMER), Some(Action::ATTBREATHE));
        assert_eq!(special_attack(0x8000, 0), Some(Action::ATTFIREL));
        assert_eq!(special_attack(0x4000, 0x10_0000), Some(Action::ATTFIRELR));
        assert_eq!(special_attack(0, 0x10_0000 | HAMMER), Some(Action::SSHOT1));
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
