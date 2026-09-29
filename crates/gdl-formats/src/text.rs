//! `TEXT/*.ROM` — the game's text, as named groups of strings.
//!
//! A chunk file (`chunk.rs`): `TEXT` is a pool of NUL-terminated strings,
//! `TOFF` their offsets; `STRS` has one 20-byte record per named group
//! `{u32 count, u32 first string, u32 font, f32 scale x, f32 scale y}`, and
//! `DEFS`/`SDEF` are the group names (pool + offsets), e.g. `PLAYER_CLASS`
//! → WARRIOR, VALKYRIE, …; `FONT` lists the fonts the groups' font numbers
//! index (20-byte records: `name[16]`, then a word the loader fills with the
//! font slot, `font.rs`). `LIST` has 8-byte records `{u32 count, u32 first}`
//! into `LOFF`, a table of group indices, and `LDEF` holds each list's name
//! (offsets into the `DEFS` pool): `CLASS_RANK` → `WAR_RANK`, `VAL_RANK`…
//! See `docs/chunk-files.md`.

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

/// A named list of groups (`LIST`/`LOFF`/`LDEF`).
#[derive(Debug, Clone)]
pub struct TextList {
    pub name: String,
    /// Indices into [`TextRom::groups`].
    pub groups: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct TextRom {
    pub groups: Vec<TextGroup>,
    /// Font names from the `FONT` chunk; a group's `font` indexes this.
    pub fonts: Vec<String>,
    pub lists: Vec<TextList>,
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

        let fonts = match file.records("FONT", 20) {
            Some(records) => records
                .map(|r| {
                    let end = r[..16].iter().position(|&b| b == 0).unwrap_or(16);
                    String::from_utf8_lossy(&r[..end]).into_owned()
                })
                .collect(),
            None => Vec::new(),
        };

        let mut lists = Vec::new();
        if let (Some(records), Some(members), Some(list_names)) =
            (file.records("LIST", 8), file.bytes("LOFF"), file.bytes("LDEF"))
        {
            for (i, r) in records.enumerate() {
                let (count, first) = (le_u32(r, 0) as usize, le_u32(r, 4) as usize);
                let name_at = list_names.get(i * 4..i * 4 + 4).ok_or(TextError::Bad("LDEF"))?;
                let groups = (first..first + count)
                    .map(|k| {
                        let at = members.get(k * 4..k * 4 + 4).ok_or(TextError::Bad("LIST"))?;
                        let group = le_u32(at, 0) as usize;
                        if group < out.len() { Ok(group) } else { Err(TextError::Bad("LOFF")) }
                    })
                    .collect::<Result<_, _>>()?;
                lists.push(TextList { name: cstr(names, le_u32(name_at, 0) as usize, "LDEF")?, groups });
            }
        }
        Ok(Self { groups: out, fonts, lists })
    }

    pub fn list(&self, name: &str) -> Option<&TextList> {
        self.lists.iter().find(|l| l.name == name)
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
                assert_eq!(rom.fonts, ["8Hi_fonts5", "font32", "initials"]);
                let names: Vec<&str> = rom.lists.iter().map(|l| l.name.as_str()).collect();
                assert_eq!(names, ["CLASS_RANK", "CLASS_TURBO", "LEGEND_ITEMS", "CONTROLS_DESC"]);
                let ranks = rom.list("CLASS_RANK").unwrap();
                assert_eq!(ranks.groups.len(), 16);
                assert_eq!(rom.groups[ranks.groups[0]].name, "WAR_RANK");
                assert_eq!(rom.groups[ranks.groups[15]].name, "HYE_RANK");
                assert_eq!(rom.group("GAME_OVER").map(|g| g.strings[0].as_str()), Some("GAME OVER"));
            }
            for l in &rom.lists {
                assert!(!l.groups.is_empty(), "{p:?}: list {} is empty", l.name);
            }
        }
        eprintln!("{total} strings");
    }
}
