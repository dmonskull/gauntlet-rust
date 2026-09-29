//! `PDATA/<class>.WAD` — per-class player data (loaded by the game's
//! player-data loader, `docs/chunk-files.md`). Decoded so far from the
//! `PDAT` chunk's single record:
//!
//! - The four stats, `(start, max)` float pairs. Their order is fixed by the
//!   classes themselves — the Dwarf is strongest and slowest, the Wizard has
//!   the most magic, the Knight and Valkyrie the most armour — and matches
//!   the game's stat names table (STRENTH, ARMOR, MAGIC, SPEED) only up to
//!   order, so it's recorded here explicitly.
//! - The body measurements the player setup copies out when a player joins
//!   (`docs/chunk-files.md`, `docs/collision.md` "Moving a player").

use crate::chunk::{ChunkError, ChunkFile};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stat {
    pub start: f32,
    pub max: f32,
}

/// A class's body measurements (`PDAT +0x48..+0x54`). Every class on the
/// disc has the same values (5, 1.5, 4.4, 2.5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerBody {
    /// `+0x48`: height; the game keeps half of it as the player's
    /// collision half-height (the vertical reach of player-vs-actor tests).
    pub height: f32,
    /// `+0x4C`: collision radius, for walls, floors and other actors.
    pub radius: f32,
    /// `+0x50`: height of the player's top point above the feet.
    pub head_height: f32,
    /// `+0x54`: height of the collision centre above the feet — the point
    /// the player's wall and floor tests start from.
    pub centre_height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerStats {
    pub strength: Stat,
    pub speed: Stat,
    pub armor: Stat,
    pub magic: Stat,
    pub body: PlayerBody,
}

impl PlayerStats {
    pub fn parse(data: &[u8]) -> Result<Option<Self>, ChunkError> {
        let file = ChunkFile::parse(data)?;
        let Some(r) = file.bytes("PDAT").filter(|b| b.len() >= 0x58) else { return Ok(None) };
        let f = |at: usize| f32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let stat = |at: usize| Stat { start: f(at), max: f(at + 4) };
        Ok(Some(Self {
            strength: stat(0x28),
            speed: stat(0x30),
            armor: stat(0x38),
            magic: stat(0x40),
            body: PlayerBody { height: f(0x48), radius: f(0x4C), head_height: f(0x50), centre_height: f(0x54) },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_archetypes_fix_the_stat_order() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let dir = std::path::Path::new(&root).join("PDATA");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: {dir:?} not present");
            return;
        };
        let mut all = std::collections::BTreeMap::new();
        for p in entries.flatten().map(|e| e.path()) {
            let s = PlayerStats::parse(&std::fs::read(&p).unwrap()).unwrap().unwrap();
            for st in [s.strength, s.speed, s.armor, s.magic] {
                assert!(st.start > 0.0 && st.start <= st.max && st.max <= 999.0, "{p:?} {st:?}");
            }
            // Every class has the same body (the collision code's defaults).
            let b = s.body;
            assert_eq!((b.height, b.radius, b.head_height, b.centre_height), (5.0, 1.5, 4.4, 2.5), "{p:?}");
            all.insert(p.file_stem().unwrap().to_string_lossy().into_owned(), s);
        }
        // The eight original classes (the unlockable Minotaur and Ogre are
        // stronger still).
        let base = ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"];
        let top = |f: fn(&PlayerStats) -> f32| {
            let best = base.iter().map(|c| f(&all[*c])).fold(f32::MIN, f32::max);
            base.iter().filter(|c| f(&all[**c]) == best).copied().collect::<Vec<_>>()
        };
        assert!(top(|s| s.strength.start).contains(&"DWF"));
        assert!(top(|s| -s.speed.start).contains(&"DWF"));
        assert!(top(|s| s.magic.start).contains(&"WIZ"));
        assert!(top(|s| s.armor.start).contains(&"KNI") && top(|s| s.armor.start).contains(&"VAL"));
    }
}
