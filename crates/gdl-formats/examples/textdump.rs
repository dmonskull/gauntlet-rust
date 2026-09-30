//! Prints a text ROM's groups (and lists) whose names contain a text (dev
//! tool).
//!
//! ```text
//! cargo run -p gdl-formats --example textdump -- <game>/Gauntlet/TEXT/SCROLL_E.ROM [text]
//! ```
fn main() {
    let path = std::env::args().nth(1).expect("usage: textdump <ROM> [text]");
    let only = std::env::args().nth(2).unwrap_or_default().to_ascii_uppercase();
    let rom = gdl_formats::text::TextRom::parse(&std::fs::read(path).unwrap()).unwrap();
    println!("fonts: {:?}", rom.fonts);
    for (i, g) in rom.groups.iter().enumerate().filter(|(_, g)| g.name.to_ascii_uppercase().contains(&only)) {
        println!("group {i} {} (font {}, scale {:?}): {} strings", g.name, g.font, g.scale, g.strings.len());
        for (k, s) in g.strings.iter().enumerate() {
            println!("  {k:2}: {s:?}");
        }
    }
    for l in rom.lists.iter().filter(|l| l.name.to_ascii_uppercase().contains(&only)) {
        let names: Vec<_> = l.groups.iter().map(|&g| rom.groups[g].name.as_str()).collect();
        println!("list {}: {names:?}", l.name);
    }
}
