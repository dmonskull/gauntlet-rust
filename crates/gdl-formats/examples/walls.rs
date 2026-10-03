//! Casts the game's wall test between two points of a level and prints
//! what it hits (dev tool: what stands between a monster and a hero).
//!
//! ```text
//! cargo run -p gdl-formats --example walls -- <game>/Gauntlet/LEVELS/levelA1 x0 y0 z0 x1 y1 z1 [radius]
//! ```
use gdl_formats::collision::LevelCollision;

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: walls <level folder> <x0> <y0> <z0> <x1> <y1> <z1> [radius]";
    let dir = args.next().expect(usage);
    let n: Vec<f32> = args.map(|s| s.parse().expect("a number")).collect();
    assert!(n.len() >= 6, "{usage}");
    let radius = n.get(6).copied().unwrap_or(1.5);
    let bytes = std::fs::read(std::path::Path::new(&dir).join("WORLDS.PS2")).expect("WORLDS.PS2");
    let world = gdl_formats::WorldFile::parse(&bytes).expect("world");
    let collision = LevelCollision::new(&world).expect("collision");
    let (from, to) = ([n[0], n[1], n[2]], [n[3], n[4], n[5]]);
    match collision.wall(from, to, radius) {
        Some(h) => {
            let node = &collision.nodes[h.node];
            println!(
                "hit node {} {} flags {:#010x} disable {} at {:?} normal {:?}",
                h.node, world.nodes[h.node].name, node.flags, node.disable, h.point, h.normal
            );
        }
        None => println!("nothing between {from:?} and {to:?} (radius {radius})"),
    }
}
