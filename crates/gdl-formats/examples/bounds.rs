//! Prints a model file's extents and normal orientation (dev tool).
fn main() {
    let path = std::env::args().nth(1).expect("usage: bounds <objects.ngc>");
    let model = gdl_formats::ModelFile::parse(&std::fs::read(path).unwrap()).unwrap();
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let (mut up, mut down, mut area_up, mut area_down) = (0, 0, 0.0f32, 0.0f32);
    for s in model.objects.iter().flat_map(|o| &o.submeshes) {
        for v in &s.vertices {
            for a in 0..3 {
                lo[a] = lo[a].min(v.position[a]);
                hi[a] = hi[a].max(v.position[a]);
            }
            if v.normal[1] > 0.7 { up += 1 } else if v.normal[1] < -0.7 { down += 1 }
        }
        // Geometric normals from triangle winding, weighted by area.
        for t in &s.triangles {
            let p = t.map(|i| s.vertices[i as usize].position);
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let ny = e1[2] * e2[0] - e1[0] * e2[2];
            if ny > 0.0 { area_up += ny } else { area_down -= ny }
        }
    }
    println!("min {lo:?}\nmax {hi:?}");
    println!("stored normals: {up} mostly +Y, {down} mostly -Y");
    println!("winding normals (area): +Y {area_up:.0}, -Y {area_down:.0}");
}
