//! Lists the boss levels' camera records (dev tool).
fn main() {
    let dir = std::env::args().nth(1).expect("usage: bcam <game>/Gauntlet/WDATA");
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let Ok(w) = gdl_formats::WorldData::parse(&std::fs::read(e.path()).unwrap()) else { continue };
        for l in w.levels.iter().filter(|l| l.boss_camera.is_some()) {
            let c = l.boss_camera.unwrap();
            println!(
                "{:4} flags {:#06x} yaw off {:5.1}° near {:5.1}/{:5.1} far {:5.1}/{:5.1} pitch {:5.1}°/{:5.1}° look {:?}/{:?} key {:?} wizard {:?}",
                l.name,
                c.flags,
                c.yaw_offset.to_degrees(),
                c.near,
                c.near_asleep,
                c.far,
                c.far_asleep,
                c.pitch_near.to_degrees(),
                c.pitch_far.to_degrees(),
                c.look_near,
                c.look_far,
                c.look_key,
                c.look_wizard
            );
        }
    }
}
