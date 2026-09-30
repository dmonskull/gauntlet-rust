//! Lists a level's item types of a class with their armour, hit points
//! and value (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example itemtypes -- <game>/Gauntlet/LEVELS/levelA1 powerup
//! ```

use gdl_formats::population::Population;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: itemtypes <level folder> [class name]");
    let only = std::env::args().nth(2).map(|c| c.to_ascii_uppercase());
    let bytes = std::fs::read(std::path::Path::new(&dir).join("WORLDS.PS2")).expect("WORLDS.PS2");
    let pop = Population::parse(&bytes).expect("population");
    let mut seen = std::collections::BTreeSet::new();
    for p in &pop.placements {
        let ty = pop.resolved_type(p);
        let class = format!("{:?}", ty.class).to_ascii_uppercase();
        if only.as_ref().is_some_and(|c| !class.contains(c.as_str())) {
            continue;
        }
        if seen.insert((class.clone(), ty.subtype, ty.name.clone())) {
            println!(
                "{class:12} {:#04x} {:16} armour {:3} hp {:4} value {:#x} amount {} duration {} shape {} extent {:?}",
                ty.subtype,
                ty.name,
                ty.armor,
                ty.hit_points,
                ty.value,
                ty.amount,
                ty.duration,
                u16::from_le_bytes([ty.raw[8], ty.raw[9]]),
                ty.extent
            );
        }
    }
}
