//! `LEVELS/<level>/ANIM.PS2` — the level's texture modifiers: textures that
//! flip through frames (water, lava, fire, torches) or scroll (mist,
//! rapids, waterfalls). See `docs/rendering.md` "Texture animation".
//!
//! Header `{u32 0, u32 0, u32 count, u32 offset}`, then `count` 0x58-byte
//! records; little-endian.

use thiserror::Error;

const RECORD_LEN: usize = 0x58;

#[derive(Debug, Error)]
pub enum TexModError {
    #[error("file is truncated: needed bytes 0x{0:X}..0x{1:X}")]
    Truncated(usize, usize),
}

/// Where a flipbook's frames start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstFrame {
    /// A texture binding of the level's model file.
    Binding(u16),
    /// The binding whose texture has this name (looked up at load).
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TexModKind {
    /// Show textures `first`, `first + 1`, … `first + count − 1` in turn.
    Frames(FirstFrame),
    /// Scroll across U (−2) or V (−3) once every `|count|` steps, backwards
    /// when `count` is negative.
    ScrollU,
    ScrollV,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexMod {
    /// `+0x04`: the texture being changed.
    pub name: String,
    /// `+0x44`: its binding in the level's model file.
    pub binding: u16,
    pub kind: TexModKind,
    /// `+0x4C`: frames in the flipbook, or steps per scroll (signed).
    pub count: i16,
    /// `+0x4E`: scroll phase, in steps.
    pub phase: i16,
    /// `+0x50`: game ticks per step (0 and 1: every tick).
    pub period: u32,
    /// `+0x54`: the step the counter starts on.
    pub start: u32,
}

impl TexMod {
    /// Every modifier in an `ANIM.PS2`.
    pub fn parse_all(file: &[u8]) -> Result<Vec<Self>, TexModError> {
        let h = file.get(..16).ok_or(TexModError::Truncated(0, 16))?;
        let (count, offset) = (le_u32(h, 8) as usize, le_u32(h, 12) as usize);
        let end = offset + count * RECORD_LEN;
        let table = file.get(offset..end).ok_or(TexModError::Truncated(offset, end))?;
        Ok(table.as_chunks::<RECORD_LEN>().0.iter().map(|r| Self::parse_record(r)).collect())
    }

    fn parse_record(r: &[u8]) -> Self {
        let cstr = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned();
        let kind = match le_u32(r, 0x48) as i32 {
            -2 => TexModKind::ScrollU,
            -3 => TexModKind::ScrollV,
            -1 => TexModKind::Frames(FirstFrame::Named(cstr(&r[0x24..0x34]))),
            first => TexModKind::Frames(FirstFrame::Binding(first as u16)),
        };
        Self {
            name: cstr(&r[0x04..0x24]),
            binding: le_u32(r, 0x44) as u16,
            kind,
            count: i16::from_le_bytes([r[0x4C], r[0x4D]]),
            phase: i16::from_le_bytes([r[0x4E], r[0x4F]]),
            period: le_u32(r, 0x50),
            start: le_u32(r, 0x54),
        }
    }

    /// The flipbook frame shown after `ticks` game ticks.
    pub fn frame(&self, ticks: u64) -> u32 {
        let n = self.count.unsigned_abs().max(1) as u64;
        ((self.steps(ticks) + self.start as u64) % n) as u32
    }

    /// The scroll offset (in texture widths) after `ticks` game ticks; `ticks`
    /// may be fractional for smooth motion between ticks.
    pub fn scroll(&self, ticks: f64) -> f32 {
        let n = self.count.unsigned_abs().max(1) as f64;
        let steps = ticks / self.period.max(1) as f64 - self.phase as f64;
        let t = (steps / n).rem_euclid(1.0) as f32;
        if self.count < 0 { -t } else { t }
    }

    fn steps(&self, ticks: u64) -> u64 {
        ticks / self.period.max(1) as u64
    }
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_scrolls_advance_by_period() {
        let m = TexMod {
            name: "MOAT_WATER".into(),
            binding: 41,
            kind: TexModKind::Frames(FirstFrame::Binding(42)),
            count: 30,
            phase: 0,
            period: 2,
            start: 0,
        };
        assert_eq!(m.frame(0), 0);
        assert_eq!(m.frame(3), 1);
        assert_eq!(m.frame(60), 0);
        let s = TexMod { kind: TexModKind::ScrollU, count: -200, period: 0, ..m };
        assert!((s.scroll(50.0) + 0.25).abs() < 1e-6);
    }

    /// Every level's modifiers name bindings its model file has.
    #[test]
    fn every_real_level_texmod_points_at_a_binding() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(levels) = std::fs::read_dir(std::path::Path::new(&root).join("LEVELS")) else {
            eprintln!("skipping: no LEVELS folder");
            return;
        };
        let (mut files, mut mods) = (0, 0);
        for dir in levels.flatten().map(|e| e.path()) {
            let (Ok(data), Ok(objects)) = (std::fs::read(dir.join("ANIM.PS2")), std::fs::read(dir.join("objects.ngc"))) else {
                continue;
            };
            let model = crate::ModelFile::parse(&objects).unwrap();
            let list = TexMod::parse_all(&data).unwrap_or_else(|e| panic!("{dir:?}: {e}"));
            for m in &list {
                let n = model.bindings.len();
                assert!((m.binding as usize) < n, "{dir:?} {m:?}");
                if let TexModKind::Frames(FirstFrame::Binding(first)) = m.kind {
                    assert!(first as usize + m.count.unsigned_abs() as usize <= n, "{dir:?} {m:?}");
                }
                assert!(m.count != 0, "{dir:?} {m:?}");
            }
            files += 1;
            mods += list.len();
        }
        eprintln!("{files} level texture-modifier files, {mods} modifiers");
    }
}
