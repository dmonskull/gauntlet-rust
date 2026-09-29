//! `PDATA/<class>.WAD` — per-class player data (loaded by the game's
//! player-data loader, `docs/chunk-files.md`). The `PDAT` chunk's single
//! record holds the four stats as `(start, max)` float pairs, the hero's
//! height and radius (`docs/items.md`: the item touch test uses them) and
//! the class's powerup duration factor. Their order is fixed by the classes themselves — the Dwarf
//! is strongest and slowest, the Wizard has the most magic, the Knight and
//! Valkyrie the most armour — and matches the game's stat names table
//! (STRENTH, ARMOR, MAGIC, SPEED) only up to order, so it's recorded here
//! explicitly.

use crate::chunk::{ChunkError, ChunkFile};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stat {
    pub start: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerStats {
    pub strength: Stat,
    pub speed: Stat,
    pub armor: Stat,
    pub magic: Stat,
    /// `+0x48`: the hero's height; the game keeps half of it as the
    /// vertical reach of the item touch test.
    pub height: f32,
    /// `+0x4C`: the hero's radius against items.
    pub radius: f32,
    /// `+0x58`: multiplies the duration of every timed powerup the hero
    /// picks up (1.0–1.3; the magic classes get the most).
    pub powerup_time: f32,
}

impl PlayerStats {
    pub fn parse(data: &[u8]) -> Result<Option<Self>, ChunkError> {
        let file = ChunkFile::parse(data)?;
        let Some(r) = file.bytes("PDAT").filter(|b| b.len() >= 0x5C) else { return Ok(None) };
        let f = |at: usize| f32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let stat = |at: usize| Stat { start: f(at), max: f(at + 4) };
        Ok(Some(Self {
            strength: stat(0x28),
            speed: stat(0x30),
            armor: stat(0x38),
            magic: stat(0x40),
            height: f(0x48),
            radius: f(0x4C),
            powerup_time: f(0x58),
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
            // Every hero is 5 tall with a 1.5 radius; powerups last up to
            // 30% longer for some classes.
            assert!(s.height > 1.0 && s.height < 20.0 && s.radius > 0.1 && s.radius < 5.0, "{p:?} {s:?}");
            assert!((1.0..=1.5).contains(&s.powerup_time), "{p:?} {s:?}");
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
