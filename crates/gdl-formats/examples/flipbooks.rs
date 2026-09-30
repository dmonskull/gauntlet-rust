//! Lists flipbook nodes' entries per action — first object, frame count
//! and start frame — against the action's length (dev tool).
fn main() {
    let path = std::env::args().nth(1).expect("usage: flipbooks <ANIM.PS2> [atree]");
    let only = std::env::args().nth(2);
    let anim = gdl_formats::anim::AnimFile::parse(&std::fs::read(path).unwrap()).unwrap();
    for t in &anim.atrees {
        if only.as_ref().is_some_and(|o| !t.name.eq_ignore_ascii_case(o)) {
            continue;
        }
        for (i, n) in t.nodes.iter().enumerate().filter(|(_, n)| n.kind == gdl_formats::anim::NodeKind::Flipbook) {
            for (a, action) in t.actions.iter().enumerate() {
                let Some(e) = t.flipbook_entry(i, a) else { continue };
                let end = i32::from(e.param) + i32::from(e.frames) - 1;
                let note = if e.frames > 1 && end < i32::from(action.frames) - 1 { "  <- ends early" } else { "" };
                println!(
                    "{:14} {:14} {:10} ({:3} frames{}) first {:24} count {:3} start {:3}{note}",
                    t.name,
                    n.name,
                    action.name,
                    action.frames,
                    if action.loops() { ", loops" } else { "" },
                    e.first,
                    e.frames,
                    e.param
                );
            }
        }
    }
}
