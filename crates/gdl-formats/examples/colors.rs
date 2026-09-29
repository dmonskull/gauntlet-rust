//! Counts submeshes by (has lightmap, has vertex colours) across level files (dev tool).
fn main() {
    let root = std::env::args().nth(1).expect("usage: colors <LEVELS dir>");
    let mut counts = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(root).unwrap().flatten() {
        let Ok(d) = std::fs::read(e.path().join("objects.ngc")) else { continue };
        let m = gdl_formats::ModelFile::parse(&d).unwrap();
        for s in m.objects.iter().flat_map(|o| &o.submeshes) {
            let colours = s.vertices.iter().any(|v| v.color.is_some());
            let all = s.vertices.iter().all(|v| v.color.is_some());
            *counts.entry((s.descriptor.lightmap != 0, colours, all)).or_insert(0) += 1;
        }
    }
    for ((lm, any, all), n) in counts {
        println!("lightmap={lm} colours(any={any}, all={all}): {n}");
    }
}
