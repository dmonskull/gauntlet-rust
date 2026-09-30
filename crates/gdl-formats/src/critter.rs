//! `CRITTER/<name>.WAD` — the data behind the game's scripted-monster
//! ("critter") system: the bosses, and the golem, gargoyles and general
//! that regular levels place. Regular monsters don't have one of these
//! files — their stats are compiled into the game (`enemy.rs`).
//!
//! A chunk file (`chunk.rs`) whose eight chunks the game's critter loader
//! looks up by tag and byte-swaps field by field, which fixes each record's
//! size and field widths. `docs/critters.md` has the loader, every field
//! the runtime reads, and what the critter update does with them:
//!
//! - `DESC` (one per file): name, model prefix and critter class.
//! - `TYPE`: one per body (a boss, or each head of a multi-part one) —
//!   stats, the targeting condition, and spans into the other tables.
//! - `MOVE`: the body's moves — an animation with a kind (idle, walk,
//!   attack, hit reaction…), a priority, the frames its blows land on and
//!   the `DAMG` records they deal, a condition on the target, a cooldown,
//!   speeds. The update picks one every frame.
//! - `PTRN`: fixed sequences of up to eight moves.
//! - `NODE`: hit spheres on skeleton nodes, some breakable.
//! - `DAMG`: what a blow does — a sphere on a node, a projectile, a breath
//!   cone, a ring on the ground — and how hard.
//! - `SFXX`: the effects and sounds moves and blows start (chained).
//! - `ADDA`: extra animated models attached to a body.

use thiserror::Error;

use crate::chunk::{ChunkError, ChunkFile};

/// Record sizes, by tag, as the loader walks them.
pub const RECORD_SIZES: [(&str, usize); 8] = [
    ("SFXX", 0x50),
    ("DAMG", 0x50),
    ("MOVE", 0x90),
    ("PTRN", 0x50),
    ("NODE", 0x50),
    ("DESC", 0x30),
    ("TYPE", 0x140),
    ("ADDA", 0x30),
];

#[derive(Debug, Error)]
pub enum CritterError {
    #[error(transparent)]
    Chunk(#[from] ChunkError),
    #[error("chunk {0} is missing")]
    MissingChunk(&'static str),
    #[error("chunk {0} is truncated")]
    Truncated(&'static str),
    #[error("the file has no critter types")]
    NoTypes,
    #[error("the file has no DESC record")]
    NoDesc,
}

/// The critter classes (`DESC +0x20`): which update runs a critter.
pub mod class {
    /// The golem: placed in regular levels, woken from a statue.
    pub const GOLEM: i16 = 3;
    /// Bosses: the level's boss locator makes them.
    pub const BOSS: i16 = 4;
    /// Gargoyles: placed, woken from a statue.
    pub const GARGOYLE: i16 = 7;
    /// The general: placed.
    pub const GENERAL: i16 = 8;
}

/// Move kinds (`MOVE +0x00`). The update looks moves up by kind.
pub mod kind {
    /// Played once when the critter is created.
    pub const INIT: i32 = 0x00;
    /// A chimera head's "all heads together" move.
    pub const TOGETHER: i32 = 0x01;
    pub const START: i32 = 0x10;
    pub const DEATH: i32 = 0x11;
    pub const READY: i32 = 0x20;
    /// Idle while unhurt (anger below 0.8).
    pub const TAUNT: i32 = 0x21;
    /// After taking enough damage in a short while.
    pub const ROAR: i32 = 0x22;
    /// Chosen when the target player attacks.
    pub const BLOCK: i32 = 0x23;
    /// First and last movement kinds (walk, back up, strafe, go to a point).
    pub const MOVE_FIRST: i32 = 0x30;
    pub const MOVE_LAST: i32 = 0x39;
    pub const WALK_LEFT: i32 = 0x32;
    pub const WALK_RIGHT: i32 = 0x33;
    pub const WALK: i32 = 0x34;
    pub const BACK: i32 = 0x35;
    pub const WALK_DIAGONAL: i32 = 0x36;
    pub const WALK_ON: i32 = 0x37;
    pub const GO_TO_POINT: i32 = 0x38;
    /// Hit reactions: a flinch, knocked back, knocked down.
    pub const FLINCH: i32 = 0x40;
    pub const KNOCKBACK: i32 = 0x41;
    pub const KNOCKDOWN: i32 = 0x42;
    /// Attack kinds are 0x7F and up (below [`SKIP`]).
    pub const ATTACK_FIRST: i32 = 0x7F;
    /// A melee sweep: blows land every frame between the hit frames.
    pub const SWEEP: i32 = 0x80;
    pub const GRAB: i32 = 0x81;
    pub const SWEEP_2: i32 = 0x83;
    pub const REPEAT: i32 = 0x85;
    pub const SWEEP_3: i32 = 0x86;
    pub const AIMED: i32 = 0x88;
    /// Skipped when stepping through the moves in order.
    pub const SKIP: i32 = 0xF0;
}

/// A condition on a target (`TYPE +0x80`, `MOVE +0x60`, `PTRN +0x30`):
/// eight floats the target scoring checks before it scores a player.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Condition {
    /// Nearer than this fails.
    pub min_distance: f32,
    /// Further than this fails, when positive.
    pub max_distance: f32,
    /// The facing is turned by this before the cone test (radians).
    pub angle: f32,
    /// The cosine between that direction and the target must reach this.
    pub min_cos: f32,
    /// The critter's anger must be at least this…
    pub min_anger: f32,
    /// …and below this, when it's above `min_anger`.
    pub max_anger: f32,
    /// `+0x18`: not read by the scoring.
    pub unused: f32,
    /// Height difference above this fails, when positive.
    pub max_height: f32,
}

impl Condition {
    fn parse(r: &[u8], at: usize) -> Self {
        let f = |i: usize| f32_at(r, at + 4 * i);
        Self {
            min_distance: f(0),
            max_distance: f(1),
            angle: f(2),
            min_cos: f(3),
            min_anger: f(4),
            max_anger: f(5),
            unused: f(6),
            max_height: f(7),
        }
    }
}

/// A run of records in one of the other tables: `count` from `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub first: i16,
    pub count: i16,
}

impl Span {
    pub fn range(&self) -> std::ops::Range<usize> {
        let first = self.first.max(0) as usize;
        first..first + self.count.max(0) as usize
    }
}

/// `DESC` (`0x30` bytes, one per file).
#[derive(Debug, Clone)]
pub struct CritterDesc {
    /// `+0x00`: `golem`, `dragon`… (the folder name under `MONSTERS/`).
    pub name: String,
    /// `+0x10`: prefix of the bodies' atree names (`GOLEM` + `1`).
    pub prefix: String,
    /// `+0x20`: see [`class`].
    pub class: i16,
}

/// `TYPE` (`0x140` bytes): one body.
#[derive(Debug, Clone)]
pub struct CritterType {
    /// `+0x00`: appended to the `DESC` prefix to name its atree.
    pub name: String,
    /// `+0x20`, `+0x30`: nodes that turn toward the target (a head); the
    /// first also carries attached effects.
    pub look_nodes: [String; 2],
    /// `+0x40`: node effects attach to.
    pub effect_node: String,
    /// `+0x50`: its `DESC`.
    pub desc: i16,
    /// `+0x52`: the subtype it registers as (−1: only made as a part).
    pub subtype: i16,
    /// `+0x5C`: behaviour flags (`docs/critters.md`).
    pub flags: u32,
    /// `+0x60`..`+0x74`: how far the look nodes turn: first node yaw,
    /// second node yaw, first pitch, second pitch, first roll…
    pub look_limits: [f32; 6],
    /// `+0x78`: height of its cylinder.
    pub height: f32,
    /// `+0x7C`: radius of its cylinder.
    pub radius: f32,
    /// `+0x80`: which players it considers at all.
    pub target: Condition,
    /// `+0xA0`: its home, when `+0xA4` is below 999; else where it's made.
    pub home: [f32; 3],
    /// `+0xAC`: how far a boss may leave its home.
    pub leash: f32,
    /// `+0xB0`: how far above the floor its root stands (fliers).
    pub hover: f32,
    /// `+0xB4`: height of the point the hero aims at.
    pub aim_height: f32,
    /// `+0xBC`: armour: taken off every blow (a weaker blow does nothing).
    pub armor: f32,
    /// `+0xC0`: its centre, in its own space.
    pub center: [f32; 3],
    /// `+0xCC`: how far it turns away from the way it was made facing.
    pub max_turn: f32,
    /// `+0xD0`: where a boss drops its key, from its root.
    pub key_offset: [f32; 3],
    /// `+0xE0`: elemental resistances (bits by blow element).
    pub resist: u32,
    /// `+0xE4`: hit points before the level's scale.
    pub hit_points: f32,
    /// `+0xE8`: experience for its hit points (a share per blow).
    pub experience: f32,
    /// `+0xEC`: a boss wakes when a player is this close (0: at once).
    pub wake_distance: f32,
    /// `+0xF4`, `+0xF6`: `SFXX` for being hit (the second for blows from
    /// the second hit effect).
    pub hit_effects: [i16; 2],
    /// `+0x110`: its `MOVE` records.
    pub moves: Span,
    /// `+0x114`: its `PTRN` records.
    pub patterns: Span,
    /// `+0x118`: its `NODE` records.
    pub nodes: Span,
    /// `+0x11C`: the next part, made with this one.
    pub child: Option<usize>,
    /// `+0x11E`: the body whose atree it shares.
    pub parent: Option<usize>,
    pub raw: Vec<u8>,
}

/// `MOVE` (`0x90` bytes).
#[derive(Debug, Clone)]
pub struct CritterMove {
    /// `+0x00`: see [`kind`].
    pub kind: i32,
    /// `+0x04`: 1 can't be interrupted by hits, 2 needs every part alive,
    /// 4 disabled, 8 animation flag, 0x10 needs its node, 0x20 turn to the
    /// home facing.
    pub flags: u32,
    /// `+0x08`: priority; the high byte decides interruptions, 0xF00 and
    /// up interrupt anything.
    pub priority: i32,
    /// `+0x10`: the move's own name.
    pub name: String,
    /// `+0x20`: its animation (an action of the body's atree).
    pub anim: String,
    /// `+0x30`: the node blows are measured from (empty: the root).
    pub node: String,
    /// `+0x40`, `+0x44`: animation frames the two blows land on (−1 none).
    pub hit_frames: [i32; 2],
    /// `+0x48`, `+0x4A`: their `DAMG` records.
    pub damage: [i16; 2],
    /// `+0x50`, `+0x52`: last frame of each blow's window (sweeps).
    pub hit_ends: [i16; 2],
    /// `+0x54`: the move that follows this one.
    pub next: i16,
    /// `+0x56`: how a following move may cut in (`docs/critters.md`).
    pub transition: i16,
    /// `+0x58`..`+0x5E`: two (`SFXX`, frame) pairs started during it.
    pub sounds: [(i16, i16); 2],
    /// `+0x60`: condition on its target.
    pub condition: Condition,
    /// `+0x80`: seconds after it ends before it may be chosen again.
    pub cooldown: f32,
    /// `+0x84`: ground speed (units/s).
    pub speed: f32,
    /// `+0x88`: turn rate (radians/s).
    pub turn: f32,
    /// `+0x8C`: bosses hold it at least this long (seconds).
    pub hold: f32,
}

/// `PTRN` (`0x50` bytes): a sequence of moves.
#[derive(Debug, Clone)]
pub struct CritterPattern {
    pub name: String,
    /// `+0x10`: 2 needs every part alive, 0x1000 never chosen on its own.
    pub flags: u16,
    /// `+0x14`: seconds before it may run again.
    pub cooldown: f32,
    /// `+0x20`: up to eight moves (−1 ends).
    pub moves: [i16; 8],
    /// `+0x30`: condition on its target.
    pub condition: Condition,
}

/// `NODE` (`0x50` bytes): a hit sphere on a skeleton node.
#[derive(Debug, Clone)]
pub struct CritterNode {
    /// `+0x00`: the skeleton node (empty: the body's root).
    pub name: String,
    /// `+0x10`: 2 breakable, 4 removes its model when broken, 8 solid.
    pub flags: u16,
    /// `+0x12`: `DAMG` started when it breaks.
    pub break_damage: i16,
    /// `+0x16`: passed to the model (not traced).
    pub model_flag: i16,
    /// `+0x18`: how far away the hero's attack search finds it (0: as far
    /// as the search looks).
    pub reach: f32,
    /// `+0x1C`: how much the search favours it among its body's spheres.
    pub weight: f32,
    /// `+0x20`: the sphere's centre, in the node's space.
    pub offset: [f32; 3],
    /// `+0x2C`: the sphere's radius.
    pub radius: f32,
    /// `+0x30`: a second node, or `+n`/`-n` steps along the model.
    pub model_node: String,
    /// `+0x40`: blows on it are scaled by this.
    pub damage_scale: f32,
    /// `+0x44`: its hit points, as a share of the body's.
    pub hit_points: f32,
}

/// `DAMG` (`0x50` bytes): what a blow does.
#[derive(Debug, Clone)]
pub struct CritterDamage {
    /// `+0x00`: 0 a sphere on the move's node, 1 a projectile, 2 an
    /// attached effect, 3 a ground effect, 4 a breath cone, 5/6 the
    /// level's generators, 7 a grab, 8 a projectile at a point, 9 a
    /// body-specific effect.
    pub kind: i16,
    /// `+0x02`: 1 aim at the target, 4 aim along the body, 8 fly straight
    /// (no arc), 0x4000 only while its node's parts live.
    pub flags: u16,
    /// `+0x04`: the blow's kind bits (0x10 strong, 0x20 knocks down…).
    pub blow: u32,
    /// `+0x08`: a projectile's lifetime (s); a cone's thickness.
    pub life: f32,
    /// `+0x0C`: sphere, projectile or ring radius; a cone's length.
    pub radius: f32,
    /// `+0x10`: a cone's nearest reach.
    pub min_range: f32,
    /// `+0x14`: turns the launch direction (radians).
    pub yaw: f32,
    /// `+0x18`: handed to the projectile (not traced).
    pub param: f32,
    /// `+0x1C`: tilts the launch direction (radians).
    pub pitch: f32,
    /// `+0x20`: where it starts, in the node's (or body's) space.
    pub offset: [f32; 3],
    /// `+0x2C`: damage, × the level's monster damage scale.
    pub damage: f32,
    /// `+0x30`, `+0x34`: launch speed range; anger picks within it.
    pub speed: [f32; 2],
    /// `+0x38`: projectile gravity.
    pub gravity: f32,
    /// `+0x3C`: trail lifetime.
    pub trail: f32,
    /// `+0x40`..`+0x46`: `SFXX` records: the projectile, its hit, trails.
    pub effects: [i16; 4],
    /// `+0x48`: random spread of the launch direction (radians).
    pub spread: f32,
}

/// `SFXX` (`0x50` bytes): an effect and/or a sound.
#[derive(Debug, Clone)]
pub struct CritterSound {
    /// `+0x00`: flags (`docs/critters.md`).
    pub flags: u32,
    /// `+0x04`: the next record started with this one (−1 none).
    pub next: i32,
    /// `+0x10`: effect name.
    pub effect: String,
    /// `+0x20`: sound name; `%c` is the realm letter.
    pub sound: String,
    /// `+0x30`: offset.
    pub offset: [f32; 3],
    /// `+0x3C`, `+0x40`: effect lifetime and size.
    pub life: f32,
    pub scale: f32,
}

#[derive(Debug, Clone)]
pub struct CritterFile {
    pub desc: CritterDesc,
    pub types: Vec<CritterType>,
    pub moves: Vec<CritterMove>,
    pub patterns: Vec<CritterPattern>,
    pub nodes: Vec<CritterNode>,
    pub damage: Vec<CritterDamage>,
    pub sounds: Vec<CritterSound>,
    /// Record count per tag, in [`RECORD_SIZES`] order.
    pub counts: [usize; 8],
    /// Every table's records, raw, in [`RECORD_SIZES`] order.
    pub tables: [Vec<Vec<u8>>; 8],
}

fn i16_at(r: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([r[at], r[at + 1]])
}

fn i32_at(r: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(r[at..at + 4].try_into().unwrap())
}

fn f32_at(r: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(r[at..at + 4].try_into().unwrap())
}

fn vec3_at(r: &[u8], at: usize) -> [f32; 3] {
    [f32_at(r, at), f32_at(r, at + 4), f32_at(r, at + 8)]
}

/// A name field: up to the first NUL; a field starting with a control
/// character (the tools left tabs in unused ones) is empty.
fn name_at(r: &[u8], at: usize, len: usize) -> String {
    let bytes = &r[at..at + len];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
    let s = &bytes[..end];
    if s.first().is_some_and(|&b| b < b' ') {
        return String::new();
    }
    String::from_utf8_lossy(s).into_owned()
}

/// A skeleton node name: the loader only looks one up when its first two
/// characters are set, so shorter ones are none.
fn node_name_at(r: &[u8], at: usize) -> String {
    let s = name_at(r, at, 0x10);
    if s.len() < 2 { String::new() } else { s }
}

impl CritterType {
    fn parse(r: &[u8]) -> Self {
        let span = |at: usize| Span { count: i16_at(r, at), first: i16_at(r, at + 2) };
        Self {
            name: name_at(r, 0, 0x20),
            look_nodes: [node_name_at(r, 0x20), node_name_at(r, 0x30)],
            effect_node: node_name_at(r, 0x40),
            desc: i16_at(r, 0x50),
            subtype: i16_at(r, 0x52),
            flags: i32_at(r, 0x5C) as u32,
            look_limits: std::array::from_fn(|i| f32_at(r, 0x60 + 4 * i)),
            height: f32_at(r, 0x78),
            radius: f32_at(r, 0x7C),
            target: Condition::parse(r, 0x80),
            home: vec3_at(r, 0xA0),
            leash: f32_at(r, 0xAC),
            hover: f32_at(r, 0xB0),
            aim_height: f32_at(r, 0xB4),
            armor: f32_at(r, 0xBC),
            center: vec3_at(r, 0xC0),
            max_turn: f32_at(r, 0xCC),
            key_offset: vec3_at(r, 0xD0),
            resist: i32_at(r, 0xE0) as u32,
            hit_points: f32_at(r, 0xE4),
            experience: f32_at(r, 0xE8),
            wake_distance: f32_at(r, 0xEC),
            hit_effects: [i16_at(r, 0xF4), i16_at(r, 0xF6)],
            moves: span(0x110),
            patterns: span(0x114),
            nodes: span(0x118),
            child: usize::try_from(i16_at(r, 0x11C)).ok(),
            parent: usize::try_from(i16_at(r, 0x11E)).ok(),
            raw: r.to_vec(),
        }
    }

    /// The home the type gives, if it gives one (`+0xA4` below 999).
    pub fn fixed_home(&self) -> Option<[f32; 3]> {
        (self.home[1] < 999.0).then_some(self.home)
    }
}

impl CritterMove {
    fn parse(r: &[u8]) -> Self {
        Self {
            kind: i32_at(r, 0),
            flags: i32_at(r, 4) as u32,
            priority: i32_at(r, 8),
            name: name_at(r, 0x10, 0x10),
            anim: name_at(r, 0x20, 0x10),
            node: node_name_at(r, 0x30),
            hit_frames: [i32_at(r, 0x40), i32_at(r, 0x44)],
            damage: [i16_at(r, 0x48), i16_at(r, 0x4A)],
            hit_ends: [i16_at(r, 0x50), i16_at(r, 0x52)],
            next: i16_at(r, 0x54),
            transition: i16_at(r, 0x56),
            sounds: [(i16_at(r, 0x58), i16_at(r, 0x5A)), (i16_at(r, 0x5C), i16_at(r, 0x5E))],
            condition: Condition::parse(r, 0x60),
            cooldown: f32_at(r, 0x80),
            speed: f32_at(r, 0x84),
            turn: f32_at(r, 0x88),
            hold: f32_at(r, 0x8C),
        }
    }

    pub fn is_attack(&self) -> bool {
        (kind::ATTACK_FIRST..kind::SKIP).contains(&self.kind)
    }

    pub fn is_movement(&self) -> bool {
        (kind::MOVE_FIRST..=kind::MOVE_LAST).contains(&self.kind)
    }
}

impl CritterPattern {
    fn parse(r: &[u8]) -> Self {
        Self {
            name: name_at(r, 0, 0x10),
            flags: i16_at(r, 0x10) as u16,
            cooldown: f32_at(r, 0x14),
            moves: std::array::from_fn(|i| i16_at(r, 0x20 + 2 * i)),
            condition: Condition::parse(r, 0x30),
        }
    }
}

impl CritterNode {
    fn parse(r: &[u8]) -> Self {
        Self {
            name: node_name_at(r, 0),
            flags: i16_at(r, 0x10) as u16,
            break_damage: i16_at(r, 0x12),
            model_flag: i16_at(r, 0x16),
            reach: f32_at(r, 0x18),
            weight: f32_at(r, 0x1C),
            offset: vec3_at(r, 0x20),
            radius: f32_at(r, 0x2C),
            model_node: node_name_at(r, 0x30),
            damage_scale: f32_at(r, 0x40),
            hit_points: f32_at(r, 0x44),
        }
    }
}

impl CritterDamage {
    fn parse(r: &[u8]) -> Self {
        Self {
            kind: i16_at(r, 0),
            flags: i16_at(r, 2) as u16,
            blow: i32_at(r, 4) as u32,
            life: f32_at(r, 8),
            radius: f32_at(r, 0xC),
            min_range: f32_at(r, 0x10),
            yaw: f32_at(r, 0x14),
            param: f32_at(r, 0x18),
            pitch: f32_at(r, 0x1C),
            offset: vec3_at(r, 0x20),
            damage: f32_at(r, 0x2C),
            speed: [f32_at(r, 0x30), f32_at(r, 0x34)],
            gravity: f32_at(r, 0x38),
            trail: f32_at(r, 0x3C),
            effects: std::array::from_fn(|i| i16_at(r, 0x40 + 2 * i)),
            spread: f32_at(r, 0x48),
        }
    }
}

impl CritterSound {
    fn parse(r: &[u8]) -> Self {
        Self {
            flags: i32_at(r, 0) as u32,
            next: i32_at(r, 4),
            effect: name_at(r, 0x10, 0x10),
            sound: name_at(r, 0x20, 0x10),
            offset: vec3_at(r, 0x30),
            life: f32_at(r, 0x3C),
            scale: f32_at(r, 0x40),
        }
    }
}

impl CritterFile {
    pub fn parse(data: &[u8]) -> Result<Self, CritterError> {
        let file = ChunkFile::parse(data)?;
        let mut tables: [Vec<Vec<u8>>; 8] = Default::default();
        for (i, (tag, size)) in RECORD_SIZES.iter().enumerate() {
            let chunk = file.get(tag).ok_or(CritterError::MissingChunk(tag))?;
            if chunk.count == 0 {
                continue;
            }
            let records = file.records(tag, *size).ok_or(CritterError::Truncated(tag))?;
            tables[i] = records.map(<[u8]>::to_vec).collect();
        }
        let table = |tag: &str| &tables[RECORD_SIZES.iter().position(|(t, _)| *t == tag).unwrap()];
        let types: Vec<CritterType> = table("TYPE").iter().map(|r| CritterType::parse(r)).collect();
        if types.is_empty() {
            return Err(CritterError::NoTypes);
        }
        let desc = table("DESC").first().ok_or(CritterError::NoDesc)?;
        let desc = CritterDesc { name: name_at(desc, 0, 0x10), prefix: name_at(desc, 0x10, 0x10), class: i16_at(desc, 0x20) };
        let moves = table("MOVE").iter().map(|r| CritterMove::parse(r)).collect();
        let patterns = table("PTRN").iter().map(|r| CritterPattern::parse(r)).collect();
        let nodes = table("NODE").iter().map(|r| CritterNode::parse(r)).collect();
        let damage = table("DAMG").iter().map(|r| CritterDamage::parse(r)).collect();
        let sounds = table("SFXX").iter().map(|r| CritterSound::parse(r)).collect();
        let counts = std::array::from_fn(|i| tables[i].len());
        Ok(Self { desc, types, moves, patterns, nodes, damage, sounds, counts, tables })
    }

    pub fn count(&self, tag: &str) -> usize {
        RECORD_SIZES.iter().position(|(t, _)| *t == tag).map_or(0, |i| self.counts[i])
    }

    /// A type's moves, in order (a move's index within its type is what
    /// `next`, patterns and the per-instance timers use).
    pub fn type_moves(&self, ty: usize) -> &[CritterMove] {
        let r = self.types[ty].moves.range();
        &self.moves[r.start.min(self.moves.len())..r.end.min(self.moves.len())]
    }

    pub fn type_patterns(&self, ty: usize) -> &[CritterPattern] {
        let r = self.types[ty].patterns.range();
        &self.patterns[r.start.min(self.patterns.len())..r.end.min(self.patterns.len())]
    }

    pub fn type_nodes(&self, ty: usize) -> &[CritterNode] {
        let r = self.types[ty].nodes.range();
        &self.nodes[r.start.min(self.nodes.len())..r.end.min(self.nodes.len())]
    }

    /// The atree a type animates with: the `DESC` prefix and the type's
    /// name (`GOLEM1`, `DRAGON`, `CHIMEAGLE`).
    pub fn atree_name(&self, ty: usize) -> String {
        format!("{}{}", self.desc.prefix, self.types[ty].name).to_ascii_uppercase()
    }
}

/// The critter file a level's enemy slot loads (the level loader's switch
/// on the slot's enemy type): the golem has fire (realm 6) and ice
/// (realm 9) variants, the gargoyle's comes from the realm enemy record's
/// name (`gar_eagl`), bosses have one each. `None` for other types.
pub fn file_for_enemy(enemy: i32, realm: u32, gargoyle: &str) -> Option<String> {
    let name = match enemy {
        0x1D => match realm {
            9 => "golemI".to_string(),
            6 => "golemF".to_string(),
            _ => "golem".to_string(),
        },
        0x20 => format!("gar_{gargoyle}"),
        0x21 => "general".into(),
        0x22 => "dragon".into(),
        0x23 => "chimera".into(),
        0x24 => "djinn".into(),
        0x25 => "drider".into(),
        0x26 => "pboss".into(),
        0x27 => "yeti".into(),
        0x28 => "wraith".into(),
        0x29 => "lich".into(),
        0x2A => "skorne1".into(),
        0x2B => "skorne2".into(),
        0x2C => "garm".into(),
        _ => return None,
    };
    Some(format!("{name}.wad"))
}

/// The folder a critter's models load from (`MONSTERS/…`): the golem and
/// general have one per realm (`golem/levelA`), a gargoyle's is
/// `gar_<kind>`, bosses' are named after the file.
pub fn model_folder(desc: &CritterDesc, realm_items: &str, gargoyle: &str) -> String {
    match desc.class {
        class::GARGOYLE => format!("{}_{}", desc.name, gargoyle),
        class::GOLEM | class::GENERAL => format!("{}/{}", desc.name, realm_items),
        _ => desc.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into())
            .into()
    }

    fn files() -> Vec<(std::path::PathBuf, CritterFile)> {
        let Ok(entries) = std::fs::read_dir(root().join("CRITTER")) else { return Vec::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .map(|p| {
                let c = CritterFile::parse(&std::fs::read(&p).unwrap()).unwrap_or_else(|e| panic!("{p:?}: {e}"));
                (p, c)
            })
            .collect()
    }

    /// Every critter file parses, and its types' spans tile the `MOVE`,
    /// `PTRN` and `NODE` tables exactly, in order, with children chained
    /// forward.
    #[test]
    fn every_real_critter_file_parses() {
        let files = files();
        if files.is_empty() {
            eprintln!("skipping: no CRITTER folder");
            return;
        }
        for (p, c) in &files {
            assert_eq!(c.count("DESC"), 1, "{p:?}");
            for (tag, pick) in [
                ("MOVE", (|t: &CritterType| t.moves) as fn(&CritterType) -> Span),
                ("PTRN", |t| t.patterns),
                ("NODE", |t| t.nodes),
            ] {
                let mut next = 0;
                for t in &c.types {
                    let s = pick(t);
                    if s.count > 0 {
                        assert_eq!(s.first as usize, next, "{p:?} {tag}");
                    }
                    next += s.count.max(0) as usize;
                }
                assert_eq!(next, c.count(tag), "{p:?} {tag}");
            }
            for (i, t) in c.types.iter().enumerate() {
                assert!(t.hit_points >= 100.0 && t.hit_points <= 10_000.0, "{p:?} {}", t.hit_points);
                if let Some(child) = t.child {
                    assert!(child > i && child < c.types.len(), "{p:?}");
                }
                if let Some(parent) = t.parent {
                    assert!(parent < i, "{p:?}");
                }
            }
        }
        eprintln!("{} critter files", files.len());
        assert!(files.len() >= 18);
    }

    /// The fields the runtime reads are in range: every class is one the
    /// update knows; every move's `next`, `DAMG` and `SFXX` index and every
    /// pattern step points inside its tables; every body has the moves the
    /// update asks for by kind (INIT, READY, DEATH).
    #[test]
    fn every_real_critter_reference_is_in_range() {
        for (p, c) in files() {
            assert!(
                [class::GOLEM, class::BOSS, class::GARGOYLE, class::GENERAL].contains(&c.desc.class),
                "{p:?} class {}",
                c.desc.class
            );
            assert!(!c.desc.prefix.is_empty(), "{p:?}");
            for ty in 0..c.types.len() {
                let moves = c.type_moves(ty);
                let n = moves.len() as i16;
                for m in moves {
                    assert!(m.next < n, "{p:?} {} next {}", m.name, m.next);
                    for d in m.damage {
                        assert!(d < c.damage.len() as i16, "{p:?} {} damage {d}", m.name);
                    }
                    for (s, _) in m.sounds {
                        assert!(s < c.sounds.len() as i16, "{p:?} {} sound {s}", m.name);
                    }
                    assert!(!m.anim.is_empty(), "{p:?} {} has no animation", m.name);
                }
                for pat in c.type_patterns(ty) {
                    for s in pat.moves {
                        assert!(s < n, "{p:?} pattern {} step {s}", pat.name);
                    }
                }
                // Parts only need READY; whole bodies need the moves the
                // update forces.
                let has = |k: i32| moves.iter().any(|m| m.kind == k);
                if c.types[ty].subtype >= 0 {
                    for k in [kind::INIT, kind::READY, kind::DEATH] {
                        assert!(has(k), "{p:?} type {ty} lacks move kind {k:#x}");
                    }
                }
            }
            for d in &c.damage {
                assert!((0..=9).contains(&d.kind), "{p:?} damage kind {}", d.kind);
                for e in d.effects {
                    assert!(e < c.sounds.len() as i16, "{p:?} effect {e}");
                }
            }
            for s in &c.sounds {
                assert!(s.next < c.sounds.len() as i32, "{p:?} sound chain {}", s.next);
            }
        }
    }

    /// Every body's atree exists in the folder the game loads it from, and
    /// every move's animation and node, and every `NODE`, names something
    /// in it. The golem and general are checked in every realm folder
    /// they have.
    #[test]
    fn every_real_critter_animation_resolves() {
        let root = root();
        let mut checked = 0;
        let mut missing = Vec::new();
        for (p, c) in files() {
            let file = p.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
            let folders: Vec<String> = match c.desc.class {
                class::GARGOYLE => {
                    let kind = file.trim_start_matches("gar_").trim_end_matches(".wad").to_string();
                    vec![model_folder(&c.desc, "", &kind)]
                }
                class::GOLEM | class::GENERAL => {
                    let dir = root.join("MONSTERS").join(c.desc.name.to_ascii_uppercase());
                    let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                    // The fire and ice golems are only loaded in their
                    // realms' levels.
                    let realm = |n: &str| match file.as_str() {
                        "golemf.wad" => n == "levelF",
                        "golemi.wad" => n == "levelI",
                        "golem.wad" => n != "levelF" && n != "levelI",
                        _ => true,
                    };
                    entries
                        .flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| n.starts_with("level") && realm(n))
                        .map(|n| model_folder(&c.desc, &n, ""))
                        .collect()
                }
                _ => vec![model_folder(&c.desc, "", "")],
            };
            for folder in folders {
                let path = root.join("MONSTERS").join(folder.to_ascii_uppercase()).join("ANIM.PS2");
                let path = if path.exists() {
                    path
                } else {
                    // The realm folders are lower-case on the disc.
                    let (a, b) = folder.split_once('/').unwrap_or((&folder, ""));
                    root.join("MONSTERS").join(a.to_ascii_uppercase()).join(b).join("ANIM.PS2")
                };
                let Ok(bytes) = std::fs::read(&path) else { panic!("{p:?}: no {path:?}") };
                let anim = crate::anim::AnimFile::parse(&bytes).unwrap();
                for ty in 0..c.types.len() {
                    let owner = c.types[ty].parent.unwrap_or(ty);
                    let name = c.atree_name(owner);
                    let tree = anim.atrees.iter().find(|a| a.name == name).unwrap_or_else(|| {
                        panic!("{p:?} {folder}: no atree {name} ({:?})", anim.atrees.iter().map(|a| &a.name).collect::<Vec<_>>())
                    });
                    for m in c.type_moves(ty) {
                        // A missing animation plays the first one (the game
                        // logs it).
                        if !tree.actions.iter().any(|a| a.name == m.anim) {
                            missing.push(format!("{folder}/{name} {}: action {}", m.name, m.anim));
                        }
                        // A node the atree lacks means the root (the eagle
                        // and lion gargoyles name the serpent's torso).
                        if !m.node.is_empty() && tree.node_index(&m.node).is_none() {
                            missing.push(format!("{folder}/{name} {}: {}", m.name, m.node));
                        }
                    }
                    for n in c.type_nodes(ty).iter().filter(|n| !n.name.is_empty()) {
                        if tree.node_index(&n.name).is_none() {
                            missing.push(format!("{folder}/{name} NODE {}", n.name));
                        }
                    }
                    checked += 1;
                }
            }
        }
        eprintln!("{checked} critter bodies resolve; move nodes that fall back to the root: {missing:?}");
        assert!(missing.len() <= 40, "{missing:?}");
    }

    /// The placed critters (statues) in every level, and which of them a
    /// wake trigger (flag 0x2000) stands within the game's reach (10 units,
    /// less the statue item's radius) of — the only way a placed statue
    /// comes alive.
    #[test]
    fn every_real_statue_and_its_wake_trigger() {
        use crate::population::{ItemClass, PlacementParams, Population};
        let Ok(entries) = std::fs::read_dir(root().join("LEVELS")) else { return };
        let (mut statues, mut woken) = (0, 0);
        let mut report = Vec::new();
        for level in entries.flatten().map(|e| e.path()) {
            let Ok(bytes) = std::fs::read(level.join("WORLDS.PS2")) else { continue };
            let Ok(pop) = Population::parse(&bytes) else { continue };
            let wakes: Vec<[f32; 3]> = pop
                .placements
                .iter()
                .filter(|p| {
                    let ty = pop.resolved_type(p);
                    matches!(p.params(ty.class), PlacementParams::Trigger { flags, .. } if flags & 0x2000 != 0)
                })
                .map(|p| p.position)
                .collect();
            for p in &pop.placements {
                let ty = pop.resolved_type(p);
                if ty.class != ItemClass::EnemyInfo || !matches!(ty.enemy(), Some(0x1D | 0x20 | 0x21)) {
                    continue;
                }
                statues += 1;
                let radius = 0.5 * ty.extent[0].max(ty.extent[1]);
                let near = wakes.iter().any(|w| {
                    ((w[0] - p.position[0]).powi(2) + (w[2] - p.position[2]).powi(2)).sqrt() - radius < 10.0
                });
                if near {
                    woken += 1;
                    report.push(format!("{} {} {:?}", level.file_name().unwrap().to_string_lossy(), ty.name, p.position));
                }
            }
        }
        eprintln!("{statues} placed critters, {woken} with a wake trigger: {report:?}");
    }
}
