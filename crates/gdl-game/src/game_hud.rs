//! The in-game status panel along the bottom of the screen, drawn as the
//! game draws it (`docs/frontend.md`, "In-game HUD"): each player has a
//! 128-wide panel at the bottom of the 512 × 384 screen — the runestone
//! bar, the class plate in the player's frame, the turbo meter, the name
//! and level, keys and potions, gold and health.
//!
//! Only player 1 plays here, so only its panel is drawn.
//!
//! For 300 fields after a level starts (not in the secret realm) or a new
//! runestone is picked up, a row of eight keys over the panel, lit for the
//! realms beaten; the quest icon while the hero holds the legendary item
//! it brought to the realm's boss, the rune-13 icon once the thirteenth
//! runestone is held.
//!
//! Above the panel, for 3 s after a gem or gargoyle piece is picked up
//! (counted only while the key row is gone), its icon and "count/needed"
//! (the needed count once its realm or wing has opened; `docs/items.md`).
//!
//! Once the hero's death is over outside the tower it is out of the level
//! until the level ends: its panel is the plain one in its colour, with
//! "IN TOWER", gold and health.
//!
//! While the hero's time stop runs, the level timer's hourglass at the
//! top left shows the time it has left, and `S_HOURGLASS` loops.
//!
//! Stand-in: the secret realm's coin count isn't drawn.

use bevy::prelude::*;
use gdl_formats::font::{FONT_8HI, INITIALS};

use crate::audio::{CALL_VOLUME, LoopSoundAt};
use crate::critters::CritterLevel;
use crate::font::{Draw2d, GameFonts, Quad, TextStyle, UiTextures};
use crate::frontend::Frontend;
use crate::level::LoadedGame;
use crate::player::{Player, PlayerChoice};
use crate::player_state::PlayerState;
use crate::population::LevelPopulation;
use crate::quest;

pub struct GameHudPlugin;

impl Plugin for GameHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (draw, draw_hourglass));
    }
}

/// The time stop's hourglass (`docs/powers.md`, "`0x8` time stop"): the
/// level timer's sprites from `POWERUPS` — the frame, the sand left above
/// and fallen below (two windows on `TIMER_SAND`) and the falling stream
/// (`SAND_ANIM`, five frames a tick apart) — with the time the first slot
/// holding bit `0x8` has left, out of what that slot had when last
/// granted.
const TIME_STOP: u32 = 0x8;
const HOURGLASS_BANK: &str = "POWERUPS";
const HOURGLASS_SOUND: &str = "S_HOURGLASS";
const HOURGLASS_CHANNEL: &str = "hourglass";
/// The hero's top point above its feet (`PDAT +0x50`, every class), where
/// the hourglass's sound is.
const HERO_TOP: f32 = 4.4;
/// The sand above: texture rows from 23 + 41 × the part gone to 64, drawn
/// from y 24 + 39 × it; the sand below: rows from 105 − 38 × it to 128,
/// from y 106 − 38 × it (each 1 texel a pixel, 128 wide at x 1).
const SAND_TOP: (f32, f32, f32, f32) = (23.0, 41.0, 64.0, 39.0);
const SAND_BOTTOM: (f32, f32, f32) = (105.0, 38.0, 128.0);
/// The stream's place and its frames.
const SAND_STREAM: Vec2 = Vec2::new(63.0, 58.0);
const SAND_FRAMES: u16 = 5;

/// What the hourglass keeps between frames: the bank's textures, the
/// time-stop slots' times last seen (a rise is a new grant), the time
/// out of which the part gone is measured, and whether it's showing.
#[derive(Default)]
struct Hourglass {
    textures: Option<UiTextures>,
    seen: Vec<((i32, u32), f32)>,
    total: f32,
    on: bool,
}

/// The level timer's hourglass while the hero's time is stopped, and its
/// looping sound. Its layers go back to front as their sort keys say
/// (frame 63913, stream 63912, sand 63911: read as depths, the smaller
/// nearer — unconfirmed).
#[allow(clippy::too_many_arguments)]
fn draw_hourglass(
    frontend: Option<Res<Frontend>>,
    state: Res<PlayerState>,
    mut game: Option<ResMut<LoadedGame>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
    mut loops: MessageWriter<LoopSoundAt>,
    players: Query<&Player>,
    mut glass: Local<Hourglass>,
    time: Res<Time<Virtual>>,
) {
    let g = &mut *glass;
    // A grant (or top-up) of a power with bit 8 sets the time it's
    // measured out of, as the game's grant does.
    let slots: Vec<((i32, u32), f32)> =
        state.active_powers().filter(|p| p.value & TIME_STOP != 0).map(|p| ((p.subtype, p.value), p.time)).collect();
    for (key, t) in &slots {
        if g.seen.iter().find(|(k, _)| k == key).is_none_or(|(_, before)| t > before) {
            g.total = *t;
        }
    }
    g.seen = slots;
    let on = state.bits.special & TIME_STOP != 0;
    // Its sound follows the hero (at its top point) while it's on.
    let hero = players.iter().next().map(|p| Vec3::from(p.mover.position) + Vec3::Y * HERO_TOP);
    match hero.filter(|_| on) {
        Some(at) => {
            loops.write(LoopSoundAt::at(HOURGLASS_CHANNEL, HOURGLASS_SOUND, at, CALL_VOLUME));
        }
        None if g.on => {
            loops.write(LoopSoundAt::stop(HOURGLASS_CHANNEL));
        }
        None => {}
    }
    g.on = on;
    let Some(&(_, left)) = g.seen.first() else { return };
    if !on || frontend.as_deref().is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    if g.textures.is_none()
        && let Some(game) = game.as_deref_mut()
    {
        g.textures = Some(UiTextures::load(&mut game.install, &[HOURGLASS_BANK]));
    }
    let Some(tex) = g.textures.as_mut() else { return };
    let gone = if g.total > 0.0 { ((g.total - left) / g.total).clamp(0.0, 1.0) } else { 0.0 };

    if let Some(frame) = tex.get("TIMER", &mut images) {
        draw.image(&frame, 1.0, 1.0, frame.size.x, frame.size.y, Color::WHITE);
    }
    let ticks = (time.elapsed_secs_f64() * 30.0) as u64;
    if let Some(stream) = tex.frame("SAND_ANIM", 1 + (ticks % u64::from(SAND_FRAMES)) as u16, &mut images) {
        draw.image(&stream, SAND_STREAM.x, SAND_STREAM.y, stream.size.x, stream.size.y, Color::WHITE);
    }
    if let Some(sand) = tex.get("TIMER_SAND", &mut images) {
        let (top_from, top_span, top_to, top_shift) = SAND_TOP;
        let shift = (top_shift * gone).round();
        let from = top_from + top_span * gone;
        let top = Rect::new(0.0, from, sand.size.x, top_to);
        let top_at = Vec2::new(1.0, top_from + 1.0 + shift);
        let top_size = Vec2::new(sand.size.x, top_span - shift);
        let (bottom_from, bottom_span, bottom_to) = SAND_BOTTOM;
        let shift = (bottom_span * gone).round();
        let bottom = Rect::new(0.0, bottom_from - bottom_span * gone, sand.size.x, bottom_to);
        let bottom_at = Vec2::new(1.0, bottom_from + 1.0 - shift);
        let bottom_size = Vec2::new(sand.size.x, bottom_to - bottom_from + shift);
        for (rect, pos, size) in [(top, top_at, top_size), (bottom, bottom_at, bottom_size)] {
            if size.y > 0.0 {
                draw.quads.push(Quad::Image { image: sand.handle.clone(), rect: Some(rect), pos, size, color: Color::WHITE });
            }
        }
    }
}

/// Player 1's panel: x of its left edge (players 2–4 at 128, 256, 384).
const PANEL_X: f32 = 0.0;
/// The score font (digits only).
const SCORE: usize = 4;
/// Name and number colours by the player's colour: yellow, blue, red,
/// green, pale.
const NUMBER_COLOUR: [[u8; 3]; 4] = [[255, 255, 128], [135, 206, 235], [255, 192, 224], [128, 255, 128]];
/// Potion icons by the potion's kind.
const POTION_ICONS: [&str; 5] = ["POTION_ICON_RED", "POTION_ICON_RED", "POTION_ICON_BLU", "POTION_ICON_YEL", "POTION_ICON_GRE"];
/// Runestone slot colours, three stones each.
const RUNE_COLOURS: [&str; 4] = ["BLU", "RED", "YEL", "GRE"];
const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
/// A panel waiting for its player: `S4` in the slot's dim colour.
const NOT_JOINED: [[u8; 3]; 4] = [[0x5A, 0x5A, 0x1E], [0x1E, 0x1E, 0x69], [0x64, 0x28, 0x28], [0x1E, 0x4B, 0x1E]];
/// A joined player's plain panel (out of the level): `S4` in its colour.
const JOINED: [[u8; 3]; 4] = [[0x78, 0x78, 0x00], [0x1E, 0x1E, 0x78], [0x78, 0x00, 0x00], [0x00, 0x64, 0x00]];
/// Each panel's width (players 2–4 follow player 1's).
const PANEL_WIDTH: f32 = 128.0;
/// Fields the key row shows for.
const KEY_ROW_FIELDS: f32 = 300.0;
/// The key row's colours: key i is lit by the boss of the realm in place i
/// of the tower's order (`quest::boss_marks`).
const KEY_COLOURS: [&str; 8] = ["BLU", "RED", "YEL", "GRE", "GRE", "RED", "YEL", "BLU"];
/// Its levels start without the key row.
const SECRET_REALM: u32 = 12;
/// The thirteenth runestone (`RUNEE1`).
const RUNE_13: i32 = 12;
/// The boss intro's first state (`critters.rs`): the hero who brought the
/// realm's legendary item still holds it.
const INTRO_START: i32 = 1;

/// The key row's count and the pickup count's, and the icons' last look.
#[derive(Default)]
struct Timers {
    /// Fields the key row still shows for.
    key_row: f32,
    /// The runestones held last frame: a new one starts the key row.
    runes: u32,
    /// The latest pickup count (what, when), and the seconds it still
    /// shows for.
    popup: Option<(u16, f32)>,
    popup_left: f32,
    /// The quest and rune-13 icons (they keep their look while the hero
    /// has no health).
    quest_icon: bool,
    rune_13: bool,
    /// The level's own item textures, where `QUEST_ICON` is (read when the
    /// icon first shows in a level).
    level_textures: Option<UiTextures>,
}

/// What the turbo meter plays (`docs/frontend.md`, "In-game HUD").
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum TurboState {
    #[default]
    Plain,
    /// A gleam at the bar's right after it moves into another band.
    Gleam,
    /// The full bar's glow.
    Glow,
}

/// The turbo meter's animation, and the bars' last look (they hold it
/// while an animation plays).
#[derive(Default)]
struct TurboShow {
    state: TurboState,
    /// Fields since the animation began.
    timer: f32,
    fraction: f32,
    fill: [u8; 3],
    under: [u8; 3],
}

/// The meter's band: 1 below 40%, 2 below 99%, 3 full.
fn turbo_band(shown: f32) -> u8 {
    let f = shown * 0.01;
    if f < 0.4 {
        1
    } else if f < 0.99 {
        2
    } else {
        3
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    frontend: Option<Res<Frontend>>,
    state: Res<PlayerState>,
    choice: Res<PlayerChoice>,
    players: Query<&Player>,
    fonts: Option<Res<GameFonts>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
    mut shown_turbo: Local<f32>,
    mut meter: Local<TurboShow>,
    mut timers: Local<Timers>,
    population: Option<Res<LevelPopulation>>,
    critters: Option<Res<CritterLevel>>,
    mut game: Option<ResMut<LoadedGame>>,
    real: Res<Time<Real>>,
    game_time: Res<Time<Virtual>>,
) {
    // The key row starts as a level starts (not in the secret realm) and
    // when a new runestone is picked up; a new pickup count starts its 3 s.
    let t = &mut *timers;
    let mut started = false;
    if let Some(level) = population.as_ref().filter(|p| p.is_changed()) {
        started = quest::level_of(&level.level).is_none_or(|(realm, _)| realm != SECRET_REALM);
        t.level_textures = None;
    }
    let runes = state.runestone_bits();
    if started || runes & !t.runes != 0 {
        t.key_row = KEY_ROW_FIELDS;
    }
    t.runes = runes;
    if let Some((_, at)) = state.popup
        && t.popup.is_none_or(|(_, seen)| at > seen)
    {
        t.popup = state.popup;
        t.popup_left = POPUP_SECONDS;
    }

    let (Some(fonts), Some(tex)) = (fonts, tex.as_deref_mut()) else { return };
    // Not on the front end's screens, and not under a menu (text draws
    // over every image, so the numbers would show through its panel; the
    // key row's count waits meanwhile, as the game's does under a menu).
    if frontend.as_deref().is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    // The hero out of the level: its panel is set up afresh, the plain
    // one in its joined colour, and says "IN TOWER".
    let out = frontend.as_deref().is_some_and(Frontend::hero_out);
    // The pickup count shows, and counts down, only once the key row is
    // gone (and not for an out hero); the key row counts fields while it
    // shows.
    let key_row = t.key_row >= 1.0;
    let popup_shows = !key_row && !out && t.popup_left > 0.0;
    if popup_shows {
        t.popup_left -= game_time.delta_secs();
    }
    if key_row {
        t.key_row = (t.key_row - game_time.delta_secs() * 60.0).max(0.0);
    }
    let x = PANEL_X;
    let colour = COLOURS.iter().position(|c| choice.variant.to_ascii_uppercase().starts_with(c)).unwrap_or(0);
    let tint = Color::srgb_u8(NUMBER_COLOUR[colour][0], NUMBER_COLOUR[colour][1], NUMBER_COLOUR[colour][2]);
    let mut p = Painter { draw: &mut draw, tex, images: &mut images };

    // Players 2–4 aren't in (one player): their panels wait, `S3` over
    // `S4` in the slot's dim colour, framed.
    for (slot, [r, g, b]) in NOT_JOINED.iter().copied().enumerate().skip(1) {
        let px = PANEL_X + PANEL_WIDTH * slot as f32;
        image(&mut p, "S3", px, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
        image(&mut p, "S4", px, 320.0, Some(Vec2::new(128.0, 64.0)), Color::srgb_u8(r, g, b));
        image(&mut p, "S4_FRAME", px, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    }

    // The runestone bar and the class plate in its frame; out, `S3` over
    // `S4` in the player's colour, framed.
    if out {
        let [r, g, b] = JOINED[colour];
        image(&mut p, "S3", x, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
        image(&mut p, "S4", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::srgb_u8(r, g, b));
    } else {
        image(&mut p, "BK_RUNE_STONE_02", x, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
        image(&mut p, &format!("S4_{}", choice.class), x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    }
    image(&mut p, "S4_FRAME", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    // Twelve runestone slots, lit for the stones held (stone n in slot n).
    for i in 0..12usize {
        let held = !out && state.runestones.contains(&(i as i32));
        if held {
            let name = format!("SM_RUNE_{}_{:02}", RUNE_COLOURS[i / 3], i % 3 + 1);
            image(&mut p, &name, x + 8.0 * i as f32 + (i / 3) as f32 + 15.0, 306.0, None, Color::WHITE);
        }
    }
    // The key row, for a hero still in play (behind the turbo meter): key
    // i lit for the boss beaten in place i of the tower's order.
    if key_row && state.alive {
        let beaten = quest::boss_marks(state.realms_beaten);
        for (i, colour) in KEY_COLOURS.iter().enumerate() {
            if beaten & (1 << i) != 0 {
                image(&mut p, &format!("SM_KEY_{colour}"), x + 12.0 + 12.0 * i as f32, 300.0, None, Color::WHITE);
            }
        }
    }
    // The quest icon while the hero holds the legendary item it brought
    // to the realm's boss (there's none in the tower), the rune-13 icon
    // with the thirteenth runestone. With no health left the icons keep
    // their look and the turbo meter is hidden; the out panel has neither.
    let has_health = state.health > 0.0;
    if has_health {
        t.quest_icon = critters.as_ref().is_some_and(|c| c.intro == INTRO_START);
        t.rune_13 = state.runestones.contains(&RUNE_13);
    } else if out {
        (t.quest_icon, t.rune_13) = (false, false);
    }
    if t.rune_13 {
        image(&mut p, "RUNE13", x + 8.0, 340.0, Some(Vec2::splat(16.0)), Color::WHITE);
    }
    if t.quest_icon {
        // The icon is among the boss level's item textures.
        if t.level_textures.is_none()
            && let (Some(game), Some(level)) = (game.as_deref_mut(), population.as_ref())
        {
            t.level_textures = Some(UiTextures::load(&mut game.install, &[&format!("ITEMS/{}", level.level)]));
        }
        if let Some(icon) = t.level_textures.as_mut().and_then(|textures| textures.get("QUEST_ICON", p.images)) {
            p.draw.image(&icon, x + 104.0, 338.0, 16.0, 16.0, Color::WHITE);
        }
    }

    // The turbo meter: the shown value eases toward the real one (up by
    // a field's worth per field, down twice as fast); below 40% a yellow
    // bar grows from the middle over a black one, then red over yellow.
    if has_health {
        let target = players.iter().next().map_or(0.0, |p| p.turbo).clamp(0.0, 100.0);
        turbo_meter(&mut p, x, target, &mut shown_turbo, &mut meter, real.delta_secs() * 60.0);
    }

    // Coin and heart.
    image(&mut p, "COIN", x + 6.0, 357.0, Some(Vec2::splat(20.0)), Color::WHITE);
    image(&mut p, "HEART", x + 61.0, 357.0, Some(Vec2::splat(20.0)), Color::WHITE);
    // Keys and potions: an icon and a count each.
    if state.keys > 0 {
        image(&mut p, "KEY_ICON", x + 8.0, 323.0, None, Color::WHITE);
    }
    if let Some(&kind) = state.potions.last() {
        let icon = POTION_ICONS[(kind.max(0) as usize).min(POTION_ICONS.len() - 1)];
        image(&mut p, icon, x + 102.0, 323.0, None, Color::WHITE);
    }
    // (The `BTMBK_LEVL` plate is made off screen and hidden; only a
    // special mode shows it.)

    // The last gem or gargoyle piece: its icon and count for 3 s.
    let popup = t.popup.filter(|_| popup_shows).and_then(|(what, _)| {
        let (icon, count, need) = if what < 0x100 {
            let c = usize::from(what);
            let need = *quest::CRYSTALS_NEEDED.get(c)?;
            // The art names the colours by three letters (`SM_CRYSTAL_ORA`);
            // the game asks for the whole word, which never matches.
            let colour: String = quest::CRYSTAL_COLOURS[c].chars().take(3).collect();
            (format!("SM_CRYSTAL_{colour}"), state.quest.crystals[c], need)
        } else {
            let g = usize::from(what - 0x100);
            let need = *quest::GARGOYLE_NEEDED.get(g)?;
            (format!("SM_{}", ["FANGS", "FEATHERS", "CLAWS"][g]), state.quest.gargoyle[g], need)
        };
        // An opened counter shows what it needed.
        Some((icon, if count < 0 { need } else { count }, need))
    });
    if let Some((icon, _, _)) = &popup {
        image(&mut p, icon, x + 28.0, 288.0, Some(Vec2::splat(16.0)), Color::WHITE);
    }

    let draw = p.draw;
    let small = TextStyle::new(SCORE, 0.8, tint);
    if state.keys > 0 {
        draw.text(&fonts, &small, x + 26.0, 327.0, &state.keys.to_string());
    }
    if !state.potions.is_empty() {
        draw.text(&fonts, &small, x + 92.0, 327.0, &state.potions.len().to_string());
    }
    // Name (initials font, centred) and level, or out of the level "IN
    // TOWER"; gold and health right-aligned at 60 and 116.
    if out {
        draw.text(&fonts, &TextStyle::new(FONT_8HI, 1.2, tint), -(x + 64.0), 340.0, "IN TOWER");
    } else {
        draw.text(&fonts, &TextStyle::new(INITIALS, 0.667, tint), -(x + 64.0), 339.0, &name(&frontend_name(frontend.as_deref())));
        draw.text(&fonts, &TextStyle::new(FONT_8HI, 1.0, Color::WHITE), -(x + 64.0), 326.0, &format!("LV {}", state.level));
    }
    let numbers = TextStyle::new(SCORE, 1.0, tint);
    let gold = state.gold.min(99_999).to_string();
    let w = fonts.width(SCORE, 1.0, &gold);
    draw.text(&fonts, &numbers, x + 60.0 - w, 359.0, &gold);
    let health = (state.health.clamp(0.0, 9999.0) as i32).to_string();
    let w = fonts.width(SCORE, 1.0, &health);
    draw.text(&fonts, &numbers, x + 116.0 - w, 359.0, &health);
    if let Some((_, count, need)) = popup {
        draw.text(&fonts, &TextStyle::new(FONT_8HI, 1.5, Color::WHITE), x + 48.0, 292.0, &format!("{count}/{need}"));
    }
}

/// Draws `name` at `(px, py)`, stretched to `size` (its own size if none).
fn image(p: &mut Painter, name: &str, px: f32, py: f32, size: Option<Vec2>, c: Color) {
    if let Some(i) = p.tex.get(name, p.images) {
        let s = size.unwrap_or(i.size);
        p.draw.image(&i, px, py, s.x, s.y, c);
    }
}

/// The turbo meter of the panel at `x`, easing its shown value toward
/// `target` over `fields`.
fn turbo_meter(p: &mut Painter, x: f32, target: f32, shown_turbo: &mut f32, m: &mut TurboShow, fields: f32) {
    let before = turbo_band(*shown_turbo);
    *shown_turbo = if *shown_turbo < target {
        (*shown_turbo + fields).min(target)
    } else {
        (*shown_turbo - 2.0 * fields).max(target)
    };
    let after = turbo_band(*shown_turbo);
    // Moving into another band gleams; full, the bar glows, over and over.
    if before != after || before == 3 && after == 3 && m.state == TurboState::Plain {
        m.state = if after == 3 { TurboState::Glow } else { TurboState::Gleam };
        m.timer = 0.0;
    }
    // The bars only change while neither plays; glowing, both are red.
    if m.state == TurboState::Plain {
        let f = *shown_turbo * 0.01;
        let v = |fraction: f32| (127.0 * fraction + 128.0) as u8;
        (m.fraction, m.fill, m.under) = if f < 0.4 {
            let fr = f / 0.4;
            (fr, [v(fr), v(fr), 0], [0, 0, 0])
        } else if f < 0.99 {
            let fr = (f - 0.4) / 0.6;
            (fr, [v(fr), 0, 0], [255, 255, 0])
        } else {
            (1.0, [255, 0, 0], [255, 255, 0])
        };
    }
    let (fill, under) = if m.state == TurboState::Glow { ([255, 0, 0], [255, 0, 0]) } else { (m.fill, m.under) };
    let rgb = |c: [u8; 3]| Color::srgb_u8(c[0], c[1], c[2]);
    if let Some(bar) = p.tex.get("TRBO_FULL_NEW", p.images) {
        p.draw.image(&bar, x, 304.0, bar.size.x, bar.size.y, rgb(under));
        let half = ((bar.size.x * m.fraction) as i32 >> 1).max(1) as f32;
        p.draw.image(&bar, x + bar.size.x / 2.0 - half, 304.0, 2.0 * half, bar.size.y, rgb(fill));
    }
    // The glint streak always lies over the bar.
    image(p, "TRBO_GLINT", x, 304.0, None, Color::WHITE);
    match m.state {
        // The gleam: frames 1–5 and back, 4 fields each.
        TurboState::Gleam => {
            let mut frame = (m.timer as i32) >> 2;
            if (5..10).contains(&frame) {
                frame = 4 - (frame - 5);
            }
            if frame < 5 {
                image(p, &format!("TRBO_GLEEM{}", frame + 1), x + 80.0, 310.0, None, Color::WHITE);
            } else {
                m.state = TurboState::Plain;
            }
        }
        // The glow fades out and back in over 120 fields.
        TurboState::Glow => {
            let mut fade = ((m.timer as i32) << 9) / 120;
            if (0x100..0x200).contains(&fade) {
                fade = 0x1FF - fade;
            }
            if fade < 0x100 {
                let alpha = (0xFF - fade) as u8;
                image(p, "TURBO_GLOW_NEW", x, 304.0, None, Color::srgba_u8(255, 255, 255, alpha));
            } else {
                m.timer = 0.0;
            }
        }
        TurboState::Plain => {}
    }
    m.timer += fields;
}

/// How long a pick-up's count shows.
const POPUP_SECONDS: f32 = 3.0;

struct Painter<'a> {
    draw: &'a mut Draw2d,
    tex: &'a mut UiTextures,
    images: &'a mut Assets<Image>,
}

fn frontend_name(frontend: Option<&Frontend>) -> String {
    frontend.map(|f| f.hero_name.clone()).unwrap_or_default()
}

/// The record's name has `_` for spaces.
fn name(n: &str) -> String {
    n.replace('_', " ")
}
