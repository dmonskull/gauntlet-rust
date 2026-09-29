//! Level collision: `WORLDS.PS2`'s collision triangles, the X/Z grid that
//! indexes them, and the queries the game runs against them every frame.
//!
//! Confirmed against `main.dol` (addresses and evidence in
//! `docs/collision.md`):
//!
//! - Collision is a separate, simplified triangle soup, not the render
//!   meshes. Each world node owns a contiguous run of triangles (node
//!   `+0x36` count, `+0x38` first). Triangles of static nodes are in world
//!   space; triangles of nodes that move (flags `0x1000` / `0x1000000`) are
//!   relative to the node, and the game transforms the query into the node's
//!   frame instead.
//! - A triangle is stored in its own plane frame: an origin (its first
//!   corner), the plane normal, a frame scale `1/sqrt(1 - ny²)`, and the
//!   other two corners as 16-bit in-plane coordinates in 1/64 units, plus a
//!   quantised height range for a cheap reject.
//! - A grid of square cells over the level's X/Z bounds lists, per cell,
//!   which nodes' triangles touch it. Cell 0 is reserved for the nodes that
//!   move; the game rebuilds their grid at runtime.
//! - Queries are swept spheres: a segment plus a radius, filtered by node
//!   flag bits and by the triangle normal's Y (floors vs walls), keeping the
//!   nearest hit. Only a triangle's front face collides.
//! - Actors and players resolve a move with different chains built on those
//!   queries: [`LevelCollision::move_actor`] and
//!   [`LevelCollision::move_player`].

use std::ops::Range;

use crate::world::{WorldError, WorldFile};

pub const TRIANGLE_STRIDE: usize = 0x28;
/// In-plane corner coordinates are stored as integer 1/64ths.
const PLANE_UNITS: f32 = 1.0 / 64.0;
/// Height ranges are stored as integer 1/64ths too.
const HEIGHT_STEPS: f32 = 64.0;
/// Beyond this |normal.y| the game treats a triangle as exactly horizontal
/// and uses the world axes as its plane frame.
const FLAT: f32 = 0.999_999;
/// Segments shorter than this in X/Z are treated as vertical.
const VERTICAL: f32 = 0.001;
/// The game's "no hit yet" score.
const NO_SCORE: f32 = 1.0e21;

/// Node flag bits the game's queries select on (node `+0x10`).
pub mod node_flags {
    /// Walls query: `0x13A`.
    pub const WALLS: u32 = 0x13A;
    /// Floor probes: `0x23C`.
    pub const FLOORS: u32 = 0x23C;
    /// Generic ray (both): `0x23E`.
    pub const ANY: u32 = 0x23E;
    /// Triangles are relative to the node's transform (it can move).
    pub const MOVES: u32 = 0x1000;
    /// Triangles are relative to the node's full matrix.
    pub const MOVES_MATRIX: u32 = 0x100_0000;
    /// Skip the normal and height filters for this node.
    pub const NO_FILTER: u32 = 0x40;
    /// Hits on this node are reported separately, never as the nearest hit.
    pub const SECONDARY: u32 = 0x200;
    /// Wall hits on these nodes don't push the mover out.
    pub const NO_PUSH: u32 = 0x38;
    /// Excluded from the moving-node pass.
    pub const DYNAMIC_OFF: u32 = 0x1000_0000;
}

/// One collision triangle, as stored (`0x28` bytes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionTriangle {
    /// `+0x00`, `+0x02`: lowest / highest corner height in 1/64 units.
    pub height_range: [i16; 2],
    /// `+0x04`: `1 / sqrt(1 - normal.y²)` (infinite for horizontal
    /// triangles, where it's unused).
    pub frame_scale: f32,
    /// `+0x08`: unit plane normal; the front face is the side it points to.
    pub normal: [f32; 3],
    /// `+0x14`: the first corner.
    pub origin: [f32; 3],
    /// `+0x20`: the second and third corners in the plane frame, 1/64 units.
    pub corners: [[i16; 2]; 2],
}

impl CollisionTriangle {
    fn parse(r: &[u8]) -> Self {
        let f = |at| le_f32(r, at);
        let h = |at| le_u16(r, at) as i16;
        Self {
            height_range: [h(0), h(2)],
            frame_scale: f(4),
            normal: [f(8), f(12), f(16)],
            origin: [f(0x14), f(0x18), f(0x1C)],
            corners: [[h(0x20), h(0x22)], [h(0x24), h(0x26)]],
        }
    }

    /// A point relative to the origin, in the plane frame: `x`, `z` along the
    /// plane and `y` the signed height above it.
    pub fn to_plane(&self, p: [f32; 3]) -> [f32; 3] {
        let [nx, ny, nz] = self.normal;
        let s = self.frame_scale;
        if ny > FLAT {
            return p;
        }
        if ny < -FLAT {
            return [p[0], -p[1], -p[2]];
        }
        [
            -p[0] * nz * s + p[2] * nx * s,
            p[0] * nx + p[1] * ny + p[2] * nz,
            s * (-ny * nx * p[0] - ny * nz * p[2] + (1.0 - ny * ny) * p[1]),
        ]
    }

    /// Inverse of [`to_plane`](Self::to_plane).
    pub fn from_plane(&self, l: [f32; 3]) -> [f32; 3] {
        let [nx, ny, nz] = self.normal;
        let s = self.frame_scale;
        let [x, y, z] = l;
        if ny > FLAT {
            return l;
        }
        if ny < -FLAT {
            return [x, -y, -z];
        }
        [
            ny * (-z * nx * s) + (-x * nz * s + y * nx),
            y * ny + s * (z * (1.0 - ny * ny)),
            nz * s * (-z * ny) + (x * nx * s + y * nz),
        ]
    }

    /// The three corners in the plane frame's X/Z.
    fn plane_corners(&self) -> [[f32; 2]; 3] {
        let c = |k: usize| [self.corners[k][0] as f32 * PLANE_UNITS, self.corners[k][1] as f32 * PLANE_UNITS];
        [[0.0, 0.0], c(0), c(1)]
    }

    /// The corners in the triangle's own space (world space for static
    /// nodes, node space for moving ones).
    pub fn vertices(&self) -> [[f32; 3]; 3] {
        self.plane_corners().map(|[x, z]| add(self.from_plane([x, 0.0, z]), self.origin))
    }

    /// The swept-sphere test: a segment from `start` to `end` with `radius`
    /// against this triangle's front face. Returns the squared distance (0
    /// when the segment crosses the triangle, or passes over it within the
    /// radius) and the closest point on the triangle.
    pub fn sweep(&self, start: [f32; 3], end: [f32; 3], radius: f32) -> Option<(f32, [f32; 3])> {
        let a = self.to_plane(sub(start, self.origin));
        if a[1] < 0.0 {
            return None; // behind the face
        }
        let b = self.to_plane(sub(end, self.origin));
        if b[1] > a[1] {
            return None; // moving away from it
        }
        let r2 = radius * radius;
        let [v0, v1, v2] = self.plane_corners();
        let edges = [(v0, v1), (v1, v2), (v2, v0)];
        let outside = |e: ([f32; 2], [f32; 2]), p: [f32; 2]| {
            (e.1[0] - e.0[0]) * (p[1] - e.0[1]) - (e.1[1] - e.0[1]) * (p[0] - e.0[0]) > 0.0
        };

        let mut crossing = !((a[1] > 0.0 && b[1] > 0.0) || (a[1] < 0.0 && b[1] < 0.0));
        let mut at = [0.0f32, 0.0];
        if !crossing {
            if (radius < a[1] && radius < b[1]) || (a[1] < -radius && b[1] < -radius) {
                return None;
            }
        } else {
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let sum = a[1].abs() + b[1].abs();
            if (dx * dx + dz * dz).sqrt() <= VERTICAL {
                at = [a[0], a[2]];
            } else if sum == 0.0 {
                crossing = false; // lies in the plane
            } else {
                let t = a[1].abs() / sum;
                at = [a[0] + dx * t, a[2] + dz * t];
            }
        }

        let (dist, point) = if crossing {
            // Where it pierces the plane; if that's outside, the nearest
            // edge it's outside of.
            let mut best: Option<(f32, [f32; 3])> = None;
            for e in edges {
                if outside(e, at) {
                    let d = edge_distance(a, b, e, true);
                    if best.is_none_or(|(bd, _)| d.0 < bd) {
                        best = Some(d);
                    }
                }
            }
            match best {
                None => (0.0, [at[0], 0.0, at[1]]),
                Some((d, _)) if d > r2 => return None,
                Some(hit) => hit,
            }
        } else {
            // Doesn't cross: hovering over it within the radius counts as a
            // touch where the nearer end projects inside.
            let inside = |p: [f32; 3]| edges.iter().all(|&e| !outside(e, [p[0], p[2]]));
            let start_nearer = b[1].abs() > a[1].abs() || a[1] == b[1];
            let end_nearer = b[1].abs() <= a[1].abs();
            if start_nearer && inside(a) {
                (0.0, [a[0], 0.0, a[2]])
            } else if end_nearer && inside(b) {
                (0.0, [b[0], 0.0, b[2]])
            } else {
                let best = edges
                    .iter()
                    .map(|&e| edge_distance(a, b, e, false))
                    .fold((f32::INFINITY, [0.0; 3]), |m, d| if d.0 < m.0 { d } else { m });
                if best.0 > r2 {
                    return None;
                }
                best
            }
        };
        Some((dist, add(self.from_plane(point), self.origin)))
    }
}

/// Squared distance between the segment `a..b` (plane frame) and a triangle
/// edge lying in the plane, with the closest point on the edge. A segment
/// that's vertical in X/Z uses the horizontal distance plus, when it doesn't
/// cross the plane, the height of its nearer end.
fn edge_distance(a: [f32; 3], b: [f32; 3], edge: ([f32; 2], [f32; 2]), crossing: bool) -> (f32, [f32; 3]) {
    let e0 = [edge.0[0], 0.0, edge.0[1]];
    let e1 = [edge.1[0], 0.0, edge.1[1]];
    let ab = sub(b, a);
    if (ab[0] * ab[0] + ab[2] * ab[2]).sqrt() >= VERTICAL {
        let (on_ab, on_edge) = closest_segment_points(a, b, e0, e1);
        let d = sub(on_ab, on_edge);
        (dot(d, d), on_edge)
    } else {
        let p = closest_on_segment_xz([a[0], a[2]], e0, e1);
        let (dx, dz) = (a[0] - p[0], a[2] - p[2]);
        let h = if crossing { 0.0 } else { a[1].abs().min(b[1].abs()) };
        (dx * dx + dz * dz + h * h, p)
    }
}

fn closest_on_segment_xz(p: [f32; 2], e0: [f32; 3], e1: [f32; 3]) -> [f32; 3] {
    let d = [e1[0] - e0[0], e1[2] - e0[2]];
    let len2 = d[0] * d[0] + d[1] * d[1];
    if len2 == 0.0 {
        return e0;
    }
    let t = (((p[0] - e0[0]) * d[0] + (p[1] - e0[2]) * d[1]) / len2).clamp(0.0, 1.0);
    [e0[0] + d[0] * t, 0.0, e0[2] + d[1] * t]
}

/// Closest points between segments `p0..p1` and `q0..q1`.
fn closest_segment_points(p0: [f32; 3], p1: [f32; 3], q0: [f32; 3], q1: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let d1 = sub(p1, p0);
    let d2 = sub(q1, q0);
    let r = sub(p0, q0);
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (s, t);
    if a <= f32::EPSILON && e <= f32::EPSILON {
        return (p0, q0);
    }
    if a <= f32::EPSILON {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if e <= f32::EPSILON {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let s0 = if denom != 0.0 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (add(p0, scale(d1, s)), add(q0, scale(d2, t)))
}

/// One row of the grid: the columns it stores and where its cells start.
/// Empty rows are stored as first `0xFFFF`, last `0xFFFE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridRow {
    pub first_column: u16,
    pub last_column: u16,
    pub first_cell: u32,
}

/// Nodes whose triangles touch one cell, and which of their triangles
/// (indices relative to the node's first triangle).
#[derive(Debug, Clone, Copy)]
pub struct CellEntry<'a> {
    pub node: usize,
    pub triangles: &'a [u16],
}

/// The X/Z lookup grid (header words 5, 7, 8 and 15–17).
#[derive(Debug, Clone)]
pub struct CollisionGrid {
    /// Level bounds minimum X and Z: the grid's corner.
    pub origin: [f32; 2],
    pub cell_size: f32,
    pub columns: u32,
    pub rows: Vec<GridRow>,
    /// Per cell: entry count in the top 10 bits, byte offset into `lists`
    /// in the low 22.
    pub cells: Vec<u32>,
    /// Cell entries: `node, count, count × triangle` as 16-bit values.
    pub lists: Vec<u16>,
}

impl CollisionGrid {
    /// The cell covering a world X/Z, clamped to the grid like the game.
    pub fn locate(&self, x: f32, z: f32) -> (u32, u32) {
        let inv = if self.cell_size == 0.0 { 1.0 } else { 1.0 / self.cell_size };
        let clamp = |v: f32, n: u32| (v as i32).clamp(0, n.saturating_sub(1) as i32) as u32;
        (clamp(inv * (x - self.origin[0]), self.columns), clamp(inv * (z - self.origin[1]), self.rows.len() as u32))
    }

    fn cell(&self, column: u32, row: u32) -> Option<u32> {
        let r = self.rows.get(row as usize)?;
        if column < r.first_column as u32 || column > r.last_column as u32 {
            return None;
        }
        self.cells.get((r.first_cell + column - r.first_column as u32) as usize).copied()
    }

    /// The entries stored for a cell (nothing outside a row's span).
    pub fn entries(&self, column: u32, row: u32) -> Vec<CellEntry<'_>> {
        let mut out = Vec::new();
        let Some(cell) = self.cell(column, row) else { return out };
        let mut at = (cell & 0x3F_FFFF) as usize / 2;
        for _ in 0..cell >> 22 {
            let (Some(&node), Some(&count)) = (self.lists.get(at), self.lists.get(at + 1)) else { break };
            let Some(triangles) = self.lists.get(at + 2..at + 2 + count as usize) else { break };
            out.push(CellEntry { node: node as usize, triangles });
            at += 2 + count as usize;
        }
        out
    }

    /// Cell 0: the nodes that move, as a plain list of node indices.
    pub fn moving_nodes(&self) -> Vec<usize> {
        let Some(&cell) = self.cells.first() else { return Vec::new() };
        let at = (cell & 0x3F_FFFF) as usize / 2;
        let n = (cell >> 22) as usize;
        self.lists.get(at..at + n).map(|s| s.iter().map(|&v| v as usize).collect()).unwrap_or_default()
    }
}

/// The collision tables of a `WORLDS.PS2`.
#[derive(Debug, Clone)]
pub struct CollisionTables {
    pub triangles: Vec<CollisionTriangle>,
    pub grid: CollisionGrid,
}

impl CollisionTables {
    /// Header words: 2/3 triangle count/offset, 5 cells, 7 cell lists,
    /// 8 grid rows (one per grid row, word 17), 9/11 bounds min X/Z,
    /// 15 cell size, 16 columns. The table sizes follow from the offsets
    /// (lists, then cells, then rows), which is how the game byte-swaps them.
    pub(crate) fn parse(file: &[u8], words: &[u32; 30]) -> Result<Self, WorldError> {
        let bad = |why: String| WorldError::BadCollision(why);
        let (count, at) = (words[2] as usize, words[3] as usize);
        let table = slice(file, at, count * TRIANGLE_STRIDE)?;
        let triangles = table.chunks(TRIANGLE_STRIDE).map(CollisionTriangle::parse).collect();

        let (lists_at, cells_at, rows_at) = (words[7] as usize, words[5] as usize, words[8] as usize);
        let lists_len = cells_at.checked_sub(lists_at).ok_or_else(|| bad("cell lists after cells".into()))?;
        let cells_len = rows_at.checked_sub(cells_at).ok_or_else(|| bad("cells after grid rows".into()))?;
        let lists = slice(file, lists_at, lists_len / 2 * 2)?.chunks(2).map(|c| le_u16(c, 0)).collect();
        let cells = slice(file, cells_at, cells_len / 4 * 4)?.chunks(4).map(|c| le_u32(c, 0)).collect();
        let rows = slice(file, rows_at, words[17] as usize * 8)?
            .chunks(8)
            .map(|r| GridRow { first_column: le_u16(r, 0), last_column: le_u16(r, 2), first_cell: le_u32(r, 4) })
            .collect();
        let f = |i: usize| f32::from_bits(words[i]);
        let grid = CollisionGrid { origin: [f(9), f(11)], cell_size: f(15), columns: words[16], rows, cells, lists };
        Ok(Self { triangles, grid })
    }

    /// Checks every grid reference against the nodes' triangle runs.
    pub(crate) fn validate(&self, nodes: &[crate::world::WorldNode]) -> Result<(), WorldError> {
        let bad = |why: String| Err(WorldError::BadCollision(why));
        for (i, n) in nodes.iter().enumerate() {
            if n.collision.end > self.triangles.len() {
                return bad(format!("node {i} triangles {:?} past {}", n.collision, self.triangles.len()));
            }
        }
        let g = &self.grid;
        for (z, r) in g.rows.iter().enumerate() {
            if r.first_column > r.last_column {
                continue;
            }
            if r.last_column as u32 >= g.columns || (r.first_cell + (r.last_column - r.first_column) as u32) as usize >= g.cells.len() {
                return bad(format!("grid row {z} {r:?} out of range"));
            }
            for x in r.first_column as u32..=r.last_column as u32 {
                let cell = g.cells[(r.first_cell + x - r.first_column as u32) as usize];
                let entries = g.entries(x, z as u32);
                if entries.len() != (cell >> 22) as usize {
                    return bad(format!("cell {x},{z} runs past the cell lists"));
                }
                for e in entries {
                    let n = nodes.get(e.node).ok_or_else(|| WorldError::BadCollision(format!("cell {x},{z} names node {}", e.node)))?;
                    if e.triangles.iter().any(|&t| t as usize >= n.collision.len()) {
                        return bad(format!("cell {x},{z} names a triangle node {} doesn't have", e.node));
                    }
                }
            }
        }
        if g.moving_nodes().iter().any(|&n| n >= nodes.len()) {
            return bad("moving-node list names a missing node".into());
        }
        Ok(())
    }
}

/// A node as the collision code sees it.
#[derive(Debug, Clone)]
pub struct CollisionNode {
    pub flags: u32,
    /// Node `+0x35`, OR'ed with its parent's; a query skips nodes sharing a
    /// bit with its mask.
    pub disable: u8,
    /// World position: the bounding sphere's centre, and for moving nodes
    /// the origin of their triangles.
    pub center: [f32; 3],
    pub radius: f32,
    pub triangles: Range<usize>,
}

impl CollisionNode {
    pub fn moves(&self) -> bool {
        self.flags & (node_flags::MOVES | node_flags::MOVES_MATRIX) != 0
    }
}

/// What a query looks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Query {
    pub radius: f32,
    /// Node flag bits, any of which selects a node.
    pub node_mask: u32,
    /// Skip nodes whose disable byte shares a bit with this.
    pub disable_mask: u8,
    /// Accepted range of the triangle normal's Y.
    pub normal_y: [f32; 2],
    /// Rank crossings above near misses (the game's query flag `0x10`):
    /// a miss within the radius scores 10000 × its squared distance.
    pub prefer_crossing: bool,
}

impl Query {
    /// The walls query: surfaces at least 30° from horizontal on either
    /// side (|ny| ≤ 0.866), nodes `0x13A`.
    pub fn walls(radius: f32) -> Self {
        Self { radius, node_mask: node_flags::WALLS, disable_mask: 2, normal_y: [-0.866, 0.866], prefer_crossing: false }
    }

    /// The floor probe: up-facing surfaces at most 60° from horizontal
    /// (ny ≥ 0.5), nodes `0x23C`.
    pub fn floors(radius: f32) -> Self {
        Self { radius, node_mask: node_flags::FLOORS, disable_mask: 0, normal_y: [0.5, 2.0], prefer_crossing: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub node: usize,
    pub triangle: usize,
    /// Closest point on the triangle, world space.
    pub point: [f32; 3],
    pub normal: [f32; 3],
    /// What the nearest hit was chosen by (lower is nearer).
    pub score: f32,
}

/// A level's collision, placed in the world, ready to query.
#[derive(Debug, Clone)]
pub struct LevelCollision {
    pub triangles: Vec<CollisionTriangle>,
    pub grid: CollisionGrid,
    pub nodes: Vec<CollisionNode>,
    pub moving: Vec<usize>,
    pub bounds: [[f32; 3]; 2],
}

/// Per-actor movement settings for [`LevelCollision::move_actor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveParams {
    /// Wall test radius (from the actor's type data).
    pub radius: f32,
    /// The floor probe starts this far above the feet and ends this far
    /// plus `probe_extra_down` below (from the actor's type data).
    pub step: f32,
    /// 3.0 in the game.
    pub probe_extra_down: f32,
    /// Radius of the floor probe: 1.0 in the game.
    pub probe_radius: f32,
    /// The furthest the actor may drop in one move (the game: 16 × frame
    /// time).
    pub max_drop: f32,
}

impl MoveParams {
    pub fn new(radius: f32, step: f32, max_drop: f32) -> Self {
        Self { radius, step, probe_extra_down: 3.0, probe_radius: 1.0, max_drop }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moved {
    /// The move actually allowed.
    pub delta: [f32; 3],
    /// A wall stopped the horizontal move.
    pub blocked_by_wall: bool,
    /// No acceptable floor ahead: the horizontal move was cancelled.
    pub no_floor: bool,
    pub wall: Option<Hit>,
    pub floor: Option<Hit>,
}

/// A player's collision shape for [`LevelCollision::move_player`], from the
/// class's `PDAT` body measurements (the same for every class on the disc).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerCollision {
    /// Wall and floor test radius (`PDAT +0x4C`): 1.5.
    pub radius: f32,
    /// Half the class height (`PDAT +0x48` × 0.5): 2.5. The floor probe
    /// reaches this plus 3 below the collision centre.
    pub half_height: f32,
    /// Collision centre above the feet (`PDAT +0x54`): 2.5. The wall and
    /// floor tests start here, so this is also the highest step a player
    /// can walk up.
    pub centre_height: f32,
    /// The furthest the player drops towards the floor per move: the game's
    /// 16 units/s × the tick (1/30 s).
    pub max_drop: f32,
}

impl PlayerCollision {
    /// The player tick, seconds.
    pub const TICK: f32 = 1.0 / 30.0;
    /// Drop speed towards the floor, units per second.
    pub const DROP_SPEED: f32 = 16.0;

    pub fn from_body(body: &crate::pdata::PlayerBody, tick: f32) -> Self {
        Self {
            radius: body.radius,
            half_height: 0.5 * body.height,
            centre_height: body.centre_height,
            max_drop: Self::DROP_SPEED * tick,
        }
    }
}

impl Default for PlayerCollision {
    /// What every class uses, at the game's 30 Hz tick.
    fn default() -> Self {
        let body = crate::pdata::PlayerBody { height: 5.0, radius: 1.5, head_height: 4.4, centre_height: 2.5 };
        Self::from_body(&body, Self::TICK)
    }
}

/// What the player's floor follow remembers between moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerGround {
    /// The floor height the player's feet are moving towards. With no floor
    /// below it's the level's [`kill_height`](LevelCollision::kill_height),
    /// so the player falls.
    pub floor: f32,
    /// The node the player last stood on.
    pub node: Option<usize>,
}

impl PlayerGround {
    /// Standing on a floor at height `floor` (usually the feet's height).
    pub fn new(floor: f32) -> Self {
        Self { floor, node: None }
    }
}

/// How the player's floor check ended (the game's return codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloorCheck {
    /// No floor under the destination: horizontal move cancelled (−2).
    Lost,
    /// Horizontal move cancelled, or standing still on the same floor (0).
    Held { cancelled: bool },
    /// Floor accepted where the probe landed (1).
    Level,
    /// Floor accepted after being pulled back to a ledge or along a wall,
    /// or moving along a ledge (2).
    Adjusted,
}

impl LevelCollision {
    pub fn new(world: &WorldFile) -> Result<Self, WorldError> {
        let positions = world.world_positions()?;
        let mut parent = vec![None; world.nodes.len()];
        for (i, n) in world.nodes.iter().enumerate() {
            let mut c = n.first_child;
            while let Some(k) = c {
                if parent[k].is_some() {
                    return Err(WorldError::Cycle(k));
                }
                parent[k] = Some(i);
                c = world.nodes[k].next_sibling;
            }
        }
        let nodes = world
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| CollisionNode {
                flags: n.flags,
                disable: n.collision_disable | parent[i].map_or(0, |p: usize| world.nodes[p].collision_disable),
                center: positions[i].unwrap_or(n.local_position),
                radius: n.radius,
                triangles: n.collision.clone(),
            })
            .collect();
        let h = &world.header;
        Ok(Self {
            triangles: world.collision.triangles.clone(),
            grid: world.collision.grid.clone(),
            nodes,
            moving: world.collision.grid.moving_nodes(),
            bounds: [h.bounds_min, h.bounds_max],
        })
    }

    /// Each triangle's corners in world space (moving nodes at rest), with
    /// its owning node.
    pub fn world_triangles(&self) -> impl Iterator<Item = (usize, usize, [[f32; 3]; 3])> + '_ {
        self.nodes.iter().enumerate().flat_map(move |(ni, n)| {
            n.triangles.clone().map(move |ti| {
                let off = if n.moves() { n.center } else { [0.0; 3] };
                (ni, ti, self.triangles[ti].vertices().map(|v| add(v, off)))
            })
        })
    }

    /// The nearest triangle a sphere of `q.radius` hits moving from `from`
    /// to `to`: the game's segment query. Grid cells are gathered from the
    /// segment's X/Z box (a superset of the cells the game walks).
    pub fn cast(&self, from: [f32; 3], to: [f32; 3], q: &Query) -> Option<Hit> {
        self.cast_skipping(from, to, q, None)
    }

    /// [`cast`](Self::cast), ignoring one triangle. The game gets the same
    /// effect by bumping the triangle's "visited" byte so the next query's
    /// stamp already matches it.
    fn cast_skipping(&self, from: [f32; 3], to: [f32; 3], q: &Query, skip: Option<usize>) -> Option<Hit> {
        let r = q.radius;
        let delta = sub(to, from);
        let len = dot(delta, delta).sqrt();
        let dir = if len > 0.0 { scale(delta, 1.0 / len) } else { [0.0; 3] };
        let y_lo = from[1].min(to[1]) - r;
        let y_hi = from[1].max(to[1]) + r;

        // (node, triangle index relative to the node); None = all of them.
        let mut candidates: Vec<(usize, Option<u16>)> = Vec::new();
        let (c0, r0) = self.grid.locate(from[0].min(to[0]) - r, from[2].min(to[2]) - r);
        let (c1, r1) = self.grid.locate(from[0].max(to[0]) + r, from[2].max(to[2]) + r);
        for row in r0..=r1 {
            for col in c0..=c1 {
                for e in self.grid.entries(col, row) {
                    candidates.extend(e.triangles.iter().map(|&t| (e.node, Some(t))));
                }
            }
        }
        for &n in &self.moving {
            if self.nodes.get(n).is_some_and(|n| n.flags & node_flags::DYNAMIC_OFF == 0) {
                candidates.push((n, None));
            }
        }
        candidates.sort_unstable();
        candidates.dedup();

        let mut best: Option<Hit> = None;
        let mut i = 0;
        while i < candidates.len() {
            let ni = candidates[i].0;
            let group_end = i + candidates[i..].iter().take_while(|c| c.0 == ni).count();
            let group = &candidates[i..group_end];
            i = group_end;
            let Some(node) = self.nodes.get(ni) else { continue };
            if node.flags & q.node_mask == 0 || node.disable & q.disable_mask != 0 || node.triangles.is_empty() {
                continue;
            }
            // Bounding-sphere reject.
            let reach = node.radius + r;
            let rel = sub(node.center, from);
            let t = dot(rel, dir);
            if t > reach + len || t < -reach {
                continue;
            }
            let perp = sub(rel, scale(dir, t));
            if dot(perp, perp) >= reach * reach {
                continue;
            }
            let off = if node.moves() { node.center } else { [0.0; 3] };
            let (start, end) = (sub(from, off), sub(to, off));
            let q_lo = (HEIGHT_STEPS * (y_lo - off[1])) as i32;
            let q_hi = (HEIGHT_STEPS * (y_hi - off[1])) as i32;
            let filtered = node.flags & node_flags::NO_FILTER == 0;

            let all: Vec<usize> = if group.iter().any(|c| c.1.is_none()) {
                node.triangles.clone().collect()
            } else {
                group.iter().filter_map(|c| c.1).map(|t| node.triangles.start + t as usize).collect()
            };
            let mut node_best: Option<Hit> = None;
            for ti in all {
                if skip == Some(ti) {
                    continue;
                }
                let Some(tri) = self.triangles.get(ti) else { continue };
                if filtered
                    && !(q.normal_y[0] <= tri.normal[1]
                        && tri.normal[1] <= q.normal_y[1]
                        && q_lo <= tri.height_range[1] as i32
                        && tri.height_range[0] as i32 <= q_hi)
                {
                    continue;
                }
                let Some((d, p)) = tri.sweep(start, end, r) else { continue };
                let to_start = {
                    let v = sub(p, start);
                    dot(v, v)
                };
                let mut score = if !q.prefer_crossing || d == 0.0 { to_start } else { 10_000.0 * d };
                if dot(dir, tri.normal).abs() < 0.25 {
                    score *= 0.95; // grazing hits count as a little nearer
                }
                if score < node_best.map_or(NO_SCORE, |h| h.score) {
                    node_best = Some(Hit { node: ni, triangle: ti, point: add(p, off), normal: tri.normal, score });
                }
            }
            if let Some(h) = node_best
                && node.flags & node_flags::SECONDARY == 0
                && h.score < best.map_or(NO_SCORE, |b| b.score)
            {
                best = Some(h);
            }
        }
        best
    }

    /// The game's wall test from `from` to `to`.
    pub fn wall(&self, from: [f32; 3], to: [f32; 3], radius: f32) -> Option<Hit> {
        self.cast(from, to, &Query::walls(radius))
    }

    /// The game's floor probe: a vertical segment from `up` above `at` to
    /// `down` below it (both measured upwards: `down` is usually negative).
    pub fn floor_probe(&self, at: [f32; 3], up: f32, down: f32, radius: f32, disable_mask: u8) -> Option<Hit> {
        let q = Query { disable_mask, prefer_crossing: false, ..Query::floors(radius) };
        self.cast([at[0], at[1] + up, at[2]], [at[0], at[1] + down, at[2]], &q)
    }

    /// The floor under `at` as the game finds it for items: searching from
    /// 4 above to 10 below, radius 1. `None` where there's no floor.
    pub fn floor_height(&self, at: [f32; 3]) -> Option<f32> {
        let q = Query::floors(1.0);
        self.cast([at[0], at[1] + 4.0, at[2]], [at[0], at[1] - 10.0, at[2]], &q).map(|h| h.point[1])
    }

    /// The topmost floor at a world X/Z (not a game query: a convenience for
    /// placing things with no height to start from).
    pub fn top_floor(&self, x: f32, z: f32) -> Option<f32> {
        let [lo, hi] = self.bounds;
        let q = Query { prefer_crossing: true, ..Query::floors(0.0) };
        self.cast([x, hi[1] + 1.0, z], [x, lo[1] - 1.0, z], &q).map(|h| h.point[1])
    }

    /// Moves an actor standing at `feet` by `delta` the way the game's actor
    /// mover does: wall test and push-out, then a floor probe at the leading
    /// edge that the actor follows up or down (at most `max_drop` down per
    /// move). With no acceptable floor ahead the horizontal move is
    /// cancelled.
    pub fn move_actor(&self, feet: [f32; 3], delta: [f32; 3], p: &MoveParams) -> Moved {
        let mut delta = delta;
        let mut out = Moved { delta, blocked_by_wall: false, no_floor: false, wall: None, floor: None };

        if let Some(w) = self.wall(feet, add(feet, delta), p.radius) {
            out.wall = Some(w);
            let flags = self.nodes[w.node].flags;
            if flags & node_flags::NO_PUSH == 0 && push_out(p.radius, feet, &mut delta, w.point, w.normal) {
                delta[0] = 0.0;
                delta[2] = 0.0;
                out.blocked_by_wall = true;
            }
        }

        let len = dot(delta, delta).sqrt();
        let dir = if len > 0.0 { scale(delta, 1.0 / len) } else { [0.0; 3] };
        let reach = p.radius + len;
        let allowance = 2.0 * reach;
        let down = -p.step - p.probe_extra_down;
        let probe = |at: [f32; 3]| self.floor_probe(at, p.step, down, p.probe_radius, 2);

        let edge = add(feet, scale(dir, reach));
        let mut ok = false;
        if let Some(mut hit) = probe(edge) {
            let mut height = hit.point[1];
            let rise = (height - feet[1]).abs();
            if rise <= allowance {
                ok = true;
                if len > 0.0 && 0.1 * len < rise {
                    match probe(add(feet, delta)) {
                        Some(h) => {
                            hit = h;
                            height = h.point[1];
                        }
                        None => ok = false,
                    }
                }
            }
            if !ok {
                height = feet[1];
                if let Some(h) = probe(feet) {
                    hit = h;
                    height = h.point[1];
                }
            }
            out.floor = Some(hit);
            delta[1] += (height - feet[1]).max(-p.max_drop);
        }
        if !ok {
            delta[0] = 0.0;
            delta[2] = 0.0;
            out.no_floor = true;
        }
        out.delta = delta;
        out
    }

    /// The height below which a player dies: the level's lowest point minus
    /// 4.5. It's also the floor a player with nothing underneath falls
    /// towards.
    pub fn kill_height(&self) -> f32 {
        self.bounds[0][1] - 4.5
    }

    /// Moves a player standing at `feet` by `delta` against the level, the
    /// way the game's player update does for plain walking
    /// (`docs/collision.md`, "Moving a player"):
    ///
    /// 1. Walls, tested 1 above the collision centre: push out along the
    ///    wall (or slide along a moving one), then test again with 0.95 of
    ///    the radius; a second wall facing more than 60° away from the first
    ///    (a corner) stops the move.
    /// 2. The floor under the destination, probed from the collision centre
    ///    down to 3 below the feet. Too big a change of floor height, or no
    ///    floor, cancels the horizontal move; a probe that only grazes a
    ///    ledge, or finds nothing a radius further on, pulls the move back
    ///    to the ledge.
    /// 3. The feet follow the floor: up at once, down at most `max_drop`.
    ///    Climbing slows the horizontal move so the whole step stays about
    ///    as long as the requested one.
    ///
    /// `ground` carries the floor being followed from move to move. Other
    /// actors, hazards, lifts and teleports are not handled here.
    pub fn move_player(&self, feet: [f32; 3], delta: [f32; 3], p: &PlayerCollision, ground: &mut PlayerGround) -> Moved {
        let mut d = delta;
        let mut out = Moved { delta, blocked_by_wall: false, no_floor: false, wall: None, floor: None };
        let centre = add(feet, [0.0, p.centre_height, 0.0]);
        // The game uses the stick's step length here; with no knockback in
        // the move that's its horizontal length.
        let step = delta[0].hypot(delta[2]);

        // Walls.
        let raised = add(centre, [0.0, 1.0, 0.0]);
        let walls = Query { disable_mask: 1, ..Query::walls(p.radius) };
        let mut wall_ignored = None;
        if let Some(w) = self.cast(raised, add(raised, d), &walls) {
            out.wall = Some(w);
            let flags = self.nodes[w.node].flags;
            if flags & node_flags::NO_PUSH != 0 {
                wall_ignored = Some(w);
            } else {
                if flags & node_flags::MOVES != 0 {
                    d = add(d, scale(w.normal, -dot(d, w.normal)));
                } else {
                    out.blocked_by_wall = push_out(p.radius, raised, &mut d, w.point, w.normal);
                }
                let again = Query { radius: 0.95 * p.radius, ..walls };
                if let Some(w2) = self.cast_skipping(raised, add(raised, d), &again, Some(w.triangle))
                    && dot(w.normal, w2.normal) < 0.5
                {
                    d = [0.0; 3];
                    out.blocked_by_wall = true;
                }
            }
        }
        let wall_hit = out.wall.is_some();

        // Floor.
        let (check, floor) = self.player_floor(feet[1], centre, &mut d, p, ground, wall_hit, wall_ignored);
        out.floor = floor;
        out.no_floor = matches!(check, FloorCheck::Lost | FloorCheck::Held { cancelled: true });

        // Follow the floor.
        let node_moves = |n: Option<usize>| n.is_some_and(|n| self.nodes[n].flags & node_flags::MOVES != 0);
        let follow = match check {
            FloorCheck::Level | FloorCheck::Adjusted => true,
            FloorCheck::Held { .. } => !node_moves(ground.node),
            FloorCheck::Lost => ground.node.is_none(),
        };
        if follow {
            d[1] += (ground.floor - feet[1]).max(-p.max_drop);
        }
        if d[1] > 0.1 * step {
            let h = step.hypot(d[1]);
            if h > 0.01 {
                d[0] *= step / h;
                d[2] *= step / h;
            }
        }
        if matches!(check, FloorCheck::Level | FloorCheck::Adjusted) {
            ground.node = floor.map(|h| h.node);
        }
        out.delta = d;
        out
    }

    /// The player's floor probe: a sphere of `radius` from `at` straight
    /// down by `depth`, disable mask 1.
    fn player_floor_probe(&self, at: [f32; 3], radius: f32, depth: f32, prefer_crossing: bool) -> Option<Hit> {
        let q = Query { disable_mask: 1, prefer_crossing, ..Query::floors(radius) };
        self.cast(at, add(at, [0.0, -depth, 0.0]), &q)
    }

    /// The player's floor check on the move `d` from `centre`, adjusting its
    /// X/Z. `wall_ignored` is a wall hit on a node that doesn't push.
    #[allow(clippy::too_many_arguments)]
    fn player_floor(
        &self,
        feet_y: f32,
        centre: [f32; 3],
        d: &mut [f32; 3],
        p: &PlayerCollision,
        ground: &mut PlayerGround,
        wall_hit: bool,
        wall_ignored: Option<Hit>,
    ) -> (FloorCheck, Option<Hit>) {
        let depth = 3.0 + p.half_height;
        let probe = |at: [f32; 3], prefer: bool| self.player_floor_probe(at, p.radius, depth, prefer);
        let stop = |d: &mut [f32; 3]| {
            d[0] = 0.0;
            d[2] = 0.0;
        };
        // After pulling the move back: a floor there is taken, none cancels.
        let retry = |d: &mut [f32; 3], ground: &mut PlayerGround| match probe(add(centre, *d), false) {
            Some(h) => {
                ground.floor = h.point[1];
                (FloorCheck::Adjusted, Some(h))
            }
            None => {
                stop(d);
                (FloorCheck::Held { cancelled: true }, None)
            }
        };

        let at = add(centre, *d);
        let horizontal = d[0].hypot(d[2]);
        let Some(hit) = probe(at, true) else {
            if horizontal < 0.001 {
                ground.node = None;
            }
            stop(d);
            ground.floor = self.kill_height();
            return (FloorCheck::Lost, None);
        };
        let floor_y = hit.point[1];
        let change = (floor_y - ground.floor).abs();

        if horizontal < 0.001 {
            // Standing still: only a big change (or a moving floor) moves
            // the floor being followed.
            if change > 3.0 || self.nodes[hit.node].flags & node_flags::MOVES != 0 {
                ground.floor = floor_y;
            }
            let check = if change > 0.001 { FloorCheck::Level } else { FloorCheck::Held { cancelled: false } };
            return (check, Some(hit));
        }

        let limit = if ground.node == Some(hit.node) { 4.0 } else { 3.0 };
        if change > limit {
            ground.floor = feet_y;
            stop(d);
            return (FloorCheck::Held { cancelled: true }, Some(hit));
        }

        if let Some(w) = wall_ignored
            && w.node != hit.node
        {
            push_out(p.radius, centre, d, w.point, w.normal);
            return retry(d, ground);
        }

        // How far the floor was found from straight below the probe, and
        // whether the move heads away from it.
        let mut off = sub(at, hit.point);
        let mut away = dot(*d, off);
        let mut dist = off[0].hypot(off[2]);
        if dist < 0.001 && !wall_hit {
            // Right below: look a radius further on too.
            let len = dot(*d, *d).sqrt();
            let lead = scale(*d, p.radius / len);
            let edge = add(at, lead);
            match probe(edge, true) {
                None => {
                    off = lead;
                    away = 0.0;
                    dist = p.radius;
                }
                Some(e) => {
                    let solid = self.nodes[e.node].flags & 0x8 != 0;
                    if !solid || (e.point[1] - floor_y).abs() >= 4.0 {
                        off = sub(edge, e.point);
                        away = dot(*d, off);
                        dist = off[0].hypot(off[2]);
                    } else {
                        dist = 0.0;
                    }
                }
            }
        }
        if dist < 0.001 || away < 0.0 {
            ground.floor = floor_y;
            let check = if dist < 0.001 || away < -0.25 { FloorCheck::Level } else { FloorCheck::Adjusted };
            return (check, Some(hit));
        }
        d[0] -= off[0];
        d[2] -= off[2];
        retry(d, ground)
    }
}

/// The game's wall push-out: moves the destination back out along the
/// wall's normal (in X/Z) until it's `radius` from the contact point. Each
/// axis is corrected when it's the minor axis of the move or the move goes
/// into the wall along it; a correction bigger than the move plus half the
/// radius zeroes that axis and reports the move as blocked (`true`).
pub fn push_out(radius: f32, from: [f32; 3], delta: &mut [f32; 3], contact: [f32; 3], normal: [f32; 3]) -> bool {
    let d = ((from[0] + delta[0]) - contact[0]) * normal[0] + ((from[2] + delta[2]) - contact[2]) * normal[2] - radius;
    if d >= 0.0 {
        return false;
    }
    let (vx, vz) = (delta[0], delta[2]);
    let x_minor = vx.abs() < vz.abs();
    let mut blocked = false;
    let cx = normal[0] * d;
    if x_minor || (vx > 0.0 && cx > 0.0) || (vx < 0.0 && cx < 0.0) {
        if 0.5 * radius + vx.abs() <= cx.abs() {
            blocked = true;
            delta[0] = 0.0;
        } else {
            delta[0] -= cx;
        }
    }
    let cz = normal[2] * d;
    if x_minor && !((vz > 0.0 && cz > 0.0) || (vz < 0.0 && cz < 0.0)) {
        return blocked;
    }
    if cz.abs() < 0.5 * radius + delta[2].abs() {
        delta[2] -= cz;
        blocked
    } else {
        delta[2] = 0.0;
        true
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn slice(file: &[u8], at: usize, len: usize) -> Result<&[u8], WorldError> {
    file.get(at..at + len).ok_or(WorldError::Truncated(at, at + len))
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le_f32(b: &[u8], at: usize) -> f32 {
    f32::from_bits(le_u32(b, at))
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }

    /// Encodes a triangle the way the level files do: normal from the
    /// counter-clockwise corners, first corner as origin, the other two in
    /// the plane frame in 1/64 units.
    fn triangle(v: [[f32; 3]; 3]) -> CollisionTriangle {
        let c = cross(sub(v[1], v[0]), sub(v[2], v[0]));
        let n = scale(c, 1.0 / dot(c, c).sqrt());
        let mut t = CollisionTriangle {
            height_range: [0, 0],
            frame_scale: 1.0 / (1.0 - n[1] * n[1]).sqrt(),
            normal: n,
            origin: v[0],
            corners: [[0; 2]; 2],
        };
        for k in 0..2 {
            let l = t.to_plane(sub(v[k + 1], v[0]));
            t.corners[k] = [(l[0] * 64.0).round() as i16, (l[2] * 64.0).round() as i16];
        }
        let ys = v.map(|p| p[1]);
        t.height_range = [
            (ys.iter().cloned().fold(f32::MAX, f32::min) * 64.0).floor() as i16,
            (ys.iter().cloned().fold(f32::MIN, f32::max) * 64.0).ceil() as i16,
        ];
        t
    }

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() <= eps)
    }

    #[test]
    fn plane_frame_round_trips() {
        for n in [[0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.3, 0.8, -0.52]] {
            let n = scale(n, 1.0 / dot(n, n).sqrt());
            let t = CollisionTriangle {
                height_range: [0, 0],
                frame_scale: 1.0 / (1.0 - n[1] * n[1]).sqrt(),
                normal: n,
                origin: [0.0; 3],
                corners: [[0; 2]; 2],
            };
            let p = [1.5, -2.0, 3.25];
            let l = t.to_plane(p);
            assert!((l[1] - dot(p, n)).abs() < 1e-4, "height is the distance along the normal");
            assert!(close(t.from_plane(l), p, 1e-4), "{n:?}");
        }
    }

    #[test]
    fn encoded_triangles_decode_to_their_corners() {
        let v = [[1.0, 2.0, 3.0], [4.0, 2.5, 3.5], [2.0, 6.0, 1.0]];
        let back = triangle(v).vertices();
        for k in 0..3 {
            assert!(close(back[k], v[k], 1.0 / 32.0), "{k}: {:?} vs {:?}", back[k], v[k]);
        }
    }

    #[test]
    fn sweep_hits_front_faces_only() {
        // Floor at y = 0 over x, z in [0, 4], facing up.
        let t = triangle([[0.0, 0.0, 0.0], [0.0, 0.0, 4.0], [4.0, 0.0, 0.0]]);
        assert!(close(t.normal, [0.0, 1.0, 0.0], 1e-6));
        let (d, p) = t.sweep([1.0, 2.0, 1.0], [1.0, -2.0, 1.0], 0.5).unwrap();
        assert_eq!(d, 0.0);
        assert!(close(p, [1.0, 0.0, 1.0], 1e-6));
        // From below: behind the face.
        assert!(t.sweep([1.0, -2.0, 1.0], [1.0, 2.0, 1.0], 0.5).is_none());
        // Moving away from it.
        assert!(t.sweep([1.0, 0.2, 1.0], [1.0, 2.0, 1.0], 0.5).is_none());
        // Passing beside it: within the radius of an edge, and not.
        let (d, p) = t.sweep([-0.3, 2.0, 1.0], [-0.3, -2.0, 1.0], 0.5).unwrap();
        assert!((d - 0.09).abs() < 1e-4 && close(p, [0.0, 0.0, 1.0], 1e-4), "{d} {p:?}");
        assert!(t.sweep([-0.8, 2.0, 1.0], [-0.8, -2.0, 1.0], 0.5).is_none());
        // Hovering within the radius counts as touching.
        assert_eq!(t.sweep([1.0, 0.4, 1.0], [1.5, 0.3, 1.0], 0.5).map(|h| h.0), Some(0.0));
        assert!(t.sweep([1.0, 0.8, 1.0], [1.5, 0.7, 1.0], 0.5).is_none());
    }

    #[test]
    fn push_out_stops_at_the_radius_and_slides() {
        let n = [-1.0, 0.0, 0.0]; // wall at x = 5 facing -x
        let mut d = [3.0, 0.0, 0.0];
        assert!(!push_out(0.5, [3.0, 0.0, 5.0], &mut d, [5.0, 0.0, 5.0], n));
        assert!(close(d, [1.5, 0.0, 0.0], 1e-6));
        // Diagonally into it: the X part stops at the radius, Z slides on.
        let mut d = [1.0, 0.0, 1.0];
        assert!(!push_out(0.5, [4.0, 0.0, 5.0], &mut d, [5.0, 0.0, 5.0], n));
        assert!(close(d, [0.5, 0.0, 1.0], 1e-6));
        // Already clear: untouched.
        let mut d = [0.1, 0.0, 0.0];
        assert!(!push_out(0.5, [3.0, 0.0, 5.0], &mut d, [5.0, 0.0, 5.0], n));
        assert_eq!(d, [0.1, 0.0, 0.0]);
        // A correction bigger than the move plus half the radius blocks it.
        let mut d = [0.0, 0.0, 0.1];
        assert!(push_out(1.0, [4.9, 0.0, 5.0], &mut d, [5.0, 0.0, 5.0], n));
        assert_eq!(d[0], 0.0);
    }

    fn put_u32(f: &mut [u8], at: usize, v: u32) {
        f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A one-node, one-cell level: a 10 × 10 floor at y = 0 and a wall at
    /// x = 5 facing -x.
    fn test_level() -> LevelCollision {
        let tris = [
            triangle([[0.0, 0.0, 0.0], [0.0, 0.0, 10.0], [10.0, 0.0, 0.0]]),
            triangle([[10.0, 0.0, 10.0], [10.0, 0.0, 0.0], [0.0, 0.0, 10.0]]),
            triangle([[5.0, 0.0, 0.0], [5.0, 0.0, 10.0], [5.0, 5.0, 0.0]]),
            triangle([[5.0, 5.0, 10.0], [5.0, 5.0, 0.0], [5.0, 0.0, 10.0]]),
        ];
        assert!(close(tris[2].normal, [-1.0, 0.0, 0.0], 1e-6));
        let level = one_node_level(&tris, 6);
        assert_eq!(level.nodes[0].triangles, 0..4);
        level
    }

    /// Two counter-clockwise (seen from the front) triangles covering the
    /// quad `a b c d`.
    fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> [CollisionTriangle; 2] {
        [triangle([a, b, c]), triangle([a, c, d])]
    }

    /// An X-aligned floor strip at height `y` from `x0` to `x1`, z 0..10.
    fn floor(x0: f32, x1: f32, y: f32) -> [CollisionTriangle; 2] {
        quad([x0, y, 0.0], [x0, y, 10.0], [x1, y, 10.0], [x1, y, 0.0])
    }

    /// A wall across z 0..10 at `x`, from `y0` to `y1`, facing -x.
    fn wall_facing_neg_x(x: f32, y0: f32, y1: f32) -> [CollisionTriangle; 2] {
        quad([x, y0, 0.0], [x, y0, 10.0], [x, y1, 10.0], [x, y1, 0.0])
    }

    /// A level of one node holding `tris` (world space) and one grid cell
    /// covering everything.
    fn one_node_level(tris: &[CollisionTriangle], flags: u32) -> LevelCollision {
        let corners: Vec<[f32; 3]> = tris.iter().flat_map(|t| t.vertices()).collect();
        let lo = corners.iter().fold([f32::MAX; 3], |m, v| [m[0].min(v[0]), m[1].min(v[1]), m[2].min(v[2])]);
        let hi = corners.iter().fold([f32::MIN; 3], |m, v| [m[0].max(v[0]), m[1].max(v[1]), m[2].max(v[2])]);
        let centre = scale(add(lo, hi), 0.5);
        let radius = corners.iter().map(|v| dot(sub(*v, centre), sub(*v, centre)).sqrt()).fold(0.0, f32::max) + 0.1;
        let size = (hi[0] - lo[0]).max(hi[2] - lo[2]).max(1.0);

        let nodes_at = 30 * 4;
        let tris_at = nodes_at + 0x3C;
        let lists_at = tris_at + tris.len() * TRIANGLE_STRIDE;
        let cells_at = lists_at + (2 + tris.len()) * 2;
        let rows_at = cells_at + 2 * 4;
        let mut f = vec![0u8; rows_at + 8];
        let words: [(usize, u32); 17] = [
            (0, 1),
            (1, nodes_at as u32),
            (2, tris.len() as u32),
            (3, tris_at as u32),
            (5, cells_at as u32),
            (7, lists_at as u32),
            (8, rows_at as u32),
            (9, lo[0].to_bits()),
            (10, lo[1].to_bits()),
            (11, lo[2].to_bits()),
            (12, hi[0].to_bits()),
            (13, hi[1].to_bits()),
            (14, hi[2].to_bits()),
            (15, size.to_bits()),
            (16, 1),
            (17, 1),
            (24, 0xF00B_AB02),
        ];
        for (i, v) in words {
            put_u32(&mut f, i * 4, v);
        }
        // Node: the given flags, no links, a bounding sphere around the
        // triangles, which are in world space.
        let n = nodes_at;
        f[n..n + 5].copy_from_slice(b"LEVEL");
        put_u32(&mut f, n + 0x10, flags);
        for (k, v) in centre.iter().enumerate() {
            put_u32(&mut f, n + 0x1C + k * 4, v.to_bits());
        }
        f[n + 0x2C..n + 0x30].copy_from_slice(&[0xFF; 4]);
        put_u32(&mut f, n + 0x30, radius.to_bits());
        f[n + 0x36..n + 0x38].copy_from_slice(&(tris.len() as u16).to_le_bytes());
        for (k, t) in tris.iter().enumerate() {
            let r = tris_at + k * TRIANGLE_STRIDE;
            f[r..r + 2].copy_from_slice(&t.height_range[0].to_le_bytes());
            f[r + 2..r + 4].copy_from_slice(&t.height_range[1].to_le_bytes());
            let floats = [t.frame_scale, t.normal[0], t.normal[1], t.normal[2], t.origin[0], t.origin[1], t.origin[2]];
            for (j, v) in floats.iter().enumerate() {
                put_u32(&mut f, r + 4 + j * 4, v.to_bits());
            }
            for (j, v) in t.corners.iter().flatten().enumerate() {
                f[r + 0x20 + j * 2..r + 0x22 + j * 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        // The one cell lists every triangle of node 0.
        let list = [0u16, tris.len() as u16].into_iter().chain(0..tris.len() as u16);
        for (j, v) in list.enumerate() {
            f[lists_at + j * 2..lists_at + j * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        put_u32(&mut f, cells_at, 0); // cell 0: nothing moves
        put_u32(&mut f, cells_at + 4, 1 << 22); // cell 1: one entry at byte 0
        put_u32(&mut f, rows_at + 4, 1); // row 0: column 0 only, cell 1
        let world = WorldFile::parse(&f).unwrap();
        LevelCollision::new(&world).unwrap()
    }

    /// Every level's collision: triangles decode to the faces their normals
    /// describe, inside their node's bounding sphere and height range; the
    /// grid only lists triangles that reach the cell; and the floor query
    /// finds every floor triangle from just above it.
    #[test]
    fn every_real_level_collision_is_consistent() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let (mut levels, mut triangles, mut owned, mut moving, mut entries_seen) = (0, 0, 0, 0, 0);
        let (mut floors, mut floors_found, mut placed, mut placed_floor) = (0, 0, 0, 0);
        let (mut unlisted, mut probes, mut degenerate, mut floors_here, mut stray) = (0, 0, 0, 0, 0);
        for level in entries.flatten().map(|e| e.path()) {
            let Ok(bytes) = std::fs::read(level.join("WORLDS.PS2")) else { continue };
            let world = WorldFile::parse(&bytes).unwrap_or_else(|e| panic!("{level:?}: {e}"));
            let c = LevelCollision::new(&world).unwrap();
            levels += 1;
            triangles += c.triangles.len();
            moving += c.moving.len();
            let [lo, hi] = c.bounds;

            let mut owner = vec![None; c.triangles.len()];
            for (ni, n) in c.nodes.iter().enumerate() {
                for t in n.triangles.clone() {
                    assert!(owner[t].replace(ni).is_none(), "{level:?}: triangle {t} has two owners");
                }
            }
            owned += owner.iter().flatten().count();

            for (ni, ti, v) in c.world_triangles() {
                let t = &c.triangles[ti];
                let n = &c.nodes[ni];
                let at = format!("{level:?} node {ni} triangle {ti}");
                assert!((dot(t.normal, t.normal) - 1.0).abs() < 1e-3, "{at}: normal {:?}", t.normal);
                let (e1, e2) = (sub(v[1], v[0]), sub(v[2], v[0]));
                let cr = cross(e1, e2);
                if dot(cr, cr) <= 1e-4 * dot(e1, e1) * dot(e2, e2) {
                    // A sliver (corners within ~0.6° of a line): 1/64 rounding
                    // can flip its winding.
                    degenerate += 1;
                } else {
                    assert!(dot(cr, t.normal) > 0.99 * dot(cr, cr).sqrt(), "{at}: winding {t:?}");
                }
                if t.normal[1].abs() < 0.99 {
                    let want = 1.0 / (1.0 - t.normal[1] * t.normal[1]).sqrt();
                    assert!((t.frame_scale / want - 1.0).abs() < 1e-3, "{at}: frame scale");
                }
                for (k, p) in t.vertices().iter().enumerate() {
                    let q = p[1] * HEIGHT_STEPS;
                    let [a, b] = t.height_range;
                    assert!(q >= a as f32 - 9.0 && q <= b as f32 + 9.0, "{at}: corner {k} height {q} outside {a}..{b}");
                    let d = sub(v[k], n.center);
                    assert!(dot(d, d).sqrt() <= n.radius + 0.05, "{at}: corner {k} outside the node's sphere");
                    if !n.moves() {
                        let inside = (0..3).all(|j| v[k][j] >= lo[j] - 0.5 && v[k][j] <= hi[j] + 0.5);
                        assert!(inside, "{at}: corner {:?} outside level bounds", v[k]);
                    }
                }
                if !n.moves() && t.normal[1] >= 0.5 && n.flags & node_flags::FLOORS != 0 && n.flags & node_flags::SECONDARY == 0 {
                    let centre = scale(add(add(v[0], v[1]), v[2]), 1.0 / 3.0);
                    floors += 1;
                    // The nearest floor from 4 above: this one or one above
                    // it — except where the game's ranking prefers a floor
                    // the probe misses by a hair (10000 × its squared
                    // distance) over a real crossing a few units away.
                    if let Some(h) = c.floor_height(add(centre, [0.0, 0.25, 0.0])) {
                        assert!(h.is_finite(), "{at}");
                        floors_found += 1;
                        floors_here += (h >= centre[1] - 0.02) as usize;
                    }
                }
            }

            // The grid lists each static triangle in the cells it covers
            // (probed at its corners and centre; the level tool misses a
            // handful of interior cells). It also has extras: it smears some
            // walls along whole rows, columns or diagonals, which only costs
            // the game time.
            let g = &c.grid;
            let mut listings = std::collections::HashSet::new();
            for (z, r) in g.rows.iter().enumerate() {
                if r.first_column > r.last_column {
                    continue;
                }
                for x in r.first_column as u32..=r.last_column as u32 {
                    let lo = [g.origin[0] + x as f32 * g.cell_size, g.origin[1] + z as f32 * g.cell_size];
                    for e in g.entries(x, z as u32) {
                        entries_seen += 1;
                        let n = &c.nodes[e.node];
                        let off = if n.moves() { n.center } else { [0.0; 3] };
                        for &t in e.triangles {
                            let ti = n.triangles.start + t as usize;
                            listings.insert((x, z as u32, ti));
                            let v = c.triangles[ti].vertices().map(|p| add(p, off));
                            let near = |k: usize, lo: f32| v.iter().any(|p| p[k] >= lo - 0.1) && v.iter().any(|p| p[k] <= lo + g.cell_size + 0.1);
                            let edge = x == 0 || z == 0 || x + 1 == g.columns || z + 1 == g.rows.len();
                            stray += (!edge && !(near(0, lo[0]) && near(2, lo[1]))) as usize;
                        }
                    }
                }
            }
            for n in c.nodes.iter().filter(|n| !n.moves()) {
                for ti in n.triangles.clone() {
                    let v = c.triangles[ti].vertices();
                    let centre = scale(add(add(v[0], v[1]), v[2]), 1.0 / 3.0);
                    for p in [centre, v[0], v[1], v[2]] {
                        let p = add(scale(p, 0.95), scale(centre, 0.05));
                        let (x, z) = g.locate(p[0], p[2]);
                        probes += 1;
                        unlisted += !listings.contains(&(x, z, ti)) as usize;
                    }
                }
            }

            let positions = world.world_positions().unwrap();
            for (n, p) in world.nodes.iter().zip(&positions) {
                if let (true, Some(p)) = (n.has_model, p) {
                    placed += 1;
                    if let Some(h) = c.floor_height(*p) {
                        assert!(h.is_finite());
                        placed_floor += 1;
                    }
                }
            }
        }
        eprintln!(
            "{levels} levels: {triangles} collision triangles ({owned} owned by nodes, {degenerate} slivers), \
             {entries_seen} cell entries ({stray} listings nowhere near their cell; {unlisted} of {probes} \
             probes find their triangle unlisted), \
             {moving} moving nodes; floor query from above {floors} floor triangles \
             finds a floor {floors_found} times, that one or higher {floors_here}; {placed_floor}/{placed} placed \
             models stand within reach of a floor"
        );
        assert!(unlisted * 1000 <= probes, "too many triangles missing from cells they cover");
        assert!(floors_found == floors, "floor query missed {} floor triangles", floors - floors_found);
        assert!(floors_here * 100 >= floors * 99, "floor query found a lower floor too often");
    }

    #[test]
    fn floor_queries_find_the_floor() {
        let level = test_level();
        assert_eq!(level.floor_height([2.0, 0.5, 2.0]), Some(0.0));
        assert_eq!(level.floor_height([8.0, -3.0, 8.0]), Some(0.0));
        assert_eq!(level.floor_height([2.0, 20.0, 2.0]), None, "too far above");
        assert_eq!(level.floor_height([30.0, 0.0, 30.0]), None, "off the level");
        assert_eq!(level.top_floor(7.0, 3.0), Some(0.0));
    }

    #[test]
    fn walls_block_from_the_front() {
        let level = test_level();
        let hit = level.wall([3.0, 1.0, 5.0], [6.0, 1.0, 5.0], 0.5).unwrap();
        assert!(close(hit.normal, [-1.0, 0.0, 0.0], 1e-6));
        assert!((hit.point[0] - 5.0).abs() < 1e-5);
        assert!(level.wall([6.0, 1.0, 5.0], [3.0, 1.0, 5.0], 0.5).is_none(), "back face");
    }

    #[test]
    fn actors_stop_at_walls_and_edges() {
        let level = test_level();
        let p = MoveParams::new(0.5, 1.0, 0.5);
        let m = level.move_actor([3.0, 0.0, 5.0], [3.0, 0.0, 0.0], &p);
        assert!(m.wall.is_some() && !m.no_floor);
        assert!(close(m.delta, [1.5, 0.0, 0.0], 1e-5), "{:?}", m.delta);
        // Free walk on open floor.
        let m = level.move_actor([1.0, 0.0, 1.0], [1.0, 0.0, 1.0], &p);
        assert!(close(m.delta, [1.0, 0.0, 1.0], 1e-5) && m.wall.is_none());
        // Off the edge of the floor: cancelled.
        let m = level.move_actor([9.5, 0.0, 2.0], [2.0, 0.0, 0.0], &p);
        assert!(m.no_floor && m.delta[0] == 0.0);
        // Standing above the floor: drops at most max_drop.
        let m = level.move_actor([2.0, 1.5, 2.0], [0.1, 0.0, 0.0], &p);
        assert!((m.delta[1] + 0.5).abs() < 1e-5, "{:?}", m.delta);
    }

    /// Walks a player `ticks` moves of `step` from `feet`, returning where
    /// it ended and the last move.
    fn walk(level: &LevelCollision, feet: [f32; 3], step: [f32; 3], ticks: usize) -> ([f32; 3], Moved, PlayerGround) {
        let p = PlayerCollision::default();
        let mut ground = PlayerGround::new(feet[1]);
        let mut feet = feet;
        let mut last = None;
        for _ in 0..ticks {
            let m = level.move_player(feet, step, &p, &mut ground);
            feet = add(feet, m.delta);
            last = Some(m);
        }
        (feet, last.unwrap(), ground)
    }

    #[test]
    fn player_defaults_are_the_class_body() {
        let p = PlayerCollision::default();
        assert_eq!((p.radius, p.half_height, p.centre_height), (1.5, 2.5, 2.5));
        assert!((p.max_drop - 16.0 / 30.0).abs() < 1e-6);
    }

    #[test]
    fn players_walk_and_stop_at_walls() {
        let level = test_level();
        let p = PlayerCollision::default();
        // Open floor: the move goes through untouched.
        let mut g = PlayerGround::new(0.0);
        let m = level.move_player([1.0, 0.0, 2.0], [0.3, 0.0, 0.2], &p, &mut g);
        assert!(close(m.delta, [0.3, 0.0, 0.2], 1e-6) && m.wall.is_none() && !m.no_floor, "{m:?}");
        assert_eq!((g.floor, g.node), (0.0, Some(0)));
        // Into the wall at x = 5: stops with the centre a radius away.
        let m = level.move_player([3.2, 0.0, 5.0], [0.5, 0.0, 0.0], &p, &mut g);
        assert!(m.wall.is_some() && close(m.delta, [0.3, 0.0, 0.0], 1e-5), "{m:?}");
        // Diagonally into it: slides along it.
        let m = level.move_player([3.5, 0.0, 5.0], [0.3, 0.0, 0.3], &p, &mut g);
        assert!(close(m.delta, [0.0, 0.0, 0.3], 1e-5), "{m:?}");
        // Held there however long it pushes.
        let (feet, _, _) = walk(&level, [1.0, 0.0, 5.0], [0.5, 0.0, 0.0], 20);
        assert!((feet[0] - 3.5).abs() < 1e-4 && feet[1] == 0.0, "{feet:?}");
    }

    #[test]
    fn players_stop_in_corners() {
        let mut tris = floor(0.0, 10.0, 0.0).to_vec();
        tris.extend(wall_facing_neg_x(5.0, 0.0, 6.0));
        // A wall at z = 5 facing -z.
        tris.extend(quad([0.0, 0.0, 5.0], [0.0, 6.0, 5.0], [10.0, 6.0, 5.0], [10.0, 0.0, 5.0]));
        let level = one_node_level(&tris, 6);
        let mut g = PlayerGround::new(0.0);
        let m = level.move_player([3.4, 0.0, 3.4], [0.2, 0.0, 0.2], &PlayerCollision::default(), &mut g);
        assert!(m.blocked_by_wall && m.delta == [0.0; 3], "{m:?}");
    }

    #[test]
    fn players_are_held_back_from_ledges() {
        // The floor ends at x = 10 with nothing below.
        let level = test_level();
        let (feet, m, g) = walk(&level, [7.0, 0.0, 2.0], [0.4, 0.0, 0.0], 30);
        // The centre stays a radius from the edge.
        assert!((feet[0] - 8.5).abs() < 1e-3 && feet[1] == 0.0, "{feet:?} {m:?}");
        assert_eq!(g.floor, 0.0);
    }

    #[test]
    fn players_step_up_and_drop_down() {
        // A 1-high step up at x = 10, then a 2-deep drop at x = 20.
        let mut tris = floor(0.0, 10.0, 0.0).to_vec();
        tris.extend(wall_facing_neg_x(10.0, 0.0, 1.0));
        tris.extend(floor(10.0, 20.0, 1.0));
        tris.extend(floor(20.0, 30.0, -1.0));
        let level = one_node_level(&tris, 6);
        let p = PlayerCollision::default();
        let mut g = PlayerGround::new(0.0);
        let mut feet = [8.0, 0.0, 5.0];
        let mut heights = Vec::new();
        for _ in 0..60 {
            let m = level.move_player(feet, [0.5, 0.0, 0.0], &p, &mut g);
            assert!(!m.no_floor, "{feet:?} {m:?}");
            // Climbing slows the horizontal part by step / |(step, rise)|.
            if m.delta[1] > 0.05 {
                let slowed = 0.5 * 0.5 / 0.5f32.hypot(m.delta[1]);
                assert!((m.delta[0] - slowed).abs() < 1e-5, "{m:?}");
            }
            // Down no faster than 16 units/s.
            assert!(m.delta[1] >= -p.max_drop - 1e-6, "{m:?}");
            feet = add(feet, m.delta);
            heights.push(feet[1]);
        }
        assert!(heights.contains(&1.0), "stepped up: {heights:?}");
        assert!(feet[0] > 25.0 && feet[1] == -1.0, "dropped down: {feet:?}");
        // Dropping takes several ticks.
        let falling = heights.iter().filter(|&&h| h > -1.0 && h < 1.0).count();
        assert!(falling >= 3, "{heights:?}");
    }

    #[test]
    fn players_cannot_climb_above_their_centre() {
        // A 3-high block at x = 10: above the 2.5 collision centre.
        let mut tris = floor(0.0, 10.0, 0.0).to_vec();
        tris.extend(floor(10.0, 20.0, 3.0));
        let level = one_node_level(&tris, 6);
        let (feet, _, _) = walk(&level, [7.0, 0.0, 5.0], [0.4, 0.0, 0.0], 30);
        assert!(feet[0] < 10.0 && feet[1] == 0.0, "{feet:?}");
    }

    #[test]
    fn players_with_no_floor_fall() {
        let level = test_level();
        let p = PlayerCollision::default();
        // Nothing below: the move is cancelled and, not standing on
        // anything, the player falls towards the kill height.
        let mut g = PlayerGround::new(0.0);
        let m = level.move_player([20.0, 0.0, 20.0], [0.3, 0.0, 0.0], &p, &mut g);
        assert!(m.no_floor && m.delta[0] == 0.0 && (m.delta[1] + p.max_drop).abs() < 1e-6, "{m:?}");
        assert_eq!(g.floor, level.kill_height());
    }

    /// From every level's entry-0 player start, a player walking in eight
    /// directions never loses the floor, never sinks below it, and never
    /// passes through a wall.
    #[test]
    fn players_walk_every_real_level_without_falling_through() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let p = PlayerCollision::default();
        // Running flat out: 12.5 units/s × 1.3, per 30 Hz tick.
        let speed = 12.5 * 1.3 / 30.0;
        let (mut levels, mut no_start, mut moves, mut walls, mut held, mut dips) = (0, 0, 0, 0, 0, 0);
        let mut travelled = 0.0f32;
        for level in entries.flatten().map(|e| e.path()) {
            let Ok(bytes) = std::fs::read(level.join("WORLDS.PS2")) else { continue };
            let world = WorldFile::parse(&bytes).unwrap();
            let c = LevelCollision::new(&world).unwrap();
            let pop = crate::population::Population::parse(&bytes).unwrap();
            let Some(start) = pop.player_start(0) else {
                no_start += 1;
                continue;
            };
            let Some(y) = c.floor_height(start.position) else {
                no_start += 1;
                continue;
            };
            levels += 1;
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4 + 0.1;
                let step = [speed * a.sin(), 0.0, speed * a.cos()];
                let mut feet = [start.position[0], y, start.position[2]];
                let mut ground = PlayerGround::new(y);
                for t in 0..45 {
                    let at = format!("{level:?} direction {k} tick {t} feet {feet:?}");
                    let m = c.move_player(feet, step, &p, &mut ground);
                    assert!(m.floor.is_some() || m.no_floor, "{at}");
                    let next = add(feet, m.delta);
                    moves += 1;
                    walls += m.wall.is_some() as usize;
                    held += m.no_floor as usize;
                    travelled += m.delta[0].hypot(m.delta[2]);
                    // Never through a wall: the centre's path crosses none.
                    let lift = [0.0, p.centre_height + 1.0, 0.0];
                    let crossed = c.cast(add(feet, lift), add(next, lift), &Query { disable_mask: 1, ..Query::walls(0.0) });
                    assert!(crossed.is_none(), "{at}: walked through {crossed:?}");
                    // Never loses the floor, and never sinks into it by more
                    // than one tick's worth: the game's own rules do dip the
                    // feet for a tick where a near miss outranks the floor
                    // right below (a seam), or where sliding along a moving
                    // node's overhang pushes the move down.
                    assert!(ground.floor > c.kill_height(), "{at}: lost the floor");
                    let dip = p.max_drop + 1e-3;
                    assert!(next[1] >= ground.floor - dip, "{at}: below the floor {ground:?} {m:?}");
                    let line = Query { prefer_crossing: true, ..Query::floors(0.0) };
                    if let Some(s) = c.cast(add(next, [0.0, 2.4, 0.0]), add(next, [0.0, 0.02, 0.0]), &line) {
                        dips += 1;
                        assert!(s.point[1] - next[1] <= dip, "{at}: feet {next:?} sunk under {s:?}; {m:?}");
                    }
                    feet = next;
                }
            }
        }
        eprintln!(
            "{levels} levels walked ({no_start} without a start on a floor): {moves} moves, {walls} touching \
             walls, {held} held back by the floor, {dips} with the feet dipped into a floor, {travelled:.0} \
             units travelled"
        );
        assert!(no_start <= 2, "{no_start} levels have no player start on a floor");
        assert!(dips * 100 <= moves, "feet dipped into floors too often");
        assert!(travelled >= 0.25 * speed * moves as f32, "players hardly moved");
    }
}
