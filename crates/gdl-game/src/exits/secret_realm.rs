//! The secret realm (`docs/items.md`, "The secret realm"): nine timed
//! levels, S1–S9, each reached through one realm level's secret exit.
//! They have no exits of their own: the level timer ends them.
//!
//! - **The timer.** A level whose record is flagged timed (the secret
//!   realm's nine) starts its timer at the record's seconds + 0.99 as it
//!   loads. It runs on play's clock — not under a menu or the message box
//!   — once the level's opening shot is over, and the hourglass at the top
//!   left shows the part gone (`game_hud.rs`). Each whole second it
//!   crosses ticks the clock (`S_SECRETCLOCK1` on even seconds,
//!   `S_SECRETCLOCK2` on odd ones, `S_SECRETCLOCKEN` on the last); the
//!   announcer says `S_TIMEISRUNNING` at 8 and counts `S_COUNT5` …
//!   `S_COUNT0`.
//! - **Out of time** the hourglass goes, every hero in play goes out at
//!   once (as through a secret exit, `items.rs`) and the party goes back
//!   to the level whose secret exit it took — the secret level counts as
//!   finished. That level comes back as they left it ([`KeptLevel`]): the
//!   first hero stands where the exit was taken, what was picked up,
//!   broken, destroyed or let out stays gone, opened doors stay open, and
//!   the secret exit is gone.
//! - **The coins.** A secret level's gold items are its secret
//!   character's coins (S1's jackal …). Each one taken counts for every
//!   hero in play, and the HUD shows the character's coin and
//!   "taken/all" for 60 s. The last one unlocks the character for every
//!   hero in play (`PlayerState::secret_characters`, saved with the
//!   character), queues `S_SECRETCHAR`, opens `ALLCOINS` in the message
//!   box and leaves a second on the timer, its seconds no longer heard.
//!
//! Stand-ins: started on a secret level without a secret exit (`--level
//! levelS1`), its timer sends the party to the tower. On the way back the
//! level's movers and levers start afresh and so does the camera (the game
//! keeps their states), and the heroes face as the first one did; the
//! opening shot plays and the level start saves the heroes' records
//! unless `play_camera.rs` and `frontend.rs` ask
//! [`SecretReturn::coming_back_to`] (the game does neither).

use std::collections::HashMap;

use bevy::prelude::*;
use gdl_formats::Population;
use gdl_formats::population::LocatorKind;

use crate::audio::{CALL_VOLUME, PlaySoundAt, QueueVoice};
use crate::exits::LevelIds;
use crate::generators::{Generator, PlacedMonsters};
use crate::items::{GoOut, ItemTick, LevelItems};
use crate::message_box::ShowMessage;
use crate::party::Party;
use crate::play_camera::PlayCamera;
use crate::player::{Player, PlayerTick};
use crate::population::LevelPopulation;

/// The secret realm's id.
pub const SECRET_REALM: u32 = 12;

pub struct SecretRealmPlugin;

impl Plugin for SecretRealmPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CoinTaken>()
            .add_message::<SecretExitTaken>()
            .init_resource::<LevelClocks>()
            .init_resource::<LevelTimer>()
            .init_resource::<SecretCoins>()
            .init_resource::<SecretReturn>()
            .add_systems(
                Update,
                start_level.after(crate::items::build_items).run_if(resource_exists_and_changed::<LevelPopulation>),
            )
            .add_systems(PreUpdate, restore_level)
            .add_systems(
                FixedUpdate,
                (run_timer.before(PlayerTick), (keep_level, count_coins).after(ItemTick)),
            );
    }
}

// ---------------------------------------------------------------------------
// The timer

/// A timed level's timer starts this much over its record's seconds (so
/// the first whole second it crosses is the record's less one).
const START_EXTRA: f64 = 0.99;
/// With more time left than the record's seconds and this, the timer is
/// set to [`OVER_TIME`] — something the disc's levels never meet.
const OVER_RECORD: f32 = 1.0;
const OVER_TIME: f32 = 5.0;
/// The time the last coin leaves.
const LAST_COIN_LEFT: f32 = 1.0;
/// The announcer's warning second, and the seconds below which he counts.
const WARNING_SECOND: i32 = 8;
const COUNT_BELOW: i32 = 6;
/// The clock's ticks play at the call's own volume, the announcer's
/// warning and count louder, all centred.
const CLOCK_VOLUME: u8 = CALL_VOLUME;
const COUNT_VOLUME: u8 = 0xE0;

/// Each timed level's record seconds, by lower-case folder (the realm
/// WADs' level records, read with the level ids, `exits.rs`).
#[derive(Resource, Clone, Debug, Default)]
pub struct LevelClocks(pub HashMap<String, i16>);

/// What a tick of the timer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerStep {
    /// Nothing to hear.
    Running,
    /// A whole second crossed: the one it's in now.
    Second(i32),
    /// The time ran out this tick.
    Up,
}

/// The level timer: its time (the record's seconds + 0.99) and what's
/// left, seconds. Nothing on an untimed level.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct LevelTimer {
    /// The level record's seconds, on a timed level.
    pub seconds: Option<i16>,
    pub total: f32,
    pub left: f32,
    /// The last coin is taken: its seconds go unheard.
    pub quiet: bool,
    /// The time has run out (the hourglass is gone).
    pub up: bool,
}

impl LevelTimer {
    /// A level's timer, from its record's seconds (none: untimed).
    pub fn new(seconds: Option<i16>) -> Self {
        // Summed in double precision, as the game does.
        let time = seconds.map_or(0.0, |s| (f64::from(s) + START_EXTRA) as f32);
        Self { seconds, total: time, left: time, quiet: false, up: false }
    }

    /// Timed and not yet out of time: the hourglass shows.
    pub fn running(&self) -> bool {
        self.seconds.is_some() && !self.up
    }

    /// Runs `dt` seconds off: the second crossed (unless the last coin has
    /// quietened it), or the time running out. Single precision, whole
    /// seconds by truncation, as the game counts them.
    pub fn step(&mut self, dt: f32) -> TimerStep {
        let Some(seconds) = self.seconds.filter(|_| !self.up) else { return TimerStep::Running };
        if f32::from(seconds) + OVER_RECORD < self.left {
            (self.total, self.left) = (OVER_TIME, OVER_TIME);
        }
        let before = self.left;
        self.left = before - dt;
        if self.left > 0.0 {
            let now = self.left as i32;
            if !self.quiet && before as i32 != now {
                return TimerStep::Second(now);
            }
            return TimerStep::Running;
        }
        self.left = 0.0;
        self.up = true;
        TimerStep::Up
    }

    /// The last coin taken: a second left, and none of it heard.
    pub fn last_coin(&mut self) {
        if self.running() {
            self.left = LAST_COIN_LEFT;
            self.quiet = true;
        }
    }

    /// The part of its time gone, 0–1 (the hourglass's sand).
    pub fn gone(&self) -> f32 {
        if self.total > 0.0 { ((self.total - self.left) / self.total).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// The sounds of a second crossed: the clock's tick, and the announcer's
/// warning at 8 or count below 6 — the count only while no secret exit
/// is being taken.
pub fn second_sounds(second: i32, exit_taken: bool) -> (&'static str, Option<String>) {
    let clock = match second {
        0 => "S_SECRETCLOCKEN",
        s if s & 1 == 0 => "S_SECRETCLOCK1",
        _ => "S_SECRETCLOCK2",
    };
    let voice = match second {
        WARNING_SECOND => Some("S_TIMEISRUNNING".to_string()),
        s if (0..COUNT_BELOW).contains(&s) && !exit_taken => Some(format!("S_COUNT{s}")),
        _ => None,
    };
    (clock, voice)
}

/// A new level: its timer and coins, every hero's coin count back to 0,
/// and the level left behind forgotten once the party is elsewhere.
#[allow(clippy::too_many_arguments)]
fn start_level(
    population: Res<LevelPopulation>,
    clocks: Res<LevelClocks>,
    items: Res<LevelItems>,
    ids: Option<Res<LevelIds>>,
    mut timer: ResMut<LevelTimer>,
    mut coins: ResMut<SecretCoins>,
    mut back: ResMut<SecretReturn>,
    mut party: ResMut<Party>,
) {
    let level = population.level.to_ascii_lowercase();
    *timer = LevelTimer::new(clocks.0.get(&level).copied());
    let id = ids.as_deref().and_then(|ids| ids.id(&population.level)).or_else(|| crate::quest::level_of(&population.level));
    *coins = SecretCoins {
        need: items.gold_items() as u32,
        character: id.filter(|&(realm, _)| realm == SECRET_REALM).and_then(|(_, index)| secret_character(index)),
    };
    for (_, state) in party.states_mut() {
        state.coins = 0;
    }
    if back.back_in.as_ref().is_some_and(|l| !l.eq_ignore_ascii_case(&population.level)) {
        back.back_in = None;
    }
    if let Some(seconds) = timer.seconds {
        info!("{}: timed, {seconds} s; {} coins of {:?}", population.level, coins.need, coins.character.map(class_code));
    }
}

/// The timer's tick (before the heroes', as the game's comes first): it
/// waits for the opening shot, then runs; each second crossed is heard,
/// and when the time is up the heroes still standing go out, back to the
/// level whose secret exit they took.
#[allow(clippy::too_many_arguments)]
fn run_timer(
    time: Res<Time>,
    mut timer: ResMut<LevelTimer>,
    camera: Option<Res<PlayCamera>>,
    items: Res<LevelItems>,
    party: Res<Party>,
    population: Option<Res<LevelPopulation>>,
    mut back: ResMut<SecretReturn>,
    mut sounds: MessageWriter<PlaySoundAt>,
    mut go: MessageWriter<GoOut>,
) {
    if !timer.running() || camera.as_deref().is_some_and(PlayCamera::opening) {
        return;
    }
    match timer.step(time.delta_secs()) {
        TimerStep::Running => {}
        TimerStep::Second(second) => {
            let (clock, voice) = second_sounds(second, items.leaving());
            sounds.write(PlaySoundAt::centred(clock, CLOCK_VOLUME));
            if let Some(voice) = voice {
                sounds.write(PlaySoundAt::centred(voice, COUNT_VOLUME));
            }
        }
        TimerStep::Up => {
            // Nobody standing: the last hero's death takes the party to
            // the tower (`frontend.rs`).
            if !party.any(|s| s.alive) {
                return;
            }
            let here = population.as_ref().map(|p| p.level.clone()).unwrap_or_default();
            let to = back.time_up(&here);
            info!("{here}: out of time, back to {to}");
            go.write(GoOut { to });
        }
    }
}

// ---------------------------------------------------------------------------
// The coins

/// Each secret level's character, by the level's index in the realm's
/// records (S1 … S9): the jackal, tigress, falconess, minotaur, sumner,
/// ogre, hyena, medusa and unicorn — classes 8–16.
const SECRET_CHARACTERS: [u8; 9] = [10, 11, 9, 8, 16, 12, 15, 14, 13];
/// Class codes in the game's class order.
const CLASSES: [&str; 17] =
    ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR", "UNI", "MED", "HYE", "SUM"];
/// The first class a character has to unlock.
const FIRST_SECRET_CLASS: u8 = 8;
/// The sixteenth's coin icon is its own name; the rest are `16_<class>COIN`.
const SUMNER: u8 = 16;
/// The HUD's count for a coin: this + the character (`PlayerState::popup`).
pub const COIN_COUNT: u16 = 0x200;
/// How long the coin count shows after each coin (a gem's, 3 s).
pub const COIN_COUNT_SECONDS: f32 = 60.0;
/// The bank with the coin icons: the secret realm's items.
pub const COIN_ICON_BANK: &str = "ITEMS/levelS";
/// The last coin's line (the announcer's, dropped past a second's wait)
/// and message.
const SECRET_CHAR_LINE: &str = "S_SECRETCHAR";
const SECRET_CHAR_WAIT: f32 = 1.0;
const ALL_COINS: &str = "ALLCOINS";

/// The level's coins: its gold items as it was built, and whose they are.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecretCoins {
    pub need: u32,
    /// The secret character (a class, 8–16), on a secret level.
    pub character: Option<u8>,
}

/// A coin taken (gold on a secret level): it counts for every hero in
/// play, whoever took it.
#[derive(Message, Clone, Copy, Debug)]
pub struct CoinTaken;

/// The secret level with this index's character.
pub fn secret_character(index: u32) -> Option<u8> {
    SECRET_CHARACTERS.get(index as usize).copied()
}

/// A class's three-letter code.
pub fn class_code(class: u8) -> &'static str {
    CLASSES.get(usize::from(class)).copied().unwrap_or("")
}

/// The HUD's icon for a character's coins (`16_JACCOIN`; the sumner's
/// `16_SUM`).
pub fn coin_icon(character: u8) -> String {
    if character >= SUMNER { "16_SUM".to_string() } else { format!("16_{}COIN", class_code(character)) }
}

/// A character's bit among the unlocked (none for the first eight).
pub fn unlock_bit(character: u8) -> u16 {
    character.checked_sub(FIRST_SECRET_CLASS).filter(|&b| b < 16).map_or(0, |b| 1 << b)
}

/// Whether a class can be picked with these characters unlocked: the
/// first eight always (for the select screen, `frontend.rs`, which is yet
/// to ask).
#[allow(dead_code)]
pub fn class_open(class: usize, unlocked: u16) -> bool {
    class < usize::from(FIRST_SECRET_CLASS) || u8::try_from(class).is_ok_and(|c| unlocked & unlock_bit(c) != 0)
}

/// One coin taken: every hero in play counts it. Whether one has them
/// all now (the level's need).
pub fn count_coin<'a>(counts: impl IntoIterator<Item = &'a mut u32>, need: u32) -> bool {
    let mut all = false;
    for count in counts {
        *count += 1;
        all |= *count >= need;
    }
    all
}

/// The coins taken this tick: counted for every hero in play, with the
/// HUD's count; the last one unlocks the character, says so and leaves
/// a second on the timer.
#[allow(clippy::too_many_arguments)]
fn count_coins(
    mut taken: MessageReader<CoinTaken>,
    coins: Res<SecretCoins>,
    mut party: ResMut<Party>,
    mut timer: ResMut<LevelTimer>,
    time: Res<Time>,
    mut voices: MessageWriter<QueueVoice>,
    mut messages: MessageWriter<ShowMessage>,
) {
    for _ in taken.read() {
        let Some(character) = coins.character else { continue };
        let now = time.elapsed_secs();
        let mut playing: Vec<_> = party.states_mut().filter(|(_, s)| s.alive).map(|(_, s)| s).collect();
        for state in playing.iter_mut() {
            state.popup = Some((COIN_COUNT + u16::from(character), now));
        }
        if !count_coin(playing.iter_mut().map(|s| &mut s.coins), coins.need) {
            continue;
        }
        for state in playing.iter_mut() {
            state.secret_characters |= unlock_bit(character);
        }
        info!("all {} coins: the {} is unlocked", coins.need, class_code(character));
        voices.write(QueueVoice::announcer(SECRET_CHAR_LINE, SECRET_CHAR_WAIT).gated());
        messages.write(ShowMessage::new(ALL_COINS, 0).stopping(SECRET_CHAR_LINE));
        timer.last_coin();
    }
}

// ---------------------------------------------------------------------------
// The way back

/// A secret exit taken (`items.rs`): where the hero touching it stood and
/// faced, and the level's items as the party leaves — the placements gone
/// (the exit with them) and the doors open.
#[derive(Message, Clone, Debug, Default, PartialEq)]
pub struct SecretExitTaken {
    pub at: [f32; 3],
    pub facing: f32,
    pub gone: Vec<usize>,
    pub opened: Vec<usize>,
}

/// A level left through its secret exit, as it was left: where the
/// secret realm's timer sends the party back.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeptLevel {
    /// Its folder.
    pub level: String,
    pub at: [f32; 3],
    pub facing: f32,
    /// Placements gone (`items.rs` frees them as the level builds).
    pub gone: Vec<usize>,
    /// Doors opened (they open again).
    pub opened: Vec<usize>,
    /// Placed monsters already out (they don't come again).
    pub placed: Vec<usize>,
}

/// The level kept through a secret exit, and the way back to it.
#[derive(Resource, Clone, Debug, Default)]
pub struct SecretReturn {
    kept: Option<KeptLevel>,
    /// On the way back: laid on the level as it loads.
    restoring: Option<KeptLevel>,
    /// The level the party came back to, while it's there.
    back_in: Option<String>,
}

impl SecretReturn {
    /// The time is up on level `here`: where the party goes — the kept
    /// level, which it'll find as it left it; without one (or kept from
    /// here), the tower (stand-in).
    pub fn time_up(&mut self, here: &str) -> String {
        match self.kept.take().filter(|k| !k.level.eq_ignore_ascii_case(here)) {
            Some(kept) => {
                let to = kept.level.clone();
                self.restoring = Some(kept);
                to
            }
            None => crate::frontend::TOWER.to_string(),
        }
    }

    /// What to lay on `level` as it's built: the kept level's state when
    /// the party is coming back to it.
    pub fn restoring(&self, level: &str) -> Option<&KeptLevel> {
        self.restoring.as_ref().filter(|k| k.level.eq_ignore_ascii_case(level))
    }

    /// Coming back to `level`, its start moved to where the secret exit
    /// was taken, facing as the hero did (`world.rs`): the first hero
    /// arrives there, the others beside it, and the play camera starts
    /// on it.
    pub fn arrive(&self, level: &str, population: &mut Population, entry: i16) {
        let Some(kept) = self.restoring(level) else { return };
        let Some(start) = population.player_start(entry) else { return };
        let starts = population.locators.iter_mut().filter(|l| l.kind == LocatorKind::PlayerStart);
        if let Some(l) = starts.find(|l| l.index == start.entry && l.position == start.position) {
            l.position = kept.at;
            l.rotation[1] = kept.facing;
        }
    }

    /// Whether the party is coming back to `level` from the secret realm:
    /// its start makes no save and has no opening shot in the game (for
    /// `frontend.rs` and `play_camera.rs`, which are yet to ask).
    #[allow(dead_code)]
    pub fn coming_back_to(&self, level: &str) -> bool {
        self.restoring(level).is_some() || self.back_in.as_ref().is_some_and(|l| l.eq_ignore_ascii_case(level))
    }
}

/// Keeps the level the heroes leave through a secret exit, with the
/// placed monsters already out.
fn keep_level(
    mut taken: MessageReader<SecretExitTaken>,
    population: Option<Res<LevelPopulation>>,
    placed: Option<Res<PlacedMonsters>>,
    mut back: ResMut<SecretReturn>,
) {
    let Some(population) = population else { return };
    for exit in taken.read() {
        let placed_out = placed.as_ref().map_or(Vec::new(), |p| p.0.iter().filter(|m| m.spawned).map(|m| m.placement).collect());
        info!("{}: secret exit taken at {:?}; {} items gone", population.level, exit.at, exit.gone.len());
        back.kept = Some(KeptLevel {
            level: population.level.clone(),
            at: exit.at,
            facing: exit.facing,
            gone: exit.gone.clone(),
            opened: exit.opened.clone(),
            placed: placed_out,
        });
    }
}

/// Back in the kept level, before its first tick: its destroyed
/// generators stay gone and its placed monsters already out don't come
/// again. (Its items are laid on as they're built, `items.rs`, and its
/// start is where the heroes arrive, [`SecretReturn::arrive`].) The
/// level's heroes, its generators and its placed monsters all come in as
/// it loads: the frame its heroes appear, the rest are there.
fn restore_level(
    mut commands: Commands,
    population: Option<Res<LevelPopulation>>,
    mut back: ResMut<SecretReturn>,
    placed: Option<ResMut<PlacedMonsters>>,
    generators: Query<(Entity, &Generator)>,
    players: Query<(), Added<Player>>,
) {
    let Some(population) = population else { return };
    let Some(kept) = back.restoring(&population.level).cloned() else { return };
    if players.is_empty() {
        return;
    }
    if let Some(mut placed) = placed {
        for m in placed.0.iter_mut().filter(|m| kept.placed.contains(&m.placement)) {
            m.spawned = true;
        }
    }
    for (entity, g) in &generators {
        if kept.gone.contains(&g.placement) {
            commands.entity(entity).despawn();
        }
    }
    info!("back in {} at {:?}", kept.level, kept.at);
    back.restoring = None;
    back.back_in = Some(kept.level);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: f32 = 1.0 / 30.0;

    /// Runs the timer to its end: the seconds heard, and the ticks it took.
    fn run(timer: &mut LevelTimer) -> (Vec<i32>, usize) {
        let mut seconds = Vec::new();
        for tick in 1.. {
            match timer.step(TICK) {
                TimerStep::Running => {}
                TimerStep::Second(s) => seconds.push(s),
                TimerStep::Up => return (seconds, tick),
            }
        }
        unreachable!()
    }

    #[test]
    fn a_timed_level_counts_down_from_its_seconds() {
        // S2: 40 s on the record, 40.99 on the timer.
        let mut timer = LevelTimer::new(Some(40));
        assert!(timer.running());
        assert_eq!((timer.total, timer.left), (40.99, 40.99));
        let (seconds, ticks) = run(&mut timer);
        // Every whole second from 39 down to 0 is heard once.
        assert_eq!(seconds, (0..40).rev().collect::<Vec<_>>());
        // 40.99 s at 30 ticks a second, give or take single precision.
        assert!((1229..=1231).contains(&ticks), "{ticks}");
        assert!(timer.up && !timer.running() && timer.left == 0.0);
        // Once up it stays up, and says nothing more.
        assert_eq!(timer.step(TICK), TimerStep::Running);
    }

    #[test]
    fn an_untimed_level_has_no_timer() {
        let mut timer = LevelTimer::new(None);
        assert!(!timer.running());
        assert_eq!(timer.step(10.0), TimerStep::Running);
        assert_eq!(timer.gone(), 0.0);
    }

    #[test]
    fn the_sand_falls_with_the_time_gone() {
        let mut timer = LevelTimer::new(Some(70));
        assert_eq!(timer.gone(), 0.0);
        for _ in 0..(30 * 71 / 2) {
            timer.step(TICK);
        }
        assert!((timer.gone() - 0.5).abs() < 0.01, "{}", timer.gone());
    }

    #[test]
    fn the_last_coin_leaves_a_quiet_second() {
        let mut timer = LevelTimer::new(Some(70));
        for _ in 0..300 {
            timer.step(TICK);
        }
        timer.last_coin();
        assert!(timer.quiet && timer.left == 1.0);
        let (seconds, ticks) = run(&mut timer);
        assert!(seconds.is_empty());
        assert!((30..=31).contains(&ticks), "{ticks}");
    }

    #[test]
    fn more_time_than_the_record_drops_to_five_seconds() {
        let mut timer = LevelTimer::new(Some(40));
        timer.left = 42.0;
        timer.step(TICK);
        assert_eq!(timer.total, 5.0);
        assert!((timer.left - (5.0 - TICK)).abs() < 1e-6);
    }

    #[test]
    fn each_second_ticks_and_the_last_are_counted() {
        assert_eq!(second_sounds(39, false), ("S_SECRETCLOCK2", None));
        assert_eq!(second_sounds(10, false), ("S_SECRETCLOCK1", None));
        assert_eq!(second_sounds(8, false), ("S_SECRETCLOCK1", Some("S_TIMEISRUNNING".into())));
        assert_eq!(second_sounds(6, false), ("S_SECRETCLOCK1", None));
        assert_eq!(second_sounds(5, false), ("S_SECRETCLOCK2", Some("S_COUNT5".into())));
        assert_eq!(second_sounds(0, false), ("S_SECRETCLOCKEN", Some("S_COUNT0".into())));
        // Through a secret exit the count isn't said; the warning is.
        assert_eq!(second_sounds(3, true), ("S_SECRETCLOCK2", None));
        assert_eq!(second_sounds(8, true).1.as_deref(), Some("S_TIMEISRUNNING"));
    }

    #[test]
    fn every_secret_level_has_its_character() {
        let names: Vec<&str> = (0..9).filter_map(secret_character).map(class_code).collect();
        assert_eq!(names, ["JAC", "TIG", "FAL", "MIN", "SUM", "OGR", "HYE", "MED", "UNI"]);
        assert_eq!(secret_character(9), None);
        assert_eq!(coin_icon(10), "16_JACCOIN");
        assert_eq!(coin_icon(16), "16_SUM");
        assert_eq!((unlock_bit(8), unlock_bit(16), unlock_bit(3)), (1, 0x100, 0));
    }

    #[test]
    fn locked_classes_open_with_their_bits() {
        assert!((0..8).all(|c| class_open(c, 0)));
        assert!(!class_open(10, 0) && !class_open(16, 0));
        assert!(class_open(10, unlock_bit(10)) && !class_open(9, unlock_bit(10)));
        assert!(class_open(16, 0x100));
    }

    #[test]
    fn every_hero_counts_each_coin_until_one_has_them_all() {
        let mut counts = [0u32, 3];
        assert!(!count_coin(counts.iter_mut(), 25));
        assert_eq!(counts, [1, 4]);
        // The one ahead reaches the need first.
        let mut counts = [23u32, 24];
        assert!(count_coin(counts.iter_mut(), 25));
        assert!(!count_coin(std::iter::empty(), 25));
    }

    #[test]
    fn out_of_time_the_party_goes_back_to_the_kept_level() {
        let mut back = SecretReturn::default();
        // Nothing kept (a secret level started on its own): the tower.
        assert_eq!(back.time_up("levelS1"), crate::frontend::TOWER);
        back.kept = Some(KeptLevel { level: "levelC2".into(), gone: vec![516], ..default() });
        assert_eq!(back.time_up("levelS1"), "levelC2");
        assert!(back.kept.is_none());
        assert_eq!(back.restoring("LEVELC2").map(|k| k.gone.clone()), Some(vec![516]));
        assert!(back.restoring("levelA1").is_none());
        assert!(back.coming_back_to("levelC2") && !back.coming_back_to("levelS1"));
    }
}
