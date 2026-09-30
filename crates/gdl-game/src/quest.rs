//! The quest's progress (`docs/items.md` "Quest items and the tower's
//! gates"): crystals by colour, gargoyle pieces, legendary items and the
//! levels entered per realm, kept in the hero's character record, and the
//! game's rules for which realms, tower gates and exits they open.
//!
//! The game keeps this per player and opens a gate when any player's
//! progress does; there's one hero here.
//!
//! At run time: entering a level marks it; the tower's exit glows show
//! only for open exits (shut exits themselves are `items.rs`'); in the
//! tower, counters that have reached what they need are announced ("You
//! now have enough Crystals…") and opened, at most one round every
//! [`ANNOUNCE_SECONDS`]. The gates themselves are `mechanics.rs`'.

use bevy::prelude::*;
use gdl_formats::population::REALM_LETTERS;

use crate::hints::ShowMessage;
use crate::player_state::PlayerState;
use crate::population::LevelPopulation;

pub struct QuestPlugin;

impl Plugin for QuestPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                enter_level.run_if(resource_exists_and_changed::<LevelPopulation>).before(crate::items::build_items),
                show_exit_glows,
                announce_unlocks,
            ),
        );
    }
}

/// A level folder name (`levelG1`) as its realm id and level (0 the first).
pub fn level_of(name: &str) -> Option<(u32, u32)> {
    let rest = name.strip_prefix("level")?;
    let mut chars = rest.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let number: u32 = chars.as_str().parse().ok()?;
    let realm = REALM_LETTERS.iter().find(|(l, _)| *l == letter)?.1;
    Some((realm, number.checked_sub(1)?))
}

/// Entering a level marks it in the hero's record: the exits to the next
/// one open. `GDL_CRYSTALS="<counter>:<n>,…"` (testing) sets crystal
/// counts once, at the first level.
fn enter_level(population: Res<LevelPopulation>, state: Option<ResMut<PlayerState>>, mut seeded: Local<bool>) {
    let Some(mut state) = state else { return };
    if !*seeded {
        *seeded = true;
        for (c, n) in std::env::var("GDL_CRYSTALS").unwrap_or_default().split(',').filter_map(|p| p.split_once(':')) {
            if let (Ok(c), Ok(n)) = (c.trim().parse::<usize>(), n.trim().parse::<i16>())
                && c < state.quest.crystals.len()
            {
                state.quest.crystals[c] = n.min(CRYSTALS_NEEDED[c]);
                info!("GDL_CRYSTALS: {} {}", CRYSTAL_COLOURS[c], state.quest.crystals[c]);
            }
        }
    }
    let Some((realm, level)) = level_of(&population.level) else { return };
    state.quest.enter_level(realm, level);
}

/// A tower exit's glow (`L1NSNC<realm letter><n>_ACTIVE`), drawn only
/// while its exit is open.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitGlow {
    pub realm: u32,
    pub level: u32,
}

impl ExitGlow {
    pub fn named(name: &str) -> Option<Self> {
        let code = name.strip_prefix("L1NSNC")?.strip_suffix("_ACTIVE")?;
        let letter = code.chars().next()?;
        let n: u32 = code.get(1..)?.parse().ok()?;
        let realm = REALM_LETTERS.iter().find(|(l, _)| *l == letter)?.1;
        Some(Self { realm, level: n.checked_sub(1)? })
    }
}

fn show_exit_glows(state: Option<Res<PlayerState>>, mut glows: Query<(&ExitGlow, &mut Visibility)>) {
    let Some(state) = state else { return };
    for (g, mut v) in &mut glows {
        let want = if state.exit_open(g.realm, g.level) { Visibility::Inherited } else { Visibility::Hidden };
        v.set_if_neq(want);
    }
}

/// In the tower: counters that have what they need open, and are
/// announced — gargoyle sections, then crystals.
fn announce_unlocks(
    time: Res<Time>,
    population: Option<Res<LevelPopulation>>,
    state: Option<ResMut<PlayerState>>,
    mut messages: MessageWriter<ShowMessage>,
    mut last: Local<Option<f32>>,
) {
    let (Some(population), Some(mut state)) = (population, state) else { return };
    if level_of(&population.level).is_none_or(|(realm, _)| realm != TOWER) {
        return;
    }
    let now = time.elapsed_secs();
    if last.is_some_and(|t| now - t < ANNOUNCE_SECONDS) {
        return;
    }
    let (counters, sections) = state.quest.newly_open();
    for s in &sections {
        messages.write(ShowMessage::new("UNLOCKSECTION", *s).voice(GARGOYLE_VOICES[*s]));
    }
    for c in &counters {
        messages.write(ShowMessage::new("UNLOCKLEVEL", *c).voice(CRYSTAL_VOICES[*c]));
    }
    if !counters.is_empty() || !sections.is_empty() {
        info!("the tower opens: crystals {counters:?}, sections {sections:?}");
        *last = Some(now);
    }
}

/// Crystal counters: 0 is the tower's (never needed), then the colours
/// that open the realms.
pub const CRYSTAL_COLOURS: [&str; 9] = ["TOWER", "ORANGE", "RED", "PURPLE", "BLUE", "GREEN", "YELLOW", "WHITE", "BLACK"];
/// Crystals each counter needs.
pub const CRYSTALS_NEEDED: [i16; 9] = [0, 15, 100, 125, 150, 175, 200, 225, 250];
/// What the announcer says when a counter's realm opens.
pub const CRYSTAL_VOICES: [&str; 9] =
    ["", "S_CRYS4TWN", "S_CRYS4MNT", "S_CRYS4CST", "S_CRYS4SKY", "S_CRYS4FOR", "S_CRYS4DES", "S_CRYS4ICE", "S_CRYS4DRM"];
/// A gem's colour code (its item's amount) → its crystal counter. Code 8
/// points past the counters in the game; nothing uses it.
pub const GEM_COUNTERS: [Option<usize>; 10] =
    [Some(4), Some(2), Some(6), Some(5), Some(1), Some(7), Some(8), Some(3), None, Some(5)];

/// Gargoyle pieces: snake fangs, eagle feathers, lion claws — the tower's
/// west wing, east wing and lower tower.
pub const GARGOYLE_NEEDED: [i16; 3] = [12, 20, 28];
pub const GARGOYLE_VOICES: [&str; 3] = ["S_FNGS4WST", "S_FTHS4WST", "S_CLWS4BTL"];

/// The tower's realm id.
pub const TOWER: u32 = 13;
/// The eight realms with crystal gates (town, mountain, castle, sky,
/// forest, desert, ice, dream), then the three that open after them.
const MAIN_REALMS: [u32; 8] = [7, 2, 1, 11, 4, 3, 9, 10];
const REALM_E: u32 = 5;
const REALM_F: u32 = 6;
const REALM_H: u32 = 8;

/// Seconds between the tower's unlock announcements, and before a closed
/// gate says what it needs again.
pub const ANNOUNCE_SECONDS: f32 = 3.0;
pub const NEED_AGAIN_SECONDS: f32 = 10.0;

/// The crystal counter that opens a realm (by realm id); 0 for the realms
/// that don't need crystals.
pub fn realm_counter(realm: u32) -> usize {
    match realm {
        1 => 3,
        2 => 2,
        3 => 6,
        4 => 5,
        7 => 1,
        9 => 7,
        10 => 8,
        11 => 4,
        _ => 0,
    }
}

/// One hero's quest progress.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quest {
    /// Crystals by counter; −1 once the counter's realm has opened.
    pub crystals: [i16; 9],
    /// Gargoyle pieces; −1 once their section has opened.
    pub gargoyle: [i16; 3],
    /// Legendary items found, a bit each.
    pub legendary: u16,
    /// Per realm id, a bit for each level entered (the exit to level n + 1
    /// needs level n's).
    pub entered: [u8; 14],
}

impl Quest {
    /// A gem of this colour code: its counter goes up unless its realm is
    /// open or it's full. Returns the counter.
    pub fn add_gem(&mut self, colour: i32) -> Option<usize> {
        let counter = GEM_COUNTERS.get(usize::try_from(colour).ok()?).copied().flatten()?;
        let c = &mut self.crystals[counter];
        if *c >= 0 && *c < CRYSTALS_NEEDED[counter] {
            *c += 1;
        }
        Some(counter)
    }

    /// A gargoyle piece (0 fang, 1 feather, 2 claw).
    pub fn add_gargoyle(&mut self, piece: i32) -> Option<usize> {
        let piece = usize::try_from(piece).ok().filter(|&p| p < 3)?;
        let c = &mut self.gargoyle[piece];
        if *c >= 0 && *c < GARGOYLE_NEEDED[piece] {
            *c += 1;
        }
        Some(piece)
    }

    /// Whether a crystal counter lets its gate open: its realm has opened,
    /// or it has what it needs.
    pub fn crystals_open(&self, counter: usize) -> bool {
        self.crystals.get(counter).is_some_and(|&c| c < 0 || c >= CRYSTALS_NEEDED[counter])
    }

    /// The same for a gargoyle section.
    pub fn gargoyle_open(&self, section: usize) -> bool {
        let section = section.min(2);
        self.gargoyle[section] < 0 || self.gargoyle[section] >= GARGOYLE_NEEDED[section]
    }

    /// Whether a realm (by id) is open, given the realms beaten (a bit per
    /// realm id) and the runestones held (a bit per stone): the tower and
    /// realms without a crystal counter always; the eight by their
    /// crystals; E once all eight are beaten, F once E is too and twelve
    /// runestones are held, H once F is beaten.
    pub fn realm_open(&self, realm: u32, beaten: u32, runestones: u32) -> bool {
        let all = |realms: &[u32]| realms.iter().all(|&r| beaten & (1 << r) != 0);
        match realm {
            TOWER => true,
            REALM_E => all(&MAIN_REALMS),
            REALM_F => all(&MAIN_REALMS) && all(&[REALM_E]) && runestones & 0xFFF == 0xFFF,
            REALM_H => all(&MAIN_REALMS) && all(&[REALM_E, REALM_F]),
            r => self.crystals_open(realm_counter(r)),
        }
    }

    /// Whether an exit to `level` (0 the first) of `realm` is open: E and F
    /// once open; H once open, its fourth level only with all thirteen
    /// runestones; everywhere else the first level always, and each next
    /// one once the one before has been entered.
    pub fn exit_open(&self, realm: u32, level: u32, beaten: u32, runestones: u32) -> bool {
        let entered_before = || level == 0 || self.entered.get(realm as usize).is_some_and(|&b| b & (1 << (level - 1)) != 0);
        match realm {
            REALM_E | REALM_F => self.realm_open(realm, beaten, runestones),
            REALM_H if !self.realm_open(realm, beaten, runestones) => false,
            REALM_H if level == 3 => runestones & 0x1FFF == 0x1FFF,
            _ => entered_before(),
        }
    }

    /// Entering a level marks it in its realm's byte.
    pub fn enter_level(&mut self, realm: u32, level: u32) {
        if let Some(b) = self.entered.get_mut(realm as usize)
            && level < 8
        {
            *b |= 1 << level;
        }
    }

    /// The tower's check: counters (crystals, then gargoyle sections) that
    /// have just reached what they need are marked open (−1) and returned
    /// for their announcements.
    pub fn newly_open(&mut self) -> (Vec<usize>, Vec<usize>) {
        let mut sections = Vec::new();
        for (i, c) in self.gargoyle.iter_mut().enumerate() {
            if *c == GARGOYLE_NEEDED[i] {
                *c = -1;
                sections.push(i);
            }
        }
        let mut counters = Vec::new();
        for (i, c) in self.crystals.iter_mut().enumerate() {
            if CRYSTALS_NEEDED[i] != 0 && *c == CRYSTALS_NEEDED[i] {
                *c = -1;
                counters.push(i);
            }
        }
        (counters, sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MAIN: u32 = (1 << 7) | (1 << 2) | (1 << 1) | (1 << 11) | (1 << 4) | (1 << 3) | (1 << 9) | (1 << 10);

    #[test]
    fn orange_gems_open_the_town() {
        let mut q = Quest::default();
        assert!(!q.realm_open(7, 0, 0));
        for _ in 0..14 {
            assert_eq!(q.add_gem(4), Some(1));
        }
        assert!(!q.crystals_open(1));
        q.add_gem(4);
        assert!(q.crystals_open(1) && q.realm_open(7, 0, 0));
        // Full: more don't count, and the tower then opens it for good.
        q.add_gem(4);
        assert_eq!(q.crystals[1], 15);
        assert_eq!(q.newly_open(), (vec![1], vec![]));
        assert_eq!(q.crystals[1], -1);
        assert!(q.realm_open(7, 0, 0));
        q.add_gem(4);
        assert_eq!(q.crystals[1], -1);
    }

    #[test]
    fn exits_follow_the_levels_entered() {
        let mut q = Quest::default();
        assert!(q.exit_open(7, 0, 0, 0));
        assert!(!q.exit_open(7, 1, 0, 0));
        q.enter_level(7, 0);
        assert!(q.exit_open(7, 1, 0, 0));
        assert!(!q.exit_open(7, 2, 0, 0));
    }

    #[test]
    fn the_last_realms_open_in_turn() {
        let q = Quest::default();
        assert!(!q.realm_open(REALM_E, ALL_MAIN & !(1 << 10), 0));
        assert!(q.realm_open(REALM_E, ALL_MAIN, 0));
        let with_e = ALL_MAIN | (1 << REALM_E);
        assert!(!q.realm_open(REALM_F, with_e, 0x7FF));
        assert!(q.realm_open(REALM_F, with_e, 0xFFF));
        let with_f = with_e | (1 << REALM_F);
        assert!(q.realm_open(REALM_H, with_f, 0));
        assert!(q.exit_open(REALM_H, 0, with_f, 0));
        assert!(!q.exit_open(REALM_H, 3, with_f, 0xFFF));
        assert!(q.exit_open(REALM_H, 3, with_f, 0x1FFF));
    }

    #[test]
    fn gargoyle_sections() {
        let mut q = Quest::default();
        for _ in 0..12 {
            q.add_gargoyle(0);
        }
        assert!(q.gargoyle_open(0) && !q.gargoyle_open(1));
        assert_eq!(q.newly_open(), (vec![], vec![0]));
    }
}
