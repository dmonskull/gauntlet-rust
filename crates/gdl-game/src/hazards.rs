//! What hurts the hero in the level itself (`docs/mechanics.md`): damage
//! tiles — spikes, flame vents, force fields, saw blades, tentacles —
//! cycling through OFF, ONA (rising), ON and ONB (going away) and hurting
//! whoever stands in them while they're out, and damaging walls, level
//! nodes flagged to hurt what runs into them.
//!
//! A tile that hurts the hero raises the hint to avoid dangerous objects.
//!
//! Stand-ins: an active phase lasts its animation (the game's own timing
//! for it isn't confirmed); damaging walls on nodes that hurt only while
//! animating (flag 0x2000000, which nothing animates yet) never hurt.

use bevy::prelude::*;
use gdl_formats::population::{ItemClass, PlacementParams};

use crate::audio::PlaySound;
use crate::hints::{Hint, ShowHint};
use crate::items::{self, LevelItems};
use crate::mechanics::LevelNodes;
use crate::monsters::MonsterLevel;
use crate::player::{Player, PlayerTick};
use crate::player_state::{DamagePlayer, PlayerState};
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

/// Type flags: hurts (1) and cycles its actions (4).
const TILE_HURTS: u16 = 0x1;
const TILE_CYCLES: u16 = 0x4;
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
struct Hazards {
    tiles: Vec<Tile>,
    /// The tile and phase that last hurt the hero: the game's guard lasts
    /// until that tile's phase is over.
    tile_guard: Option<(usize, u32)>,
    /// Game seconds before a wall can hurt the hero again.
    wall_guard: f32,
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
        if !p.active_for(1) {
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
    state: Option<Res<PlayerState>>,
    mut players: Query<&mut Player>,
    mut hurt: MessageWriter<DamagePlayer>,
    mut sounds: MessageWriter<PlaySound>,
    mut hints: MessageWriter<ShowHint>,
) {
    let (Some(mut h), Some(mut items), Some(state)) = (hazards, items, state) else { return };
    let h = &mut *h;
    let (damage_scale, time_scale) = level.as_ref().map_or((1.0, 1.0), |l| (l.tuning.hazard_damage, l.tuning.tile_time));
    let realm = items.realm();

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

    // Hurting: out (state 2 or 4) and standing in it.
    let Ok(mut p) = players.single_mut() else { return };
    if let Some((i, phase)) = h.tile_guard
        && h.tiles.get(i).is_some_and(|t| t.phase == phase)
    {
        return;
    }
    h.tile_guard = None;
    if !state.alive || levitating(&state) {
        return;
    }
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
            hurt.write(DamagePlayer { amount });
        }
        if let Some(name) = TILE_SOUNDS.get(realm).and_then(|row| row.get(t.subtype.clamp(0, 6) as usize))
            && !name.is_empty()
        {
            sounds.write(PlaySound((*name).into()));
        }
        hints.write(ShowHint(Hint::AvoidObjects));
        debug!("damage tile {} hurts the hero for {amount:.1}", t.placement);
        // Once per phase: until this one is over.
        h.tile_guard = Some((i, t.phase));
        break;
    }
}

/// The special powerup's levitation bit keeps the hero off damage tiles.
fn levitating(state: &PlayerState) -> bool {
    state.bits.special & crate::player_state::power::LEVITATE != 0
}

/// Node flags that make a wall hurt, and those that only hurt while the
/// node animates.
const HURTS: u32 = 0xF_0000;
const HURTS_WHILE_MOVING: u32 = 0x200_0000;
const NODE_MOVING: u32 = 0x800_0000;

fn walls(
    hazards: Option<ResMut<Hazards>>,
    nodes: Option<Res<LevelNodes>>,
    state: Option<Res<PlayerState>>,
    mut players: Query<&mut Player>,
    mut hurt: MessageWriter<DamagePlayer>,
) {
    let (Some(mut h), Some(nodes), Some(state)) = (hazards, nodes, state) else { return };
    h.wall_guard = (h.wall_guard - 1.0 / 30.0).max(0.0);
    let Ok(mut p) = players.single_mut() else { return };
    let Some((node, point)) = p.wall_hit else { return };
    if !state.alive || h.wall_guard > 0.0 {
        return;
    }
    // The node's flags OR'ed up its parents.
    let mut flags = 0;
    let mut n = Some(node);
    let mut steps = 0;
    while let Some(k) = n {
        flags |= nodes.nodes.get(k).map_or(0, |w| w.flags);
        n = nodes.parent.get(k).copied().flatten();
        steps += 1;
        if steps > nodes.nodes.len() {
            break;
        }
    }
    if flags & HURTS == 0 || (flags & HURTS_WHILE_MOVING != 0 && flags & NODE_MOVING == 0) {
        return;
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
        hurt.write(DamagePlayer { amount });
    }
    h.wall_guard = 1.0;
    debug!("wall node {node} hurts the hero for {amount:.1}");
}

#[cfg(test)]
mod tests {
    use super::*;

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
