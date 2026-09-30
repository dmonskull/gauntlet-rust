//! The hero's own state as the game keeps it in its player record: health,
//! gold, keys, potions, level and experience, and the timed powerups it
//! carries (`docs/items.md` has the record offsets and the code behind each
//! rule). It outlives levels, like the game's record does.
//!
//! Other systems change it through [`PlayerState`]'s methods or, to hurt the
//! hero without touching its internals, by writing a [`DamagePlayer`]
//! message.

use bevy::prelude::*;
use gdl_formats::pdata::PlayerStats;

use crate::audio::PlaySound;
use crate::level::LoadedGame;
use crate::player::{PlayerChoice, PlayerSpawn, PlayerTick};
use crate::population::LevelPopulation;
use crate::quest::Quest;

/// Health a new hero starts with.
pub const START_HEALTH: f32 = 500.0;
/// Health never goes above this, whatever the level.
pub const HEALTH_CAP: f32 = 9999.0;
/// Gold stops counting here.
pub const GOLD_CAP: u32 = 99_999;
/// The key ring holds this many (a level setting in the game, set to 9 for
/// every level when it loads).
pub const MAX_KEYS: u32 = 9;
/// Potion slots (same source as [`MAX_KEYS`]).
pub const MAX_POTIONS: usize = 9;
/// The player record has this many powerup slots.
pub const POWER_SLOTS: usize = 11;
/// Highest hero level.
pub const MAX_LEVEL: u32 = 99;
/// Below this much health the hero dies.
const DEATH_BELOW: f32 = 1.0;

/// Stand-ins for the hero's size until the class record loads (the values
/// every class record on the disc holds).
const DEFAULT_HEIGHT: f32 = 5.0;
const DEFAULT_RADIUS: f32 = 1.5;
/// Every class's top point is this high (`PDAT +0x50`).
pub const DEFAULT_HEAD: f32 = 4.4;

/// Hurts the hero by `amount` health (negative: heals it). Senders pass
/// what should come off, after armour and the armour powers
/// (`Player::take_blow`).
#[derive(Message, Clone, Copy, Debug)]
pub struct DamagePlayer {
    pub amount: f32,
}

/// A timed or counted powerup the hero carries — one slot of the record's
/// eleven.
#[derive(Clone, Debug, PartialEq)]
pub struct Power {
    /// The item subtype that granted it (5 weapon, 6 armour, 7 speed,
    /// 8 magic, 9 special).
    pub subtype: i32,
    /// The item type's value: which power, as bits.
    pub value: u32,
    /// The item type's amount (shots, strength...), summed over pickups.
    pub amount: f32,
    /// Seconds left; negative for powers that don't run out by time.
    pub time: f32,
}

/// What healing did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heal {
    /// Already at full health; nothing was added (food stays on the floor).
    Refused,
    /// Healed and topped out at the maximum.
    Filled,
    /// Healed, still below the maximum.
    Healed,
}

#[derive(Resource, Clone, Debug)]
pub struct PlayerState {
    pub health: f32,
    pub level: u32,
    pub experience: u32,
    pub gold: u32,
    pub keys: u32,
    /// Each potion's kind (the item type's value), in pickup order.
    pub potions: Vec<i32>,
    pub powers: Vec<Power>,
    /// Runestones held, by the stone's number (its item type's amount).
    pub runestones: Vec<i32>,
    /// Crystals, gargoyle pieces, legendary items and the levels entered
    /// (`quest.rs`).
    pub quest: Quest,
    /// The last quest piece picked up, for the HUD's count: a crystal
    /// counter, or `0x100` + a gargoyle piece; and when (game seconds).
    pub popup: Option<(u16, f32)>,
    /// Realms whose boss this hero has beaten, a bit per realm id (the
    /// record's `+0x1EC8`, set by the boss's death for every player).
    pub realms_beaten: u32,
    pub alive: bool,
    /// The class's three-letter code (`WAR`), for its voice and sounds.
    pub class: String,
    /// The hero's radius and half height against items (class record).
    pub radius: f32,
    pub half_height: f32,
    /// How far the hero's top point is above the feet (class record
    /// `+0x50`): what the camera looks at.
    pub head_height: f32,
    /// The class's powerup duration factor.
    pub powerup_time: f32,
    /// What its powerups add up to this tick.
    pub bits: PowerBits,
    /// Fields until the next low-health warning.
    warning_timer: i32,
}

/// What a hero's powerups add up to (the game's stats routine, every
/// tick): its weapon bits (player `+0x11C`, which its blows and missiles
/// carry — one element, the longest-lasting, in the low four), armour
/// (`+0x120`) and special bits (`+0x124`, with `0x10000` while a speed
/// power runs), what speed and magic powers add to its speed (`+0x110`)
/// and magic power (`+0x10C`), and the turbo a turbo power fills in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PowerBits {
    pub weapon: u32,
    pub armour: u32,
    pub special: u32,
    pub speed: f32,
    pub magic: f32,
    pub turbo: f32,
}

/// Powerup subtypes: weapon, armour, speed, magic, special.
pub mod power {
    pub const WEAPON: i32 = 5;
    pub const ARMOUR: i32 = 6;
    pub const SPEED: i32 = 7;
    pub const MAGIC: i32 = 8;
    pub const SPECIAL: i32 = 9;
    /// The element in a weapon's low four bits.
    pub const ELEMENT: u32 = 0xF;
    /// Special bits: levitation, the turbo refill, a speed power running.
    pub const LEVITATE: u32 = 0x1;
    pub const TURBO: u32 = 0x8_0000;
    pub const SPEEDING: u32 = 0x1_0000;
}

/// The tower's realm id: powerups don't run down there.
const TOWER_REALM: u32 = 13;

/// A turbo power fills the meter by this, up to its top.
const TURBO_FILL: f32 = 100.0;

impl Default for PlayerState {
    fn default() -> Self {
        Self::new("WAR", None)
    }
}

impl PlayerState {
    /// A new hero of `class`, as the game resets one: level 1, no
    /// experience, gold, keys or potions, 500 health.
    pub fn new(class: &str, stats: Option<&PlayerStats>) -> Self {
        Self {
            health: START_HEALTH,
            level: 1,
            experience: 0,
            gold: 0,
            keys: 0,
            potions: Vec::new(),
            powers: Vec::new(),
            bits: PowerBits::default(),
            runestones: Vec::new(),
            quest: Quest::default(),
            popup: None,
            realms_beaten: 0,
            alive: true,
            class: class.to_ascii_uppercase(),
            radius: stats.map_or(DEFAULT_RADIUS, |s| s.body.radius),
            half_height: 0.5 * stats.map_or(DEFAULT_HEIGHT, |s| s.body.height),
            head_height: stats.map_or(DEFAULT_HEAD, |s| s.body.head_height),
            powerup_time: stats.map_or(1.0, |s| s.powerup_time),
            warning_timer: 0,
        }
    }

    /// 100 more per level above the first, from 500, capped.
    pub fn max_health(&self) -> f32 {
        (100.0 * (self.level.max(1) - 1) as f32 + START_HEALTH).min(HEALTH_CAP)
    }

    /// Adds health the way the game does: refused at full health (for a
    /// positive amount), otherwise added and capped at the maximum.
    pub fn heal(&mut self, amount: f32) -> Heal {
        let max = self.max_health();
        if amount > 0.0 && self.health >= max {
            return Heal::Refused;
        }
        self.health += amount;
        if self.health > max {
            self.health = max;
            Heal::Filled
        } else {
            Heal::Healed
        }
    }

    /// Experience needed to go from `level` to the next one.
    pub fn experience_to_leave(level: u32) -> u32 {
        let l = level;
        if l + 1 < 61 { l * ((l + 1) * 30 + 1000) } else { (l - 59) * 4600 + 165_200 }
    }

    /// Adds experience and raises the level while it's enough (at most 99),
    /// as the game does; returns the levels gained.
    pub fn add_experience(&mut self, amount: u32) -> u32 {
        self.experience = self.experience.saturating_add(amount);
        let mut gained = 0;
        while self.level < MAX_LEVEL && Self::experience_to_leave(self.level) <= self.experience {
            self.level += 1;
            gained += 1;
        }
        gained
    }

    /// Takes health away; returns `true` if this killed the hero. A
    /// negative amount — the gold armour's heal — adds health, with no cap
    /// (the game's hurt-player routine has none).
    pub fn damage(&mut self, amount: f32) -> bool {
        if !self.alive || amount == 0.0 {
            return false;
        }
        self.health -= amount;
        if self.health < DEATH_BELOW {
            self.health = 0.0;
            self.alive = false;
            return true;
        }
        false
    }

    pub fn add_gold(&mut self, amount: u32) {
        self.gold = (self.gold + amount).min(GOLD_CAP);
    }

    /// Takes up to `count` keys: all of them if they fit on the ring, as
    /// many as fit otherwise. Returns how many were taken.
    pub fn take_keys(&mut self, count: u32) -> u32 {
        let taken = count.min(MAX_KEYS.saturating_sub(self.keys));
        self.keys += taken;
        taken
    }

    /// Uses a key if there is one.
    pub fn use_key(&mut self) -> bool {
        let had = self.keys > 0;
        self.keys = self.keys.saturating_sub(1);
        had
    }

    /// Adds up to `count` potions of `kind`; returns how many fitted.
    /// The runestones held as bits (stone n → bit n), as the game keeps
    /// them.
    pub fn runestone_bits(&self) -> u32 {
        self.runestones.iter().filter(|&&n| (0..32).contains(&n)).fold(0, |b, &n| b | (1 << n))
    }

    /// Whether the exit to `level` (0 the first) of `realm` is open for
    /// this hero (`quest.rs`).
    pub fn exit_open(&self, realm: u32, level: u32) -> bool {
        self.quest.exit_open(realm, level, self.realms_beaten, self.runestone_bits())
    }

    pub fn take_potions(&mut self, kind: i32, count: u32) -> u32 {
        let room = MAX_POTIONS.saturating_sub(self.potions.len()) as u32;
        let taken = count.min(room);
        self.potions.extend(std::iter::repeat_n(kind, taken as usize));
        taken
    }

    /// Grants a powerup: the same power again adds its amount and half its
    /// time (or takes a negative time outright); a new one takes a free
    /// slot, or the one closest to running out.
    pub fn grant_power(&mut self, subtype: i32, value: u32, amount: f32, duration: f32) {
        let time = duration * self.powerup_time;
        if let Some(p) = self.powers.iter_mut().find(|p| p.subtype == subtype && p.value == value) {
            if amount > 0.0 {
                p.amount += amount;
            }
            if p.time >= 0.0 && time > 0.0 {
                p.time += 0.5 * time;
            } else if time < 0.0 {
                p.time = time;
            }
            return;
        }
        let power = Power { subtype, value, amount, time };
        if self.powers.len() < POWER_SLOTS {
            self.powers.push(power);
        } else if let Some(p) = self
            .powers
            .iter_mut()
            .filter(|p| p.time >= 0.0)
            .min_by(|a, b| a.time.total_cmp(&b.time))
        {
            *p = power;
        }
    }

    /// The powerups' tick (the game's stats routine): their clocks run
    /// down by `dt` seconds ([`power_clock`] decides how many), and what
    /// they add up to is worked out — a power counts on the tick it runs
    /// out, then it's dropped; a turbo power is spent at once.
    pub fn tick_powers(&mut self, dt: f32) -> PowerBits {
        let mut b = PowerBits::default();
        let mut element_time = -1.0f32;
        for p in &mut self.powers {
            if p.time == 0.0 {
                continue;
            }
            if p.time > 0.0 {
                p.time = (p.time - dt).max(0.0);
            }
            match p.subtype {
                power::WEAPON if p.value & power::ELEMENT == 0 => b.weapon |= p.value,
                power::WEAPON => {
                    // One element: the one that lasts longest.
                    if element_time < 0.0 || (p.time > 0.0 && p.time > element_time) {
                        b.weapon = (b.weapon & !power::ELEMENT) | p.value;
                        element_time = p.time;
                    }
                    b.weapon |= p.value & !power::ELEMENT;
                }
                power::ARMOUR => b.armour |= p.value,
                power::SPEED => {
                    b.speed += p.amount;
                    b.special |= power::SPEEDING;
                }
                power::MAGIC => b.magic += p.amount,
                power::SPECIAL => {
                    b.special |= p.value;
                    if p.value & power::TURBO != 0 {
                        b.turbo += TURBO_FILL;
                        if p.time >= 0.0 {
                            p.time = 0.0;
                        }
                    }
                }
                _ => {}
            }
        }
        self.powers.retain(|p| p.time != 0.0);
        self.bits = b;
        b
    }
}

pub struct PlayerStatePlugin;

impl Plugin for PlayerStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamagePlayer>()
            .init_resource::<PlayerState>()
            // A new hero whenever the class choice changes.
            .add_systems(Update, new_hero.run_if(resource_changed::<PlayerChoice>).before(PlayerSpawn))
            .add_systems(FixedUpdate, (take_damage, powers_and_warning.in_set(PowersTick)).chain().after(PlayerTick))
            .add_systems(Update, test_powers.run_if(resource_exists_and_changed::<LevelPopulation>));
    }
}

/// Sets up the chosen class's hero with its class record, and a loaded
/// character's saved record on top (`saves.rs`).
fn new_hero(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    choice: Option<Res<PlayerChoice>>,
    saves: Option<ResMut<crate::saves::Saves>>,
) {
    let class = choice.map_or_else(|| "WAR".to_string(), |c| c.class.clone());
    let stats = game
        .install
        .read(&format!("PDATA/{class}.WAD"))
        .ok()
        .and_then(|b| PlayerStats::parse(&b).ok().flatten());
    if stats.is_none() {
        warn!("no PDATA record for {class}; using stand-in hero size");
    }
    let mut state = PlayerState::new(&class, stats.as_ref());
    // Debugging aid: `GDL_KEYS=n` starts the hero with n keys.
    if let Some(keys) = std::env::var("GDL_KEYS").ok().and_then(|k| k.parse().ok()) {
        state.take_keys(keys);
    }
    if let Some(saved) = saves.and_then(|mut s| s.pending.take()) {
        saved.apply(&mut state);
    }
    commands.insert_resource(state);
}

fn take_damage(
    mut hits: MessageReader<DamagePlayer>,
    mut state: ResMut<PlayerState>,
    camera: Option<Res<crate::play_camera::PlayCamera>>,
) {
    // No harm comes to the hero during a camera cut.
    let cut = camera.is_some_and(|c| c.in_cut());
    for hit in hits.read() {
        if cut {
            continue;
        }
        let died = state.damage(hit.amount);
        debug!("the hero takes {:.1}: {:.1} health", hit.amount, state.health);
        if died {
            info!("the hero has died");
        }
    }
}

/// Fields (1/60 s) the game counts per 30 Hz tick.
pub const FIELDS_PER_TICK: i32 = 2;

/// The powerups' tick.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PowersTick;

/// `GDL_POWERS="<subtype>:<value>[:<amount>[:<seconds>]],…"` (a testing
/// aid): powerups granted at the first level start, as picking them up
/// would (`0x…` values allowed; seconds default to 60, −1 for counted
/// ones). E.g. `5:1` a fire weapon, `7:0:4:40` a speed boost.
fn test_powers(mut state: ResMut<PlayerState>, mut done: Local<bool>) {
    if *done {
        return;
    }
    *done = true;
    let Ok(spec) = std::env::var("GDL_POWERS") else { return };
    let num = |s: &str| -> Option<f32> {
        match s.trim().strip_prefix("0x") {
            Some(h) => u32::from_str_radix(h, 16).ok().map(|v| v as f32),
            None => s.trim().parse().ok(),
        }
    };
    for part in spec.split(',') {
        let f: Vec<&str> = part.split(':').collect();
        let (Some(subtype), Some(value)) = (f.first().and_then(|s| num(s)), f.get(1).and_then(|s| s.trim().strip_prefix("0x").map_or_else(|| s.trim().parse().ok(), |h| u32::from_str_radix(h, 16).ok())))
        else {
            continue;
        };
        let amount = f.get(2).and_then(|s| num(s)).unwrap_or(0.0);
        let seconds = f.get(3).and_then(|s| num(s)).unwrap_or(60.0);
        state.grant_power(subtype as i32, value, amount, seconds);
        info!("GDL_POWERS: subtype {subtype} value {value:#x} amount {amount} for {seconds} s");
    }
}

/// How fast the powerups' clocks run: not at all in the tower or during a
/// camera cut; on a boss level three times as fast while the boss is awake
/// and alive, and not at all before it wakes or once it's dead.
pub fn power_clock(in_tower: bool, cut: bool, boss_level: bool, boss_awake: bool, boss_dead: bool) -> f32 {
    if in_tower || cut {
        0.0
    } else if !boss_level {
        1.0
    } else if boss_awake && !boss_dead {
        BOSS_POWER_CLOCK
    } else {
        0.0
    }
}

/// Powerups run down this much faster in a boss fight.
const BOSS_POWER_CLOCK: f32 = 3.0;

/// Counts powerups down ([`power_clock`]) and adds them up, and sounds
/// the low-health warning: at 200 health or less the game plays `S_WARN`
/// every 120 fields (60 below 100, 30 below 25) — not in the tower, or
/// while invulnerable.
fn powers_and_warning(
    time: Res<Time>,
    mut state: ResMut<PlayerState>,
    population: Option<Res<LevelPopulation>>,
    camera: Option<Res<crate::play_camera::PlayCamera>>,
    level: Option<Res<crate::monsters::MonsterLevel>>,
    boss: Option<Res<crate::critters::BossWatch>>,
    mut sound: MessageWriter<PlaySound>,
) {
    let in_tower = population.as_ref().and_then(|p| crate::quest::level_of(&p.level)).is_some_and(|(realm, _)| realm == TOWER_REALM);
    let cut = camera.is_some_and(|c| c.in_cut());
    let boss_level = level.is_some_and(|l| l.boss >= 0);
    let (awake, dead) = boss.map_or((false, false), |b| (b.awake, b.dead));
    state.tick_powers(time.delta_secs() * power_clock(in_tower, cut, boss_level, awake, dead));
    if !state.alive || state.health > 200.0 {
        return;
    }
    state.warning_timer -= FIELDS_PER_TICK;
    if state.warning_timer < 1 {
        let invulnerable = state.bits.armour & (crate::damage::resists::INVULNERABLE | crate::damage::resists::GOLD) != 0;
        if !in_tower && !invulnerable {
            sound.write(PlaySound("S_WARN".into()));
        }
        state.warning_timer = match state.health {
            h if h < 25.0 => 30,
            h if h < 100.0 => 60,
            _ => 120,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_follows_the_game() {
        let mut s = PlayerState::default();
        assert_eq!(s.max_health(), 500.0);
        assert_eq!(s.heal(10.0), Heal::Refused, "food is refused at full health");
        s.damage(100.0);
        assert_eq!(s.heal(25.0), Heal::Healed);
        assert_eq!(s.health, 425.0);
        assert_eq!(s.heal(200.0), Heal::Filled);
        assert_eq!(s.health, 500.0);
        // Poisoned food is a negative heal and always lands.
        assert_eq!(s.heal(-50.0), Heal::Healed);
        assert_eq!(s.health, 450.0);
        s.level = 5;
        assert_eq!(s.max_health(), 900.0);
        s.level = 200;
        assert_eq!(s.max_health(), HEALTH_CAP);
        assert!(!s.damage(449.0) && s.alive, "1 health left is still alive");
        assert!(s.damage(0.5) && !s.alive && s.health == 0.0);
        assert!(!s.damage(10.0), "dead heroes don't die again");
    }

    #[test]
    fn experience_raises_the_level_like_the_game() {
        let mut s = PlayerState::default();
        assert_eq!(PlayerState::experience_to_leave(1), 1060);
        assert_eq!(PlayerState::experience_to_leave(2), 2180);
        assert_eq!(s.add_experience(1059), 0);
        assert_eq!(s.add_experience(1), 1);
        assert_eq!(s.level, 2);
        // Thresholds are totals: 2180, 3360, 4600, 5900, 7260, 8680, 10160
        // reach levels 3–9 with 11060; level 10 needs 11700.
        assert_eq!(s.add_experience(10_000), 7);
        assert_eq!(s.level, 9);
        assert_eq!(PlayerState::experience_to_leave(60), 4600 + 165_200);
    }

    #[test]
    fn keys_potions_gold_cap() {
        let mut s = PlayerState::default();
        assert_eq!(s.take_keys(3), 3);
        assert_eq!(s.take_keys(10), 6, "the ring takes what fits");
        assert_eq!(s.take_keys(1), 0);
        assert!(s.use_key() && s.keys == 8);
        assert_eq!(s.take_potions(2, 1), 1);
        assert_eq!(s.take_potions(1, 20), 8);
        assert_eq!(s.potions.len(), MAX_POTIONS);
        s.add_gold(99_990);
        s.add_gold(100);
        assert_eq!(s.gold, GOLD_CAP);
    }

    #[test]
    fn powers_add_up_as_the_game_does() {
        let mut s = PlayerState { powerup_time: 1.0, ..PlayerState::default() };
        s.grant_power(power::WEAPON, 1, 0.0, 30.0);
        s.grant_power(power::WEAPON, 2, 0.0, 90.0);
        s.grant_power(power::WEAPON, 0x80000, 0.0, 45.0);
        s.grant_power(power::SPEED, 0, 4.0, 40.0);
        s.grant_power(power::SPECIAL, power::TURBO, 0.0, 1.0);
        // The longest-lasting element wins; other weapon bits add.
        let b = s.tick_powers(0.5);
        assert_eq!(b.weapon, 0x80002);
        assert_eq!((b.speed, b.special & power::SPEEDING), (4.0, power::SPEEDING));
        // The turbo power fills the meter once and is spent.
        assert_eq!(b.turbo, 100.0);
        assert_eq!(s.tick_powers(0.5).turbo, 0.0);
        // Held (the tower): nothing runs down.
        let before: Vec<f32> = s.powers.iter().map(|p| p.time).collect();
        s.tick_powers(5.0 * power_clock(true, false, false, false, false));
        assert_eq!(before, s.powers.iter().map(|p| p.time).collect::<Vec<_>>());
    }

    #[test]
    fn a_boss_fight_runs_the_clocks_fast() {
        assert_eq!(power_clock(false, false, false, false, false), 1.0);
        assert_eq!(power_clock(false, true, false, false, false), 0.0);
        // Before the boss wakes, while it fights, once it's dead.
        assert_eq!(power_clock(false, false, true, false, false), 0.0);
        assert_eq!(power_clock(false, false, true, true, false), 3.0);
        assert_eq!(power_clock(false, false, true, true, true), 0.0);
    }

    #[test]
    fn powers_stack_and_expire() {
        let mut s = PlayerState { powerup_time: 1.3, ..PlayerState::default() };
        s.grant_power(5, 1, 0.0, 90.0);
        assert!((s.powers[0].time - 117.0).abs() < 1e-3, "class factor scales the time");
        s.grant_power(5, 1, 0.0, 90.0);
        assert!((s.powers[0].time - 175.5).abs() < 1e-3, "the same power adds half its time");
        s.grant_power(5, 0x10_0000, 5.0, -1.0);
        s.grant_power(5, 0x10_0000, 5.0, -1.0);
        assert_eq!(s.powers[1].amount, 10.0);
        s.tick_powers(200.0);
        assert_eq!(s.powers.len(), 1, "timed power ran out, counted one stays");
        for v in 0..20 {
            s.grant_power(9, 1 << v, 0.0, v as f32 + 1.0);
        }
        assert_eq!(s.powers.len(), POWER_SLOTS);
    }
}
