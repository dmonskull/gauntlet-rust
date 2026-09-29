//! Lists a model file's texture names with their binding counts (dev tool).
fn main() {
    let path = std::env::args().nth(1).expect("usage: texnames <objects.ngc>");
    let model = gdl_formats::ModelFile::parse(&std::fs::read(path).unwrap()).unwrap();
    println!("{} bindings, {} names", model.bindings.len(), model.texture_names.len());
    for (i, t) in model.texture_names.iter().enumerate() {
        println!("{i:4} {t:?}");
    }
}
