//! Prints an ANIM.PS2's atrees: skeleton, and the first actions (dev tool).
fn main() {
    let path = std::env::args().nth(1).expect("usage: skeleton <ANIM.PS2>");
    let anim = gdl_formats::anim::AnimFile::parse(&std::fs::read(path).unwrap()).unwrap();
    for t in &anim.atrees {
        println!("atree {:?}: {} nodes, {} actions, clips: {}", t.name, t.nodes.len(), t.actions.len(), t.clips.is_some());
        for (i, n) in t.nodes.iter().enumerate() {
            println!("  {i:2} {:14} parent={:?} offset={:.3?}", n.name, n.parent, n.offset);
        }
        let names: Vec<_> = t.actions.iter().map(|a| format!("{}({},{},{:?})", a.name, a.frames, a.rate, a.params)).collect();
        println!("  actions: {}", names.join(" "));
    }
}
