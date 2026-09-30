//! Lists a folder's texture modifiers (`ANIM.PS2`) with the textures they
//! change and the atrees that own them (dev tool).
fn main() {
    let dir = std::env::args().nth(1).expect("usage: texmods <folder with ANIM.PS2 and objects.ngc>");
    let dir = std::path::Path::new(&dir);
    let bytes = std::fs::read(dir.join("ANIM.PS2")).unwrap();
    let model = std::fs::read(dir.join("objects.ngc")).ok().and_then(|b| gdl_formats::ModelFile::parse(&b).ok());
    let atrees = gdl_formats::anim::AnimFile::parse(&bytes).map(|a| a.atrees).unwrap_or_default();
    let name_of = |binding: u16| {
        model.as_ref().and_then(|m| m.texture_names.iter().find(|t| t.binding == binding)).map_or("?", |t| t.name.as_str())
    };
    for m in gdl_formats::texmod::TexMod::parse_all(&bytes).unwrap() {
        let owner = usize::try_from(m.owner).ok().and_then(|i| atrees.get(i)).map_or("-", |a| a.name.as_str());
        println!(
            "owner {:3} {owner:12} {:16} binding {:4} ({}) {:?} count {} phase {} period {} start {}",
            m.owner,
            m.name,
            m.binding,
            name_of(m.binding),
            m.kind,
            m.count,
            m.phase,
            m.period,
            m.start
        );
    }
}
