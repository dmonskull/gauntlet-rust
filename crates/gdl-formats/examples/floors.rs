//! Lists every floor down a vertical line through a level, as all queries
//! and as the player's floor check sees them (dev tool: pick `GDL_WARP`
//! points a hero stands on).
//!
//! ```text
//! cargo run -p gdl-formats --example floors -- <game>/Gauntlet/LEVELS/levelA1 112.75 81 [radius]
//! ```
use gdl_formats::collision::{LevelCollision, Query};

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: floors <level folder> <x> <z> [radius]";
    let dir = args.next().expect(usage);
    let mut num = |what: &str| args.next().map(|s| s.parse::<f32>().unwrap_or_else(|_| panic!("{what}: not a number")));
    let (x, z) = (num("x").expect(usage), num("z").expect(usage));
    let radius = num("radius").unwrap_or(0.5);
    let bytes = std::fs::read(std::path::Path::new(&dir).join("WORLDS.PS2")).expect("WORLDS.PS2");
    let world = gdl_formats::WorldFile::parse(&bytes).expect("world");
    let collision = LevelCollision::new(&world).expect("collision");
    let [lo, hi] = collision.bounds;
    println!("bounds {lo:?} – {hi:?}, falling out below {}", collision.kill_height());
    for (label, disable_mask) in [("every query", 0), ("the player's", 1)] {
        println!("{label} (radius {radius}):");
        let q = Query { disable_mask, prefer_crossing: false, ..Query::floors(radius) };
        let mut y = hi[1] + 1.0;
        while let Some(h) = collision.cast([x, y, z], [x, lo[1] - 1.0, z], &q) {
            let n = &collision.nodes[h.node];
            println!(
                "  y {:9.3}  node {:5} {:24} flags {:#010x} disable {}",
                h.point[1], h.node, world.nodes[h.node].name, n.flags, n.disable
            );
            y = h.point[1].min(y) - radius - 0.05;
        }
    }
}
