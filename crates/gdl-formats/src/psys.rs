//! Particle-system records (`docs/rendering.md`, "Particle-system nodes"):
//! `0x138`-byte parameter blocks the game's particle library applies to an
//! emitter. A level's `WORLDS.PS2` lists them at header words 28/29 (count,
//! offset); a world node named `…PSYS<letter>…` runs the record whose letter
//! matches. Model `ANIM` files carry the same records.
//!
//! Each record says which fields it sets (the bit set at word 4); the rest
//! keep the preset's (or the library's) defaults. Times are seconds, turned
//! into 30 Hz frames by the game.

use crate::world::WorldError;

/// Record size.
pub const RECORD_LEN: usize = 0x138;
/// World header words holding the records' count and offset.
const COUNT_WORD: usize = 28;
const OFFSET_WORD: usize = 29;

/// Field-present bits (word 4).
pub mod fields {
    pub const PRESET: u32 = 0x1;
    pub const COUNT_A: u32 = 0x2;
    pub const COUNT_C: u32 = 0x4;
    pub const COUNT_B: u32 = 0x8;
    pub const EMIT_TIMES: u32 = 0x10;
    pub const LIFE: u32 = 0x20;
    pub const SPREAD: u32 = 0x40;
    pub const DIRECTION: u32 = 0x80;
    pub const ACCEL: u32 = 0x100;
    pub const SPEEDS: u32 = 0x200;
    pub const DRAG: u32 = 0x400;
    pub const SPIN: u32 = 0x800;
    pub const SPIN_B: u32 = 0x1000;
    pub const SPEED_C: u32 = 0x2000;
    pub const TEXTURE: u32 = 0x4000;
    pub const WORD15: u32 = 0x8000;
    pub const COLOURS: u32 = 0x10000;
    pub const ALPHAS: u32 = 0x20000;
    pub const SIZES: u32 = 0x40000;
    pub const DURATION: u32 = 0x80000;
}

/// One particle-system record, as stored (little-endian on the disc).
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleRecord {
    /// Word 0: at least `0x100` for a valid record.
    pub kind: u32,
    /// `+0x04`: the built-in preset applied first (field bit 1).
    pub preset: i16,
    /// `+0x06`: the letter a world node's `PSYS<letter>` name selects.
    pub letter: u8,
    /// Words 2 and 3: flag values and which of them the record sets.
    pub flag_values: u32,
    pub flag_mask: u32,
    /// Word 4: which fields follow ([`fields`]).
    pub fields: u32,
    /// All 78 words, for fields not named yet.
    pub words: [u32; RECORD_LEN / 4],
}

impl ParticleRecord {
    pub fn parse(r: &[u8]) -> Option<Self> {
        let r = r.get(..RECORD_LEN)?;
        let words: [u32; RECORD_LEN / 4] = std::array::from_fn(|i| u32::from_le_bytes(r[i * 4..i * 4 + 4].try_into().unwrap()));
        Some(Self {
            kind: words[0],
            preset: i16::from_le_bytes([r[4], r[5]]),
            letter: r[6],
            flag_values: words[2],
            flag_mask: words[3],
            fields: words[4],
            words,
        })
    }

    fn f(&self, word: usize) -> f32 {
        f32::from_bits(self.words[word])
    }

    pub fn has(&self, bit: u32) -> bool {
        self.fields & bit != 0
    }

    /// Words 8/9: the emitter's two times (seconds; negative: none).
    pub fn emit_times(&self) -> [f32; 2] {
        [self.f(8), self.f(9)]
    }

    /// Words 10/11: a particle's shortest and longest life, seconds.
    pub fn life(&self) -> [f32; 2] {
        [self.f(10), self.f(11)]
    }

    /// Word 14: the spray's half-angle, degrees (359 or more: all round).
    pub fn spread_degrees(&self) -> f32 {
        self.f(14)
    }

    /// Words 0x10–0x17: the particles' texture name.
    pub fn texture(&self) -> String {
        let bytes: Vec<u8> = self.words[0x10..0x18].iter().flat_map(|w| w.to_le_bytes()).collect();
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }

    /// Words 0x18–0x1A: the emitting direction.
    pub fn direction(&self) -> [f32; 3] {
        [self.f(0x18), self.f(0x19), self.f(0x1A)]
    }

    /// Words 0x1B–0x1D: the particles' acceleration.
    pub fn acceleration(&self) -> [f32; 3] {
        [self.f(0x1B), self.f(0x1C), self.f(0x1D)]
    }

    /// Words 0x1E–0x21: four speeds (units a second; the game stores them
    /// per frame).
    pub fn speeds(&self) -> [f32; 4] {
        [self.f(0x1E), self.f(0x1F), self.f(0x20), self.f(0x21)]
    }

    /// Word 0x22: drag, per cent.
    pub fn drag(&self) -> f32 {
        self.f(0x22)
    }

    /// Words 0x26–0x29: four colour keys, `0xAARRGGBB`-like (alpha in the
    /// top byte, set by field bit `0x20000`).
    pub fn colour_keys(&self) -> [u32; 4] {
        [self.words[0x26], self.words[0x27], self.words[0x28], self.words[0x29]]
    }

    /// Words 0x2A–0x2D: four size keys.
    pub fn size_keys(&self) -> [f32; 4] {
        [self.f(0x2A), self.f(0x2B), self.f(0x2C), self.f(0x2D)]
    }
}

/// The particle records of a `WORLDS.PS2`.
pub fn world_records(file: &[u8]) -> Result<Vec<ParticleRecord>, WorldError> {
    let word = |i: usize| -> Option<usize> {
        file.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
    };
    let (Some(count), Some(offset)) = (word(COUNT_WORD), word(OFFSET_WORD)) else { return Ok(Vec::new()) };
    if count == 0 {
        return Ok(Vec::new());
    }
    let end = count.checked_mul(RECORD_LEN).and_then(|n| n.checked_add(offset)).unwrap_or(usize::MAX);
    if end > file.len() {
        return Err(WorldError::Truncated(offset, end));
    }
    Ok((0..count).filter_map(|i| ParticleRecord::parse(&file[offset + i * RECORD_LEN..])).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_has_its_particle_records() {
        let root = std::path::PathBuf::from(
            std::env::var("GAUNTLET_ASSET_ROOT")
                .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into()),
        );
        let Ok(dirs) = std::fs::read_dir(root.join("LEVELS")) else { return };
        let mut total = 0;
        for d in dirs.flatten() {
            let Ok(file) = std::fs::read(d.path().join("WORLDS.PS2")) else { continue };
            let records = world_records(&file).unwrap_or_else(|e| panic!("{:?}: {e}", d.path()));
            for r in &records {
                assert!(r.kind >= 0x100, "{:?}: kind {:#x}", d.path(), r.kind);
                assert!(r.letter.is_ascii_alphanumeric(), "{:?}: letter {}", d.path(), r.letter);
            }
            total += records.len();
        }
        assert!(total > 50, "{total} records");
    }
}
