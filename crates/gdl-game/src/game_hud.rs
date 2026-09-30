//! The in-game status panel along the bottom of the screen, drawn as the
//! game draws it (`docs/frontend.md`, "In-game HUD"): each player has a
//! 128-wide panel at the bottom of the 512 × 384 screen — the runestone
//! bar, the class plate in the player's frame, the turbo meter, the name
//! and level, keys and potions, gold and health.
//!
//! Only player 1 plays here, so only its panel is drawn.
//!
//! Above the panel, for 3 s after a gem or gargoyle piece is picked up,
//! its icon and "count/needed" (the needed count once its realm or wing
//! has opened; `docs/items.md`).
//!
//! Stand-ins: the legendary key row shown for the first 300 fields of a
//! level, the quest and rune-13 icons, the secret realm's coin count and
//! the dead player's "Wait In Tower" / "Quit Game" prompt aren't drawn.

use bevy::prelude::*;
use gdl_formats::font::{FONT_8HI, INITIALS};

use crate::font::{Draw2d, GameFonts, TextStyle, UiTextures};
use crate::frontend::Frontend;
use crate::player::{Player, PlayerChoice};
use crate::player_state::PlayerState;
use crate::quest;

pub struct GameHudPlugin;

impl Plugin for GameHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, draw);
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
/// Each panel's width (players 2–4 follow player 1's).
const PANEL_WIDTH: f32 = 128.0;

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
    real: Res<Time<Real>>,
    game_time: Res<Time<Virtual>>,
) {
    let (Some(fonts), Some(tex)) = (fonts, tex.as_deref_mut()) else { return };
    // Not on the front end's screens, and not under a menu (text draws
    // over every image, so the numbers would show through its panel).
    if frontend.as_deref().is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    let x = PANEL_X;
    let colour = COLOURS.iter().position(|c| choice.variant.to_ascii_uppercase().starts_with(c)).unwrap_or(0);
    let tint = Color::srgb_u8(NUMBER_COLOUR[colour][0], NUMBER_COLOUR[colour][1], NUMBER_COLOUR[colour][2]);
    let mut p = Painter { draw: &mut draw, tex, images: &mut images };
    let image = |p: &mut Painter, name: &str, px: f32, py: f32, size: Option<Vec2>, c: Color| {
        if let Some(i) = p.tex.get(name, p.images) {
            let s = size.unwrap_or(i.size);
            p.draw.image(&i, px, py, s.x, s.y, c);
        }
    };

    // Players 2–4 aren't in (one player): their panels wait, `S3` over
    // `S4` in the slot's dim colour, framed.
    for (slot, [r, g, b]) in NOT_JOINED.iter().copied().enumerate().skip(1) {
        let px = PANEL_X + PANEL_WIDTH * slot as f32;
        image(&mut p, "S3", px, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
        image(&mut p, "S4", px, 320.0, Some(Vec2::new(128.0, 64.0)), Color::srgb_u8(r, g, b));
        image(&mut p, "S4_FRAME", px, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    }

    // The runestone bar and the class plate in its frame.
    image(&mut p, "BK_RUNE_STONE_02", x, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
    image(&mut p, &format!("S4_{}", choice.class), x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    image(&mut p, "S4_FRAME", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    // Twelve runestone slots, lit for the stones held (stone n in slot n).
    for i in 0..12usize {
        let held = state.runestones.contains(&(i as i32));
        if held {
            let name = format!("SM_RUNE_{}_{:02}", RUNE_COLOURS[i / 3], i % 3 + 1);
            image(&mut p, &name, x + 8.0 * i as f32 + (i / 3) as f32 + 15.0, 306.0, None, Color::WHITE);
        }
    }

    // The turbo meter: the shown value eases toward the real one (up by
    // a field's worth per field, down twice as fast); below 40% a yellow
    // bar grows from the middle over a black one, then red over yellow.
    let target = players.iter().next().map_or(0.0, |p| p.turbo).clamp(0.0, 100.0);
    let fields = real.delta_secs() * 60.0;
    let before = turbo_band(*shown_turbo);
    *shown_turbo = if *shown_turbo < target {
        (*shown_turbo + fields).min(target)
    } else {
        (*shown_turbo - 2.0 * fields).max(target)
    };
    let after = turbo_band(*shown_turbo);
    // Moving into another band gleams; full, the bar glows, over and over.
    let m = &mut *meter;
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
    image(&mut p, "TRBO_GLINT", x, 304.0, None, Color::WHITE);
    match m.state {
        // The gleam: frames 1–5 and back, 4 fields each.
        TurboState::Gleam => {
            let mut frame = (m.timer as i32) >> 2;
            if (5..10).contains(&frame) {
                frame = 4 - (frame - 5);
            }
            if frame < 5 {
                image(&mut p, &format!("TRBO_GLEEM{}", frame + 1), x + 80.0, 310.0, None, Color::WHITE);
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
                image(&mut p, "TURBO_GLOW_NEW", x, 304.0, None, Color::srgba_u8(255, 255, 255, alpha));
            } else {
                m.timer = 0.0;
            }
        }
        TurboState::Plain => {}
    }
    m.timer += fields;

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
    let popup = state.popup.filter(|&(_, at)| game_time.elapsed_secs() - at < POPUP_SECONDS).and_then(|(what, _)| {
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
    // Name (initials font, centred), level, gold and health right-aligned
    // at 60 and 116.
    draw.text(&fonts, &TextStyle::new(INITIALS, 0.667, tint), -(x + 64.0), 339.0, &name(&frontend_name(frontend.as_deref())));
    draw.text(&fonts, &TextStyle::new(FONT_8HI, 1.0, Color::WHITE), -(x + 64.0), 326.0, &format!("LV {}", state.level));
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
