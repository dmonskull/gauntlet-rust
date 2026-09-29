//! Prints the alpha histogram of the textures an object uses (dev tool).
fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: alpha <level dir> <object names...>");
    let model = gdl_formats::ModelFile::parse(&std::fs::read(format!("{dir}/objects.ngc")).unwrap()).unwrap();
    let textures = std::fs::read(format!("{dir}/textures.ngc")).unwrap();
    for name in a {
        let Some(o) = model.objects.iter().find(|o| o.name == name) else { continue };
        for s in &o.submeshes {
            let b = &model.bindings[s.descriptor.texture as usize];
            let Ok(img) = gdl_formats::texture::decode(&textures, b) else { continue };
            let mut hist = std::collections::BTreeMap::new();
            for p in img.pixels.as_chunks::<4>().0 {
                *hist.entry(p[3] / 32 * 32).or_insert(0usize) += 1;
            }
            let opaque: Vec<_> = img.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 64).collect();
            let avg = |c: usize| opaque.iter().map(|p| p[c] as f32).sum::<f32>() / opaque.len().max(1) as f32;
            println!("  rgb of alpha>64 texels: {:.0} {:.0} {:.0}", avg(0), avg(1), avg(2));
            println!("{name} tex {} fmt {:#x} flags {:#x} {}x{}: {hist:?}", s.descriptor.texture, b.format, b.flags_raw, img.width, img.height);
        }
    }
}
