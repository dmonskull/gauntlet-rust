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
//! The level timer's hourglass at the top left: on a timed level (the
//! secret realm's) the part of its time gone, the sand falling once the
//! opening shot is over (`exits/secret_realm.rs`); while a hero's time
//! stop runs, the time that has left instead, with `S_HOURGLASS` looping.
//!
//! On a secret level each coin taken shows the level's coins taken and
//! all of them, under the character's coin, for 60 s.

use bevy::prelude::*;
use gdl_formats::font::{FONT_8HI, INITIALS};

use crate::audio::{CALL_VOLUME, LoopSoundAt};
use crate::critters::CritterLevel;
use crate::exits::secret_realm::{self, COIN_COUNT, COIN_COUNT_SECONDS, LevelTimer, SECRET_REALM, SecretCoins};
use crate::font::{Draw2d, GameFonts, Quad, TextStyle, UiTextures};
use crate::frontend::Frontend;
use crate::level::LoadedGame;
use crate::party::{MAX_PLAYERS, Member, Party};
use crate::play_camera::PlayCamera;
use crate::player::Player;
use crate::population::LevelPopulation;
use crate::quest;

pub struct GameHudPlugin;

impl Plugin for GameHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (draw, draw_hourglass).after(crate::shop::ShopDraw));
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

/// The level timer's hourglass — on a timed level, or while a hero's time
/// is stopped, showing the time stop's — and the time stop's looping
/// sound. Its layers go back to front as their sort keys say (frame
/// 63913, stream 63912, sand 63911: read as depths, the smaller nearer —
/// unconfirmed).
#[allow(clippy::too_many_arguments)]
fn draw_hourglass(
    frontend: Option<Res<Frontend>>,
    party: Res<Party>,
    mut game: Option<ResMut<LoadedGame>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
    mut loops: MessageWriter<LoopSoundAt>,
    players: Query<&Player>,
    mut glass: Local<Hourglass>,
    time: Res<Time<Virtual>>,
    (timer, camera): (Res<LevelTimer>, Option<Res<PlayCamera>>),
) {
    let g = &mut *glass;
    // The player whose time stop runs (the first, if several).
    let Some((slot, state)) = party.states().find(|(_, s)| s.bits.special & TIME_STOP != 0).or_else(|| party.states().next())
    else {
        return;
    };
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
    let hero = players.iter().find(|p| p.slot == slot).map(|p| Vec3::from(p.mover.position) + Vec3::Y * HERO_TOP);
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
    // The time stop's part gone (its sand always falling), else the level
    // timer's (its sand falling once the opening shot is over); the level
    // timer's sprites go when its time is up.
    let stopped = g.seen.first().filter(|_| on).map(|&(_, left)| {
        let gone = if g.total > 0.0 { ((g.total - left) / g.total).clamp(0.0, 1.0) } else { 0.0 };
        (gone, true)
    });
    let opening = camera.as_deref().is_some_and(PlayCamera::opening);
    let Some((gone, stream)) = stopped.or_else(|| timer.running().then(|| (timer.gone(), !opening))) else { return };
    if frontend.as_deref().is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    if g.textures.is_none()
        && let Some(game) = game.as_deref_mut()
    {
        g.textures = Some(UiTextures::load(&mut game.install, &[HOURGLASS_BANK]));
    }
    let Some(tex) = g.textures.as_mut() else { return };

    if let Some(frame) = tex.get("TIMER", &mut images) {
        draw.image(&frame, 1.0, 1.0, frame.size.x, frame.size.y, Color::WHITE);
    }
    let ticks = (time.elapsed_secs_f64() * 30.0) as u64;
    if stream && let Some(stream) = tex.frame("SAND_ANIM", 1 + (ticks % u64::from(SAND_FRAMES)) as u16, &mut images) {
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
pub(crate) const NOT_JOINED: [[u8; 3]; 4] = [[0x5A, 0x5A, 0x1E], [0x1E, 0x1E, 0x69], [0x64, 0x28, 0x28], [0x1E, 0x4B, 0x1E]];
/// A joined player's plain panel (out of the level): `S4` in its colour.
pub(crate) const JOINED: [[u8; 3]; 4] = [[0x78, 0x78, 0x00], [0x1E, 0x1E, 0x78], [0x78, 0x00, 0x00], [0x00, 0x64, 0x00]];
/// Each panel's width (players 2–4 follow player 1's).
const PANEL_WIDTH: f32 = 128.0;
/// Fields the key row shows for.
const KEY_ROW_FIELDS: f32 = 300.0;
/// The key row's colours: key i is lit by the boss of the realm in place i
/// of the tower's order (`quest::boss_marks`).
const KEY_COLOURS: [&str; 8] = ["BLU", "RED", "YEL", "GRE", "GRE", "RED", "YEL", "BLU"];
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

/// One panel's own counts and animations (each player's).
#[derive(Default)]
struct PanelShow {
    timers: Timers,
    shown_turbo: f32,
    meter: TurboShow,
}

#[allow(clippy::too_many_arguments)]
fn draw(
    frontend: Option<Res<Frontend>>,
    party: Res<Party>,
    (players, pads): (Query<&Player>, Query<Entity, With<Gamepad>>),
    fonts: Option<Res<GameFonts>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
    mut panels: Local<[PanelShow; MAX_PLAYERS]>,
    mut level_textures: Local<Option<UiTextures>>,
    population: Option<Res<LevelPopulation>>,
    critters: Option<Res<CritterLevel>>,
    mut game: Option<ResMut<LoadedGame>>,
    real: Res<Time<Real>>,
    game_time: Res<Time<Virtual>>,
    coins: Option<Res<SecretCoins>>,
    (options, online, free): (Res<crate::options::GameOptions>, Option<Res<crate::online::Online>>, Res<crate::camera::FreeLook>),
) {
    // The key row starts as a level starts (not in the secret realm) and
    // when a new runestone is picked up; a new pickup count starts its 3 s
    // (a coin's, 60 s).
    let mut started = false;
    if let Some(level) = population.as_ref().filter(|p| p.is_changed()) {
        started = quest::level_of(&level.level).is_none_or(|(realm, _)| realm != SECRET_REALM);
        *level_textures = None;
    }
    for (slot, state) in party.states() {
        let t = &mut panels[slot].timers;
        let runes = state.runestone_bits();
        if started || runes & !t.runes != 0 {
            t.key_row = KEY_ROW_FIELDS;
        }
        t.runes = runes;
        if let Some((_, at)) = state.popup
            && t.popup.is_none_or(|(_, seen)| at > seen)
        {
            t.popup = state.popup;
            t.popup_left = if state.popup.is_some_and(|(what, _)| what >= COIN_COUNT) { COIN_COUNT_SECONDS } else { POPUP_SECONDS };
        }
    }

    let (Some(fonts), Some(tex)) = (fonts, tex.as_deref_mut()) else { return };
    // Not on the front end's screens but the shop's (the game draws the
    // panels under it), and not under a menu (text draws over every image,
    // so the numbers would show through its panel; the key row's count
    // waits meanwhile, as the game's does under a menu).
    if frontend.as_deref().is_some_and(|f| !(f.playing() || f.in_shop()) || f.menu_open()) {
        return;
    }
    // With a pad connected that nobody plays with, the next free panel says
    // so (not the game's: our auto-detection of a new player).
    let free_pads = crate::party::free_pads(&party, pads.iter());
    let next_free = (0..MAX_PLAYERS).find(|&s| party.get(s).is_none() && !frontend.as_deref().is_some_and(|f| f.waiting(s)));
    let mut p = Painter { draw: &mut draw, tex, images: &mut images };
    let layout = personal_layout(&party, &options, online.is_some(), free.0, frontend.as_deref().is_none_or(Frontend::playing));
    let personal = layout.is_some();
    let split = layout.as_ref().is_some_and(|(_, split)| *split);
    let locals = layout.as_ref().map(|(l, _)| l.clone()).unwrap_or_default();
    for slot in 0..MAX_PLAYERS {
        // A pane of its own keeps its player's status at its foot. One
        // screen for one player (alone, or online) shows every player's
        // panel in its place, as the classic view does — but not the empty
        // ones, which would cover the weapon and say nothing.
        if personal && !locals.contains(&slot) && (split || party.get(slot).is_none()) {
            continue;
        }
        let first_quad = p.draw.quads.len();
        let first_text = p.draw.texts.len();
        let x = PANEL_X + PANEL_WIDTH * slot as f32;
        let Some(member) = party.get(slot) else {
            // A slot nobody plays: its panel waits, `S3` over `S4` in the
            // slot's dim colour, framed — a player waiting to join at the
            // tower in their colour, "IN TOWER".
            let waiting = frontend.as_deref().is_some_and(|f| f.waiting(slot));
            let [r, g, b] = if waiting { JOINED[slot] } else { NOT_JOINED[slot] };
            image(&mut p, "S3", x, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
            image(&mut p, "S4", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::srgb_u8(r, g, b));
            image(&mut p, "S4_FRAME", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
            let [r, g, b] = NUMBER_COLOUR[slot];
            let tint = Color::srgb_u8(r, g, b);
            if waiting {
                p.draw.text(&fonts, &TextStyle::new(FONT_8HI, 1.2, tint), -(x + 64.0), 340.0, "IN TOWER");
            } else if free_pads > 0 && next_free == Some(slot) {
                p.draw.text(&fonts, &TextStyle::new(FONT_8HI, 1.2, tint), -(x + 64.0), 340.0, "PRESS START");
            }
            continue;
        };
        let out = frontend.as_deref().is_some_and(|f| f.hero_out(slot));
        let turbo = players.iter().find(|h| h.slot == slot).map_or(0.0, |h| h.turbo);
        let textures = (&mut *level_textures, game.as_deref_mut(), population.as_deref());
        let intro_start = critters.as_ref().is_some_and(|c| c.intro == INTRO_START);
        let fields = (game_time.delta_secs(), real.delta_secs() * 60.0);
        let coin_need = coins.as_ref().map_or(0, |c| c.need);
        draw_panel(&mut p, &fonts, x, member, out, turbo, intro_start, &mut panels[slot], textures, fields, coin_need);
        if let Some(place) = layout.as_ref().and_then(|(locals, split)| PanelPlace::of(slot, locals, *split)) {
            place.apply(p.draw, first_quad, first_text);
        }
    }
}

/// First-person play's screens: the local players and whether they have
/// panes of their own (local co-op), or None in classic play.
pub(crate) fn personal_layout(party: &Party, options: &crate::options::GameOptions, online: bool, free: bool, playing: bool) -> Option<(Vec<usize>, bool)> {
    let locals: Vec<_> = party.members().filter(|(_, m)| !m.devices.remote).map(|(s, _)| s).collect();
    let personal = !free && playing && locals.iter().any(|&slot| options.player(if online { 0 } else { slot }).first_person);
    personal.then(|| {
        let split = !online && locals.len() > 1;
        (locals, split)
    })
}

/// Where a personal view puts a player's panel: at the foot of their own
/// pane (smaller in a two-by-two grid). What's drawn for the panel at the
/// classic layout's `PANEL_X + PANEL_WIDTH × slot` moves with it, and so
/// does that player's hint box (`hints.rs`).
pub(crate) struct PanelPlace {
    from_x: f32,
    scale: f32,
    target: Vec2,
    /// The pane's left edge and width, on the 512 × 384 screen.
    pub pane: (f32, f32),
}

impl PanelPlace {
    pub(crate) fn of(slot: usize, locals: &[usize], split: bool) -> Option<Self> {
        let index = locals.iter().position(|&s| s == slot)?;
        let (at, extent) = crate::first_person::rect(index, if split { locals.len() } else { 1 });
        let scale = if split && locals.len() > 2 { 0.75 } else { 1.0 };
        let origin = at * Vec2::new(512.0, 384.0);
        let area = extent * Vec2::new(512.0, 384.0);
        let from_x = PANEL_X + PANEL_WIDTH * slot as f32;
        // In a pane of its own, centred at its foot; on one screen, where
        // its slot's panel always is (the other players' are beside it).
        let x = if split { origin.x + (area.x - PANEL_WIDTH * scale) * 0.5 } else { from_x };
        let target = Vec2::new(x, origin.y + area.y - 384.0 * scale);
        Some(Self { from_x, scale, target, pane: (origin.x, area.x) })
    }

    /// Moves the quads drawn since `first_quad` / `first_text`.
    pub(crate) fn apply(&self, draw: &mut Draw2d, first_quad: usize, first_text: usize) {
        let from = Vec2::new(self.from_x, 0.0);
        for q in draw.quads[first_quad..].iter_mut().chain(&mut draw.texts[first_text..]) {
            match q {
                Quad::Image { pos, size, .. } => {
                    *pos = (*pos - from) * self.scale + self.target;
                    *size *= self.scale;
                }
                Quad::Text { pos, size, .. } => {
                    *pos = (*pos - from) * self.scale + self.target;
                    *size *= self.scale;
                }
            }
        }
    }
}

/// A player's panel at `x`: the runestone bar and class plate (out of the
/// level, the plain one saying "IN TOWER"), the key row or the pickup
/// count above it, the turbo meter, keys, potions, name, level, gold and
/// health.
#[allow(clippy::too_many_arguments)]
fn draw_panel(
    p: &mut Painter,
    fonts: &GameFonts,
    x: f32,
    member: &Member,
    out: bool,
    turbo: f32,
    intro_start: bool,
    show: &mut PanelShow,
    (level_textures, game, population): (&mut Option<UiTextures>, Option<&mut LoadedGame>, Option<&LevelPopulation>),
    (game_secs, real_fields): (f32, f32),
    coin_need: u32,
) {
    let (state, choice) = (&member.state, &member.choice);
    let t = &mut show.timers;
    // The pickup count shows, and counts down, only once the key row is
    // gone (and not for an out hero); the key row counts fields while it
    // shows.
    let key_row = t.key_row >= 1.0;
    let popup_shows = !key_row && !out && t.popup_left > 0.0;
    if popup_shows {
        t.popup_left -= game_secs;
    }
    if key_row {
        t.key_row = (t.key_row - game_secs * 60.0).max(0.0);
    }
    let colour = COLOURS.iter().position(|c| choice.variant.to_ascii_uppercase().starts_with(c)).unwrap_or(0);
    let tint = Color::srgb_u8(NUMBER_COLOUR[colour][0], NUMBER_COLOUR[colour][1], NUMBER_COLOUR[colour][2]);

    // The runestone bar and the class plate in its frame; out, `S3` over
    // `S4` in the player's colour, framed.
    if out {
        let [r, g, b] = JOINED[colour];
        image(p, "S3", x, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
        image(p, "S4", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::srgb_u8(r, g, b));
    } else {
        image(p, "BK_RUNE_STONE_02", x, 304.0, Some(Vec2::new(128.0, 16.0)), Color::WHITE);
        image(p, &format!("S4_{}", choice.class), x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    }
    image(p, "S4_FRAME", x, 320.0, Some(Vec2::new(128.0, 64.0)), Color::WHITE);
    // Twelve runestone slots, lit for the stones held (stone n in slot n).
    for i in 0..12usize {
        let held = !out && state.runestones.contains(&(i as i32));
        if held {
            let name = format!("SM_RUNE_{}_{:02}", RUNE_COLOURS[i / 3], i % 3 + 1);
            image(p, &name, x + 8.0 * i as f32 + (i / 3) as f32 + 15.0, 306.0, None, Color::WHITE);
        }
    }
    // The key row, for a hero still in play (behind the turbo meter): key
    // i lit for the boss beaten in place i of the tower's order.
    if key_row && state.alive {
        let beaten = quest::boss_marks(state.realms_beaten);
        for (i, colour) in KEY_COLOURS.iter().enumerate() {
            if beaten & (1 << i) != 0 {
                image(p, &format!("SM_KEY_{colour}"), x + 12.0 + 12.0 * i as f32, 300.0, None, Color::WHITE);
            }
        }
    }
    // The quest icon while the hero holds the legendary item it brought
    // to the realm's boss (there's none in the tower), the rune-13 icon
    // with the thirteenth runestone. With no health left the icons keep
    // their look and the turbo meter is hidden; the out panel has neither.
    let has_health = state.health > 0.0;
    if has_health {
        t.quest_icon = intro_start && state.quest.legendary != 0;
        t.rune_13 = state.runestones.contains(&RUNE_13);
    } else if out {
        (t.quest_icon, t.rune_13) = (false, false);
    }
    if t.rune_13 {
        image(p, "RUNE13", x + 8.0, 340.0, Some(Vec2::splat(16.0)), Color::WHITE);
    }
    // The quest icon is among the boss level's item textures, a secret
    // level's coins among the secret realm's.
    let coin = t.popup.is_some_and(|(what, _)| what >= COIN_COUNT) && popup_shows;
    if (t.quest_icon || coin)
        && level_textures.is_none()
        && let (Some(game), Some(level)) = (game, population)
    {
        *level_textures = Some(UiTextures::load(&mut game.install, &[&items_bank(&level.level)]));
    }
    if t.quest_icon
        && let Some(icon) = level_textures.as_mut().and_then(|textures| textures.get("QUEST_ICON", p.images))
    {
        p.draw.image(&icon, x + 104.0, 338.0, 16.0, 16.0, Color::WHITE);
    }

    // The turbo meter: the shown value eases toward the real one (up by
    // a field's worth per field, down twice as fast); below 40% a yellow
    // bar grows from the middle over a black one, then red over yellow.
    if has_health {
        turbo_meter(p, x, turbo.clamp(0.0, 100.0), &mut show.shown_turbo, &mut show.meter, real_fields);
    }

    // Coin and heart.
    image(p, "COIN", x + 6.0, 357.0, Some(Vec2::splat(20.0)), Color::WHITE);
    image(p, "HEART", x + 61.0, 357.0, Some(Vec2::splat(20.0)), Color::WHITE);
    // Keys and potions: an icon and a count each.
    if state.keys > 0 {
        image(p, "KEY_ICON", x + 8.0, 323.0, None, Color::WHITE);
    }
    if let Some(&kind) = state.potions.last() {
        let icon = POTION_ICONS[(kind.max(0) as usize).min(POTION_ICONS.len() - 1)];
        image(p, icon, x + 102.0, 323.0, None, Color::WHITE);
    }
    // (The `BTMBK_LEVL` plate is made off screen and hidden; only a
    // special mode shows it.)

    // The last gem or gargoyle piece: its icon and count for 3 s; a
    // secret level's coins for 60 s.
    let popup = t.popup.filter(|_| popup_shows).and_then(|(what, _)| {
        let (icon, count, need) = if what >= COIN_COUNT {
            let character = u8::try_from(what - COIN_COUNT).ok()?;
            let count = |n: u32| i16::try_from(n).unwrap_or(i16::MAX);
            return Some((secret_realm::coin_icon(character), count(state.coins), count(coin_need)));
        } else if what < 0x100 {
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
        // A coin's is among the level's item textures.
        let own = if coin { level_textures.as_mut().and_then(|textures| textures.get(icon, p.images)) } else { None };
        match own {
            Some(i) => p.draw.image(&i, x + 28.0, 288.0, 16.0, 16.0, Color::WHITE),
            None => image(p, icon, x + 28.0, 288.0, Some(Vec2::splat(16.0)), Color::WHITE),
        }
    }

    let draw = &mut *p.draw;
    let small = TextStyle::new(SCORE, 0.8, tint);
    if state.keys > 0 {
        draw.text(fonts, &small, x + 26.0, 327.0, &state.keys.to_string());
    }
    if !state.potions.is_empty() {
        draw.text(fonts, &small, x + 92.0, 327.0, &state.potions.len().to_string());
    }
    // Name (initials font, centred) and level, or out of the level "IN
    // TOWER"; gold and health right-aligned at 60 and 116.
    if out {
        draw.text(fonts, &TextStyle::new(FONT_8HI, 1.2, tint), -(x + 64.0), 340.0, "IN TOWER");
    } else {
        draw.text(fonts, &TextStyle::new(INITIALS, 0.667, tint), -(x + 64.0), 339.0, &name(&member.name));
        draw.text(fonts, &TextStyle::new(FONT_8HI, 1.0, Color::WHITE), -(x + 64.0), 326.0, &format!("LV {}", state.level));
    }
    let numbers = TextStyle::new(SCORE, 1.0, tint);
    let gold = state.gold.min(99_999).to_string();
    let w = fonts.width(SCORE, 1.0, &gold);
    draw.text(fonts, &numbers, x + 60.0 - w, 359.0, &gold);
    let health = (state.health.clamp(0.0, 9999.0) as i32).to_string();
    let w = fonts.width(SCORE, 1.0, &health);
    draw.text(fonts, &numbers, x + 116.0 - w, 359.0, &health);
    if let Some((_, count, need)) = popup {
        draw.text(fonts, &TextStyle::new(FONT_8HI, 1.5, Color::WHITE), x + 48.0, 292.0, &format!("{count}/{need}"));
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

/// How long a pick-up's count shows (a coin's, `COIN_COUNT_SECONDS`).
const POPUP_SECONDS: f32 = 3.0;

/// A level's own item bank, which has its HUD icons: a boss level's quest
/// icon, the secret realm's coins.
fn items_bank(level: &str) -> String {
    match quest::level_of(level) {
        Some((realm, _)) if realm == SECRET_REALM => secret_realm::COIN_ICON_BANK.to_string(),
        _ => format!("ITEMS/{level}"),
    }
}

struct Painter<'a> {
    draw: &'a mut Draw2d,
    tex: &'a mut UiTextures,
    images: &'a mut Assets<Image>,
}

/// The record's name has `_` for spaces.
fn name(n: &str) -> String {
    n.replace('_', " ")
}
