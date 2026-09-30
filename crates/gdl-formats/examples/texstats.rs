//! Prints a texture binding's format, alpha histogram and colours (dev
//! tool).
//!
//! ```text
//! cargo run -p gdl-formats --example texstats -- <folder with objects.ngc> <binding>
//! ```

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: texstats <folder> <binding>");
    let binding: usize = a.next().and_then(|b| b.parse().ok()).expect("binding");
    let model = gdl_formats::ModelFile::parse(&std::fs::read(format!("{dir}/objects.ngc")).unwrap()).unwrap();
    let textures = std::fs::read(format!("{dir}/textures.ngc")).unwrap();
    let b = &model.bindings[binding];
    let img = gdl_formats::texture::decode(&textures, b).expect("decode");
    let mut hist = std::collections::BTreeMap::new();
    for p in img.pixels.as_chunks::<4>().0 {
        *hist.entry(p[3] / 32 * 32).or_insert(0usize) += 1;
    }
    let px: Vec<_> = img.pixels.as_chunks::<4>().0.iter().collect();
    let avg = |c: usize| px.iter().map(|p| p[c] as f32).sum::<f32>() / px.len().max(1) as f32;
    let max = |c: usize| px.iter().map(|p| p[c]).max().unwrap_or(0);
    println!("binding {binding} fmt {:#x} flags {:#x} {}x{}", b.format, b.flags_raw, img.width, img.height);
    println!("alpha histogram: {hist:?}");
    println!("average rgba {:.0} {:.0} {:.0} {:.0}; max {} {} {} {}", avg(0), avg(1), avg(2), avg(3), max(0), max(1), max(2), max(3));
    // Raw RGBA next to it, for looking at (`<binding>.rgba`, width × height).
    if let Some(out) = a.next() {
        std::fs::write(&out, &img.pixels).expect("write");
    }
}
