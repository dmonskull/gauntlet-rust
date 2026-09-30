//! Prints a level's particle-system records (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example psys -- <game>/Gauntlet/LEVELS/levelL1 [letter]
//! ```

use gdl_formats::psys::world_records;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: psys <level folder> [letter]");
    let only = std::env::args().nth(2).and_then(|l| l.bytes().next());
    let bytes = std::fs::read(std::path::Path::new(&dir).join("WORLDS.PS2")).expect("WORLDS.PS2");
    for r in world_records(&bytes).expect("records") {
        if only.is_some_and(|l| l != r.letter) {
            continue;
        }
        println!(
            "{} kind {:#x} preset {} flags {:#x}/{:#x} fields {:#x} texture {:?} emit {:?} life {:?} spread {:.1} dir {:?} accel {:?} speeds {:?} drag {:.3} colours {:08x?} sizes {:?}",
            r.letter as char,
            r.kind,
            r.preset,
            r.flag_values,
            r.flag_mask,
            r.fields,
            r.texture(),
            r.emit_times(),
            r.life(),
            r.spread_degrees(),
            r.direction(),
            r.acceleration(),
            r.speeds(),
            r.drag(),
            r.colour_keys(),
            r.size_keys()
        );
    }
}
