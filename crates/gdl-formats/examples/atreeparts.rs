//! Lists a bank's atrees with their node, action and particle-system
//! counts (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example atreeparts -- <game>/Gauntlet/ITEMS/levelL [name prefix]
//! ```
fn main() {
    let dir = std::env::args().nth(1).expect("usage: atreeparts <bank folder> [prefix]");
    let prefix = std::env::args().nth(2).unwrap_or_default().to_ascii_uppercase();
    let bytes = std::fs::read(std::path::Path::new(&dir).join("ANIM.PS2")).expect("ANIM.PS2");
    let anim = gdl_formats::anim::AnimFile::parse(&bytes).expect("anim");
    for a in anim.atrees.iter().filter(|a| a.name.to_ascii_uppercase().starts_with(&prefix)) {
        let frames: Vec<u16> = a.actions.iter().map(|x| x.frames).collect();
        println!("{:16} nodes {:3} actions {:?} particles {}", a.name, a.nodes.len(), frames, a.particles.len());
    }
}
