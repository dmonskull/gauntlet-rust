//! World particle systems (`docs/rendering.md`, "Particle-system nodes"):
//! the torch flames, smoke, pool fires, mist, embers and fireflies that
//! a level's `…PSYS<letter>…` nodes give off.
//!
//! Each node runs the level's particle record with its letter
//! (`gdl_formats::psys`), laid over the built-in preset the record names.
//! Particles leave the node's origin at the record's rate, within its
//! spray cone around its direction, at its speed, jittered by its start
//! box; they rise (or fall) by the record's buoyancy, and over their life
//! run through its four colour, alpha and size keys. They're drawn as
//! camera-facing squares of the record's texture, additively when the
//! record says so.
//!
//! Stand-ins: the game's library keeps particles in packed rings and works
//! them out from tables its emitters' callbacks fill; this simulates each
//! particle directly from the record's values instead. The emitter's
//! timed phases (the record's two times; 999 s on every record here) are
//! taken as "always on", the unnamed fields (word 0x24, the counts) aren't
//! used, and trigger-switched emitters always run.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use gdl_formats::psys::{ParticleRecord, fields};

use crate::level_material::LevelMaterial;

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, simulate.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// A particle system's working values, in world units and seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Shortest life, and the most a random extra adds.
    pub life: [f32; 2],
    /// Half-angle of the spray cone, radians (π: all round).
    pub spread: f32,
    pub direction: Vec3,
    /// Half-size of the box particles start in.
    pub jitter: Vec3,
    /// Particles a second.
    pub rate: f32,
    /// How much the rate varies (0–1).
    pub rate_jitter: f32,
    /// Upward acceleration, units/s² (the record's buoyancy × −32).
    pub rise: f32,
    /// Start speed, units a second.
    pub speed: f32,
    /// Colour keys (linear 0–1 RGB) and alpha keys over the particle's life.
    pub colours: [[f32; 3]; 4],
    pub alphas: [f32; 4],
    /// Size keys (the square's width), units.
    pub sizes: [f32; 4],
    pub additive: bool,
}

/// Flag bit that makes a system draw additively.
const ADDITIVE_FLAG: u32 = 0x80;
/// The game turns the buoyancy into a per-frame acceleration with
/// −32 / 900: −32 units/s² per unit.
const RISE_SCALE: f32 = -32.0;

/// The built-in presets (the table the game's parameter routine starts
/// from when a record's field bit 1 is set), by id: life, spread (degrees,
/// full cone), rate, rate jitter (per cent), buoyancy, speed, colour keys,
/// size keys, flags.
#[allow(clippy::type_complexity)]
const PRESETS: [([f32; 2], f32, f32, f32, f32, f32, [u32; 4], [f32; 4], u32); 8] = [
    ([0.4, 0.4], 360.0, 100.0, 0.0, 0.0, 10.0, [0xFFFF_0000, 0xFF00_00FF, 0xFF00_00FF, 0x0000_00FF], [0.1, 1.0, 1.0, 0.1], 0x800),
    ([10.0, 5.0], 0.0, 1.0, 1.0, 0.0, 15.0, [0xFFFF_FFFF; 4], [1.0; 4], 0x800),
    ([0.4, 0.3], 20.0, 60.0, 0.0, 0.0, 15.0, [0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0x00FF_FFFF], [0.3; 4], 0x800),
    ([1.0, 5.0], 40.0, 7.0, 0.0, 0.0, 3.0, [0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0x00FF_FFFF], [1.0, 3.0, 3.0, 7.0], 0x800),
    ([0.4, 0.4], 360.0, 100.0, 0.0, 0.0, 10.0, [0xFFFF_0000, 0xFF00_00FF, 0xFF00_00FF, 0x0000_00FF], [0.1, 1.0, 1.0, 0.1], 0x808),
    ([10.0, 5.0], 0.0, 1.0, 1.0, 0.0, 45.0, [0xFFFF_FFFF; 4], [1.0; 4], 0x808),
    ([1.0, 0.0], 20.0, 60.0, 0.0, 0.0, 30.0, [0xFFFF_FFFF; 4], [0.3; 4], 0x808),
    ([1.0, 5.0], 360.0, 7.0, 0.0, -0.1, 3.0, [0xFFFF_FFFF; 4], [1.0, 3.0, 3.0, 7.0], 0x808),
];

fn spread_radians(degrees: f32) -> f32 {
    // The game: under 0 or 359 and up is all round; 0–1 taken as is;
    // otherwise π × degrees / 360 (the half-angle of a full cone).
    if !(0.0..359.0).contains(&degrees) {
        std::f32::consts::PI
    } else if degrees <= 1.0 {
        degrees
    } else {
        std::f32::consts::PI * degrees / 360.0
    }
}

fn colour(word: u32) -> [f32; 3] {
    let c = |shift: u32| ((word >> shift) & 0xFF) as f32 / 255.0;
    [c(16), c(8), c(0)]
}

fn alpha(word: u32) -> f32 {
    (word >> 24) as f32 / 255.0
}

impl Params {
    /// A record laid over its preset.
    pub fn of(r: &ParticleRecord) -> Self {
        let preset = PRESETS[usize::try_from(r.preset).unwrap_or(0).min(PRESETS.len() - 1)];
        let (life, spread, rate, rate_jitter, rise, speed, cols, sizes, flags) = preset;
        let mut p = Params {
            life,
            spread: spread_radians(spread),
            direction: Vec3::Y,
            jitter: Vec3::ZERO,
            rate,
            rate_jitter: rate_jitter * 0.01,
            rise: rise * RISE_SCALE,
            speed,
            colours: cols.map(colour),
            alphas: cols.map(alpha),
            sizes,
            additive: flags & ADDITIVE_FLAG != 0,
        };
        let w = |i: usize| f32::from_bits(r.words[i]);
        if r.has(fields::LIFE) {
            p.life = [w(10), w(11)];
        }
        if r.has(fields::SPREAD) {
            p.spread = spread_radians(w(14));
        }
        if r.has(fields::DIRECTION) {
            p.direction = Vec3::new(w(0x18), w(0x19), w(0x1A)).normalize_or(Vec3::Y);
        }
        if r.has(fields::ACCEL) {
            p.jitter = Vec3::new(w(0x1B), w(0x1C), w(0x1D));
        }
        if r.has(fields::SPEEDS) {
            p.rate = w(0x1E);
        }
        if r.has(fields::DRAG) {
            p.rate_jitter = w(0x22) * 0.01;
        }
        if r.has(fields::SPIN) {
            p.rise = w(0x23) * RISE_SCALE;
        }
        if r.has(fields::SPEED_C) {
            p.speed = w(0x25);
        }
        let keys = r.colour_keys();
        if r.has(fields::COLOURS) {
            p.colours = keys.map(colour);
        }
        if r.has(fields::ALPHAS) {
            p.alphas = keys.map(alpha);
        }
        if r.has(fields::SIZES) {
            p.sizes = r.size_keys();
        }
        if r.flag_mask & ADDITIVE_FLAG != 0 {
            p.additive = r.flag_values & ADDITIVE_FLAG != 0;
        }
        p
    }

    /// The most particles alive at once.
    fn capacity(&self) -> usize {
        ((self.rate * (self.life[0] + self.life[1].max(0.0))).ceil() as usize + 4).min(512)
    }
}

/// Linear through four evenly spaced keys at `t` (0–1).
fn keyed<T: Copy + std::ops::Mul<f32, Output = T> + std::ops::Add<Output = T>>(keys: [T; 4], t: f32) -> T {
    let x = t.clamp(0.0, 1.0) * 3.0;
    let i = (x.floor() as usize).min(2);
    let f = x - i as f32;
    keys[i] * (1.0 - f) + keys[i + 1] * f
}

struct Particle {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
}

/// A running particle system: its mesh is rebuilt every frame from its
/// particles, facing the camera.
#[derive(Component)]
pub struct Emitter {
    params: Params,
    origin: Vec3,
    particles: Vec<Particle>,
    /// Particles owed to the rate, carried between frames.
    owed: f32,
    rng: u32,
    mesh: Handle<Mesh>,
}

impl Emitter {
    fn random(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0xFF_FFFF) as f32 / 0x100_0000 as f32
    }

    fn spawn(&mut self) {
        let p = &self.params;
        let (dir, spread, jitter, speed, life) = (p.direction, p.spread, p.jitter, p.speed, p.life);
        // A direction in the cone: turn off the axis by up to the spread,
        // round it at any angle.
        let off = spread * self.random().sqrt();
        let round = std::f32::consts::TAU * self.random();
        let side = dir.any_orthonormal_vector();
        let other = dir.cross(side);
        let d = dir * off.cos() + (side * round.cos() + other * round.sin()) * off.sin();
        let j = Vec3::new(self.random() * 2.0 - 1.0, self.random() * 2.0 - 1.0, self.random() * 2.0 - 1.0) * jitter;
        let life = life[0] + life[1].max(0.0) * self.random();
        self.particles.push(Particle { position: self.origin + j, velocity: d * speed, age: 0.0, life: life.max(0.01) });
    }
}

/// Makes the emitters for a level's `PSYS` nodes: `emitters` is (node
/// name, world origin); `texture` finds a texture by name.
pub fn spawn_emitters(
    records: &[ParticleRecord],
    emitters: impl IntoIterator<Item = (String, Vec3)>,
    mut texture: impl FnMut(&str) -> Option<Handle<Image>>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
) -> usize {
    let mut made = 0;
    let mut looks: std::collections::HashMap<u8, Option<(Params, Handle<LevelMaterial>)>> = Default::default();
    for (i, (name, origin)) in emitters.into_iter().enumerate() {
        let Some(letter) = name.find("PSYS").and_then(|at| name.as_bytes().get(at + 4).copied()) else { continue };
        let look = looks.entry(letter).or_insert_with(|| {
            let r = records.iter().find(|r| r.letter == letter)?;
            let params = Params::of(r);
            let image = texture(&r.texture());
            let mode = if params.additive { AlphaMode::Add } else { AlphaMode::Blend };
            let material = materials.add(LevelMaterial::new(image, None, mode).with_depth(true, false));
            Some((params, material))
        });
        let Some((params, material)) = look.clone() else {
            debug!("no particle record for {name}");
            continue;
        };
        trace!("particle system {name} at {origin}");
        let mesh = meshes.add(empty_mesh());
        commands.spawn((
            Emitter {
                particles: Vec::with_capacity(params.capacity()),
                params,
                origin,
                owed: 0.0,
                rng: 0x9E37_79B9 ^ (i as u32).wrapping_mul(0x85EB_CA6B),
                mesh: mesh.clone(),
            },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::default(),
            bevy::camera::visibility::NoFrustumCulling,
            // The mesh starts empty, so give it room: particles stay within
            // a few tens of units of their emitter.
            bevy::camera::primitives::Aabb::from_min_max(origin - Vec3::splat(40.0), origin + Vec3::splat(40.0)),
            crate::world::LevelEntity,
        ));
        made += 1;
    }
    made
}

fn empty_mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new())
        .with_inserted_indices(Indices::U32(Vec::new()))
}

/// Emits, moves and ages every particle, then rebuilds each emitter's
/// camera-facing squares.
fn simulate(
    time: Res<Time<Virtual>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut emitters: Query<&mut Emitter>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let dt = time.delta_secs().min(0.1);
    let Some((_, cam)) = cameras.iter().find(|(c, _)| c.is_active) else { return };
    let (right, up) = (cam.right().as_vec3(), cam.up().as_vec3());
    let mut logged = false;
    for mut e in &mut emitters {
        let t = time.elapsed_secs();
        if !logged && (t % 10.0) < dt && t > 1.0 {
            logged = true;
            trace!(
                "emitter at {} has {} particles; {:?}",
                e.origin,
                e.particles.len(),
                e.particles.first().map(|p| (p.position, p.age, p.life))
            );
        }
        let e = &mut *e;
        // Emit.
        if dt > 0.0 {
            let jitter = e.params.rate_jitter;
            let wobble = if jitter > 0.0 { 1.0 + jitter * (2.0 * e.random() - 1.0) } else { 1.0 };
            e.owed += e.params.rate * wobble * dt;
            let cap = e.params.capacity();
            while e.owed >= 1.0 {
                e.owed -= 1.0;
                if e.particles.len() < cap {
                    e.spawn();
                }
            }
        }
        // Move and age.
        let rise = e.params.rise;
        for p in &mut e.particles {
            p.velocity.y += rise * dt;
            p.position += p.velocity * dt;
            p.age += dt;
        }
        e.particles.retain(|p| p.age < p.life);

        // Draw.
        let Some(mesh) = meshes.get_mut(&e.mesh) else { continue };
        let n = e.particles.len();
        let mut positions = Vec::with_capacity(n * 4);
        let mut colours = Vec::with_capacity(n * 4);
        let mut uvs = Vec::with_capacity(n * 4);
        let mut indices = Vec::with_capacity(n * 12);
        for p in &e.particles {
            let t = p.age / p.life;
            let half = 0.5 * keyed(e.params.sizes, t);
            let c = keyed(e.params.colours.map(Vec3::from), t);
            let a = keyed(e.params.alphas, t);
            let base = positions.len() as u32;
            for (sx, sy, u, v) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
                positions.push((p.position + (right * sx + up * sy) * half).to_array());
                // Raw 0–1 values: the level shader multiplies them in gamma
                // space like the game's vertex colours (× 2 there, so half).
                colours.push([0.5 * c.x, 0.5 * c.y, 0.5 * c.z, a]);
                uvs.push([u, v]);
            }
            // Both windings: the level material culls back faces.
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
        }
        let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
        mesh.insert_indices(Indices::U32(indices));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_run_through_four_points() {
        assert_eq!(keyed([0.0, 1.0, 1.0, 0.0], 0.0), 0.0);
        assert!((keyed([0.0, 1.0, 1.0, 0.0], 1.0 / 3.0) - 1.0).abs() < 1e-5);
        assert!((keyed([0.0, 1.0, 1.0, 0.0], 0.5) - 1.0).abs() < 1e-5);
        assert_eq!(keyed([0.0, 1.0, 1.0, 0.0], 1.0), 0.0);
    }

    #[test]
    fn spread_is_half_the_cone() {
        assert!((spread_radians(80.0) - 80f32.to_radians() / 2.0).abs() < 1e-5);
        assert_eq!(spread_radians(360.0), std::f32::consts::PI);
    }
}
