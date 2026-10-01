//! Melee combat: the controls' logical buttons and what they ask the hero
//! to do, the game's target search and hit test, and the [`Hit`] messages
//! other systems apply to whatever they attached a [`Targetable`] to.
//! Numbers and rules are the game's own (`docs/combat.md`); the action
//! chaining itself is in `actions.rs`.
//!
//! Interface for other systems: put a [`Targetable`] on an entity whose
//! `GlobalTransform` translation is the target's reference point (the same
//! kind of point as the hero's: its feet), remove it when the target can no
//! longer be hit (dead, destroyed), and read [`Hit`] messages.

use gdl_formats::detmath::Det;
use std::f32::consts::FRAC_PI_4;

use bevy::prelude::*;

use crate::actions::{Action, Range, Strike};
use crate::locomotion::{RUN_THRESHOLD, wrap};

/// The game's logical buttons, as bits of the per-player control words.
/// The pad (or keyboard) is mapped onto them by a control scheme.
pub mod button {
    pub const MAGIC: u32 = 0x100;
    pub const QUICK: u32 = 0x200;
    pub const POWER: u32 = 0x400;
    pub const TURBO: u32 = 0x800;
    pub const DEFEND: u32 = 0x1000;
    pub const CHARGE: u32 = 0x2000;
    pub const STRAFE: u32 = 0x4000;
    pub const MAGIC_SHIELD: u32 = 0x8000;
    pub const THROW_MAGIC: u32 = 0x10000;
    pub const COMBO_MOVE: u32 = 0x20000;
    /// The D-pad (the power menu, `power_menu.rs`): the game's raw pad
    /// bits for it.
    pub const DPAD_LEFT: u32 = 0x1000_0000;
    pub const DPAD_RIGHT: u32 = 0x2000_0000;
    pub const DPAD_UP: u32 = 0x4000_0000;
    pub const DPAD_DOWN: u32 = 0x8000_0000;
}

/// One tick of the controls: buttons held, and those that went down this
/// tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub held: u32,
    pub pressed: u32,
}

impl Buttons {
    /// This tick's held buttons, given last tick's.
    pub fn from_held(held: u32, previous: u32) -> Self {
        Self { held, pressed: held & !previous }
    }
}

/// Which way a strafe goes relative to the hero's facing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
    Left,
    Right,
}

impl Side {
    /// `offset` is the stick heading minus the facing: within 45° is
    /// forward, beyond 135° back, positive angles right.
    pub fn from_offset(offset: f32) -> Self {
        let a = wrap(offset);
        if !(-3.0 * FRAC_PI_4..=3.0 * FRAC_PI_4).contains(&a) {
            Side::Back
        } else if a > FRAC_PI_4 {
            Side::Right
        } else if a < -FRAC_PI_4 {
            Side::Left
        } else {
            Side::Front
        }
    }
}

/// What the controls ask for this tick (the game's player intents).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Idle,
    Walk,
    Run,
    Defend,
    /// The charge (shove), with enough turbo.
    Charge,
    StrafeWalk(Side),
    Quick,
    Power,
    StrafeAttack(Side),
    /// Turbo held and attack pressed.
    Turbo,
    /// One of the three magic buttons.
    Magic,
}

/// Stick magnitude at which the hero stops walking and runs.
pub const RUN_MAGNITUDE: f32 = RUN_THRESHOLD;
/// Turbo needed for the charge (a co-op combo needs 50, the turbo attacks
/// 40 and 100).
pub const CHARGE_TURBO: f32 = 5.0;

/// The game's intent classifier for one player, with the default control
/// scheme (the C-stick unused). `offset` is the stick heading minus the
/// facing; `turbo` the hero's turbo meter.
pub fn classify(b: Buttons, magnitude: f32, offset: f32, turbo: f32) -> Intent {
    use button::*;
    let held = b.held;
    if held & (THROW_MAGIC | MAGIC_SHIELD | MAGIC) != 0 {
        return Intent::Magic;
    }
    // COMBO_MOVE with 50 turbo and a partner in reach asks for the co-op
    // combo; there's never a partner yet.
    if held & TURBO != 0 && b.pressed & QUICK != 0 && turbo >= 0.0 {
        return Intent::Turbo;
    }
    if b.pressed & DEFEND != 0 {
        return Intent::Defend;
    }
    if held & STRAFE != 0 && magnitude > 0.0 {
        let side = Side::from_offset(offset);
        return if held & (QUICK | POWER) == 0 { Intent::StrafeWalk(side) } else { Intent::StrafeAttack(side) };
    }
    if b.pressed & CHARGE != 0 && turbo >= CHARGE_TURBO {
        return Intent::Charge;
    }
    if held & QUICK != 0 {
        return Intent::Quick;
    }
    if held & POWER != 0 {
        return Intent::Power;
    }
    if magnitude > RUN_MAGNITUDE {
        Intent::Run
    } else if magnitude > 0.0 {
        Intent::Walk
    } else {
        Intent::Idle
    }
}

impl Intent {
    /// The strafe and defend intents keep the hero facing where it faces.
    pub fn keeps_facing(self) -> bool {
        matches!(self, Intent::Defend | Intent::StrafeWalk(_) | Intent::StrafeAttack(_))
    }

    pub fn is_attack(self) -> bool {
        matches!(self, Intent::Quick | Intent::Power)
    }
}

/// What the game distinguishes when it searches for and damages a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// A monster. Walking into one attacks it; one whose floor step is at
    /// most 2 units (the small ones) gets the low attacks.
    Monster,
    /// A generator. Walking into one attacks it; searched with its radius
    /// capped at 5 and within twice its height vertically.
    Generator,
    /// Containers and obstacles (barrels, chests, …): searched like
    /// generators but a little further away than they are; walking into one
    /// doesn't attack it.
    Breakable,
    /// Level objects with hit points of their own (the game's second object
    /// table). Walking into one attacks it.
    Object,
}

/// Something the hero can target and hit.
#[derive(Component, Clone, Debug)]
pub struct Targetable {
    pub kind: TargetKind,
    /// Distances are measured to this far from the reference point.
    pub radius: f32,
    /// Generators and breakables are found within twice this vertically.
    pub height: f32,
    /// What the low test measures: a monster's floor step (`+0x23C`, low
    /// at most 2), a generator's or breakable's height (at most 3.5).
    pub size: f32,
    /// A critter's body or hit sphere: its reference point is the centre
    /// of it, and the critter counts once (see [`CritterAim`]).
    pub critter: Option<CritterAim>,
    /// A boss critter's (type class 4): the hero neither walks into nor
    /// charges it.
    pub boss: bool,
    /// Per attacker, until when (seconds, `Time<Fixed>` elapsed) they can't
    /// hit this again. Melee blows don't use one; the charge would.
    #[allow(dead_code)]
    cooldowns: Vec<(Entity, f64)>,
}

/// The cooldown half is for blows that use one (the charge); no melee blow
/// does.
#[allow(dead_code)]
impl Targetable {
    pub fn new(kind: TargetKind, radius: f32, height: f32) -> Self {
        Self { kind, radius, height, size: height, critter: None, boss: false, cooldowns: Vec::new() }
    }

    /// A boss critter's.
    pub fn of_boss(mut self, boss: bool) -> Self {
        self.boss = boss;
        self
    }

    /// Part of a critter.
    pub fn of_critter(mut self, aim: CritterAim) -> Self {
        self.critter = Some(aim);
        self
    }

    /// The size the low test uses, when it isn't the height (monsters).
    pub fn with_size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Whether `attacker` may hit this at time `now`.
    pub fn can_be_hit_by(&self, attacker: Entity, now: f64) -> bool {
        self.cooldowns.iter().all(|&(who, until)| who != attacker || until <= now)
    }

    /// After a hit with a cooldown: `attacker` can't hit again for
    /// `seconds`. The game keeps four such slots per target.
    pub fn start_cooldown(&mut self, attacker: Entity, now: f64, seconds: f32) {
        if seconds <= 0.0 {
            return;
        }
        self.cooldowns.retain(|&(who, until)| who != attacker && until > now);
        if self.cooldowns.len() >= COOLDOWN_SLOTS {
            // Replace the one that runs out soonest.
            if let Some(i) = (0..self.cooldowns.len()).min_by(|&a, &b| self.cooldowns[a].1.total_cmp(&self.cooldowns[b].1)) {
                self.cooldowns.remove(i);
            }
        }
        self.cooldowns.push((attacker, now + seconds as f64));
    }
}

#[allow(dead_code)]
const COOLDOWN_SLOTS: usize = 4;

/// Which critter (a body, or one of a boss's parts) a target belongs to:
/// the game tests a critter's live hit spheres before its body, and finds
/// or hits the critter once — its body only when none of its spheres will
/// do (`docs/critters.md`, "Found and hit").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CritterAim {
    /// The critter's body target (for the body, the target itself).
    pub body: Entity,
    /// A hit sphere's place among its critter's, its reach and weight.
    pub sphere: Option<SphereAim>,
}

/// A hit sphere as the searches see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphereAim {
    /// Its place among its critter's spheres: the first that will do wins.
    pub index: usize,
    /// How far away the hero's search finds it (0: as far as it looks).
    pub reach: f32,
    /// How much the search favours it among its critter's.
    pub weight: f32,
}

/// Keeps one of each critter's targets among `hits` (those a test
/// accepted): its first sphere, else its body; everything else stays.
pub fn one_per_critter<T>(hits: Vec<T>, aim: impl Fn(&T) -> Option<CritterAim>) -> Vec<T> {
    let first_sphere = |body: Entity| {
        hits.iter()
            .filter_map(|h| aim(h).filter(|a| a.body == body).and_then(|a| a.sphere).map(|s| s.index))
            .min()
    };
    let wanted: Vec<bool> = hits
        .iter()
        .map(|h| match aim(h) {
            None => true,
            Some(a) => match (a.sphere, first_sphere(a.body)) {
                (Some(s), Some(first)) => s.index == first,
                (None, first) => first.is_none(),
                (Some(_), None) => true,
            },
        })
        .collect();
    hits.into_iter().zip(wanted).filter_map(|(h, w)| w.then_some(h)).collect()
}

/// Extra kinds of blow, as the game hands them to the target (a hero's
/// carry its weapon power-up bits too, the element in the low four).
pub mod hit_kind {
    /// The blow's element (1 fire, 2 lightning, 3 light, 4 acid;
    /// `damage::ELEMENTS`).
    pub const ELEMENT: u32 = 0xF;
    /// The slow attack or a lunge (double damage).
    pub const STRONG: u32 = 0x10;
    /// A combo finisher (triple damage), or the kick on a low monster.
    pub const HEAVY: u32 = 0x20;
    /// The kinds that knock the target back or down.
    pub const KNOCKS: u32 = 0x1_0170;
    /// Magic and poison: armour doesn't stop them.
    pub const MAGIC: u32 = 0x200;
    pub const POISON: u32 = 0x800;
    /// Death's drain (a negative blow, no element).
    pub const DRAIN: u32 = 0x1000;
    /// No hit look (blood spray) where it lands.
    pub const NO_HIT_LOOK: u32 = 0x100_0000;
    /// A small monster's blow (or any while the enemies are shrunk):
    /// levitation dodges it.
    pub const SMALL_MONSTER: u32 = 0x4000_0000;
}

/// A blow landing on a target. Whoever owns the target applies it.
#[derive(Message, Clone, Debug)]
pub struct Hit {
    pub target: Entity,
    pub attacker: Entity,
    /// Hit points to take off (the attacker's strength × the blow's
    /// multiplier); the target's own resistances aren't applied.
    pub damage: f32,
    /// [`hit_kind`] bits.
    pub kind: u32,
    /// The push the game adds to a monster's push accumulator: the
    /// attacker's facing in X/Z and 0.05 × damage (at most 2) in Y. Zero for
    /// generators and breakables, which get none.
    pub push: Vec3,
    /// Where the blow lands: the attacker's position plus its facing ×
    /// (2 + its radius), for effects.
    pub at: Vec3,
    pub target_kind: TargetKind,
    /// A thrown weapon or missile rather than a hand blow (the game's
    /// "far" hits: they pick the far sound versions).
    pub ranged: bool,
}

/// How far the search looks, units.
pub const SEARCH_RANGE: f32 = 30.0;
/// How far a throw looks on a boss level.
pub const BOSS_THROW_RANGE: f32 = 200.0;
/// Cosine of the widest search angle (next to the hero): 60° either side.
/// It narrows linearly to straight ahead at [`SEARCH_RANGE`].
pub const SEARCH_CONE: f32 = 0.5;
/// Monsters are found within this many units vertically.
pub const MONSTER_VERTICAL: f32 = 10.0;
/// Generator and breakable radii are capped at this in the search.
pub const GENERATOR_RADIUS_CAP: f32 = 5.0;
/// Breakables count this much further away than they are.
pub const BREAKABLE_DISTANCE_SCALE: f32 = 1.2;
/// Past its own radius, how far the hero reaches with a blow.
pub const REACH: f32 = 2.0;
/// Walking to within this (past the hero's radius) of a monster, generator
/// or object attacks it.
pub const WALK_INTO: f32 = 1.0;
/// Range bands past the hero's radius (one more while attacking).
pub const CLOSE: f32 = 1.0;
pub const MEDIUM: f32 = 2.0;
/// Low targets: monster floor step, generator/breakable height.
pub const LOW_MONSTER: f32 = 2.0;
pub const LOW_GENERATOR: f32 = 3.5;
/// Push per point of damage, and its cap.
pub const PUSH_PER_DAMAGE: f32 = 0.05;
pub const PUSH_MAX: f32 = 2.0;
/// Damage multipliers of the strong blows and the finishers.
pub const STRONG_MULTIPLIER: f32 = 2.0;
pub const FINISHER_MULTIPLIER: f32 = 3.0;
/// Radius of the line-of-sight test between a hero and what it hits.
pub const SIGHT_RADIUS: f32 = 0.1;

/// Derived strength (damage per blow) from the strength stat: 5 at 0, 20 at
/// 1000.
pub fn strength(stat: f32) -> f32 {
    (5.0 + 0.001 * stat * 15.0).clamp(5.0, 20.0)
}

/// The nearest target the search found.
#[derive(Clone, Copy, Debug)]
pub struct Found {
    pub entity: Entity,
    pub kind: TargetKind,
    /// Distance to the target's surface.
    pub distance: f32,
    /// Unit vector from the searcher to the target.
    pub direction: Vec3,
    pub position: Vec3,
    pub height: f32,
    /// The target's size for the low test ([`Targetable::size`]).
    pub size: f32,
}

impl Found {
    /// A short target within reach: the low attacks and kick.
    pub fn is_low(&self, hero_radius: f32) -> bool {
        let limit = match self.kind {
            TargetKind::Monster => LOW_MONSTER,
            TargetKind::Generator | TargetKind::Breakable => LOW_GENERATOR,
            TargetKind::Object => return false,
        };
        self.size <= limit && self.distance < REACH + hero_radius
    }

    /// Walking into it starts an attack.
    pub fn attacked_by_walking_into(&self) -> bool {
        self.kind != TargetKind::Breakable
    }
}

/// The game's target search: from `origin`, the nearest target (by distance
/// to its surface) within [`SEARCH_RANGE`] inside a cone around `heading`
/// that narrows with distance.
pub fn search<'a>(
    origin: Vec3,
    heading: f32,
    candidates: impl IntoIterator<Item = (Entity, Vec3, &'a Targetable)>,
) -> Option<Found> {
    search_within(origin, heading, SEARCH_RANGE, candidates)
}

/// [`search`] out to `range` (a throw on a boss level looks 200 units
/// out, `r2-0x5a58`).
pub fn search_within<'a>(
    origin: Vec3,
    heading: f32,
    range: f32,
    candidates: impl IntoIterator<Item = (Entity, Vec3, &'a Targetable)>,
) -> Option<Found> {
    let dir = Vec3::new(heading.dsin(), 0.0, heading.dcos());
    let narrowing = (1.0 - SEARCH_CONE) / range;
    // Inside the cone: the margin by which it is (the game's test).
    let margin = |n: Vec3, distance: f32| -> Option<f32> {
        let horizontal = Vec2::new(n.x, n.z).length();
        let m = n.x * dir.x + n.z * dir.z - horizontal * (distance * narrowing + SEARCH_CONE);
        (m > 0.0).then_some(m)
    };
    let mut best: Option<Found> = None;
    // Critters: the best-placed sphere of each (its surface distance over
    // its weight × its margin inside the cone, least wins), and their
    // bodies, found only when none of their spheres is.
    let mut spheres: Vec<(Entity, f32, Found)> = Vec::new();
    let mut bodies: Vec<Found> = Vec::new();
    for (entity, position, t) in candidates {
        let v = position - origin;
        let length = v.length();
        if let Some(aim) = t.critter {
            if length > range {
                continue;
            }
            let n = if length > 0.0 { v / length } else { dir };
            let distance = length - t.radius;
            let found = Found { entity, kind: t.kind, distance, direction: n, position, height: t.height, size: t.size };
            match aim.sphere {
                Some(s) => {
                    if s.reach > 0.0 && length > s.reach {
                        continue;
                    }
                    let Some(m) = margin(n, distance) else { continue };
                    let score = distance / (s.weight * m);
                    match spheres.iter_mut().find(|(body, ..)| *body == aim.body) {
                        Some(slot) if score < slot.1 => *slot = (aim.body, score, found),
                        Some(_) => {}
                        None if score < f32::MAX => spheres.push((aim.body, score, found)),
                        None => {}
                    }
                }
                None => {
                    if margin(n, distance).is_some() {
                        bodies.push(found);
                    }
                }
            }
            continue;
        }
        let distance = match t.kind {
            TargetKind::Monster => {
                if v.y.abs() > MONSTER_VERTICAL {
                    continue;
                }
                length - t.radius
            }
            TargetKind::Object => {
                if length > range {
                    continue;
                }
                length - t.radius
            }
            TargetKind::Generator | TargetKind::Breakable => {
                if v.y.abs() > 2.0 * t.height {
                    continue;
                }
                let scale = if t.kind == TargetKind::Breakable { BREAKABLE_DISTANCE_SCALE } else { 1.0 };
                length * scale - t.radius.min(GENERATOR_RADIUS_CAP)
            }
        };
        if distance > range || best.is_some_and(|b| distance >= b.distance) {
            continue;
        }
        let n = if length > 0.0 { v / length } else { dir };
        if margin(n, distance).is_none() {
            continue;
        }
        best = Some(Found { entity, kind: t.kind, distance, direction: n, position, height: t.height, size: t.size });
    }
    let critters = spheres
        .iter()
        .map(|(_, _, f)| *f)
        .chain(bodies.into_iter().filter(|b| !spheres.iter().any(|(body, ..)| *body == b.entity)));
    for f in critters {
        if f.distance <= range && best.is_none_or(|b| f.distance < b.distance) {
            best = Some(f);
        }
    }
    best
}

/// Range flags for the chaining from what the search found; `attacking`:
/// the controls ask for an attack (the bands reach one unit further).
pub fn range(found: Option<&Found>, hero_radius: f32, attacking: bool) -> Range {
    let extra = if attacking { 1.0 } else { 0.0 };
    let mut bits = 0;
    let distance = found.map_or(SEARCH_RANGE, |f| f.distance);
    if let Some(f) = found {
        bits |= match f.kind {
            TargetKind::Monster | TargetKind::Object => Range::MONSTER_OR_OBJECT,
            TargetKind::Generator | TargetKind::Breakable => Range::GENERATOR,
        };
        if f.is_low(hero_radius) {
            bits |= Range::LOW;
        }
    }
    bits |= if distance < CLOSE + hero_radius + extra {
        Range::CLOSE
    } else if distance < MEDIUM + hero_radius + extra {
        Range::MEDIUM
    } else {
        Range::FAR
    };
    Range(bits)
}

/// What the hero asks the chaining for. `stick`: the stick magnitude;
/// `walked_into`: the attack comes from walking into a target; `combo`: the
/// combo count; `previous`: last tick's request (kept when turbo can't
/// pay for a turbo attack).
pub fn request(intent: Intent, range: Range, stick: f32, walked_into: bool, combo: u32, previous: Action) -> Action {
    let low = range.has(Range::LOW);
    let lunge = range.has(Range::MEDIUM) && !low && stick != 0.0;
    let in_reach = range.has(Range::CLOSE) || walked_into;
    let gait = |i: Intent| match i {
        Intent::Run => Action::RUN1,
        Intent::Walk => Action::WALK1,
        _ => Action::READY,
    };
    match intent {
        Intent::Idle | Intent::Walk | Intent::Run => gait(intent),
        Intent::Defend => Action::DEFEND1,
        Intent::StrafeWalk(side) => match side {
            Side::Front => Action::STRAFE_WLKF1,
            Side::Back => Action::STRAFE_WLKB1,
            Side::Left => Action::STRAFE_WLKL1,
            Side::Right => Action::STRAFE_WLKR1,
        },
        Intent::StrafeAttack(side) => match side {
            Side::Front => Action::STRAFE_ATKF1,
            Side::Back => Action::STRAFE_ATKB1,
            Side::Left => Action::STRAFE_ATKL1,
            Side::Right => Action::STRAFE_ATKR1,
        },
        Intent::Quick => {
            if lunge {
                Action::ATTSTEP1
            } else if in_reach {
                if low { Action::ATTLOWK } else { Action::ATTQUICK1 }
            } else {
                Action::THROW1S
            }
        }
        Intent::Power => {
            if combo != 0 && stick != 0.0 && range.has(Range::CLOSE | Range::MEDIUM) {
                Action::ATTSTART
            } else if lunge {
                Action::ATTSTEP1
            } else if in_reach {
                if low { Action::ATTLOW1 } else { Action::ATTSTART }
            } else {
                Action::ATTPWRATHROW
            }
        }
        // Turbo attacks need 40 turbo; below that the request stands.
        Intent::Turbo => previous,
        Intent::Charge => Action::SHOVE,
        // Without potions the magic buttons do nothing but let the hero
        // walk on.
        Intent::Magic => Action::READY,
    }
}

/// Damage, kind and push of a melee blow from its strike flags.
pub fn blow(strike: Strike, strength: f32, target: &Found) -> (f32, u32) {
    let mut damage = strength;
    let mut kind = 0;
    if strike.0 & 0xF0 != 0 {
        kind |= hit_kind::HEAVY;
        damage *= FINISHER_MULTIPLIER;
    } else if strike.0 & Strike::STRONG != 0 {
        kind |= hit_kind::STRONG;
        damage *= STRONG_MULTIPLIER;
    } else if strike.0 & Strike::KICK != 0 && target.kind == TargetKind::Monster && target.size <= LOW_MONSTER {
        kind |= hit_kind::HEAVY;
    }
    (damage, kind)
}

/// The push the game hands a monster with a blow of `damage` from an
/// attacker facing `facing`.
pub fn push(facing: f32, damage: f32) -> Vec3 {
    Vec3::new(facing.dsin(), (PUSH_PER_DAMAGE * damage).min(PUSH_MAX), facing.dcos())
}

/// Heading of a direction (0 = +Z, π/2 = +X).
pub fn heading_of(v: Vec3) -> f32 {
    v.x.datan2(v.z)
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Hit>();
        if let Some(dummy) = DummySpec::from_env() {
            app.insert_resource(dummy)
                .add_systems(Update, (spawn_dummy.after(crate::player::PlayerSpawn), dummy_hits));
        }
    }
}

/// `GDL_DUMMY=<distance>[,<kind>[,<radius>[,<height>]]]`: a practice target
/// that far in front of the hero at each level start (kind `monster`,
/// `generator`, `breakable` or `object`; default a 1-unit-radius, 4-unit
/// monster). It logs the hits it takes and flashes red.
#[derive(Resource, Clone, Copy)]
struct DummySpec {
    distance: f32,
    kind: TargetKind,
    radius: f32,
    height: f32,
}

impl DummySpec {
    fn from_env() -> Option<Self> {
        let v = std::env::var("GDL_DUMMY").ok()?;
        let mut parts = v.split(',').map(str::trim);
        let distance = parts.next()?.parse().ok()?;
        let kind = match parts.next().unwrap_or("monster") {
            "generator" => TargetKind::Generator,
            "breakable" => TargetKind::Breakable,
            "object" => TargetKind::Object,
            _ => TargetKind::Monster,
        };
        let radius = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let height = parts.next().and_then(|s| s.parse().ok()).unwrap_or(4.0);
        Some(Self { distance, kind, radius, height })
    }
}

#[derive(Component)]
struct Dummy {
    hits: u32,
    damage: f32,
    flash: f32,
    material: Handle<StandardMaterial>,
}

const DUMMY_COLOUR: Color = Color::srgb(0.9, 0.8, 0.2);
const DUMMY_HIT_COLOUR: Color = Color::srgb(1.0, 0.1, 0.1);

fn spawn_dummy(
    mut commands: Commands,
    spec: Res<DummySpec>,
    players: Query<&crate::player::Player, Added<crate::player::Player>>,
    ground: Option<Res<crate::world::LevelGround>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for player in &players {
        let (feet, facing) = (Vec3::from(player.start.0), player.start.1);
        let mut at = feet + Vec3::new(facing.dsin(), 0.0, facing.dcos()) * spec.distance;
        if let Some(y) = ground.as_ref().and_then(|g| g.0.floor_height((at + Vec3::Y * 2.0).to_array())) {
            at.y = y;
        }
        let material = materials.add(StandardMaterial { base_color: DUMMY_COLOUR, unlit: true, ..default() });
        // The reference point is the feet; the (centred) cylinder sits on it.
        commands.spawn((
            Targetable::new(spec.kind, spec.radius, spec.height),
            Dummy { hits: 0, damage: 0.0, flash: 0.0, material: material.clone() },
            Transform::from_translation(at),
            Visibility::default(),
            crate::world::LevelEntity,
            children![(
                Mesh3d(meshes.add(Cylinder::new(spec.radius, spec.height))),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, spec.height / 2.0, 0.0),
            )],
        ));
        info!("practice target ({:?}) at {at:?}, {:.1} units ahead", spec.kind, spec.distance);
    }
}

fn dummy_hits(
    time: Res<Time>,
    mut hits: MessageReader<Hit>,
    mut dummies: Query<(Entity, &mut Dummy)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for hit in hits.read() {
        if let Ok((_, mut d)) = dummies.get_mut(hit.target) {
            d.hits += 1;
            d.damage += hit.damage;
            d.flash = 0.2;
            info!(
                "practice target hit #{} by {:?} at {:.1?}: {:.1} damage (kind {:#x}, push {:.2?}), {:.1} total",
                d.hits, hit.attacker, hit.at, hit.damage, hit.kind, hit.push, d.damage
            );
        }
    }
    for (_, mut d) in &mut dummies {
        let was = d.flash > 0.0;
        d.flash -= time.delta_secs();
        if let Some(m) = materials.get_mut(&d.material) {
            if d.flash > 0.0 {
                m.base_color = DUMMY_HIT_COLOUR;
            } else if was {
                m.base_color = DUMMY_COLOUR;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;

    fn e(n: u32) -> Entity {
        Entity::from_raw_u32(n).unwrap()
    }

    #[test]
    fn a_critter_counts_once() {
        let (a, b) = (e(1), e(2));
        let sphere = |body, index| Some(CritterAim { body, sphere: Some(SphereAim { index, reach: 0.0, weight: 1.0 }) });
        let hits = vec![
            ("a sphere 2", sphere(a, 2)),
            ("a body", Some(CritterAim { body: a, sphere: None })),
            ("monster", None),
            ("a sphere 0", sphere(a, 0)),
            ("b body", Some(CritterAim { body: b, sphere: None })),
        ];
        let kept: Vec<&str> = one_per_critter(hits, |h| h.1).into_iter().map(|h| h.0).collect();
        assert_eq!(kept, ["monster", "a sphere 0", "b body"]);
    }

    #[test]
    fn critters_are_found_by_their_best_placed_sphere() {
        let body = e(10);
        let aim = |index, reach, weight| {
            Targetable::new(TargetKind::Object, 1.0, 1.0)
                .of_critter(CritterAim { body, sphere: Some(SphereAim { index, reach, weight }) })
        };
        let whole = Targetable::new(TargetKind::Object, 3.0, 4.0).of_critter(CritterAim { body, sphere: None });
        // Straight ahead is +Z. The nearer sphere is off to the side; the
        // heavier, straighter one wins, and the search reports its surface
        // distance.
        let near = aim(0, 0.0, 1.0);
        let straight = aim(1, 0.0, 5.0);
        let far = aim(2, 5.0, 100.0);
        let candidates = [
            (e(11), Vec3::new(3.0, 0.0, 6.0), &near),
            (e(12), Vec3::new(0.0, 0.0, 10.0), &straight),
            // Beyond its reach of 5.
            (e(13), Vec3::new(0.0, 0.0, 8.0), &far),
            (body, Vec3::new(0.0, 0.0, 9.0), &whole),
        ];
        let found = search(Vec3::ZERO, 0.0, candidates).unwrap();
        assert_eq!(found.entity, e(12));
        assert!((found.distance - 9.0).abs() < 1e-5);
        // No sphere will do: the body is found.
        let candidates = [(e(13), Vec3::new(0.0, 0.0, 8.0), &far), (body, Vec3::new(0.0, 0.0, 9.0), &whole)];
        assert_eq!(search(Vec3::ZERO, 0.0, candidates).unwrap().entity, body);
    }

    #[test]
    fn strength_spans_five_to_twenty() {
        assert_eq!(strength(0.0), 5.0);
        assert!((strength(500.0) - 12.5).abs() < 1e-5);
        assert_eq!(strength(999.0 * 2.0), 20.0);
    }

    #[test]
    fn strafe_sides() {
        assert_eq!(Side::from_offset(0.2), Side::Front);
        assert_eq!(Side::from_offset(1.2), Side::Right);
        assert_eq!(Side::from_offset(-1.2), Side::Left);
        assert_eq!(Side::from_offset(3.0), Side::Back);
        assert_eq!(Side::from_offset(-2.5), Side::Back);
    }

    #[test]
    fn classifier_priorities() {
        use button::*;
        let press = |b: u32| Buttons { held: b, pressed: b };
        let hold = |b: u32| Buttons { held: b, pressed: 0 };
        assert_eq!(classify(hold(QUICK), 1.0, 0.0, 0.0), Intent::Quick);
        assert_eq!(classify(hold(POWER), 0.0, 0.0, 0.0), Intent::Power);
        assert_eq!(classify(hold(QUICK | POWER), 0.0, 0.0, 0.0), Intent::Quick);
        // Turbo and defend share a button: a press defends, holding it while
        // pressing attack is a turbo attack.
        assert_eq!(classify(press(TURBO | DEFEND), 0.0, 0.0, 0.0), Intent::Defend);
        assert_eq!(
            classify(Buttons { held: TURBO | DEFEND | QUICK, pressed: QUICK }, 0.0, 0.0, 0.0),
            Intent::Turbo
        );
        assert_eq!(classify(hold(STRAFE), 1.0, 1.5, 0.0), Intent::StrafeWalk(Side::Right));
        assert_eq!(classify(hold(STRAFE | QUICK), 1.0, 3.0, 0.0), Intent::StrafeAttack(Side::Back));
        assert_eq!(classify(hold(STRAFE), 0.0, 1.5, 0.0), Intent::Idle);
        assert_eq!(classify(press(CHARGE), 0.0, 0.0, 0.0), Intent::Idle);
        assert_eq!(classify(press(CHARGE), 0.0, 0.0, 10.0), Intent::Charge);
        assert_eq!(classify(hold(MAGIC), 1.0, 0.0, 0.0), Intent::Magic);
        assert_eq!(classify(Buttons::default(), 0.5, 0.0, 0.0), Intent::Walk);
        assert_eq!(classify(Buttons::default(), 0.9, 0.0, 0.0), Intent::Run);
    }

    #[test]
    fn search_picks_the_nearest_surface_inside_the_narrowing_cone() {
        let near = Targetable::new(TargetKind::Monster, 1.0, 4.0);
        let far = Targetable::new(TargetKind::Monster, 1.0, 4.0);
        let origin = Vec3::ZERO;
        let found = search(
            origin,
            0.0,
            [(e(1), Vec3::new(0.0, 0.0, 10.0), &far), (e(2), Vec3::new(0.0, 0.0, 4.0), &near)],
        )
        .unwrap();
        assert_eq!(found.entity, e(2));
        assert!((found.distance - 3.0).abs() < 1e-5);
        // 50° off is inside the cone up close, outside it far away.
        let off = |d: f32| Vec3::new(d * 50f32.to_radians().dsin(), 0.0, d * 50f32.to_radians().dcos());
        assert!(search(origin, 0.0, [(e(3), off(3.0), &near)]).is_some());
        assert!(search(origin, 0.0, [(e(3), off(20.0), &near)]).is_none());
        // Behind: never.
        assert!(search(origin, 0.0, [(e(4), Vec3::new(0.0, 0.0, -3.0), &near)]).is_none());
        // Too far above for a monster.
        assert!(search(origin, 0.0, [(e(5), Vec3::new(0.0, 11.0, 3.0), &near)]).is_none());
        // Beyond 30 units.
        assert!(search(origin, 0.0, [(e(6), Vec3::new(0.0, 0.0, 32.0), &near)]).is_none());
    }

    #[test]
    fn generators_cap_their_radius_and_breakables_count_further() {
        let big = Targetable::new(TargetKind::Generator, 8.0, 4.0);
        let f = search(Vec3::ZERO, 0.0, [(e(1), Vec3::new(0.0, 0.0, 10.0), &big)]).unwrap();
        assert!((f.distance - 5.0).abs() < 1e-5);
        let barrel = Targetable::new(TargetKind::Breakable, 1.0, 2.0);
        let f = search(Vec3::ZERO, 0.0, [(e(2), Vec3::new(0.0, 0.0, 10.0), &barrel)]).unwrap();
        assert!((f.distance - 11.0).abs() < 1e-5);
        assert!(search(Vec3::ZERO, 0.0, [(e(3), Vec3::new(0.0, 5.0, 3.0), &barrel)]).is_none());
    }

    #[test]
    fn range_bands() {
        let t = Targetable::new(TargetKind::Monster, 1.0, 4.0);
        let at = |d: f32| search(Vec3::ZERO, 0.0, [(e(1), Vec3::new(0.0, 0.0, d + 1.0), &t)]);
        let r = 1.5;
        assert!(range(at(2.0).as_ref(), r, false).has(Range::CLOSE));
        assert!(range(at(3.0).as_ref(), r, false).has(Range::MEDIUM));
        assert!(range(at(3.0).as_ref(), r, true).has(Range::CLOSE));
        assert!(range(at(5.0).as_ref(), r, false).has(Range::FAR));
        assert!(range(None, r, true).has(Range::FAR));
        let short = Targetable::new(TargetKind::Monster, 1.0, 1.5);
        let f = search(Vec3::ZERO, 0.0, [(e(2), Vec3::new(0.0, 0.0, 3.0), &short)]);
        assert!(range(f.as_ref(), r, true).has(Range::LOW));
    }

    #[test]
    fn requests() {
        let close = Range(Range::CLOSE);
        let low = Range(Range::CLOSE | Range::LOW);
        let medium = Range(Range::MEDIUM);
        let far = Range(Range::FAR);
        let r = |i, range, stick| request(i, range, stick, false, 0, Action::READY);
        assert_eq!(r(Intent::Quick, close, 0.0), Action::ATTQUICK1);
        assert_eq!(r(Intent::Quick, low, 0.0), Action::ATTLOWK);
        assert_eq!(r(Intent::Quick, medium, 1.0), Action::ATTSTEP1);
        assert_eq!(r(Intent::Quick, medium, 0.0), Action::THROW1S);
        assert_eq!(r(Intent::Quick, far, 1.0), Action::THROW1S);
        assert_eq!(request(Intent::Quick, far, 1.0, true, 0, Action::READY), Action::ATTQUICK1);
        assert_eq!(r(Intent::Power, close, 0.0), Action::ATTSTART);
        assert_eq!(r(Intent::Power, low, 0.0), Action::ATTLOW1);
        assert_eq!(r(Intent::Power, far, 0.0), Action::ATTPWRATHROW);
        assert_eq!(request(Intent::Power, medium, 1.0, false, 2, Action::READY), Action::ATTSTART);
        assert_eq!(request(Intent::Turbo, far, 0.0, false, 0, Action::WALK1), Action::WALK1);
        assert_eq!(r(Intent::StrafeWalk(Side::Left), far, 1.0), Action::STRAFE_WLKL1);
    }

    #[test]
    fn blows() {
        let t = Targetable::new(TargetKind::Monster, 1.0, 1.5);
        let f = search(Vec3::ZERO, 0.0, [(e(1), Vec3::new(0.0, 0.0, 2.0), &t)]).unwrap();
        assert_eq!(blow(Strike(Strike::NORMAL), 10.0, &f), (10.0, 0));
        assert_eq!(blow(Strike(Strike::STRONG), 10.0, &f), (20.0, hit_kind::STRONG));
        assert_eq!(blow(Strike(Strike::FINISHER), 10.0, &f), (30.0, hit_kind::HEAVY));
        assert_eq!(blow(Strike(Strike::KICK), 10.0, &f), (10.0, hit_kind::HEAVY));
        assert_eq!(push(0.0, 60.0), Vec3::new(0.0, 2.0, 1.0));
        let p = push(PI / 2.0, 10.0);
        assert!((p - Vec3::new(1.0, 0.5, 0.0)).length() < 1e-5);
        assert!((heading_of(Vec3::X) - PI / 2.0).abs() < 1e-6);
    }

    #[test]
    fn cooldowns_block_one_attacker_for_a_while() {
        let mut t = Targetable::new(TargetKind::Monster, 1.0, 4.0);
        t.start_cooldown(e(1), 10.0, 1.0);
        assert!(!t.can_be_hit_by(e(1), 10.5));
        assert!(t.can_be_hit_by(e(2), 10.5));
        assert!(t.can_be_hit_by(e(1), 11.0));
        t.start_cooldown(e(1), 10.0, 0.0);
        for n in 2..8 {
            t.start_cooldown(e(n), 10.0, n as f32);
        }
        assert_eq!(t.cooldowns.len(), COOLDOWN_SLOTS);
    }
}
