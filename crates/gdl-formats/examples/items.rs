//! Lists a level's placed items: class, type, model name, position and
//! rotation (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example items -- <game>/Gauntlet/LEVELS/levelL1 [class]
//! ```

use gdl_formats::population::Population;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: items <level folder> [class name]");
    let only = std::env::args().nth(2).map(|c| c.to_ascii_uppercase());
    let bytes = std::fs::read(std::path::Path::new(&dir).join("WORLDS.PS2")).expect("WORLDS.PS2");
    let pop = Population::parse(&bytes).expect("population");
    for (i, p) in pop.placements.iter().enumerate() {
        let ty = pop.resolved_type(p);
        let class = format!("{:?}", ty.class).to_ascii_uppercase();
        if only.as_ref().is_some_and(|c| !class.contains(c.as_str())) {
            continue;
        }
        let [x, y, z] = p.position;
        let [rx, ry, rz] = p.rotation;
        println!(
            "{i:4} {class:12} {:16} {:16} players {:2} flags {:#04x} pos ({x:8.2}, {y:7.2}, {z:8.2}) rot ({:6.1}°, {:6.1}°, {:6.1}°) params {:?}",
            ty.name,
            p.model_name(ty),
            p.players,
            p.flags,
            rx.to_degrees(),
            ry.to_degrees(),
            rz.to_degrees(),
            p.params(ty.class)
        );
    }
    for l in &pop.locators {
        let [x, y, z] = l.position;
        println!("locator {:?} param {} index {} pos ({x:8.2}, {y:7.2}, {z:8.2}) rot {:.2?}", l.kind, l.param, l.index, l.rotation);
    }
}
