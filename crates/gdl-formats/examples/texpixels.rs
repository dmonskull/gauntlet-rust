//! Prints a texture binding's size and distinct texels (dev tool).
fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: texpixels <bank dir> <binding>");
    let binding: usize = a.next().expect("binding").parse().unwrap();
    let model = gdl_formats::ModelFile::parse(&std::fs::read(format!("{dir}/objects.ngc")).unwrap()).unwrap();
    let textures = std::fs::read(format!("{dir}/textures.ngc")).unwrap();
    let b = &model.bindings[binding];
    let img = gdl_formats::texture::decode(&textures, b).unwrap();
    let mut seen = std::collections::BTreeMap::new();
    for p in img.pixels.as_chunks::<4>().0 {
        *seen.entry(*p).or_insert(0usize) += 1;
    }
    println!("{}x{} fmt {:#x} at {:#x}: {} distinct", img.width, img.height, b.format, b.texture_offset, seen.len());
    let o = b.texture_offset as usize;
    println!("  raw {:02x?}", &textures[o..o + 48]);
    for (p, n) in seen.iter().take(8) {
        println!("  {p:?} × {n}");
    }
}
