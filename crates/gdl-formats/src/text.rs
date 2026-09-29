//! `TEXT/*.ROM` — the game's text, as named groups of strings.
//!
//! A chunk file (`chunk.rs`): `TEXT` is a pool of NUL-terminated strings,
//! `TOFF` their offsets; `STRS` has one 20-byte record per named group
//! `{u32 count, u32 first string, u32 font, f32 scale x, f32 scale y}`, and
//! `DEFS`/`SDEF` are the group names (pool + offsets), e.g. `PLAYER_CLASS`
//! → WARRIOR, VALKYRIE, …; `FONT` lists the fonts. See
//! `docs/chunk-files.md`.

use thiserror::Error;

use crate::chunk::{ChunkError, ChunkFile, le_u32};

#[derive(Debug, Error)]
pub enum TextError {
    #[error(transparent)]
    Chunk(#[from] ChunkError),
    #[error("missing {0} chunk")]
    Missing(&'static str),
    #[error("{0} points outside its pool")]
    Bad(&'static str),
}

#[derive(Debug, Clone)]
pub struct TextGroup {
    pub name: String,
    pub strings: Vec<String>,
    pub font: u32,
    pub scale: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct TextRom {
    pub groups: Vec<TextGroup>,
}

impl TextRom {
    pub fn parse(data: &[u8]) -> Result<Self, TextError> {
        let file = ChunkFile::parse(data)?;
        let chunk = |tag: &'static str| file.bytes(tag).ok_or(TextError::Missing(tag));
        let pool = chunk("TEXT")?;
        let offsets = chunk("TOFF")?;
        let names = chunk("DEFS")?;
        let name_offsets = chunk("SDEF")?;
        let groups = file.records("STRS", 20).ok_or(TextError::Missing("STRS"))?;

        let cstr = |pool: &[u8], at: usize, what: &'static str| -> Result<String, TextError> {
            let rest = pool.get(at..).ok_or(TextError::Bad(what))?;
            let end = rest.iter().position(|&b| b == 0).ok_or(TextError::Bad(what))?;
            Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
        };
        let string = |i: usize| -> Result<String, TextError> {
            let at = offsets.get(i * 4..i * 4 + 4).ok_or(TextError::Bad("STRS"))?;
            cstr(pool, le_u32(at, 0) as usize, "TOFF")
        };

        let mut out = Vec::new();
        for (i, r) in groups.enumerate() {
            let (count, first) = (le_u32(r, 0) as usize, le_u32(r, 4) as usize);
            let name_at = name_offsets.get(i * 4..i * 4 + 4).ok_or(TextError::Bad("SDEF"))?;
            out.push(TextGroup {
                name: cstr(names, le_u32(name_at, 0) as usize, "SDEF")?,
                strings: (first..first + count).map(string).collect::<Result<_, _>>()?,
                font: le_u32(r, 8),
                scale: [f32::from_bits(le_u32(r, 12)), f32::from_bits(le_u32(r, 16))],
            });
        }
        Ok(Self { groups: out })
    }

    pub fn group(&self, name: &str) -> Option<&TextGroup> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// The `index`-th string of group `name`.
    pub fn get(&self, name: &str, index: usize) -> Option<&str> {
        self.group(name)?.strings.get(index).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_real_text_rom_parses() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("TEXT");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let mut total = 0;
        for p in entries.flatten().map(|e| e.path()) {
            let rom = TextRom::parse(&std::fs::read(&p).unwrap()).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            total += rom.groups.iter().map(|g| g.strings.len()).sum::<usize>();
            if p.file_name().is_some_and(|n| n == "ENGLISH.ROM") {
                assert_eq!(rom.get("PLAYER_CLASS", 3), Some("ARCHER"));
                assert_eq!(rom.get("PLAYER_COLOR", 1), Some("BLUE"));
            }
        }
        eprintln!("{total} strings");
    }
}
