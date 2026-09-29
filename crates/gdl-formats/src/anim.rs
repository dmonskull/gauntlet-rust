//! `ANIM.PS2` — skeletons ("atrees"), their actions, and keyframed
//! animation clips.
//!
//! Confirmed against `main.dol`; see `docs/animation-format.md` for where
//! each rule comes from. Little-endian throughout.
//!
//! A file holds one or more atrees. An atree has:
//! - a **skeleton**: nodes with a name, a rest offset from their parent and
//!   a parent index (parents precede children). A node's model is the object
//!   named `<atree name><node name>` in the matching `objects.ngc`.
//! - **actions**: named clips (`READY`, `IDLE1`, `RUN`, ...) with a frame
//!   count and playback parameters.
//! - optionally **clips**: for every (bone, action) a track of up to nine
//!   channels (rotation xyz in radians, translation xyz, scale xyz),
//!   keyframed on a per-track bitmap and optionally delta-compressed through
//!   256-entry lookup tables.

use thiserror::Error;

const ATREE_ENTRY: usize = 0x24;
const ACTION_STRIDE: usize = 0x30;
const NODE_STRIDE: usize = 0x3C;
const TRACK_ENTRY: usize = 8;
const FLIPBOOK_STRIDE: usize = 0x28;
const DELTA_TABLE_LEN: usize = 256;

/// Track flag bits: which channels are stored.
pub const ROTATION_BITS: [u16; 3] = [0x001, 0x002, 0x004];
pub const TRANSLATION_BITS: [u16; 3] = [0x010, 0x020, 0x040];
pub const SCALE_BITS: [u16; 3] = [0x100, 0x200, 0x400];
/// Rotation matrix uses the alternate Euler order.
pub const ALT_EULER: u16 = 0x0080;
/// Keys after the first are one byte per channel into the delta tables.
pub const DELTA_KEYS: u16 = 0x2000;
/// A single key: the track is a static pose.
pub const STATIC_POSE: u16 = 0x4000;

/// Rotation keys only interpolate when they differ by less than this;
/// larger jumps hold the earlier key.
const ROTATION_LERP_LIMIT: f32 = std::f32::consts::FRAC_PI_2;
/// Within this many frames of the next key, the next key is used as-is.
const SNAP_TO_NEXT_KEY: f32 = 0.125;

#[derive(Debug, Error)]
pub enum AnimError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("node {0} has parent {1}, which isn't an earlier node")]
    BadParent(usize, i32),
    #[error("track for bone {bone}, action {action}: {why}")]
    BadTrack { bone: usize, action: usize, why: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Not animated.
    Static,
    /// Posed by a clip track.
    Skeletal,
    /// Swaps meshes per frame, one flipbook entry per action.
    Flipbook,
    /// Other animation kinds (kind 3: indexes the action table; kind 4:
    /// texture animation) — not implemented yet.
    Other(u16),
}

#[derive(Debug, Clone)]
pub struct SkeletonNode {
    pub name: String,
    /// Rest offset from the parent.
    pub offset: [f32; 3],
    pub parent: Option<usize>,
    pub kind: NodeKind,
    /// Node `+0x2E`; bit 0 = the node has no model of its own.
    pub node_flags: u16,
    /// Node `+0x30`: flags the game sets on the node's instance (bit 0
    /// hides it).
    pub render_flags: u32,
    /// Node `+0x34`: a byte offset, per `kind` — from the clips header to
    /// this bone's run of track entries, or from the flipbook list header to
    /// its first entry (negative = none). See [`Atree::clip_bone`] and
    /// [`Atree::flipbook_entry`].
    pub index: i32,
}

/// Instance render flag: hidden.
pub const RENDER_HIDDEN: u32 = 0x1;

impl SkeletonNode {
    pub fn has_model(&self) -> bool {
        self.node_flags & 1 == 0 && !self.name.is_empty()
    }

    pub fn hidden(&self) -> bool {
        self.render_flags & RENDER_HIDDEN != 0
    }
}

#[derive(Debug, Clone)]
pub struct Action {
    pub name: String,
    pub frames: u16,
    /// Second parameter: 30 for almost every action.
    pub rate: u16,
    /// Remaining parameters; the first is 1 on looping actions like
    /// `READY` and `IDLE2_LOOP`.
    pub params: [u16; 4],
}

impl Action {
    pub fn loops(&self) -> bool {
        self.params[0] & 1 != 0
    }
}

/// One channel group's value at a key or sample time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Radians.
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Pose {
    fn default() -> Self {
        Self { rotation: [0.0; 3], translation: [0.0; 3], scale: [1.0; 3] }
    }
}

/// A decoded track: key frame numbers and their poses.
#[derive(Debug, Clone)]
pub struct Track {
    pub flags: u16,
    pub keys: Vec<(u16, Pose)>,
}

impl Track {
    /// Samples at `frame` (fractional), following the game's rules: linear
    /// interpolation, rotations hold across jumps of π/2 or more, snap to
    /// the next key within 1/8 frame, rotations wrapped to (−π, π].
    pub fn sample(&self, frame: f32) -> Pose {
        let Some(&(_, first)) = self.keys.first() else {
            return Pose::default();
        };
        let next = self.keys.partition_point(|&(f, _)| (f as f32) <= frame);
        let mut pose = if next == 0 {
            first
        } else if next >= self.keys.len() {
            self.keys[self.keys.len() - 1].1
        } else {
            let (f0, a) = self.keys[next - 1];
            let (f1, b) = self.keys[next];
            if f1 as f32 - frame <= SNAP_TO_NEXT_KEY {
                b
            } else {
                let t = (frame - f0 as f32) / (f1 as f32 - f0 as f32);
                let lerp = |x: f32, y: f32| x + (y - x) * t;
                Pose {
                    rotation: std::array::from_fn(|i| {
                        let d = b.rotation[i] - a.rotation[i];
                        if d <= -ROTATION_LERP_LIMIT || d >= ROTATION_LERP_LIMIT {
                            a.rotation[i]
                        } else {
                            lerp(a.rotation[i], b.rotation[i])
                        }
                    }),
                    translation: std::array::from_fn(|i| lerp(a.translation[i], b.translation[i])),
                    scale: std::array::from_fn(|i| lerp(a.scale[i], b.scale[i])),
                }
            }
        };
        for r in &mut pose.rotation {
            if *r > std::f32::consts::PI {
                *r -= std::f32::consts::TAU;
            } else if *r <= -std::f32::consts::PI {
                *r += std::f32::consts::TAU;
            }
        }
        pose
    }
}

/// The keyframe data shared by an atree's actions.
#[derive(Debug, Clone)]
pub struct Clips {
    pub num_actions: usize,
    pub num_bones: usize,
    /// Delta lookup tables; absent (offset 0) in files whose keys are all
    /// full floats.
    rotation_deltas: Option<Vec<f32>>,
    translation_deltas: Option<Vec<f32>>,
    scale_deltas: Option<Vec<f32>>,
    /// Bone-major: `entries[bone * num_actions + action]`.
    entries: Vec<(u16, u16, u32)>,
    /// Byte offset of `entries` from the clips header.
    track_table: u32,
    keys: Vec<u8>,
}

/// One entry of an atree's object-animation ("flipbook") list: monsters
/// without skeletal clips animate by swapping whole pre-posed meshes.
/// There's one entry per (flipbook node, action), node-major; `first` names
/// the mesh for frame 0 and frame *k* is the *k*-th object after it in the
/// model file (whose objects are sorted by name).
#[derive(Debug, Clone)]
pub struct FlipbookEntry {
    pub first: String,
    pub frames: u16,
    pub param: u16,
}

#[derive(Debug, Clone)]
pub struct Atree {
    /// Model name prefix (e.g. `ARC_BLU`, or `ARC` for shared animation).
    pub name: String,
    pub nodes: Vec<SkeletonNode>,
    pub actions: Vec<Action>,
    pub clips: Option<Clips>,
    pub flipbook: Vec<FlipbookEntry>,
    /// Byte offset of the first flipbook entry from the list header.
    flipbook_base: u32,
}

impl Atree {
    pub fn node_index(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// The flipbook entry flipbook node `node` shows while playing `action`.
    pub fn flipbook_entry(&self, node: usize, action: usize) -> Option<&FlipbookEntry> {
        let n = self.nodes.get(node)?;
        let rel = (n.index as i64).checked_sub(self.flipbook_base as i64)?;
        if n.kind != NodeKind::Flipbook || rel < 0 || rel % FLIPBOOK_STRIDE as i64 != 0 {
            return None;
        }
        self.flipbook.get(rel as usize / FLIPBOOK_STRIDE + action)
    }

    /// The clip bone a skeletal node is animated by.
    pub fn clip_bone(&self, node: usize) -> Option<usize> {
        let n = self.nodes.get(node)?;
        let clips = self.clips.as_ref()?;
        let run = (clips.num_actions * TRACK_ENTRY) as i64;
        let rel = n.index as i64 - clips.track_table as i64;
        if n.kind != NodeKind::Skeletal || rel < 0 || run == 0 || rel % run != 0 {
            return None;
        }
        Some((rel / run) as usize).filter(|&b| b < clips.num_bones)
    }

    /// Decodes `bone`'s track for `action`. `None` when the bone isn't
    /// animated by it (it stays at its rest pose).
    pub fn track(&self, bone: usize, action: usize) -> Result<Option<Track>, AnimError> {
        let Some(clips) = &self.clips else { return Ok(None) };
        let frames = self.actions.get(action).map_or(0, |a| a.frames as usize);
        clips.track(bone, action, frames)
    }
}

impl Clips {
    fn track(&self, bone: usize, action: usize, frames: usize) -> Result<Option<Track>, AnimError> {
        let bad = |why: String| AnimError::BadTrack { bone, action, why };
        let Some(&(flags, channels, offset)) = self.entries.get(bone * self.num_actions + action) else {
            return Ok(None);
        };
        // No channel bits and no high byte: the game leaves the bone at rest.
        if flags & 0x0F == 0 && flags >> 8 == 0 {
            return Ok(None);
        }
        let groups = [ROTATION_BITS, TRANSLATION_BITS, SCALE_BITS];
        let present = groups.iter().flatten().filter(|&&b| flags & b != 0).count();
        if present != channels as usize {
            return Err(bad(format!("flags 0x{flags:04X} name {present} channels, entry says {channels}")));
        }
        let data = self.keys.get(offset as usize..).ok_or_else(|| bad("offset past key data".into()))?;

        let read_f32 = |at: usize| -> Result<f32, AnimError> {
            data.get(at..at + 4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .ok_or_else(|| bad("key data truncated".into()))
        };
        let read_full_key = |at: usize| -> Result<Pose, AnimError> {
            let mut pose = Pose::default();
            let mut n = 0;
            for (g, bits) in groups.iter().enumerate() {
                for (i, &bit) in bits.iter().enumerate() {
                    if flags & bit != 0 {
                        let v = read_f32(at + n * 4)?;
                        n += 1;
                        match g {
                            0 => pose.rotation[i] = v,
                            1 => pose.translation[i] = v,
                            _ => pose.scale[i] = v,
                        }
                    }
                }
            }
            Ok(pose)
        };

        if flags & STATIC_POSE != 0 {
            return Ok(Some(Track { flags, keys: vec![(0, read_full_key(0)?)] }));
        }

        let words = frames.div_ceil(32);
        let bitmap = data.get(..words * 4).ok_or_else(|| bad("keyframe bitmap truncated".into()))?;
        let key_frames: Vec<u16> = (0..frames)
            .filter(|&f| {
                let w = u32::from_le_bytes(bitmap[(f / 32) * 4..(f / 32) * 4 + 4].try_into().unwrap());
                w & (1 << (f % 32)) != 0
            })
            .map(|f| f as u16)
            .collect();
        let base = words * 4;
        let mut keys = Vec::with_capacity(key_frames.len().max(1));
        let first_frame = key_frames.first().copied().unwrap_or(0);
        let mut pose = read_full_key(base)?;
        keys.push((first_frame, pose));

        let delta = flags & DELTA_KEYS != 0;
        let n = channels as usize;
        for (k, &frame) in key_frames.iter().enumerate().skip(1) {
            if delta {
                let at = base + n * 4 + (k - 1) * n;
                let bytes = data.get(at..at + n).ok_or_else(|| bad("delta keys truncated".into()))?;
                let mut b = bytes.iter();
                for (g, bits) in groups.iter().enumerate() {
                    let table = match g {
                        0 => self.rotation_deltas.as_ref(),
                        1 => self.translation_deltas.as_ref(),
                        _ => self.scale_deltas.as_ref(),
                    };
                    for (i, &bit) in bits.iter().enumerate() {
                        if flags & bit != 0 {
                            let index = *b.next().unwrap() as usize;
                            let d = table.ok_or_else(|| bad("delta keys without a delta table".into()))?[index];
                            match g {
                                0 => pose.rotation[i] += d,
                                1 => pose.translation[i] += d,
                                _ => pose.scale[i] += d,
                            }
                        }
                    }
                }
            } else {
                pose = read_full_key(base + k * n * 4)?;
            }
            keys.push((frame, pose));
        }
        Ok(Some(Track { flags, keys }))
    }
}

#[derive(Debug, Clone)]
pub struct AnimFile {
    pub atrees: Vec<Atree>,
}

impl AnimFile {
    pub fn parse(file: &[u8]) -> Result<Self, AnimError> {
        let h = slice(file, 0, 0x10)?;
        let count = le_u16(h, 0) as usize;
        let table = le_u32(h, 4) as usize;
        let mut atrees = Vec::with_capacity(count);
        for i in 0..count {
            let e = slice(file, table + i * ATREE_ENTRY, ATREE_ENTRY)?;
            let at = le_u32(e, 0x20) as usize;
            atrees.push(parse_atree(file, at)?);
        }
        Ok(Self { atrees })
    }
}

fn parse_atree(file: &[u8], at: usize) -> Result<Atree, AnimError> {
    let h = slice(file, at, 0x38)?;
    let rel = |i: usize| at + le_u32(h, i * 4) as usize;
    let actions_off = le_u32(h, 0);
    let clips_off = le_u32(h, 4);
    let nodes_at = rel(3);
    let num_nodes = le_u32(h, 0x10) as usize;
    let num_actions = le_u32(h, 0x14) as usize;
    let name = cstr(&h[0x18..0x36]);

    let mut nodes = Vec::with_capacity(num_nodes);
    for (i, e) in slice(file, nodes_at, num_nodes * NODE_STRIDE)?.chunks(NODE_STRIDE).enumerate() {
        let parent = le_u32(e, 0x38) as i32;
        let parent = match parent {
            p if p < 0 => None,
            p if (p as usize) < i => Some(p as usize),
            p => return Err(AnimError::BadParent(i, p)),
        };
        nodes.push(SkeletonNode {
            name: cstr(&e[..0x20]),
            offset: [le_f32(e, 0x20), le_f32(e, 0x24), le_f32(e, 0x28)],
            parent,
            kind: match le_u16(e, 0x2C) {
                0 => NodeKind::Static,
                1 => NodeKind::Skeletal,
                2 => NodeKind::Flipbook,
                k => NodeKind::Other(k),
            },
            node_flags: le_u16(e, 0x2E),
            render_flags: le_u32(e, 0x30),
            index: le_u32(e, 0x34) as i32,
        });
    }

    let mut actions = Vec::with_capacity(num_actions);
    if actions_off != 0 {
        let base = at + actions_off as usize;
        for e in slice(file, base, num_actions * ACTION_STRIDE)?.chunks(ACTION_STRIDE) {
            actions.push(Action {
                name: cstr(&e[..0x20]),
                frames: le_u16(e, 0x20),
                rate: le_u16(e, 0x22),
                params: [le_u16(e, 0x24), le_u16(e, 0x26), le_u16(e, 0x28), le_u16(e, 0x2A)],
            });
        }
    }

    let clips = if clips_off != 0 { parse_clips(file, at + clips_off as usize)? } else { None };
    let flipbook_off = le_u32(h, 8);
    let (flipbook, flipbook_base) =
        if flipbook_off != 0 { parse_flipbook(file, at + flipbook_off as usize)? } else { (Vec::new(), 0) };
    Ok(Atree { name, nodes, actions, clips, flipbook, flipbook_base })
}

/// `{u32 offset from here, u32 count}`, then `0x28`-byte entries: object
/// name[0x20], (runtime object slot), frame count, a parameter.
fn parse_flipbook(file: &[u8], at: usize) -> Result<(Vec<FlipbookEntry>, u32), AnimError> {
    let h = slice(file, at, 8)?;
    let rel = le_u32(h, 0);
    let count = le_u32(h, 4) as usize;
    let entries = slice(file, at + rel as usize, count * FLIPBOOK_STRIDE)?
        .chunks(FLIPBOOK_STRIDE)
        .map(|e| FlipbookEntry { first: cstr(&e[..0x20]), frames: le_u16(e, 0x24), param: le_u16(e, 0x26) })
        .collect();
    Ok((entries, rel))
}

/// `None` for the empty block skeleton-only files carry (no actions, no
/// bones, every offset pointing back at the header).
fn parse_clips(file: &[u8], at: usize) -> Result<Option<Clips>, AnimError> {
    let h = slice(file, at, 0x1C)?;
    let off = |i: usize| le_u32(h, i * 4) as usize;
    let num_actions = le_u32(h, 0x14) as usize;
    let num_bones = le_u32(h, 0x18) as usize;
    if num_actions == 0 || num_bones == 0 {
        return Ok(None);
    }
    // An offset of 0 points back at this header: no table.
    let table = |o: usize| -> Result<Option<Vec<f32>>, AnimError> {
        if o == 0 {
            return Ok(None);
        }
        Ok(Some(
            slice(file, at + o, DELTA_TABLE_LEN * 4)?
                .chunks(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        ))
    };
    let entries = slice(file, at + off(4), num_bones * num_actions * TRACK_ENTRY)?
        .chunks(TRACK_ENTRY)
        .map(|e| (le_u16(e, 0), le_u16(e, 2), le_u32(e, 4)))
        .collect();
    Ok(Some(Clips {
        num_actions,
        num_bones,
        rotation_deltas: table(off(0))?,
        translation_deltas: table(off(1))?,
        scale_deltas: table(off(2))?,
        entries,
        track_table: off(4) as u32,
        keys: file.get(at + off(3)..).unwrap_or_default().to_vec(),
    }))
}

/// The game's two rotation builders, as row-major 4×4 matrices for row
/// vectors (`v' = v · M`). The same 16 numbers, read column-major, are the
/// equivalent column-vector matrix.
pub fn rotation_matrix(rotation: [f32; 3], flags: u16) -> [f32; 16] {
    let (ca, sa) = (rotation[0].cos(), -rotation[0].sin());
    let (cb, sb) = (rotation[1].cos(), -rotation[1].sin());
    let (cc, sc) = (rotation[2].cos(), -rotation[2].sin());
    let mut m = [0.0f32; 16];
    if flags & ALT_EULER == 0 {
        m[0] = cc * cb;
        m[1] = sc * cb;
        m[2] = sb;
        m[4] = -sc * ca + (-cc * sb) * sa;
        m[5] = cc * ca + (-sc * sb) * sa;
        m[6] = cb * sa;
        m[8] = sc * sa + (-cc * sb) * ca;
        m[9] = -cc * sa + (-sc * sb) * ca;
        m[10] = cb * ca;
    } else {
        m[0] = cb * cc;
        m[4] = -cb * sc;
        m[8] = -sb;
        m[1] = -(sa * sb) * cc + ca * sc;
        m[5] = (sa * sb) * sc + ca * cc;
        m[9] = -sa * cb;
        m[2] = (ca * sb) * cc + sa * sc;
        m[6] = (ca * sb) * -sc + sa * cc;
        m[10] = ca * cb;
    }
    m[15] = 1.0;
    m
}

fn slice(file: &[u8], at: usize, len: usize) -> Result<&[u8], AnimError> {
    file.get(at..at + len).ok_or(AnimError::Truncated(at, at + len))
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
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

    fn pose(r: f32, t: f32) -> Pose {
        Pose { rotation: [r, 0.0, 0.0], translation: [t, 0.0, 0.0], scale: [1.0; 3] }
    }

    #[test]
    fn sampling_follows_the_game_rules() {
        let track = Track { flags: 0x11, keys: vec![(0, pose(0.0, 0.0)), (4, pose(1.0, 8.0)), (8, pose(3.0, 0.0))] };
        let p = track.sample(2.0);
        assert!((p.rotation[0] - 0.5).abs() < 1e-6 && (p.translation[0] - 4.0).abs() < 1e-6);
        // Rotation jump of 2.0 rad (>= pi/2) holds the earlier key.
        assert_eq!(track.sample(6.0).rotation[0], 1.0);
        assert_eq!(track.sample(6.0).translation[0], 4.0);
        // Within 1/8 frame of the next key: use it.
        assert_eq!(track.sample(3.9), pose(1.0, 8.0));
        // Past the end: last key, rotation wrapped to (-pi, pi].
        assert_eq!(track.sample(20.0).rotation[0], 3.0);
    }

    #[test]
    fn rotation_matrices_are_orthonormal() {
        for flags in [0, ALT_EULER] {
            let m = rotation_matrix([0.3, -1.1, 2.0], flags);
            let row = |r: usize| [m[r * 4], m[r * 4 + 1], m[r * 4 + 2]];
            for i in 0..3 {
                for j in 0..3 {
                    let d: f32 = (0..3).map(|k| row(i)[k] * row(j)[k]).sum();
                    assert!((d - if i == j { 1.0 } else { 0.0 }).abs() < 1e-5, "{flags} {i} {j} {d}");
                }
            }
        }
        assert_eq!(rotation_matrix([0.0; 3], 0)[..11], [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }

    fn anim_files() -> Vec<(String, Vec<u8>)> {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let mut out = Vec::new();
        fn walk(dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_uppercase().ends_with("ANIM.PS2")) {
                    out.push((p.display().to_string(), std::fs::read(&p).unwrap()));
                }
            }
        }
        walk(std::path::Path::new(&root), &mut out);
        out
    }

    /// Every ANIM.PS2 on the disc parses, and every track of every action
    /// decodes to finite values within the file.
    #[test]
    fn every_real_anim_file_decodes() {
        let files = anim_files();
        let (mut atrees, mut tracks, mut keys, mut flipbooks) = (0, 0, 0, 0);
        for (path, data) in &files {
            let anim = AnimFile::parse(data).unwrap_or_else(|e| panic!("{path}: {e}"));
            for tree in &anim.atrees {
                atrees += 1;
                for (i, n) in tree.nodes.iter().enumerate() {
                    match n.kind {
                        NodeKind::Flipbook if !tree.flipbook.is_empty() => {
                            flipbooks += 1;
                            for a in 0..tree.actions.len() {
                                assert!(tree.flipbook_entry(i, a).is_some(), "{path} {}: {} action {a}", tree.name, n.name);
                            }
                        }
                        NodeKind::Skeletal if tree.clips.is_some() => {
                            assert!(tree.clip_bone(i).is_some(), "{path} {}: {} index {}", tree.name, n.name, n.index);
                        }
                        _ => {}
                    }
                }
                let Some(clips) = &tree.clips else { continue };
                assert_eq!(clips.num_actions, tree.actions.len(), "{path}");
                // Effect atrees can carry clips for more bones than nodes.
                for bone in 0..clips.num_bones {
                    for action in 0..clips.num_actions {
                        let Some(t) = tree.track(bone, action).unwrap_or_else(|e| panic!("{path}: {e}")) else {
                            continue;
                        };
                        tracks += 1;
                        keys += t.keys.len();
                        for (_, p) in &t.keys {
                            let all = p.rotation.iter().chain(&p.translation).chain(&p.scale);
                            assert!(all.clone().all(|v| v.is_finite() && v.abs() < 1.0e4), "{path} {bone} {action} {p:?}");
                        }
                    }
                }
            }
        }
        if !files.is_empty() {
            eprintln!("{} files, {atrees} atrees, {flipbooks} flipbook nodes, {tracks} tracks, {keys} keys", files.len());
        }
    }
}
