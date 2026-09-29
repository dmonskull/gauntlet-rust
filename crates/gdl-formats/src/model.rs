//! `objects.ngc` — a level's (or character's) models: header, texture
//! bindings, name tables, and geometry.
//!
//! Everything here is confirmed against `main.dol`; see
//! `docs/objects-ngc-format.md` for the full writeup and which function each
//! rule comes from. In short:
//!
//! - The file is little-endian throughout (shared with the PS2 build). The
//!   load-time fixup `FUN_800b7534` byte-swaps the header and tables.
//! - Geometry is stored as **PS2 VIF packets** (DMA-tagged `UNPACK` streams
//!   for the PS2's vector unit). The GameCube build interprets them in
//!   software in `FUN_800c48c0`; [`Submesh::decode`] follows that function's
//!   rules exactly, including its scale constants (read from `main.dol`'s
//!   small-data area: positions ÷128, normals `(v−15)/15`, UVs ÷128).

use thiserror::Error;

/// Version values observed on real disc data.
pub const KNOWN_VERSIONS: [u32; 2] = [0xF00B000D, 0xF00B000C];

const HEADER_OFFSET: usize = 0x40;
const OBJECT_STRIDE: usize = 0x40;
const BINDING_STRIDE: usize = 0x40;
const OBJECT_NAME_STRIDE: usize = 0x18;
const TEXTURE_NAME_STRIDE: usize = 0x24;
/// Current format (`0xF00B000D`): strip table entries are 4 × u16.
const STRIP_ENTRY_STRIDE: usize = 8;
/// Older format (`0xF00B000C`, only `levelL2` on the retail disc — which the
/// game itself flags with its "bad version" warning): 3 × u16, no
/// `lightmap_param`.
const LEGACY_STRIP_ENTRY_STRIDE: usize = 6;
const LEGACY_VERSION: u32 = 0xF00B000C;

/// `FUN_800c48c0` divides unpacked positions by this (`r2-0x481c`).
pub const POSITION_DIVISOR: f32 = 128.0;
/// Normal components are 5-bit, biased by 15 and divided by 15 (`r2-0x482c`).
pub const NORMAL_DIVISOR: f32 = 15.0;
/// UVs are divided by 32768 (`r2-0x4830`) then scaled by 256 (`r2-0x4800` ×
/// the default texture transform at `0x80127b68`), i.e. ÷128 overall.
pub const UV_DIVISOR: f32 = 128.0;
/// The second UV pair is divided by 32768 with no texture-matrix scale.
pub const LIGHTMAP_UV_DIVISOR: f32 = 32768.0;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("unrecognized version 0x{0:08X} (known: {KNOWN_VERSIONS:X?})")]
    UnknownVersion(u32),
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
    #[error("object {object} submesh {submesh}: {why}")]
    BadGeometry { object: usize, submesh: usize, why: String },
}

/// The header at `0x40..0x80`. `*_offset` fields are file-relative.
#[derive(Debug, Clone)]
pub struct ModelHeader {
    pub version: u32,
    pub num_objects: u32,
    pub num_bindings: u32,
    /// Always equal to `num_objects` on real data; counts the name table.
    pub num_object_names: u32,
    pub num_texture_names: u32,
    pub objects_offset: u32,
    pub bindings_offset: u32,
    pub object_names_offset: u32,
    pub texture_names_offset: u32,
    /// Extra submesh descriptors for objects with more than one submesh.
    pub strip_table_offset: u32,
    /// Start of the VIF geometry packets.
    pub geometry_offset: u32,
    pub unnamed_0x6c: u32,
    pub unnamed_0x70: u32,
    pub unnamed_0x74: u32,
    pub unnamed_0x78: u32,
    pub unnamed_0x7c: u16,
    pub unnamed_0x7e: u16,
}

impl ModelHeader {
    pub fn parse(file: &[u8]) -> Result<Self, ModelError> {
        let b = slice(file, HEADER_OFFSET, 0x40)?;
        let version = le_u32(b, 0x00);
        if !KNOWN_VERSIONS.contains(&version) {
            return Err(ModelError::UnknownVersion(version));
        }
        Ok(Self {
            version,
            num_objects: le_u32(b, 0x04),
            num_bindings: le_u32(b, 0x08),
            num_object_names: le_u32(b, 0x0C),
            num_texture_names: le_u32(b, 0x10),
            objects_offset: le_u32(b, 0x14),
            bindings_offset: le_u32(b, 0x18),
            object_names_offset: le_u32(b, 0x1C),
            texture_names_offset: le_u32(b, 0x20),
            strip_table_offset: le_u32(b, 0x24),
            geometry_offset: le_u32(b, 0x28),
            unnamed_0x6c: le_u32(b, 0x2C),
            unnamed_0x70: le_u32(b, 0x30),
            unnamed_0x74: le_u32(b, 0x34),
            unnamed_0x78: le_u32(b, 0x38),
            unnamed_0x7c: le_u16(b, 0x3C),
            unnamed_0x7e: le_u16(b, 0x3E),
        })
    }
}

/// Binds a submesh to a texture in the sibling `textures.ngc`. Submeshes
/// refer to these by index (the low 16 bits of the game's `texidx`).
/// Confirmed from `FUN_800c7510` (load-time bind), `FUN_800c70c4` (GX
/// texture object setup) and `FUN_800c6d0c` (draw-time bind).
#[derive(Debug, Clone, Copy)]
pub struct MaterialBinding {
    /// On-disk `+0x00`: texture format selector — see
    /// [`crate::texture::TextureFormat::from_selector`].
    pub format: u8,
    /// On-disk `+0x08`. Runtime flags; bit `0x100` = untextured.
    pub flags_raw: u16,
    /// On-disk `+0x0C`: offset of the texture (palette first, if any) in
    /// `textures.ngc`.
    pub texture_offset: u32,
    /// On-disk `+0x16`. Zero means no texture.
    pub width: u16,
    /// On-disk `+0x18`. Zero means no texture.
    pub height: u16,
}

impl MaterialBinding {
    fn parse(entry: &[u8]) -> Self {
        Self {
            format: entry[0],
            flags_raw: le_u16(entry, 0x08),
            texture_offset: le_u32(entry, 0x0C),
            width: le_u16(entry, 0x16),
            height: le_u16(entry, 0x18),
        }
    }

    /// Mirrors `FUN_800c7510`: zero width or height means untextured.
    pub fn is_textured(&self) -> bool {
        self.width != 0 && self.height != 0
    }
}

/// A named texture (animated/scrolling textures are looked up by name).
#[derive(Debug, Clone)]
pub struct TextureName {
    pub name: String,
    pub binding: u16,
    pub width: u16,
    pub height: u16,
}

/// Which textures and how much packet data one submesh uses. The first
/// submesh's descriptor lives in the object record at `+0x10`; the rest in
/// the strip table. Field meanings come from the draw loop `FUN_800c3bbc`:
/// `+2` goes through `FUN_800c3d60` to `FUN_800c6a78` (GX texture map 0),
/// `+4` to `FUN_800c6bf0` (GX texture map 1, enabled only when non-zero).
#[derive(Debug, Clone, Copy)]
pub struct SubmeshDescriptor {
    /// Packet size in 16-byte quadwords, including the DMA tag.
    pub qwords: u16,
    /// Diffuse texture: index into [`ModelFile::bindings`], sampled with
    /// [`Vertex::uv`].
    pub texture: u16,
    /// Lightmap: index into [`ModelFile::bindings`], sampled with
    /// [`Vertex::lightmap_uv`]. `0` means none.
    pub lightmap: u16,
    /// Passed alongside the lightmap; meaning not confirmed yet.
    pub lightmap_param: i16,
}

impl SubmeshDescriptor {
    fn parse(b: &[u8]) -> Self {
        Self {
            qwords: le_u16(b, 0),
            texture: le_u16(b, 2),
            lightmap: le_u16(b, 4),
            lightmap_param: le_u16(b, 6) as i16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Prelit vertex colour, when the packet carries one.
    pub color: Option<[u8; 3]>,
    pub uv: [f32; 2],
    /// Second UV set (the `V4-16` UV unpack's 3rd/4th components, ÷32768),
    /// present only with that unpack format.
    pub lightmap_uv: Option<[f32; 2]>,
}

#[derive(Debug, Clone)]
pub struct Submesh {
    pub descriptor: SubmeshDescriptor,
    pub vertices: Vec<Vertex>,
    /// Triangle list (indices into `vertices`) with the game's strip
    /// splitting and winding already applied.
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Debug, Clone)]
pub struct ModelObject {
    pub name: String,
    pub name_hash: u32,
    pub flags: u32,
    pub submeshes: Vec<Submesh>,
}

/// A fully parsed `objects.ngc`.
#[derive(Debug, Clone)]
pub struct ModelFile {
    pub header: ModelHeader,
    pub bindings: Vec<MaterialBinding>,
    pub texture_names: Vec<TextureName>,
    pub objects: Vec<ModelObject>,
}

impl ModelFile {
    pub fn parse(file: &[u8]) -> Result<Self, ModelError> {
        let header = ModelHeader::parse(file)?;

        let bindings = table(file, header.bindings_offset, header.num_bindings, BINDING_STRIDE)?
            .map(MaterialBinding::parse)
            .collect();

        let texture_names =
            table(file, header.texture_names_offset, header.num_texture_names, TEXTURE_NAME_STRIDE)?
                .map(|e| TextureName {
                    name: cstr(&e[..0x1E]),
                    binding: le_u16(e, 0x1E),
                    width: le_u16(e, 0x20),
                    height: le_u16(e, 0x22),
                })
                .filter(|t| !t.name.is_empty())
                .collect();

        // Name table entries point back at their object by index.
        let mut names = vec![String::new(); header.num_objects as usize];
        for e in table(file, header.object_names_offset, header.num_object_names, OBJECT_NAME_STRIDE)? {
            let index = le_u16(e, 0x14) as usize;
            if let Some(slot) = names.get_mut(index) {
                *slot = cstr(&e[..0x10]);
            }
        }

        let mut objects = Vec::with_capacity(header.num_objects as usize);
        for (i, rec) in table(file, header.objects_offset, header.num_objects, OBJECT_STRIDE)?.enumerate() {
            let count = le_u32(rec, 0x0C) as usize;
            let strip_table = le_u32(rec, 0x18) as usize;
            let mut at = le_u32(rec, 0x1C) as usize;

            let mut descriptors = Vec::with_capacity(count);
            if count > 0 {
                descriptors.push(SubmeshDescriptor::parse(&rec[0x10..0x18]));
            }
            let stride = if header.version == LEGACY_VERSION {
                LEGACY_STRIP_ENTRY_STRIDE
            } else {
                STRIP_ENTRY_STRIDE
            };
            for k in 1..count {
                let e = slice(file, strip_table + (k - 1) * stride, stride)?;
                let mut entry = [0u8; STRIP_ENTRY_STRIDE];
                entry[..stride].copy_from_slice(e);
                descriptors.push(SubmeshDescriptor::parse(&entry));
            }

            let mut submeshes = Vec::with_capacity(count);
            for (k, descriptor) in descriptors.into_iter().enumerate() {
                let len = descriptor.qwords as usize * 16;
                let packet = slice(file, at, len)?;
                let (vertices, triangles) = decode_packet(packet).map_err(|why| ModelError::BadGeometry {
                    object: i,
                    submesh: k,
                    why,
                })?;
                submeshes.push(Submesh { descriptor, vertices, triangles });
                at += len;
            }

            objects.push(ModelObject {
                name: std::mem::take(&mut names[i]),
                name_hash: le_u32(rec, 0x04),
                flags: le_u32(rec, 0x08),
                submeshes,
            });
        }

        Ok(Self { header, bindings, texture_names, objects })
    }
}

/// Decodes one DMA-tagged VIF packet following `FUN_800c48c0`.
fn decode_packet(packet: &[u8]) -> Result<(Vec<Vertex>, Vec<[u32; 3]>), String> {
    let words = packet.len() / 4;
    let qwc = le_u16(packet, 0) as usize;
    if (qwc + 1) * 16 != packet.len() {
        return Err(format!("DMA tag says {qwc}+1 qwords, descriptor says {}", packet.len() / 16));
    }
    let word = |i: usize| -> Result<u32, String> {
        if i < words { Ok(le_u32(packet, i * 4)) } else { Err(format!("read past packet end (word {i})")) }
    };
    let cmd = |i: usize| -> Result<u8, String> { Ok((word(i)? >> 24) as u8) };
    let bytes = |word_index: usize| &packet[(word_index * 4).min(packet.len())..];

    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut i = 2; // skip the 64-bit DMA tag
    while i < words {
        if word(i)? == 0 {
            break;
        }
        let n = word(i + 1)? as usize;
        if n > 0x400 {
            return Err(format!("implausible batch vertex count {n}"));
        }

        // Positions (count n+1: the unpack carries one trailing entry).
        let pos_code = i + 5;
        let (pos_stride, pos_words) = match cmd(pos_code)? {
            0x69 => (6, ((n + 1) * 48).div_ceil(32)),
            0x6A => (3, ((n + 1) * 24).div_ceil(32)),
            _ => (12, (n + 1) * 3),
        };
        let pos_kind = cmd(pos_code)?;
        let positions = bytes(pos_code + 1);

        let packed_words = (n * 16).div_ceil(32) + 1; // incl. VIF code
        let normal_code = pos_code + pos_words + 1;
        let normals = bytes(normal_code + 1);
        let mut uv_code = normal_code + packed_words;

        // Optional vertex colours: the next unpack targets VU address 3.
        let colors = if word(uv_code)? & 0x0FFF == 3 {
            let c = bytes(uv_code + 1);
            uv_code += packed_words;
            Some(c)
        } else {
            None
        };

        let uv_kind = cmd(uv_code)?;
        let uvs = bytes(uv_code + 1);
        let (uv_stride, after_uv) = match uv_kind {
            0x6D => (8, uv_code + n * 2 + 1),
            0x66 => (2, uv_code + packed_words),
            _ => (4, uv_code + n + 1),
        };
        i = after_uv + 1; // skip MSCAL/MSCNT

        let need = |buf: &[u8], len: usize, what: &str| {
            if buf.len() < len { Err(format!("{what} data truncated")) } else { Ok(()) }
        };
        need(positions, n * pos_stride, "position")?;
        need(normals, n * 2, "normal")?;
        need(uvs, n * uv_stride, "uv")?;
        if let Some(c) = colors {
            need(c, n * 2, "color")?;
        }

        let base = vertices.len() as u32;
        let mut restart = Vec::with_capacity(n);
        for v in 0..n {
            let p = &positions[v * pos_stride..];
            let raw = match pos_kind {
                0x69 => [le_i16(p, 0) as f32, le_i16(p, 2) as f32, le_i16(p, 4) as f32],
                0x6A => [p[0] as i8 as f32, p[1] as i8 as f32, p[2] as i8 as f32],
                _ => [le_i32(p, 0) as f32, le_i32(p, 4) as f32, le_i32(p, 8) as f32],
            };
            let packed = le_u16(normals, v * 2);
            let axis = |shift: u16| (((packed >> shift) & 0x1F) as f32 - 15.0) / NORMAL_DIVISOR;
            let uv = match uv_kind {
                0x66 => [uvs[v * 2] as f32, uvs[v * 2 + 1] as f32],
                _ => [le_u16(uvs, v * uv_stride) as f32, le_u16(uvs, v * uv_stride + 2) as f32],
            };
            let color = colors.map(|c| {
                let packed = le_u16(c, v * 2);
                let ch = |shift: u16| (((packed >> shift) & 0x1F) << 3) as u8;
                [ch(0), ch(5), ch(10)]
            });
            let lightmap_uv = (uv_kind == 0x6D).then(|| {
                let at = v * uv_stride;
                [le_u16(uvs, at + 4) as f32, le_u16(uvs, at + 6) as f32].map(|c| c / LIGHTMAP_UV_DIVISOR)
            });
            restart.push(packed & 0x8000 != 0);
            vertices.push(Vertex {
                position: raw.map(|c| c / POSITION_DIVISOR),
                normal: [axis(0), axis(5), axis(10)],
                color,
                uv: uv.map(|c| c / UV_DIVISOR),
                lightmap_uv,
            });
        }

        // Strip splitting exactly as FUN_800c48c0 does before each
        // FUN_800c4254 (GX triangle strip) call.
        let mut start = 0;
        for (v, &flag) in restart.iter().enumerate() {
            if flag && v - start > 1 {
                strip_to_triangles(base, start, v, &mut triangles);
                start = v - 1;
            }
        }
        strip_to_triangles(base, start, n, &mut triangles);
    }
    Ok((vertices, triangles))
}

fn strip_to_triangles(base: u32, start: usize, end: usize, out: &mut Vec<[u32; 3]>) {
    for k in start + 2..end {
        let (a, b, c) = (base + k as u32 - 2, base + k as u32 - 1, base + k as u32);
        out.push(if (k - start).is_multiple_of(2) { [a, b, c] } else { [b, a, c] });
    }
}

fn table(
    file: &[u8],
    offset: u32,
    count: u32,
    stride: usize,
) -> Result<std::slice::Chunks<'_, u8>, ModelError> {
    Ok(slice(file, offset as usize, count as usize * stride)?.chunks(stride))
}

fn slice(file: &[u8], at: usize, len: usize) -> Result<&[u8], ModelError> {
    file.get(at..at + len).ok_or(ModelError::Truncated(at, at + len))
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le_i32(b: &[u8], at: usize) -> i32 {
    le_u32(b, at) as i32
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

fn le_i16(b: &[u8], at: usize) -> i16 {
    le_u16(b, at) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn levels() -> Vec<(String, Vec<u8>, Vec<u8>)> {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = Path::new(&root).join("LEVELS");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return Vec::new();
        };
        let mut out: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let objects = std::fs::read(p.join("objects.ngc")).ok()?;
                let textures = std::fs::read(p.join("textures.ngc")).ok()?;
                Some((e.file_name().to_string_lossy().into_owned(), objects, textures))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Hand-built packet: one batch of a 4-vertex strip with the restart
    /// flag on the first two vertices, s16 positions, V4-16 UVs, no colour.
    fn quad_packet() -> Vec<u8> {
        let mut p = Vec::new();
        let push32 = |p: &mut Vec<u8>, v: u32| p.extend_from_slice(&v.to_le_bytes());
        // DMA tag (qwc patched below), then header unpack: code + 4 words.
        push32(&mut p, 0);
        push32(&mut p, 0);
        push32(&mut p, 0x6C01_8000);
        for v in [4u32, 0, 1.0f32.to_bits(), (-1.0f32).to_bits()] {
            push32(&mut p, v);
        }
        // Positions: V3-16, 5 entries (4 + trailer) = 30 bytes -> 8 words.
        push32(&mut p, 0x6905_8001);
        let pos: [[i16; 3]; 5] = [[0, 0, 0], [128, 0, 0], [0, 128, 0], [128, 128, 0], [1, 0, 0]];
        let mut raw = Vec::new();
        for v in pos {
            for c in v {
                raw.extend_from_slice(&c.to_le_bytes());
            }
        }
        raw.resize(32, 0);
        p.extend_from_slice(&raw);
        // Normals V4-5: +z, restart flag on vertices 0 and 1.
        push32(&mut p, 0x6F04_8002);
        let nz = |restart: bool| (15u16 | 15 << 5 | 30 << 10) | if restart { 0x8000 } else { 0 };
        for r in [true, true, false, false] {
            p.extend_from_slice(&nz(r).to_le_bytes());
        }
        // UVs V4-16 unsigned.
        push32(&mut p, 0x6D04_C004);
        for uv in [[0u16, 0], [128, 0], [0, 128], [128, 128]] {
            for c in [uv[0], uv[1], 0, 0] {
                p.extend_from_slice(&c.to_le_bytes());
            }
        }
        push32(&mut p, 0x1400_0000); // MSCAL
        while p.len() % 16 != 0 {
            p.push(0);
        }
        let qwc = (p.len() / 16 - 1) as u16;
        p[0..2].copy_from_slice(&qwc.to_le_bytes());
        p
    }

    #[test]
    fn decodes_a_hand_built_strip() {
        let (verts, tris) = decode_packet(&quad_packet()).unwrap();
        assert_eq!(verts.len(), 4);
        assert_eq!(verts[3].position, [1.0, 1.0, 0.0]);
        assert_eq!(verts[3].uv, [1.0, 1.0]);
        assert_eq!(verts[3].lightmap_uv, Some([0.0, 0.0]));
        assert!((verts[0].normal[2] - 1.0).abs() < 1e-6 && verts[0].normal[0] == 0.0);
        assert_eq!(tris, [[0, 1, 2], [2, 1, 3]]);
    }

    #[test]
    fn restart_flag_splits_strips_like_the_game() {
        let mut out = Vec::new();
        // Game: flag at v=3 with start=0 -> emit [0,3), restart at 2.
        strip_to_triangles(0, 0, 3, &mut out);
        strip_to_triangles(0, 2, 5, &mut out);
        assert_eq!(out, [[0, 1, 2], [2, 3, 4]]);
    }

    #[test]
    fn rejects_garbage_version() {
        assert!(matches!(ModelHeader::parse(&[0u8; 0x80]), Err(ModelError::UnknownVersion(0))));
    }

    #[test]
    fn rejects_truncated_file() {
        assert!(matches!(ModelHeader::parse(&[0u8; 0x50]), Err(ModelError::Truncated(..))));
    }

    /// Every level's objects.ngc parses completely: every packet's DMA tag
    /// agrees with its descriptor, every decoded batch stays inside its
    /// packet, every submesh points at a real binding, and every textured
    /// binding points inside that level's textures.ngc.
    #[test]
    fn every_real_level_parses_completely() {
        let levels = levels();
        let (mut objects, mut submeshes, mut tris) = (0, 0, 0);
        for (name, file, textures) in &levels {
            let model = ModelFile::parse(file).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(model.header.num_objects, model.header.num_object_names, "{name}");
            for b in model.bindings.iter().filter(|b| b.is_textured()) {
                assert!((b.texture_offset as usize) < textures.len(), "{name}");
            }
            for o in &model.objects {
                assert!(!o.name.is_empty(), "{name}: unnamed object");
                for s in &o.submeshes {
                    for b in [s.descriptor.texture, s.descriptor.lightmap] {
                        assert!(
                            (b as usize) < model.bindings.len(),
                            "{name} {}: binding {b} of {}",
                            o.name,
                            model.bindings.len()
                        );
                    }
                    for v in &s.vertices {
                        assert!(v.position.iter().all(|c| c.is_finite() && c.abs() < 1.0e6), "{name}");
                    }
                    tris += s.triangles.len();
                }
                submeshes += o.submeshes.len();
            }
            objects += model.objects.len();
        }
        if !levels.is_empty() {
            eprintln!("{} levels: {objects} objects, {submeshes} submeshes, {tris} triangles", levels.len());
        }
    }
}
