//! `WORLDS.PS2` — a level's scene graph: named nodes placing models.
//!
//! Confirmed against `main.dol` (see `docs/worlds-format.md` for where):
//! the game walks the tree (next sibling at node `+0x2C`, first child at
//! `+0x2E`), builds each node's instance at its translation *relative to its
//! parent*, and finds its model by binary-searching every loaded model
//! file's name table for the node's name. Little-endian like the models.
//!
//! The same file carries the level's collision; see [`crate::collision`].

use thiserror::Error;

use crate::anim::{self, Track};
use crate::collision::CollisionTables;

const NODE_STRIDE: usize = 0x3C;
const HEADER_WORDS: usize = 30;

/// `(version & 0xF00BAB00) == 0xF00BAB00` marks the extended format; the low
/// byte is a revision (retail levels use 2).
pub const VERSION_MAGIC: u32 = 0xF00B_AB00;

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("node {0} links to node {1}, which doesn't exist")]
    BadLink(usize, i32),
    #[error("scene graph has a cycle through node {0}")]
    Cycle(usize),
    #[error("collision tables: {0}")]
    BadCollision(String),
    #[error("animated object {0}: {1}")]
    BadAnimation(usize, String),
}

#[derive(Debug, Clone)]
pub struct WorldHeader {
    pub num_nodes: u32,
    pub nodes_offset: u32,
    /// Collision triangles (`0x28`-byte records, see [`crate::collision`]).
    pub num_triangles: u32,
    pub triangles_offset: u32,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// Side length of the level's X/Z lookup grid cells.
    pub cell_size: f32,
    pub grid_width: u32,
    pub grid_depth: u32,
    pub version: u32,
    /// All 30 header words, for fields not named yet.
    pub words: [u32; HEADER_WORDS],
}

#[derive(Debug, Clone)]
pub struct WorldNode {
    pub name: String,
    pub flags: u32,
    /// Node `+0x18` in the file (the game reuses the field for the parent
    /// pointer once loaded): flags the node's instance is created with,
    /// including how it's drawn — see [`render_flags`].
    pub render_flags: u32,
    /// Translation relative to the parent node.
    pub local_position: [f32; 3],
    /// Node `+0x28 == 1`: the node draws the model with its name.
    pub has_model: bool,
    pub next_sibling: Option<usize>,
    pub first_child: Option<usize>,
    /// Node `+0x30`; equals the model record's `+0x04` bounding value, and
    /// bounds the node's collision triangles.
    pub radius: f32,
    /// Node `+0x35`: collision queries skip nodes sharing a bit with their
    /// mask (0 in every retail level).
    pub collision_disable: u8,
    /// The node's collision triangles: node `+0x38` first (−1 = none),
    /// `+0x36` count.
    pub collision: std::ops::Range<usize>,
}

/// Instance flags that change how a model is drawn (`docs/rendering.md`).
/// The game masks an instance's flags with `0x1090D7C0` for its draw call.
pub mod render_flags {
    /// Depth test off (compare always).
    pub const NO_DEPTH_TEST: u32 = 0x40;
    /// Depth writes off.
    pub const NO_DEPTH_WRITE: u32 = 0x80;
    /// The lightmap stage is skipped.
    pub const NO_LIGHTMAP: u32 = 0x4000;
    /// Environment-mapped: the texture is looked up by the normal along
    /// the camera's right and up instead of the model's own coordinates
    /// (the game's draw flag `0x20000`): keys, blades, ice, glass, the
    /// menu arrow.
    pub const ENV_MAP: u32 = 0x8000;
    /// Additive: source × alpha + destination (PS2 ALPHA 0x48; normal
    /// blending is 0x44).
    pub const ADDITIVE: u32 = 0x80_0000;
    /// Facing mode, applied to the instance's matrix while drawing:
    /// `0x1…` turns about Y toward the camera, `0x3…` also tilts, `0x5…`/
    /// `0x6…`/`0x7…` tilt at most 15°/30°/45°, `0x4…` takes the camera's
    /// rotation.
    pub const FACING_MASK: u32 = 0x0F00_0000;
}

#[derive(Debug, Clone)]
pub struct WorldFile {
    pub header: WorldHeader,
    pub nodes: Vec<WorldNode>,
    pub collision: CollisionTables,
}

impl WorldFile {
    pub fn parse(file: &[u8]) -> Result<Self, WorldError> {
        let h = slice(file, 0, HEADER_WORDS * 4)?;
        let words: [u32; HEADER_WORDS] = std::array::from_fn(|i| le_u32(h, i * 4));
        let f = |i: usize| f32::from_bits(words[i]);
        let header = WorldHeader {
            num_nodes: words[0],
            nodes_offset: words[1],
            num_triangles: words[2],
            triangles_offset: words[3],
            bounds_min: [f(9), f(10), f(11)],
            bounds_max: [f(12), f(13), f(14)],
            cell_size: f(15),
            grid_width: words[16],
            grid_depth: words[17],
            version: words[24],
            words,
        };

        let count = header.num_nodes as usize;
        let table = slice(file, header.nodes_offset as usize, count * NODE_STRIDE)?;
        let link = |node: usize, v: i16| -> Result<Option<usize>, WorldError> {
            match v {
                v if v < 0 => Ok(None),
                v if (v as usize) < count => Ok(Some(v as usize)),
                v => Err(WorldError::BadLink(node, v as i32)),
            }
        };
        let mut nodes = Vec::with_capacity(count);
        for (i, e) in table.chunks(NODE_STRIDE).enumerate() {
            let end = e[..0x10].iter().position(|&c| c == 0).unwrap_or(0x10);
            nodes.push(WorldNode {
                name: String::from_utf8_lossy(&e[..end]).into_owned(),
                flags: le_u32(e, 0x10),
                render_flags: le_u32(e, 0x18),
                local_position: [le_f32(e, 0x1C), le_f32(e, 0x20), le_f32(e, 0x24)],
                has_model: le_u32(e, 0x28) == 1,
                next_sibling: link(i, le_u16(e, 0x2C) as i16)?,
                first_child: link(i, le_u16(e, 0x2E) as i16)?,
                radius: le_f32(e, 0x30),
                collision_disable: e[0x35],
                collision: match (le_u32(e, 0x38) as i32, le_u16(e, 0x36) as i16) {
                    (first, count) if first >= 0 && count > 0 => first as usize..first as usize + count as usize,
                    _ => 0..0,
                },
            });
        }
        let collision = CollisionTables::parse(file, &words)?;
        collision.validate(&nodes)?;
        Ok(Self { header, nodes, collision })
    }

    /// Each reachable node's world position, walking the tree from node 0
    /// in the game's order (node 0 and its siblings are the roots).
    /// Unreachable nodes get `None`.
    pub fn world_positions(&self) -> Result<Vec<Option<[f32; 3]>>, WorldError> {
        let mut out = vec![None; self.nodes.len()];
        if self.nodes.is_empty() {
            return Ok(out);
        }
        // (node, parent's world position)
        let mut stack = vec![(0usize, [0.0f32; 3])];
        while let Some((start, parent)) = stack.pop() {
            let mut cursor = Some(start);
            while let Some(i) = cursor {
                if out[i].is_some() {
                    return Err(WorldError::Cycle(i));
                }
                let n = &self.nodes[i];
                let p = [
                    parent[0] + n.local_position[0],
                    parent[1] + n.local_position[1],
                    parent[2] + n.local_position[2],
                ];
                out[i] = Some(p);
                if let Some(child) = n.first_child {
                    stack.push((child, p));
                }
                cursor = n.next_sibling;
            }
        }
        Ok(out)
    }
}

/// Header words of the animated objects: the version word, the clips
/// header (offset), how many, and their table (offset).
const VERSION_WORD: usize = 24;
const ANIM_CLIPS_WORD: usize = 25;
const ANIM_COUNT_WORD: usize = 26;
const ANIM_TABLE_WORD: usize = 27;
/// An animated object's record.
const ANIM_ENTRY: usize = 0x10;
/// A clips header's track table offset (its fifth word).
const CLIPS_TRACK_TABLE: usize = 0x10;
const TRACK_ENTRY: usize = 8;

/// A world object the level animates itself (`docs/worlds-format.md`,
/// "Animated objects"): one track, the same format as the characters'
/// clips, played at 30 frames a second — round and round, or forward and
/// back as the triggers aimed at it turn it on and off
/// (`docs/mechanics.md`).
#[derive(Debug, Clone)]
pub struct ObjectAnimation {
    /// The node it poses.
    pub node: usize,
    /// Its length in frames.
    pub frames: u16,
    /// Rotation (radians; the node's own, replacing none), translation
    /// (added to where the file puts the node) and scale. `None` when the
    /// track has no channels: the game leaves the node be.
    pub track: Option<Track>,
}

/// The level's animated objects (header words 25–27: the clips header,
/// the count and the table of `0x10`-byte records — node index (i16),
/// frame count (i16), two words the game uses at run time, the frame
/// (f32) and the file offset of the object's track entry). None without
/// the extended header or the clips.
pub fn object_animations(file: &[u8]) -> Result<Vec<ObjectAnimation>, WorldError> {
    let h = slice(file, 0, HEADER_WORDS * 4)?;
    let word = |i: usize| le_u32(h, i * 4);
    let (version, clips_at) = (word(VERSION_WORD), word(ANIM_CLIPS_WORD) as usize);
    if version & VERSION_MAGIC != VERSION_MAGIC || version & 0xFF == 0 || clips_at == 0 {
        return Ok(Vec::new());
    }
    let count = word(ANIM_COUNT_WORD) as usize;
    let table = slice(file, word(ANIM_TABLE_WORD) as usize, count * ANIM_ENTRY)?;
    let bad = |k: usize, why: String| WorldError::BadAnimation(k, why);
    let Some(clips) = anim::parse_clips(file, clips_at).map_err(|e| bad(0, e.to_string()))? else {
        return Ok(Vec::new());
    };
    let tracks = clips_at + le_u32(slice(file, clips_at, 0x1C)?, CLIPS_TRACK_TABLE) as usize;
    let nodes = word(0) as usize;
    table
        .chunks(ANIM_ENTRY)
        .enumerate()
        .map(|(k, e)| {
            let node = le_u16(e, 0) as i16;
            let frames = le_u16(e, 2) as i16;
            let entry = le_u32(e, 0xC) as usize;
            if node < 0 || node as usize >= nodes {
                return Err(bad(k, format!("node {node} of {nodes}")));
            }
            if frames < 1 {
                return Err(bad(k, format!("{frames} frames")));
            }
            // The entry is one of the clips' track entries: the object's
            // bone (one action).
            let rel = entry.checked_sub(tracks).filter(|r| r % TRACK_ENTRY == 0);
            let bone = rel.map(|r| r / TRACK_ENTRY).filter(|&b| b < clips.num_bones);
            let Some(bone) = bone else {
                return Err(bad(k, format!("track entry {entry:#x} isn't in the table at {tracks:#x}")));
            };
            let track = clips.track(bone, 0, frames as usize).map_err(|e| bad(k, e.to_string()))?;
            Ok(ObjectAnimation { node: node as usize, frames: frames as u16, track })
        })
        .collect()
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
    use crate::model::ModelFile;

    fn node(name: &str, pos: [f32; 3], sibling: i16, child: i16, model: bool) -> Vec<u8> {
        let mut e = vec![0u8; NODE_STRIDE];
        e[..name.len()].copy_from_slice(name.as_bytes());
        e[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
        for (k, c) in pos.iter().enumerate() {
            e[0x1C + k * 4..0x20 + k * 4].copy_from_slice(&c.to_le_bytes());
        }
        e[0x28..0x2C].copy_from_slice(&(model as u32).to_le_bytes());
        e[0x2C..0x2E].copy_from_slice(&sibling.to_le_bytes());
        e[0x2E..0x30].copy_from_slice(&child.to_le_bytes());
        e
    }

    #[test]
    fn positions_accumulate_down_the_tree() {
        // 0 -> sibling 1; 1 has child 2; 2 has sibling 3.
        let mut f = vec![0u8; HEADER_WORDS * 4];
        f[0..4].copy_from_slice(&4u32.to_le_bytes());
        f[4..8].copy_from_slice(&((HEADER_WORDS * 4) as u32).to_le_bytes());
        f.extend(node("ROOT", [1.0, 0.0, 0.0], 1, -1, false));
        f.extend(node("GROUP", [10.0, 0.0, 0.0], -1, 2, false));
        f.extend(node("A", [0.0, 5.0, 0.0], 3, -1, true));
        f.extend(node("B", [0.0, 0.0, 7.0], -1, -1, true));
        let world = WorldFile::parse(&f).unwrap();
        assert_eq!(world.nodes[2].name, "A");
        let p = world.world_positions().unwrap();
        assert_eq!(p[0], Some([1.0, 0.0, 0.0]));
        assert_eq!(p[2], Some([10.0, 5.0, 0.0]));
        assert_eq!(p[3], Some([10.0, 0.0, 7.0]));
    }

    #[test]
    fn rejects_out_of_range_links() {
        let mut f = vec![0u8; HEADER_WORDS * 4];
        f[0..4].copy_from_slice(&1u32.to_le_bytes());
        f[4..8].copy_from_slice(&((HEADER_WORDS * 4) as u32).to_le_bytes());
        f.extend(node("X", [0.0; 3], 5, -1, false));
        assert!(matches!(WorldFile::parse(&f), Err(WorldError::BadLink(0, 5))));
    }

    /// Every level's animated objects parse: nodes in range, every key
    /// finite (`docs/worlds-format.md`, "Animated objects").
    #[test]
    fn every_real_level_animation_parses() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let (mut levels, mut objects, mut keys) = (0, 0, 0);
        for level in entries.flatten().map(|e| e.path()) {
            let Ok(w) = std::fs::read(level.join("WORLDS.PS2")) else { continue };
            let world = WorldFile::parse(&w).unwrap_or_else(|e| panic!("{level:?}: {e}"));
            let anims = object_animations(&w).unwrap_or_else(|e| panic!("{level:?}: {e}"));
            for a in &anims {
                assert!(a.node < world.nodes.len(), "{level:?}");
                let Some(t) = &a.track else { continue };
                for (f, p) in &t.keys {
                    assert!(*f < a.frames, "{level:?} node {}: key at {f} of {}", a.node, a.frames);
                    let all = p.rotation.iter().chain(&p.translation).chain(&p.scale);
                    assert!(all.clone().all(|v| v.is_finite()), "{level:?} node {}", a.node);
                }
                keys += t.keys.len();
            }
            objects += anims.len();
            levels += 1;
        }
        eprintln!("{levels} worlds: {objects} animated objects, {keys} keys");
        assert!(levels == 0 || objects > 1000, "too few animated objects: {objects}");
    }

    /// Every level's WORLDS.PS2 parses, walks without cycles, and the model
    /// nodes it places mostly resolve by name in the same level's
    /// objects.ngc (the rest live in shared model files the game also
    /// searches).
    #[test]
    fn every_real_world_parses_and_links() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let (mut levels, mut placed, mut found) = (0, 0, 0);
        for level in entries.flatten().map(|e| e.path()) {
            let (Ok(w), Ok(m)) =
                (std::fs::read(level.join("WORLDS.PS2")), std::fs::read(level.join("objects.ngc")))
            else {
                continue;
            };
            let world = WorldFile::parse(&w).unwrap_or_else(|e| panic!("{level:?}: {e}"));
            assert_eq!(world.header.version & 0xFFFF_FF00, VERSION_MAGIC, "{level:?}");
            let positions = world.world_positions().unwrap_or_else(|e| panic!("{level:?}: {e}"));
            let model = ModelFile::parse(&m).unwrap();
            let names: std::collections::HashSet<_> = model.objects.iter().map(|o| o.name.as_str()).collect();
            for (n, p) in world.nodes.iter().zip(&positions) {
                if n.has_model && p.is_some() {
                    placed += 1;
                    found += names.contains(n.name.as_str()) as usize;
                }
            }
            levels += 1;
        }
        eprintln!("{levels} worlds: {placed} placed models, {found} found in their level's objects.ngc");
        assert!(levels == 0 || found * 10 > placed * 9, "too few nodes resolve: {found}/{placed}");
    }
}
