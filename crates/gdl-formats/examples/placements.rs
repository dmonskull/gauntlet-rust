//! Counts each level's placed monsters by (monster, level, AI) (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example placements -- <game>/Gauntlet/LEVELS [ai]
//! ```
//!
//! With an AI number (`0x12` or `18`), only the levels placing it.

use std::collections::BTreeMap;

use gdl_formats::population::{ItemClass, PlacementParams, Population};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: placements <LEVELS folder> [ai]");
    let only = std::env::args().nth(2).map(|a| match a.strip_prefix("0x") {
        Some(hex) => i16::from_str_radix(hex, 16).expect("AI"),
        None => a.parse().expect("AI"),
    });
    let mut levels: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    levels.sort();
    for level in levels {
        let Ok(bytes) = std::fs::read(level.join("WORLDS.PS2")) else { continue };
        let Ok(population) = Population::parse(&bytes) else { continue };
        let mut counts: BTreeMap<(String, i16, i16), usize> = BTreeMap::new();
        for p in &population.placements {
            let Some(ty) = population.item_types.get(p.item_type) else { continue };
            if ty.class != ItemClass::EnemyInfo {
                continue;
            }
            if let PlacementParams::Enemy { level, ai, .. } = p.params(ty.class)
                && only.is_none_or(|a| a == ai)
            {
                *counts.entry((ty.name.clone(), level, ai)).or_default() += 1;
            }
        }
        if counts.is_empty() {
            continue;
        }
        let list: Vec<_> = counts.iter().map(|((name, lv, ai), n)| format!("{n}×{name} lv{lv} ai{ai:#x}")).collect();
        println!("{}: {}", level.file_name().unwrap().to_string_lossy(), list.join(", "));
    }
}
