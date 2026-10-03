//! Prints, for an action of an atree, each bone's track: how many keys
//! and the first key's rotation (dev tool: what a clip with no frames
//! holds).
//!
//! ```text
//! cargo run -p gdl-formats --example cliptracks -- <game>/Gauntlet/PLAYERS/ARC/ANIM THROW1
//! ```
fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: cliptracks <bank folder> <action name>");
    let name = args.next().expect("an action name").to_ascii_uppercase();
    let bytes = std::fs::read(std::path::Path::new(&dir).join("ANIM.PS2")).expect("ANIM.PS2");
    let anim = gdl_formats::anim::AnimFile::parse(&bytes).expect("anim");
    for atree in &anim.atrees {
        let Some(action) = atree.actions.iter().position(|a| a.name == name) else { continue };
        println!("{} action {action} {name}: {} frames, rate {}", atree.name, atree.actions[action].frames, atree.actions[action].rate);
        for (bone, node) in atree.nodes.iter().enumerate() {
            match atree.track(bone, action) {
                Ok(Some(track)) => println!("  {bone:3} {:16} {} keys, first {:?}", node.name, track.keys.len(), track.keys.first().map(|k| k.1.rotation)),
                Ok(None) => println!("  {bone:3} {:16} at rest", node.name),
                Err(e) => println!("  {bone:3} {:16} {e}", node.name),
            }
        }
    }
}
