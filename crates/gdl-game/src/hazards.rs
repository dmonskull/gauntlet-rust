//! What hurts the hero in the level itself (`docs/mechanics.md`): damage
//! tiles — spikes, flame vents, force fields, saw blades, tentacles —
//! cycling through OFF, ONA (rising), ON and ONB (going away) and hurting
//! whoever stands in them while they're out, and damaging walls, level
//! nodes flagged to hurt what runs into them.
//!
//! A tile that hurts the hero raises the hint to avoid dangerous objects.
//!
//! Stand-ins: an active phase lasts its animation (the game's own timing
//! for it isn't confirmed).

use bevy::prelude::*;
use gdl_formats::population::{ItemClass, PlacementParams};

use crate::audio::{CALL_VOLUME, PlaySoundAt};
use crate::hints::{Hint, ShowHint};
use crate::items::{self, LevelItems};
use crate::mechanics::{LevelNodes, Mechanics};
use crate::monsters::MonsterLevel;
use crate::player::{Player, PlayerTick};
use crate::party::{MAX_PLAYERS, Party};
use crate::player_state::{Cry, DamagePlayer, HurtHero, PlayerState};
use crate::population::LevelPopulation;

pub struct HazardsPlugin;

impl Plugin for HazardsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, setup.run_if(resource_exists_and_changed::<LevelPopulation>))
            .add_systems(FixedUpdate, (tiles, walls).after(PlayerTick));
    }
}

/// Video fields per 30 Hz tick.
const FIELDS: f32 = 2.0;

/// Damage-tile sounds by realm id (A–K = 1–11) and tile subtype.
const TILE_SOUNDS: [[&str; 7]; 12] = [
    ["", "", "", "", "", "", ""],
    ["S_SPIKEA", "", "S_FFIELDZAPA", "S_BUZZSAW", "S_TENTACLES", "S_TENTACLES", ""],
    ["", "S_FIREHOLE", "", "", "", "", ""],
    ["S_SPIKEC", "S_FIREHOLEC", "S_FFIELDZAPC", "", "", "", "S_SPIKEGATE"],
    ["S_LOGSPIKE", "S_FIREHOLED", "", "S_SWINGBLADE", "S_TENTACLESD", "S_TENTACLESD", ""],
    ["", "", "", "", "", "", ""],
    ["", "S_FIREHOLEF", "", "", "", "", ""],
    ["", "S_FIREHOLEG", "", "", "", "", ""],
    ["", "S_FIREHOLEH", "", "", "", "", ""],
    ["", "S_FIREHOLEI", "", "", "", "", ""],
    ["", "S_FIREHOLEJ", "", "", "", "", ""],
    ["", "S_FIREHOLEK", "", "", "", "", ""],
];

/// How a tile's blow is voiced: spikes (0), saws and blades (3) and the
/// first tentacles (4) make the hero scream, the rest groan.
fn tile_cry(subtype: i32) -> Cry {
    if matches!(subtype, 0 | 3 | 4) { Cry::Scream } else { Cry::Pain }
}

/// The tentacles' sounds play louder than their calls' own.
fn tile_volume(name: &str) -> u8 {
    if matches!(name, "S_TENTACLES" | "S_TENTACLESD") { TENTACLES_VOLUME } else { CALL_VOLUME }
}
const TENTACLES_VOLUME: u8 = 0xB4;

/// On the dragon's level the fire holes have their own sound.
fn tile_sound(name: &'static str, boss: i32) -> &'static str {
    if name == "S_FIREHOLE" && boss == DRAGON_BOSS { "S_FIREHOLE2" } else { name }
}
/// The dragon, as a level's boss type.
const DRAGON_BOSS: i32 = 0x22;

/// Type flags: hurts (1) and cycles its actions (4).
const TILE_HURTS: u16 = 0x1;
const TILE_CYCLES: u16 = 0x4;
/// A tile held off by the time stop waits this many fields once time
/// runs again.
const STOPPED_WAIT: f32 = 30.0;
/// Kind bits in a tile's value that push the hero out along the tile.
const TILE_PUSH: u32 = 0x30;
/// Added to every tile blow's flags.
const TILE_BLOW: u32 = 0x80;

struct Tile {
    placement: usize,
    subtype: i32,
    /// Hit points per blow, before the level's scale.
    damage: f32,
    /// The type's off time (× 2 fields; negative: random).
    off_time: i16,
    value: u32,
    flags: u16,
    /// Fields left in the current phase (the OFF wait).
    timer: f32,
    action: usize,
    /// Counts the phases (action starts), for the hurt guard.
    phase: u32,
    started: bool,
}

#[derive(Resource, Default)]
pub struct Hazards {
    tiles: Vec<Tile>,
    /// By slot: the tile and phase that last hurt the hero — the game's
    /// guard lasts until that tile's phase is over.
    tile_guard: [Option<(usize, u32)>; MAX_PLAYERS],
    /// By slot: game seconds before a wall can hurt the hero again.
    wall_guard: [f32; MAX_PLAYERS],
    rng: u32,
}

impl Hazards {
    /// The game's `random(n)`: 0..n (xorshift stand-in for its generator).
    fn random(&mut self, n: u32) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        if n == 0 { 0 } else { x % n }
    }

    /// Fields a tile waits off: the type's time × 2 × the level's scale; a
    /// negative time is random between |v| and 3 |v|.
    fn off_fields(&mut self, v: i16, scale: f32) -> f32 {
        let fields = if v < 0 {
            let n = (-2 * i32::from(v)) as u32;
            (n / 2 + self.random(n)) as f32
        } else {
            f32::from(v) * 2.0
        };
        fields * scale
    }
}

fn setup(mut commands: Commands, population: Res<LevelPopulation>) {
    let pop = &population.population;
    let mut h = Hazards { rng: 0x2545_F491, ..default() };
    for (placement, p) in pop.placements.iter().enumerate() {
        if !p.active_for(population.players) {
            continue;
        }
        let ty = pop.resolved_type(p);
        if ty.class != ItemClass::DamageTile {
            continue;
        }
        let (damage, off) = match p.params(ty.class) {
            PlacementParams::DamageTile { damage, off_time } => (damage, off_time),
            _ => (0, 0),
        };
        let type_damage = i16::from_le_bytes([ty.raw[0x40], ty.raw[0x41]]);
        let damage = f32::from(if damage != 0 { damage } else { type_damage });
        let off_time = if off != 0 { off.saturating_mul(-3) } else { i16::from_le_bytes([ty.raw[0x48], ty.raw[0x49]]) };
        let timer = 0.0;
        debug!("damage tile {placement} {} at {:?}: damage {damage}, off {off_time}", ty.name, p.position);
        h.tiles.push(Tile {
            placement,
            subtype: ty.subtype,
            damage,
            off_time,
            value: ty.value as u32,
            flags: ty.flags,
            timer,
            action: 0,
            phase: 0,
            started: false,
        });
    }
    info!("hazards: {} damage tiles", h.tiles.len());
    commands.insert_resource(h);
}

#[allow(clippy::too_many_arguments)]
fn tiles(
    hazards: Option<ResMut<Hazards>>,
    items: Option<ResMut<LevelItems>>,
    level: Option<Res<MonsterLevel>>,
    party: Res<Party>,
    mut players: Query<&mut Player>,
    mut hurt: MessageWriter<HurtHero>,
    mut sounds: MessageWriter<PlaySoundAt>,
    mut hints: MessageWriter<ShowHint>,
    stop: Res<crate::player_state::TimeStop>,
) {
    let (Some(mut h), Some(mut items)) = (hazards, items) else { return };
    let h = &mut *h;
    let (damage_scale, time_scale) = level.as_ref().map_or((1.0, 1.0), |l| (l.tuning.hazard_damage, l.tuning.tile_time));
    let realm = items.realm();

    // Time stopped: every cycling tile is held off, its wait at 30 fields.
    if stop.0 {
        for t in h.tiles.iter_mut().filter(|t| t.flags & TILE_CYCLES != 0) {
            if t.action != 0 || items.view(t.placement).is_some_and(|v| v.state != 0) {
                items.play(t.placement, 0);
                items.set_state(t.placement, 0);
            }
            t.started = true;
            t.action = 0;
            t.timer = STOPPED_WAIT;
        }
        return;
    }

    // Cycle: OFF waits its time, the other actions play through.
    for i in 0..h.tiles.len() {
        let (placement, cycles) = (h.tiles[i].placement, h.tiles[i].flags & TILE_CYCLES != 0);
        let Some(view) = items.view(placement) else { continue };
        if !cycles || view.actions == 0 {
            continue;
        }
        let actions = view.actions;
        let done = view.done;
        let t = &mut h.tiles[i];
        if !t.started {
            t.started = true;
            items.play(placement, 0);
            items.set_state(placement, 0);
            let v = t.off_time;
            h.tiles[i].timer = h.off_fields(v, time_scale);
            continue;
        }
        let next = if t.action == 0 {
            t.timer -= FIELDS;
            t.timer <= 0.0
        } else {
            done
        };
        if next {
            let action = (t.action + 1) % actions;
            t.action = action;
            t.phase += 1;
            trace!("damage tile {placement} action {action}");
            items.play(placement, action);
            items.set_state(placement, action);
            if action == 0 {
                let v = t.off_time;
                h.tiles[i].timer = h.off_fields(v, time_scale);
            }
        }
    }

    // Hurting: out (state 2 or 4) and standing in it — each hero.
    for mut p in &mut players {
        let slot = p.slot.min(MAX_PLAYERS - 1);
        let Some(state) = party.state(slot) else { continue };
        if let Some((i, phase)) = h.tile_guard[slot]
            && h.tiles.get(i).is_some_and(|t| t.phase == phase)
        {
            continue;
        }
        h.tile_guard[slot] = None;
        if !state.alive || levitating(state) {
            continue;
        }
        if let Some(guard) = tile_hurts(h, &items, &mut p, state, (damage_scale, realm, level.as_deref()), (&mut hurt, &mut sounds, &mut hints)) {
            h.tile_guard[slot] = Some(guard);
        }
    }
}

/// The first damage tile out under the hero hurts it: the guard to set
/// (the tile and its phase).
fn tile_hurts(
    h: &Hazards,
    items: &LevelItems,
    p: &mut Player,
    state: &PlayerState,
    (damage_scale, realm, level): (f32, usize, Option<&MonsterLevel>),
    (hurt, sounds, hints): (&mut MessageWriter<HurtHero>, &mut MessageWriter<PlaySoundAt>, &mut MessageWriter<ShowHint>),
) -> Option<(usize, u32)> {
    let feet = p.mover.position;
    for (i, t) in h.tiles.iter().enumerate() {
        if t.flags & TILE_HURTS == 0 || !(t.action == 2 || t.action == 4) {
            continue;
        }
        let Some(view) = items.view(t.placement) else { continue };
        if !view.live || items::contact(&view.shape, true, feet, feet, state.radius, state.half_height).is_none() {
            continue;
        }
        let flags = t.value | TILE_BLOW;
        let push = if t.value & TILE_PUSH != 0 {
            let z = view.shape.axes[1];
            -Vec3::new(z[0], 0.0, z[2])
        } else {
            Vec3::ZERO
        };
        let amount = p.take_blow(t.damage * damage_scale, flags, push);
        if amount != 0.0 {
            hurt.write(HurtHero { slot: p.slot, amount, kind: flags, cry: tile_cry(t.subtype) });
        }
        if let Some(name) = TILE_SOUNDS.get(realm).and_then(|row| row.get(t.subtype.clamp(0, 6) as usize))
            && !name.is_empty()
        {
            // Panned at the hero's feet.
            let boss = level.map_or(-1, |l| l.boss);
            sounds.write(PlaySoundAt::panned(tile_sound(name, boss), Vec3::from(feet), tile_volume(name)));
        }
        hints.write(ShowHint::to(p.slot, Hint::AvoidObjects));
        debug!("damage tile {} hurts player {} for {amount:.1}", t.placement, p.slot + 1);
        // Once per phase: until this one is over.
        return Some((i, t.phase));
    }
    None
}

/// The special powerup's levitation bit keeps the hero off damage tiles.
fn levitating(state: &PlayerState) -> bool {
    state.bits.special & crate::player_state::power::LEVITATE != 0
}

/// Node flags that make a wall hurt, and those that only hurt while the
/// node animates (the animated mode, `mechanics.rs`) — while it's
/// animating or a mover moves it.
const HURTS: u32 = 0xF_0000;
const HURTS_WHILE_MOVING: u32 = 0x200_0000;
const NODE_MOVING: u32 = 0x800_0000;

fn walls(
    hazards: Option<ResMut<Hazards>>,
    nodes: Option<Res<LevelNodes>>,
    mechanics: Option<Res<Mechanics>>,
    party: Res<Party>,
    mut players: Query<&mut Player>,
    mut hurt: MessageWriter<DamagePlayer>,
) {
    let (Some(mut h), Some(nodes)) = (hazards, nodes) else { return };
    for guard in &mut h.wall_guard {
        *guard = (*guard - 1.0 / 30.0).max(0.0);
    }
    for mut p in &mut players {
        let slot = p.slot.min(MAX_PLAYERS - 1);
        if party.state(slot).is_some_and(|s| s.alive) && h.wall_guard[slot] <= 0.0 && wall_hurts(&nodes, mechanics.as_deref(), &mut p, &mut hurt) {
            h.wall_guard[slot] = 1.0;
        }
    }
}

/// The wall the hero ran into this tick hurts it, if its node's flags say
/// so: whether it did.
fn wall_hurts(nodes: &LevelNodes, mechanics: Option<&Mechanics>, p: &mut Player, hurt: &mut MessageWriter<DamagePlayer>) -> bool {
    let Some((node, point)) = p.wall_hit else { return false };
    // The node's flags OR'ed up its parents, with what the game sets on
    // them as it runs.
    let mut flags = 0;
    let mut n = Some(node);
    let mut steps = 0;
    while let Some(k) = n {
        flags |= nodes.nodes.get(k).map_or(0, |w| w.flags) | mechanics.map_or(0, |m| m.node_flags(k));
        n = nodes.parent.get(k).copied().flatten();
        steps += 1;
        if steps > nodes.nodes.len() {
            break;
        }
    }
    if flags & HURTS == 0 || (flags & HURTS_WHILE_MOVING != 0 && flags & NODE_MOVING == 0) {
        return false;
    }
    let (damage, blow) = match flags & HURTS {
        0x3_0000 | 0x4_0000 | 0x5_0000 => (15.0, 0x20),
        0x2_0000 => (10.0, 0x10),
        _ => (5.0, 0),
    };
    let feet = Vec3::from(p.mover.position);
    let push = if blow != 0 {
        let d = feet - Vec3::from(point);
        Vec3::new(d.x, 0.0, d.z).normalize_or_zero()
    } else {
        Vec3::ZERO
    };
    let amount = p.take_blow(damage, blow, push);
    if amount != 0.0 {
        hurt.write(DamagePlayer { slot: p.slot, amount });
    }
    debug!("wall node {node} hurts player {} for {amount:.1}", p.slot + 1);
    true
}

/// The level's hazards as a sync point carries them (`resync.rs`): each
/// damage tile's place in its cycle and the heroes' guards, as the host
/// has them.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct HazardsSave {
    /// Per tile: timer, action, phase, started.
    tiles: Vec<(f32, usize, u32, bool)>,
    tile_guard: [Option<(usize, u32)>; MAX_PLAYERS],
    wall_guard: [f32; MAX_PLAYERS],
    rng: u32,
}

impl Hazards {
    /// What the machines compare online (`online.rs`).
    pub fn sync_hash(&self) -> u64 {
        use gdl_formats::detmath::sync_bits;
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for t in &self.tiles {
            (sync_bits(t.timer), t.action, t.phase, t.started).hash(&mut h);
        }
        (self.tile_guard, self.wall_guard.map(sync_bits), self.rng).hash(&mut h);
        h.finish()
    }

    pub(crate) fn save_synced(&self) -> HazardsSave {
        HazardsSave {
            tiles: self.tiles.iter().map(|t| (t.timer, t.action, t.phase, t.started)).collect(),
            tile_guard: self.tile_guard,
            wall_guard: self.wall_guard,
            rng: self.rng,
        }
    }

    /// Takes a sync point's hazards over (the same level: the tiles line
    /// up).
    pub(crate) fn load_synced(&mut self, save: &HazardsSave) {
        if save.tiles.len() != self.tiles.len() {
            warn!("sync point: the level's damage tiles don't line up; left as they are");
            return;
        }
        for (t, s) in self.tiles.iter_mut().zip(&save.tiles) {
            (t.timer, t.action, t.phase, t.started) = *s;
        }
        self.tile_guard = save.tile_guard;
        self.wall_guard = save.wall_guard;
        self.rng = save.rng;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_sounds_as_the_game_plays_them() {
        assert_eq!(tile_volume("S_TENTACLES"), 0xB4);
        assert_eq!(tile_volume("S_TENTACLESD"), 0xB4);
        assert_eq!(tile_volume("S_SPIKEA"), CALL_VOLUME);
        assert_eq!(tile_sound("S_FIREHOLE", DRAGON_BOSS), "S_FIREHOLE2");
        assert_eq!(tile_sound("S_FIREHOLE", -1), "S_FIREHOLE");
        assert_eq!(tile_sound("S_FIREHOLEC", DRAGON_BOSS), "S_FIREHOLEC");
        assert_eq!(tile_cry(0), Cry::Scream);
        assert_eq!(tile_cry(1), Cry::Pain);
        assert_eq!(tile_cry(4), Cry::Scream);
        assert_eq!(tile_cry(5), Cry::Pain);
    }

    #[test]
    fn off_times() {
        let mut h = Hazards { rng: 1, ..default() };
        assert_eq!(h.off_fields(10, 1.0), 20.0);
        assert_eq!(h.off_fields(10, 0.8), 16.0);
        for _ in 0..50 {
            let f = h.off_fields(-40, 1.0);
            assert!((40.0..120.0).contains(&f), "{f}");
        }
    }
}
