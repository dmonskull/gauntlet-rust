//! Prints one object of a model file: its submeshes' textures, flags and
//! extents, and its vertices (dev tool).
//! Usage: object_dump <objects.ngc> <object name> [--verts]
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: object_dump <objects.ngc> <name> [--verts]");
    let name = args.next().expect("an object name");
    let verts = args.next().is_some_and(|a| a == "--verts");
    let model = gdl_formats::ModelFile::parse(&std::fs::read(path).unwrap()).unwrap();
    let Some(o) = model.objects.iter().find(|o| o.name.eq_ignore_ascii_case(&name)) else {
        let near: Vec<&str> = model.objects.iter().map(|o| o.name.as_str()).filter(|n| n.contains(&name[..name.len().min(4)])).collect();
        println!("no object {name}; like it: {near:?}");
        return;
    };
    println!("object {}: flags {:#x}, {} submeshes", o.name, o.flags, o.submeshes.len());
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (i, s) in o.submeshes.iter().enumerate() {
        let (mut slo, mut shi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &s.vertices {
            for a in 0..3 {
                slo[a] = slo[a].min(v.position[a]);
                shi[a] = shi[a].max(v.position[a]);
                lo[a] = lo[a].min(v.position[a]);
                hi[a] = hi[a].max(v.position[a]);
            }
        }
        let d = &s.descriptor;
        let texture = model.texture_names.iter().find(|t| t.binding == d.texture).map_or("?", |t| t.name.as_str());
        println!(
            "  submesh {i}: texture {} ({texture}), lightmap {}, {} vertices, {} triangles, min {slo:?} max {shi:?}",
            d.texture,
            d.lightmap,
            s.vertices.len(),
            s.triangles.len()
        );
        println!("    binding: {:?}", model.bindings.get(d.texture as usize));
        if verts {
            for v in &s.vertices {
                println!("    {v:?}");
            }
            for t in &s.triangles {
                println!("    tri {t:?}");
            }
        }
    }
    println!("min {lo:?}\nmax {hi:?}");
}
