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
use crate::player::{PlayerChoice, PlayerTick};

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
/// Below this much health the hero dies.
const DEATH_BELOW: f32 = 1.0;

/// Stand-ins for the hero's size until the class record loads (the values
/// every class record on the disc holds).
const DEFAULT_HEIGHT: f32 = 5.0;
const DEFAULT_RADIUS: f32 = 1.5;

/// Hurts the hero by `amount` health. The game runs damage through armour
/// and the level's difficulty factor before it lands; senders pass the
/// amount that should come off (see `docs/items.md`).
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
    /// Legendary items, gems and quest pieces picked up: (subtype, amount).
    pub treasures: Vec<(i32, i32)>,
    pub alive: bool,
    /// The class's three-letter code (`WAR`), for its voice and sounds.
    pub class: String,
    /// The hero's radius and half height against items (class record).
    pub radius: f32,
    pub half_height: f32,
    /// The class's powerup duration factor.
    pub powerup_time: f32,
    /// Fields until the next low-health warning.
    warning_timer: i32,
}

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
            runestones: Vec::new(),
            treasures: Vec::new(),
            alive: true,
            class: class.to_ascii_uppercase(),
            radius: stats.map_or(DEFAULT_RADIUS, |s| s.body.radius),
            half_height: 0.5 * stats.map_or(DEFAULT_HEIGHT, |s| s.body.height),
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

    /// Takes health away; returns `true` if this killed the hero.
    pub fn damage(&mut self, amount: f32) -> bool {
        if !self.alive || amount <= 0.0 {
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

    /// Runs the powerup clocks down by `dt` seconds, dropping the ones
    /// that run out.
    pub fn tick_powers(&mut self, dt: f32) {
        for p in &mut self.powers {
            if p.time > 0.0 {
                p.time = (p.time - dt).max(0.0);
            }
        }
        self.powers.retain(|p| p.time != 0.0);
    }
}

pub struct PlayerStatePlugin;

impl Plugin for PlayerStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamagePlayer>()
            .init_resource::<PlayerState>()
            .add_systems(Startup, new_hero)
            .add_systems(FixedUpdate, (take_damage, powers_and_warning).chain().after(PlayerTick));
    }
}

/// Sets up the chosen class's hero with its class record.
fn new_hero(mut commands: Commands, mut game: ResMut<LoadedGame>, choice: Option<Res<PlayerChoice>>) {
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
    commands.insert_resource(state);
}

fn take_damage(mut hits: MessageReader<DamagePlayer>, mut state: ResMut<PlayerState>) {
    for hit in hits.read() {
        if state.damage(hit.amount) {
            info!("the hero has died");
        }
    }
}

/// Fields (1/60 s) the game counts per 30 Hz tick.
pub const FIELDS_PER_TICK: i32 = 2;

/// Counts powerups down and sounds the low-health warning: at 200 health
/// or less the game plays `S_WARN` every 120 fields (60 below 100, 30 below
/// 25).
fn powers_and_warning(time: Res<Time>, mut state: ResMut<PlayerState>, mut sound: MessageWriter<PlaySound>) {
    state.tick_powers(time.delta_secs());
    if !state.alive || state.health > 200.0 {
        return;
    }
    state.warning_timer -= FIELDS_PER_TICK;
    if state.warning_timer < 1 {
        sound.write(PlaySound("S_WARN".into()));
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
