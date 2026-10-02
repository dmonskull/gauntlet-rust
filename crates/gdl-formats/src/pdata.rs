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
//! - The class's powerup duration factor (`docs/items.md`).

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
    /// `+0x58`: multiplies the duration of every timed powerup the hero
    /// picks up (1.0–1.3; the magic classes get the most).
    pub powerup_time: f32,
    /// `+0x5C`: where a thrown weapon leaves the hand, in the hero's own
    /// frame (X across, Y up, Z forward) from its collision centre.
    pub throw_offset: [f32; 3],
    /// `+0x158`: the same for the power throw.
    pub power_throw_offset: [f32; 3],
}

impl PlayerStats {
    pub fn parse(data: &[u8]) -> Result<Option<Self>, ChunkError> {
        let file = ChunkFile::parse(data)?;
        let Some(r) = file.bytes("PDAT").filter(|b| b.len() >= 0x5C) else { return Ok(None) };
        let f = |at: usize| f32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let stat = |at: usize| Stat { start: f(at), max: f(at + 4) };
        let vec3 = |at: usize| if r.len() >= at + 12 { [f(at), f(at + 4), f(at + 8)] } else { [0.0; 3] };
        Ok(Some(Self {
            strength: stat(0x28),
            speed: stat(0x30),
            armor: stat(0x38),
            magic: stat(0x40),
            body: PlayerBody { height: f(0x48), radius: f(0x4C), head_height: f(0x50), centre_height: f(0x54) },
            powerup_time: f(0x58),
            throw_offset: vec3(0x5C),
            power_throw_offset: vec3(0x158),
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
            assert!((1.0..=1.5).contains(&s.powerup_time), "{p:?} {s:?}");
            // Throws leave within a couple of units of the centre.
            for v in [s.throw_offset, s.power_throw_offset] {
                assert!(v.iter().all(|c| c.abs() <= 2.0), "{p:?} {v:?}");
            }
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
        assert_eq!(all["ARC"].throw_offset, [0.0, 1.5, 0.0]);
        assert_eq!(all["WAR"].power_throw_offset, [0.0, 0.5, 1.5]);
    }
}

/// A hero's attack tables. These are distinct from the critter tables:
/// hero damage records are 88 bytes, and SFXX has two model selectors
/// before its position and a packed colour at the end. See `docs/chunk-files.md`.
#[derive(Debug, Clone)]
pub struct HeroAttacks {
    /// Close/low/medium power, 360, power throw, turbo B, both turbo C
    /// records, combo 1, combo 3, the auxiliary action and victory.
    pub indices: [i16; 12],
    pub damage: Vec<HeroDamage>,
    pub effects: Vec<HeroEffect>,
}

#[derive(Debug, Clone)]
pub struct HeroDamage {
    pub kind: i16,
    pub flags: u16,
    pub blow: u32,
    pub size: f32,
    pub radius: f32,
    pub unused: f32,
    /// A blast's lifetime override; kind 10 uses it as shot spacing.
    pub duration: f32,
    /// A missile's lifetime override (zero uses the effect's duration).
    pub missile_life: f32,
    pub trail_scale: f32,
    pub yaw: f32,
    pub cone: f32,
    pub pitch: f32,
    pub offset: [f32; 3],
    /// Negative values multiply the hero's strength by their absolute value.
    pub damage: f32,
    pub speed: [f32; 2],
    pub gravity: f32,
    /// Primary effect, hit effect, trail effect.
    pub effects: [i16; 3],
    pub next: i16,
    pub start: i16,
    pub end: i16,
    pub hint: i16,
}

#[derive(Debug, Clone)]
pub struct HeroEffect {
    pub flags: u32,
    pub next: i32,
    pub effect: String,
    pub sound: String,
    pub model_selectors: [i16; 2],
    pub offset: [f32; 3],
    /// A model effect's lifetime; flags select other interpretations.
    pub duration: f32,
    pub parameters: [f32; 2],
    /// Packed colour copied to the model, including its alpha.
    pub color: u32,
}

impl HeroAttacks {
    pub fn parse(data: &[u8]) -> Result<Option<Self>, ChunkError> {
        let file = ChunkFile::parse(data)?;
        let Some(pdat) = file.bytes("PDAT") else { return Ok(None) };
        if pdat.len() < 0x24 { return Err(ChunkError::Truncated(0, 0x24)); }
        let short = |r: &[u8], at: usize| i16::from_le_bytes(r[at..at + 2].try_into().unwrap());
        let uint = |r: &[u8], at: usize| u32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let float = |r: &[u8], at: usize| f32::from_le_bytes(r[at..at + 4].try_into().unwrap());
        let vec3 = |r: &[u8], at: usize| std::array::from_fn(|i| float(r, at + 4 * i));
        let name = |r: &[u8], at: usize| {
            let bytes = &r[at..at + 16];
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(16);
            if bytes.first().is_some_and(|&b| b < b' ') { String::new() }
            else { String::from_utf8_lossy(&bytes[..end]).into_owned() }
        };
        let table = |tag: &str, size: usize| -> Result<Vec<&[u8]>, ChunkError> {
            let Some(c) = file.get(tag) else { return Ok(Vec::new()) };
            let needed = c.count as usize * size;
            let bytes = file.bytes(tag).unwrap_or_default();
            if bytes.len() < needed {
                return Err(ChunkError::Truncated(c.offset as usize, c.offset as usize + needed));
            }
            Ok(bytes[..needed].chunks_exact(size).collect())
        };
        let damage = table("DAMG", 0x58)?.into_iter().map(|r| HeroDamage {
            kind: short(r, 0), flags: short(r, 2) as u16, blow: uint(r, 4),
            size: float(r, 8), radius: float(r, 0x0C), unused: float(r, 0x10),
            duration: float(r, 0x14), missile_life: float(r, 0x18), trail_scale: float(r, 0x1C),
            yaw: float(r, 0x20), cone: float(r, 0x24), pitch: float(r, 0x28), offset: vec3(r, 0x2C),
            damage: float(r, 0x38), speed: [float(r, 0x3C), float(r, 0x40)], gravity: float(r, 0x44),
            effects: std::array::from_fn(|i| short(r, 0x48 + 2 * i)), next: short(r, 0x4E),
            start: short(r, 0x50), end: short(r, 0x52), hint: short(r, 0x54),
        }).collect();
        let effects = table("SFXX", 0x50)?.into_iter().map(|r| HeroEffect {
            flags: uint(r, 0), next: uint(r, 4) as i32, effect: name(r, 0x10), sound: name(r, 0x20),
            model_selectors: [short(r, 0x30), short(r, 0x32)], offset: vec3(r, 0x34), duration: float(r, 0x40),
            parameters: [float(r, 0x44), float(r, 0x48)], color: uint(r, 0x4C),
        }).collect();
        Ok(Some(Self { indices: std::array::from_fn(|i| short(pdat, 0x0C + 2 * i)), damage, effects }))
    }
}

#[cfg(test)]
mod attack_tests {
    use super::*;
    #[test]
    fn attack_tables_from_every_real_class_have_valid_links() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT").unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(entries) = std::fs::read_dir(std::path::Path::new(&root).join("PDATA")) else { return };
        let mut count = 0;
        for path in entries.flatten().map(|e| e.path()) {
            let a = HeroAttacks::parse(&std::fs::read(&path).unwrap()).unwrap().unwrap();
            let valid = |index: i16, n: usize| index == -1 || (index >= 0 && (index as usize) < n);
            assert!(a.indices.iter().all(|&i| valid(i, a.damage.len())), "{path:?}");
            for r in &a.damage {
                assert!(valid(r.next, a.damage.len()), "{path:?}");
                assert!(r.effects.iter().all(|&i| valid(i, a.effects.len())), "{path:?}");
                assert!(r.damage.is_finite() && r.radius >= 0.0 && r.size >= 0.0);
            }
            assert!(a.effects.iter().all(|e| e.next == -1 || (e.next >= 0 && (e.next as usize) < a.effects.len())));
            if path.file_stem().unwrap() == "VAL" {
                let combo = &a.damage[a.indices[8] as usize];
                assert_eq!((combo.kind, combo.radius, combo.damage, combo.start), (4, 15.0, 100.0, 1));
                assert_eq!(a.effects[combo.effects[0] as usize].color, 0x7fff_ffff);
            }
            count += 1;
        }
        assert_eq!(count, 16);
    }
}
