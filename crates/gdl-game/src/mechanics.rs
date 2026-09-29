//! The level's working parts (`docs/mechanics.md`): trigger pads and
//! switches, the world nodes they move — lifts, elevators, doors, trap
//! walls — bridges that appear and vanish, rotators, and carrying the hero
//! on whatever it stands on.
//!
//! Each tick, after the hero has moved: the hero's touches mark the
//! triggers (and the triggers chained after them), each trigger updates
//! its target node's state byte, then each mover heads for its on or off
//! height at 4 units a second (bridges show or hide instead). Moved nodes
//! pose their collision and their drawn group ([`MovingGroup`], split out
//! of the merged level meshes by `world.rs`), and a hero standing on one
//! is carried with it. A trigger coming on can shake the camera (flag
//! 0x1000), cut to its camera point (`play_camera::StartCut`) and wake a
//! statue (flag 0x2000, listed in `Mechanics::woken` for `critters.rs`).
//! Movers rumble while they move and clunk when they stop (one loop at a
//! time, as in the game); bridges sound as they open and close.
//!
//! Stand-ins: triggers run on
//! or off screen; quest triggers (flag 0x40),
//! subtype 1 rotators and the node flag 0x2000000 mode aren't done; only
//! players (not monsters) hold a mover still by standing on it.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use gdl_formats::collision::NodePose;
use gdl_formats::population::{LocatorKind, PlacementParams, Population};
use gdl_formats::WorldNode;

use crate::audio::{LoopSound, PlaySound};
use crate::items::{self, LevelItems};
use crate::level_material::LevelMaterial;
use crate::monsters::MonsterLevel;
use crate::play_camera::{Shake, StartCut};
use crate::player::{Player, PlayerTick};
use crate::player_state::PlayerState;
use crate::population::LevelPopulation;
use crate::world::LevelGround;

pub struct MechanicsPlugin;

impl Plugin for MechanicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, setup.run_if(resource_exists_and_changed::<LevelPopulation>))
            .add_systems(FixedUpdate, tick.after(PlayerTick))
            .add_systems(Update, pose_groups);
    }
}

/// Video fields per 30 Hz tick.
const FIELDS: f32 = 2.0;
/// Seconds per tick.
const DT: f32 = 1.0 / 30.0;
/// How fast movers travel, units per second.
const MOVER_SPEED: f32 = 4.0;
/// Within this of its goal a mover has arrived.
const ARRIVED: f32 = 0.001;
/// Fields a held lift waits before turning round.
const LIFT_WAIT: f32 = 120.0;
/// Alpha a vanishing bridge loses (or an appearing one gains) per field.
const FADE_STEP: i32 = 8;
/// From this alpha on a vanishing bridge is gone.
const FADED: i32 = 0xF8;

/// Trigger subtypes and their default flags (the low byte).
const BRIDGESW: i32 = 0x16;
const DOORSW: i32 = 0x17;
const BRIDGEPAD: i32 = 0x14;
const LIFTPAD: i32 = 0x1B;
const LIFTEND: i32 = 0x1D;
/// A switch that's hit rather than touched.
const HIT_SWITCH: i32 = 0x1F;

fn default_flags(subtype: i32, placed: u16) -> u16 {
    let low = match subtype {
        0x14 => 0x10,
        0x15 => 0x08,
        0x16 => 0x12,
        0x17 => 0x0A,
        0x19 => 0x804,
        0x1A => 0x02,
        0x1B => 0x80C,
        0x1C => 0x09,
        0x1D => 0x0A,
        _ => return placed | 8,
    };
    (placed & 0xFF00) | low
}

// Trigger flags.
const OFF_SWITCH: u16 = 0x1;
const SWITCH: u16 = 0x2;
const LIFT: u16 = 0x4;
const QUEST: u16 = 0x40;
const KEEP_TOUCHES: u16 = 0xC0;
const STAND_ON_TARGET: u16 = 0x100;
const CHAINED_TO: u16 = 0x200;
const ALL_PLAYERS: u16 = 0x400;
const LIFT_DELAY: u16 = 0x800;
/// Coming on wakes the nearest placed statue monster (`critters.rs`).
const WAKES: u16 = 0x2000;
/// Shakes the camera when it comes on.
const SHAKES: u16 = 0x1000;

// Mover kind flags (the trigger flags' low byte) and state bits.
const MOVES_CARRYING: u8 = 0x8;
const BRIDGE: u8 = 0x10;
const RETURNS: u8 = 0x20;
const KEEPS_STATE: u8 = 0x47;
const PLAYERS: u8 = 0xF;
const MOVING: u8 = 0x10;
const ON: u8 = 0x20;

/// The world file's scene graph, for what moves nodes.
#[derive(Resource)]
pub struct LevelNodes {
    pub nodes: Vec<WorldNode>,
    pub parent: Vec<Option<usize>>,
    /// Each node's world position at rest (its origin).
    pub origin: Vec<[f32; 3]>,
}

impl LevelNodes {
    pub fn new(nodes: Vec<WorldNode>) -> Self {
        let mut parent = vec![None; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            let mut c = n.first_child;
            while let Some(k) = c {
                if k >= nodes.len() || parent[k].is_some() {
                    break;
                }
                parent[k] = Some(i);
                c = nodes[k].next_sibling;
            }
        }
        let origin = (0..nodes.len())
            .map(|i| {
                let mut at = [0.0; 3];
                let mut n = Some(i);
                let mut steps = 0;
                while let Some(k) = n {
                    let p = nodes[k].local_position;
                    at = [at[0] + p[0], at[1] + p[1], at[2] + p[2]];
                    n = parent[k];
                    steps += 1;
                    if steps > nodes.len() {
                        break;
                    }
                }
                at
            })
            .collect();
        Self { nodes, parent, origin }
    }

    /// The nearest of `roots` at or above `node`.
    pub fn group_of(&self, node: usize, roots: &HashSet<usize>) -> Option<usize> {
        let mut n = Some(node);
        let mut steps = 0;
        while let Some(k) = n {
            if roots.contains(&k) {
                return Some(k);
            }
            n = self.parent.get(k).copied().flatten();
            steps += 1;
            if steps > self.nodes.len() {
                break;
            }
        }
        None
    }

    fn depth(&self, node: usize) -> usize {
        let mut d = 0;
        let mut n = self.parent.get(node).copied().flatten();
        while let Some(k) = n {
            d += 1;
            n = self.parent[k];
            if d > self.nodes.len() {
                break;
            }
        }
        d
    }
}

/// The nodes something moves: trigger targets and rotators' nodes.
pub fn moving_roots(population: &Population) -> HashSet<usize> {
    population
        .placements
        .iter()
        .filter(|p| p.active_for(1))
        .filter_map(|p| match p.params(population.resolved_type(p).class) {
            PlacementParams::Trigger { target, .. } | PlacementParams::Rotator { target, .. } => target,
            _ => None,
        })
        .collect()
}

/// A group of level meshes drawn at a moving node's pose.
#[derive(Component)]
pub struct MovingGroup {
    root: usize,
    /// How far faded out it's drawn (bridges).
    fade: f32,
}

impl MovingGroup {
    pub fn new(root: usize) -> Self {
        Self { root, fade: 0.0 }
    }
}

struct Trigger {
    placement: usize,
    subtype: i32,
    target: Option<usize>,
    flags: u16,
    /// Touch radius (0: the type's shape).
    radius: f32,
    id: u8,
    next: u8,
    /// The trigger after it in its chain.
    chain: Option<usize>,
    /// Players touching it (bit per player).
    touches: u8,
    /// Fields.
    timer: f32,
    /// What it shows: 0 off, 2 on.
    action: u8,
    /// The pad's own animation is heading for this.
    shown: u8,
    /// Rides its target (lift pads): its touch centre at rest.
    rides: Option<[f32; 3]>,
    /// The camera point (locator index) whose index is this trigger's id:
    /// shown when it comes on.
    cut: Option<usize>,
}

struct Mover {
    node: usize,
    kind: i32,
    flags: u8,
    off: f32,
    on: f32,
    offset: f32,
    state: u8,
    /// The state at the last update, for its sounds.
    previous: u8,
    /// Its sound: 0–5 a loop while it moves (`MOVER_LOOPS`), 11 a one-shot
    /// on arriving on, above 10 a one-shot when it starts or stops; −1 none.
    sound: i8,
    /// How faded out a bridge is (0 shown, 255 gone).
    alpha: i32,
}

/// Mover loop sets (`S_ELV<set><realm>` while moving, `S_ELV<set>STP<realm>`
/// when it stops, `B` added on boss levels; `*` marks a plain name,
/// `S_<name>ROTATE` / `S_<name>STOP`).
const MOVER_LOOPS: [&str; 6] = ["MET", "ROPE", "CHAIN", "ICE", "STONE", "*ROCK"];

/// Mover one-shots by sound − 10 (rows 1–4) and realm id.
const MOVER_SHOTS: [[&str; 12]; 4] = [
    ["", "S_TRAPA", "", "S_TRAPC", "S_TRAPD", "S_TRAPE", "S_TRAPF", "S_TRAPG", "", "S_TRAPI", "S_TRAPJ", "S_TRAPK"],
    ["", "", "", "S_QUAKEC", "", "", "", "", "", "", "", "S_ELVCNNK"],
    ["", "S_BRIDOPA", "", "S_BRIDOPC", "S_BRIDOPD", "", "", "", "S_BRIDOPH", "S_BRIDOPI", "", ""],
    ["", "S_BRIDCLA", "", "S_BRIDCLC", "S_BRIDCLD", "", "", "", "S_BRIDCLH", "S_BRIDCLI", "", ""],
];

/// A mover's loop and stop sounds in realm `letter` (boss levels use their
/// own recordings).
fn mover_loop(sound: i8, letter: char, boss_level: bool) -> Option<(String, String)> {
    let set = MOVER_LOOPS.get(usize::try_from(sound).ok()?)?;
    let letter = if letter == 'T' { 'G' } else { letter };
    let b = if boss_level { "B" } else { "" };
    Some(match set.strip_prefix('*') {
        Some(name) => (format!("S_{name}ROTATE"), format!("S_{name}STOP")),
        None => (format!("S_ELV{set}{letter}{b}"), format!("S_ELV{set}STP{letter}{b}")),
    })
}

struct Rotator {
    placement: usize,
    node: usize,
    subtype: i32,
    /// Radians per field.
    angle: f32,
    limit: f32,
    total: f32,
    touched: bool,
    done: bool,
}

/// The current level's triggers, movers and rotators.
#[derive(Resource, Default)]
pub struct Mechanics {
    triggers: Vec<Trigger>,
    movers: Vec<Mover>,
    mover_of: HashMap<usize, usize>,
    rotators: Vec<Rotator>,
    /// Moving roots, parents before children.
    roots: Vec<usize>,
    root_set: HashSet<usize>,
    /// Each root's world pose now and a tick ago.
    poses: HashMap<usize, (NodePose, NodePose)>,
    /// Nodes each root poses (its subtree, less deeper roots').
    members: HashMap<usize, Vec<usize>>,
    /// Bridges hidden now.
    hidden: HashSet<usize>,
    /// Bridges part faded (0 shown … 1 gone).
    fades: HashMap<usize, f32>,
    /// Placements of wake triggers (flag 0x2000) that came on since the
    /// critter update last took them.
    pub woken: Vec<usize>,
}

fn setup(mut commands: Commands, population: Res<LevelPopulation>, nodes: Option<Res<LevelNodes>>) {
    let Some(nodes) = nodes else { return };
    let pop = &population.population;
    let mut m = Mechanics::default();
    for (placement, p) in pop.placements.iter().enumerate() {
        if !p.active_for(1) {
            continue;
        }
        let ty = pop.resolved_type(p);
        match p.params(ty.class) {
            PlacementParams::Trigger { target, flags, radius, sound, id, next, off, on } => {
                let target = target.filter(|&t| t < nodes.nodes.len());
                let flags = default_flags(ty.subtype, flags);
                if let Some(node) = target {
                    let kind = ty.subtype;
                    match m.mover_of.get(&node) {
                        Some(&i) => {
                            let mv = &mut m.movers[i];
                            if (LIFTPAD..=LIFTEND).contains(&mv.kind) && (LIFTPAD..=LIFTEND).contains(&kind) {
                                mv.kind = LIFTPAD;
                            }
                        }
                        None => {
                            let (off, on) = (0.1 * f32::from(off), 0.1 * f32::from(on));
                            m.mover_of.insert(node, m.movers.len());
                            m.movers.push(Mover {
                                node,
                                kind,
                                flags: flags as u8,
                                off,
                                on,
                                offset: off,
                                state: 0,
                                previous: 0,
                                sound,
                                alpha: 0,
                            });
                        }
                    }
                }
                debug!(
                    "trigger {placement} {:#x} flags {flags:#x} at {:?} -> node {target:?} ({:?}) heights {off}/{on} id {id} next {next}",
                    ty.subtype,
                    p.position,
                    target.map(|t| nodes.nodes[t].name.as_str())
                );
                let radius = match radius {
                    0 => 0.0,
                    0xFF => 0.01,
                    r => 0.5 * f32::from(r),
                };
                m.triggers.push(Trigger {
                    placement,
                    subtype: ty.subtype,
                    target,
                    flags,
                    radius,
                    id,
                    next,
                    chain: None,
                    touches: 0,
                    timer: 0.0,
                    action: 0,
                    shown: 0,
                    rides: None,
                    cut: (id != 0)
                        .then(|| pop.locators.iter().position(|l| l.kind == LocatorKind::Transmitter(9) && l.index == i16::from(id)))
                        .flatten(),
                });
            }
            PlacementParams::Rotator { target: Some(node), angle, limit } if node < nodes.nodes.len() => {
                m.rotators.push(Rotator {
                    placement,
                    node,
                    subtype: ty.subtype,
                    angle,
                    limit,
                    total: 0.0,
                    touched: false,
                    done: false,
                });
            }
            _ => {}
        }
    }
    // Chains: each trigger with a next id leads to the one with that id.
    for i in 0..m.triggers.len() {
        let next = m.triggers[i].next;
        if next == 0 {
            continue;
        }
        let found = (0..m.triggers.len()).find(|&k| k != i && m.triggers[k].id == next && m.triggers[k].flags & QUEST == 0);
        if let Some(k) = found {
            m.triggers[i].chain = Some(k);
            m.triggers[k].flags |= CHAINED_TO;
        }
    }
    let mut roots: Vec<usize> = m.movers.iter().map(|mv| mv.node).chain(m.rotators.iter().map(|r| r.node)).collect();
    roots.sort_by_key(|&r| (nodes.depth(r), r));
    roots.dedup();
    m.root_set = roots.iter().copied().collect();
    for n in 0..nodes.nodes.len() {
        if let Some(root) = nodes.group_of(n, &m.root_set) {
            m.members.entry(root).or_default().push(n);
        }
    }
    for &r in &roots {
        m.poses.insert(r, (NodePose::REST, NodePose::REST));
    }
    m.roots = roots;
    info!(
        "mechanics: {} triggers, {} movers, {} rotators, {} chained",
        m.triggers.len(),
        m.movers.len(),
        m.rotators.len(),
        m.triggers.iter().filter(|t| t.chain.is_some()).count()
    );
    // Movers that start off their rest height.
    let start: Vec<(usize, f32)> = m.movers.iter().filter(|mv| mv.offset != 0.0).map(|mv| (mv.node, mv.offset)).collect();
    for (node, offset) in start {
        let pose = NodePose::translation([0.0, offset, 0.0]);
        m.poses.insert(node, (pose, pose));
    }
    commands.insert_resource(m);
}

#[allow(clippy::too_many_arguments)]
fn tick(
    mechanics: Option<ResMut<Mechanics>>,
    nodes: Option<Res<LevelNodes>>,
    items: Option<ResMut<LevelItems>>,
    ground: Option<ResMut<LevelGround>>,
    state: Option<Res<PlayerState>>,
    mut players: Query<&mut Player>,
    mut models: Query<&mut Transform, Without<Player>>,
    mut cuts: MessageWriter<StartCut>,
    mut shakes: MessageWriter<Shake>,
    mut sounds: MessageWriter<PlaySound>,
    mut loops: MessageWriter<LoopSound>,
    level: Option<Res<MonsterLevel>>,
) {
    let (Some(mut mech), Some(nodes), Some(mut items), Some(mut ground)) = (mechanics, nodes, items, ground) else {
        return;
    };
    let mech = &mut *mech;
    let alive = state.as_ref().is_none_or(|s| s.alive);
    let mut player = players.single_mut().ok();
    let standing = player.as_ref().and_then(|p| p.ground.node);
    let (feet, radius, half) = match (&player, &state) {
        (Some(p), Some(s)) => (Some(p.mover.position), s.radius, s.half_height),
        _ => (None, 0.0, 0.0),
    };

    // Touches.
    if let (Some(feet), true) = (feet, alive) {
        for i in 0..mech.triggers.len() {
            let t = &mech.triggers[i];
            if t.flags & CHAINED_TO != 0 || t.subtype == HIT_SWITCH {
                continue;
            }
            let Some(view) = items.view(t.placement) else { continue };
            if !view.live {
                continue;
            }
            let mut shape = view.shape;
            if t.radius > 0.0 {
                shape.kind = 1;
                shape.radius = t.radius;
            } else if t.subtype == LIFTPAD {
                shape.radius *= 2.0;
            }
            if items::contact(&shape, true, feet, feet, radius, half).is_none() {
                continue;
            }
            let quest = t.flags & QUEST != 0;
            let mut k = Some(i);
            while let Some(j) = k {
                mech.triggers[j].touches |= 1;
                k = if quest { None } else { mech.triggers[j].chain };
                if k == Some(i) {
                    break;
                }
            }
        }
        for r in &mut mech.rotators {
            if r.subtype != 2 || r.touched {
                continue;
            }
            if let Some(view) = items.view(r.placement)
                && items::contact(&view.shape, true, feet, feet, radius, half).is_some()
            {
                r.touched = true;
            }
        }
    }

    // Triggers.
    let on_target = |target: usize| standing.is_some_and(|n| n == target || nodes.parent.get(n).copied().flatten() == Some(target));
    for i in 0..mech.triggers.len() {
        let (flags, target) = (mech.triggers[i].flags, mech.triggers[i].target);
        let t = &mut mech.triggers[i];
        if t.timer > 0.0 {
            t.timer -= FIELDS;
        }
        let mut m = t.touches;
        if m != 0 && flags & STAND_ON_TARGET != 0 {
            let standing_bits = if target.is_some_and(on_target) { 1 } else { 0 };
            t.touches &= 0xF0 | standing_bits;
            m = t.touches;
        }
        if flags & ALL_PLAYERS != 0 && m != 1 {
            t.touches = 0;
            m = 0;
        }
        let before = t.action;
        let mover = target.and_then(|n| mech.mover_of.get(&n).copied());
        match mover {
            None => {
                if flags & LIFT == 0 {
                    if m != 0 {
                        t.action = 2;
                    }
                } else {
                    t.action = if m != 0 { 2 } else { 0 };
                }
            }
            Some(k) => {
                let st = &mut mech.movers[k].state;
                let t = &mut mech.triggers[i];
                if flags & OFF_SWITCH != 0 {
                    if m == 0 {
                        t.action = if *st & ON == 0 { 2 } else { 0 };
                    } else {
                        if *st & 0xF0 == ON {
                            *st &= PLAYERS;
                        }
                        t.action = 2;
                    }
                } else if flags & SWITCH != 0 {
                    if m == 0 {
                        t.action = if *st & ON != 0 { 2 } else { 0 };
                    } else {
                        if *st & 0xF0 == 0 {
                            *st = (*st & PLAYERS) | ON;
                        }
                        t.action = 2;
                    }
                    if t.subtype == DOORSW {
                        // The game shows hint 5 here; hints live in items.rs.
                    }
                } else if flags & LIFT != 0 {
                    if m == 0 {
                        t.action = 0;
                        if flags & LIFT_DELAY != 0 {
                            // (players − 1) × 60 fields; one player.
                            t.timer = 0.0;
                        }
                    } else {
                        let lo = *st & PLAYERS;
                        if *st & MOVING == 0 {
                            if lo < lo | m {
                                *st = (*st & 0xF0) | m;
                            }
                            if t.timer < 1.0 {
                                *st ^= ON;
                                *st = (*st & 0xF0) | m;
                                t.timer = LIFT_WAIT;
                            }
                        } else {
                            t.timer = LIFT_WAIT;
                        }
                        t.action = 2;
                    }
                } else if m == 0 {
                    t.action = if *st & ON != 0 { 2 } else { 0 };
                } else {
                    *st = m | ON;
                    t.action = 2;
                }
            }
        }
        let t = &mut mech.triggers[i];
        if t.action != 0 && before == 0 {
            debug!("trigger {} (subtype {:#x}) on{}", t.placement, t.subtype, if t.cut.is_some() { ", with a camera cut" } else { "" });
            // A rumble.
            if flags & SHAKES != 0 {
                shakes.write(Shake { amplitude: 0.1, what: 0, delay: 0.0, fields: 180.0, priority: 100 });
            }
            // Show what it did from its camera point.
            if m != 0
                && let Some(locator) = t.cut
            {
                cuts.write(StartCut { locator, node: target });
            }
            if flags & WAKES != 0 {
                let placement = t.placement;
                mech.woken.push(placement);
            }
        }
        if flags & KEEP_TOUCHES == 0 {
            t.touches = if t.touches & 0xF == 0 { 0 } else { t.touches & 0xF0 };
        }
    }

    // The pads' own animations: OFF, ONA (going on), ON, OFFA (going off).
    for t in &mut mech.triggers {
        let Some(view) = items.view(t.placement) else { continue };
        if view.actions < 4 {
            continue;
        }
        if t.action != t.shown {
            t.shown = t.action;
            items.play(t.placement, if t.action == 2 { 1 } else { 3 });
        } else if view.done && (view.action == 1 || view.action == 3) {
            items.play(t.placement, t.shown as usize);
            items.set_state(t.placement, t.shown as usize);
        }
    }

    // Movers.
    // Only this resource holds the level's collision, so it can be changed
    // in place.
    let mut collision = std::sync::Arc::get_mut(&mut ground.0);
    let mut hidden = HashSet::new();
    let mut fades = HashMap::new();
    let (letter, boss_level) = level.as_ref().map_or(('A', false), |l| (l.realm, l.boss >= 0));
    let realm = items.realm();
    let mut looping = None;
    for mv in &mut mech.movers {
        let carrying = standing == Some(mv.node);
        let mut st = mv.state;
        // Sounds, from this state against the last update's.
        let prev = mv.previous;
        mv.previous = st;
        if mv.sound >= 0 {
            let shot = |row: i8| MOVER_SHOTS.get((row - 1) as usize).and_then(|r| r.get(realm)).filter(|n| !n.is_empty());
            if mv.kind == BRIDGEPAD || mv.kind == BRIDGESW {
                if (st ^ prev) & ON != 0 {
                    let row = if st & ON == 0 { 4 } else { 3 };
                    if let Some(name) = shot(row) {
                        sounds.write(PlaySound((*name).into()));
                    }
                }
            } else if mv.sound < 10 {
                if let Some((run, stop)) = mover_loop(mv.sound, letter, boss_level) {
                    if st & MOVING != 0 {
                        looping.get_or_insert(run);
                    } else if prev & MOVING != 0 {
                        sounds.write(PlaySound(stop));
                    }
                }
            } else if mv.sound == 11 {
                if st & ON != 0 && prev & ON == 0 && let Some(name) = shot(1) {
                    sounds.write(PlaySound((*name).into()));
                }
            } else if (st ^ prev) & MOVING != 0 && let Some(name) = shot(mv.sound - 10) {
                sounds.write(PlaySound((*name).into()));
            }
        }
        let mut disable = 0u8;
        let moving;
        if mv.flags & BRIDGE != 0 {
            let hide = if mv.flags & RETURNS == 0 { st & PLAYERS == 0 } else { st & (PLAYERS | ON) != 0 };
            disable = if hide { 0xFF } else { 0 };
            let before = mv.alpha;
            mv.alpha = if hide { (mv.alpha + FADE_STEP * FIELDS as i32).min(0xFF) } else { (mv.alpha - FADE_STEP * FIELDS as i32).max(0) };
            moving = mv.alpha != before && mv.alpha < FADED;
            if mv.alpha >= FADED {
                hidden.insert(mv.node);
            }
            fades.insert(mv.node, mv.alpha as f32 / 255.0);
        } else if mv.flags & MOVES_CARRYING != 0 || !carrying {
            let goal = if st & ON != 0 { mv.on } else { mv.off };
            let mut d = goal - mv.offset;
            let step = MOVER_SPEED * DT;
            let mut go = true;
            if d <= ARRIVED {
                if d >= -ARRIVED {
                    if mv.flags & RETURNS == 0 {
                        go = false;
                    } else {
                        st ^= ON;
                    }
                } else if d < -step {
                    d = -step;
                }
            } else if step < d {
                d = step;
            }
            if go {
                mv.offset += d;
            } else if mv.state & MOVING != 0 {
                debug!("mover {} arrived at {:.2}", mv.node, mv.offset);
            }
            moving = go;
        } else {
            moving = false;
        }
        if moving {
            if mv.flags & MOVES_CARRYING == 0 {
                disable = 1;
            }
            st |= MOVING;
        } else {
            st &= !MOVING;
        }
        if mv.flags & KEEPS_STATE == 0 && (mv.flags & BRIDGE == 0 || !carrying) {
            st = 0;
        }
        mv.state = st;
        if let Some(c) = collision.as_mut() {
            c.set_disable(mv.node, disable);
        }
    }
    mech.hidden = hidden;
    mech.fades = fades;
    loops.write(LoopSound { key: "mover", name: looping });

    // Rotators; a touched one grinds round (its realm's sound) until it
    // reaches its limit.
    let rotator_sound = match realm {
        1 => Some(("S_ROCKROTATE", "S_ROCKSTOP")),
        9 => Some(("S_METLROTATE", "S_METLROTATESTO")),
        _ => None,
    };
    let mut grinding = false;
    for r in &mut mech.rotators {
        match r.subtype {
            0 => r.total += r.angle * FIELDS,
            2 if r.touched && !r.done => {
                r.total += r.angle * FIELDS;
                grinding = true;
                if r.total.abs() >= r.limit.abs() {
                    r.total = r.limit.abs().copysign(r.total);
                    r.done = true;
                    if let Some((_, stop)) = rotator_sound {
                        sounds.write(PlaySound(stop.into()));
                    }
                }
            }
            _ => {}
        }
    }
    let grind = rotator_sound.filter(|_| grinding).map(|(run, _)| run.to_string());
    loops.write(LoopSound { key: "rotator", name: grind });

    // World poses, parents first: a node's own move, then its parent's.
    let mut world: HashMap<usize, NodePose> = HashMap::new();
    for &root in &mech.roots {
        let mut local = NodePose::REST;
        if let Some(&k) = mech.mover_of.get(&root) {
            local = NodePose::translation([0.0, mech.movers[k].offset, 0.0]);
        }
        if let Some(r) = mech.rotators.iter().find(|r| r.node == root) {
            local = local.then(&NodePose::turn_about(nodes.origin[root], r.total));
        }
        let above = nodes.parent[root].and_then(|p| nodes.group_of(p, &mech.root_set)).and_then(|p| world.get(&p).copied());
        let pose = match above {
            Some(parent) => local.then(&parent),
            None => local,
        };
        world.insert(root, pose);
    }
    for (&root, &pose) in &world {
        let entry = mech.poses.entry(root).or_insert((pose, pose));
        *entry = (entry.1, pose);
    }
    if let Some(c) = collision {
        for (root, members) in &mech.members {
            let pose = world[root];
            for &n in members {
                c.set_pose(n, pose);
            }
        }
    }

    // Lift pads ride their lift.
    for t in &mut mech.triggers {
        let Some(target) = t.target else { continue };
        if t.subtype != LIFTPAD && t.flags & STAND_ON_TARGET == 0 {
            continue;
        }
        let Some(root) = nodes.group_of(target, &mech.root_set) else { continue };
        let Some(pose) = world.get(&root) else { continue };
        let rest = match t.rides {
            Some(r) => r,
            None => {
                let Some(view) = items.view(t.placement) else { continue };
                t.rides = Some(view.shape.centre);
                view.shape.centre
            }
        };
        if let Some((moved, Some(model))) = items.move_centre(t.placement, pose.apply(rest))
            && moved != [0.0; 3]
            && let Ok(mut transform) = models.get_mut(model)
        {
            transform.translation += Vec3::from(moved);
        }
    }

    // Carry the hero with what it stands on.
    if let (Some(p), Some(node)) = (player.as_mut(), standing)
        && let Some(root) = nodes.group_of(node, &mech.root_set)
        && let Some(&(before, now)) = mech.poses.get(&root)
        && before != now
    {
        let delta = before.delta_to(&now);
        let old = p.mover.position;
        let new = delta.apply(old);
        p.mover.position = new;
        p.ground.floor += new[1] - old[1];
        // The turn about the vertical: where the X axis went.
        let x = delta.apply_vector([1.0, 0.0, 0.0]);
        p.mover.facing -= x[2].atan2(x[0]);
    }
}

/// Puts each moving group where its node is, between the last two ticks,
/// fades bridges in and out (8 alpha steps a field) and hides vanished
/// ones.
fn pose_groups(
    time: Res<Time<Fixed>>,
    mechanics: Option<Res<Mechanics>>,
    mut groups: Query<(&mut MovingGroup, &mut Transform, &mut Visibility, &Children)>,
    parts: Query<&MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
) {
    let Some(mech) = mechanics else { return };
    let f = time.overstep_fraction();
    for (mut g, mut transform, mut visibility, children) in &mut groups {
        let Some(&(before, now)) = mech.poses.get(&g.root) else { continue };
        let a = to_transform(&before);
        let b = to_transform(&now);
        transform.translation = a.translation.lerp(b.translation, f);
        transform.rotation = a.rotation.slerp(b.rotation, f);
        let want = if mech.hidden.contains(&g.root) { Visibility::Hidden } else { Visibility::Inherited };
        visibility.set_if_neq(want);
        let fade = mech.fades.get(&g.root).copied().unwrap_or(0.0);
        if fade != g.fade {
            g.fade = fade;
            for c in children {
                let Ok(handle) = parts.get(*c) else { continue };
                let Some(m) = materials.get_mut(&handle.0) else { continue };
                m.uv_offset.w = fade;
                // Opaque parts blend while they fade.
                if fade > 0.0 && matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                    m.alpha_mode = AlphaMode::Blend;
                }
            }
        }
    }
}

fn to_transform(p: &NodePose) -> Transform {
    let m = &p.rotation;
    let rotation = Mat3::from_cols(Vec3::new(m[0], m[3], m[6]), Vec3::new(m[1], m[4], m[7]), Vec3::new(m[2], m[5], m[8]));
    Transform { translation: Vec3::from(p.translation), rotation: Quat::from_mat3(&rotation), scale: Vec3::ONE }
}

impl Mechanics {
    /// Whether the mover on `node` is moving (or a bridge fading).
    pub fn node_moving(&self, node: usize) -> bool {
        self.mover_of.get(&node).is_some_and(|&k| self.movers[k].state & MOVING != 0)
    }
}

/// Hit switches (subtype 0x1F): a blow presses them for every player, down
/// their chain.
pub fn hit_switch(mech: &mut Mechanics, placement: usize) {
    let Some(i) = mech.triggers.iter().position(|t| t.placement == placement && t.subtype == HIT_SWITCH) else {
        return;
    };
    let mut k = Some(i);
    while let Some(j) = k {
        mech.triggers[j].touches |= 0xF;
        k = mech.triggers[j].chain;
        if k == Some(i) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtype_flags() {
        assert_eq!(default_flags(0x14, 0x1234), 0x1210);
        assert_eq!(mover_loop(0, 'A', false), Some(("S_ELVMETA".into(), "S_ELVMETSTPA".into())));
        assert_eq!(mover_loop(4, 'G', true), Some(("S_ELVSTONEGB".into(), "S_ELVSTONESTPGB".into())));
        assert_eq!(mover_loop(5, 'B', false), Some(("S_ROCKROTATE".into(), "S_ROCKSTOP".into())));
        assert_eq!(mover_loop(-1, 'A', false), None);
        assert_eq!(default_flags(0x18, 0x0002), 0x000A);
        assert_eq!(default_flags(LIFTPAD, 0), 0x80C);
    }

    #[test]
    fn poses_turn_into_transforms() {
        let p = NodePose::turn_about([0.0; 3], std::f32::consts::FRAC_PI_2);
        let t = to_transform(&p);
        let x = t.rotation * Vec3::X;
        let v = p.apply_vector([1.0, 0.0, 0.0]);
        assert!((x - Vec3::from(v)).length() < 1e-5);
    }
}
