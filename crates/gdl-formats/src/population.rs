//! A level's population: the item types, item placements and locators
//! stored in its `WORLDS.PS2` (header words 18–23), which the game turns
//! into pickups, generators, monsters, doors, triggers, exits, player starts
//! and camera points when a level starts.
//!
//! Confirmed against `main.dol`; `docs/level-population.md` has the record
//! layouts, the functions that read each field and what's still unknown.
//! Little-endian like the rest of the file.

use thiserror::Error;

const HEADER_WORDS: usize = 30;
const ITEM_TYPE_STRIDE: usize = 0x50;
const PLACEMENT_STRIDE: usize = 0x3C;
const LOCATOR_STRIDE: usize = 0x1C;

/// Header words holding (count, offset) for each table.
const ITEM_TYPES_WORDS: (usize, usize) = (18, 19);
const PLACEMENTS_WORDS: (usize, usize) = (20, 21);
const LOCATORS_WORDS: (usize, usize) = (22, 23);

/// The game's own names for item classes, indexed by class (its debug
/// display clamps negative classes to 0 and prints `RANDOM`).
pub const CLASS_NAMES: [&str; 14] = [
    "RANDOM",
    "POWERUP",
    "CONTAINER",
    "GENERATOR",
    "ENEMYINFO",
    "TRIGGER",
    "TRAP",
    "DOOR",
    "DAMAGETILE",
    "EXIT",
    "OBSTACLE",
    "TRANSPORTER",
    "ROTATOR",
    "SOUND",
];

/// The game's names for item subtypes (the second word of an item type),
/// indexed by subtype; empty where the game has no name.
pub const SUBTYPE_NAMES: [&str; 50] = [
    "RANDOM", "GOLD", "KEY", "FOOD", "POTION", "WEAPON", "ARMOR", "SPEED", "MAGIC", "SPECIAL", //
    "RUNESTONE", "", "", "", "", "", "", "", "", "", //
    "BRIDGEPAD", "DOORPAD", "BRIDGESW", "DOORSW", "ACTIVESW", "ELEVPAD", "ELEVSW", "LIFTPAD", "LIFTSW",
    "LIFTEND", //
    "", "", "", "", "", "", "", "", "", "", //
    "FALLING", "SAFEROCK", "WALL", "BARREL", "EXP BARREL", "POI BARREL", "CHEST", "CHEST GOLD", "CHEST SLVR",
    "FALLING2SECRET",
];

/// Monster types by id: (the name item types use, generator model code).
/// Item names match the first column case-insensitively; generators draw
/// `GEN_<code><strength>`. Ids 28 and above skip 28, as the game's table does.
pub const ENEMY_CODES: [(i32, &str, &str); 44] = [
    (0, "sco", "SCO"),
    (1, "tro", "TRO"),
    (2, "dem", "DEM"),
    (3, "rat", "RAT"),
    (4, "gru", "GRU"),
    (5, "kni", "KNI"),
    (6, "sna", "SNA"),
    (7, "sor", "SOR"),
    (8, "mum", "MUM"),
    (9, "spi", "SPI"),
    (10, "liz", "LIZ"),
    (11, "tre", "TRE"),
    (12, "mag", "MAG"),
    (13, "zom", "ZOM"),
    (14, "pla", "PLA"),
    (15, "wol", "WOL"),
    (16, "ice", "ICE"),
    (17, "wrm", "WRM"),
    (18, "dog", "DOG"),
    (19, "ske", "SKE"),
    (20, "gho", "GHO"),
    (21, "aci", "ACI"),
    (22, "han", "HAN"),
    (23, "imp", "IMP"),
    (24, "war", "WAR"),
    (25, "sky", "SKY"),
    (26, "wind", "WIND"),
    (27, "grm", "GRM"),
    (29, "golem", "GOLEM"),
    (30, "death", "DEATH"),
    (31, "it", "IT"),
    (32, "gar", "GAR"),
    (33, "general", "GEN"),
    (34, "dragon", "DRAGON"),
    (35, "chimera", "CHIM"),
    (36, "djinn", "DJINN"),
    (37, "drider", "DRIDER"),
    (38, "pboss", "PBOSS"),
    (39, "yeti", "YETI"),
    (40, "wraith", "WRAITH"),
    (41, "lich", "LICH"),
    (42, "skorne1", "SKORNE"),
    (43, "skorne2", "SKORNE"),
    (44, "garm", "GARM1"),
];

/// The game's realm letters (first character of a level code like `A6`)
/// and realm ids; exit destinations are written as these codes.
pub const REALM_LETTERS: [(char, u32); 14] = [
    ('A', 1),
    ('B', 2),
    ('C', 3),
    ('D', 4),
    ('E', 5),
    ('F', 6),
    ('G', 7),
    ('H', 8),
    ('I', 9),
    ('J', 10),
    ('K', 11),
    ('L', 13),
    ('S', 12),
    ('T', 0),
];

/// The tower (hub) realm: the only one whose levels have several player
/// starts, chosen by entry index; everywhere else the game uses entry 0.
pub const HUB_REALM: u32 = 13;

#[derive(Debug, Error)]
pub enum PopulationError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("placement {0} uses item type {1}, but there are only {2}")]
    BadItemType(usize, i32, usize),
    #[error("random item type {0} picks from item type {1}, but there are only {2}")]
    BadChoice(usize, i32, usize),
}

/// What an item type is; the first word of its record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemClass {
    /// `-1`: picks one of several other item types when placed.
    Random,
    Powerup,
    Container,
    Generator,
    /// A monster placed directly.
    EnemyInfo,
    Trigger,
    Trap,
    Door,
    DamageTile,
    Exit,
    Obstacle,
    Transporter,
    Rotator,
    Sound,
    Other(i32),
}

impl ItemClass {
    pub fn from_raw(v: i32) -> Self {
        match v {
            -1 => Self::Random,
            1 => Self::Powerup,
            2 => Self::Container,
            3 => Self::Generator,
            4 => Self::EnemyInfo,
            5 => Self::Trigger,
            6 => Self::Trap,
            7 => Self::Door,
            8 => Self::DamageTile,
            9 => Self::Exit,
            10 => Self::Obstacle,
            11 => Self::Transporter,
            12 => Self::Rotator,
            13 => Self::Sound,
            v => Self::Other(v),
        }
    }

    pub fn raw(self) -> i32 {
        match self {
            Self::Random => -1,
            Self::Powerup => 1,
            Self::Container => 2,
            Self::Generator => 3,
            Self::EnemyInfo => 4,
            Self::Trigger => 5,
            Self::Trap => 6,
            Self::Door => 7,
            Self::DamageTile => 8,
            Self::Exit => 9,
            Self::Obstacle => 10,
            Self::Transporter => 11,
            Self::Rotator => 12,
            Self::Sound => 13,
            Self::Other(v) => v,
        }
    }

    /// The game's name for the class.
    pub fn name(self) -> &'static str {
        CLASS_NAMES.get(self.raw().max(0) as usize).copied().unwrap_or("UNKNOWN")
    }
}

/// An item type record (`0x50` bytes).
#[derive(Debug, Clone)]
pub struct ItemType {
    pub class: ItemClass,
    /// Second word: the powerup kind (gold, key, food...), trigger kind,
    /// obstacle kind, or for a random type the number of choices.
    pub subtype: i32,
    /// Model / atree name; for generators and placed monsters, the monster.
    pub name: String,
    /// Random types only: the item types to pick from.
    pub choices: Vec<usize>,
    /// `+0x0C..+0x1C`. The first two feed the item's radius (half the larger).
    pub extent: [f32; 4],
    /// `+0x1C`: offset of the item's centre from its position, turned with it.
    pub center_offset: [f32; 3],
    /// `+0x3C`: powerup value — potion colour, or the bit of the power granted.
    pub value: i32,
    /// `+0x40`: powerup amount — gold value, health from food, keys.
    pub amount: i16,
    /// `+0x42`: armour taken off each blow; -1 for items blows don't hurt.
    pub armor: i8,
    /// `+0x44`: hit points (multiplied by a generator's strength).
    pub hit_points: i16,
    /// `+0x46`: item flags copied to each placed item.
    pub flags: u16,
    /// `+0x4A`: powerup duration in seconds.
    pub duration: i16,
    /// The whole record, for fields not named yet.
    pub raw: [u8; ITEM_TYPE_STRIDE],
}

impl ItemType {
    /// `+0x0A` bit 0: the item keeps its placed height. Everything else is
    /// dropped onto the floor below when the level starts (sounds,
    /// rotators and some obstacles and triggers keep theirs).
    pub fn keeps_height(&self) -> bool {
        self.class != ItemClass::Random && self.raw[0x0A] & 1 != 0
    }

    /// The game's name for the subtype, where it has one.
    pub fn subtype_name(&self) -> Option<&'static str> {
        if self.class == ItemClass::Random {
            return None;
        }
        SUBTYPE_NAMES.get(usize::try_from(self.subtype).ok()?).copied().filter(|s| !s.is_empty())
    }

    /// Monster type id for a generator or placed monster, from its name
    /// (as the game resolves it). `None` for names outside the table, such
    /// as `BOSSGEN` (the level's boss) or unused leftovers.
    pub fn enemy(&self) -> Option<i32> {
        ENEMY_CODES.iter().find(|(_, name, _)| name.eq_ignore_ascii_case(&self.name)).map(|e| e.0)
    }
}

/// An item placed in the level (`0x3C` bytes).
#[derive(Debug, Clone)]
pub struct Placement {
    /// Index into [`Population::item_types`].
    pub item_type: usize,
    /// `+2`: player count gate — see [`Placement::active_for`].
    pub players: u8,
    /// `+3`: bit 0 sets an item flag, bit 1 places it without a model,
    /// bit 2 sets a model flag.
    pub flags: u8,
    /// `+4`, `+6`: copied onto the item unread by placement.
    pub links: [i16; 2],
    /// `+8`: overrides the type's model name (walls name a level object;
    /// sounds name the sound).
    pub name: String,
    pub position: [f32; 3],
    /// Euler angles in radians, applied X then Y then Z; see
    /// [`rotation_matrix`].
    pub rotation: [f32; 3],
    /// `+0x30..+0x3C`: class-specific parameters; see [`Placement::params`].
    pub params: [u8; 12],
}

impl Placement {
    /// Whether the game creates this item with `players` players: `n` in
    /// 1..=10 means "at least n", `n` above 10 means "exactly n − 10", 0
    /// means always.
    pub fn active_for(&self, players: u8) -> bool {
        match self.players {
            n if n > 10 => players == n - 10,
            n => n <= players,
        }
    }

    /// Model name the game uses: the placement's own, else the type's.
    pub fn model_name<'a>(&'a self, ty: &'a ItemType) -> &'a str {
        if self.name.is_empty() { &ty.name } else { &self.name }
    }

    fn i16_at(&self, at: usize) -> i16 {
        i16::from_le_bytes([self.params[at], self.params[at + 1]])
    }

    fn i32_at(&self, at: usize) -> i32 {
        i32::from_le_bytes(self.params[at..at + 4].try_into().unwrap())
    }

    fn f32_at(&self, at: usize) -> f32 {
        f32::from_bits(self.i32_at(at) as u32)
    }

    /// The parameters decoded for the item's class.
    pub fn params(&self, class: ItemClass) -> PlacementParams {
        match class {
            ItemClass::Container => PlacementParams::Container {
                contents: usize::try_from(self.i16_at(0)).ok(),
                param: self.i16_at(4),
            },
            ItemClass::Generator => PlacementParams::Generator {
                strength: self.i16_at(0),
                ai: self.i16_at(2),
                max: self.i16_at(4),
                rate: self.i16_at(6),
            },
            ItemClass::EnemyInfo => PlacementParams::Enemy {
                level: self.i16_at(0),
                ai: self.i16_at(2),
                range: self.f32_at(4),
                param: self.i16_at(8),
            },
            ItemClass::Trigger => PlacementParams::Trigger {
                target: usize::try_from(self.i16_at(0)).ok(),
                flags: self.i16_at(2) as u16,
                time: self.params[4],
                id: self.params[6],
                next: self.params[7],
            },
            ItemClass::Exit => PlacementParams::Exit {
                destination: (self.i32_at(0) == 0).then(|| cstr(&self.params[4..12])).filter(|s| !s.is_empty()),
            },
            ItemClass::Transporter => {
                PlacementParams::Transporter { id: self.i32_at(0), destination: self.i32_at(4) }
            }
            ItemClass::Obstacle => {
                PlacementParams::Obstacle { subtype: self.i16_at(0), count: self.i16_at(2) }
            }
            ItemClass::Rotator => PlacementParams::Rotator {
                target: usize::try_from(self.i32_at(0)).ok(),
                param: self.i32_at(4),
                speed: self.f32_at(8),
            },
            ItemClass::Sound => PlacementParams::Sound { radius: self.f32_at(0) },
            ItemClass::Powerup => PlacementParams::Powerup { count: self.i16_at(0) },
            _ => PlacementParams::None,
        }
    }
}

/// A placement's class-specific parameters, as the item constructor reads
/// them. Names follow the game's debug display where it has one.
#[derive(Debug, Clone, PartialEq)]
pub enum PlacementParams {
    /// Keys with a count above 1 become the key ring; some powerups take a
    /// count.
    Powerup { count: i16 },
    /// `contents`: the item type released when it's broken or opened.
    Container { contents: Option<usize>, param: i16 },
    /// Debug display: `GENERATOR (<monster>-<ai>) Lv<strength> Max=<max>`.
    /// Strength 1–3 picks the model `GEN_<code><strength>`; a zero `max`
    /// or `rate` takes a per-strength default (10/5/2 and 5/10/15).
    Generator { strength: i16, ai: i16, max: i16, rate: i16 },
    Enemy { level: i16, ai: i16, range: f32, param: i16 },
    /// `target`: index of the `WORLDS.PS2` node it moves; `id` / `next`
    /// chain triggers together.
    Trigger { target: Option<usize>, flags: u16, time: u8, id: u8, next: u8 },
    /// A level code such as `A6` (realm letter + level digit), or `None`
    /// for the default destination.
    Exit { destination: Option<String> },
    /// Transporters pair up: each sends to the one whose `id` is its
    /// `destination`.
    Transporter { id: i32, destination: i32 },
    /// `subtype` overrides the type's (0 = keep it).
    Obstacle { subtype: i16, count: i16 },
    /// `target`: index of the `WORLDS.PS2` node it turns.
    Rotator { target: Option<usize>, param: i32, speed: f32 },
    Sound { radius: f32 },
    None,
}

/// What a locator marks; the first byte of its record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocatorKind {
    /// 1–4 and 9: what the game's messages call "transmitters" — camera
    /// points. 1 is an entry's starting camera (by index), 9 is linked to
    /// the trigger whose id is the locator's index.
    Transmitter(u8),
    /// 5.
    Milestone,
    /// 6: where the boss appears (only boss levels have one).
    Boss,
    /// 7: where the players start; the index is the entry.
    PlayerStart,
    /// 8 and 10.
    Lookout(u8),
    Other(u8),
}

impl LocatorKind {
    pub fn from_raw(v: u8) -> Self {
        match v {
            1..=4 | 9 => Self::Transmitter(v),
            5 => Self::Milestone,
            6 => Self::Boss,
            7 => Self::PlayerStart,
            8 | 10 => Self::Lookout(v),
            v => Self::Other(v),
        }
    }
}

/// A locator record (`0x1C` bytes).
#[derive(Debug, Clone)]
pub struct Locator {
    pub kind: LocatorKind,
    /// `+1`: copied onto cameras and lookouts.
    pub param: u8,
    /// `+2`: entry index (starts, starting cameras) or trigger id.
    pub index: i16,
    pub position: [f32; 3],
    /// Euler angles in radians (X, Y, Z).
    pub rotation: [f32; 3],
}

/// Where the players start a level, and which way they face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerStart {
    /// Entry index (0 except in the tower hub).
    pub entry: i16,
    pub position: [f32; 3],
    /// The locator's Y rotation, radians — the angle the game gives the
    /// players on arrival.
    pub yaw: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Population {
    pub item_types: Vec<ItemType>,
    pub placements: Vec<Placement>,
    pub locators: Vec<Locator>,
}

impl Population {
    /// Reads the population tables of a whole `WORLDS.PS2` file.
    pub fn parse(worlds: &[u8]) -> Result<Self, PopulationError> {
        let h = slice(worlds, 0, HEADER_WORDS * 4)?;
        let word = |i: usize| le_u32(h, i * 4) as usize;
        let table = |(count, offset): (usize, usize), stride: usize| {
            let n = word(count);
            let at = word(offset);
            n.checked_mul(stride)
                .ok_or(PopulationError::Truncated(at, usize::MAX))
                .and_then(|len| slice(worlds, at, len))
                .map(|t| t.chunks_exact(stride))
        };

        let mut item_types = Vec::new();
        for r in table(ITEM_TYPES_WORDS, ITEM_TYPE_STRIDE)? {
            item_types.push(parse_item_type(r));
        }
        // A random type lists up to 16 item type indices after its count.
        let count = item_types.len();
        for (i, t) in item_types.iter_mut().enumerate().filter(|(_, t)| t.class == ItemClass::Random) {
            for k in 0..t.subtype.clamp(0, 16) as usize {
                match le_u16(&t.raw, 8 + k * 2) as i16 {
                    c if c >= 0 && (c as usize) < count => t.choices.push(c as usize),
                    c => return Err(PopulationError::BadChoice(i, c as i32, count)),
                }
            }
        }

        let mut placements = Vec::new();
        for (i, r) in table(PLACEMENTS_WORDS, PLACEMENT_STRIDE)?.enumerate() {
            let ty = le_u16(r, 0) as i16;
            if ty < 0 || ty as usize >= count {
                return Err(PopulationError::BadItemType(i, ty as i32, count));
            }
            placements.push(Placement {
                item_type: ty as usize,
                players: r[2],
                flags: r[3],
                links: [le_u16(r, 4) as i16, le_u16(r, 6) as i16],
                name: cstr(&r[8..0x18]),
                position: vec3(r, 0x18),
                rotation: vec3(r, 0x24),
                params: r[0x30..0x3C].try_into().unwrap(),
            });
        }

        let locators = table(LOCATORS_WORDS, LOCATOR_STRIDE)?
            .map(|r| Locator {
                kind: LocatorKind::from_raw(r[0]),
                param: r[1],
                index: le_u16(r, 2) as i16,
                position: vec3(r, 4),
                rotation: vec3(r, 0x10),
            })
            .collect();

        Ok(Self { item_types, placements, locators })
    }

    /// The item type a placement ends up with, following random types to
    /// their first choice (the game picks one at run time).
    pub fn resolved_type(&self, placement: &Placement) -> &ItemType {
        self.resolve(placement.item_type)
    }

    /// Follows random item types to their first choice.
    pub fn resolve(&self, mut index: usize) -> &ItemType {
        for _ in 0..16 {
            let t = &self.item_types[index];
            match t.choices.first() {
                Some(&c) if t.class == ItemClass::Random => index = c,
                _ => return t,
            }
        }
        &self.item_types[index]
    }

    /// Every player start, in file order.
    pub fn player_starts(&self) -> impl Iterator<Item = PlayerStart> + '_ {
        self.locators.iter().filter(|l| l.kind == LocatorKind::PlayerStart).map(|l| PlayerStart {
            entry: l.index,
            position: l.position,
            yaw: l.rotation[1],
        })
    }

    /// The start for `entry`, falling back to entry 0 as the game does
    /// (outside the hub, every start counts as entry 0).
    pub fn player_start(&self, entry: i16) -> Option<PlayerStart> {
        self.player_starts()
            .find(|s| s.entry == entry)
            .or_else(|| self.player_starts().find(|s| s.entry == 0))
            .or_else(|| self.player_starts().next())
    }
}

/// Rotation the game builds for a placement from its Euler angles: start
/// from identity, then rotate about X, Y and Z in that order. Row-major for
/// row vectors (the translation would go in the last row), so read as
/// columns it's the matrix for column vectors.
pub fn rotation_matrix(rotation: [f32; 3]) -> [f32; 9] {
    let mut m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    // Each step mixes two columns (a, b) of every row: (1, 2) for X,
    // (0, 2) for Y, (0, 1) for Z — the same formula for all three.
    let mut turn = |a: usize, b: usize, angle: f32| {
        let (s, c) = angle.sin_cos();
        for row in 0..3 {
            let (x, y) = (m[row * 3 + a], m[row * 3 + b]);
            m[row * 3 + a] = c * x - s * y;
            m[row * 3 + b] = c * y + s * x;
        }
    };
    turn(1, 2, rotation[0]);
    turn(0, 2, rotation[1]);
    turn(0, 1, rotation[2]);
    m
}

/// Level folder name for an exit destination code like `A6` or `g1`.
pub fn level_for_code(code: &str) -> Option<String> {
    let mut c = code.chars();
    let letter = c.next()?.to_ascii_uppercase();
    let digit = c.next().filter(char::is_ascii_digit)?;
    REALM_LETTERS.iter().any(|(l, _)| *l == letter).then(|| format!("level{letter}{digit}"))
}

fn parse_item_type(r: &[u8]) -> ItemType {
    let class = ItemClass::from_raw(le_u32(r, 0) as i32);
    ItemType {
        class,
        subtype: le_u32(r, 4) as i32,
        name: if class == ItemClass::Random { String::new() } else { cstr(&r[0x28..0x38]) },
        choices: Vec::new(),
        extent: [le_f32(r, 0xC), le_f32(r, 0x10), le_f32(r, 0x14), le_f32(r, 0x18)],
        center_offset: vec3(r, 0x1C),
        value: le_u32(r, 0x3C) as i32,
        amount: le_u16(r, 0x40) as i16,
        armor: r[0x42] as i8,
        hit_points: le_u16(r, 0x44) as i16,
        flags: le_u16(r, 0x46),
        duration: le_u16(r, 0x4A) as i16,
        raw: r.try_into().unwrap(),
    }
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn slice(file: &[u8], at: usize, len: usize) -> Result<&[u8], PopulationError> {
    at.checked_add(len)
        .and_then(|end| file.get(at..end))
        .ok_or(PopulationError::Truncated(at, at.saturating_add(len)))
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(le_u32(b, at))
}

fn vec3(b: &[u8], at: usize) -> [f32; 3] {
    [le_f32(b, at), le_f32(b, at + 4), le_f32(b, at + 8)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelFile;
    use crate::world::WorldFile;
    use std::collections::{BTreeMap, HashSet};

    fn put_u32(f: &mut [u8], at: usize, v: u32) {
        f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put_f32s(f: &mut [u8], at: usize, v: &[f32]) {
        for (k, x) in v.iter().enumerate() {
            put_u32(f, at + k * 4, x.to_bits());
        }
    }

    /// A `WORLDS.PS2` header followed by the given population tables.
    fn file(types: &[Vec<u8>], places: &[Vec<u8>], locs: &[Vec<u8>]) -> Vec<u8> {
        let mut f = vec![0u8; HEADER_WORDS * 4];
        for (words, recs) in [(ITEM_TYPES_WORDS, types), (PLACEMENTS_WORDS, places), (LOCATORS_WORDS, locs)] {
            let at = f.len() as u32;
            put_u32(&mut f, words.0 * 4, recs.len() as u32);
            put_u32(&mut f, words.1 * 4, at);
            for r in recs {
                f.extend(r);
            }
        }
        f
    }

    fn item_type(class: i32, subtype: i32, name: &str) -> Vec<u8> {
        let mut r = vec![0u8; ITEM_TYPE_STRIDE];
        put_u32(&mut r, 0, class as u32);
        put_u32(&mut r, 4, subtype as u32);
        r[0x28..0x28 + name.len()].copy_from_slice(name.as_bytes());
        r
    }

    fn placement(ty: i16, players: u8, pos: [f32; 3], rot: [f32; 3], params: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; PLACEMENT_STRIDE];
        r[0..2].copy_from_slice(&ty.to_le_bytes());
        r[2] = players;
        put_f32s(&mut r, 0x18, &pos);
        put_f32s(&mut r, 0x24, &rot);
        r[0x30..0x30 + params.len()].copy_from_slice(params);
        r
    }

    fn locator(kind: u8, index: i16, pos: [f32; 3], rot: [f32; 3]) -> Vec<u8> {
        let mut r = vec![0u8; LOCATOR_STRIDE];
        r[0] = kind;
        r[2..4].copy_from_slice(&index.to_le_bytes());
        put_f32s(&mut r, 4, &pos);
        put_f32s(&mut r, 0x10, &rot);
        r
    }

    #[test]
    fn parses_types_placements_and_locators() {
        let mut key = item_type(1, 2, "KEY");
        key[0x40..0x42].copy_from_slice(&1i16.to_le_bytes());
        let mut random = item_type(-1, 2, "");
        random[8..10].copy_from_slice(&0u16.to_le_bytes());
        random[10..12].copy_from_slice(&2u16.to_le_bytes());
        let generator = item_type(3, 0, "GRU");
        // strength 2, ai 7, max 6, rate 14
        let gen_params = [2, 0, 7, 0, 6, 0, 14, 0];
        let f = file(
            &[key, random, generator],
            &[
                placement(0, 1, [1.0, 2.0, 3.0], [0.0; 3], &[]),
                placement(1, 12, [0.0; 3], [0.0; 3], &[]),
                placement(2, 0, [5.0, 0.0, 5.0], [0.0, 1.5, 0.0], &gen_params),
            ],
            &[locator(2, 0, [0.0; 3], [0.5, 0.0, 0.0]), locator(7, 0, [4.0, 0.0, -2.0], [0.0, 3.0, 0.0])],
        );
        let p = Population::parse(&f).unwrap();
        assert_eq!(p.item_types[0].class, ItemClass::Powerup);
        assert_eq!(p.item_types[0].subtype_name(), Some("KEY"));
        assert_eq!(p.item_types[0].amount, 1);
        assert_eq!(p.item_types[1].choices, vec![0, 2]);
        assert_eq!(p.resolved_type(&p.placements[1]).name, "KEY");
        assert_eq!(p.item_types[2].enemy(), Some(4));
        assert_eq!(p.placements[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(
            p.placements[2].params(ItemClass::Generator),
            PlacementParams::Generator { strength: 2, ai: 7, max: 6, rate: 14 }
        );
        // Player count gates: 1 = one or more, 12 = exactly two, 0 = always.
        assert!(p.placements[0].active_for(1) && p.placements[0].active_for(4));
        assert!(!p.placements[1].active_for(1) && p.placements[1].active_for(2) && !p.placements[1].active_for(3));
        assert!(p.placements[2].active_for(1));
        assert_eq!(p.locators[0].kind, LocatorKind::Transmitter(2));
        let start = p.player_start(0).unwrap();
        assert_eq!((start.position, start.yaw), ([4.0, 0.0, -2.0], 3.0));
        // Outside the hub every start is entry 0, so any entry finds it.
        assert_eq!(p.player_start(5), Some(start));
    }

    #[test]
    fn rejects_bad_indices() {
        let f = file(&[item_type(1, 1, "COIN")], &[placement(3, 0, [0.0; 3], [0.0; 3], &[])], &[]);
        assert!(matches!(Population::parse(&f), Err(PopulationError::BadItemType(0, 3, 1))));
        let mut random = item_type(-1, 1, "");
        random[8..10].copy_from_slice(&9u16.to_le_bytes());
        let f = file(&[random], &[], &[]);
        assert!(matches!(Population::parse(&f), Err(PopulationError::BadChoice(0, 9, 1))));
        assert!(matches!(Population::parse(&[0; 8]), Err(PopulationError::Truncated(..))));
    }

    #[test]
    fn rotation_applies_x_then_y_then_z() {
        let close = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        let h = std::f32::consts::FRAC_PI_2;
        // Row vectors: v' = v · M, so row i is where axis i goes.
        // About X: +Y to +Z. About Y: +X to +Z. About Z: +X to +Y.
        assert!(close(&rotation_matrix([h, 0.0, 0.0]), &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0]));
        assert!(close(&rotation_matrix([0.0, h, 0.0]), &[0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0]));
        assert!(close(&rotation_matrix([0.0, 0.0, h]), &[0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0]));
        // X first: +Y goes to +Z (X), then +Z to -X (Y).
        assert!(close(&rotation_matrix([h, h, 0.0])[3..6], &[-1.0, 0.0, 0.0]));
    }

    #[test]
    fn level_codes() {
        assert_eq!(level_for_code("A6").as_deref(), Some("levelA6"));
        assert_eq!(level_for_code("g1").as_deref(), Some("levelG1"));
        assert_eq!(level_for_code("Z1"), None);
        assert_eq!(level_for_code(""), None);
    }

    fn inside(p: [f32; 3], min: [f32; 3], max: [f32; 3], margin: f32) -> bool {
        (0..3).all(|k| p[k] >= min[k] - margin && p[k] <= max[k] + margin)
    }

    /// Every level's population parses, and it's plausible: placements sit
    /// inside the level, every level has a start, targets name real nodes,
    /// exits lead to real levels and transporters pair up.
    #[test]
    fn every_real_level_populates() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let mut levels: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        levels.sort();
        let names: HashSet<String> =
            levels.iter().filter_map(|l| Some(l.file_name()?.to_str()?.to_ascii_lowercase())).collect();

        let (mut parsed, mut placed, mut outside) = (0, 0, 0);
        let (mut targets, mut targets_ok, mut walls, mut walls_ok) = (0, 0, 0, 0);
        let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
        for level in &levels {
            let Ok(w) = std::fs::read(level.join("WORLDS.PS2")) else { continue };
            let name = level.file_name().unwrap().to_string_lossy().into_owned();
            let world = WorldFile::parse(&w).unwrap_or_else(|e| panic!("{name}: {e}"));
            let pop = Population::parse(&w).unwrap_or_else(|e| panic!("{name}: {e}"));
            let (min, max) = (world.header.bounds_min, world.header.bounds_max);
            let objects: HashSet<String> = std::fs::read(level.join("objects.ngc"))
                .ok()
                .and_then(|m| ModelFile::parse(&m).ok())
                .map(|m| m.objects.into_iter().map(|o| o.name).collect())
                .unwrap_or_default();

            let starts: Vec<_> = pop.player_starts().collect();
            assert!(!starts.is_empty(), "{name}: no player start");
            assert_eq!(starts.iter().filter(|s| s.entry == 0).count(), 1, "{name}: entry 0 starts");
            for s in &starts {
                assert!(inside(s.position, min, max, 5.0), "{name}: start {s:?} outside the level");
            }

            let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
            let mut transporters = Vec::new();
            for p in &pop.placements {
                let class = pop.item_types[p.item_type].class;
                *by_class.entry(class.name()).or_default() += 1;
                placed += 1;
                outside += !inside(p.position, min, max, 1.0) as usize;
                match p.params(class) {
                    PlacementParams::Trigger { target: Some(t), .. }
                    | PlacementParams::Rotator { target: Some(t), .. } => {
                        // A model node or a group (drawbridges, lifts): every
                        // node gets an instance at run time.
                        assert!(t < world.nodes.len(), "{name}: target node {t} of {}", world.nodes.len());
                        targets += 1;
                        targets_ok += world.nodes[t].has_model as usize;
                    }
                    PlacementParams::Exit { destination: Some(d) } => {
                        let to = level_for_code(&d).unwrap_or_else(|| panic!("{name}: exit to '{d}'"));
                        assert!(names.contains(&to.to_ascii_lowercase()), "{name}: exit to missing {to}");
                    }
                    PlacementParams::Transporter { id, destination } => transporters.push((id, destination)),
                    PlacementParams::Container { contents: Some(c), .. } => {
                        assert!(c < pop.item_types.len(), "{name}: container holds type {c}")
                    }
                    _ => {}
                }
                if class == ItemClass::Obstacle && !p.name.is_empty() {
                    walls += 1;
                    walls_ok += objects.contains(&p.name) as usize;
                }
            }
            for &(id, dest) in &transporters {
                assert!(transporters.iter().any(|t| t.0 == dest), "{name}: transporter {id} goes to missing {dest}");
            }
            eprintln!(
                "{name}: {} starts, {} generators, {} monsters, {} locators; {by_class:?}",
                starts.len(),
                by_class.get("GENERATOR").copied().unwrap_or(0),
                by_class.get("ENEMYINFO").copied().unwrap_or(0),
                pop.locators.len()
            );
            for (k, v) in by_class {
                *totals.entry(k).or_default() += v;
            }
            parsed += 1;
        }
        eprintln!(
            "{parsed} levels, {placed} placements ({outside} outside their level's bounds); \
             {targets} trigger/rotator targets ({targets_ok} model nodes, the rest groups); \
             {walls_ok}/{walls} named obstacles are level objects; {totals:?}"
        );
        assert!(outside * 100 <= placed, "too many placements outside their level: {outside}/{placed}");
        assert!(walls_ok * 10 >= walls * 9, "too few named obstacles resolve: {walls_ok}/{walls}");
    }
}
