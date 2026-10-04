//! Writes a texture binding as a binary PPM (colour) and PGM (alpha), each
//! texel enlarged (dev tool).
//! Usage: texppm <bank dir> <binding> <out prefix> [scale]
use std::io::Write;
fn main() {
    let mut a = std::env::args().skip(1);
    let dir = a.next().expect("usage: texppm <bank dir> <binding> <out prefix> [scale]");
    let binding: usize = a.next().expect("binding").parse().unwrap();
    let out = a.next().expect("out prefix");
    let k: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let model = gdl_formats::ModelFile::parse(&std::fs::read(format!("{dir}/objects.ngc")).unwrap()).unwrap();
    let textures = std::fs::read(format!("{dir}/textures.ngc")).unwrap();
    let img = gdl_formats::texture::decode(&textures, &model.bindings[binding]).unwrap();
    let (w, h) = (img.width as usize, img.height as usize);
    let mut rgb = Vec::with_capacity(w * h * k * k * 3);
    let mut alpha = Vec::with_capacity(w * h * k * k);
    for y in 0..h * k {
        for x in 0..w * k {
            let p = &img.pixels[((y / k) * w + x / k) * 4..][..4];
            rgb.extend_from_slice(&p[..3]);
            alpha.push(p[3]);
        }
    }
    let mut f = std::fs::File::create(format!("{out}.ppm")).unwrap();
    write!(f, "P6\n{} {}\n255\n", w * k, h * k).unwrap();
    f.write_all(&rgb).unwrap();
    let mut f = std::fs::File::create(format!("{out}_alpha.pgm")).unwrap();
    write!(f, "P5\n{} {}\n255\n", w * k, h * k).unwrap();
    f.write_all(&alpha).unwrap();
}
