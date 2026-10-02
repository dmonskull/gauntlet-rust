//! Prints the catalog names of sound ids (`bank << 16 | call`, hex) — the
//! game plays sounds by id (dev tool). An argument that isn't hex finds
//! the sounds whose names contain it instead.
//!
//! ```text
//! cargo run -p gdl-formats --example soundids -- <game>/Gauntlet/AUDIO/AUDATPS2.ROM e00a3 3000b
//! ```
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: soundids <AUDATPS2.ROM> <id>...");
    let catalog = gdl_formats::audio::AudioCatalog::parse(&std::fs::read(path).unwrap()).unwrap();
    for arg in args {
        let Ok(id) = u32::from_str_radix(arg.trim_start_matches("0x"), 16) else {
            let want = arg.to_ascii_uppercase();
            for s in catalog.sounds.iter().filter(|s| s.name.to_ascii_uppercase().contains(&want)) {
                println!("{}: {:#x}", s.name, (s.bank << 16) | s.call);
            }
            continue;
        };
        let (bank, call) = ((id >> 16) as usize, (id & 0xFFFF) as usize);
        let names: Vec<_> = catalog.sounds.iter().filter(|s| s.bank == bank && s.call == call).map(|s| s.name.as_str()).collect();
        println!("{id:#x}: {names:?}");
    }
}
