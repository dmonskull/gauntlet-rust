//! `CRITTER/<boss>.WAD` — the bosses' behaviour data. "Critter" is the
//! game's name for its boss/scripted-monster system (golem, dragon, lich,
//! gargoyles, …); regular monsters don't have one of these files — their
//! stats are compiled into the game (`enemy.rs`).
//!
//! A chunk file (`chunk.rs`) whose eight chunks the game's critter loader
//! looks up by tag and byte-swaps field by field, which fixes each record's
//! size and field widths. Only the `TYPE` record's hit points and its ranges
//! into the other tables are named so far; the rest is kept raw.
//! `docs/monsters.md` has the loader and what's known.

use thiserror::Error;

use crate::chunk::{ChunkError, ChunkFile};

/// Record sizes, by tag, as the loader walks them.
pub const RECORD_SIZES: [(&str, usize); 8] = [
    ("SFXX", 0x50),
    ("DAMG", 0x50),
    ("MOVE", 0x90),
    ("PTRN", 0x50),
    ("NODE", 0x50),
    ("DESC", 0x30),
    ("TYPE", 0x140),
    ("ADDA", 0x30),
];

#[derive(Debug, Error)]
pub enum CritterError {
    #[error(transparent)]
    Chunk(#[from] ChunkError),
    #[error("chunk {0} is missing")]
    MissingChunk(&'static str),
    #[error("chunk {0} is truncated")]
    Truncated(&'static str),
    #[error("the file has no critter types")]
    NoTypes,
}

/// A run of records in one of the other tables: `count` from `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub first: i16,
    pub count: i16,
}

impl Span {
    pub fn range(&self) -> std::ops::Range<usize> {
        let first = self.first.max(0) as usize;
        first..first + self.count.max(0) as usize
    }
}

/// One `TYPE` record (`0x140` bytes): a boss, or one part of a boss made
/// of several (the chimera's heads), chained through `child`.
#[derive(Debug, Clone)]
pub struct CritterType {
    /// `+0xE4`: hit points before the level's hit point scale.
    pub hit_points: f32,
    /// `+0x110`: this type's `MOVE` records.
    pub moves: Span,
    /// `+0x114`: its `PTRN` records.
    pub patterns: Span,
    /// `+0x118`: its `NODE` records.
    pub nodes: Span,
    /// `+0x11C`: the next part, spawned with this one.
    pub child: Option<usize>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CritterFile {
    pub types: Vec<CritterType>,
    /// Record count per tag, in [`RECORD_SIZES`] order.
    pub counts: [usize; 8],
    /// Every table's records, raw, in [`RECORD_SIZES`] order.
    pub tables: [Vec<Vec<u8>>; 8],
}

impl CritterFile {
    pub fn parse(data: &[u8]) -> Result<Self, CritterError> {
        let file = ChunkFile::parse(data)?;
        let mut tables: [Vec<Vec<u8>>; 8] = Default::default();
        for (i, (tag, size)) in RECORD_SIZES.iter().enumerate() {
            let chunk = file.get(tag).ok_or(CritterError::MissingChunk(tag))?;
            if chunk.count == 0 {
                continue;
            }
            let records = file.records(tag, *size).ok_or(CritterError::Truncated(tag))?;
            tables[i] = records.map(<[u8]>::to_vec).collect();
        }
        let types: Vec<CritterType> = tables[6]
            .iter()
            .map(|r| {
                let i16_at = |at: usize| i16::from_le_bytes([r[at], r[at + 1]]);
                let span = |at: usize| Span { count: i16_at(at), first: i16_at(at + 2) };
                CritterType {
                    hit_points: f32::from_le_bytes(r[0xE4..0xE8].try_into().unwrap()),
                    moves: span(0x110),
                    patterns: span(0x114),
                    nodes: span(0x118),
                    child: usize::try_from(i16_at(0x11C)).ok(),
                    raw: r.clone(),
                }
            })
            .collect();
        if types.is_empty() {
            return Err(CritterError::NoTypes);
        }
        let counts = std::array::from_fn(|i| tables[i].len());
        Ok(Self { types, counts, tables })
    }

    pub fn count(&self, tag: &str) -> usize {
        RECORD_SIZES.iter().position(|(t, _)| *t == tag).map_or(0, |i| self.counts[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every critter file parses, and its types' spans tile the `MOVE`,
    /// `PTRN` and `NODE` tables exactly, in order, with children chained
    /// forward.
    #[test]
    fn every_real_critter_file_parses() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(entries) = std::fs::read_dir(std::path::Path::new(&root).join("CRITTER")) else {
            eprintln!("skipping: no CRITTER folder");
            return;
        };
        let mut files = 0;
        for p in entries.flatten().map(|e| e.path()) {
            let c = CritterFile::parse(&std::fs::read(&p).unwrap()).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            assert_eq!(c.count("DESC"), 1, "{p:?}");
            for (tag, pick) in [
                ("MOVE", (|t: &CritterType| t.moves) as fn(&CritterType) -> Span),
                ("PTRN", |t| t.patterns),
                ("NODE", |t| t.nodes),
            ] {
                let mut next = 0;
                for t in &c.types {
                    let s = pick(t);
                    if s.count > 0 {
                        assert_eq!(s.first as usize, next, "{p:?} {tag}");
                    }
                    next += s.count.max(0) as usize;
                }
                assert_eq!(next, c.count(tag), "{p:?} {tag}");
            }
            for (i, t) in c.types.iter().enumerate() {
                assert!(t.hit_points >= 100.0 && t.hit_points <= 10_000.0, "{p:?} {}", t.hit_points);
                if let Some(child) = t.child {
                    assert!(child > i && child < c.types.len(), "{p:?}");
                }
            }
            files += 1;
        }
        eprintln!("{files} critter files");
        assert!(files >= 18);
    }
}
