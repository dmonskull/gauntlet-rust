//! The lag sign: all the screen shows of the network online. A small
//! medallion in the top right corner, clear of the panels — a red gem
//! sending out three golden waves, lit one after another — while the game
//! waits on another machine or the machines put their games together
//! again (`resync.rs`). Ours, not the game's (which had no network): it's
//! painted here, in the gold and gems of the game's own panels.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::font::{Draw2d, Flush2d, UiImage};
use crate::online::Lockstep;
use crate::resync::Resync;

pub struct LagSignPlugin;

impl Plugin for LagSignPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, make).add_systems(PostUpdate, show.before(Flush2d));
    }
}

/// The sign's textures are this many pixels square.
const SIZE: u32 = 96;
/// Where it's drawn on the game's 512 × 384 screen, and how big.
const AT: Vec2 = Vec2::new(474.0, 8.0);
const SHOWN_SIZE: f32 = 30.0;
/// The game has waited this long on the network before it shows.
const LAG_SHOWN: f32 = 0.25;
/// It stays this long after a sync point.
const SYNC_SHOWN: f32 = 1.2;
/// Seconds it takes to come and to go, and each wave to light.
const FADE_IN: f32 = 0.12;
const FADE_OUT: f32 = 0.45;
const WAVE_SECONDS: f32 = 0.22;

/// The gem's place and the waves' radii round it, with the sign's square
/// running −1 to 1.
const GEM: Vec2 = Vec2::new(0.0, 0.42);
const GEM_RADIUS: f32 = 0.14;
const WAVES: [f32; 3] = [0.34, 0.57, 0.80];
const WAVE_HALF: f32 = 0.072;
/// Each wave spans this far either side of straight up (its cosine).
const WAVE_SPAN: f32 = 0.669;

#[derive(Resource)]
struct LagSign {
    plate: UiImage,
    waves: [UiImage; 3],
    /// How far it has come in, 0–1, and seconds it has shown.
    shown: f32,
    turn: f32,
}

fn make(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut image = |paint: &dyn Fn(Vec2) -> [f32; 4]| {
        let image = images.add(painted(paint));
        UiImage { handle: image, size: Vec2::splat(SIZE as f32) }
    };
    let plate = image(&plate);
    let waves = [0, 1, 2].map(|k| image(&|p| lit_wave(p, WAVES[k])));
    commands.insert_resource(LagSign { plate, waves, shown: 0.0, turn: 0.0 });
}

/// Shows the sign while the game lags: it comes quickly, goes slowly, and
/// its waves light in turn, out from the gem.
fn show(
    time: Res<Time<Real>>,
    lock: Res<Lockstep>,
    resync: Option<Res<Resync>>,
    fe: Option<Res<crate::frontend::Frontend>>,
    sign: Option<ResMut<LagSign>>,
    mut d: ResMut<Draw2d>,
    mut forced: Local<Option<bool>>,
) {
    let Some(mut sign) = sign else { return };
    // `GDL_LAG_SIGN=1` (testing): always shown.
    let forced = *forced.get_or_insert_with(|| std::env::var("GDL_LAG_SIGN").is_ok_and(|v| !v.is_empty() && v != "0"));
    let syncing = lock.sync.is_some() || resync.is_some_and(|r| r.under_way() || r.since.is_some_and(|s| s < SYNC_SHOWN));
    let lagging = lock.on && fe.is_some_and(|f| f.playing()) && (lock.waited > LAG_SHOWN || syncing);
    let dt = time.delta_secs();
    sign.shown = if lagging || forced { (sign.shown + dt / FADE_IN).min(1.0) } else { (sign.shown - dt / FADE_OUT).max(0.0) };
    if sign.shown <= 0.0 {
        sign.turn = 0.0;
        return;
    }
    sign.turn += dt;
    let white = |alpha: f32| Color::srgba(1.0, 1.0, 1.0, alpha.clamp(0.0, 1.0));
    d.image(&sign.plate, AT.x, AT.y, SHOWN_SIZE, SHOWN_SIZE, white(sign.shown));
    // The waves light one after another, hold, and start over.
    let beat = (sign.turn / WAVE_SECONDS) % 5.0;
    for (k, wave) in sign.waves.iter().enumerate() {
        let lit = (beat - k as f32).clamp(0.0, 1.0) * (4.6 - beat).clamp(0.0, 1.0);
        if lit > 0.0 {
            d.image(wave, AT.x, AT.y, SHOWN_SIZE, SHOWN_SIZE, white(sign.shown * lit));
        }
    }
}

/// A texture from a colour for each point of the square (−1 to 1 each
/// way, y down), red, green, blue and opacity 0–1.
fn painted(paint: &dyn Fn(Vec2) -> [f32; 4]) -> Image {
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / (SIZE as f32 * 0.5) - Vec2::ONE;
            pixels.extend(paint(p).map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
        }
    }
    let mut image = Image::new(
        Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// A pixel's width in the square's units: edges are softened over it.
const SOFT: f32 = 2.0 / SIZE as f32;

/// How much of a pixel at signed distance `d` from an edge is inside it
/// (negative inside).
fn cover(d: f32) -> f32 {
    (0.5 - d / SOFT).clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// `top` laid over `under` with opacity `alpha`.
fn over(under: [f32; 4], top: [f32; 3], alpha: f32) -> [f32; 4] {
    let a = alpha + under[3] * (1.0 - alpha);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let c: [f32; 3] = std::array::from_fn(|i| (top[i] * alpha + under[i] * under[3] * (1.0 - alpha)) / a);
    [c[0], c[1], c[2], a]
}

const OUTLINE: [f32; 3] = [0.10, 0.05, 0.02];
const GOLD_DARK: [f32; 3] = [0.45, 0.27, 0.06];
const GOLD: [f32; 3] = [0.86, 0.62, 0.16];
const GOLD_BRIGHT: [f32; 3] = [1.0, 0.92, 0.55];
const BRONZE: [f32; 3] = [0.30, 0.19, 0.08];

/// Signed distance from `p` to wave `radius`'s stroke: an arc round the
/// gem, straight up and [`WAVE_SPAN`] either side, with round ends. Also
/// how far across the stroke `p` is (−1 its inner edge, 1 its outer).
fn wave(p: Vec2, radius: f32) -> (f32, f32) {
    let v = p - GEM;
    let length = v.length().max(1e-6);
    let up = -v.y / length;
    if up >= WAVE_SPAN {
        let across = (length - radius) / WAVE_HALF;
        return ((length - radius).abs() - WAVE_HALF, across);
    }
    // Past an end: round its tip.
    let side = (1.0 - WAVE_SPAN * WAVE_SPAN).sqrt();
    let tip = GEM + Vec2::new(side * v.x.signum(), -WAVE_SPAN) * radius;
    (p.distance(tip) - WAVE_HALF, (length - radius) / WAVE_HALF)
}

/// The medallion: a gold rim round a dark field, the gem in its bezel and
/// the three waves unlit.
fn plate(p: Vec2) -> [f32; 4] {
    let r = p.length();
    let mut c = [0.0; 4];
    // Lit from the top left, as the game's panels are.
    let light = (-(p.x * 0.6 + p.y * 0.8) / r.max(1e-6)).clamp(-1.0, 1.0);
    // The field, darker toward its foot.
    c = over(c, mix([0.13, 0.09, 0.22], [0.04, 0.03, 0.08], p.y * 0.5 + 0.5), cover(r - 0.76) * 0.94);
    // The rim: dark edges either side of bevelled gold.
    c = over(c, OUTLINE, cover(r - 0.97) * cover(0.72 - r));
    let bevel = 0.5 + 0.5 * light * (1.0 - ((r - 0.845) / 0.065).abs()).clamp(0.2, 1.0);
    let gold = if bevel < 0.5 { mix(GOLD_DARK, GOLD, bevel * 2.0) } else { mix(GOLD, GOLD_BRIGHT, bevel * 2.0 - 1.0) };
    c = over(c, gold, cover(r - 0.92) * cover(0.77 - r));
    // The waves, unlit: dull bronze with a dark edge.
    for radius in WAVES {
        let (d, across) = wave(p, radius);
        c = over(c, OUTLINE, cover(d - 0.035));
        c = over(c, mix(BRONZE, [0.42, 0.28, 0.12], across * 0.5 + 0.5), cover(d));
    }
    // The gem: a gold bezel, and red glass with a glint.
    let g = p.distance(GEM);
    c = over(c, OUTLINE, cover(g - GEM_RADIUS - 0.095));
    let around = ((GEM - p).dot(Vec2::new(0.6, 0.8)) / g.max(1e-6)).clamp(-1.0, 1.0);
    c = over(c, mix(GOLD_DARK, GOLD_BRIGHT, 0.5 + 0.5 * around), cover(g - GEM_RADIUS - 0.06));
    c = over(c, OUTLINE, cover(g - GEM_RADIUS - 0.012));
    let depth = (g / GEM_RADIUS).clamp(0.0, 1.0);
    c = over(c, mix([1.0, 0.25, 0.18], [0.45, 0.02, 0.04], depth * depth), cover(g - GEM_RADIUS + 0.012));
    let glint = p.distance(GEM + Vec2::new(-0.045, -0.05));
    over(c, [1.0, 0.95, 0.9], cover(glint - 0.035) * 0.9)
}

/// One wave lit: bright bevelled gold, and its glow.
fn lit_wave(p: Vec2, radius: f32) -> [f32; 4] {
    let (d, across) = wave(p, radius);
    let glow = (1.0 - d / 0.11).clamp(0.0, 1.0);
    let c = over([0.0; 4], [1.0, 0.72, 0.25], glow * glow * 0.55);
    over(c, mix(GOLD, GOLD_BRIGHT, across * 0.5 + 0.5), cover(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign is a round plate: clear at its corners, solid at its
    /// middle, with the gem red and each wave where it should be.
    #[test]
    fn the_sign_is_a_medallion_with_a_gem_and_three_waves() {
        assert_eq!(plate(Vec2::new(-0.99, -0.99))[3], 0.0);
        assert!(plate(Vec2::ZERO)[3] > 0.9);
        let gem = plate(GEM);
        assert!(gem[0] > 0.8 && gem[1] < 0.4, "{gem:?}");
        for radius in WAVES {
            // Straight up from the gem, on the wave; and between the waves, off it.
            assert!(lit_wave(GEM - Vec2::Y * radius, radius)[3] > 0.99);
            assert!(wave(GEM - Vec2::Y * (radius + 0.115), radius).0 > 0.0);
            // Beside the gem, off its ends.
            assert!(wave(GEM + Vec2::X * radius, radius).0 > 0.0);
        }
        // The biggest wave stays inside the field.
        let tip = GEM + Vec2::new((1.0 - WAVE_SPAN * WAVE_SPAN).sqrt(), -WAVE_SPAN) * WAVES[2];
        assert!(tip.length() + WAVE_HALF < 0.72);
    }
}
