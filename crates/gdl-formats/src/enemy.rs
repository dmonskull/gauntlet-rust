//! Regular monsters — what the game calls enemies: grunts, demons, ghosts,
//! the things generators make. Unlike the bosses (`critter.rs`), their stats
//! aren't in any data file: they're per-type tables compiled into `main.dol`,
//! indexed by the enemy type id of [`crate::population::ENEMY_CODES`]. The
//! values below are copied from those tables; `docs/monsters.md` has where
//! each one lives and which code reads it.
//!
//! Times in the game's monster code count video fields (60 per second): the
//! game runs at 30 frames a second, so every frame is two fields and each
//! per-field rate here moves twice per 30 Hz tick ([`FIELDS_PER_TICK`]).

use crate::population::ENEMY_CODES;

/// Video fields per 30 Hz game frame.
pub const FIELDS_PER_TICK: f32 = 2.0;

/// Enemy type ids below this are regular monsters with tiers (the tier
/// scales hit points and damage); from here up they're special types.
pub const FIRST_SPECIAL_TYPE: i32 = 28;

/// Per-type stats of a regular monster.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyStats {
    pub id: i32,
    /// Twice the floor step: the monster's step is half of this.
    pub step_base: f32,
    /// Collision radius (walls, players, other monsters).
    pub radius: f32,
    /// Copied onto the monster when it's created; not read anywhere we've
    /// traced yet.
    pub unknown: f32,
    /// Height of the monster's centre above its feet: generators test the
    /// spawn spot from here.
    pub center_height: f32,
    /// Ground covered per video field at walking pace, before the level's
    /// speed scale.
    pub speed: f32,
    /// Damage of one hit at full tier, before the level's damage scale.
    pub damage: f32,
    /// Hit points at full tier, before the level's hit point scale.
    pub hit_points: f32,
    /// The AI a monster gets when its generator or placement doesn't choose
    /// one (7: chase the nearest player; 2: the scorpion/rat/mage variant).
    pub default_ai: i16,
    /// Turn rate, radians per video field (running turns 3× faster).
    pub turn: f32,
}

#[allow(clippy::too_many_arguments)] // one table row
const fn t(
    id: i32,
    step_base: f32,
    radius: f32,
    unknown: f32,
    center_height: f32,
    speed: f32,
    damage: f32,
    hit_points: f32,
    default_ai: i16,
) -> EnemyStats {
    EnemyStats {
        id,
        step_base,
        radius,
        unknown,
        center_height,
        speed,
        damage,
        hit_points,
        default_ai,
        turn: std::f32::consts::PI / 64.0,
    }
}

/// The game's per-type table (ids 0–33; id 28 is unused, as in the game's
/// monster name table). Columns: step × 2, radius, unknown, centre height,
/// speed per field, damage, hit points, default AI. Every type turns π/64
/// per field.
#[rustfmt::skip]
pub const ENEMY_TYPES: [Option<EnemyStats>; 34] = [
    Some(t(0, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 2)),   // sco
    Some(t(1, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),   // tro
    Some(t(2, 6.0, 1.8, 3.8, 3.0, 0.12, 18.0, 46.0, 7)),  // dem
    Some(t(3, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 2)),   // rat
    Some(t(4, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),   // gru
    Some(t(5, 6.0, 1.8, 3.8, 3.0, 0.1, 18.0, 46.0, 7)),   // kni
    Some(t(6, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 7)),   // sna
    Some(t(7, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),   // sor
    Some(t(8, 6.0, 1.8, 3.8, 3.0, 0.1, 18.0, 46.0, 7)),   // mum
    Some(t(9, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 7)),   // spi
    Some(t(10, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),  // liz
    Some(t(11, 6.0, 1.8, 3.8, 3.0, 0.1, 18.0, 46.0, 7)),  // tre
    Some(t(12, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 2)),  // mag
    Some(t(13, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),  // zom
    Some(t(14, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),  // pla
    Some(t(15, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 7)),  // wol
    Some(t(16, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),  // ice
    Some(t(17, 6.0, 1.8, 3.8, 3.0, 0.1, 18.0, 46.0, 7)),  // wrm
    Some(t(18, 3.0, 1.5, 2.0, 1.5, 0.1, 12.0, 21.0, 7)),  // dog
    Some(t(19, 6.0, 1.5, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),  // ske
    Some(t(20, 6.0, 1.8, 3.8, 3.0, 0.1, 18.0, 46.0, 7)),  // gho
    Some(t(21, 3.0, 0.75, 0.5, 1.5, 0.02, 12.0, 21.0, 7)), // aci
    Some(t(22, 3.0, 0.75, 0.5, 1.5, 0.05, 12.0, 21.0, 7)), // han
    Some(t(23, 6.0, 2.0, 3.8, 3.0, 0.1, 15.0, 30.0, 7)),  // imp
    Some(t(24, 6.0, 2.0, 3.8, 3.0, 0.1, 15.0, 46.0, 7)),  // war
    Some(t(25, 6.0, 2.0, 3.8, 3.0, 0.1, 15.0, 46.0, 7)),  // sky
    Some(t(26, 6.0, 2.0, 3.8, 3.0, 0.1, 15.0, 46.0, 7)),  // wind
    Some(t(27, 10.0, 4.0, 3.8, 4.0, 0.1, 20.0, 100.0, 7)), // grm
    None,
    Some(t(29, 12.0, 3.0, 5.0, 4.0, 0.09, 20.0, 200.0, 19)), // golem
    Some(t(30, 6.0, 1.5, 3.0, 3.0, 0.125, 1.0, 100.0, 3)),  // death
    Some(t(31, 5.0, 1.5, 3.0, 3.0, 0.1, 0.0, 9999.0, 27)),  // it
    Some(t(32, 10.0, 6.0, 5.0, 3.0, 0.1, 30.0, 500.0, 7)),  // gar
    Some(t(33, 6.0, 2.0, 4.0, 3.0, 0.09, 20.0, 200.0, 7)),  // general
];

/// The stats for an enemy type id.
pub fn enemy_stats(id: i32) -> Option<&'static EnemyStats> {
    ENEMY_TYPES.get(usize::try_from(id).ok()?)?.as_ref()
}

/// The game's name for an enemy type (`gru`), from the monster name table.
pub fn enemy_name(id: i32) -> Option<&'static str> {
    ENEMY_CODES.iter().find(|e| e.0 == id).map(|e| e.1)
}

/// The tier the game's stat formulas use: 0 counts as 1, the special
/// variants (4–7) as 2, and AI 18 always as 1.
pub fn stat_tier(tier: i32, ai: i16) -> f32 {
    if ai == 18 {
        return 1.0;
    }
    match tier {
        t if t > 3 => 2.0,
        t => t as f32,
    }
}

/// Level-dependent scales from the realm's level record
/// ([`crate::world_data::LevelTuning`]) that monster stats use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyScales {
    pub hit_points: f32,
    pub speed: f32,
    pub awareness: f32,
    pub damage: f32,
}

impl Default for EnemyScales {
    fn default() -> Self {
        Self { hit_points: 1.0, speed: 1.0, awareness: 1.0, damage: 1.0 }
    }
}

/// A monster's working numbers at creation, as the game's enemy setup
/// derives them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyInstance {
    pub hit_points: f32,
    /// 1–3: which third of the full hit points it has (drives its damage).
    pub strength: u8,
    pub damage: f32,
    /// Players further away than this (units) aren't noticed.
    pub awareness: f32,
    /// Ground covered per 30 Hz tick at walking pace.
    pub speed_per_tick: f32,
    /// Most the body turns per 30 Hz tick, walking.
    pub turn_per_tick: f32,
    pub radius: f32,
    pub step: f32,
}

/// Awareness range before the level's scale.
pub const AWARENESS: f32 = 30.0;

/// Experience a hero earns for a blow that lands on a monster of each type
/// (by enemy id), and for the blow that kills it.
pub const HIT_EXPERIENCE: [u32; 34] =
    [1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 2, 1, 2, 3, 1, 2, 3, 1, 1, 2, 3, 3, 3, 1, 0, 2, 1, 1, 2, 2];
pub const KILL_EXPERIENCE: [u32; 34] =
    [1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 2, 1, 2, 3, 1, 2, 3, 1, 1, 2, 3, 3, 3, 15, 0, 30, 1, 2, 300, 30];

/// Experience for a blow on monster type `enemy` (killing or not), on a
/// level with the given experience level and scale, for a hero of
/// `hero_level`: the table value × the scale, divided down by 1 + 0.1 ×
/// the levels the hero is above the level's.
pub fn experience(enemy: i32, killed: bool, hero_level: u32, level: f32, scale: f32) -> u32 {
    scaled_experience(enemy, killed, 1, hero_level, level, scale)
}

/// Experience for a blow on a generator of monster type `enemy`: five
/// times a blow on one of its monsters, before the level's scaling.
pub fn generator_experience(enemy: i32, destroyed: bool, hero_level: u32, level: f32, scale: f32) -> u32 {
    scaled_experience(enemy, destroyed, 5, hero_level, level, scale)
}

fn scaled_experience(enemy: i32, killed: bool, times: u32, hero_level: u32, level: f32, scale: f32) -> u32 {
    let table = if killed { &KILL_EXPERIENCE } else { &HIT_EXPERIENCE };
    let base = (table.get(enemy.max(0) as usize).copied().unwrap_or(0) * times) as f32;
    let mut scale = scale;
    if level > 0.0 && hero_level as f32 > level {
        scale *= 1.0 / (0.1 * (hero_level as f32 - level) + 1.0);
    }
    (base * scale) as u32
}

/// Whole hit points a blow of `damage` takes off an item (a generator or
/// a breakable) with `armor`, from a hero of `hero_level` on a level whose
/// experience level is `level`: heroes above it hit harder (+10% a level),
/// heroes below softer (-1% a level); every blow does at least 1.
pub fn item_damage(damage: f32, armor: i8, hero_level: u32, level: f32) -> i32 {
    let mut damage = damage;
    if level > 0.0 {
        let above = hero_level as f32 - level;
        if above > 0.0 {
            damage *= 0.1 * above + 1.0;
        } else if above < 0.0 {
            damage *= 1.0 + 0.01 * above;
        }
        if damage < 1.0 {
            damage = 1.0;
        }
    }
    if armor >= 0 {
        damage -= f32::from(armor);
        if damage <= 0.0 {
            damage = 1.0;
        }
    }
    // Rounded half away from zero.
    (damage + if damage < 0.0 { -0.5 } else { 0.5 }) as i32
}

impl EnemyStats {
    /// Floor step: how far up the floor probe starts and (+ 5) how far
    /// down it searches.
    pub fn step(&self) -> f32 {
        0.5 * self.step_base
    }

    /// Full-tier hit points on this level.
    pub fn base_hit_points(&self, scales: &EnemyScales) -> f32 {
        self.hit_points * scales.hit_points
    }

    /// Hit points of a monster of `tier` (regular types: a third per tier;
    /// special types always the full amount).
    pub fn hit_points_at(&self, tier: i32, ai: i16, scales: &EnemyScales) -> f32 {
        let hp = self.base_hit_points(scales);
        if self.id < FIRST_SPECIAL_TYPE { 0.333 * hp * stat_tier(tier, ai) } else { hp }
    }

    /// Everything the game derives when it creates a monster.
    pub fn instance(&self, tier: i32, ai: i16, scales: &EnemyScales) -> EnemyInstance {
        let hit_points = self.hit_points_at(tier, ai, scales);
        let full = self.base_hit_points(scales);
        let strength = strength_of(hit_points, full);
        // Damage scales with the same thirds, except for Death.
        let mut damage = self.damage * scales.damage;
        if hit_points <= 0.667 * full && self.id != 30 {
            damage *= if hit_points <= 0.333 * full { 0.333 } else { 0.667 };
        }
        EnemyInstance {
            hit_points,
            strength,
            damage,
            awareness: AWARENESS * scales.awareness,
            speed_per_tick: self.speed * scales.speed * FIELDS_PER_TICK,
            turn_per_tick: self.turn * FIELDS_PER_TICK,
            radius: self.radius,
            step: self.step(),
        }
    }
}

/// Which third of `full` hit points a monster has: 3 above two thirds, 2
/// above one third, 1 above zero, else 0.
pub fn strength_of(hit_points: f32, full: f32) -> u8 {
    if hit_points > 0.667 * full {
        3
    } else if hit_points > 0.333 * full {
        2
    } else if hit_points > 0.0 {
        1
    } else {
        0
    }
}

/// The atree (and model prefix) for an enemy at `tier`: the type's name and
/// the tier number (`GRU2`), 0 counting as 1; tiers 4–7 are the special
/// variants lettered A, B, S, F (`GRUA`).
pub fn atree_name(id: i32, tier: i32) -> Option<String> {
    let name = enemy_name(id)?.to_ascii_uppercase();
    Some(match tier {
        4..=7 => format!("{name}{}", ['A', 'B', 'S', 'F'][(tier - 4) as usize]),
        0 => format!("{name}1"),
        t => format!("{name}{t}"),
    })
}

/// The sounds a monster type makes when a blow lands on it and when it
/// dies, as catalog names (`docs/monsters.md`, "Hit and death sounds"):
/// close versions for melee, far ones for thrown blows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterSounds {
    /// Dying, close then far: strength 1, and stronger.
    pub die: [[String; 2]; 2],
    /// Hit, close then far: strength 1; stronger ones on their first
    /// blow, then later ones.
    pub hit: [[String; 3]; 2],
}

impl MonsterSounds {
    /// The death sound for a monster of this strength.
    pub fn die(&self, strength: i16, far: bool) -> &str {
        &self.die[usize::from(far)][usize::from(strength >= 2)]
    }

    /// The hit sound for a monster of this strength that has now taken
    /// `hits` blows (counting this one).
    pub fn hit(&self, strength: i16, hits: i16, far: bool) -> &str {
        let set = &self.hit[usize::from(far)];
        match (strength < 2, hits < 2) {
            (true, _) => &set[0],
            (false, true) => &set[1],
            (false, false) => &set[2],
        }
    }
}

/// Enemy types whose sounds come in a weak (`<name>1`) and a strong
/// (`<name>2`) set, the strong one with two hit sounds.
const PAIRED_SOUND_TYPES: [i32; 18] = [1, 2, 4, 5, 7, 8, 10, 11, 13, 14, 16, 17, 19, 20, 23, 24, 25, 26];
const BGRUNT_TYPE: i32 = 0x1B;
const GOLEM_TYPE: i32 = 0x1D;

/// Builds a realm enemy's sound names from its `ENMY` name (`GRUNT`), its
/// subtype, the level's boss (`LevelTuning::boss_enemy`) and the
/// realm's letter (`A` for `levelA1`). `None` for critters and bosses
/// (subtypes 5 and 9), which have their own sounds, and unnamed records.
pub fn monster_sounds(enemy: i32, subtype: i32, name: &str, boss: i32, realm: char) -> Option<MonsterSounds> {
    if subtype == 5 || subtype == 9 || name.is_empty() {
        return None;
    }
    // The two name prefixes, and whether the strong set has two hit sounds
    // (0: one set, 1: weak + strong, 2: strong only).
    let (weak, strong, sets) = if PAIRED_SOUND_TYPES.contains(&enemy) {
        if subtype < 10 && boss < 0 {
            (format!("{name}1"), format!("{name}2"), 1)
        } else {
            (format!("{name}2"), format!("{name}2"), 2)
        }
    } else if enemy == BGRUNT_TYPE {
        (format!("{name}1"), format!("{name}1"), 0)
    } else if enemy == GOLEM_TYPE {
        let letter = if realm == 'T' { 'G' } else { realm };
        (format!("GOL{letter}"), format!("GOL{letter}"), 0)
    } else {
        (name.to_string(), name.to_string(), 0)
    };
    let die = if enemy == GOLEM_TYPE { "KILL" } else { "DIE" };
    // Some boss levels use their own recordings: the name cut to 14
    // characters and the level's letter added.
    let variant = match boss {
        0x24 => Some('C'),
        0x25 => Some('D'),
        0x29 => Some('B'),
        _ => None,
    };
    let finish = |mut s: String| {
        if let Some(v) = variant {
            s.truncate(14);
            s.push(v);
        }
        // Catalog names hold 15 characters.
        s.truncate(15);
        s
    };
    let set = |range: &str| {
        let strong_hits = if sets == 0 {
            [format!("S_{strong}HIT{range}"), format!("S_{strong}HIT{range}")]
        } else {
            [format!("S_{strong}HIT1{range}"), format!("S_{strong}HIT2{range}")]
        };
        let [first, later] = strong_hits.map(finish);
        let weak_hit = if sets < 2 { finish(format!("S_{weak}HIT{range}")) } else { first.clone() };
        (
            [finish(format!("S_{weak}{die}{range}")), finish(format!("S_{strong}{die}{range}"))],
            [weak_hit, first, later],
        )
    };
    let (close_die, close_hit) = set("CLOSE");
    let (far_die, far_hit) = set("FAR");
    Some(MonsterSounds { die: [close_die, far_die], hit: [close_hit, far_hit] })
}

/// A generator's sounds in realm `realm` (the level folder's letter) for
/// one making `enemy`: when a blow lands, and when it's destroyed. Only
/// the eleven monster realms have them; the jungle's demon generators use
/// their own.
pub fn generator_sounds(realm: char, enemy: i32) -> Option<(String, String)> {
    if !('A'..='K').contains(&realm) {
        return None;
    }
    if realm == 'J' && enemy == 0x18 {
        return Some(("S_GENDAMWAR".into(), "S_GENKILLWAR".into()));
    }
    Some((format!("S_GENDAM{realm}"), format!("S_GENKILL{realm}")))
}

/// Which enemy types a level loads, from its realm's `ENMY` records
/// (`world_data::RealmEnemy`): each is a (type, subtype) pair, and the
/// subtypes 1–5 are slots — 1 small, 2 main, 3 elite, 4 special variants,
/// 5 critter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelEnemies {
    pub loaded: Vec<(i32, i32)>,
}

impl LevelEnemies {
    /// The type filling a slot, if any.
    pub fn slot(&self, subtype: i32) -> Option<i32> {
        self.loaded.iter().rev().find(|(_, s)| *s == subtype).map(|(t, _)| *t)
    }

    /// Level data names only a few placeholder monsters; the game swaps in
    /// the realm's own when it builds a generator or placed monster: `rat`
    /// becomes the small slot's type; `gru` and `kni` become the special
    /// variants' type for tiers 4 and up, else the main slot's, else the
    /// elite slot's. Everything else stays itself.
    pub fn substitute(&self, enemy: i32, tier: i32) -> i32 {
        const RAT: i32 = 3;
        const GRU: i32 = 4;
        const KNI: i32 = 5;
        let base = if enemy == RAT { self.slot(1).unwrap_or(-1) } else { enemy };
        if enemy != GRU && enemy != KNI {
            return base;
        }
        if tier >= 4
            && let Some(t) = self.slot(4)
        {
            return t;
        }
        self.slot(2).or(self.slot(3)).unwrap_or(base)
    }

    /// Whether the level loads `enemy` at all (the game refuses to make a
    /// monster whose type it hasn't loaded).
    pub fn has(&self, enemy: i32) -> bool {
        self.loaded.iter().any(|(t, _)| *t == enemy)
    }

    /// The `MONSTERS/` folders the level loads for `enemy`: `<name>aux` for
    /// the special variants' subtype, `<name><n>` for subtypes 11+, else
    /// `<name>`.
    pub fn folders(&self, enemy: i32) -> Vec<String> {
        let Some(name) = enemy_name(enemy) else { return Vec::new() };
        let name = name.to_ascii_uppercase();
        self.loaded
            .iter()
            .filter(|(t, _)| *t == enemy)
            .map(|&(_, s)| match s {
                4 => format!("{name}AUX"),
                s if s >= 11 => format!("{name}{}", s - 10),
                _ => name.clone(),
            })
            .collect()
    }
}

/// Generator settings the placement leaves at 0, by strength (1–3):
/// the most monsters alive at once, and the spawn rate.
pub fn generator_defaults(strength: i16) -> (i16, i16) {
    let i = (strength - 1).clamp(0, 2) as usize;
    ([10, 5, 2][i], [5, 10, 15][i])
}

/// Video fields a generator waits after making a monster: 6 × rate, grown
/// by up to double as `ramp` (0–1) climbs.
pub fn generator_wait(rate: u8, ramp: f32) -> f32 {
    6.0 * rate as f32 * (1.0 + ramp)
}

/// How far `ramp` climbs per monster made: it wraps to 0 after passing 1,
/// so the wait grows over each run of 2 × max monsters.
pub fn generator_ramp_step(max: u8) -> f32 {
    1.0 / (2.0 * max.max(1) as f32)
}

/// The monster actions the game plays, by the index its code uses
/// (`READY` 0 … `DEATH` 0x20).
pub const ACTION_NAMES: [&str; 33] = [
    "READY",
    "START",
    "TAUNT",
    "WALK",
    "RUN",
    "FLY",
    "HOVER",
    "LANDING",
    "WALKTOREADY",
    "READYTOWALK",
    "WALKTOREADY",
    "READYTOWALK",
    "ATTACK1",
    "ATTACK1R",
    "ATTACK2",
    "ATTACK2R",
    "ATTACK3",
    "ATTACK3R",
    "ATTACK4",
    "ATTACK4R",
    "ATTACK5",
    "ATTACK5R",
    "RUNATTACK1",
    "RUNATTACK2",
    "THROW1",
    "THROW2",
    "THROWF",
    "ATTTOREADY",
    "HIT1",
    "HIT2",
    "HIT3",
    "GETUP",
    "DEATH",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lines_up_with_the_name_table() {
        for (i, s) in ENEMY_TYPES.iter().enumerate() {
            match s {
                Some(s) => {
                    assert_eq!(s.id, i as i32);
                    assert!(enemy_name(s.id).is_some(), "type {i} has no name");
                    assert!(s.radius > 0.0 && s.step() > 0.0 && s.speed > 0.0 && s.hit_points > 0.0);
                }
                None => assert!(enemy_name(i as i32).is_none()),
            }
        }
    }

    #[test]
    fn tiers_are_thirds_of_the_full_monster() {
        let gru = enemy_stats(4).unwrap();
        let s = EnemyScales::default();
        let [a, b, c] = [1, 2, 3].map(|t| gru.instance(t, 7, &s));
        assert!((a.hit_points - 9.99).abs() < 1e-3 && (c.hit_points - 29.97).abs() < 1e-3);
        assert_eq!([a.strength, b.strength, c.strength], [1, 2, 3]);
        assert!((a.damage - 15.0 * 0.333).abs() < 1e-4);
        assert!((b.damage - 15.0 * 0.667).abs() < 1e-4);
        assert_eq!(c.damage, 15.0);
        // Special variants count as tier 2, AI 18 as tier 1.
        assert_eq!(gru.instance(5, 7, &s).strength, 2);
        assert_eq!(gru.instance(3, 18, &s).strength, 1);
        // 0.1 per field, two fields a tick.
        assert!((a.speed_per_tick - 0.2).abs() < 1e-6);
        assert_eq!(a.step, 3.0);
    }

    #[test]
    fn placeholders_become_the_realms_monsters() {
        // Castle A1: main grunt, small rat, critters.
        let castle = LevelEnemies { loaded: vec![(4, 2), (3, 1), (29, 5), (33, 5), (32, 5)] };
        assert_eq!(castle.substitute(3, 0), 3);
        assert_eq!(castle.substitute(5, 0), 4);
        // Mountain B4: demons elite, trolls special, scorpions small.
        let mount = LevelEnemies { loaded: vec![(2, 3), (0, 1), (1, 4), (29, 5)] };
        assert_eq!(mount.substitute(4, 1), 2);
        assert_eq!(mount.substitute(4, 5), 1);
        assert_eq!(mount.substitute(3, 0), 0);
        assert_eq!(mount.folders(1), ["TROAUX"]);
        let hell = LevelEnemies { loaded: vec![(24, 13)] };
        assert_eq!(hell.folders(24), ["WAR3"]);
        assert_eq!(hell.substitute(3, 0), -1);
        assert!(!hell.has(4));
    }

    /// Every generator and placed monster in every level, run through its
    /// realm's enemy list: whatever resolves to a regular monster the level
    /// loads finds its tier's atree in the folders it loads.
    #[test]
    fn every_real_level_monster_has_a_model() {
        use crate::anim::AnimFile;
        use crate::population::{ItemClass, PlacementParams, Population};
        use crate::world_data::WorldData;
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let root = std::path::Path::new(&root);
        let Ok(realms) = std::fs::read_dir(root.join("WDATA")) else {
            eprintln!("skipping: no WDATA folder");
            return;
        };
        let mut atrees: std::collections::HashMap<String, Vec<String>> = Default::default();
        let mut atrees_in = |folder: &str| -> Vec<String> {
            atrees
                .entry(folder.to_string())
                .or_insert_with(|| {
                    std::fs::read(root.join("MONSTERS").join(folder).join("ANIM.PS2"))
                        .ok()
                        .and_then(|b| AnimFile::parse(&b).ok())
                        .map_or_else(Vec::new, |a| a.atrees.into_iter().map(|t| t.name).collect())
                })
                .clone()
        };
        let (mut made, mut refused) = (0, 0);
        for realm in realms.flatten().map(|e| e.path()) {
            let world = WorldData::parse(&std::fs::read(&realm).unwrap()).unwrap();
            for level in &world.levels {
                let Ok(file) = std::fs::read(root.join("LEVELS").join(level.folder()).join("WORLDS.PS2")) else {
                    continue;
                };
                let enemies =
                    LevelEnemies { loaded: world.level_enemies(level).iter().map(|e| (e.enemy, e.subtype)).collect() };
                let pop = Population::parse(&file).unwrap();
                for p in &pop.placements {
                    let ty = pop.resolved_type(p);
                    let Some(named) = ty.enemy() else { continue };
                    let (id, tier) = match p.params(ty.class) {
                        PlacementParams::Generator { strength, .. } if ty.class == ItemClass::Generator => {
                            (enemies.substitute(named, 0), strength.max(1) as i32)
                        }
                        PlacementParams::Enemy { level: l, .. } => (enemies.substitute(named, l as i32), l as i32),
                        _ => continue,
                    };
                    if !enemies.has(id) || enemy_stats(id).is_none_or(|s| s.id >= FIRST_SPECIAL_TYPE) {
                        refused += 1;
                        continue;
                    }
                    let want = atree_name(id, tier).unwrap();
                    let found = enemies.folders(id).iter().any(|f| atrees_in(f).contains(&want));
                    assert!(
                        found,
                        "{} placement at {:?}: {want} not in {:?}",
                        level.name,
                        p.position,
                        enemies.folders(id)
                    );
                    made += 1;
                }
            }
        }
        eprintln!("{made} generators/monsters resolve to a model, {refused} refused");
        assert!(made > 1000);
    }

    #[test]
    fn blows_on_items() {
        assert_eq!(item_damage(10.0, 2, 1, 0.0), 8);
        assert_eq!(item_damage(10.0, 2, 13, 3.0), 18);
        assert_eq!(item_damage(10.0, -1, 1, 11.0), 9);
        assert_eq!(item_damage(1.0, 5, 1, 0.0), 1);
        assert_eq!(generator_experience(4, false, 1, 0.0, 1.0), 10);
        assert_eq!(generator_experience(4, true, 1, 0.0, 2.0), 20);
        assert_eq!(generator_sounds('B', 1), Some(("S_GENDAMB".into(), "S_GENKILLB".into())));
        assert_eq!(generator_sounds('L', 1), None);
    }

    #[test]
    fn sound_names() {
        let grunt = monster_sounds(4, 2, "GRUNT", -1, 'A').unwrap();
        assert_eq!(grunt.hit(1, 1, false), "S_GRUNT1HITCLOS");
        assert_eq!(grunt.hit(3, 1, false), "S_GRUNT2HIT1CLO");
        assert_eq!(grunt.hit(3, 2, false), "S_GRUNT2HIT2CLO");
        assert_eq!(grunt.hit(3, 2, true), "S_GRUNT2HIT2FAR");
        assert_eq!(grunt.die(1, false), "S_GRUNT1DIECLOS");
        assert_eq!(grunt.die(2, false), "S_GRUNT2DIECLOS");
        assert_eq!(grunt.die(2, true), "S_GRUNT2DIEFAR");
        let rat = monster_sounds(3, 1, "RAT", -1, 'A').unwrap();
        assert_eq!((rat.hit(2, 5, false), rat.die(1, false)), ("S_RATHITCLOSE", "S_RATDIECLOSE"));
        let ske = monster_sounds(0x1A, 2, "SKE", 0x24, 'C').unwrap();
        assert_eq!((ske.hit(1, 1, false), ske.die(1, false)), ("S_SKE2HIT1CLOSC", "S_SKE2DIECLOSEC"));
        assert_eq!(ske.hit(3, 3, true), "S_SKE2HIT2FARC");
        let spider = monster_sounds(9, 1, "SPID", 0x25, 'D').unwrap();
        assert_eq!(spider.die(3, false), "S_SPIDDIECLOSED");
        let forest = monster_sounds(0x18, 13, "FTRENT", -1, 'F').unwrap();
        assert_eq!(forest.hit(1, 1, false), "S_FTRENT2HIT1CL");
        assert_eq!(monster_sounds(0x1D, 5, "GOLLUM", -1, 'A'), None);
    }

    #[test]
    fn names_and_generator_rules() {
        assert_eq!(atree_name(4, 2).as_deref(), Some("GRU2"));
        assert_eq!(atree_name(4, 0).as_deref(), Some("GRU1"));
        assert_eq!(atree_name(4, 4).as_deref(), Some("GRUA"));
        assert_eq!(atree_name(29, 1).as_deref(), Some("GOLEM1"));
        assert_eq!(generator_defaults(1), (10, 5));
        assert_eq!(generator_defaults(3), (2, 15));
        assert_eq!(generator_defaults(0), (10, 5));
        assert_eq!(generator_wait(5, 0.0), 30.0);
        assert_eq!(generator_ramp_step(5), 0.1);
        assert_eq!(ACTION_NAMES[0xc], "ATTACK1");
        assert_eq!(ACTION_NAMES[0x20], "DEATH");
    }
}
