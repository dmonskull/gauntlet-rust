//! Lists a level's world nodes whose names contain a text, with their
//! flags, children and positions (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example nodes -- <game>/Gauntlet/LEVELS/levelL1 LIGHTRAY
//! ```
fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: nodes <level folder> [text]");
    let text = args.next().unwrap_or_default().to_ascii_uppercase();
    let bytes = std::fs::read(std::path::Path::new(&dir).join("WORLDS.PS2")).expect("WORLDS.PS2");
    let world = gdl_formats::WorldFile::parse(&bytes).expect("world");
    // World positions: each node's translation added to its parent's.
    let mut parent = vec![None; world.nodes.len()];
    for (i, n) in world.nodes.iter().enumerate() {
        let mut c = n.first_child;
        while let Some(k) = c {
            parent[k] = Some(i);
            c = world.nodes[k].next_sibling;
        }
    }
    let origin = |mut i: usize| {
        let mut at = [0.0f32; 3];
        loop {
            for (a, v) in at.iter_mut().zip(world.nodes[i].local_position) {
                *a += v;
            }
            match parent[i] {
                Some(p) => i = p,
                None => return at,
            }
        }
    };
    for (i, n) in world.nodes.iter().enumerate().filter(|(_, n)| n.name.to_ascii_uppercase().contains(&text)) {
        let mut children = 0;
        let mut c = n.first_child;
        while let Some(k) = c {
            children += 1;
            c = world.nodes[k].next_sibling;
        }
        println!(
            "{i:5} {:24} flags {:#010x} render {:#010x} model {} children {children} at {:?}",
            n.name,
            n.flags,
            n.render_flags,
            n.has_model,
            origin(i)
        );
    }
}
