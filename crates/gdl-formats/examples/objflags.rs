//! Lists the objects of model files whose flags (`+0x08`) have any of the
//! given bits (dev tool).
//! Usage: objflags <hex mask> <objects.ngc>...
fn main() {
    let mut args = std::env::args().skip(1);
    let mask = u32::from_str_radix(args.next().expect("usage: objflags <hex mask> <objects.ngc>...").trim_start_matches("0x"), 16).unwrap();
    for path in args {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(model) = gdl_formats::ModelFile::parse(&bytes) else { continue };
        let hits: Vec<&str> = model.objects.iter().filter(|o| o.flags & mask != 0).map(|o| o.name.as_str()).collect();
        if !hits.is_empty() {
            let shown: Vec<&str> = hits.iter().copied().take(6).collect();
            println!("{path}: {} of {} objects, e.g. {shown:?}", hits.len(), model.objects.len());
        }
    }
}
