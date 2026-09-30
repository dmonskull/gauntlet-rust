//! Lists each atree's texture modifiers: the ones its actions run
//! (`+0x28`/`+0x2C`) and its kind-3 nodes' (dev tool).
fn main() {
    let path = std::env::args().nth(1).expect("usage: tmnodes <ANIM.PS2> [atree]");
    let only = std::env::args().nth(2);
    let bytes = std::fs::read(path).unwrap();
    let anim = gdl_formats::anim::AnimFile::parse(&bytes).unwrap();
    let texmods = gdl_formats::texmod::TexMod::parse_all(&bytes).unwrap();
    let show = |k: usize| {
        let m = &texmods[k];
        format!("{k:4} {:18} binding {:4} {:?} count {} phase {} period {}", m.name, m.binding, m.kind, m.count, m.phase, m.period)
    };
    for (a, t) in anim.atrees.iter().enumerate() {
        if only.as_ref().is_some_and(|o| !t.name.eq_ignore_ascii_case(o)) {
            continue;
        }
        let owned = texmods.iter().filter(|m| usize::try_from(m.owner) == Ok(a)).count();
        if owned == 0 {
            continue;
        }
        println!("atree {a} {}: {owned} owned, {} nodes", t.name, t.texmod_nodes.len());
        for action in &t.actions {
            for k in action.texmods() {
                println!("  action {:12} {}", action.name, show(k));
            }
        }
        for &(n, k) in &t.texmod_nodes {
            println!("  node {:28} {}", t.nodes[n].name, show(k));
        }
    }
}
