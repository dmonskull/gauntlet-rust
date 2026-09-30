//! Lists every critter file's `NODE` hit spheres (dev tool).
//!
//! ```text
//! cargo run -p gdl-formats --example critnodes -- <game>/Gauntlet/CRITTER
//! ```
fn main() {
    let dir = std::env::args().nth(1).expect("usage: critnodes <CRITTER dir>");
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(file) = gdl_formats::critter::CritterFile::parse(&std::fs::read(&path).unwrap()) else { continue };
        for (t, ty) in file.types.iter().enumerate() {
            for n in file.type_nodes(t) {
                println!(
                    "{} {}: {:>10} flags {:#x} reach {} weight {} radius {} scale {} hp {}",
                    path.file_stem().unwrap().to_string_lossy(),
                    ty.name,
                    n.name,
                    n.flags,
                    n.reach,
                    n.weight,
                    n.radius,
                    n.damage_scale,
                    n.hit_points
                );
            }
        }
    }
}
