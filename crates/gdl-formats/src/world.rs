//! `WORLDS.PS2` — a level's scene graph: named nodes placing models.
//!
//! Confirmed against `main.dol` (see `docs/worlds-format.md` for where):
//! the game walks the tree (next sibling at node `+0x2C`, first child at
//! `+0x2E`), builds each node's instance at its translation *relative to its
//! parent*, and finds its model by binary-searching every loaded model
//! file's name table for the node's name. Little-endian like the models.

use thiserror::Error;

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
}

#[derive(Debug, Clone)]
pub struct WorldHeader {
    pub num_nodes: u32,
    pub nodes_offset: u32,
    /// Second table: `0x28`-byte records holding two vec3s (bounds?).
    pub num_regions: u32,
    pub regions_offset: u32,
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
    /// Translation relative to the parent node.
    pub local_position: [f32; 3],
    /// Node `+0x28 == 1`: the node draws the model with its name.
    pub has_model: bool,
    pub next_sibling: Option<usize>,
    pub first_child: Option<usize>,
    /// Node `+0x30`; equals the model record's `+0x04` bounding value.
    pub radius: f32,
}

#[derive(Debug, Clone)]
pub struct WorldFile {
    pub header: WorldHeader,
    pub nodes: Vec<WorldNode>,
}

impl WorldFile {
    pub fn parse(file: &[u8]) -> Result<Self, WorldError> {
        let h = slice(file, 0, HEADER_WORDS * 4)?;
        let words: [u32; HEADER_WORDS] = std::array::from_fn(|i| le_u32(h, i * 4));
        let f = |i: usize| f32::from_bits(words[i]);
        let header = WorldHeader {
            num_nodes: words[0],
            nodes_offset: words[1],
            num_regions: words[2],
            regions_offset: words[3],
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
                local_position: [le_f32(e, 0x1C), le_f32(e, 0x20), le_f32(e, 0x24)],
                has_model: le_u32(e, 0x28) == 1,
                next_sibling: link(i, le_u16(e, 0x2C) as i16)?,
                first_child: link(i, le_u16(e, 0x2E) as i16)?,
                radius: le_f32(e, 0x30),
            });
        }
        Ok(Self { header, nodes })
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
