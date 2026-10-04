//! The level walker (testing: `GDL_WALK=1`; never online): plays a level
//! on its own from its start to its exit — on foot, through the hero's own
//! controls — and says in the log what it did and where it stopped
//! (`docs/mechanics.md`, "Walking the levels").
//!
//! Whenever it needs something to do it maps where the hero can walk from
//! where it stands: a grid of the steps the game's own player move allows
//! (`LevelCollision::move_player`), less the steps an item stops
//! (`LevelItems::stops_step`), plus the transporters' hops. Then it picks
//! the nearest thing worth doing: a pad or lever to stand on, a hit switch
//! to throw at, a key to take, a door to open with one, a barrel, wall or
//! generator in the way to break, a moving floor to ride — and the exit as
//! soon as it can be walked to (`GDL_WALK=all`: once nothing else is left
//! to try). It fights what comes near. The map is made again whenever the
//! level's moving parts or the items in the way have changed.
//!
//! It ends with one line: `walker: <level> FINISHED …` or `walker: <level>
//! STUCK …`, with what it never reached; `GDL_WALK_SHOT=<file.png>` saves
//! a picture as it stops, and the game closes. Run it with
//! `GDL_IMMORTAL=1 GDL_SKIP_BOXES=1` (`tools/walk.sh`).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

use bevy::prelude::*;
use gdl_formats::LevelCollision;
use gdl_formats::collision::{PlayerCollision, PlayerGround};
use gdl_formats::detmath::Det;
use gdl_formats::population::ItemClass;

use crate::combat::{TargetKind, Targetable, button};
use crate::items::{self, LevelItems, Shape};
use crate::mechanics::{Mechanics, Switch};
use crate::party::{Inputs, Party};
use crate::play_camera::PlayCamera;
use crate::player::Player;
use crate::population::LevelPopulation;
use crate::world::LevelGround;

/// The map's grid, and the longest move it tries at once (a hero runs
/// about this far in a tick).
const CELL: f32 = 1.0;
const STEP: f32 = 0.5;
/// Places at one grid cell closer in height than this are one place.
const LEVEL: f32 = 2.0;
/// The map stops growing here.
const MOST_NODES: usize = 400_000;
/// A place is reached within this of it.
const ARRIVE: f32 = 0.5;
/// Ticks without getting nearer before a step is given up.
const STUCK: u32 = 45;
/// How often one thing is tried again (after something else changed).
const MOST_TRIES: u8 = 4;
/// Ticks it waits with nothing to do before it gives up; longer on a
/// floor that moves.
const PATIENCE: u32 = 600;
const RIDE_PATIENCE: u32 = 2400;
/// Ticks between looks while it waits.
const LOOK_EVERY: u32 = 20;
/// It fights monsters and generators this near, for at most `FIGHT` ticks
/// before a rest.
const FIGHT_RANGE: f32 = 9.0;
const FIGHT: u32 = 240;
const REST: u32 = 120;
/// A hit switch is thrown at from within this, in sight.
const THROW_RANGE: f32 = 12.0;
/// Goal numbers of the floors that move start here (items are numbered by
/// their placements).
const RIDES: usize = 1 << 28;
/// A secret exit's subtype: left alone.
const SECRET_EXIT: i32 = 0x32;
/// A key's powerup subtype.
const KEY: i32 = 2;
/// Frames between the last line and closing, for the picture to be saved.
const CLOSE_AFTER: u32 = 45;

pub struct WalkerPlugin;

impl Plugin for WalkerPlugin {
    fn build(&self, app: &mut App) {
        let Ok(mode) = std::env::var("GDL_WALK") else { return };
        if mode.is_empty() || mode == "0" {
            return;
        }
        app.insert_resource(Walker { all: mode.eq_ignore_ascii_case("all"), ..default() }).add_systems(Update, close);
    }
}

/// Whether the walker has the first player's controls.
pub(crate) fn walking(walker: Option<Res<Walker>>, lock: Res<crate::online::Lockstep>) -> bool {
    walker.is_some() && !lock.on
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Exit,
    Pad,
    Hit,
    Turner,
    Key,
    Door,
    Break,
    Ride,
}

/// Something to do: what (an item's placement, or a moving floor's node
/// past [`RIDES`]), where it is, and the place on the map to do it from.
#[derive(Clone, Copy, Debug)]
struct Goal {
    id: usize,
    kind: Kind,
    at: [f32; 3],
    node: u32,
    cost: f32,
}

#[derive(Default)]
enum State {
    /// Look at the level and pick something.
    #[default]
    Plan,
    Go {
        goal: Goal,
        path: Vec<u32>,
        next: usize,
        best: f32,
        stalled: u32,
    },
    Act {
        goal: Goal,
        ticks: u32,
    },
    /// What a trigger set going comes to rest.
    Settle {
        ticks: u32,
    },
    /// Nothing to do: look again in a while.
    Wait {
        ticks: u32,
    },
}

#[derive(Clone, Copy, Default)]
struct Tried {
    tries: u8,
    /// [`Walker::progress`] when it was last tried.
    progress: u64,
}

#[derive(Resource, Default)]
pub(crate) struct Walker {
    all: bool,
    level: String,
    map: Option<Map>,
    state: State,
    tried: HashMap<usize, Tried>,
    /// Counts the changes in what there is to do.
    progress: u64,
    seen: u64,
    /// Ticks waited since the last change.
    waited: u32,
    ticks: u64,
    fought: u32,
    rest: u32,
    last_feet: Option<[f32; 3]>,
    /// Everything it could ever have walked to, and what it did.
    reached: HashSet<usize>,
    done: Vec<String>,
    maps: u32,
    /// How it ended; then the frames until the game closes.
    ended: Option<String>,
    closing: Option<u32>,
}

/// One place the hero can stand.
struct Node {
    at: [f32; 3],
    ground: PlayerGround,
    /// The steps on from here: where to, how far, and whether it's a
    /// transporter's hop.
    edges: Vec<(u32, f32, bool)>,
}

/// Where the hero can walk from where it stood when this was made.
struct Map {
    nodes: Vec<Node>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// The steps an item stopped: its placement and the place stepped
    /// from.
    stopped: Vec<(usize, u32)>,
    /// What it was made against ([`key`]).
    key: u64,
}

enum Step {
    To([f32; 3], PlayerGround),
    Item(usize),
    No,
}

/// What the walker looks at.
struct Seen<'a> {
    collision: &'a LevelCollision,
    items: &'a LevelItems,
    mech: &'a Mechanics,
    feet: [f32; 3],
    ground: PlayerGround,
    keys: u32,
    r: f32,
    h: f32,
}

fn level_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).dhypot(a[2] - b[2])
}

/// What a map is made against: the moving parts where they are and the
/// items in the way.
fn key(seen: &Seen) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seen.mech.shape_key().hash(&mut h);
    seen.items.in_way().hash(&mut h);
    h.finish()
}

impl Map {
    fn cell(at: [f32; 3]) -> (i32, i32) {
        ((at[0] / CELL).floor() as i32, (at[2] / CELL).floor() as i32)
    }

    fn add(&mut self, at: [f32; 3], ground: PlayerGround) -> u32 {
        let n = self.nodes.len() as u32;
        self.nodes.push(Node { at, ground, edges: Vec::new() });
        self.cells.entry(Self::cell(at)).or_default().push(n);
        n
    }

    fn at_cell(&self, cell: (i32, i32), y: f32) -> Option<u32> {
        self.cells.get(&cell)?.iter().copied().find(|&n| (self.nodes[n as usize].at[1] - y).abs() < LEVEL)
    }

    /// The place nearest `feet`, if the hero stands about there.
    fn nearest(&self, feet: [f32; 3]) -> Option<u32> {
        let (cx, cz) = Self::cell(feet);
        let mut best: Option<(u32, f32)> = None;
        for (dx, dz) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (dx, dz))) {
            for &n in self.cells.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                let at = self.nodes[n as usize].at;
                let d = level_distance(at, feet);
                if (at[1] - feet[1]).abs() < LEVEL && d < 1.2 && best.is_none_or(|(_, b)| d < b) {
                    best = Some((n, d));
                }
            }
        }
        best.map(|(n, _)| n)
    }

    /// The places within `reach` of `centre` (level distance).
    fn around(&self, centre: [f32; 3], reach: f32) -> impl Iterator<Item = u32> + '_ {
        let (cx, cz) = Self::cell(centre);
        let n = (reach / CELL).ceil() as i32 + 1;
        (-n..=n)
            .flat_map(move |dx| (-n..=n).map(move |dz| (cx + dx, cz + dz)))
            .flat_map(|c| self.cells.get(&c).into_iter().flatten().copied())
            .filter(move |&k| level_distance(self.nodes[k as usize].at, centre) <= reach)
    }

    /// One step of the map: from `from` toward `target` (level) in moves of
    /// at most [`STEP`], as the hero's own move has it, down onto the floor
    /// it comes to.
    fn step(
        collision: &LevelCollision,
        body: &PlayerCollision,
        from: [f32; 3],
        ground: PlayerGround,
        target: [f32; 2],
        stops: &impl Fn([f32; 3], [f32; 3]) -> Option<usize>,
    ) -> Step {
        let (dx, dz) = (target[0] - from[0], target[1] - from[2]);
        let moves = (dx.dhypot(dz) / STEP).ceil().max(1.0);
        let d = [dx / moves, 0.0, dz / moves];
        let (mut at, mut g) = (from, ground);
        for _ in 0..moves as usize {
            let m = collision.move_player(at, d, body, &mut g);
            if m.no_floor || m.delta[0].abs() + m.delta[2].abs() < 1e-3 {
                return Step::No;
            }
            let to = [at[0] + m.delta[0], at[1] + m.delta[1], at[2] + m.delta[2]];
            if let Some(item) = stops(at, to) {
                return Step::Item(item);
            }
            at = to;
        }
        if g.floor <= collision.kill_height() + 0.5 {
            return Step::No;
        }
        at[1] = g.floor;
        Step::To(at, g)
    }

    /// Maps where the hero can walk from where it stands.
    fn build(seen: &Seen) -> Self {
        let collision = seen.collision;
        let body = PlayerCollision::default();
        // The items in the way, in buckets by where they are.
        const BUCKET: f32 = 8.0;
        let bucket = |x: f32, z: f32| ((x / BUCKET).floor() as i32, (z / BUCKET).floor() as i32);
        let mut near: HashMap<(i32, i32), Vec<(usize, Shape)>> = HashMap::new();
        for placement in seen.items.in_way() {
            let Some(view) = seen.items.view(placement) else { continue };
            let s = view.shape;
            // Walls have their triangles about their own place.
            let reach = if s.kind == 4 { 24.0 } else { s.radius + s.half[0].max(s.half[1]) } + seen.r + 2.0 * STEP;
            let (lo, hi) = (bucket(s.centre[0] - reach, s.centre[2] - reach), bucket(s.centre[0] + reach, s.centre[2] + reach));
            for bx in lo.0..=hi.0 {
                for bz in lo.1..=hi.1 {
                    near.entry((bx, bz)).or_default().push((placement, s));
                }
            }
        }
        let stops = |from: [f32; 3], to: [f32; 3]| {
            near.get(&bucket(to[0], to[2]))?.iter().find_map(|&(placement, shape)| {
                let hit = if shape.kind == 4 {
                    seen.items.stops_step(placement, from, to, seen.r, seen.h)
                } else {
                    items::contact(&shape, false, from, to, seen.r, seen.h).is_some()
                };
                hit.then_some(placement)
            })
        };
        // The transporters, and where each sends the hero: onto the floor
        // under its partner.
        let hops: Vec<(Shape, [f32; 3])> = seen
            .items
            .views()
            .filter(|v| v.live && v.ty.class == ItemClass::Transporter)
            .filter_map(|v| {
                let c = seen.items.transporter_to(v.placement)?;
                let floor = collision.floor_probe([c[0], c[1] - 1.0, c[2]], 4.0, -10.0, body.radius, 0)?;
                Some((v.shape, [c[0], floor.point[1], c[2]]))
            })
            .collect();

        let mut map = Map { nodes: Vec::new(), cells: HashMap::new(), stopped: Vec::new(), key: key(seen) };
        let mut queue = VecDeque::from([map.add(seen.feet, seen.ground)]);
        while let Some(n) = queue.pop_front() {
            if map.nodes.len() >= MOST_NODES {
                warn!("walker: the map is full at {MOST_NODES} places");
                break;
            }
            let (from, ground) = (map.nodes[n as usize].at, map.nodes[n as usize].ground);
            let (cx, cz) = Self::cell(from);
            for (dx, dz) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dz| (dx, dz))).filter(|&d| d != (0, 0)) {
                let target = [(cx + dx) as f32 * CELL + 0.5 * CELL, (cz + dz) as f32 * CELL + 0.5 * CELL];
                match Self::step(collision, &body, from, ground, target, &stops) {
                    Step::To(at, g) => {
                        let cell = Self::cell(at);
                        if cell == (cx, cz) && (at[1] - from[1]).abs() < LEVEL {
                            continue;
                        }
                        let cost = level_distance(at, from) + (at[1] - from[1]).abs();
                        let to = map.at_cell(cell, at[1]).unwrap_or_else(|| {
                            let to = map.add(at, g);
                            queue.push_back(to);
                            to
                        });
                        if to != n {
                            map.nodes[n as usize].edges.push((to, cost, false));
                        }
                    }
                    Step::Item(item) => map.stopped.push((item, n)),
                    Step::No => {}
                }
            }
            for &(shape, to_feet) in &hops {
                if items::contact(&shape, true, from, from, seen.r, seen.h).is_none() || level_distance(to_feet, from) < 2.0 {
                    continue;
                }
                let to = map.at_cell(Self::cell(to_feet), to_feet[1]).unwrap_or_else(|| {
                    let to = map.add(to_feet, PlayerGround::new(to_feet[1]));
                    queue.push_back(to);
                    to
                });
                map.nodes[n as usize].edges.push((to, 6.0, true));
            }
        }
        map
    }

    /// The cost of the way to every place from `from`, and the place
    /// before each on it.
    fn routes(&self, from: u32) -> (Vec<f32>, Vec<u32>) {
        let mut cost = vec![f32::INFINITY; self.nodes.len()];
        let mut before = vec![u32::MAX; self.nodes.len()];
        let mut heap = BinaryHeap::new();
        cost[from as usize] = 0.0;
        heap.push(Reverse((0u32, from)));
        while let Some(Reverse((c, n))) = heap.pop() {
            if c as f32 / 64.0 > cost[n as usize] + 0.02 {
                continue;
            }
            for &(to, w, _) in &self.nodes[n as usize].edges {
                let next = cost[n as usize] + w;
                if next < cost[to as usize] {
                    cost[to as usize] = next;
                    before[to as usize] = n;
                    heap.push(Reverse(((next * 64.0) as u32, to)));
                }
            }
        }
        (cost, before)
    }

    fn path(before: &[u32], to: u32) -> Vec<u32> {
        let mut path = vec![to];
        let mut n = to;
        while before[n as usize] != u32::MAX {
            n = before[n as usize];
            path.push(n);
        }
        path.reverse();
        path
    }

    fn hop(&self, from: u32, to: u32) -> bool {
        self.nodes[from as usize].edges.iter().any(|&(t, _, hop)| t == to && hop)
    }

    fn cut(&mut self, from: u32, to: u32) {
        self.nodes[from as usize].edges.retain(|&(t, ..)| t != to);
    }

    /// The reachable place best inside `shape` (nearest its centre) where
    /// the hero touches it.
    fn inside(&self, shape: &Shape, pass: bool, cost: &[f32], (r, h): (f32, f32)) -> Option<u32> {
        self.around(shape.centre, shape.radius + r)
            .filter(|&n| cost[n as usize].is_finite())
            .filter(|&n| {
                let at = self.nodes[n as usize].at;
                items::contact(shape, pass, at, at, r, h).is_some()
            })
            .min_by(|&a, &b| {
                let d = |n: u32| level_distance(self.nodes[n as usize].at, shape.centre);
                d(a).total_cmp(&d(b))
            })
    }

    /// The nearest reachable place within `range` of `target` with nothing
    /// between them.
    fn in_sight(&self, collision: &LevelCollision, target: [f32; 3], range: f32, cost: &[f32]) -> Option<u32> {
        let mut near: Vec<u32> = self
            .around(target, range)
            .filter(|&n| cost[n as usize].is_finite() && (self.nodes[n as usize].at[1] - target[1]).abs() < 6.0)
            .collect();
        near.sort_by(|&a, &b| cost[a as usize].total_cmp(&cost[b as usize]));
        near.into_iter().find(|&n| {
            let at = self.nodes[n as usize].at;
            collision.wall([at[0], at[1] + 2.5, at[2]], target, 0.1).is_none()
        })
    }
}

/// A trigger's touch shape, as the mechanics test it.
fn pad_shape(shape: Shape, switch: &Switch) -> Shape {
    let mut shape = shape;
    if let Some(radius) = switch.radius {
        shape.kind = 1;
        shape.radius = radius;
    } else if switch.subtype == 0x1B {
        shape.radius *= 2.0;
    }
    shape
}

impl Walker {
    fn tried(&self, id: usize) -> Tried {
        self.tried.get(&id).copied().unwrap_or_default()
    }

    /// Whether `id` is worth trying (again).
    fn fresh(&self, id: usize) -> bool {
        let t = self.tried(id);
        t.tries == 0 || (t.tries < MOST_TRIES && t.progress != self.progress)
    }

    fn mark(&mut self, id: usize) {
        let progress = self.progress;
        let t = self.tried.entry(id).or_default();
        t.tries += 1;
        t.progress = progress;
    }

    /// Everything there is to do that the hero can walk to now.
    fn goals(&self, seen: &Seen, map: &Map, cost: &[f32]) -> Vec<Goal> {
        let size = (seen.r, seen.h);
        let switches = seen.mech.switches();
        let turners = seen.mech.turners();
        let mut goals = Vec::new();
        let mut add = |id: usize, kind: Kind, at: [f32; 3], node: Option<u32>| {
            if let Some(node) = node {
                goals.push(Goal { id, kind, at, node, cost: cost[node as usize] });
            }
        };
        for v in seen.items.views().filter(|v| v.live) {
            let (id, at) = (v.placement, v.shape.centre);
            match v.ty.class {
                ItemClass::Exit if v.flags & items::CLOSED == 0 && v.ty.subtype != SECRET_EXIT => {
                    add(id, Kind::Exit, at, map.inside(&v.shape, true, cost, size));
                }
                ItemClass::Trigger => {
                    let Some(s) = switches.iter().find(|s| s.placement == id) else { continue };
                    if s.hit {
                        add(id, Kind::Hit, at, map.in_sight(seen.collision, at, THROW_RANGE, cost));
                    } else if !s.chained {
                        add(id, Kind::Pad, at, map.inside(&pad_shape(v.shape, s), true, cost, size));
                    }
                }
                ItemClass::Rotator if turners.iter().any(|&(p, touched)| p == id && !touched) => {
                    add(id, Kind::Turner, at, map.inside(&v.shape, true, cost, size));
                }
                ItemClass::Powerup if v.ty.subtype == KEY => {
                    add(id, Kind::Key, at, map.inside(&v.shape, false, cost, size));
                }
                _ => {}
            }
        }
        // What stopped a step: the nearest place each was met from.
        let mut met: HashMap<usize, u32> = HashMap::new();
        for &(item, from) in &map.stopped {
            if cost[from as usize].is_finite() && met.get(&item).is_none_or(|&m| cost[from as usize] < cost[m as usize]) {
                met.insert(item, from);
            }
        }
        for (item, from) in met {
            let Some(v) = seen.items.view(item) else { continue };
            let kind = match v.ty.class {
                ItemClass::Door => Kind::Door,
                ItemClass::Generator | ItemClass::EnemyInfo => Kind::Break,
                _ if crate::breakables::hittable(&v) => Kind::Break,
                _ => continue,
            };
            add(item, kind, v.shape.centre, Some(from));
        }
        // Floors that move, each from the place nearest the middle of what
        // can be stood on of it.
        let mut floors: HashMap<usize, (Vec3, Vec<u32>)> = HashMap::new();
        for (n, node) in map.nodes.iter().enumerate() {
            if let Some(g) = node.ground.node.filter(|&g| cost[n].is_finite() && seen.collision.nodes[g].moves()) {
                let e = floors.entry(g).or_default();
                e.0 += Vec3::from(node.at);
                e.1.push(n as u32);
            }
        }
        for (floor, (sum, places)) in floors {
            let middle = (sum / places.len() as f32).to_array();
            let node = places.into_iter().min_by(|&a, &b| {
                let d = |n: u32| level_distance(map.nodes[n as usize].at, middle);
                d(a).total_cmp(&d(b))
            });
            add(RIDES + floor, Kind::Ride, middle, node);
        }
        goals
    }

    /// Looks at the level and picks what to do next.
    fn plan(&mut self, seen: &Seen) -> State {
        let key = key(seen);
        let start = self.map.as_ref().filter(|m| m.key == key).and_then(|m| m.nearest(seen.feet));
        let start = match start {
            Some(n) => n,
            None => {
                let began = std::time::Instant::now();
                let map = Map::build(seen);
                self.maps += 1;
                debug!("walker: map {} of {} places in {:.2} s", self.maps, map.nodes.len(), began.elapsed().as_secs_f32());
                self.map = Some(map);
                0
            }
        };
        let Some(map) = self.map.as_ref() else { return State::Plan };
        let (cost, before) = map.routes(start);
        let goals = self.goals(seen, map, &cost);

        // What there is to do, and how things stand: a change makes what
        // was tried worth trying again.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut ids: Vec<usize> = goals.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        ids.hash(&mut h);
        seen.items.in_way().hash(&mut h);
        seen.keys.hash(&mut h);
        for s in seen.mech.switches() {
            s.on.hash(&mut h);
        }
        let now = h.finish();
        if now != self.seen {
            self.seen = now;
            self.progress += 1;
            self.waited = 0;
        }
        self.reached.extend(goals.iter().map(|g| g.id));

        let go = |goal: Goal| State::Go { goal, path: Map::path(&before, goal.node), next: 0, best: f32::INFINITY, stalled: 0 };
        let exit = goals.iter().filter(|g| g.kind == Kind::Exit && self.tried(g.id).tries < MOST_TRIES).min_by(|a, b| a.cost.total_cmp(&b.cost));
        if let Some(&exit) = exit.filter(|_| !self.all) {
            return go(exit);
        }
        let switches = seen.mech.switches();
        let worth = |g: &&Goal| {
            let t = self.tried(g.id);
            match g.kind {
                Kind::Exit | Kind::Ride => false,
                Kind::Door => seen.keys > 0 && self.fresh(g.id),
                // One that's on already has done what it does.
                Kind::Pad | Kind::Hit => self.fresh(g.id) && !(t.tries > 0 && switches.iter().any(|s| s.placement == g.id && s.on)),
                _ => self.fresh(g.id),
            }
        };
        let order = |g: &Goal| (self.tried(g.id).tries, (g.cost * 16.0) as u32);
        if let Some(&next) = goals.iter().filter(worth).min_by_key(|g| order(g)) {
            return go(next);
        }
        if let Some(&exit) = exit {
            return go(exit);
        }
        if let Some(&ride) = goals.iter().filter(|g| g.kind == Kind::Ride && self.fresh(g.id)).min_by_key(|g| order(g)) {
            return go(ride);
        }
        State::Wait { ticks: 0 }
    }

    /// How it ended, with what it did and never reached.
    fn end(&mut self, seen: &Seen, how: String) {
        let switches = seen.mech.switches();
        let never: Vec<String> = seen
            .items
            .views()
            .filter(|v| v.live && !self.reached.contains(&v.placement))
            .filter_map(|v| {
                let what = match v.ty.class {
                    ItemClass::Exit if v.ty.subtype == SECRET_EXIT => return None,
                    ItemClass::Exit => "exit",
                    ItemClass::Trigger => {
                        let s = switches.iter().find(|s| s.placement == v.placement)?;
                        if s.chained {
                            return None;
                        }
                        if s.hit { "hit switch" } else { "pad" }
                    }
                    ItemClass::Rotator if seen.mech.turners().iter().any(|&(p, _)| p == v.placement) => "turner",
                    ItemClass::Powerup if v.ty.subtype == KEY => "key",
                    _ => return None,
                };
                let c = v.shape.centre;
                Some(format!("{what} {} ({:.0},{:.0},{:.0})", v.placement, c[0], c[1], c[2]))
            })
            .collect();
        let on = switches.iter().filter(|s| s.on).count();
        let line = format!(
            "walker: {} {how} after {} ticks at ({:.1},{:.1},{:.1}); {} of {} triggers on, {} maps; did: [{}]; never reached: [{}]",
            self.level,
            self.ticks,
            seen.feet[0],
            seen.feet[1],
            seen.feet[2],
            on,
            switches.len(),
            self.maps,
            self.done.join(", "),
            never.join(", "),
        );
        info!("{line}");
        self.ended = Some(line);
    }
}

/// A stick pushed toward `to` from `from`, as far as `push`, for a camera
/// looking along `yaw`.
fn stick_toward(from: [f32; 3], to: [f32; 3], push: f32, yaw: f32) -> Vec2 {
    let d = Vec2::new(to[0] - from[0], to[2] - from[2]).normalize_or_zero();
    let forward = Vec2::new(yaw.dsin(), yaw.dcos());
    let right = Vec2::new(forward.y, -forward.x);
    Vec2::new(d.dot(right), d.dot(forward)) * push
}

/// Plays the first local player's hero: its stick and buttons this tick.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn steer(
    mut walker: ResMut<Walker>,
    mut inputs: ResMut<Inputs>,
    (ground, items, mech, population): (Option<Res<LevelGround>>, Option<Res<LevelItems>>, Option<Res<Mechanics>>, Option<Res<LevelPopulation>>),
    (party, camera, boxes, scene): (Res<Party>, Option<Res<PlayCamera>>, Res<crate::message_box::MessageBox>, Res<crate::tower_scenes::Scene>),
    players: Query<&Player>,
    targets: Query<(&GlobalTransform, &Targetable)>,
) {
    let (Some(ground), Some(items), Some(mech), Some(population)) = (ground, items, mech, population) else { return };
    let Some((slot, _)) = party.members().find(|(_, m)| !m.devices.remote) else { return };
    let Some(player) = players.iter().find(|p| p.slot == slot) else { return };
    let Some(state) = party.state(slot) else { return };
    let w = &mut *walker;
    if w.ended.is_none() && w.level != population.level {
        // A new level: from the start (the first level's end closes the
        // game before another comes).
        *w = Walker { all: w.all, level: population.level.clone(), ..default() };
        info!("walker: {} begins", w.level);
    }
    let mut set = |stick: Vec2, buttons: u32| {
        let input = &mut inputs.slots[slot];
        input.stick = stick;
        input.held = (input.held & !input.buttons()) | buttons;
    };
    if w.ended.is_some() {
        set(Vec2::ZERO, 0);
        return;
    }
    // The pads aren't read: nothing to do but wait.
    let cut = camera.as_ref().is_some_and(|c| c.in_cut() || c.opening());
    if !state.alive || player.grabbed.is_some() || cut || boxes.holds_input() || scene.holds_input() {
        set(Vec2::ZERO, 0);
        return;
    }
    let feet = player.mover.position;
    let seen = Seen {
        collision: &ground.0,
        items: &items,
        mech: &mech,
        feet,
        ground: player.ground,
        keys: state.keys,
        r: state.radius,
        h: state.half_height,
    };
    w.ticks += 1;
    if items.leaving() {
        w.end(&seen, "FINISHED".into());
        set(Vec2::ZERO, 0);
        return;
    }
    let yaw = camera.as_ref().map_or(0.0, |c| c.yaw_of(slot));
    // Moved by something else than its own feet (carried, put back at the
    // start, an unexpected transporter): look again.
    let jumped = w.last_feet.is_some_and(|l| level_distance(l, feet) > 3.0 || (l[1] - feet[1]).abs() > 3.0);
    w.last_feet = Some(feet);

    // Monsters and generators near, in sight: fought first, for a while.
    let centre = Vec3::from(feet) + Vec3::Y * 2.5;
    let foe = targets
        .iter()
        .filter(|(_, t)| matches!(t.kind, TargetKind::Monster | TargetKind::Generator) && !t.boss && t.critter.is_none())
        .map(|(at, t)| (at.translation(), centre.distance(at.translation()) - t.radius))
        .filter(|&(at, d)| d < FIGHT_RANGE && (at.y - centre.y).abs() < 4.0)
        .filter(|&(at, _)| ground.0.wall(centre.to_array(), [at.x, centre.y, at.z], 0.1).is_none())
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if w.rest > 0 {
        w.rest -= 1;
    } else if let Some((at, _)) = foe.filter(|_| !matches!(w.state, State::Act { goal: Goal { kind: Kind::Exit, .. }, .. })) {
        w.fought += 1;
        if w.fought > FIGHT {
            (w.fought, w.rest) = (0, REST);
        }
        set(stick_toward(feet, at.to_array(), 0.3, yaw), button::QUICK);
        return;
    } else {
        w.fought = 0;
    }

    let state_now = std::mem::take(&mut w.state);
    let (next, stick, buttons) = match state_now {
        State::Plan => (w.plan(&seen), Vec2::ZERO, 0),
        State::Go { goal, path, mut next, mut best, mut stalled } => 'go: {
            let Some(map) = w.map.as_mut() else { break 'go (State::Plan, Vec2::ZERO, 0) };
            // Past each place it has come to.
            while next < path.len() {
                let at = map.nodes[path[next] as usize].at;
                if level_distance(at, feet) < ARRIVE && (at[1] - feet[1]).abs() < 3.0 {
                    (next, best, stalled) = (next + 1, f32::INFINITY, 0);
                } else {
                    break;
                }
            }
            if next >= path.len() {
                break 'go (State::Act { goal, ticks: 0 }, Vec2::ZERO, 0);
            }
            let at = map.nodes[path[next] as usize].at;
            let hop = next > 0 && map.hop(path[next - 1], path[next]);
            let d = level_distance(at, feet);
            if jumped && !hop {
                break 'go (State::Plan, Vec2::ZERO, 0);
            }
            if d < best - 0.05 {
                (best, stalled) = (d, 0);
            } else {
                stalled += 1;
            }
            if stalled > if hop { 150 } else { STUCK } {
                // It can't get there: not that way again.
                if next > 0 {
                    debug!("walker: no way from {:?} on to {at:?}", map.nodes[path[next - 1] as usize].at);
                    map.cut(path[next - 1], path[next]);
                } else {
                    w.map = None;
                }
                break 'go (State::Plan, Vec2::ZERO, 0);
            }
            // A transporter takes it there once it stands on it.
            let stick = if hop { Vec2::ZERO } else { stick_toward(feet, at, 1.0, yaw) };
            (State::Go { goal, path, next, best, stalled }, stick, 0)
        }
        State::Act { goal, ticks } => 'act: {
            let ticks = ticks + 1;
            let switch = seen.mech.switches().into_iter().find(|s| s.placement == goal.id);
            let on = switch.is_some_and(|s| s.on);
            let in_way = seen.items.in_way().contains(&goal.id);
            let toward = stick_toward(feet, goal.at, 1.0, yaw);
            let stay = |stick: Vec2, buttons: u32| (State::Act { goal, ticks }, stick, buttons);
            let name = seen.items.view(goal.id).map(|v| v.ty.name.clone()).unwrap_or_default();
            let finish = |w: &mut Walker, what: String, next: State| {
                debug!("walker: {what}");
                w.done.push(what);
                w.mark(goal.id);
                (next, Vec2::ZERO, 0)
            };
            match goal.kind {
                Kind::Exit => {
                    if ticks > 450 {
                        break 'act finish(w, format!("exit {} didn't take the hero", goal.id), State::Plan);
                    }
                    stay(Vec2::ZERO, 0)
                }
                Kind::Pad => {
                    if on && ticks >= 5 {
                        break 'act finish(w, format!("pad {} {name} on", goal.id), State::Settle { ticks: 0 });
                    }
                    if ticks > 60 {
                        break 'act finish(w, format!("pad {} {name} stayed off", goal.id), State::Settle { ticks: 0 });
                    }
                    stay(Vec2::ZERO, 0)
                }
                Kind::Turner => {
                    let touched = seen.mech.turners().iter().any(|&(p, t)| p == goal.id && t);
                    if touched || ticks > 60 {
                        let how = if touched { "turning" } else { "didn't turn" };
                        break 'act finish(w, format!("turner {} {how}", goal.id), State::Settle { ticks: 0 });
                    }
                    stay(Vec2::ZERO, 0)
                }
                Kind::Key => {
                    let taken = seen.items.view(goal.id).is_none_or(|v| !v.live);
                    if taken || ticks > 45 {
                        let how = if taken { "taken" } else { "not taken" };
                        break 'act finish(w, format!("key {} {how}", goal.id), State::Plan);
                    }
                    stay(toward * 0.3, 0)
                }
                Kind::Door => {
                    if !in_way {
                        break 'act finish(w, format!("door {} {name} open", goal.id), State::Plan);
                    }
                    if ticks > 150 {
                        break 'act finish(w, format!("door {} {name} stayed shut", goal.id), State::Plan);
                    }
                    stay(toward, 0)
                }
                Kind::Hit => {
                    if on {
                        break 'act finish(w, format!("hit switch {} {name} on", goal.id), State::Settle { ticks: 0 });
                    }
                    if ticks > 240 {
                        break 'act finish(w, format!("hit switch {} {name} stayed off", goal.id), State::Plan);
                    }
                    stay(toward * 0.3, if ticks > 6 { button::QUICK } else { 0 })
                }
                Kind::Break => {
                    if !in_way {
                        break 'act finish(w, format!("{name} {} out of the way", goal.id), State::Plan);
                    }
                    if ticks > 300 {
                        break 'act finish(w, format!("{name} {} stayed in the way", goal.id), State::Plan);
                    }
                    stay(toward * 0.3, if ticks > 6 { button::QUICK } else { 0 })
                }
                Kind::Ride => finish(w, format!("floor {} stood on", goal.id - RIDES), State::Wait { ticks: 0 }),
            }
        }
        State::Settle { ticks } => {
            // Whatever it set going comes to rest (the hero carried with
            // it); then at once, before a lift sets off again.
            if (seen.mech.on_the_move() && ticks < 1800) || ticks < 3 {
                (State::Settle { ticks: ticks + 1 }, Vec2::ZERO, 0)
            } else {
                (w.plan(&seen), Vec2::ZERO, 0)
            }
        }
        State::Wait { ticks } => {
            if ticks < LOOK_EVERY {
                (State::Wait { ticks: ticks + 1 }, Vec2::ZERO, 0)
            } else {
                w.waited += LOOK_EVERY;
                let riding = seen.ground.node.is_some_and(|n| seen.collision.nodes[n].moves());
                let patience = if riding { RIDE_PATIENCE } else { PATIENCE };
                let next = w.plan(&seen);
                if matches!(next, State::Wait { .. }) && w.waited > patience && !seen.mech.on_the_move() {
                    w.end(&seen, "STUCK".into());
                }
                (next, Vec2::ZERO, 0)
            }
        }
    };
    w.state = next;
    set(stick, buttons);
}

/// Once it has ended: the picture, then the game closes.
fn close(mut walker: ResMut<Walker>, mut commands: Commands, mut exit: MessageWriter<AppExit>) {
    if walker.ended.is_none() {
        return;
    }
    match walker.closing {
        None => {
            if let Some(path) = std::env::var_os("GDL_WALK_SHOT") {
                use bevy::render::view::screenshot::{Screenshot, save_to_disk};
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(std::path::PathBuf::from(path)));
            }
            walker.closing = Some(CLOSE_AFTER);
        }
        Some(0) => {
            exit.write(AppExit::Success);
        }
        Some(n) => walker.closing = Some(n - 1),
    }
}
