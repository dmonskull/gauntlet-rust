//! The after-level screen and the tower's Shop and Inventory
//! (`docs/shop.md`).
//!
//! After a level each hero who left through the exit gets, in their own
//! column: the tally (three heaps — gold, kills, experience — rising), the
//! level-up page if a level was gained (after levelH4 the final stats, and
//! nothing more), the shop, the stats page and the inventory; then the
//! select screen's character menu, where others can join. From the Tower
//! Menu, the Shop is the list and the stats page, the Inventory the
//! inventory alone; then the tower starts again.
//!
//! The front end opens it ([`ShopScreen::open`]), runs it every frame with
//! each player's presses ([`tick`]) and acts on how it ends
//! ([`ShopOutcome`]). [`ShopPlugin`] loads its data ([`ShopData`]), draws
//! it while it's open and plays its sounds. The rules — the item table,
//! buying and selling on a [`PlayerState`], the tally, the stats, the
//! list's layout — are plain functions.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use gdl_formats::detmath::Det;
use gdl_formats::ModelFile;
use gdl_formats::chunk::ChunkFile;
use gdl_formats::font::FONT32;
use gdl_formats::pdata::PlayerStats;
use gdl_formats::text::TextRom;

use crate::audio::{LoopSoundAt, PlaySound, PlaySoundAt, QueueVoice};
use crate::font::{Draw2d, GameFonts, Quad, TextStyle, UiImage, UiTextures};
use crate::level::LoadedGame;
use crate::locomotion::{STAT_CAP, stat_at_level};
use crate::party::{MAX_PLAYERS, Party};
use crate::player_state::{MAX_KEYS, MAX_LEVEL, MAX_POTIONS, PlayerState, Power};
use crate::tower_scenes::Rank;

pub struct ShopPlugin;

impl Plugin for ShopPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShopScreen>().add_systems(Startup, load_shop).add_systems(
            Update,
            (play_sounds, draw_screen.run_if(|s: Res<ShopScreen>| s.is_open())).chain().in_set(ShopDraw),
        );
    }
}

/// Where the screen is drawn and its sounds played: the front end's
/// [`tick`] goes before it so a frame shows what that tick did.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShopDraw;

// ---------------------------------------------------------------------------
// The item table

/// One of the shop's items (`SHPDATA/SHOP.WAD`, chunk `ITEM`).
#[derive(Clone, Debug, PartialEq)]
pub struct ShopItem {
    /// Its icon's texture name; empty for EXIT.
    pub icon: String,
    /// What the list says, lines split by `\n`.
    pub text: String,
    /// The text's scale (× 0.5).
    pub text_scale: f32,
    pub kind: i32,
    pub price: i32,
    pub amount: i32,
}

/// An `ITEM` record's length.
const ITEM_LEN: usize = 0x50;

/// The items of a shop file, in its order (cheapest first, EXIT first).
pub fn parse_items(bytes: &[u8]) -> Option<Vec<ShopItem>> {
    let file = ChunkFile::parse(bytes).ok()?;
    let records = file.records("ITEM", ITEM_LEN)?;
    let int = |r: &[u8], at: usize| i32::from_le_bytes(r[at..at + 4].try_into().unwrap());
    Some(
        records
            .map(|r| ShopItem {
                icon: cstr(&r[..0x20]),
                text: cstr(&r[0x20..0x40]),
                text_scale: f32::from_le_bytes(r[0x40..0x44].try_into().unwrap()),
                kind: int(r, 0x44),
                price: int(r, 0x48),
                amount: int(r, 0x4C),
            })
            .collect(),
    )
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| c as char).collect()
}

/// A raw stat, in the order the stats page lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    Strength = 0,
    Armour = 1,
    Magic = 2,
    Speed = 3,
}

/// What buying an item does, by its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Type 0 and any type the buy switch doesn't know: the shop is left.
    Exit,
    Key,
    Potion,
    /// Ten bought points.
    Stat(Stat),
    /// Heals by the item's amount.
    Heal(f32),
    /// A power, granted as a pickup's is (held until turned on).
    Power { subtype: i32, value: u32, amount: f32, duration: f32 },
}

/// The buy switch's grants: type, amount, duration (seconds; −1 counted),
/// subtype, value.
const GRANTS: [(i32, f32, f32, i32, u32); 32] = [
    (2, 0.0, 90.0, 5, 0x20_0000),
    (4, 0.0, 30.0, 9, 0x100),
    (9, 0.0, 30.0, 6, 0x2_0000),
    (10, 0.0, 45.0, 9, 0x80),
    (11, 0.0, 30.0, 5, 0x2000_0000),
    (12, 0.0, 45.0, 5, 0x8_0000),
    (13, 3.0, -1.0, 5, 0x1000_0000),
    (14, 0.0, 15.0, 6, 0x40_0000),
    (15, 0.0, 15.0, 6, 0x20_0000),
    (16, 0.0, 20.0, 6, 0x11_0000),
    (18, 0.0, 60.0, 9, 0x1),
    (19, 0.0, 30.0, 9, 0x100),
    (20, 0.0, 90.0, 5, 1),
    (21, 0.0, 90.0, 5, 2),
    (22, 0.0, 90.0, 5, 3),
    (23, 0.0, 90.0, 5, 4),
    (24, 5.0, -1.0, 5, 0x10_0000),
    (25, 5.0, -1.0, 9, 0x10),
    (26, 5.0, -1.0, 9, 0x40),
    (27, 5.0, -1.0, 9, 0x20),
    (28, 4.0, 40.0, 9, 0x1_0000),
    (29, 0.0, 15.0, 9, 0x200),
    (30, 0.0, 15.0, 9, 0x4),
    (31, 0.0, 30.0, 6, 0x1_0000),
    (32, 0.0, 120.0, 9, 0x2),
    (33, 0.0, 15.0, 6, 0x2008),
    (34, 0.0, 45.0, 5, 0x40_0000),
    (35, 0.0, 120.0, 6, 0x8_0000),
    (36, 0.0, 120.0, 9, 0x20_0000),
    (37, 0.0, 120.0, 9, 0x40_0000),
    (38, 0.0, 120.0, 9, 0x10_0000),
    (39, 0.0, 25.0, 9, 0x8),
];

/// The power each type sells back (subtype, value): the grants' but for
/// the electric-and-fire shield (16), growth (19) and shrink (29).
const SOLD: [(i32, i32, u32); 29] = [
    (2, 5, 0x20_0000),
    (9, 6, 0x2_0000),
    (10, 9, 0x80),
    (11, 5, 0x2000_0000),
    (12, 5, 0x8_0000),
    (13, 5, 0x1000_0000),
    (14, 6, 0x40_0000),
    (15, 6, 0x20_0000),
    (18, 9, 0x1),
    (20, 5, 1),
    (21, 5, 2),
    (22, 5, 3),
    (23, 5, 4),
    (24, 5, 0x10_0000),
    (25, 9, 0x10),
    (26, 9, 0x40),
    (27, 9, 0x20),
    (28, 9, 0x1_0000),
    (30, 9, 0x4),
    (31, 6, 0x1_0000),
    (32, 9, 0x2),
    (33, 6, 0x2008),
    (34, 5, 0x40_0000),
    (35, 6, 0x8_0000),
    (36, 9, 0x20_0000),
    (37, 9, 0x40_0000),
    (38, 9, 0x10_0000),
    (39, 9, 0x8),
    // Type 0 has no entry: EXIT is never owned.
    (-1, 0, 0),
];

/// What buying an item of `kind` does.
pub fn effect(kind: i32, amount: i32) -> Effect {
    match kind {
        1 => Effect::Key,
        3 => Effect::Potion,
        5 => Effect::Stat(Stat::Strength),
        6 => Effect::Stat(Stat::Speed),
        7 => Effect::Stat(Stat::Armour),
        8 => Effect::Stat(Stat::Magic),
        17 => Effect::Heal(amount as f32),
        _ => GRANTS.iter().find(|g| g.0 == kind).map_or(Effect::Exit, |&(_, amount, duration, subtype, value)| {
            Effect::Power { subtype, value, amount, duration }
        }),
    }
}

/// The power an item of `kind` is sold back as.
pub fn sold_power(kind: i32) -> Option<(i32, u32)> {
    SOLD.iter().find(|s| s.0 == kind && s.1 != 0).map(|&(_, subtype, value)| (subtype, value))
}

/// What a sale brings: three quarters of the price (truncated).
pub fn sale_price(price: i32) -> i32 {
    price * 3 / 4
}

// ---------------------------------------------------------------------------
// Stats

/// Points bought in the shop, by stat: the hero's record keeps them
/// (`PlayerState::bought`); the screen takes them as it opens and gives
/// them back ([`ShopScreen::bonus`]).
pub use crate::player_state::StatBonus;

impl StatBonus {
    #[allow(dead_code)] // the stats page reads the array
    pub fn get(&self, stat: Stat) -> f32 {
        match stat {
            Stat::Strength => self.strength,
            Stat::Armour => self.armour,
            Stat::Magic => self.magic,
            Stat::Speed => self.speed,
        }
    }

    fn add(&mut self, stat: Stat, points: f32) {
        match stat {
            Stat::Strength => self.strength += points,
            Stat::Armour => self.armour += points,
            Stat::Magic => self.magic += points,
            Stat::Speed => self.speed += points,
        }
    }

    fn as_array(&self) -> [f32; 4] {
        [self.strength, self.armour, self.magic, self.speed]
    }
}

/// Each shop stat buy adds this many points.
const STAT_POINTS: f32 = 10.0;

/// The raw stats — strength, armour, magic, speed — at `level` with
/// `bought` points: the class's start + 5 a level up to its maximum, plus
/// the points, at most 999; the summoner has 999 in all four.
pub fn raw_stats(stats: Option<&PlayerStats>, summoner: bool, level: u32, bought: &StatBonus) -> [f32; 4] {
    if summoner {
        return [STAT_CAP; 4];
    }
    let Some(s) = stats else { return bought.as_array().map(|b| b.min(STAT_CAP)) };
    let at = |stat: gdl_formats::pdata::Stat, b: f32| stat_at_level(stat.start, stat.max, level, b);
    [at(s.strength, bought.strength), at(s.armor, bought.armour), at(s.magic, bought.magic), at(s.speed, bought.speed)]
}

/// The experience a level starts at: `(L − 1)(30L + 1000)`, from 61 on
/// `(L − 60) × 4600 + 165200`.
pub fn level_threshold(level: u32) -> u32 {
    if level <= 60 { level.saturating_sub(1) * (30 * level + 1000) } else { (level - 60) * 4600 + 165_200 }
}

/// The level `experience` gives (at most 99).
pub fn level_for(experience: u32) -> u32 {
    (1..=MAX_LEVEL).rev().find(|&l| level_threshold(l) <= experience).unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Buying and selling

/// The power slot the shop finds `subtype`/`value` in. The game's search
/// only steps past a slot whose time is above 0: from the first empty slot
/// or counted power (time −1) on it finds nothing.
pub fn owned_slot(powers: &[Power], subtype: i32, value: u32) -> Option<usize> {
    for (i, p) in powers.iter().enumerate() {
        if p.time <= 0.0 {
            return None;
        }
        if p.subtype == subtype && p.value == value {
            return Some(i);
        }
    }
    None
}

/// Whether the hero owns an item the shop would buy back.
pub fn owns(item: &ShopItem, state: &PlayerState) -> bool {
    if let Some((subtype, value)) = sold_power(item.kind) {
        return owned_slot(&state.powers, subtype, value).is_some();
    }
    match item.kind {
        1 => state.keys > 0,
        3 => !state.potions.is_empty(),
        _ => false,
    }
}

/// Whether the hero can buy an item: gold enough, and room for a key or a
/// potion, a raw stat below 999, health below its maximum.
pub fn can_buy(item: &ShopItem, state: &PlayerState, raw: &[f32; 4]) -> bool {
    if i64::from(state.gold) < i64::from(item.price) {
        return false;
    }
    match effect(item.kind, item.amount) {
        Effect::Key => state.keys < MAX_KEYS,
        Effect::Potion => state.potions.len() < MAX_POTIONS,
        Effect::Stat(s) => raw[s as usize] < STAT_CAP,
        Effect::Heal(_) => state.health < state.max_health(),
        _ => true,
    }
}

/// How a purchase went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purchase {
    /// Not enough gold, or no room: nothing changes.
    Refused,
    Bought,
    /// EXIT: the shop is left.
    Exit,
}

/// Buys an item: the price comes off the gold and the item's effect goes
/// on the hero; a stat's points go on `bonus` and on this visit's
/// `bought`. `potion_kind` is asked for only when a potion is bought (the
/// game's random kind 1–4).
pub fn buy(
    item: &ShopItem,
    state: &mut PlayerState,
    bonus: &mut StatBonus,
    bought: &mut StatBonus,
    potion_kind: impl FnOnce() -> i32,
) -> Purchase {
    if i64::from(state.gold) < i64::from(item.price) {
        return Purchase::Refused;
    }
    let done = match effect(item.kind, item.amount) {
        Effect::Exit => {
            pay(state, item.price);
            return Purchase::Exit;
        }
        Effect::Key => state.keys < MAX_KEYS && {
            state.keys += 1;
            true
        },
        Effect::Potion => state.potions.len() < MAX_POTIONS && {
            state.potions.push(potion_kind());
            true
        },
        Effect::Stat(s) => {
            bonus.add(s, STAT_POINTS);
            bought.add(s, STAT_POINTS);
            true
        }
        Effect::Heal(amount) => {
            state.heal(amount);
            true
        }
        Effect::Power { subtype, value, amount, duration } => {
            state.grant_power(subtype, value, amount, duration);
            true
        }
    };
    if !done {
        return Purchase::Refused;
    }
    pay(state, item.price);
    Purchase::Bought
}

fn pay(state: &mut PlayerState, price: i32) {
    state.gold = (i64::from(state.gold) - i64::from(price)).clamp(0, i64::from(u32::MAX)) as u32;
}

/// Sells an owned item back: a key, the last potion, or the power's slot
/// emptied, for three quarters of its price. Returns whether it sold.
pub fn sell(item: &ShopItem, state: &mut PlayerState) -> bool {
    let sold = if let Some((subtype, value)) = sold_power(item.kind) {
        match owned_slot(&state.powers, subtype, value) {
            Some(i) => {
                state.powers[i] = Power::default();
                true
            }
            None => false,
        }
    } else {
        match item.kind {
            1 => state.use_key(),
            3 => state.potions.pop().is_some(),
            _ => false,
        }
    };
    if sold {
        // The sale isn't capped like a pickup's gold.
        pay(state, -sale_price(item.price));
    }
    sold
}

// ---------------------------------------------------------------------------
// The tally

/// The heaps: gold, kills, experience (`SHP_GOLD`, `SHP_BONES`,
/// `SHP_EXP`), and their words.
const GOLD: usize = 0;
const KILLS: usize = 1;
const EXPERIENCE: usize = 2;
const HEAPS: [&str; 3] = ["SHP_GOLD", "SHP_BONES", "SHP_EXP"];
const TALLY_WORDS: [&str; 3] = ["Gold", "Kills", "Exp."];
const TALLY_WORDS_Y: [f32; 3] = [32.0, 52.0, 72.0];
/// Heaps rise from this line, start this high, and stand at least / at
/// most this high.
const HEAP_FLOOR: f32 = 320.0;
const HEAP_START: f32 = 20.0;
const HEAP_LOW: i64 = 64;
const HEAP_FULL: i64 = 208;
/// How fast a heap rises (and the gold heap falls as gold is spent), a
/// field.
const HEAP_RATE: f32 = 1.5;

/// A heap's height for an amount against the level's full mark.
pub fn tally_height(gained: i64, mark: i64) -> i64 {
    (gained * HEAP_FULL / (mark + 1).max(1)).clamp(HEAP_LOW, HEAP_FULL)
}

/// The heaps in the order they rise, tallest first (ties: gold, then
/// experience, then kills).
pub fn tally_rows(gold: i64, kills: i64, experience: i64) -> [usize; 3] {
    if gold >= experience && gold >= kills {
        if experience < kills { [GOLD, KILLS, EXPERIENCE] } else { [GOLD, EXPERIENCE, KILLS] }
    } else if experience >= gold && experience >= kills {
        if gold < kills { [EXPERIENCE, KILLS, GOLD] } else { [EXPERIENCE, GOLD, KILLS] }
    } else if gold < experience {
        [KILLS, EXPERIENCE, GOLD]
    } else {
        [KILLS, GOLD, EXPERIENCE]
    }
}

#[derive(Clone, Debug, Default)]
struct Tally {
    /// Gold, kills and experience the level brought.
    amounts: [i64; 3],
    heights: [f32; 3],
    shown: [f32; 3],
    rows: [usize; 3],
    /// The row that rose this frame; none once they're all up.
    rising: Option<usize>,
}

impl Tally {
    fn new(amounts: [i64; 3], marks: [i32; 3]) -> Self {
        let h = |i: usize| tally_height(amounts[i], i64::from(marks[i]));
        let heights = [h(GOLD), h(KILLS), h(EXPERIENCE)];
        Self {
            amounts,
            heights: heights.map(|h| h as f32),
            shown: [HEAP_START; 3],
            rows: tally_rows(heights[GOLD], heights[KILLS], heights[EXPERIENCE]),
            rising: Some(0),
        }
    }

    /// One frame: the first heap not up rises; the rest wait.
    fn rise(&mut self, fields: f32) {
        self.rising = None;
        for (row, &heap) in self.rows.iter().enumerate() {
            if self.shown[heap] < self.heights[heap] {
                self.shown[heap] = (self.shown[heap] + HEAP_RATE * fields).min(self.heights[heap]);
                self.rising = Some(row);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The list

/// The list's window, its middle, and how far outside it items fade.
const WINDOW_TOP: f32 = 72.0;
const WINDOW_BOTTOM: f32 = 224.0;
const WINDOW_MIDDLE: f32 = 148.0;
const FADE: f32 = 64.0;
/// What an icon and a line of text take in the list's layout.
const ICON_STEP: f32 = 24.0;
const TEXT_GAP: f32 = 16.0;
/// `font32`'s height, and the list's text scale (× the item's).
const FONT32_HEIGHT: f32 = 32.0;
const LIST_SCALE: f32 = 0.5;
/// A text's lines are at most this many.
const MAX_LINES: usize = 16;
/// Fields a bought or sold row flashes red.
const FLASH_FIELDS: f32 = 30.0;
/// Slowest scroll, pixels a field.
const SCROLL_MIN: f32 = 2.0;
/// Item flags: owned (sold back with X), buyable.
const OWNED: u8 = 4;
const BUYABLE: u8 = 2;

/// A text's height in the list: its lines × trunc(32 × 0.5 × its scale).
pub fn text_height(text: &str, scale: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let lines = (text.matches('\n').count() + 1).min(MAX_LINES);
    lines as f32 * (FONT32_HEIGHT * LIST_SCALE * scale).trunc()
}

/// Each item's place from the first: an icon takes 24, a text its height
/// and 16.
pub fn item_offsets(items: &[ShopItem], icons: &[bool]) -> Vec<f32> {
    let mut y = 0.0;
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let at = y;
            if icons.get(i).copied().unwrap_or(false) {
                y += ICON_STEP;
            }
            if !item.text.is_empty() {
                y += text_height(&item.text, item.text_scale) + TEXT_GAP;
            }
            at
        })
        .collect()
}

/// Where each item stands with the cursor on `cursor`: that item in the
/// window's middle, but the first no lower than its top and then the last
/// no higher than its bottom.
pub fn list_targets(offsets: &[f32], cursor: usize) -> Vec<f32> {
    let (Some(&first), Some(&last), Some(&at)) = (offsets.first(), offsets.last(), offsets.get(cursor)) else {
        return Vec::new();
    };
    let rel = |o: f32| (o - at).clamp(-1024.0, 1024.0);
    let mut base = WINDOW_MIDDLE;
    if WINDOW_TOP < base + rel(first) {
        base = WINDOW_TOP - rel(first);
    }
    if base + rel(last) < WINDOW_BOTTOM {
        base = WINDOW_BOTTOM - rel(last);
    }
    offsets.iter().map(|&o| base + rel(o)).collect()
}

/// An item's transparency (the game's, 0 opaque to 255) at `y`: fading
/// outside the window, gone (256) past the fade.
fn list_fade(y: f32) -> f32 {
    if y < WINDOW_TOP - FADE || y > WINDOW_BOTTOM + FADE {
        256.0
    } else if y < WINDOW_TOP {
        ((WINDOW_TOP - y) * 510.0 / FADE).trunc().min(255.0)
    } else if y > WINDOW_BOTTOM {
        ((y - WINDOW_BOTTOM) * 510.0 / FADE).trunc().min(255.0)
    } else {
        0.0
    }
}

/// A skipped item in the window is drawn this faded.
const SKIPPED_FADE: f32 = 160.0;

#[derive(Clone, Debug, Default)]
struct List {
    cursor: usize,
    /// The game's window top (seven rows); it only matters when a purchase
    /// moves the cursor above it.
    top: i64,
    /// Down past this wraps to EXIT.
    length: usize,
    /// Pixels a field while the list moves; 0 while it's still.
    speed: f32,
    placed: bool,
    ys: Vec<f32>,
    flash: Vec<f32>,
    flags: Vec<u8>,
    skip: Vec<bool>,
    /// Gold as the list opened, and the gold heap's height.
    gold_in: u32,
    heap: f32,
}

/// The last item the hero can afford or owns.
fn last_reachable(items: &[ShopItem], state: &PlayerState) -> usize {
    items
        .iter()
        .enumerate()
        .rfind(|(_, item)| i64::from(state.gold) >= i64::from(item.price) || owns(item, state))
        .map_or(0, |(i, _)| i)
}

/// The gold heap's height for `gold` against the gold the list opened
/// with.
fn gold_heap(gold: u32, gold_in: u32) -> f32 {
    let room = HEAP_FULL - HEAP_START as i64;
    (HEAP_START as i64 + i64::from(gold) * room / (i64::from(gold_in) + 1)) as f32
}

/// What the list needs besides itself.
struct ListCx<'a> {
    items: &'a [ShopItem],
    offsets: &'a [f32],
    stats: Option<&'a PlayerStats>,
    summoner: bool,
    sounds: &'a mut Vec<ShopSound>,
    rng: &'a mut u32,
}

impl List {
    fn new(items: &[ShopItem], state: &PlayerState, raw: &[f32; 4], heap: f32) -> Self {
        let n = items.len();
        let mut list = Self {
            length: items.iter().filter(|i| i64::from(i.price) <= i64::from(state.gold)).count().max(1),
            ys: vec![0.0; n],
            flash: vec![0.0; n],
            flags: vec![0; n],
            skip: vec![false; n],
            gold_in: state.gold,
            heap,
            ..default()
        };
        list.mark(items, state, raw);
        list
    }

    /// The owned and buyable flags, and the items skipped (neither), from
    /// item 1 on: EXIT is never skipped.
    fn mark(&mut self, items: &[ShopItem], state: &PlayerState, raw: &[f32; 4]) {
        for (i, item) in items.iter().enumerate().skip(1) {
            let owned = owns(item, state);
            let buyable = can_buy(item, state, raw);
            self.flags[i] = if owned { OWNED } else { 0 } | if buyable { BUYABLE } else { 0 };
            self.skip[i] = !owned && !buyable;
        }
    }

    /// One frame of the list: the gold heap, the cursor, buying and
    /// selling, then the items moving. Returns true when EXIT is bought.
    fn frame(
        &mut self,
        cx: &mut ListCx,
        state: &mut PlayerState,
        bonus: &mut StatBonus,
        bought: &mut StatBonus,
        press: ShopPress,
        fields: f32,
    ) -> bool {
        let items = cx.items;
        let n = items.len();
        if n == 0 {
            return true;
        }
        let level = state.level;
        let raw = |b: &StatBonus| raw_stats(cx.stats, cx.summoner, level, b);

        // The gold heap: spending lowers it, a sale raises it at once.
        let target = gold_heap(state.gold, self.gold_in);
        self.heap = if target < self.heap { (self.heap - HEAP_RATE * fields).max(target) } else { target };

        // The cursor: down and up, again while it lands on a skipped item.
        let mut speed = if self.speed > 0.0 { self.speed } else { 1.0 };
        let mut row = self.cursor as i64 - self.top;
        let mut snap = false;
        let mut ticked = false;
        for _ in 0..=n {
            if press.down {
                if !ticked {
                    cx.sounds.push(ShopSound::Plain(TICK_DOWN));
                    ticked = true;
                }
                if self.speed > 0.0 {
                    speed = self.speed + 1.0;
                }
                self.cursor += 1;
                if self.cursor >= self.length {
                    self.cursor = 0;
                    snap = true;
                    row = 0;
                    self.top = 0;
                }
            }
            if press.up {
                if !ticked {
                    cx.sounds.push(ShopSound::Plain(TICK_UP));
                    ticked = true;
                }
                if self.speed > 0.0 {
                    speed = self.speed + 1.0;
                }
                if self.cursor == 0 {
                    self.length = last_reachable(items, state) + 1;
                    self.cursor = self.length - 1;
                    row = self.cursor as i64;
                    self.top = row - 6;
                    if self.top < 0 {
                        row -= self.top;
                        self.top = 0;
                    }
                    snap = true;
                } else {
                    self.cursor -= 1;
                }
            }
            if !self.skip[self.cursor.min(n - 1)] {
                break;
            }
        }
        self.cursor = self.cursor.min(n - 1);

        // A buys and X sells only while the list is still; B goes to EXIT.
        let still = self.speed == 0.0;
        let mut changed = false;
        let mut exit = false;
        if still && press.sell && self.flags[self.cursor] & OWNED != 0 {
            if sell(&items[self.cursor], state) {
                cx.sounds.push(ShopSound::Plain(GOLD_SOUND));
                changed = true;
            } else {
                cx.sounds.push(ShopSound::Plain(NO));
            }
        }
        if !(still && press.accept) {
            if press.back && self.cursor != 0 {
                self.cursor = 0;
                snap = true;
            }
        } else {
            let rng = &mut *cx.rng;
            match buy(&items[self.cursor], state, bonus, bought, || random(rng, 4) as i32 + 1) {
                Purchase::Refused => cx.sounds.push(ShopSound::Plain(NO)),
                done => {
                    exit = done == Purchase::Exit;
                    changed = true;
                    if self.cursor != 0 {
                        cx.sounds.push(ShopSound::Plain(GOLD_SOUND));
                        info!("shop: bought {:?} for {}; {} gold left", items[self.cursor].text, items[self.cursor].price, state.gold);
                    }
                }
            }
        }
        if changed {
            self.mark(items, state, &raw(bonus));
            self.flash[self.cursor] = FLASH_FIELDS;
            while self.cursor > 0
                && (i64::from(items[self.cursor].price) > i64::from(state.gold) || self.skip[self.cursor])
            {
                self.cursor -= 1;
            }
            if (self.cursor as i64) < self.top {
                self.top = self.cursor as i64;
                self.length = last_reachable(items, state) + 1;
                snap = true;
            }
        }
        if !snap {
            let d = self.cursor as i64 - self.top;
            if d >= 7 {
                self.top = self.cursor as i64 - 6;
            } else if d < 0 {
                self.top += d;
            } else if row < d {
                if row > 2 && (self.length as i64 - 1) - (self.top + 6) > 0 {
                    self.top += 1;
                }
            } else if d < row && row < 4 && self.top > 0 {
                self.top -= 1;
            }
        }

        // The items move toward their places.
        let speed = if snap || !self.placed { -1.0 } else { speed.max(SCROLL_MIN) };
        self.placed = true;
        let mut moved = false;
        for (y, target) in self.ys.iter_mut().zip(list_targets(cx.offsets, self.cursor)) {
            if (*y - target).abs() <= 1.0 {
                continue;
            }
            if speed < 0.0 {
                *y = target;
            } else {
                let step = speed * fields;
                *y = if *y > target { (*y - step).max(target) } else { (*y + step).min(target) };
                moved = true;
            }
        }
        for f in &mut self.flash {
            if *f >= 1.0 {
                *f = (*f - fields).max(0.0);
            }
        }
        self.speed = if speed >= 1.0 && moved { speed } else { 0.0 };
        exit
    }
}

/// The game's `random(n)`: 0..n (an xorshift stands in for its generator).
fn random(rng: &mut u32, n: u32) -> u32 {
    let mut x = if *rng == 0 { 0x9E37_79B9 } else { *rng };
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *rng = x;
    if n == 0 { 0 } else { x % n }
}

// ---------------------------------------------------------------------------
// The stats pages

/// The pages' lights: when the first starts (the level-up page, the stats
/// page after the shop) and how long each lasts, in fields.
const LEVEL_UP_START: f32 = 90.0;
const STATS_START: f32 = 30.0;
const LIGHT: f32 = 60.0;
/// Each level adds this much to the maximum health.
const HEALTH_PER_LEVEL: f32 = 100.0;

#[derive(Clone, Debug, Default, PartialEq)]
struct StatsPage {
    after_shop: bool,
    level: u32,
    before: [f32; 4],
    after: [f32; 4],
    health_before: f32,
    health_after: f32,
    /// When each changed stat lights up, and the health rows.
    lights: [Option<f32>; 4],
    health_light: Option<f32>,
    /// The prompt shows, and A leaves, from here.
    end: f32,
}

/// The level-up page (`after_shop` false) or the stats page after the
/// shop: the raw stats before — at the level before with the points kept
/// as the level began, less this visit's — and now; each that changed
/// lights up in turn.
fn stats_page(
    after_shop: bool,
    stats: Option<&PlayerStats>,
    summoner: bool,
    (level_before, level): (u32, u32),
    (kept, bonus, bought): (&StatBonus, &StatBonus, &StatBonus),
    max_health: f32,
) -> StatsPage {
    let level_before = if after_shop { level } else { level_before };
    let bought = bought.as_array();
    let mut before = raw_stats(stats, summoner, level_before, kept);
    for (b, &k) in before.iter_mut().zip(&bought) {
        *b -= k;
    }
    let after = raw_stats(stats, summoner, level, bonus);
    let mut at = if after_shop { STATS_START } else { LEVEL_UP_START };
    let mut lights = [None; 4];
    for (light, (b, a)) in lights.iter_mut().zip(before.iter().zip(&after)) {
        if b != a {
            *light = Some(at);
            at += LIGHT;
        }
    }
    let health_light = (!after_shop).then(|| {
        let start = at;
        at += LIGHT;
        start
    });
    StatsPage {
        after_shop,
        level,
        before,
        after,
        health_before: max_health - HEALTH_PER_LEVEL * (level as f32 - level_before as f32),
        health_after: max_health,
        lights,
        health_light,
        end: at,
    }
}

/// The rank under the level: `LEGEND` at 99, the class's name below 10,
/// else the class's `CLASS_RANK` group at (level ÷ 10) ÷ 2.
fn rank_text(text: &TextRom, class: usize, level: u32) -> Option<String> {
    let rank = if level >= MAX_LEVEL {
        text.get("LEGEND", 0)?
    } else if level < 10 {
        text.get("PLAYER_CLASS_LC", class)?
    } else {
        let list = text.list("CLASS_RANK")?;
        let group = text.groups.get(*list.groups.get(class)?)?;
        group.strings.get((level / 10 / 2) as usize)?.as_str()
    };
    Some(rank.to_string())
}

/// The final stats' totals for a hero (the game's per-class counters):
/// From the hero's record (`PlayerState::kills` …), passed in by the front end.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FinalStats {
    pub kills: u32,
    pub generators: u32,
    pub gold: u32,
    /// Play time, seconds.
    pub seconds: f32,
}

/// The final stats' lines light up from these fields, 60 each, and the
/// prompt comes after the last.
const FINAL_LIGHTS: [f32; 4] = [90.0, 150.0, 210.0, 270.0];
const FINAL_END: f32 = 330.0;
/// levelH4 (realm 8, level 3) ends with the final stats.
const FINAL_LEVEL: (u32, u32) = (8, 3);

// ---------------------------------------------------------------------------
// The inventory

/// The inventory flies in over 120 fields and out over 15.
const INVENTORY_IN: f32 = 120.0;
const INVENTORY_OUT: f32 = 15.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Inventory {
    open: bool,
    closing: bool,
    timer: f32,
}

/// The inventory's pictures, x and y from the column's corner: the window,
/// the gargoyle pieces (with counts) and the crystals (counters 1–8).
const WINDOW: (&str, f32, f32) = ("WINDOW_EMPTY", 0.0, 0.0);
const GARGOYLE_PIECES: [(&str, f32, f32); 3] = [("FANGS", 56.0, 116.0), ("FEATHER", 56.0, 140.0), ("CLAW", 56.0, 164.0)];
const CRYSTALS: [(&str, f32, f32); 8] = [
    ("ORANGE_CRYSTLE", 6.0, 200.0),
    ("RED_CRYSTLE", 6.0, 216.0),
    ("PURPLE_CRYSTLE", 6.0, 232.0),
    ("CYAN_CRYSTLE", 6.0, 248.0),
    ("GREEN_CRYSTLE", 64.0, 200.0),
    ("YELLOW_CRYSTLE", 64.0, 216.0),
    ("WHITE_CRYSTLE", 64.0, 232.0),
    ("BLACK_CRYSTLE", 64.0, 248.0),
];
/// The legendary items by realm id: `<name>` held, `<name>_EMPTY` not.
const LEGENDARY: [Option<(&str, f32, f32)>; 12] = [
    None,
    Some(("SCIMITAR", 96.0, 32.0)),
    Some(("ICE_AX", 76.0, 32.0)),
    Some(("LAMP", 94.0, 58.0)),
    Some(("BILLOWS", 77.0, 56.0)),
    Some(("SOUL_SAVIOR", 98.0, 82.0)),
    None,
    Some(("BOOK", 54.0, 32.0)),
    None,
    Some(("FIRE_SCROLL", 56.0, 80.0)),
    Some(("LANTERN", 77.0, 81.0)),
    Some(("JAVILIN", 54.0, 56.0)),
];
/// The shards' pieces of the window, by their bit of the boss marks.
const PIECES: [Option<(&str, f32, f32)>; 9] = [
    None,
    Some(("LITCH_PIECE", 0.0, 86.0)),
    Some(("DRAGON_PIECE", 0.0, 124.0)),
    Some(("CHIMERA_PIECE", 0.0, 167.0)),
    Some(("PLAGUE_PIECE", 22.0, 131.0)),
    Some(("DRYDER_PIECE", 0.0, 106.0)),
    Some(("GENIE_PIECE", 25.0, 72.0)),
    Some(("YETTI_PIECE", 0.0, 148.0)),
    Some(("WRAITH_PIECE", 0.0, 63.0)),
];
/// Counts beside the gargoyle pieces and crystals: offset and scale.
const GARGOYLE_COUNT: (f32, f32, f32) = (30.0, 2.0, 0.5);
const CRYSTAL_COUNT: (f32, f32, f32) = (28.0, 2.0, 0.35);

/// A picture's place, size and transparency as the inventory opens (`t`
/// 0 → 1) or closes (`t` 0 → 1), at its own size `w` × `h`.
fn inventory_look(x: f32, y: f32, w: f32, h: f32, col: f32, inv: &Inventory) -> (Vec2, Vec2, f32) {
    if inv.closing {
        let t = inv.timer / INVENTORY_OUT;
        let pos = Vec2::new((10.0 * (x - 64.0) * t + x + col).trunc(), (10.0 * (y - 180.0) * t + y).trunc());
        let size = Vec2::new((5.0 * w * t + w).trunc(), (5.0 * h * t + h).trunc());
        (pos, size, (255.0 * t).trunc())
    } else if inv.open {
        (Vec2::new(x + col, y), Vec2::new(w, h), 0.0)
    } else {
        let t = inv.timer / INVENTORY_IN;
        let spin = (x + y + (180.0 * t * t * t).trunc()) as i32;
        let angle = (spin.rem_euclid(60) as f32 * 6.0).to_radians();
        let r = (1.0 - t) * 60.0;
        // Where it's drawn only: the screen's state doesn't read it.
        let pos = Vec2::new((r * angle.dcos() + x + col).trunc(), (r * angle.dsin() + y).trunc());
        let grow = 1.0 - t * t;
        (pos, Vec2::new((w * grow + w).trunc(), (h * grow + h).trunc()), (255.0 * grow).trunc())
    }
}

/// The counts' transparency as the inventory opens or closes.
fn inventory_count_fade(inv: &Inventory) -> f32 {
    if inv.closing {
        (255.0 * inv.timer / INVENTORY_OUT).trunc()
    } else if inv.open {
        0.0
    } else {
        (255.0 * (1.0 - inv.timer / INVENTORY_IN)).trunc()
    }
}

// ---------------------------------------------------------------------------
// The screen

/// Which screen: the after-level one, or the Tower Menu's Shop or
/// Inventory (the game's three kinds of the screen, 0, 1, 2 — `docs/shop.md`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShopKind {
    #[default]
    AfterLevel,
    Shop,
    Inventory,
}

/// How a frame of the screen ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopOutcome {
    /// Still going.
    Continue,
    /// Over: the next level — the tower, which after the Shop or Inventory
    /// starts again (the game's next-level start). After a level with every
    /// hero out it's this at once.
    Done,
    /// Over after a level: the select screen with each player who took
    /// part ([`ShopScreen::took_part`]) at the character menu, the cursor
    /// on Done (Change, Load and Quit disabled after levelH4), the empty
    /// columns free to join; when everyone is ready, the tower.
    SelectScreen,
}

/// One player's presses this frame (edges): the game's A, X, B and the
/// pad's up and down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShopPress {
    pub up: bool,
    pub down: bool,
    /// A: go on, buy.
    pub accept: bool,
    /// X: sell.
    pub sell: bool,
    /// B: the cursor to EXIT.
    pub back: bool,
}

/// What the screen opens with.
#[derive(Clone, Debug, Default)]
pub struct ShopOpen {
    pub kind: ShopKind,
    /// The level last played outside the tower (`levelG1`): after a level,
    /// the tally's full marks and sound, and levelH4's final stats.
    pub level: Option<String>,
    /// Each slot's record as that level began (the game's kept copy): what
    /// the tally counts from, and the level before.
    pub level_start: [Option<PlayerState>; MAX_PLAYERS],
    /// After a level, the heroes out of it (dead): they take no part.
    pub out: [bool; MAX_PLAYERS],
    /// Monsters killed and generators destroyed in the level, by slot.
    pub kills: [u32; MAX_PLAYERS],
    /// Points bought in the shop so far, by slot, and as the level began
    /// (none: the same).
    pub bonus: [StatBonus; MAX_PLAYERS],
    pub kept_bonus: Option<[StatBonus; MAX_PLAYERS]>,
    /// After levelH4: each hero's totals.
    pub totals: [FinalStats; MAX_PLAYERS],
}

impl ShopOpen {
    pub fn new(kind: ShopKind) -> Self {
        Self { kind, ..default() }
    }
}

/// A player's step (the game's `+0xA64`, `docs/shop.md`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// The heaps rise; A once they're up.
    #[default]
    Tally,
    LevelUp,
    FinalStats,
    List,
    /// The stats after shopping.
    Stats,
    Inventory,
    Closing,
    /// This player waits for the others.
    Done,
}

#[derive(Clone, Debug, Default)]
struct Slot {
    step: Step,
    /// Fields on this page (the game's `+0xA6C`).
    t: f32,
    colour: usize,
    /// The class's place in the class order for its text (the summoner's
    /// is the wizard's), and in the original eight.
    text_class: usize,
    summoner: bool,
    stats: Option<PlayerStats>,
    level_before: u32,
    kept: StatBonus,
    bonus: StatBonus,
    /// This visit's stat buys (the game's `+0xA70…`).
    bought: StatBonus,
    tally: Tally,
    page: StatsPage,
    list: List,
    inventory: Inventory,
    totals: FinalStats,
    rank: String,
    /// The magic potion line at levels 25 and 50.
    magic: Option<String>,
}

/// A sound the screen asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ShopSound {
    /// Centred, at the call's own volume.
    Plain(&'static str),
    /// Centred, louder (`S_STNDGLASS` at `0xFF`).
    Loud(&'static str, u8),
    /// The tally's sound, again whenever it isn't playing (some), or
    /// stopped (none).
    Tally(Option<String>),
    /// The announcer's sentence for a level gained: the hero's name line,
    /// then `S_HAS`, `S_GAINEDLEVEL`.
    GainedLevel(String),
}

/// The screen (a resource): closed until [`ShopScreen::open`].
#[derive(Resource, Default)]
pub struct ShopScreen {
    kind: Option<ShopKind>,
    slots: [Option<Slot>; MAX_PLAYERS],
    items: Vec<ShopItem>,
    icons: Vec<bool>,
    offsets: Vec<f32>,
    tally_sound: Option<String>,
    tally_playing: bool,
    final_level: bool,
    /// Fields since it opened (the glowing text's pulse).
    t: f32,
    rng: u32,
    sounds: Vec<ShopSound>,
}

/// Sounds.
const SELECT: &str = "S_OPTMENUSEL";
const NO: &str = "S_NO";
const GOLD_SOUND: &str = "S_PICKUPMAGIC";
const TICK_DOWN: &str = "S_SECRETCLOCK2";
const TICK_UP: &str = "S_SECRETCLOCK1";
const GLASS: &str = "S_STNDGLASS";
const GLASS_VOLUME: u8 = 0xFF;
const TALLY_VOLUME: u8 = 0xE0;
const TALLY_CHANNEL: &str = "shop_tally";
/// The level-up sentence: the hero's name, these, at most 3 s away.
const GAINED: [&str; 2] = ["S_HAS", "S_GAINEDLEVEL"];
const GAINED_MOST_WAIT: f32 = 3.0;
const POJO: u32 = 0x400;
const POJO_NAME: &str = "S_POJO2";

/// The colours' order (`PlayerChoice::variant`).
const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
/// The summoner's place in the class order.
const SUMMONER: usize = 16;
/// The wizard's: the summoner plays as one.
const WIZARD: usize = 2;

/// The tally's sound for the realm just played: `S_TALLYSFX<letter>` for
/// realms 1–11 (A–K).
fn tally_sound(realm: u32) -> Option<String> {
    (1..=11).contains(&realm).then(|| format!("S_TALLYSFX{}", (b'A' + (realm - 1) as u8) as char))
}

impl ShopScreen {
    pub fn is_open(&self) -> bool {
        self.kind.is_some()
    }

    /// Whether `slot`'s player took part in the screen last opened.
    pub fn took_part(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(Option::is_some)
    }

    /// `slot`'s bought stat points, with this visit's: give them back to
    /// the hero when the screen ends.
    pub fn bonus(&self, slot: usize) -> Option<StatBonus> {
        self.slots.get(slot)?.as_ref().map(|s| s.bonus)
    }

    /// Opens the screen for the players in `party`: after a level those
    /// still in it, else everyone.
    pub fn open(&mut self, data: &ShopData, party: &Party, open: ShopOpen) {
        let after_level = open.kind == ShopKind::AfterLevel;
        let level = open.level.as_deref().and_then(crate::quest::level_of);
        let marks = open.level.as_deref().and_then(|l| data.marks.get(&l.to_ascii_lowercase()).copied()).unwrap_or_default();
        let icons: Vec<bool> =
            data.items.iter().map(|i| !i.icon.is_empty() && data.icons.contains(&i.icon.to_ascii_uppercase())).collect();
        let rng = if self.rng == 0 { 0x2545_F491 } else { self.rng };
        *self = Self {
            kind: Some(open.kind),
            offsets: item_offsets(&data.items, &icons),
            items: data.items.clone(),
            icons,
            tally_sound: level.filter(|_| after_level).and_then(|(realm, _)| tally_sound(realm)),
            final_level: after_level && level == Some(FINAL_LEVEL),
            rng,
            ..default()
        };
        for (slot, member) in party.members() {
            let state = &member.state;
            if after_level && (open.out[slot] || !state.alive) {
                continue;
            }
            let start = open.level_start[slot].as_ref();
            let code = member.choice.class.to_ascii_uppercase();
            let class = crate::character::class_index(&code).unwrap_or(0);
            let summoner = class == SUMMONER;
            let text_class = if summoner { WIZARD } else { class };
            let variant = member.choice.variant.to_ascii_uppercase();
            let gained = |now: u32, then: u32| i64::from(now) - i64::from(then);
            let amounts = [
                start.map_or(0, |s| gained(state.gold, s.gold)),
                i64::from(open.kills[slot]),
                start.map_or(0, |s| gained(state.experience, s.experience)),
            ];
            let text = data.text.as_ref();
            let magic_group = match state.level {
                25 => Some("MAGIC_ATT1"),
                50 => Some("MAGIC_ATT2"),
                _ => None,
            };
            let base_class = if text_class > 7 { text_class - 8 } else { text_class };
            let mut s = Slot {
                colour: COLOURS.iter().position(|c| variant.starts_with(c)).unwrap_or(0),
                text_class,
                summoner,
                stats: data.classes.get(&code).copied(),
                level_before: start.map_or(state.level, |s| level_for(s.experience)),
                bonus: open.bonus[slot],
                kept: open.kept_bonus.map_or(open.bonus[slot], |k| k[slot]),
                tally: Tally::new(amounts, marks),
                totals: open.totals[slot],
                rank: text.and_then(|t| rank_text(t, text_class, state.level)).unwrap_or_default(),
                magic: text.zip(magic_group).and_then(|(t, g)| t.get(g, base_class)).map(str::to_string),
                ..default()
            };
            s.step = match open.kind {
                ShopKind::AfterLevel => Step::Tally,
                ShopKind::Shop => Step::List,
                ShopKind::Inventory => Step::Inventory,
            };
            if s.step == Step::List {
                s.list = self.new_list(&s, state);
            }
            self.slots[slot] = Some(s);
        }
        info!(
            "the {:?} screen opens for players {:?}",
            open.kind,
            (0..MAX_PLAYERS).filter(|&p| self.took_part(p)).map(|p| p + 1).collect::<Vec<_>>()
        );
    }

    fn new_list(&self, s: &Slot, state: &PlayerState) -> List {
        let raw = raw_stats(s.stats.as_ref(), s.summoner, state.level, &s.bonus);
        List::new(&self.items, state, &raw, s.tally.heights[GOLD])
    }

    /// Closes the screen (its bought points stay readable).
    pub fn close(&mut self) {
        if self.tally_playing {
            self.sounds.push(ShopSound::Tally(None));
            self.tally_playing = false;
        }
        self.kind = None;
    }

    /// The sounds asked for since last taken.
    pub fn take_sounds(&mut self) -> Vec<ShopSound> {
        std::mem::take(&mut self.sounds)
    }
}

/// One frame of the screen: `fields` elapsed (60 a second), each slot's
/// presses. The players' records change as they buy and sell.
pub fn tick(screen: &mut ShopScreen, fields: f32, presses: &[ShopPress; MAX_PLAYERS], party: &mut Party) -> ShopOutcome {
    let Some(kind) = screen.kind else { return ShopOutcome::Done };
    screen.t += fields;
    let mut rising = false;
    for (slot, &press) in presses.iter().enumerate() {
        let Some(mut s) = screen.slots[slot].take() else { continue };
        match party.state_mut(slot) {
            Some(state) => {
                step(screen, kind, &mut s, state, press, fields);
                rising |= s.step == Step::Tally && s.tally.rising.is_some();
            }
            // Gone from the party: nothing more for them.
            None => s.step = Step::Done,
        }
        screen.slots[slot] = Some(s);
    }
    // The tally's sound: asked for every frame a heap rises, stopped after.
    if rising && let Some(name) = &screen.tally_sound {
        screen.sounds.push(ShopSound::Tally(Some(name.clone())));
    } else if !rising && screen.tally_playing {
        screen.sounds.push(ShopSound::Tally(None));
    }
    screen.tally_playing = rising;
    if screen.slots.iter().flatten().all(|s| s.step == Step::Done) {
        let anyone = screen.slots.iter().any(Option::is_some);
        screen.close();
        return if kind == ShopKind::AfterLevel && anyone { ShopOutcome::SelectScreen } else { ShopOutcome::Done };
    }
    ShopOutcome::Continue
}

/// One player's frame.
fn step(screen: &mut ShopScreen, kind: ShopKind, s: &mut Slot, state: &mut PlayerState, press: ShopPress, fields: f32) {
    let go_on = |screen: &mut ShopScreen| screen.sounds.push(ShopSound::Plain(SELECT));
    match s.step {
        Step::Tally => {
            s.tally.rise(fields);
            if s.tally.rising.is_none() && press.accept {
                go_on(screen);
                s.t = 0.0;
                enter_level_up(screen, s, state);
            }
        }
        Step::LevelUp | Step::Stats => {
            if s.t == 0.0 {
                if s.step == Step::LevelUp {
                    screen.sounds.push(ShopSound::GainedLevel(name_line(s, state)));
                }
                s.t = 1.0;
            }
            s.t += fields;
            if s.t >= s.page.end && press.accept {
                go_on(screen);
                s.t = 0.0;
                if s.step == Step::LevelUp {
                    after_level_up(screen, s, state);
                } else if kind == ShopKind::Shop {
                    s.step = Step::Done;
                } else {
                    open_inventory(screen, s);
                }
            }
        }
        Step::FinalStats => {
            s.t += fields;
            if s.t > FINAL_END - 1.0 && press.accept {
                go_on(screen);
                s.t = 0.0;
                s.step = Step::Done;
            }
        }
        Step::List => {
            let mut cx = ListCx {
                items: &screen.items,
                offsets: &screen.offsets,
                stats: s.stats.as_ref(),
                summoner: s.summoner,
                sounds: &mut screen.sounds,
                rng: &mut screen.rng,
            };
            if s.list.frame(&mut cx, state, &mut s.bonus, &mut s.bought, press, fields) {
                s.step = Step::Stats;
                s.page = page_for(true, s, state);
            }
        }
        Step::Inventory => {
            let inv = &mut s.inventory;
            if !inv.open {
                inv.timer += fields;
                inv.open = inv.timer > INVENTORY_IN - 1.0;
            }
            if press.accept {
                go_on(screen);
                s.inventory = Inventory { open: false, closing: true, timer: 0.0 };
                s.step = Step::Closing;
            }
        }
        Step::Closing => {
            s.inventory.timer += fields;
            if s.inventory.timer > INVENTORY_OUT - 1.0 {
                s.step = Step::Done;
            }
        }
        Step::Done => {}
    }
}

fn page_for(after_shop: bool, s: &Slot, state: &PlayerState) -> StatsPage {
    stats_page(
        after_shop,
        s.stats.as_ref(),
        s.summoner,
        (s.level_before, state.level),
        (&s.kept, &s.bonus, &s.bought),
        state.max_health(),
    )
}

/// After the tally: the level-up page, straight through when no level was
/// gained.
fn enter_level_up(screen: &mut ShopScreen, s: &mut Slot, state: &PlayerState) {
    if s.level_before == state.level {
        after_level_up(screen, s, state);
    } else {
        s.step = Step::LevelUp;
        s.page = page_for(false, s, state);
    }
}

/// After the level-up page: the final stats after levelH4, else the shop.
fn after_level_up(screen: &mut ShopScreen, s: &mut Slot, state: &PlayerState) {
    if screen.final_level {
        s.step = Step::FinalStats;
    } else {
        s.step = Step::List;
        s.list = screen.new_list(s, state);
    }
}

fn open_inventory(screen: &mut ShopScreen, s: &mut Slot) {
    screen.sounds.push(ShopSound::Loud(GLASS, GLASS_VOLUME));
    s.inventory = Inventory::default();
    s.step = Step::Inventory;
}

/// The hero's name as the announcer says it first ("Blue Warrior"), or
/// the Pojo's.
fn name_line(s: &Slot, state: &PlayerState) -> String {
    if state.bits.special & POJO != 0 {
        POJO_NAME.to_string()
    } else {
        Rank { level: state.level, class: s.text_class, colour: s.colour }.name_line()
    }
}

/// The announcer's "<Blue Warrior> has gained a level".
fn gained_sentence(name: String) -> QueueVoice {
    GAINED.iter().fold(QueueVoice::announcer(name, GAINED_MOST_WAIT).gated(), |v, line| v.then(*line))
}

// ---------------------------------------------------------------------------
// Data

/// What the screen reads from the disc: the shop's items, the classes'
/// stats, each level's full marks (gold, kills, experience) and the text.
#[derive(Resource, Default)]
pub struct ShopData {
    pub items: Vec<ShopItem>,
    /// Icon textures there are (`SELECT/`, `INVENTORY/`), upper case.
    icons: HashSet<String>,
    classes: HashMap<String, PlayerStats>,
    /// By lower-case level folder (`levelg1`).
    marks: HashMap<String, [i32; 3]>,
    text: Option<TextRom>,
}

/// The in-game power icons (`INVENTORY/`): three of the shop's icons are
/// only there.
#[derive(Resource)]
pub struct ShopTextures(pub UiTextures);

/// The class codes in the game's order.
const CLASS_CODES: [&str; 17] =
    ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR", "UNI", "MED", "HYE", "SUM"];

/// A level record's length, and where its full marks are.
const LEVEL_LEN: usize = 0x10C;
const MARKS_AT: usize = 0xE0;

/// Each level's full marks from a realm's world data (`LEVL` records:
/// `+0x08` name, `+0xE0` gold, `+0xE4` kills, `+0xE8` experience).
pub fn parse_marks(bytes: &[u8]) -> Vec<(String, [i32; 3])> {
    let Ok(file) = ChunkFile::parse(bytes) else { return Vec::new() };
    let Some(records) = file.records("LEVL", LEVEL_LEN) else { return Vec::new() };
    records
        .map(|r| {
            let int = |at: usize| i32::from_le_bytes(r[at..at + 4].try_into().unwrap());
            (format!("level{}", cstr(&r[0x08..0x18])), [int(MARKS_AT), int(MARKS_AT + 4), int(MARKS_AT + 8)])
        })
        .collect()
}

fn load_shop(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let install = &mut game.install;
    let items = install.read("SHPDATA/SHOP.WAD").ok().and_then(|b| parse_items(&b)).unwrap_or_default();
    if items.is_empty() {
        warn!("SHPDATA/SHOP.WAD didn't load: the shop has nothing for sale");
    }
    let mut icons = HashSet::new();
    for dir in ["SELECT", "INVENTORY"] {
        if let Some(model) = install.read(&format!("{dir}/objects.ngc")).ok().and_then(|b| ModelFile::parse(&b).ok()) {
            icons.extend(model.texture_names.iter().map(|t| t.name.to_ascii_uppercase()));
        }
    }
    let classes: HashMap<String, PlayerStats> = CLASS_CODES
        .iter()
        .filter_map(|&c| {
            let bytes = install.read(&format!("PDATA/{c}.WAD")).ok()?;
            Some((c.to_string(), PlayerStats::parse(&bytes).ok().flatten()?))
        })
        .collect();
    let wads: Vec<String> = install
        .files()
        .iter()
        .filter(|f| {
            let f = f.to_ascii_uppercase();
            f.starts_with("WDATA/") && f.ends_with(".WAD")
        })
        .cloned()
        .collect();
    let mut marks = HashMap::new();
    for path in wads {
        if let Ok(bytes) = install.read(&path) {
            marks.extend(parse_marks(&bytes).into_iter().map(|(name, m)| (name.to_ascii_lowercase(), m)));
        }
    }
    let text = install.read("TEXT/ENGLISH.ROM").ok().and_then(|b| TextRom::parse(&b).ok());
    info!("the shop: {} items, {} classes, {} levels' marks", items.len(), classes.len(), marks.len());
    commands.insert_resource(ShopData { items, icons, classes, marks, text });
    commands.insert_resource(ShopTextures(UiTextures::load(install, &["INVENTORY"])));
}

fn play_sounds(
    mut screen: ResMut<ShopScreen>,
    mut plain: MessageWriter<PlaySound>,
    mut loud: MessageWriter<PlaySoundAt>,
    mut loops: MessageWriter<LoopSoundAt>,
    mut voices: MessageWriter<QueueVoice>,
) {
    if screen.sounds.is_empty() {
        return;
    }
    for sound in screen.take_sounds() {
        match sound {
            ShopSound::Plain(name) => {
                plain.write(PlaySound(name.to_string()));
            }
            ShopSound::Loud(name, volume) => {
                loud.write(PlaySoundAt::centred(name, volume));
            }
            ShopSound::Tally(Some(name)) => {
                loops.write(LoopSoundAt { key: TALLY_CHANNEL, slot: 0, name: Some(name), at: None, volume: TALLY_VOLUME, follow_volume: false });
            }
            ShopSound::Tally(None) => {
                loops.write(LoopSoundAt::stop(TALLY_CHANNEL));
            }
            ShopSound::GainedLevel(name) => {
                voices.write(gained_sentence(name));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing

/// The columns' left edges and centres.
const COLUMN: [f32; 4] = [0.0, 128.0, 256.0, 384.0];
const CENTRE: [f32; 4] = [64.0, 192.0, 320.0, 448.0];
/// The pages' titles, centred at y 8 on the coloured plate.
const TITLE_Y: f32 = 8.0;
const TITLE_SCALE: f32 = 0.45;
/// The glowing words' colour and the plain ones'.
const PINK: [u8; 3] = [0xFF, 0x80, 0xC0];
/// The A button's picture.
const BUTTON_A: &str = "BUTTON_X";

fn draw_screen(
    screen: Res<ShopScreen>,
    party: Res<Party>,
    mut draw2d: ResMut<Draw2d>,
    fonts: Option<Res<GameFonts>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut extra: Option<ResMut<ShopTextures>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(fonts), Some(tex)) = (fonts, tex.as_deref_mut()) else { return };
    let mut p = Paint {
        d: &mut draw2d,
        fonts: &fonts,
        tex,
        extra: extra.as_deref_mut().map(|e| &mut e.0),
        images: &mut images,
        pulse: crate::frontend::pulse(screen.t),
    };
    draw(&screen, &party, &mut p);
}

/// The 2D drawing the screen does: the textures it looks for, the fonts,
/// and the glowing text's pulse.
pub struct Paint<'a> {
    pub d: &'a mut Draw2d,
    pub fonts: &'a GameFonts,
    pub tex: &'a mut UiTextures,
    /// Where else to look for a texture (`INVENTORY/`).
    pub extra: Option<&'a mut UiTextures>,
    pub images: &'a mut Assets<Image>,
    pub pulse: f32,
}

impl Paint<'_> {
    fn texture(&mut self, name: &str) -> Option<UiImage> {
        self.tex.get(name, self.images).or_else(|| self.extra.as_deref_mut()?.get(name, self.images))
    }

    fn image(&mut self, name: &str, x: f32, y: f32, color: Color) {
        if let Some(i) = self.texture(name) {
            self.d.image(&i, x, y, i.size.x, i.size.y, color);
        }
    }

    fn sized(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, color: Color) {
        if let Some(i) = self.texture(name) {
            self.d.image(&i, x, y, w, h, color);
        }
    }

    fn text(&mut self, scale: f32, x: f32, y: f32, color: Color, text: &str) {
        self.d.text(self.fonts, &TextStyle::new(FONT32, scale, color), x, y, text);
    }

    /// The game's glowing text (`docs/shop.md`).
    fn glow(&mut self, scale: f32, x: f32, y: f32, text: &str) {
        self.d.shimmer(self.fonts, FONT32, scale, x, y, text, crate::frontend::glow_colour(), self.pulse);
    }

    /// Text over several lines, `glow`ing or in `color`; returns its height.
    fn lines(&mut self, scale: f32, x: f32, y: f32, glow: bool, color: Color, text: &str) -> f32 {
        let step = self.fonts.line_height(FONT32, scale).trunc();
        let mut at = y;
        for line in text.split('\n').take(MAX_LINES) {
            if glow { self.glow(scale, x, at, line) } else { self.text(scale, x, at, color, line) }
            at += step;
        }
        at - y
    }

    fn title(&mut self, slot: usize, title: &str) {
        self.text(TITLE_SCALE, -CENTRE[slot], TITLE_Y, Color::BLACK, title);
    }

    /// A button and "Continue" at `(x, y)` (the A button `size` square).
    fn prompt(&mut self, slot: usize, (dx, y): (f32, f32), size: f32, (text_dx, text_y): (f32, f32), text: &str) {
        let col = COLUMN[slot];
        self.sized(BUTTON_A, col + dx, y, size, size, Color::WHITE);
        self.glow(0.5, col + text_dx, text_y, text);
    }
}

/// The game's transparency (0 opaque, 255 almost clear) as an alpha.
fn alpha(transparency: f32) -> f32 {
    (1.0 - transparency / 256.0).clamp(0.0, 1.0)
}

/// Draws the screen: four columns, each player's page in theirs.
pub fn draw(screen: &ShopScreen, party: &Party, p: &mut Paint) {
    // The HUD row's backing, as the select screen draws it.
    for (slot, &col) in COLUMN.iter().enumerate() {
        let [r, g, b] =
            if party.get(slot).is_some() { crate::game_hud::JOINED[slot] } else { crate::game_hud::NOT_JOINED[slot] };
        p.sized("S3", col, 304.0, 128.0, 16.0, Color::WHITE);
        p.sized("S4", col, 320.0, 128.0, 64.0, Color::srgb_u8(r, g, b));
        p.sized("S4_FRAME", col, 320.0, 128.0, 64.0, Color::WHITE);
    }
    for (slot, &col) in COLUMN.iter().enumerate() {
        p.image(&format!("S1_PLYR{}", slot + 1), col, 0.0, Color::WHITE);
        p.image(&format!("S2_PLYR{}", slot + 1), col, 256.0, Color::WHITE);
    }
    // Behind the borders: the heaps and the scrolls.
    for (slot, s) in screen.slots.iter().enumerate() {
        let Some(s) = s else { continue };
        match s.step {
            Step::Tally => {
                // Tallest at the back (the game gives the three one depth).
                let shown = s.tally.rising.map_or(3, |r| r + 1);
                for &heap in &s.tally.rows[..shown] {
                    draw_heap(p, HEAPS[heap], COLUMN[slot], s.tally.shown[heap]);
                }
            }
            Step::List => {
                if party.state(slot).is_some_and(|st| st.gold > 0) {
                    draw_heap(p, HEAPS[GOLD], COLUMN[slot], s.list.heap);
                }
                p.image("SHOP_SCROLL_1", COLUMN[slot], 0.0, Color::WHITE);
                p.image("SHOP_SCROLL_2", COLUMN[slot], 256.0, Color::WHITE);
            }
            _ => {}
        }
    }
    for &col in &COLUMN {
        p.image("S1_BORDER", col, 0.0, Color::WHITE);
        p.image("S2_BORDER", col, 256.0, Color::WHITE);
    }
    for (slot, member) in party.members() {
        let colour = COLOURS.iter().position(|c| member.choice.variant.to_ascii_uppercase().starts_with(c)).unwrap_or(0);
        p.image(&format!("SHOP_TOP_{}", COLOURS[colour]), COLUMN[slot] + 32.0, 0.0, Color::WHITE);
    }
    for (slot, s) in screen.slots.iter().enumerate() {
        let (Some(s), Some(state)) = (s, party.state(slot)) else { continue };
        match s.step {
            Step::Tally => draw_tally(p, slot, s),
            Step::LevelUp | Step::Stats => draw_stats(p, slot, s),
            Step::FinalStats => draw_final(p, slot, s),
            Step::List => draw_list(p, screen, slot, s),
            Step::Inventory | Step::Closing => draw_inventory(p, slot, s, state),
            Step::Done => {}
        }
    }
}

/// A heap `h` high: its texture's top `h` rows, standing on the floor
/// line (no taller than its texture).
fn draw_heap(p: &mut Paint, name: &str, col: f32, h: f32) {
    let Some(i) = p.texture(name) else { return };
    let h = h.min(i.size.y).max(0.0);
    p.d.quads.push(Quad::Image {
        image: i.handle.clone(),
        rect: Some(Rect::new(0.0, 0.0, i.size.x, h)),
        pos: Vec2::new(col, HEAP_FLOOR - h),
        size: Vec2::new(i.size.x, h),
        color: Color::WHITE,
    });
}

fn draw_tally(p: &mut Paint, slot: usize, s: &Slot) {
    let col = COLUMN[slot];
    let rising = s.tally.rising.map(|r| s.tally.rows[r]);
    for heap in [GOLD, KILLS, EXPERIENCE] {
        let words = format!("{}: {}", TALLY_WORDS[heap], s.tally.amounts[heap]);
        if rising == Some(heap) {
            p.glow(0.5, col + 16.0, TALLY_WORDS_Y[heap], &words);
        } else {
            p.text(0.5, col + 16.0, TALLY_WORDS_Y[heap], Color::WHITE, &words);
        }
    }
    if rising.is_none() {
        p.prompt(slot, (16.0, 89.0), 20.0, (32.0, 92.0), " Continue");
    }
    p.title(slot, "Stats");
}

const STAT_WORDS: [&str; 4] = ["Strength", "Armor", "Magic", "Speed"];
const STAT_Y: [f32; 4] = [96.0, 116.0, 136.0, 156.0];
const STAT_SCALE: f32 = 0.48;

fn draw_stats(p: &mut Paint, slot: usize, s: &Slot) {
    let (col, page, t) = (COLUMN[slot], &s.page, s.t);
    let (words_x, values_x) = (col + 8.0, col + 88.0);
    let level = format!("Level {}", page.level);
    if page.after_shop {
        p.text(0.75, -CENTRE[slot], 32.0, Color::WHITE, &level);
    } else {
        p.glow(0.75, -CENTRE[slot], 32.0, &level);
    }
    p.text(0.6, -CENTRE[slot], 64.0, Color::WHITE, &s.rank);
    let lit = |start: Option<f32>| start.is_some_and(|a| a < t && t < a + LIGHT);
    let past = |start: Option<f32>| start.is_some_and(|a| a < t);
    for (k, (&word, &y)) in STAT_WORDS.iter().zip(&STAT_Y).enumerate() {
        let light = page.lights[k];
        if lit(light) {
            p.glow(STAT_SCALE, words_x, y, word);
        } else {
            p.text(STAT_SCALE, words_x, y, Color::WHITE, word);
        }
        if past(light) {
            p.glow(STAT_SCALE, values_x, y, &format!("{}", page.after[k] as i32));
        } else {
            p.text(STAT_SCALE, values_x, y, Color::WHITE, &format!("{}", page.before[k] as i32));
        }
    }
    for (word, y) in [("Max", 188.0), ("Health", 204.0)] {
        if lit(page.health_light) {
            p.glow(STAT_SCALE, words_x, y, word);
        } else {
            p.text(STAT_SCALE, words_x, y, Color::WHITE, word);
        }
    }
    if past(page.health_light) {
        p.glow(STAT_SCALE, values_x, 196.0, &format!("{}", page.health_after as i32));
    } else {
        let shown = if page.after_shop { page.health_after } else { page.health_before };
        p.text(STAT_SCALE, values_x, 196.0, Color::WHITE, &format!("{}", shown as i32));
    }
    if !page.after_shop
        && let Some(magic) = &s.magic
    {
        let [r, g, b] = PINK;
        p.lines(0.45, -CENTRE[slot], 224.0, false, Color::srgb_u8(r, g, b), magic);
    }
    if t >= page.end {
        p.prompt(slot, (16.0, 280.0), 16.0, (40.0, 280.0), "Continue");
    }
    p.title(slot, "Stats");
}

fn draw_final(p: &mut Paint, slot: usize, s: &Slot) {
    let (x, t, totals) = (-CENTRE[slot], s.t, &s.totals);
    p.glow(0.56, x, 32.0, "Final Stats");
    let fields = totals.seconds * 60.0;
    let days = (fields / 5_184_000.0).trunc();
    let rest = fields - days * 5_184_000.0;
    let hours = (rest / 216_000.0).trunc();
    let minutes = ((rest - hours * 216_000.0) / 3600.0).trunc();
    #[allow(clippy::type_complexity)]
    let rows: [(&[(&str, f32)], Vec<(String, f32)>); 4] = [
        (&[("Enemies Killed", 60.0)], vec![(totals.kills.to_string(), 78.0)]),
        (&[("Generators", 98.0), ("Destroyed", 116.0)], vec![(totals.generators.to_string(), 134.0)]),
        (&[("Gold Found", 154.0)], vec![(totals.gold.to_string(), 172.0)]),
        (
            &[("Total Playtime", 192.0)],
            vec![
                (format!("{} Days", days as u32), 210.0),
                (format!("{} Hours", hours as u32), 228.0),
                (format!("{} Minutes", minutes as u32), 246.0),
            ],
        ),
    ];
    for ((words, values), &start) in rows.iter().zip(&FINAL_LIGHTS) {
        for &(word, y) in words.iter() {
            if start < t && t < start + LIGHT {
                p.glow(STAT_SCALE, x, y, word);
            } else {
                p.text(STAT_SCALE, x, y, Color::WHITE, word);
            }
        }
        if t > start {
            for (value, y) in values {
                p.glow(STAT_SCALE, x, *y, value);
            }
        }
    }
    if t > FINAL_END - 1.0 {
        p.prompt(slot, (16.0, 280.0), 16.0, (40.0, 280.0), "Continue");
    }
    p.title(slot, "Stats");
}

fn draw_list(p: &mut Paint, screen: &ShopScreen, slot: usize, s: &Slot) {
    let (col, list) = (COLUMN[slot], &s.list);
    let (mut more_up, mut more_down) = (false, false);
    for (i, item) in screen.items.iter().enumerate() {
        let y = list.ys[i];
        let skip = list.skip[i];
        let mut fade = list_fade(y);
        if fade > 0.0 && !skip {
            if y < WINDOW_TOP {
                more_up = true;
            } else {
                more_down = true;
            }
        }
        if fade >= 256.0 {
            continue;
        }
        if fade == 0.0 && skip {
            fade = SKIPPED_FADE;
        }
        let a = alpha(fade);
        let cursor = i == list.cursor;
        let ink = if list.flash[i] > 0.0 { Color::srgb(1.0, 0.0, 0.0) } else { Color::BLACK };
        let icon = screen.icons[i];
        if icon {
            p.image(&item.icon, col + 20.0, y, Color::WHITE.with_alpha(a));
        }
        if item.price > 0 {
            let owned = list.flags[i] & OWNED != 0;
            let prices = [Some((format!("B:{}", item.price), if owned { y - 6.0 } else { y + 12.0 })), owned.then(|| (format!("S:{}", sale_price(item.price)), y + 12.0))];
            for (words, at) in prices.into_iter().flatten() {
                if cursor {
                    p.glow(LIST_SCALE, col + 58.0, at, &words);
                } else {
                    p.text(LIST_SCALE, col + 58.0, at, ink.with_alpha(a), &words);
                }
            }
        }
        if !item.text.is_empty() {
            let at = y + if icon { 32.0 } else { 12.0 };
            p.lines(LIST_SCALE * item.text_scale, -CENTRE[slot], at, cursor, ink.with_alpha(a), &item.text);
        }
    }
    if more_up {
        p.image("MORE_UP", CENTRE[slot] - 32.0, WINDOW_TOP - 40.0, Color::WHITE);
    }
    if more_down {
        p.image("MORE_DOWN", CENTRE[slot] - 32.0, WINDOW_BOTTOM + 56.0, Color::WHITE);
    }
    p.title(slot, "Shop");
}

fn draw_inventory(p: &mut Paint, slot: usize, s: &Slot, state: &PlayerState) {
    let (col, inv) = (COLUMN[slot], &s.inventory);
    let quest = &state.quest;
    let marks = crate::quest::boss_marks(state.realms_beaten);
    let picture = |p: &mut Paint, name: &str, x: f32, y: f32| {
        let Some(i) = p.texture(name) else { return };
        let (pos, size, fade) = inventory_look(x, y, i.size.x, i.size.y, col, inv);
        p.d.image(&i, pos.x, pos.y, size.x, size.y, Color::WHITE.with_alpha(alpha(fade)));
    };
    let (name, x, y) = WINDOW;
    picture(p, name, x, y);
    for &(name, x, y) in GARGOYLE_PIECES.iter().chain(&CRYSTALS) {
        picture(p, name, x, y);
    }
    for (bit, entry) in LEGENDARY.iter().enumerate() {
        let Some((name, x, y)) = *entry else { continue };
        let held = quest.legendary & (1 << bit) != 0;
        let name = if held { name.to_string() } else { format!("{name}_EMPTY") };
        picture(p, &name, x, y);
    }
    for (bit, entry) in PIECES.iter().enumerate() {
        if let Some((name, x, y)) = *entry
            && marks & (1 << bit) != 0
        {
            picture(p, name, x, y);
        }
    }
    let a = alpha(inventory_count_fade(inv));
    let need = |held: i16, need: i16| if held < 0 || held > need { need } else { held };
    let counts = GARGOYLE_PIECES
        .iter()
        .zip(quest.gargoyle.iter().zip(crate::quest::GARGOYLE_NEEDED))
        .map(|(&(_, x, y), (&held, needed))| (x, y, GARGOYLE_COUNT, need(held, needed), needed))
        .chain(
            CRYSTALS.iter().zip(quest.crystals[1..].iter().zip(&crate::quest::CRYSTALS_NEEDED[1..])).map(
                |(&(_, x, y), (&held, &needed))| (x, y, CRYSTAL_COUNT, need(held, needed), needed),
            ),
        );
    for (x, y, (dx, dy, scale), held, needed) in counts {
        let at = col + x + dx;
        let held = held.to_string();
        let w = p.fonts.width(FONT32, scale, &held);
        p.text(scale, at - w, y + dy, Color::WHITE.with_alpha(a), &held);
        p.text(scale, at, y + dy, Color::WHITE.with_alpha(a), &format!("/{needed}"));
    }
    if !inv.closing {
        p.prompt(slot, (16.0, 280.0), 16.0, (40.0, 280.0), "Continue");
    }
    p.title(slot, "Inv");
}
