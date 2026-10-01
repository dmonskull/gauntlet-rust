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
//! Quest triggers (flag 0x40) are the tower's gates: shut until the
//! hero's crystals or gargoyle pieces open them (`quest.rs`).
//!
//! Animated objects (the world file's table, `docs/mechanics.md`,
//! "Animated objects") pose their nodes from their tracks at 30 frames a
//! second: round and round, or — aimed at by a trigger — back to their
//! first frame while it's off and on to their last while it's on, with the
//! cut that shows them waiting for them.
//!
//! Stand-ins: triggers run on
//! or off screen; subtype 1 rotators aren't done; only players (not
//! monsters) hold a mover still by standing on it; an animated object's
//! scale is drawn but its collision doesn't scale (not confirmed), and
//! the bursting ones (node type 0x50000: H1's fire, the I realm's
//! minecarts) show their explosion and sound as their loop comes round
//! but its blast doesn't hurt yet (`effects.rs`).

use gdl_formats::detmath::Det;
use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use gdl_formats::anim::{Pose, ROTATION_BITS, SCALE_BITS, TRANSLATION_BITS, Track, rotation_matrix as pose_matrix};
use gdl_formats::collision::NodePose;
use gdl_formats::population::{LocatorKind, PlacementParams, Population};
use gdl_formats::WorldNode;

use crate::audio::{LoopSoundAt, PlaySoundAt};
use crate::effects::EffectAt;
use crate::items::{self, LevelItems};
use crate::level_material::LevelMaterial;
use crate::monsters::MonsterLevel;
use crate::play_camera::{PlayCamera, Shake, StartCut};
use crate::player::{Player, PlayerTick};
use crate::message_box::ShowMessage;
use crate::party::Party;
use crate::player_state::TimeStop;
use crate::population::LevelPopulation;
use crate::quest;
use crate::world::LevelGround;

pub struct MechanicsPlugin;

impl Plugin for MechanicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            setup.run_if(resource_exists_and_changed::<LevelPopulation>).after(crate::quest::seed_tests),
        )
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

/// The nodes something moves: every trigger's target — the game makes
/// a mover of it whether or not the trigger is for the party, and holds
/// it at its off height (or hides a bridge) — the party's rotators' nodes
/// and the animated objects.
pub fn moving_roots(population: &Population, players: u8) -> HashSet<usize> {
    population
        .placements
        .iter()
        .filter_map(|p| match p.params(population.resolved_type(p).class) {
            PlacementParams::Trigger { target, .. } => target,
            PlacementParams::Rotator { target, .. } if p.active_for(players) => target,
            _ => None,
        })
        .chain(population.animations.iter().filter(|a| a.track.is_some()).map(|a| a.node))
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

/// The movers', bridges' and rotators' sounds play at this requested
/// volume.
const MECHANISM_VOLUME: u8 = 0xE0;

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

/// The movers the item set-up registers (`docs/mechanics.md`,
/// "Registration"): every trigger's target, for the party or not — the
/// first registration setting its kind, flags, heights and sound, later
/// ones filling heights still 0 and a sound still 0 or none, lift kinds
/// merging — each at its off height.
fn register_movers(pop: &Population, nodes: usize) -> (Vec<Mover>, HashMap<usize, usize>) {
    let mut movers: Vec<Mover> = Vec::new();
    let mut mover_of = HashMap::new();
    for p in &pop.placements {
        let ty = pop.resolved_type(p);
        let PlacementParams::Trigger { target: Some(node), flags, sound, off, on, .. } = p.params(ty.class) else { continue };
        if node >= nodes {
            continue;
        }
        let kind = ty.subtype;
        match mover_of.get(&node) {
            Some(&i) => {
                let mv: &mut Mover = &mut movers[i];
                if (LIFTPAD..=LIFTEND).contains(&mv.kind) && (LIFTPAD..=LIFTEND).contains(&kind) {
                    mv.kind = LIFTPAD;
                }
                if mv.off == 0.0 {
                    mv.off = 0.1 * f32::from(off);
                    mv.offset = mv.off;
                }
                if mv.on == 0.0 {
                    mv.on = 0.1 * f32::from(on);
                }
                if mv.sound <= 0 {
                    mv.sound = sound;
                }
            }
            None => {
                let (off, on) = (0.1 * f32::from(off), 0.1 * f32::from(on));
                mover_of.insert(node, movers.len());
                movers.push(Mover {
                    node,
                    kind,
                    flags: default_flags(kind, flags) as u8,
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
    (movers, mover_of)
}

/// The nodes in the animated mode: each animated object's, and every
/// node under one (the loader passes the flag down).
fn animated_nodes(pop: &Population, nodes: &LevelNodes) -> HashSet<usize> {
    let table: HashSet<usize> = pop.animations.iter().map(|a| a.node).filter(|&n| n < nodes.nodes.len()).collect();
    (0..nodes.nodes.len()).filter(|&n| nodes.group_of(n, &table).is_some()).collect()
}

/// Where the level's nodes stand as its items are dropped onto their
/// floors: the level load runs the world's update once just before the
/// item set-up (`docs/level-population.md`), which poses every animated
/// object at its first frame, and the item set-up runs the mover
/// update once after making the items, so each mover's node — and
/// everything under it — is at its off height (not bridges and particle
/// nodes, which don't move; a mover in the animated mode only gets its
/// play flags). Every node so moved, with its pose.
pub fn start_poses(pop: &Population, nodes: &LevelNodes) -> HashMap<usize, NodePose> {
    let (movers, _) = register_movers(pop, nodes.nodes.len());
    let animated = animated_nodes(pop, nodes);
    let mut local: HashMap<usize, NodePose> = movers
        .iter()
        .filter(|mv| mv.flags & BRIDGE == 0 && !animated.contains(&mv.node))
        .filter(|mv| nodes.nodes[mv.node].flags & gdl_formats::collision::node_flags::PARTICLES == 0)
        .map(|mv| (mv.node, NodePose::translation([0.0, mv.off, 0.0])))
        .collect();
    for a in pop.animations.iter().filter(|a| a.node < nodes.nodes.len()) {
        let Some(track) = &a.track else { continue };
        local.insert(a.node, animated_pose(track, &track.sample(0.0), nodes.origin[a.node]));
    }
    let root_set: HashSet<usize> = local.keys().copied().collect();
    let mut roots: Vec<usize> = root_set.iter().copied().collect();
    roots.sort_by_key(|&r| (nodes.depth(r), r));
    let mut world: HashMap<usize, NodePose> = HashMap::new();
    for &root in &roots {
        let here = local[&root];
        let above = nodes.parent[root].and_then(|p| nodes.group_of(p, &root_set)).and_then(|p| world.get(&p).copied());
        world.insert(root, above.map_or(here, |parent| here.then(&parent)));
    }
    (0..nodes.nodes.len())
        .filter_map(|n| nodes.group_of(n, &root_set).map(|r| (n, world[&r])))
        .filter(|(_, pose)| !pose.is_rest())
        .collect()
}

/// Node flags the animated objects play by (the game keeps them in the
/// node's `+0x10`): back to the first frame, on to the last — both: held
/// where it is, neither: round and round — and which end it's at.
const PLAY_BACK: u32 = 0x10_0000;
const PLAY_ON: u32 = 0x20_0000;
const AT_START: u32 = 0x40_0000;
const AT_END: u32 = 0x80_0000;
/// Its frame moved this tick (a cut showing the node holds meanwhile);
/// the movers set it on a node they move too.
const ANIMATING: u32 = 0x800_0000;
/// The animated mode's node flag.
const ANIMATED_MODE: u32 = 0x200_0000;
/// Frames a second the animated objects play at (`r2-0x5040` × the
/// frame's time).
const ANIMATION_FPS: f32 = 30.0;
/// They wait while a camera cut has at least this many fields left to
/// hold (playing in its last ten, and through an endless one).
const CUT_HOLDS_ANIMATIONS: f32 = 11.0;
const ENDLESS_CUT: f32 = 99_999.0;

/// A world object the level animates (the world file's table,
/// `docs/mechanics.md`, "Animated objects").
struct Animation {
    node: usize,
    frames: u16,
    track: Track,
    frame: f32,
    /// The node's pose from the track: turned about its origin by the
    /// track's angles and moved by its translation.
    local: NodePose,
    /// The track's scale, where it has one.
    scale: Option<[f32; 3]>,
    /// Bursts as its loop comes round (node type 0x50000), and is hidden
    /// for it now.
    bursts: bool,
    burst_hidden: bool,
}

/// Where an animated object's track puts its node, about the node's
/// origin: the angles become its rotation (the game's own two builders),
/// the translation adds to where the file puts it; channels the track
/// lacks leave the node's (identity, none) alone.
fn animated_pose(track: &Track, pose: &Pose, origin: [f32; 3]) -> NodePose {
    let has = |bits: [u16; 3]| bits.iter().any(|&b| track.flags & b != 0);
    let rotation = if has(ROTATION_BITS) {
        // Row vectors (v · M) to column vectors, row-major.
        let m = pose_matrix(pose.rotation, track.flags);
        [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]]
    } else {
        NodePose::REST.rotation
    };
    let t = if has(TRANSLATION_BITS) { pose.translation } else { [0.0; 3] };
    let turned = NodePose { rotation, translation: [0.0; 3] }.apply_vector(origin);
    NodePose { rotation, translation: std::array::from_fn(|i| origin[i] + t[i] - turned[i]) }
}

/// One tick of an animated object (the game's per-object update): with
/// both play flags it's held, else it poses its node at its frame and —
/// unless it goes round and round while time is stopped — moves the frame
/// on: back to the first frame (`PLAY_BACK`), on to the last (`PLAY_ON`)
/// or round (neither), flagging the end it's at and whether it animated.
/// A bursting one (node type `0x50000`) going round bursts where it is as
/// its lap ends and is hidden till the next lap's first frames: where it
/// burst.
fn play_animation(a: &mut Animation, play: &mut u32, stopped: bool, origin: [f32; 3]) -> Option<[f32; 3]> {
    let (back, on) = (*play & PLAY_BACK != 0, *play & PLAY_ON != 0);
    if back && on {
        *play &= !ANIMATING;
        return None;
    }
    let pose = a.track.sample(a.frame);
    a.local = animated_pose(&a.track, &pose, origin);
    a.scale = SCALE_BITS.iter().any(|&b| a.track.flags & b != 0).then_some(pose.scale);
    let round = !back && !on;
    if stopped && round {
        return None;
    }
    *play = (*play & !(AT_START | AT_END)) | ANIMATING;
    let last = f32::from(a.frames.saturating_sub(1));
    let mut burst = None;
    if back {
        a.frame -= ANIMATION_FPS * DT;
        if a.frame < 0.0 {
            a.frame = 0.0;
            *play = (*play & !ANIMATING) | AT_START;
        }
    } else {
        a.frame += ANIMATION_FPS * DT;
        if a.frame.trunc() >= last {
            if round {
                a.frame = 0.0;
                if a.bursts && !a.burst_hidden {
                    a.burst_hidden = true;
                    burst = Some(a.local.apply(origin));
                }
            } else {
                a.frame = last;
                *play &= !ANIMATING;
            }
            *play |= AT_END;
        } else if a.frame < BURST_SHOWN && a.burst_hidden {
            a.burst_hidden = false;
        }
    }
    burst
}

/// A burst node shows again once its next lap has this many frames to go
/// past the first.
const BURST_SHOWN: f32 = 2.0;
/// Node types (`flags & 0x100F0000`) that burst as their loop comes round.
const BURSTS: u32 = 0x5_0000;
const BURST_TYPE: u32 = 0x100F_0000;
/// The burst's sound in the ice realm (the only one with one) and its
/// requested volume.
const BURST_SOUND: &str = "S_MINECAREXPLO";
const BURST_VOLUME: u8 = 0x7F;

/// What a burst shows by realm id: the ice and sky realms the realm's
/// own `WORLD_EXP` (from its items), at 1; the rest the effect table's
/// EXPLOSION at 1.5 across (the game's 1.5 × 1 × 1.5).
fn burst_effect(realm: usize) -> (&'static str, Option<&'static str>, f32) {
    match realm {
        9 => ("WORLD_EXP", Some("ITEMS/levelI"), 1.0),
        11 => ("WORLD_EXP", Some("ITEMS/levelK"), 1.0),
        _ => ("EXPLOSION", None, 1.5),
    }
}

/// A mover on a node in the animated mode (kind flags `flags`, state
/// `st`): no heights — on plays its node's animation on to the last frame,
/// off back to the first, and it has arrived at that end; a returning one
/// (kind flag 0x20) goes round while held and stops at its end once let
/// go. Whether it's moving; `None` when a hero stands on it (without kind
/// flag 8), which holds it where it is.
fn drive_animation(flags: u8, carrying: bool, st: &mut u8, play: &mut u32) -> Option<bool> {
    if flags & MOVES_CARRYING == 0 && carrying {
        *play |= PLAY_BACK | PLAY_ON;
        *st &= !MOVING;
        return None;
    }
    let mut go = true;
    if flags & RETURNS == 0 {
        *play = if *st & ON == 0 { (*play | PLAY_BACK) & !PLAY_ON } else { (*play | PLAY_ON) & !PLAY_BACK };
        if (*play & PLAY_BACK != 0 && *play & AT_START != 0) || (*play & PLAY_ON != 0 && *play & AT_END != 0) {
            go = false;
        }
    } else if *st & PLAYERS == 0 {
        if *play & AT_END != 0 {
            *st &= !(ON | MOVING);
            go = false;
            *play |= PLAY_BACK | PLAY_ON;
        }
    } else {
        *st |= ON | MOVING;
        *play &= !(PLAY_BACK | PLAY_ON);
    }
    Some(go)
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
    /// Seconds of play (ticks), and when each shut quest gate (by id) may
    /// say what it needs again.
    clock: f32,
    need_again: HashMap<u8, f32>,
    /// The movers' loop asked for last tick (the one playing).
    mover_loop: Option<String>,
    /// The animated objects; the nodes in their mode (theirs and their
    /// subtrees', which the game passes the flag down to); those nodes'
    /// play flags; and the animation of each animated node.
    animations: Vec<Animation>,
    animated: HashSet<usize>,
    play: HashMap<usize, u32>,
    animation_of: HashMap<usize, usize>,
    /// Roots drawn scaled: their scale and the point it's about.
    scales: HashMap<usize, ([f32; 3], [f32; 3])>,
    /// The triggers for every player that the drop put on their target
    /// have been made stand-on ones.
    stand_rule: bool,
}

impl Mechanics {
    /// Fires the triggers with this id the way the tower does
    /// (`docs/items.md`, "Quest items and the tower's gates"): each — not
    /// ones that need standing on their target, or with flag `0x8000` —
    /// and those chained after it come on for good (the game also sets
    /// their `0x400`); `snap` puts their movers at their on heights at once
    /// (as the tower loads), else they travel there.
    pub fn fire(&mut self, id: u8, snap: bool) {
        for i in 0..self.triggers.len() {
            let t = &self.triggers[i];
            if t.id != id || t.flags & (STAND_ON_TARGET | 0x8000) != 0 {
                continue;
            }
            let mut k = Some(i);
            let mut steps = 0;
            while let Some(j) = k {
                let t = &mut self.triggers[j];
                t.flags |= ALL_PLAYERS;
                t.action = 2;
                if let Some(&mv) = t.target.and_then(|n| self.mover_of.get(&n)) {
                    let mv = &mut self.movers[mv];
                    mv.state = ON | PLAYERS;
                    if snap {
                        mv.previous = mv.state;
                        mv.offset = mv.on;
                    }
                }
                k = t.chain;
                steps += 1;
                if k == Some(i) || steps > self.triggers.len() {
                    break;
                }
            }
            debug!("trigger {} (id {id}) fired{}", self.triggers[i].placement, if snap { " as the level loads" } else { "" });
        }
    }
}

/// A quest gate (trigger flag 0x40): touched while its crystals (ids
/// below 100: the crystal counter of that number) or its gargoyle section
/// (101 on: fangs, feathers, claws; 104 and up count as claws) haven't
/// opened it, it says what it needs — at most every
/// [`quest::NEED_AGAIN_SECONDS`] — and forgets the touch. Its touches
/// then go down its chain.
fn quest_gate(mech: &mut Mechanics, i: usize, party: &Party, messages: &mut MessageWriter<ShowMessage>) {
    let t = &mech.triggers[i];
    if t.touches != 0 {
        // Any player's progress opens it.
        let id = i32::from(t.id);
        let (open, need) = if id < 100 {
            (party.any(|s| s.quest.crystals_open(id as usize)), Some(("NEEDCRYSTALS", id as usize)))
        } else {
            let section = id - 101;
            let open = party.any(|s| s.quest.gargoyle_open(section.clamp(0, 2) as usize));
            (open, (0..3).contains(&section).then_some(("NEEDGARGITEMS", section as usize)))
        };
        if !open {
            if let Some((group, index)) = need
                && mech.need_again.get(&t.id).is_none_or(|&at| mech.clock >= at)
            {
                messages.write(ShowMessage::new(group, index));
                mech.need_again.insert(t.id, mech.clock + quest::NEED_AGAIN_SECONDS);
            }
            mech.triggers[i].touches = 0;
        }
    }
    let touches = mech.triggers[i].touches;
    let mut k = mech.triggers[i].chain;
    while let Some(j) = k {
        if j == i {
            break;
        }
        mech.triggers[j].touches = touches;
        k = mech.triggers[j].chain;
    }
}

fn setup(
    mut commands: Commands,
    population: Res<LevelPopulation>,
    nodes: Option<Res<LevelNodes>>,
    party: Res<Party>,
) {
    let Some(nodes) = nodes else { return };
    let pop = &population.population;
    let mut m = Mechanics::default();
    (m.movers, m.mover_of) = register_movers(pop, nodes.nodes.len());
    for (placement, p) in pop.placements.iter().enumerate() {
        let ty = pop.resolved_type(p);
        // Only the party's triggers and rotators run.
        if !p.active_for(population.players) {
            continue;
        }
        match p.params(ty.class) {
            PlacementParams::Trigger { target, flags, radius, id, next, off, on, .. } => {
                let target = target.filter(|&t| t < nodes.nodes.len());
                let flags = default_flags(ty.subtype, flags);
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
    // Animated objects: the loader puts their nodes (and every node under
    // them) in the animated mode; the item set-up has every trigger's
    // target play back to its first frame.
    m.animated = animated_nodes(pop, &nodes);
    for a in pop.animations.iter().filter(|a| a.node < nodes.nodes.len()) {
        let Some(track) = a.track.clone() else { continue };
        m.animation_of.insert(a.node, m.animations.len());
        let bursts = nodes.nodes[a.node].flags & BURST_TYPE == BURSTS;
        m.animations.push(Animation {
            node: a.node,
            frames: a.frames,
            track,
            frame: 0.0,
            local: NodePose::REST,
            scale: None,
            bursts,
            burst_hidden: false,
        });
    }
    for mv in &m.movers {
        m.play.insert(mv.node, PLAY_BACK);
    }
    let mut roots: Vec<usize> = m
        .movers
        .iter()
        .map(|mv| mv.node)
        .chain(m.rotators.iter().map(|r| r.node))
        .chain(m.animations.iter().map(|a| a.node))
        .collect();
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
    // The tower opens the gates the quest has opened for good as it loads
    // (any player's).
    if quest::level_of(&population.level).is_some_and(|(realm, _)| realm == quest::TOWER) {
        let mut gates: Vec<u8> = party.states().flat_map(|(_, s)| s.quest.tower_gates(s.runestone_bits())).collect();
        gates.sort_unstable();
        gates.dedup();
        for id in gates {
            m.fire(id, true);
        }
    }
    info!(
        "mechanics: {} triggers, {} movers ({} animated), {} rotators, {} chained, {} animated objects",
        m.triggers.len(),
        m.movers.len(),
        m.movers.iter().filter(|mv| m.animated.contains(&mv.node)).count(),
        m.rotators.len(),
        m.triggers.iter().filter(|t| t.chain.is_some()).count(),
        m.animations.len()
    );
    // Movers that start off their rest height, animated objects at their
    // first frame.
    for a in &mut m.animations {
        a.local = animated_pose(&a.track, &a.track.sample(0.0), nodes.origin[a.node]);
    }
    for (root, pose) in world_poses(&m, &nodes) {
        m.poses.insert(root, (pose, pose));
    }
    commands.insert_resource(m);
}

/// A hero in play as the mechanics see it: its slot's bit, feet, radius,
/// half height and the node it stands on.
type InPlay = (u8, [f32; 3], f32, f32, Option<usize>);

#[allow(clippy::too_many_arguments)]
fn tick(
    mechanics: Option<ResMut<Mechanics>>,
    nodes: Option<Res<LevelNodes>>,
    items: Option<ResMut<LevelItems>>,
    ground: Option<ResMut<LevelGround>>,
    party: Res<Party>,
    mut players: Query<&mut Player>,
    mut models: Query<&mut Transform, Without<Player>>,
    mut cuts: MessageWriter<StartCut>,
    mut shakes: MessageWriter<Shake>,
    mut sounds: MessageWriter<PlaySoundAt>,
    mut loops: MessageWriter<LoopSoundAt>,
    level: Option<Res<MonsterLevel>>,
    mut messages: MessageWriter<ShowMessage>,
    (camera, time_stop): (Option<Res<PlayCamera>>, Option<Res<TimeStop>>),
    mut effects: MessageWriter<EffectAt>,
) {
    let (Some(mut mech), Some(nodes), Some(mut items), Some(mut ground)) = (mechanics, nodes, items, ground) else {
        return;
    };
    let mech = &mut *mech;
    // The heroes in play: their slot's bit, feet, size and the node each
    // stands on.
    let heroes: Vec<InPlay> = players
        .iter()
        .filter_map(|p| {
            let s = party.state(p.slot).filter(|s| s.alive)?;
            Some((1u8 << p.slot.min(3), p.mover.position, s.radius, s.half_height, p.ground.node))
        })
        .collect();
    // Every player in the game, by bit, and how many.
    let in_game = party.members().fold(0u8, |bits, (slot, _)| bits | 1 << slot.min(3));
    let player_count = party.len().max(1);

    // A trigger for every player (0x400) the item drop put on its target
    // — or on a child of it — counts only while stood on there (the drop
    // gives it flag 0x100).
    if !mech.stand_rule && items.views().next().is_some() {
        for t in &mut mech.triggers {
            if t.flags & ALL_PLAYERS != 0
                && let (Some(target), Some(floor)) = (t.target, items.view(t.placement).and_then(|v| v.floor_node))
                && (floor == target || nodes.parent.get(floor).copied().flatten() == Some(target))
            {
                t.flags |= STAND_ON_TARGET;
            }
        }
        mech.stand_rule = true;
    }

    // Touches.
    for &(bit, feet, radius, half, _) in &heroes {
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
                mech.triggers[j].touches |= bit;
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
    mech.clock += DT;
    // The players standing on a node or on one of its children, by bit.
    let on_target = |target: usize| {
        heroes
            .iter()
            .filter(|h| h.4.is_some_and(|n| n == target || nodes.parent.get(n).copied().flatten() == Some(target)))
            .fold(0u8, |bits, h| bits | h.0)
    };
    for i in 0..mech.triggers.len() {
        let (flags, target) = (mech.triggers[i].flags, mech.triggers[i].target);
        if flags & QUEST != 0 {
            quest_gate(mech, i, &party, &mut messages);
        }
        let t = &mut mech.triggers[i];
        if t.timer > 0.0 {
            t.timer -= FIELDS;
        }
        let mut m = t.touches;
        if m != 0 && flags & STAND_ON_TARGET != 0 {
            let standing_bits = target.map_or(0, on_target);
            t.touches &= 0xF0 | standing_bits;
            m = t.touches;
        }
        // Every player in the game on it.
        if flags & ALL_PLAYERS != 0 && m != in_game {
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
                            // (players − 1) × 60 fields.
                            t.timer = 60.0 * (player_count - 1) as f32;
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
                cuts.write(StartCut::trigger(locator, target));
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

    // Animated objects (the game runs them before the item update's
    // movers): each poses its node at its frame, then moves the frame on —
    // unless a camera cut holds more than ten fields yet; round-and-round
    // ones wait while time is stopped.
    let cut_holds = camera
        .as_ref()
        .and_then(|c| c.cut_counts())
        .is_some_and(|(hold, _)| (CUT_HOLDS_ANIMATIONS..=ENDLESS_CUT).contains(&hold));
    if !cut_holds {
        let stopped = time_stop.as_ref().is_some_and(|t| t.0);
        let (name, bank, scale) = burst_effect(items.realm());
        for a in &mut mech.animations {
            let play = mech.play.entry(a.node).or_default();
            if let Some(at) = play_animation(a, play, stopped, nodes.origin[a.node]) {
                // A burst: the realm's explosion where it is (its blast is
                // `effects.rs`'s, not here yet), the ice realm's sound.
                debug!("animated node {} bursts at {at:?}", a.node);
                effects.write(EffectAt { name, bank, at: Vec3::from(at), facing: 0.0, scale });
                if items.realm() == 9 {
                    sounds.write(PlaySoundAt::faded(BURST_SOUND, Vec3::from(at), BURST_VOLUME));
                }
            }
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
    // Sounds to play at a node, once the nodes' poses are known.
    let mut shots: Vec<(usize, String)> = Vec::new();
    for mv in &mut mech.movers {
        let carrying = heroes.iter().any(|h| h.4 == Some(mv.node));
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
                        shots.push((mv.node, (*name).into()));
                    }
                }
            } else if mv.sound < 10 {
                if let Some((run, stop)) = mover_loop(mv.sound, letter, boss_level) {
                    if st & MOVING != 0 {
                        looping.get_or_insert((run, mv.node));
                    } else if prev & MOVING != 0 && mech.mover_loop.as_deref() == Some(run.as_str()) {
                        // Its stop sound only if its set's loop was playing.
                        shots.push((mv.node, stop));
                    }
                }
            } else if mv.sound == 11 {
                if st & ON != 0 && prev & ON == 0 && let Some(name) = shot(1) {
                    shots.push((mv.node, (*name).into()));
                }
            } else if (st ^ prev) & MOVING != 0 && let Some(name) = shot(mv.sound - 10) {
                shots.push((mv.node, (*name).into()));
            }
        }
        let mut disable = 0u8;
        let moving;
        let animated = mech.animated.contains(&mv.node);
        if mv.flags & BRIDGE == 0 && animated {
            // The animated mode: no heights; on plays the node's animation
            // on to its last frame, off back to its first, and it has
            // arrived at that end. A returning one (kind flag 0x20) goes
            // round while held and stops at its end once let go.
            let play = mech.play.entry(mv.node).or_insert(PLAY_BACK);
            let Some(go) = drive_animation(mv.flags, carrying, &mut st, play) else {
                // Stood on: held where it is.
                mv.state = st;
                if let Some(c) = collision.as_mut() {
                    c.set_disable(mv.node, 0);
                }
                continue;
            };
            moving = go;
        } else if mv.flags & BRIDGE != 0 {
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
    // A burst animated object is hidden till its next lap starts.
    hidden.extend(mech.animations.iter().filter(|a| a.burst_hidden).map(|a| a.node));
    mech.hidden = hidden;
    mech.fades = fades;

    // Rotators; a touched one grinds round (its realm's sound) until it
    // reaches its limit.
    let rotator_sound = match realm {
        1 => Some(("S_ROCKROTATE", "S_ROCKSTOP")),
        9 => Some(("S_METLROTATE", "S_METLROTATESTO")),
        _ => None,
    };
    let mut grinding = None;
    for r in &mut mech.rotators {
        match r.subtype {
            0 => r.total += r.angle * FIELDS,
            2 if r.touched && !r.done => {
                r.total += r.angle * FIELDS;
                if r.total.abs() >= r.limit.abs() {
                    r.total = r.limit.abs().copysign(r.total);
                    r.done = true;
                    if let Some((_, stop)) = rotator_sound {
                        shots.push((r.node, stop.into()));
                    }
                } else {
                    // Each one grinding on pans the one loop: the last's
                    // pan holds.
                    grinding = Some(r.node);
                }
            }
            _ => {}
        }
    }
    let grind = rotator_sound.zip(grinding).map(|((run, _), node)| (run.to_string(), node));

    let world = world_poses(mech, &nodes);
    mech.scales = mech
        .animations
        .iter()
        .filter_map(|a| a.scale.map(|s| (a.node, (s, nodes.origin[a.node]))))
        .collect();
    for (&root, &pose) in &world {
        let entry = mech.poses.entry(root).or_insert((pose, pose));
        *entry = (entry.1, pose);
    }

    // The movers' and rotators' sounds, where their nodes are now: the
    // one-shots panned; one loop for all the movers, following the first
    // that moves, one for the rotators, following the last grinding on
    // (`docs/audio-format.md`, "Positional sounds").
    let place = |node: usize| {
        let origin = nodes.origin.get(node).copied().unwrap_or_default();
        Vec3::from(world.get(&node).map_or(origin, |pose| pose.apply(origin)))
    };
    for (node, name) in shots {
        sounds.write(PlaySoundAt::panned(name, place(node), MECHANISM_VOLUME));
    }
    mech.mover_loop = looping.as_ref().map(|(name, _)| name.clone());
    for (key, sound) in [("mover", looping), ("rotator", grind)] {
        loops.write(match sound {
            Some((name, node)) => LoopSoundAt::at(key, name, place(node), MECHANISM_VOLUME),
            None => LoopSoundAt::stop(key),
        });
    }
    if let Some(c) = collision {
        for (root, members) in &mech.members {
            let pose = world[root];
            for &n in members {
                c.set_pose(n, pose);
            }
        }
    }

    // Items ride the moving floor the drop put them on (the game hangs
    // them under its node): lift pads on their lifts, pickups on
    // platforms.
    for (placement, node) in items.riders() {
        let Some(pose) = nodes.group_of(node, &mech.root_set).and_then(|root| world.get(&root)) else { continue };
        if let Some((Some(model), at, rotation)) = items.ride(placement, pose)
            && let Ok(mut transform) = models.get_mut(model)
        {
            transform.translation = Vec3::from(at);
            transform.rotation = Quat::from_mat3(&Mat3::from_cols_array(&rotation));
        }
    }

    // Carry each hero with what it stands on.
    for mut p in &mut players {
        if let Some(node) = p.ground.node
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
            p.mover.facing -= x[2].datan2(x[0]);
        }
    }
}

/// Each moving root's world pose, parents first: a node's own move (a
/// mover's height, a rotator's turn, an animated object's pose), then its
/// parent's.
fn world_poses(mech: &Mechanics, nodes: &LevelNodes) -> HashMap<usize, NodePose> {
    let mut world: HashMap<usize, NodePose> = HashMap::new();
    for &root in &mech.roots {
        let mut local = NodePose::REST;
        if let Some(&k) = mech.mover_of.get(&root)
            && !mech.animated.contains(&root)
        {
            local = NodePose::translation([0.0, mech.movers[k].offset, 0.0]);
        }
        if let Some(r) = mech.rotators.iter().find(|r| r.node == root) {
            local = local.then(&NodePose::turn_about(nodes.origin[root], r.total));
        }
        if let Some(&k) = mech.animation_of.get(&root) {
            local = local.then(&mech.animations[k].local);
        }
        let above = nodes.parent[root].and_then(|p| nodes.group_of(p, &mech.root_set)).and_then(|p| world.get(&p).copied());
        let pose = match above {
            Some(parent) => local.then(&parent),
            None => local,
        };
        world.insert(root, pose);
    }
    world
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
        // An animated object's scale, about its node's origin.
        transform.scale = match mech.scales.get(&g.root) {
            Some(&(scale, pivot)) => {
                let (s, p) = (Vec3::from(scale), Vec3::from(pivot));
                let shift = transform.rotation * (p - s * p);
                transform.translation += shift;
                s
            }
            None => Vec3::ONE,
        };
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
    /// Whether the mover on `node` is moving (or a bridge fading); an
    /// animated node, whether its frame moved this tick.
    pub fn node_moving(&self, node: usize) -> bool {
        if self.animated.contains(&node) {
            return self.play.get(&node).is_some_and(|p| p & ANIMATING != 0);
        }
        self.mover_of.get(&node).is_some_and(|&k| self.movers[k].state & MOVING != 0)
    }

    /// The node flags the game sets at run time that its other code reads
    /// beside the file's: `0x2000000` on a node in the animated mode, and
    /// `0x8000000` while its animation plays — or while a mover moves it.
    pub fn node_flags(&self, node: usize) -> u32 {
        if self.animated.contains(&node) {
            let playing = self.play.get(&node).is_some_and(|p| p & ANIMATING != 0);
            return ANIMATED_MODE | if playing { ANIMATING } else { 0 };
        }
        let moving = self.mover_of.get(&node).is_some_and(|&k| self.movers[k].flags & BRIDGE == 0 && self.movers[k].state & MOVING != 0);
        if moving { ANIMATING } else { 0 }
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

    /// A track turning about Y through `keys` (frame, angle), `frames` long.
    fn turning(frames: u16, keys: &[(u16, f32)]) -> Animation {
        let keys = keys.iter().map(|&(f, y)| (f, Pose { rotation: [0.0, y, 0.0], ..Pose::default() })).collect();
        Animation {
            node: 0,
            frames,
            track: Track { flags: 0x0002, keys },
            frame: 0.0,
            local: NodePose::REST,
            scale: None,
            bursts: false,
            burst_hidden: false,
        }
    }

    #[test]
    fn a_switched_animation_plays_on_then_back() {
        let mut a = turning(5, &[(0, 0.0), (4, 1.0)]);
        // As the level loads every trigger target plays back, and is at
        // its start after one tick.
        let (mut play, mut st) = (PLAY_BACK, 0u8);
        play_animation(&mut a, &mut play, false, [0.0; 3]);
        assert_eq!(drive_animation(0x02, false, &mut st, &mut play), Some(false));
        assert_eq!((a.frame, play & (AT_START | ANIMATING)), (0.0, AT_START));
        // Switched on (the animations run before the movers each tick): it
        // turns the mover's way the tick after, frames 0 to 4 a tick each,
        // and has arrived once the last comes up.
        st = ON;
        let tick = |a: &mut Animation, st: &mut u8, play: &mut u32| {
            play_animation(a, play, false, [0.0; 3]);
            drive_animation(0x02, false, st, play).unwrap()
        };
        let moving: Vec<bool> = (0..7).map(|_| tick(&mut a, &mut st, &mut play)).collect();
        assert_eq!(moving, [true, true, true, true, false, false, false]);
        assert_eq!(a.frame, 4.0);
        assert_eq!(play & (PLAY_ON | AT_END | ANIMATING), PLAY_ON | AT_END);
        // Posed at the last key: a turn of 1 radian about Y.
        let x = a.local.apply_vector([1.0, 0.0, 0.0]);
        assert!((x[0] - 1f32.dcos()).abs() < 1e-5 && (x[2].abs() - 1f32.dsin()).abs() < 1e-5, "{x:?}");
        // Off again: back to the first frame — a tick longer, as it only
        // stops once the frame goes below the first.
        st = 0;
        let moving: Vec<bool> = (0..7).map(|_| tick(&mut a, &mut st, &mut play)).collect();
        assert_eq!(moving, [true, true, true, true, true, false, false]);
        assert_eq!(a.frame, 0.0);
        assert_eq!(play & (PLAY_BACK | AT_START), PLAY_BACK | AT_START);
        // A hero standing on it (no kind flag 8) holds it.
        st = ON;
        assert_eq!(drive_animation(0x02, true, &mut st, &mut play), None);
        play_animation(&mut a, &mut play, false, [0.0; 3]);
        assert_eq!((a.frame, play & ANIMATING), (0.0, 0));
    }

    /// On the disc: the level load poses the animated objects at their
    /// first frame before the items are dropped, so G1's lift pads land
    /// on the platforms they send — the swinging arm's swung out, the
    /// lift lowered — and ride them (real data).
    #[test]
    fn items_drop_onto_animated_objects_at_their_first_frame() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(bytes) = std::fs::read(std::path::Path::new(&root).join("LEVELS/levelG1/WORLDS.PS2")) else {
            eprintln!("skipping: no levelG1");
            return;
        };
        let pop = Population::parse(&bytes).expect("population");
        let world = gdl_formats::WorldFile::parse(&bytes).expect("world");
        let nodes = LevelNodes::new(world.nodes.clone());
        let starts = start_poses(&pop, &nodes);
        let mut c = gdl_formats::LevelCollision::new(&world).expect("collision");
        for (&n, &pose) in &starts {
            c.set_pose(n, pose);
        }
        let node = |name: &str| world.nodes.iter().position(|n| n.name == name).expect(name);
        for (target, floor) in [("G1WOOD_ARM", "G1FLOOR#29"), ("G1PLAT1", "G1FLOOR#44")] {
            let target = node(target);
            let pad = pop
                .placements
                .iter()
                .find(|p| {
                    let ty = pop.resolved_type(p);
                    matches!(p.params(ty.class), PlacementParams::Trigger { target: Some(t), flags, .. } if t == target && flags & STAND_ON_TARGET != 0)
                })
                .expect("its lift pad");
            let hit = c.floor_probe(pad.position, 4.0, -10.0, 1.0, 0).expect("a floor under the pad");
            assert_eq!(world.nodes[hit.node].name, floor, "{target}");
        }
    }

    #[test]
    fn bursting_animations_burst_as_their_lap_ends() {
        let mut a = turning(3, &[(0, 0.0), (2, 0.5)]);
        a.bursts = true;
        let mut play = 0;
        let origin = [4.0, 1.0, -2.0];
        // Frames 0, 1, then round: it bursts where it is, hidden till the
        // next lap's first frames.
        assert_eq!(play_animation(&mut a, &mut play, false, origin), None);
        let burst = play_animation(&mut a, &mut play, false, origin);
        assert!(burst.is_some_and(|b| (0..3).all(|k| (b[k] - origin[k]).abs() < 1e-4)), "{burst:?}");
        assert!(a.burst_hidden);
        assert_eq!(play_animation(&mut a, &mut play, false, origin), None);
        assert!(!a.burst_hidden);
    }

    #[test]
    fn untriggered_animations_go_round_unless_time_stops() {
        let mut a = turning(3, &[(0, 0.0), (2, 0.5)]);
        let mut play = 0;
        let frames: Vec<f32> = (0..4)
            .map(|_| {
                play_animation(&mut a, &mut play, false, [0.0; 3]);
                a.frame
            })
            .collect();
        assert_eq!(frames, [1.0, 0.0, 1.0, 0.0]);
        play_animation(&mut a, &mut play, true, [0.0; 3]);
        assert_eq!(a.frame, 0.0);
    }

    #[test]
    fn animated_poses_turn_about_the_node() {
        let a = turning(1, &[(0, std::f32::consts::FRAC_PI_2)]);
        let origin = [10.0, 2.0, -4.0];
        let pose = animated_pose(&a.track, &a.track.sample(0.0), origin);
        // The origin stays put; a point a unit along X from it turns as the
        // game's builder turns it (row vectors: X · M is M's first row).
        let at = pose.apply(origin);
        assert!(at.iter().zip(origin).all(|(a, b)| (a - b).abs() < 1e-5), "{at:?}");
        let m = pose_matrix([0.0, std::f32::consts::FRAC_PI_2, 0.0], 0x0002);
        let x = pose.apply_vector([1.0, 0.0, 0.0]);
        assert!((0..3).all(|i| (x[i] - m[i]).abs() < 1e-5), "{x:?} vs {m:?}");
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
