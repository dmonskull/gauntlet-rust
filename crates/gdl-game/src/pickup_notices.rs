//! What a pickup shows and says besides its own sound (`docs/items.md`,
//! "Pickup notices"):
//!
//! - a plate with a picture of what was picked up — gold, junk, a key or
//!   a ring of them, meat, fruit, bad meat or fruit, magic, a power, a
//!   runestone, a legendary item, a crystal, a gargoyle piece — rises from
//!   under the player's panel over 80 fields, covers it for 90 and sinks
//!   back out of sight; up to 24 at once, each on its own (they slide over
//!   each other). A level's start clears them, and so does the end of the
//!   hero's death (out of the level, or up again in the tower);
//! - a runestone none of the heroes held has the announcer count the
//!   stones found: `S_RUNEFOUND1` for the first, `S_RUNE<n>` then
//!   `S_RUNEFOUND2` for 2–12, nothing at thirteen.
//!
//! The pickups send [`PickupNotice`] with the item's subtype and the value
//! the game passes (`items.rs`); the count follows the stones held.

use bevy::prelude::*;

use crate::audio::{QueueVoice, VoiceQueue};
use crate::font::{Draw2d, Flush2d, UiTextures};
use crate::frontend::Frontend;
use crate::items::ItemTick;
use crate::message_box::DrawBox;
use crate::play_camera::PlayCamera;
use crate::party::{MAX_PLAYERS, Party};
use crate::population::LevelPopulation;
use crate::quest;

pub struct PickupNoticesPlugin;

impl Plugin for PickupNoticesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PickupNotice>()
            .init_resource::<Plates>()
            .add_systems(FixedUpdate, count_runestones.after(ItemTick))
            .add_systems(PostUpdate, show_plates.before(DrawBox).before(Flush2d));
    }
}

/// A pickup to show over the player's panel: the item's subtype and the
/// value the game passes with it — gold's amount, the keys taken (or,
/// when not all fit, those left on the floor), food's health (negative
/// for poison), the number of a runestone, legendary item, crystal
/// counter or gargoyle piece; 0 for potions and powers.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickupNotice {
    /// Over whose panel.
    pub slot: usize,
    pub subtype: i32,
    pub value: i32,
}

/// The secret realm's gold is its coins.
const SECRET_REALM: u32 = 12;
/// Gold worth less than this is junk.
const JUNK_BELOW: i32 = 11;

/// The picture a pickup's plate shows (128 × 64, `STATIC`), none for
/// what gets no plate (scrolls, and subtypes 11, 12).
fn picture(subtype: i32, value: i32, secret_realm: bool) -> Option<&'static str> {
    Some(match subtype {
        1 if secret_realm => "COINHUD",
        1 if value < JUNK_BELOW => "JUNK",
        1 => "GOLD",
        2 if value < 2 => "KEY",
        2 => "KEY_RING",
        3 if value >= 100 => "MEAT",
        3 if value < -99 => "BADMEAT",
        3 if value < 0 => "BADFRUIT",
        3 => "FRUIT",
        4 => "MAGIC",
        5..=9 => "SPECIALS",
        10 => "RUNESTONE",
        13 => "LEGEND",
        15 => "CRYSTAL",
        16 => "GOLDNICON",
        _ => return None,
    })
}

/// The plate: the `S3` strip over the picture, 128 wide, at the panel's
/// x. Its top starts at the screen's bottom edge, rises to the panel's
/// top and sinks until it's this far down.
const PLATE_WIDTH: f32 = 128.0;
const STRIP: &str = "S3";
const STRIP_HEIGHT: f32 = 16.0;
const START_Y: f32 = 384.0;
const UP_Y: f32 = 304.0;
const GONE_Y: f32 = 400.0;
/// Fields it stays up.
const HOLD_FIELDS: f32 = 90.0;
/// At most this many at once; more aren't shown.
const MOST_PLATES: usize = 24;
/// Player 1's panel.
const PANEL_X: f32 = 0.0;
/// Players 2–4's panels follow player 1's this far apart.
const PANEL_WIDTH: f32 = 128.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Just made: it shows where it starts for a frame.
    New,
    Rising,
    /// Fields left.
    Up(f32),
    Sinking,
}

/// One pickup's plate.
#[derive(Clone, Debug, PartialEq)]
struct Plate {
    picture: &'static str,
    /// The top of its strip.
    y: f32,
    phase: Phase,
}

impl Plate {
    fn new(picture: &'static str) -> Self {
        Self { picture, y: START_Y, phase: Phase::New }
    }

    /// Moves it on by `fields`: after its first frame, up a pixel a field
    /// until it's up (the field it gets there, its hold starts), then its
    /// hold, then down a pixel a field. False once it's gone.
    fn step(&mut self, fields: f32) -> bool {
        match self.phase {
            Phase::New => self.phase = Phase::Rising,
            Phase::Rising => {
                self.y -= fields;
                if self.y <= UP_Y {
                    self.y = UP_Y;
                    self.phase = Phase::Up(HOLD_FIELDS);
                }
            }
            Phase::Up(left) => {
                let left = left - fields;
                self.phase = if left > 0.0 { Phase::Up(left) } else { Phase::Sinking };
            }
            Phase::Sinking => {
                self.y += fields;
                return self.y < GONE_Y;
            }
        }
        true
    }
}

/// The plates showing, oldest first.
#[derive(Resource, Default)]
struct Plates([Vec<Plate>; MAX_PLAYERS]);

/// Takes the pickups' notices and moves and draws their plates over the
/// panel (under the message box). They're cleared as a level starts and
/// as the hero's death ends (it's out of the level, or up again in the
/// tower); under a menu or off the play screen they're neither moved nor
/// drawn; they stop with play's clock (the message box) and, still drawn,
/// under the level's opening shot.
#[allow(clippy::too_many_arguments)]
fn show_plates(
    mut plates: ResMut<Plates>,
    mut notices: MessageReader<PickupNotice>,
    frontend: Option<Res<Frontend>>,
    camera: Option<Res<PlayCamera>>,
    party: Res<Party>,
    population: Option<Res<LevelPopulation>>,
    mut was_alive: Local<[bool; MAX_PLAYERS]>,
    time: Res<Time<Virtual>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
) {
    let level_start = population.as_ref().is_some_and(|p| p.is_changed());
    let out = |slot: usize| frontend.as_deref().is_some_and(|f| f.hero_out(slot));
    for (slot, plates) in plates.0.iter_mut().enumerate() {
        let alive = party.state(slot).is_some_and(|s| s.alive);
        let up_again = alive && !was_alive[slot];
        was_alive[slot] = alive;
        if out(slot) || up_again || level_start {
            plates.clear();
        }
    }
    let secret = population
        .as_ref()
        .and_then(|p| quest::level_of(&p.level))
        .is_some_and(|(realm, _)| realm == SECRET_REALM);
    for n in notices.read() {
        let Some(plates) = plates.0.get_mut(n.slot) else { continue };
        if plates.len() < MOST_PLATES
            && !out(n.slot)
            && let Some(picture) = picture(n.subtype, n.value, secret)
        {
            info!("pickup notice {picture} for player {} (subtype {}, {})", n.slot + 1, n.subtype, n.value);
            plates.push(Plate::new(picture));
        }
    }
    if frontend.as_deref().is_some_and(|f| !f.playing() || f.menu_open()) {
        return;
    }
    if !camera.as_deref().is_some_and(PlayCamera::opening) {
        let fields = time.delta_secs() * 60.0;
        for plates in &mut plates.0 {
            plates.retain_mut(|p| p.step(fields));
        }
    }
    let Some(tex) = tex.as_deref_mut() else { return };
    for (slot, plates) in plates.0.iter().enumerate() {
        let x = PANEL_X + PANEL_WIDTH * slot as f32;
        // Every strip lies behind every picture (the game's sort keys,
        // 63980 and 63979, in front of the panel's 64000).
        if let Some(strip) = tex.get(STRIP, &mut images) {
            for p in plates {
                draw.image(&strip, x, p.y, PLATE_WIDTH, STRIP_HEIGHT, Color::WHITE);
            }
        }
        for p in plates {
            if let Some(pic) = tex.get(p.picture, &mut images) {
                draw.image(&pic, x, p.y + STRIP_HEIGHT, PLATE_WIDTH, pic.size.y, Color::WHITE);
            }
        }
    }
}

/// The announcer's lines for this many stones found: the first's own
/// line, or the number and the line after it; nothing past twelve.
fn count_lines(count: u32) -> Vec<String> {
    match count {
        1 => vec!["S_RUNEFOUND1".into()],
        2..=12 => vec![format!("S_RUNE{count}"), "S_RUNEFOUND2".into()],
        _ => Vec::new(),
    }
}

/// The thirteen stones' bits.
const ALL_STONES: u32 = 0x1FFF;
/// Ticks after a level start or a new hero during which the stones held
/// are taken as they are (what a level start sets up lands then: a
/// loaded record's, the tests' `GDL_RUNES`).
const SETTLE_TICKS: u32 = 30;

/// A new runestone in play has the announcer count them, whatever the
/// wait (and not once a boss level's end has begun) — each player's.
fn count_runestones(
    party: Res<Party>,
    population: Option<Res<LevelPopulation>>,
    mut seen: Local<[Option<(u32, u32)>; MAX_PLAYERS]>,
    mut voices: MessageWriter<QueueVoice>,
) {
    let level_start = population.as_ref().is_some_and(|p| p.is_changed());
    for slot in 0..MAX_PLAYERS {
        let Some(state) = party.state(slot) else {
            seen[slot] = None;
            continue;
        };
        let bits = state.runestone_bits() & ALL_STONES;
        let (before, ticks) = match seen[slot] {
            Some((b, t)) if !level_start => (b, t + 1),
            _ => {
                seen[slot] = Some((bits, 0));
                continue;
            }
        };
        seen[slot] = Some((bits, ticks));
        if ticks < SETTLE_TICKS || bits & !before == 0 {
            continue;
        }
        let lines = count_lines(bits.count_ones());
        info!("player {}: runestone {:#x} found, {} held, the announcer says {lines:?}", slot + 1, bits & !before, bits.count_ones());
        if !lines.is_empty() {
            voices.write(QueueVoice { queue: VoiceQueue::Announcer, lines, most_wait: None, gated: true });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_by_what_was_picked_up() {
        assert_eq!(picture(1, 25, false), Some("GOLD"));
        assert_eq!(picture(1, 10, false), Some("JUNK"));
        assert_eq!(picture(1, 100, true), Some("COINHUD"));
        assert_eq!(picture(2, 1, false), Some("KEY"));
        assert_eq!(picture(2, 3, false), Some("KEY_RING"));
        assert_eq!(picture(3, 100, false), Some("MEAT"));
        assert_eq!(picture(3, 50, false), Some("FRUIT"));
        assert_eq!(picture(3, -50, false), Some("BADFRUIT"));
        assert_eq!(picture(3, -100, false), Some("BADMEAT"));
        assert_eq!(picture(4, 0, false), Some("MAGIC"));
        assert_eq!(picture(7, 0, false), Some("SPECIALS"));
        assert_eq!(picture(10, 4, false), Some("RUNESTONE"));
        assert_eq!(picture(13, 2, false), Some("LEGEND"));
        assert_eq!(picture(15, 1, false), Some("CRYSTAL"));
        assert_eq!(picture(16, 0, false), Some("GOLDNICON"));
        for s in [0, 11, 12, 14, 17] {
            assert_eq!(picture(s, 1, false), None, "{s}");
        }
    }

    #[test]
    fn a_plate_rises_holds_and_sinks() {
        let mut p = Plate::new("GOLD");
        // A frame where it starts, then 80 fields up, a pixel a field.
        assert!(p.step(2.0));
        assert_eq!((p.y, p.phase), (START_Y, Phase::Rising));
        for _ in 0..39 {
            assert!(p.step(2.0));
        }
        assert_eq!((p.y, p.phase), (306.0, Phase::Rising));
        assert!(p.step(2.0));
        assert_eq!((p.y, p.phase), (UP_Y, Phase::Up(HOLD_FIELDS)));
        // 90 fields held; then it sinks from the next field.
        for _ in 0..45 {
            assert!(p.step(2.0));
        }
        assert_eq!((p.y, p.phase), (UP_Y, Phase::Sinking));
        // 96 fields down, and gone at the bottom.
        for _ in 0..47 {
            assert!(p.step(2.0));
        }
        assert_eq!(p.y, 398.0);
        assert!(!p.step(2.0));
    }

    #[test]
    fn the_announcer_counts_the_stones() {
        assert_eq!(count_lines(1), ["S_RUNEFOUND1"]);
        assert_eq!(count_lines(2), ["S_RUNE2", "S_RUNEFOUND2"]);
        assert_eq!(count_lines(12), ["S_RUNE12", "S_RUNEFOUND2"]);
        assert!(count_lines(13).is_empty());
        assert!(count_lines(0).is_empty());
    }

    /// Every picture and count line exists (real data).
    #[test]
    fn pictures_and_lines_are_in_the_data() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let root = std::path::Path::new(&root);
        let (Ok(statics), Ok(catalog)) =
            (std::fs::read(root.join("STATIC/objects.ngc")), std::fs::read(root.join("AUDIO/AUDATPS2.ROM")))
        else {
            eprintln!("skipping: no game data");
            return;
        };
        let model = gdl_formats::ModelFile::parse(&statics).unwrap();
        let names: Vec<&str> = model.texture_names.iter().map(|t| t.name.as_str()).collect();
        for p in [
            "COINHUD", "JUNK", "GOLD", "KEY", "KEY_RING", "MEAT", "BADMEAT", "BADFRUIT", "FRUIT", "MAGIC", "SPECIALS",
            "RUNESTONE", "LEGEND", "CRYSTAL", "GOLDNICON", STRIP,
        ] {
            assert!(names.contains(&p), "{p}");
        }
        let catalog = gdl_formats::audio::AudioCatalog::parse(&catalog).unwrap();
        for n in 1..=12 {
            for line in count_lines(n) {
                assert!(catalog.find_sound(&line).is_some(), "{line}");
            }
        }
    }
}
