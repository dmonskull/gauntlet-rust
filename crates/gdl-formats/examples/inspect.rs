//! Lists a model file's objects, submeshes and extents (dev tool).
fn main() {
    let path = std::env::args().nth(1).expect("usage: inspect <objects.ngc>");
    let model = gdl_formats::ModelFile::parse(&std::fs::read(path).unwrap()).unwrap();
    let h = &model.header;
    println!("version {:08X}: {} objects, {} bindings, {} texture names; unnamed 6c={:X} 70={:X} 74={:X} 78={:X} 7c={} 7e={}",
        h.version, h.num_objects, h.num_bindings, h.num_texture_names, h.unnamed_0x6c, h.unnamed_0x70, h.unnamed_0x74, h.unnamed_0x78, h.unnamed_0x7c, h.unnamed_0x7e);
    for o in &model.objects {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let mut verts = 0;
        for v in o.submeshes.iter().flat_map(|s| &s.vertices) {
            verts += 1;
            for a in 0..3 { lo[a] = lo[a].min(v.position[a]); hi[a] = hi[a].max(v.position[a]); }
        }
        let tex: Vec<_> = o.submeshes.iter().map(|s| (s.descriptor.texture, s.descriptor.lightmap)).collect();
        println!("{:16} flags={:08X} verts={verts:5} tex={tex:?} lo={lo:.1?} hi={hi:.1?}", o.name, o.flags);
    }
}
