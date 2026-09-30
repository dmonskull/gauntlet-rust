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
use crate::player_state::{PlayerState, power};
use crate::population::LevelPopulation;
use crate::effects::{MAGIC_BUTTONS, MagicIntent, MagicState, UsePotion};
use crate::flash::{self, Flash, FlashColours, Retexture};
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
    /// Blows taken since the last tick: damage, kind flags, summed push
    /// directions (the game's `+0x8D0`, `+0x8D4`, `+0x8DC`).
    pending_hit: (f32, u32, Vec3),
    /// Its hit flash (`flash.rs`), and the invulnerability's chrome in
    /// the same slot ([`show_chrome`]).
    flash: Flash,
    chrome: Flash,
    chrome_look: Retexture,
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
        p.boss_level = boss_level;
        let speed = (p.base_speed + b.speed).clamp(locomotion::SPEED_MIN, locomotion::SPEED_MAX);
        if p.mover.speed != speed {
            p.mover.speed = speed;
        }
        if b.turbo > 0.0 {
            p.turbo = (p.turbo + b.turbo).min(TURBO_MAX);
        }
    }
}

/// The chrome blinks in its last this many seconds, this many times a
/// second (on in the odd eighths).
const CHROME_BLINKS_FROM: f32 = 3.0;
const CHROME_BLINK_RATE: f32 = 8.0;

/// The invulnerability power-ups' chrome (`docs/powers.md`): the stats
/// routine re-arms the hero's timed texture effect with `CHROMEGOLD` (with
/// the gold armour) or `CHROMESILVER` every tick the longest
/// invulnerability's time is unlimited, over 3 s or in an odd eighth of a
/// second — so it blinks in its last three seconds — and, like a flash,
/// it shows for the tick it's armed and the next. It takes the slot from
/// a hit flash.
#[allow(clippy::type_complexity)]
fn show_chrome(
    state: Option<Res<PlayerState>>,
    colours: Res<FlashColours>,
    mut players: Query<(&mut Player, &Animator)>,
    mut drawn: Query<&mut MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    (mut tags, mut commands): (Query<&mut MeshTag>, Commands),
) {
    let Some(state) = state else { return };
    let invulnerable = crate::damage::resists::INVULNERABLE;
    // The longest one's time; negative for one that doesn't run out.
    let time = state
        .powers
        .iter()
        .filter(|p| p.subtype == power::ARMOUR && p.value & invulnerable != 0)
        .map(|p| p.time)
        .reduce(|a, b| if a < 0.0 || b < 0.0 { -1.0 } else { a.max(b) });
    let armed = time.is_some_and(|t| t < 0.0 || t > CHROME_BLINKS_FROM || (t * CHROME_BLINK_RATE) as i32 % 2 == 1);
    let gold = state.bits.armour & crate::damage::resists::GOLD != 0;
    for (mut p, animator) in &mut players {
        let p = &mut *p;
        if armed {
            p.chrome.start();
        }
        if p.chrome.step() {
            debug!("chrome {} (time {time:?})", if p.chrome.on() { "on" } else { "off" });
        }
        let texture = if p.chrome.on() { colours.chrome[usize::from(gold)].as_ref() } else { None };
        if texture.is_some() && p.flash.stop() {
            flash::tag_body(animator, |_| true, 0, &mut tags, &mut commands);
        }
        p.chrome_look.show(texture, animator, &mut drawn, &mut materials);
    }
}

/// Knockback speeds the hero's reactions add along the blow's push.
const STRONG_KNOCKBACK: f32 = 16.0;
const KNOCKDOWN_KNOCKBACK: f32 = 32.0;
const FLING_KNOCKBACK: f32 = 100.0;
/// Blows of more than this flash the hero (and draw a reaction).
const HIT_FLASH_DAMAGE: f32 = 1.0;

/// The game's reaction class for the blows a hero took this tick (see
/// `docs/combat.md` "Blows that land on the hero"): 0 none, 1 a flinch,
/// 2 a stun (kind 0x80), 3 kind 0x2000, 10 a strong knockback, 20 a
/// knockdown, 30 fling; knockback classes gain 1 when the push comes from
/// behind the facing. Returns the class, the knockback speed, and the
/// heading to face (for the knockback classes).
fn hit_reaction(damage: f32, flags: u32, push: Vec3, facing: f32) -> (u32, f32, Option<f32>) {
    if damage <= 1.0 {
        return (if flags & 0x80 != 0 { 2 } else { 0 }, 0.0, None);
    }
    let (mut class, speed) = if flags & 0x10000 != 0 {
        (30, FLING_KNOCKBACK)
    } else if flags & 0x40 != 0 {
        (20, FLING_KNOCKBACK)
    } else if flags & 0x120 != 0 {
        (20, KNOCKDOWN_KNOCKBACK)
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

/// Blows of this or less don't knock the hero about.
const KNOCKLESS_DAMAGE: f32 = 2.0;

impl Player {
    /// A blow lands on the hero (the game's hurt-player routine,
    /// `docs/combat.md`): through its armour and armour powers
    /// (`damage::resist`), then queued for its next tick's reaction — a
    /// blow of 2 or less without its knockback, one armour stopped still
    /// with its stun. Returns the health it takes, for a [`DamagePlayer`]:
    /// negative for the gold armour's heal, which draws no reaction.
    ///
    /// [`DamagePlayer`]: crate::player_state::DamagePlayer
    pub fn take_blow(&mut self, damage: f32, kind: u32, push: Vec3) -> f32 {
        let mut kind = kind;
        let d = crate::damage::resist(damage, &mut kind, self.armor, self.armour_bits, self.boss_level);
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
            .add_systems(FixedUpdate, (apply_powers, show_chrome).after(crate::player_state::PowersTick))
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
        pending_hit: (0.0, 0, Vec3::ZERO),
        flash: Flash::default(),
        chrome: Flash::default(),
        chrome_look: Retexture::default(),
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
    (mut shots, mut potions): (MessageWriter<HeroShot>, MessageWriter<UsePotion>),
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

        let mut requested =
            combat::request(intent, p.actions.range, stick.magnitude, walked_into, p.actions.combo, p.request);
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
        // Blows taken: flinch, knockback or knockdown. A flinch only
        // interrupts standing and moving about; the rest override.
        let (mut hit_damage, mut hit_flags, mut hit_push) = std::mem::take(&mut p.pending_hit);
        // Invulnerable: no reaction at all.
        if cut || p.armour_bits & crate::damage::resists::INVULNERABLE != 0 {
            (hit_damage, hit_flags, hit_push) = (0.0, 0, Vec3::ZERO);
        }
        let (reaction, knock, reaction_face) = hit_reaction(hit_damage, hit_flags, hit_push, facing);
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
        // Turbo attacks: a full meter swings ATTPWRC, 40 or more ATTPWRB.
        if intent == Intent::Turbo {
            if p.turbo >= TURBO_MAX {
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
    let (damage, kind) = combat::blow(strike, p.strength, &found);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_flinch_knock_back_or_down_like_the_game() {
        let from_front = Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(hit_reaction(0.5, 0, from_front, 0.0).0, 0, "a scratch does nothing");
        assert_eq!(hit_reaction(5.0, 0x4000_0000, from_front, 0.0).0, 1);
        let (class, speed, face) = hit_reaction(5.0, 0x10, from_front, 0.0);
        assert_eq!((class, speed), (11, 16.0), "pushed back by a blow from the front");
        assert!(face.unwrap().abs() < 1e-5, "turns to face the attacker");
        assert_eq!(hit_reaction(5.0, 0x20, -from_front, 0.0).0, 20);
        assert_eq!(reaction_action(21), Some(Action(0x83)));
    }
}
