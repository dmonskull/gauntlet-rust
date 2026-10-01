//! The critters' health meters (`docs/critters.md`, "The health meters"),
//! made with the critter as the game's critter set-up makes them:
//!
//! - a type with flag 4 (every boss; the chimera's three heads, not its
//!   body) gets a meter across the top of the 2D screen — at most three a
//!   level: its sprites (`TYPE +0xF8`, two on the disc) side by side, 256
//!   wide, at y 8 from its start; with flag 8 a back (`<name>_METER_BG<n>`)
//!   at the level's next start, which moves on by `TYPE +0xFA`; its fill
//!   (`<name>_METER_FG<n>`) over the last back made (the chimera's lion and
//!   snake share the eagle's). Every sprite is drawn at transparency 0x70.
//!   Each tick the hit points shown move toward the critter's by 3 a video
//!   field; the fill runs from its left margin (`TYPE +0xFC`) across both
//!   sprites to its right one (`+0xFE`). While the level's boss is frozen
//!   (the dragon's intro) the backs are tinted red. All go as the boss's
//!   body goes (the victory);
//! - a type with flag `0x800` (the golems and gargoyles) gets the 3D
//!   `GMETER` from its own folder on its root at `TYPE +0x100`, turned
//!   about the vertical to face the screen (the game's facing mode
//!   `0x2000000`): a glass tube with a red filler whose length (its
//!   `RED_FILLE` node's scale across) is the hit points left; it goes as
//!   the critter's last hit point does.
//!
//! The sprites come from the critter's own folder (`MONSTERS/DRAGON`:
//! `METER_BG1`… 256 × 64).

use bevy::prelude::*;
use gdl_formats::critter::CritterType;

use super::{Critter, CritterLevel};
use crate::character::{Animate, Animator};
use crate::font::{Draw2d, Quad, UiTextures};
use crate::frontend::Frontend;
use crate::level::LoadedGame;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(FixedUpdate, fill.after(super::tick_critters))
        .add_systems(Update, (draw.after(crate::shop::ShopDraw), fill_solid.after(Animate)));
}

/// `TYPE +0x5C` flags: a 2D meter, with a back; the 3D meter.
pub(super) const FLAT: u32 = 0x4;
pub(super) const BACK: u32 = 0x8;
pub(super) const SOLID: u32 = 0x800;

/// At most this many 2D meters a level (the game logs "Too many health
/// meters" past it).
const MOST: usize = 3;
/// Each sprite is this wide (the meter's fill is measured in its texels,
/// one a screen unit); they stand side by side at this height.
const SPRITE: f32 = 256.0;
const TOP: f32 = 8.0;
/// The hit points shown move toward the real ones by this many a video
/// field, two fields a tick.
const CHASE_PER_FIELD: f32 = 3.0;
const FIELDS_PER_TICK: f32 = 2.0;
/// Transparency 0x70 on every sprite: alpha 0x80 − 0x70 / 2 of 0x80.
const ALPHA: f32 = (0x80 - 0x70 / 2) as f32 / 128.0;
/// The backs' colour while the boss is frozen: the game's `0xFF8080FF`,
/// each channel halved plus one, of 0x80.
const FROZEN: [f32; 3] = [1.0, 65.0 / 128.0, 65.0 / 128.0];
/// The 3D meter's filler node.
const FILLER: &str = "RED_FILLE";
/// The 3D meter's atree in the critter's folder.
pub(super) const SOLID_MODEL: &str = "GMETER";

/// One 2D meter.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Meter {
    /// Its sprites' names start with this (`EAGLE_`, or nothing for a type
    /// with no name of its own).
    prefix: String,
    sprites: usize,
    /// The fill's margins at the left of the first sprite and the right of
    /// the last (texels).
    left: f32,
    right: f32,
    /// It has a back of its own.
    back: bool,
    /// Where its sprites start: its fill's, and its back's.
    start: f32,
    /// Hit points: when it was made (full), and shown now.
    full: f32,
    shown: f32,
}

/// A level's 2D meters (the game's three slots and its two running starts),
/// the folder their sprites come from, and whether they're gone.
#[derive(Debug, Default)]
pub(super) struct Meters {
    meters: Vec<Meter>,
    /// Where the next back goes, and where the last one went (fills
    /// without a back of their own go there).
    next: f32,
    last: f32,
    pub(super) folder: String,
    pub(super) hidden: bool,
}

impl Meters {
    /// A critter of type `t` is made with `hp` hit points: its 2D meter if
    /// its type has one and there's room, by number.
    pub(super) fn add(&mut self, t: &CritterType, hp: f32) -> Option<usize> {
        if t.flags & FLAT == 0 {
            return None;
        }
        if self.meters.len() >= MOST {
            warn!("too many health meters: {}", self.meters.len());
            return None;
        }
        let back = t.flags & BACK != 0;
        if back {
            self.last = self.next;
            self.next += f32::from(t.meter[1]);
        }
        let prefix = if t.name.is_empty() { String::new() } else { format!("{}_", t.name) };
        self.meters.push(Meter {
            prefix,
            sprites: usize::try_from(t.meter[0]).unwrap_or(0),
            left: f32::from(t.meter[2]),
            right: f32::from(t.meter[3]),
            back,
            start: self.last,
            full: hp,
            shown: hp,
        });
        Some(self.meters.len() - 1)
    }

    /// A tick: meter `i`'s shown hit points move toward `hp`.
    pub(super) fn step(&mut self, i: usize, hp: f32) {
        if let Some(m) = self.meters.get_mut(i) {
            m.shown = chase(m.shown, hp, CHASE_PER_FIELD * FIELDS_PER_TICK);
        }
    }
}

/// The shown hit points after a tick: toward `hp` (never below 0) by at
/// most `step`.
fn chase(shown: f32, hp: f32, step: f32) -> f32 {
    let hp = hp.max(0.0);
    if shown < hp { (shown + step).min(hp) } else { (shown - step).max(hp) }
}

/// How much of each fill sprite shows, in texels from its left (the
/// screen width it's drawn at, and a 256th of its texture per texel):
/// `None` for all of it. Two sprites share the fill: the first from its
/// left margin to its end, then the second from its start to its right
/// margin. With one sprite, the part between the margins by the share left
/// (as the game measures it, from the sprite's left).
fn fills(m: &Meter) -> Vec<Option<f32>> {
    let share = if m.full > 0.0 { m.shown / m.full } else { 0.0 };
    if m.sprites == 2 {
        let f = 2.0 * share;
        let first = (f < 1.0).then(|| (f * (SPRITE - m.left) + m.left).trunc());
        let second = if f - 1.0 > 0.0 { ((f - 1.0) * (SPRITE - m.right)).trunc() } else { 0.0 };
        vec![first, Some(second)]
    } else {
        let w = if share > 0.0 { (share * (SPRITE - (m.left + m.right))).trunc() } else { 0.0 };
        vec![Some(w)]
    }
}

/// The meters' hit points, each tick, from the critters they were made
/// for (the body's update steps its own and its parts').
fn fill(level: Option<ResMut<CritterLevel>>, critters: Query<&Critter>) {
    let Some(mut level) = level else { return };
    let level = &mut *level;
    for c in &critters {
        for k in std::iter::once(c).chain(c.parts.iter()) {
            if let Some(i) = k.meter {
                level.meters.step(i, k.hit_points);
            }
        }
    }
}

/// The sprites, with the HUD (in play, under no menu).
fn draw(
    level: Option<Res<CritterLevel>>,
    critters: Query<&Critter>,
    frontend: Option<Res<Frontend>>,
    mut game: Option<ResMut<LoadedGame>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
    mut textures: Local<Option<(String, UiTextures)>>,
) {
    let Some(level) = level else { return };
    let meters = &level.meters;
    if meters.hidden || meters.meters.is_empty() || frontend.as_deref().is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    if textures.as_ref().is_none_or(|(f, _)| *f != meters.folder)
        && let Some(game) = game.as_deref_mut()
    {
        *textures = Some((meters.folder.clone(), UiTextures::load(&mut game.install, &[meters.folder.as_str()])));
    }
    let Some((_, tex)) = textures.as_mut() else { return };
    let frozen = level.boss.and_then(|b| critters.get(b).ok()).is_some_and(|c| c.frozen > 0.0);
    let back = if frozen { Color::srgba(FROZEN[0], FROZEN[1], FROZEN[2], ALPHA) } else { Color::srgba(1.0, 1.0, 1.0, ALPHA) };
    let fill = Color::srgba(1.0, 1.0, 1.0, ALPHA);
    // Backs first, then fills, in the order they were made.
    for m in meters.meters.iter().filter(|m| m.back) {
        for n in 0..m.sprites {
            let Some(img) = tex.get(&format!("{}METER_BG{}", m.prefix, n + 1), &mut images) else { continue };
            draw.image(&img, m.start + SPRITE * n as f32, TOP, img.size.x, img.size.y, back);
        }
    }
    for m in &meters.meters {
        for (n, shows) in fills(m).into_iter().enumerate() {
            let Some(img) = tex.get(&format!("{}METER_FG{}", m.prefix, n + 1), &mut images) else { continue };
            let x = m.start + SPRITE * n as f32;
            match shows {
                None => draw.image(&img, x, TOP, img.size.x, img.size.y, fill),
                // An empty one draws nothing (the game keeps its last width
                // over the texture's first column: at most a texel).
                Some(w) if w <= 0.0 => {}
                Some(w) => draw.quads.push(Quad::Image {
                    image: img.handle.clone(),
                    rect: Some(Rect::new(0.0, 0.0, img.size.x * w / SPRITE, img.size.y)),
                    pos: Vec2::new(x, TOP),
                    size: Vec2::new(w, img.size.y),
                    color: fill,
                }),
            }
        }
    }
}

/// A golem's or gargoyle's 3D meter, on its critter's root.
#[derive(Component)]
pub(super) struct SolidMeter {
    critter: Entity,
}

/// Puts the 3D meter on a critter just made (its root entity `root`, its
/// type `t`), facing the screen.
pub(super) fn add_solid(model: &crate::character::CharacterModel, root: Entity, t: &CritterType, commands: &mut Commands) {
    let meter = model.spawn(Transform::from_translation(Vec3::from(t.meter_offset)), commands);
    commands.entity(meter).insert((SolidMeter { critter: root }, crate::billboard::Billboard::ScreenYaw, ChildOf(root)));
}

/// Each frame (after the pose): the filler's length across is the hit
/// points left; at none the meter goes.
fn fill_solid(
    mut commands: Commands,
    meters: Query<(Entity, &SolidMeter, &Animator)>,
    critters: Query<&Critter>,
    mut bones: Query<&mut Transform>,
) {
    for (e, m, animator) in &meters {
        let Ok(c) = critters.get(m.critter) else { continue };
        if c.hit_points <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let share = if c.full_hit_points > 0.0 { c.hit_points / c.full_hit_points } else { 0.0 };
        if let Some(mut t) = animator.node(FILLER).and_then(|n| animator.bone(n)).and_then(|b| bones.get_mut(b).ok()) {
            t.scale = Vec3::new(share, 1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(name: &str, flags: u32, meter: [i16; 4]) -> CritterType {
        let mut raw = vec![0u8; 0x140];
        raw[..name.len()].copy_from_slice(name.as_bytes());
        raw[0x5C..0x60].copy_from_slice(&flags.to_le_bytes());
        for (i, v) in meter.iter().enumerate() {
            raw[0xF8 + 2 * i..0xFA + 2 * i].copy_from_slice(&v.to_le_bytes());
        }
        CritterType::parse(&raw)
    }

    /// The chimera's heads: the eagle's back and fill at the start, the
    /// lion's and snake's fills over it; no fourth.
    #[test]
    fn heads_share_the_first_back() {
        let mut m = Meters::default();
        assert_eq!(m.add(&ty("", 0x82, [2, 256, 49, 68]), 6000.0), None);
        assert_eq!(m.add(&ty("EAGLE", 0xE, [2, 256, 49, 68]), 1200.0), Some(0));
        assert_eq!(m.add(&ty("LION", 0x6, [2, 256, 49, 68]), 1500.0), Some(1));
        assert_eq!(m.add(&ty("SNAKE", 0x6, [2, 256, 49, 68]), 1200.0), Some(2));
        assert_eq!(m.add(&ty("MORE", 0xE, [2, 256, 49, 68]), 1.0), None);
        assert_eq!(m.meters.iter().map(|x| (x.prefix.as_str(), x.back, x.start)).collect::<Vec<_>>(), [
            ("EAGLE_", true, 0.0),
            ("LION_", false, 0.0),
            ("SNAKE_", false, 0.0)
        ]);
        // A second meter with a back would start 256 on.
        let mut two = Meters::default();
        two.add(&ty("", 0xE, [2, 256, 46, 72]), 2000.0);
        two.add(&ty("", 0xE, [2, 256, 46, 72]), 2000.0);
        assert_eq!(two.meters[1].start, 256.0);
        assert_eq!(two.meters[0].prefix, "");
    }

    /// The shown hit points move 6 a tick toward the real ones (not below
    /// 0), and the fill runs from the left margin over both sprites to the
    /// right one.
    #[test]
    fn the_fill_follows_the_hit_points() {
        let mut m = Meters::default();
        m.add(&ty("", 0x1E, [2, 256, 46, 72]), 2000.0);
        let fill = |m: &Meters| fills(&m.meters[0]);
        // Full: the first sprite whole, the second to its right margin.
        assert_eq!(fill(&m), [None, Some(184.0)]);
        m.step(0, 1900.0);
        assert_eq!(m.meters[0].shown, 1994.0);
        for _ in 0..20 {
            m.step(0, 1900.0);
        }
        assert_eq!(m.meters[0].shown, 1900.0);
        m.meters[0].shown = 1000.0;
        assert_eq!(fill(&m), [None, Some(0.0)]);
        m.meters[0].shown = 500.0;
        assert_eq!(fill(&m), [Some(151.0), Some(0.0)]);
        m.step(0, -50.0);
        assert_eq!(m.meters[0].shown, 494.0);
        m.meters[0].shown = 0.0;
        assert_eq!(fill(&m), [Some(46.0), Some(0.0)]);
        assert_eq!(chase(3.0, -10.0, 6.0), 0.0);
        assert_eq!(chase(10.0, 20.0, 6.0), 16.0);
    }
}
