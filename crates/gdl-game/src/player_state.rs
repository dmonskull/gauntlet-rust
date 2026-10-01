//! The hero's own state as the game keeps it in its player record: health,
//! gold, keys, potions, level and experience, and the timed powerups it
//! carries (`docs/items.md` has the record offsets and the code behind each
//! rule). It outlives levels, like the game's record does.
//!
//! Other systems change it through [`PlayerState`]'s methods or, to hurt the
//! hero without touching its internals, by writing a [`DamagePlayer`] (or a
//! [`HurtHero`], which says how the blow is voiced) message. The hero's
//! cries as it's hurt and as it dies are the game's damage routine's
//! (`docs/audio-format.md`, "The hero's cries"). `GDL_IMMORTAL=1`
//! (testing: scripted tours of a level) keeps the hero at its last hit
//! point instead of dying.

use bevy::prelude::*;
use gdl_formats::pdata::PlayerStats;

use gdl_install::GameInstall;

use crate::audio::{CALL_VOLUME, PlaySoundAt, QueueHeroLine, QueueVoice};
use crate::party::{Devices, MAX_PLAYERS, Member, Party};
use crate::player::{PlayerChoice, PlayerTick};
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
/// Rising a level (or several at once) heals the hero this much.
const LEVEL_UP_HEALTH: f32 = 100.0;
/// Below this much health the hero dies.
const DEATH_BELOW: f32 = 1.0;

/// `GDL_IMMORTAL=1`: the hero never dies (testing).
fn immortal() -> bool {
    static IMMORTAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IMMORTAL.get_or_init(|| std::env::var("GDL_IMMORTAL").is_ok_and(|v| !v.is_empty() && v != "0"))
}

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
    /// The hero's slot (`party.rs`).
    pub slot: usize,
    pub amount: f32,
}

/// How a blow on the hero is voiced: the game's damage routine's sound mode,
/// which whoever lands the blow chooses (`docs/audio-format.md`, "The
/// hero's cries").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cry {
    /// Nothing: the sender played its own sound (a big monster's blow), or
    /// it's poisoned food.
    Silent,
    /// The hurt sound by the blow's kind, at most every half second, and a
    /// pain line for a blow of 61 or more, or each 30 health lost.
    #[default]
    Hurt,
    /// A pain line (fire holes and the like).
    Pain,
    /// The class's scream, `S_<CLS>DIE1` (spikes, blades, tentacles).
    Scream,
}

/// Hurts the hero by `amount` like [`DamagePlayer`], for a blow of `kind`
/// voiced as `cry`. A `DamagePlayer` is a blow of kind 0 voiced
/// [`Cry::Hurt`].
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct HurtHero {
    /// The hero's slot (`party.rs`).
    pub slot: usize,
    pub amount: f32,
    pub kind: u32,
    pub cry: Cry,
}

/// Heals the hero the way the game's heal routine does (refused at full
/// health, capped at the maximum): the Health Vampire's drink.
#[derive(Message, Clone, Copy, Debug)]
pub struct HealPlayer {
    pub slot: usize,
    pub amount: f32,
}

/// Spends one use of a counted power: the first slot of `subtype` with
/// any of `bits` (a breath, the crossbow, the hammer).
#[derive(Message, Clone, Copy, Debug)]
pub struct SpendPower {
    pub slot: usize,
    pub subtype: i32,
    pub bits: u32,
}

/// A change to the party from the select screen.
#[derive(Message, Clone, Debug)]
pub enum PartyChange {
    /// `slot` plays `choice` under `name` from `devices` — with a fresh
    /// record (a saved character's laid on it), or, not fresh, the record
    /// it has (its name changed).
    Set {
        slot: usize,
        choice: PlayerChoice,
        name: String,
        saved: Option<Box<crate::saves::SavedCharacter>>,
        fresh: bool,
        devices: Devices,
    },
    /// `slot`'s player leaves the game.
    Leave(usize),
    /// Everyone leaves (a new game).
    Clear,
}

/// A timed or counted powerup the hero carries — one slot of the record's
/// eleven (empty while its time is 0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
    pub state: SlotState,
}

/// What a power slot is doing (the game's byte `+0x1E0 + slot`,
/// `docs/powers.md` "Held powers"): a power picked up is held until the
/// hero turns it on in the power menu (`power_menu.rs`), and one turned
/// off keeps its time for later. Only a slot that's on runs down and
/// works.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlotState {
    #[default]
    Empty,
    Held,
    On,
    Off,
}

impl Power {
    /// The slot holds a power.
    pub fn live(&self) -> bool {
        self.time != 0.0
    }

    /// The slot's power is on: it runs down and works.
    pub fn active(&self) -> bool {
        self.live() && self.state == SlotState::On
    }
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

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub health: f32,
    pub level: u32,
    pub experience: u32,
    pub gold: u32,
    pub keys: u32,
    /// Each potion's kind (the item type's value), in pickup order.
    pub potions: Vec<i32>,
    pub powers: [Power; POWER_SLOTS],
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
    /// Special bits: levitation, invisibility, the turbo refill, a speed
    /// power running.
    pub const LEVITATE: u32 = 0x1;
    pub const INVISIBLE: u32 = 0x4;
    /// Stops time for the enemies ([`super::TimeStop`]).
    pub const TIME_STOP: u32 = 0x8;
    pub const GROW: u32 = 0x100;
    /// Shrinks the enemies, not the hero ([`super::EnemyScale`]).
    pub const SHRINK: u32 = 0x200;
    /// The hero becomes Pojo.
    pub const POJO: u32 = 0x400;
    pub const TURBO: u32 = 0x8_0000;
    pub const SPEEDING: u32 = 0x1_0000;
}

/// The enemies' scale (the game's `r13-0x7320`, set every frame): 1,
/// × 0.667 for each playing hero with the shrink power, outside boss
/// levels. Below 1 monsters and critters are drawn at it, take twice the
/// damage and deal half (`docs/powers.md`, "`0x200` shrink").
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct EnemyScale(pub f32);

impl Default for EnemyScale {
    fn default() -> Self {
        Self(1.0)
    }
}

impl EnemyScale {
    pub fn shrunk(self) -> bool {
        self.0 < 1.0
    }
}

/// Time stopped (the game's `r13-0x731C`, set every frame while any
/// playing hero has the time-stop power): monsters stand frozen, critters
/// hold their moves, generators don't make any and damage tiles stay off
/// (`docs/powers.md`, "`0x8` time stop").
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimeStop(pub bool);

/// Each shrinking hero takes the enemies down to this much.
const SHRINK_SCALE: f32 = 0.667;
/// The grow power's end has no sound from this level on (the hero is big
/// anyway).
const BIG_LEVEL: u32 = 99;

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
    /// What the machines compare of a hero's record online (`online.rs`):
    /// its numbers and holdings, not the HUD's popup.
    pub fn sync_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.health.to_bits(), self.level, self.experience, self.gold, self.keys, &self.potions, &self.runestones).hash(&mut h);
        (self.realms_beaten, self.alive, self.warning_timer).hash(&mut h);
        format!("{:?} {:?} {:?}", self.powers, self.quest, self.bits).hash(&mut h);
        h.finish()
    }

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
            powers: [Power::default(); POWER_SLOTS],
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
    /// as the game does; returns the levels gained. Rising heals the hero
    /// by 100 (once, however many levels; uncapped).
    pub fn add_experience(&mut self, amount: u32) -> u32 {
        self.experience = self.experience.saturating_add(amount);
        let mut gained = 0;
        while self.level < MAX_LEVEL && Self::experience_to_leave(self.level) <= self.experience {
            self.level += 1;
            gained += 1;
        }
        if gained > 0 && self.alive {
            self.health += LEVEL_UP_HEALTH;
        }
        gained
    }

    /// Takes experience away (a Death's drain; none at level 99): below
    /// its level's threshold the hero drops levels. Returns the levels
    /// lost.
    pub fn lose_experience(&mut self, amount: u32) -> u32 {
        if self.level >= MAX_LEVEL {
            return 0;
        }
        self.experience = self.experience.saturating_sub(amount);
        let mut lost = 0;
        while self.level > 1 && self.experience < Self::experience_to_leave(self.level - 1) {
            self.level -= 1;
            lost += 1;
        }
        lost
    }

    /// The experience a Death's drain takes (and the halo's drain of one
    /// gives) at a time: a hundredth of (level − 1) × 60 + 1000, or of
    /// 4600 from level 61.
    pub fn drain_step(&self) -> u32 {
        let step = if self.level < 61 { (self.level.max(1) - 1) * 60 + 1000 } else { 4600 };
        step / 100
    }

    /// Takes health away; returns `true` if this killed the hero. A
    /// negative amount — the gold armour's heal — adds health, with no cap
    /// (the game's hurt-player routine has none).
    pub fn damage(&mut self, amount: f32) -> bool {
        if !self.alive || amount == 0.0 {
            return false;
        }
        self.health -= amount;
        if self.health < DEATH_BELOW && immortal() {
            self.health = DEATH_BELOW;
        } else if self.health < DEATH_BELOW {
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
    /// Returns the slot it went to. A new power is held (off) until the
    /// hero turns it on; one already carried keeps its state.
    pub fn grant_power(&mut self, subtype: i32, value: u32, amount: f32, duration: f32) -> Option<usize> {
        let time = duration * self.powerup_time;
        if let Some(i) = self.powers.iter().position(|p| p.live() && p.subtype == subtype && p.value == value) {
            let p = &mut self.powers[i];
            if amount > 0.0 {
                p.amount += amount;
            }
            if p.time >= 0.0 && time > 0.0 {
                p.time += 0.5 * time;
            } else if time < 0.0 {
                p.time = time;
            }
            return Some(i);
        }
        // A free slot, else the one nearest to running out.
        let slot = self.powers.iter().position(|p| !p.live()).or_else(|| {
            (0..POWER_SLOTS)
                .filter(|&i| self.powers[i].time >= 0.0)
                .min_by(|&a, &b| self.powers[a].time.total_cmp(&self.powers[b].time))
        })?;
        self.powers[slot] = Power { subtype, value, amount, time, state: SlotState::Held };
        Some(slot)
    }

    /// Turns slot `i`'s power on, or off if it's on (the power menu's Up):
    /// a held or switched-off power comes on; one on is put away with the
    /// time it has left.
    pub fn toggle_power(&mut self, i: usize) {
        let Some(p) = self.powers.get_mut(i).filter(|p| p.live()) else { return };
        p.state = if p.state == SlotState::On { SlotState::Off } else { SlotState::On };
    }

    /// The powers that are on.
    pub fn active_powers(&self) -> impl Iterator<Item = &Power> {
        self.powers.iter().filter(|p| p.active())
    }

    /// Spends one use of a counted power (`docs/powers.md`, "Timing"): the
    /// first slot of `subtype` with any of `bits` loses 1 from its amount
    /// and ends at 0; a negative amount never runs out.
    pub fn spend_power(&mut self, subtype: i32, bits: u32) {
        let Some(p) = self.powers.iter_mut().find(|p| p.active() && p.subtype == subtype && p.value & bits != 0) else {
            return;
        };
        if p.amount < 0.0 {
            return;
        }
        p.amount -= 1.0;
        if p.amount <= 0.0 {
            *p = Power::default();
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
            if !p.active() {
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
        for p in &mut self.powers {
            if !p.live() {
                *p = Power::default();
            }
        }
        self.bits = b;
        b
    }
}

pub struct PlayerStatePlugin;

impl Plugin for PlayerStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamagePlayer>()
            .add_message::<HurtHero>()
            .add_message::<SpendPower>()
            .add_message::<HealPlayer>()
            .add_message::<PartyChange>()
            .add_systems(Update, set_members.before(crate::player::PlayerSpawn))
            .init_resource::<Party>()
            .init_resource::<EnemyScale>()
            .init_resource::<TimeStop>()
            .add_systems(
                FixedUpdate,
                (take_damage, spend_powers, powers_and_warning.in_set(PowersTick)).chain().after(PlayerTick),
            )
            .add_systems(Update, test_powers.run_if(resource_exists_and_changed::<LevelPopulation>));
    }
}

/// A new player of `choice`'s class: the class record's hero
/// (`PDATA/<class>.WAD`), a saved character's record laid on top
/// (`saves.rs`), and `GDL_KEYS=n` keys (testing).
pub fn new_member(
    install: &mut GameInstall,
    choice: PlayerChoice,
    name: &str,
    saved: Option<&crate::saves::SavedCharacter>,
    devices: Devices,
) -> Member {
    let stats = install.read(&format!("PDATA/{}.WAD", choice.class)).ok().and_then(|b| PlayerStats::parse(&b).ok().flatten());
    if stats.is_none() {
        warn!("no PDATA record for {}; using stand-in hero size", choice.class);
    }
    let mut state = PlayerState::new(&choice.class, stats.as_ref());
    if let Some(keys) = std::env::var("GDL_KEYS").ok().and_then(|k| k.parse().ok()) {
        state.take_keys(keys);
    }
    if let Some(saved) = saved {
        saved.apply(&mut state);
    }
    Member { choice, name: name.to_string(), state, devices }
}

/// Makes the select screen's changes to the party.
fn set_members(mut changes: MessageReader<PartyChange>, mut game: ResMut<crate::level::LoadedGame>, mut party: ResMut<Party>) {
    for change in changes.read() {
        match change {
            PartyChange::Set { slot, choice, name, saved, fresh, devices } => {
                if let Some(member) = party.get_mut(*slot)
                    && !fresh
                    && member.choice == *choice
                {
                    member.name = name.clone();
                    member.devices = *devices;
                    continue;
                }
                let member = new_member(&mut game.install, choice.clone(), name, saved.as_deref(), *devices);
                info!("player {}: {} the {} ({})", slot + 1, member.name, member.choice.class, member.choice.variant);
                party.join(*slot, member);
            }
            PartyChange::Leave(slot) => {
                if let Some(m) = party.leave(*slot) {
                    info!("player {}: {} leaves the game", slot + 1, m.name);
                }
            }
            PartyChange::Clear => *party = Party::default(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn take_damage(
    mut hits: MessageReader<DamagePlayer>,
    mut hurts: MessageReader<HurtHero>,
    mut heals: MessageReader<HealPlayer>,
    mut party: ResMut<Party>,
    camera: Option<Res<crate::play_camera::PlayCamera>>,
    heroes: Query<&crate::player::Player>,
    (mut cries, time): (Local<[Cries; MAX_PLAYERS]>, Res<Time>),
    (mut sounds, mut lines, mut announce): (MessageWriter<PlaySoundAt>, MessageWriter<QueueHeroLine>, MessageWriter<QueueVoice>),
) {
    for h in heals.read() {
        if let Some(state) = party.state_mut(h.slot)
            && state.alive
        {
            state.heal(h.amount);
        }
    }
    for c in cries.iter_mut() {
        c.wait -= FIELDS_PER_TICK;
    }
    // No harm comes to the heroes during a camera cut.
    let cut = camera.is_some_and(|c| c.in_cut());
    let blows: Vec<HurtHero> = hits
        .read()
        .map(|h| HurtHero { slot: h.slot, amount: h.amount, kind: 0, cry: Cry::Hurt })
        .chain(hurts.read().copied())
        .collect();
    for hit in blows {
        if cut {
            continue;
        }
        let Some(member) = party.get_mut(hit.slot) else { continue };
        let choice = member.choice.clone();
        let state = &mut member.state;
        let (in_play, before) = (state.alive, state.health);
        let died = state.damage(hit.amount);
        debug!("player {} takes {:.1}: {:.1} health", hit.slot + 1, hit.amount, state.health);
        if died {
            info!("player {} has died", hit.slot + 1);
        }
        // Only a hero in play cries out, at its feet.
        let feet = heroes.iter().find(|p| p.slot == hit.slot).map(|p| Vec3::from(p.mover.position));
        let Some(feet) = feet.filter(|_| in_play) else { continue };
        let cries = &mut cries[hit.slot.min(MAX_PLAYERS - 1)];
        let hero = Hero {
            class: &state.class,
            pojo: state.bits.special & power::POJO != 0,
            invulnerable: state.bits.armour & crate::damage::resists::INVULNERABLE != 0,
        };
        // The field count's parity picks between two warnings.
        let even = ((time.elapsed_secs() * 60.0) as u64).is_multiple_of(2);
        let voiced = if died { death_cries(hero) } else { cries.voice(hero, before, state.health, hit, even) };
        for v in voiced {
            debug!("the hero cries {v:?}");
            match v {
                Voiced::Sound(name, volume) => {
                    sounds.write(PlaySoundAt::panned(name, feet, volume));
                }
                Voiced::Line(line, volume) => {
                    lines.write(QueueHeroLine { line, volume, at: feet });
                }
                Voiced::Warning(line, most_wait) => {
                    let name = hero_name_line(Some(&choice), hero.pojo);
                    announce.write(QueueVoice::announcer(name, most_wait).then(line).gated());
                }
            }
        }
    }
}

/// The classes whose cries the game's tables hold (the secret characters
/// read past them).
const CRYING_CLASSES: [&str; 8] = ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"];
/// The hurt sounds by the blow's kind: `0x20000`, then `0x40000`, else
/// the plain one.
const HURT_KIND_2: u32 = 0x2_0000;
const HURT_KIND_3: u32 = 0x4_0000;
/// Poison (the blow kind): its hurt is the class's poisoned line.
const POISON_KIND: u32 = 0x800;
/// The hurt sound waits this many fields before it plays again.
const HURT_WAIT: i32 = 30;
/// A pain line for a blow this big, or every this much health lost.
const BIG_BLOW: i32 = 61;
const PAIN_EVERY: f32 = 30.0;
/// The announcer's warnings as the health falls past 150 ("needs food,
/// badly") and past 50 (its life force running out or it about to die,
/// by the field count's parity): the line after the hero's name, and the
/// sentence's longest wait.
const WARN_HIGH: (i32, &str, f32) = (150, "S_BADLY", 1.0);
const WARN_LOW: i32 = 50;
const WARN_LOW_EVEN: (&str, f32) = ("S_LIFEFORCE", 1.0);
const WARN_LOW_ODD: (&str, f32) = ("S_ABOUT", 0.5);
/// The colours' codes in the hero's name lines, and the Pojo's name.
const NAME_COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
const POJO_NAME: &str = "S_POJO2";

/// The hero's name as the announcer says it first (`S_BLUWAR2`, "Blue
/// Warrior"; the Pojo's `S_POJO2`).
fn hero_name_line(choice: Option<&PlayerChoice>, pojo: bool) -> String {
    if pojo {
        return POJO_NAME.into();
    }
    let colour = choice.and_then(|c| NAME_COLOURS.iter().position(|n| c.variant.to_ascii_uppercase().starts_with(n))).unwrap_or(0);
    let class = choice.and_then(|c| crate::character::class_index(&c.class)).unwrap_or(0);
    crate::tower_scenes::Rank { level: 0, class, colour }.name_line()
}
/// Requested volumes: the hurt sounds and `S_PLAYERDIES`, a scream and the
/// death cry, the pain lines, the poisoned line.
const SCREAM_VOLUME: u8 = 0xE0;
const PAIN_VOLUME: u8 = 0xE0;
const POISONED_VOLUME: u8 = 0xC0;

/// The hero crying out: its class, whether it's the Pojo, whether it's
/// invulnerable.
#[derive(Clone, Copy, Debug)]
struct Hero<'a> {
    class: &'a str,
    pojo: bool,
    invulnerable: bool,
}

impl Hero<'_> {
    fn cries(&self) -> bool {
        CRYING_CLASSES.contains(&self.class)
    }
}

/// A cry: a sound played at the hero's feet, or a line in the heroes'
/// voice queue, each at its requested volume; or the announcer's warning
/// (a line after the hero's name, and its longest wait).
#[derive(Clone, Debug, PartialEq)]
enum Voiced {
    Sound(String, u8),
    Line(String, u8),
    Warning(&'static str, f32),
}

/// The hero's death cries: `S_PLAYERDIES`, and the class's `S_<CLS>DIE2`
/// (the Pojo's `S_POJOPOISON`).
fn death_cries(hero: Hero) -> Vec<Voiced> {
    let mut out = vec![Voiced::Sound("S_PLAYERDIES".into(), CALL_VOLUME)];
    if hero.cries() {
        let cry = if hero.pojo { "S_POJOPOISON".to_string() } else { format!("S_{}DIE2", hero.class) };
        out.push(Voiced::Sound(cry, SCREAM_VOLUME));
    }
    out
}

/// What the game's damage routine remembers between blows: the fields
/// until the hurt sound may play again, the damage counted toward the
/// next pain line, and its dice.
#[derive(Default)]
struct Cries {
    wait: i32,
    lost: f32,
    seed: u32,
}

impl Cries {
    /// The cries for a blow `hit` the hero lived through, its health going
    /// from `before` to `after`; `even`, the field count's parity.
    fn voice(&mut self, hero: Hero, before: f32, after: f32, hit: HurtHero, even: bool) -> Vec<Voiced> {
        let mut out = Vec::new();
        let hurts = hit.amount > 0.0;
        if hurts {
            self.lost += hit.amount;
        }
        let (was, now) = ((before + 0.5) as i32, (after + 0.5) as i32);
        let mut cry = hit.cry;
        if was > WARN_HIGH.0 && now <= WARN_HIGH.0 {
            out.push(Voiced::Warning(WARN_HIGH.1, WARN_HIGH.2));
        } else if was > WARN_LOW && now <= WARN_LOW {
            let (line, wait) = if even { WARN_LOW_EVEN } else { WARN_LOW_ODD };
            out.push(Voiced::Warning(line, wait));
        } else {
            match cry {
                Cry::Pain => {
                    if hurts {
                        out.extend(self.pain_line(hero));
                    }
                    self.lost = 0.0;
                }
                Cry::Scream => {
                    if !hero.invulnerable && hero.cries() {
                        let scream = if hero.pojo { "S_POJOPAIN".to_string() } else { format!("S_{}DIE1", hero.class) };
                        out.push(Voiced::Sound(scream, SCREAM_VOLUME));
                    }
                    self.lost = 0.0;
                }
                Cry::Hurt if was - now < BIG_BLOW => {
                    if self.lost >= PAIN_EVERY {
                        self.lost -= PAIN_EVERY;
                        if hurts {
                            out.extend(self.pain_line(hero));
                        }
                        cry = Cry::Silent;
                    }
                }
                Cry::Hurt => {
                    if hurts {
                        out.extend(self.pain_line(hero));
                    }
                    cry = Cry::Silent;
                    self.lost = 0.0;
                }
                Cry::Silent => {}
            }
        }
        if cry == Cry::Hurt {
            if hit.kind & POISON_KIND != 0 {
                if hurts && hero.cries() {
                    let line = if hero.pojo { "S_POJOPOISON".to_string() } else { format!("S_{}POISON", hero.class) };
                    out.push(Voiced::Line(line, POISONED_VOLUME));
                }
            } else if self.wait < 1 {
                if hurts {
                    out.push(Voiced::Sound(hurt_sound(hit.kind).into(), CALL_VOLUME));
                }
                self.wait = HURT_WAIT;
            }
        }
        out
    }

    /// One of the class's four pain lines (the Pojo's own); none for the
    /// classes the game's table doesn't hold.
    fn pain_line(&mut self, hero: Hero) -> Option<Voiced> {
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let n = (self.seed >> 16) % 4 + 1;
        let line = if hero.pojo { "S_POJOPAIN".to_string() } else { format!("S_{}PAIN{n}", hero.class) };
        hero.cries().then_some(Voiced::Line(line, PAIN_VOLUME))
    }
}

/// The hurt sound for a blow of `kind`.
fn hurt_sound(kind: u32) -> &'static str {
    if kind & HURT_KIND_2 != 0 {
        "S_PLYRDMG2"
    } else if kind & HURT_KIND_3 != 0 {
        "S_PLYRDMG3"
    } else {
        "S_PLYRDMG"
    }
}

/// Spends the counted powers' uses (not in the tower).
fn spend_powers(mut spent: MessageReader<SpendPower>, mut party: ResMut<Party>, population: Option<Res<LevelPopulation>>) {
    let in_tower = population.as_ref().and_then(|p| crate::quest::level_of(&p.level)).is_some_and(|(realm, _)| realm == TOWER_REALM);
    for s in spent.read() {
        if !in_tower && let Some(state) = party.state_mut(s.slot) {
            state.spend_power(s.subtype, s.bits);
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
fn test_powers(mut party: ResMut<Party>, mut done: Local<bool>) {
    let Some(state) = party.state_mut(0) else { return };
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
        // Turned on at once (a power picked up is held until the hero turns
        // it on).
        if let Some(i) = state.grant_power(subtype as i32, value, amount, seconds)
            && state.powers[i].state != SlotState::On
        {
            state.toggle_power(i);
        }
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

/// The hero's top point above its feet (`PDAT +0x50`, every class).
const HERO_TOP: f32 = 4.4;
/// The powers' ends play at this requested volume.
const LAPSE_VOLUME: u8 = 0xE0;

/// The low-health warning's requested volume: louder the lower the
/// health — the call's own from 100, then 0x98, 0xB1 below 25 and 0xCA at
/// 10 or less.
fn warning_volume(health: f32) -> u8 {
    if health <= 10.0 {
        0xCA
    } else if health < 25.0 {
        0xB1
    } else if health < 100.0 {
        0x98
    } else {
        0x7F
    }
}

/// Counts powerups down ([`power_clock`]) and adds them up, sets the
/// enemies' scale and the time stop, plays the sounds of the levitation,
/// growth, shrink and Pojo running out, and sounds the low-health warning: at 200 health or less
/// the game plays `S_WARN` every 120 fields (60 below 100, 30 below 25) —
/// not in the tower, or while invulnerable.
#[allow(clippy::too_many_arguments)]
fn powers_and_warning(
    time: Res<Time>,
    mut party: ResMut<Party>,
    population: Option<Res<LevelPopulation>>,
    camera: Option<Res<crate::play_camera::PlayCamera>>,
    level: Option<Res<crate::monsters::MonsterLevel>>,
    boss: Option<Res<crate::critters::BossWatch>>,
    (mut enemies, mut stop): (ResMut<EnemyScale>, ResMut<TimeStop>),
    mut sound: MessageWriter<PlaySoundAt>,
    heroes: Query<&crate::player::Player>,
) {
    let in_tower = population.as_ref().and_then(|p| crate::quest::level_of(&p.level)).is_some_and(|(realm, _)| realm == TOWER_REALM);
    let cut = camera.is_some_and(|c| c.in_cut());
    let boss_level = level.is_some_and(|l| l.boss >= 0);
    let (awake, dead) = boss.map_or((false, false), |b| (b.awake, b.dead));
    let clock = time.delta_secs() * power_clock(in_tower, cut, boss_level, awake, dead);
    // The shrink and the time stop act on the whole level: any player's.
    let (mut shrink, mut time_stop) = (false, false);
    for (slot, state) in party.states_mut() {
        let before = state.bits.special;
        let now = state.tick_powers(clock).special;
        let ended = before & !now;
        shrink |= now & power::SHRINK != 0;
        time_stop |= state.alive && now & power::TIME_STOP != 0;
        // The powers' ends are heard at the hero's top point (the shrink's,
        // centred), louder than the calls' own (`docs/audio-format.md`).
        let top = heroes.iter().find(|p| p.slot == slot).map(|p| Vec3::from(p.mover.position) + Vec3::Y * HERO_TOP);
        let lapse = |name: &str| match top {
            Some(at) => PlaySoundAt::panned(name, at, LAPSE_VOLUME),
            None => PlaySoundAt::centred(name, LAPSE_VOLUME),
        };
        if ended & power::LEVITATE != 0 {
            sound.write(lapse("S_LEVITATEDOWN"));
        }
        if ended & power::GROW != 0 && state.level < BIG_LEVEL {
            sound.write(lapse("S_UNGROW"));
        }
        if ended & power::POJO != 0 {
            sound.write(lapse("S_UNPOJO"));
        }
        if !state.alive || state.health > 200.0 {
            continue;
        }
        state.warning_timer -= FIELDS_PER_TICK;
        if state.warning_timer < 1 {
            let invulnerable = state.bits.armour & (crate::damage::resists::INVULNERABLE | crate::damage::resists::GOLD) != 0;
            if !in_tower && !invulnerable {
                sound.write(PlaySoundAt::centred("S_WARN", warning_volume(state.health)));
            }
            state.warning_timer = match state.health {
                h if h < 25.0 => 30,
                h if h < 100.0 => 60,
                _ => 120,
            };
        }
    }
    let scale = EnemyScale(if !boss_level && shrink { SHRINK_SCALE } else { 1.0 });
    if scale.0 > enemies.0 {
        sound.write(PlaySoundAt::centred("S_UNSHRINK", LAPSE_VOLUME));
    }
    if *enemies != scale {
        *enemies = scale;
    }
    let stopped = TimeStop(time_stop);
    if *stop != stopped {
        *stop = stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAR: Hero = Hero { class: "WAR", pojo: false, invulnerable: false };

    fn hurt(amount: f32, kind: u32, cry: Cry) -> HurtHero {
        HurtHero { slot: 0, amount, kind, cry }
    }

    fn sound(name: &str, volume: u8) -> Voiced {
        Voiced::Sound(name.into(), volume)
    }

    /// A blow's cries, the field count even.
    fn voice(c: &mut Cries, hero: Hero, before: f32, after: f32, hit: HurtHero) -> Vec<Voiced> {
        c.voice(hero, before, after, hit, true)
    }

    #[test]
    fn a_hurt_hero_cries_at_most_every_half_second() {
        let mut c = Cries::default();
        assert_eq!(voice(&mut c, WAR, 500.0, 490.0, hurt(10.0, 0, Cry::Hurt)), [sound("S_PLYRDMG", 0x7F)]);
        // Its wait: nothing until 30 fields have passed.
        assert_eq!(voice(&mut c, WAR, 490.0, 480.0, hurt(10.0, 0, Cry::Hurt)), []);
        c.wait -= HURT_WAIT;
        assert_eq!(voice(&mut c, WAR, 480.0, 475.0, hurt(5.0, 0x2_0000, Cry::Hurt)), [sound("S_PLYRDMG2", 0x7F)]);
        (c.wait, c.lost) = (0, 0.0);
        assert_eq!(voice(&mut c, WAR, 475.0, 470.0, hurt(5.0, 0x4_0000, Cry::Hurt)), [sound("S_PLYRDMG3", 0x7F)]);
        // Silent blows say nothing; poison says the class's line.
        c.wait = 0;
        assert_eq!(voice(&mut c, WAR, 470.0, 460.0, hurt(10.0, 0, Cry::Silent)), []);
        c.lost = 0.0;
        assert_eq!(voice(&mut c, WAR, 460.0, 455.0, hurt(5.0, POISON_KIND, Cry::Hurt)), [Voiced::Line("S_WARPOISON".into(), 0xC0)]);
    }

    #[test]
    fn big_blows_and_lost_health_bring_pain_lines() {
        let mut c = Cries::default();
        // 61 at once: a pain line in place of the hurt sound.
        let v = voice(&mut c, WAR, 400.0, 339.0, hurt(61.0, 0, Cry::Hurt));
        assert!(matches!(&v[..], [Voiced::Line(l, 0xE0)] if l.starts_with("S_WARPAIN")), "{v:?}");
        // Every 30 lost: a pain line, the rest counted on.
        c.wait = 0;
        assert_eq!(voice(&mut c, WAR, 339.0, 319.0, hurt(20.0, 0, Cry::Hurt)), [sound("S_PLYRDMG", 0x7F)]);
        c.wait = 0;
        let v = voice(&mut c, WAR, 319.0, 304.0, hurt(15.0, 0, Cry::Hurt));
        assert!(matches!(&v[..], [Voiced::Line(..)]), "{v:?}");
        assert_eq!(c.lost, 5.0);
        // Crossing 150 health: the announcer's warning in place of the
        // pain line; the hurt sound still plays.
        c.wait = 0;
        assert_eq!(
            voice(&mut c, WAR, 151.0, 80.0, hurt(71.0, 0, Cry::Hurt)),
            [Voiced::Warning("S_BADLY", 1.0), sound("S_PLYRDMG", 0x7F)]
        );
        // Past 50: by the field count's parity.
        c.wait = 0;
        assert_eq!(c.voice(WAR, 60.0, 50.0, hurt(10.0, 0, Cry::Pain), true), [Voiced::Warning("S_LIFEFORCE", 1.0)]);
        assert_eq!(c.voice(WAR, 51.0, 40.0, hurt(11.0, 0, Cry::Pain), false), [Voiced::Warning("S_ABOUT", 0.5)]);
        // Both in one blow: the first.
        assert_eq!(c.voice(WAR, 200.0, 10.0, hurt(190.0, 0, Cry::Silent), true), [Voiced::Warning("S_BADLY", 1.0)]);
    }

    #[test]
    fn tiles_scream_or_groan() {
        let mut c = Cries::default();
        assert_eq!(voice(&mut c, WAR, 500.0, 490.0, hurt(10.0, 0x80, Cry::Scream)), [sound("S_WARDIE1", 0xE0)]);
        let pojo = Hero { pojo: true, ..WAR };
        assert_eq!(voice(&mut c, pojo, 490.0, 480.0, hurt(10.0, 0x80, Cry::Scream)), [sound("S_POJOPAIN", 0xE0)]);
        let tough = Hero { invulnerable: true, ..WAR };
        assert_eq!(voice(&mut c, tough, 480.0, 480.0, hurt(0.0, 0x80, Cry::Scream)), []);
        let v = voice(&mut c, WAR, 480.0, 470.0, hurt(10.0, 0x80, Cry::Pain));
        assert!(matches!(&v[..], [Voiced::Line(l, 0xE0)] if l.starts_with("S_WARPAIN")), "{v:?}");
        // The secret characters' cries aren't in the game's tables; their
        // hurt sounds are.
        let minotaur = Hero { class: "MIN", ..WAR };
        assert_eq!(voice(&mut c, minotaur, 470.0, 460.0, hurt(10.0, 0x80, Cry::Scream)), []);
        c.wait = 0;
        assert_eq!(voice(&mut c, minotaur, 460.0, 450.0, hurt(10.0, 0, Cry::Hurt)), [sound("S_PLYRDMG", 0x7F)]);
    }

    #[test]
    fn the_death_cries() {
        assert_eq!(death_cries(WAR), [sound("S_PLAYERDIES", 0x7F), sound("S_WARDIE2", 0xE0)]);
        assert_eq!(death_cries(Hero { pojo: true, ..WAR })[1], sound("S_POJOPOISON", 0xE0));
        assert_eq!(death_cries(Hero { class: "MIN", ..WAR }), [sound("S_PLAYERDIES", 0x7F)]);
    }

    #[test]
    fn the_warning_grows_louder() {
        assert_eq!(warning_volume(200.0), 0x7F);
        assert_eq!(warning_volume(100.0), 0x7F);
        assert_eq!(warning_volume(99.0), 0x98);
        assert_eq!(warning_volume(25.0), 0x98);
        assert_eq!(warning_volume(24.0), 0xB1);
        assert_eq!(warning_volume(10.5), 0xB1);
        assert_eq!(warning_volume(10.0), 0xCA);
        assert_eq!(warning_volume(1.0), 0xCA);
    }

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
        assert_eq!(s.health, START_HEALTH + 200.0, "each rise heals 100");
    }

    #[test]
    fn deaths_drain_takes_levels_back() {
        let mut s = PlayerState::default();
        s.add_experience(2180);
        assert_eq!((s.level, s.drain_step()), (3, 11));
        // 2180 reaches level 3; 1060 level 2.
        assert_eq!(s.lose_experience(11), 1);
        assert_eq!(s.level, 2);
        assert_eq!(s.lose_experience(5000), 1);
        assert_eq!((s.level, s.experience), (1, 0));
        s.level = MAX_LEVEL;
        assert_eq!(s.lose_experience(100), 0, "nothing at 99");
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
        all_on(&mut s);
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

    /// Turns every power carried on, as the power menu would one by one.
    fn all_on(s: &mut PlayerState) {
        for i in 0..POWER_SLOTS {
            if s.powers[i].live() && s.powers[i].state != SlotState::On {
                s.toggle_power(i);
            }
        }
    }

    #[test]
    fn powers_are_held_until_turned_on_and_put_away_with_their_time() {
        let mut s = PlayerState::default();
        let i = s.grant_power(power::SPECIAL, power::LEVITATE, 0.0, 30.0).unwrap();
        // Held: no effect, no time spent.
        assert_eq!(s.powers[i].state, SlotState::Held);
        assert_eq!(s.tick_powers(5.0).special & power::LEVITATE, 0);
        assert_eq!(s.powers[i].time, 30.0);
        // On: it works and runs down.
        s.toggle_power(i);
        assert_eq!(s.tick_powers(5.0).special & power::LEVITATE, power::LEVITATE);
        assert_eq!(s.powers[i].time, 25.0);
        // Off: saved for later with what's left.
        s.toggle_power(i);
        assert_eq!(s.powers[i].state, SlotState::Off);
        assert_eq!(s.tick_powers(10.0).special & power::LEVITATE, 0);
        assert_eq!(s.powers[i].time, 25.0);
        // Another of the same tops it up and keeps it off.
        s.grant_power(power::SPECIAL, power::LEVITATE, 0.0, 30.0);
        assert_eq!((s.powers[i].time, s.powers[i].state), (40.0, SlotState::Off));
        // On again until it runs out, when the slot empties.
        s.toggle_power(i);
        s.tick_powers(41.0);
        assert!(!s.powers[i].live() && s.powers[i].state == SlotState::Empty);
    }

    #[test]
    fn counted_powers_are_spent_one_use_at_a_time() {
        let mut s = PlayerState::default();
        s.grant_power(power::SPECIAL, 0x10, 2.0, -1.0);
        all_on(&mut s);
        s.spend_power(power::SPECIAL, 0x70);
        assert_eq!(s.tick_powers(0.1).special & 0x10, 0x10, "one use left");
        s.spend_power(power::SPECIAL, 0x70);
        assert_eq!(s.tick_powers(0.1).special & 0x10, 0, "used up");
        assert!(s.powers.iter().all(|p| !p.live()));
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
        all_on(&mut s);
        s.tick_powers(200.0);
        assert_eq!(s.powers.iter().filter(|p| p.live()).count(), 1, "timed power ran out, counted one stays");
        for v in 0..20 {
            s.grant_power(9, 1 << v, 0.0, v as f32 + 1.0);
        }
        assert_eq!(s.powers.iter().filter(|p| p.live()).count(), POWER_SLOTS);
    }
}
