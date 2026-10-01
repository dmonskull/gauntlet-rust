//! Lists a bank's atrees with their node, action and particle-system
//! counts (dev tool). With `--nodes`, each atree's nodes too: name,
//! parent, kind, flags and rest offset; and its actions' names.
//!
//! ```text
//! cargo run -p gdl-formats --example atreeparts -- <game>/Gauntlet/ITEMS/levelL [name prefix] [--nodes]
//! ```
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let nodes = args.iter().any(|a| a == "--nodes");
    let mut plain = args.iter().filter(|a| !a.starts_with("--"));
    let dir = plain.next().expect("usage: atreeparts <bank folder> [prefix] [--nodes]");
    let prefix = plain.next().cloned().unwrap_or_default().to_ascii_uppercase();
    let bytes = std::fs::read(std::path::Path::new(&dir).join("ANIM.PS2")).expect("ANIM.PS2");
    let anim = gdl_formats::anim::AnimFile::parse(&bytes).expect("anim");
    for a in anim.atrees.iter().filter(|a| a.name.to_ascii_uppercase().starts_with(&prefix)) {
        let frames: Vec<u16> = a.actions.iter().map(|x| x.frames).collect();
        println!("{:16} nodes {:3} actions {:?} particles {}", a.name, a.nodes.len(), frames, a.particles.len());
        if !nodes {
            continue;
        }
        for (i, n) in a.nodes.iter().enumerate() {
            println!(
                "    {i:3} {:20} parent {:>4} kind {:?} flags {:#06x} render {:#010x} model {} offset [{:.2}, {:.2}, {:.2}]",
                n.name,
                n.parent.map_or("-".to_string(), |p| p.to_string()),
                n.kind,
                n.node_flags,
                n.render_flags,
                n.has_model(),
                n.offset[0],
                n.offset[1],
                n.offset[2]
            );
        }
        for (i, x) in a.actions.iter().enumerate() {
            println!("    action {i}: {:?} {} frames", x.name, x.frames);
            // Each animated node's keys in it: frame, translation,
            // rotation (degrees), scale.
            for (n, node) in a.nodes.iter().enumerate() {
                let Some(track) = a.clip_bone(n).and_then(|b| a.track(b, i).ok().flatten()) else { continue };
                let keys: Vec<String> = track
                    .keys
                    .iter()
                    .map(|(f, p)| {
                        let r = p.rotation.map(f32::to_degrees);
                        format!(
                            "{f}: t({:.2},{:.2},{:.2}) r({:.0},{:.0},{:.0}) s({:.2},{:.2},{:.2})",
                            p.translation[0], p.translation[1], p.translation[2], r[0], r[1], r[2], p.scale[0], p.scale[1], p.scale[2]
                        )
                    })
                    .collect();
                println!("        {} flags {:#06x}: {}", node.name, track.flags, keys.join("; "));
            }
            // Each flipbook node's run in it: first object, frames, the
            // action frame it starts on.
            for (n, node) in a.nodes.iter().enumerate() {
                if let Some(e) = a.flipbook_entry(n, i) {
                    println!("        {} flipbook: {:?} × {} from frame {}", node.name, e.first, e.frames, e.param);
                }
            }
        }
    }
}
