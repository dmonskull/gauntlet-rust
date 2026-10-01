//! The game's on-screen hints ("USE KEY TO OPEN DOORS"): each is a text
//! group in `TEXT/ENGLISH.ROM` spoken by the announcer from the `VOICE1`
//! bank. Pickups, doors and transporters raise them by number through
//! [`ShowHint`]; `docs/items.md` has the table they come from. (The longer
//! messages — scrolls, the tower's notices — are the message box's,
//! `message_box.rs`.) The announcer's line waits in the announcer's voice
//! queue, dropped if more than half a second is queued ahead of it.
//!
//! The box (`docs/items.md`, "Hints"): the `Scroll_A` parchment at three
//! quarters, 64 wider than the text's widest line and 16 taller than its
//! lines × (font height + 8), centred over the player's panel (x 64 for
//! player 1, y 250) and kept on screen; the lines in the group's font and
//! scale, 2 apart, centred in it, in the player's dark ink (player 1's
//! `0x1F1F00`). It stays up for 60 fields a line and 30 more, waiting and
//! hidden during a camera cut. After each hint, the tutorial ones wait a
//! while before the next: 0, 2, 4, 7 seconds, then 10 each time.
//! Stand-in: hints shown once are remembered for the session, not in the
//! character's record.

use bevy::prelude::*;
use gdl_formats::text::TextRom;

use crate::audio::QueueVoice;
use crate::character;
use crate::font::{Draw2d, Flush2d, GameFonts, TextStyle, UiTextures};
use crate::frontend::Frontend;
use crate::level::LoadedGame;
use crate::message_box::{DrawBox, MessageBox};
use crate::play_camera::PlayCamera;
use crate::player::PlayerChoice;
use crate::party::{MAX_PLAYERS, Party};

/// A hint stays up this many fields a line, and this many more.
const FIELDS_A_LINE: f32 = 60.0;
const FIELDS_MORE: f32 = 30.0;
/// The waits after each hint before the next tutorial hint (fields); the
/// last repeats.
const COOLDOWNS: [f32; 5] = [0.0, 120.0, 240.0, 420.0, 600.0];
/// A hint's line is dropped when it would wait longer than this, seconds.
const VOICE_MOST_WAIT: f32 = 0.5;
/// The box: its panel, how see-through (the game's 0x40: alpha 0x60 of
/// 0x80), the margins its text gets across and down, the gap its height
/// is measured with and the gap its lines are drawn with.
const PANEL: &str = "Scroll_A";
const PANEL_ALPHA: f32 = 0.75;
const MARGIN_ACROSS: f32 = 64.0;
const MARGIN_DOWN: f32 = 16.0;
const MEASURE_GAP: f32 = 8.0;
const LINE_GAP: f32 = 2.0;
/// Player 1's panel: the box's centre, and its ink.
const CENTRE: (f32, f32) = (64.0, 250.0);
/// Each player's panel follows the first's this far apart; a hint for
/// every player is centred here.
const PANEL_WIDTH: f32 = 128.0;
const CENTRE_ALL: (f32, f32) = (256.0, 192.0);
/// The players' inks (dark yellow, blue, red, green) and everyone's.
const INKS: [[u8; 3]; MAX_PLAYERS] = [[0x1F, 0x1F, 0x00], [0x00, 0x00, 0x1F], [0x1F, 0x00, 0x00], [0x00, 0x1F, 0x00]];
const INK_ALL: [u8; 3] = [0x16, 0x0C, 0x03];
/// The box stays within x 0–511 and y 2–304 (the panels' top).
const SCREEN_RIGHT: f32 = 511.0;
const SCREEN_TOP: f32 = 2.0;
const PANELS_TOP: f32 = 304.0;

/// The hints items raise, by the game's hint number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hint {
    /// 1: touching a door without a key.
    UseKeyOnDoor,
    /// 2: a key picked up in a level without doors; also a locked chest
    /// without a key.
    UseKeyOnChest,
    /// 3: potion slots full.
    MagicFull,
    /// 4: key ring full.
    KeysFull,
    /// 6: magic pressed without a potion.
    CollectMagicFirst,
    /// 7, then 0x5E, then 0x5F: picking up potions.
    UseMagic,
    ThrowMagic,
    MagicShield,
    /// 8: a key picked up where there are doors.
    SaveKeys,
    /// 9: after a transporter.
    Transporter,
    /// 0xB: a hero has stood in an exit for 6 fields while the others
    /// haven't come (co-op).
    WaitForOthers,
    /// 0xF: food worth 100 or more.
    EatMeat,
    /// 0x10: food worth 50–99.
    EatFruit,
    /// 0xE: a hero's blow sets off a potion lying on the floor.
    ShootPotion,
    /// 0x11: more than 24 gold at once.
    CollectGold,
    /// 0x14: a hero's blow damages a secret wall (a nameless obstacle).
    SecretWalls,
    /// 0x15: a damage tile hurts the hero.
    AvoidObjects,
    /// 0x1B: a wooden barrel breaks open.
    SomeBarrels,
    /// 0x1C: poisoned food.
    PoisonedFood,
    /// 0x85: food at full health.
    HealthFull,
    /// 0x86: a dead general drops what it held.
    GeneralsCarryItems,
    /// 0x87: an explosion blows a powerup to pieces.
    ExplosionsDestroyItems,
    /// 0x88: poison gas spoils food.
    GasSpoilsFood,
    /// 0x89: a CHESTEXP explodes.
    ChestsExplode,
    /// 0x8A: a dead gargoyle drops its golden piece (or what it held).
    DefeatGargoyles,
    /// 0: a blow that isn't magic lands on Death.
    KillDeathWithMagic,
    /// 0x80, 0x82: a Death drains experience, health.
    DeathDrainsExperience,
    DeathDrainsHealth,
    /// 0x81, 0x83: a Death leaves after draining its fill (the game's
    /// table pairs these texts the other way round).
    DeathLeftAfterExperience,
    DeathLeftAfterHealth,
    /// 0x71 + n: legendary item n (1–11) picked up — its name, spoken.
    Legendary(u8),
    /// A timed power-up picked up: the game's hint number
    /// ([`POWER_HINTS`], [`Hint::for_power`]).
    Power(u8),
    /// 0x22: experience raised the hero's level — "LEVEL %d EXPERIENCE"
    /// with the level; shown every time.
    LevelUp,
}

/// The power-up pickups' hints (`docs/powers.md`, "Pickup hints"; the
/// game's hint table): hint number, text group, announcer
/// line, and whether it's shown only once (every one but the Pojo's,
/// mode 0).
const POWER_HINTS: [(u8, &str, &str, bool); 37] = [
    (0x20, "SPEEDUP", "S_XSPEED", true),
    (0x21, "MAGICUP", "S_XMAGIC", true),
    (0x23, "INVULNERABILITY", "S_INVULVOX", true),
    (0x24, "INVISIBILITY", "S_INVISVOX", true),
    (0x25, "THREEWAYS", "S_3WAYSHOTVOX", true),
    (0x26, "REFLECTSHOT", "S_REFLECTVOX", true),
    (0x27, "SEETHRU", "S_XRAYVOX", true),
    (0x28, "REDAMULET", "S_FIREAMVOX", true),
    (0x29, "BLUEAMULET", "S_LGHTNGAMVOX", true),
    (0x2A, "YELAMULET", "S_LIGHTAMVOX", true),
    (0x2B, "GREENAMULET", "S_ACIDAMVOX", true),
    (0x2F, "FIVEWAYS", "S_5WAYSHOTVOX", true),
    (0x30, "SUPERSHOT", "S_SUPERVOX", true),
    (0x31, "HALO", "S_ANTIDEATHVOX", true),
    (0x33, "TIMESTOP", "S_STOPPEDVOX", true),
    (0x34, "REFLECTARMOR", "S_REFLECTSHVOX", true),
    (0x35, "LEVITATION", "S_LEVVOX", true),
    (0x36, "INVULNERABILITY", "S_INVULVOX", true),
    (0x51, "FIREBREATHE", "S_FIREBRVOX", true),
    (0x52, "ACIDBREATHE", "S_ACIDBRVOX", true),
    (0x53, "ELECBREATHE", "S_LGHTNGBRVOX", true),
    (0x54, "PHOENIX", "S_PHOENIXVOX", true),
    (0x56, "HAMMERMSG", "S_HAMMERVOX", true),
    (0x57, "RAPIDFIREMSG", "S_RAPIDFIREVOX", true),
    (0x58, "GROWTHMSG", "S_GROWTHVOX", true),
    (0x59, "SHRINKMSG", "S_SHRINKVOX", true),
    (0x5B, "FIRESHIELDMSG", "S_FIREWALLSHVOX", true),
    (0x5C, "ELECSHIELDMSG", "S_LGHTNGSHVOX", true),
    (POJO_HINT, "POJOMSG", "S_POJOVOX", false),
    (0x62, "MASKMSG", "S_MASKVOX", true),
    (0x63, "HORNSMSG", "S_HORNSVOX", true),
    (0x64, "GAUNTLETMSG", "S_GAUNTLETVOX", true),
    (0x71, "TURBOBOOST", "S_TURBOBOOST", true),
    (0x84, "GASMASK", "S_GASMASK", true),
    (0x94, "MIKEY", "S_PICKUPCRYST", true),
    (0x95, "HANDOFDEATH", "S_PICKUPCRYST", true),
    (0x96, "HEALTHVAMP", "S_PICKUPCRYST", true),
];

/// The Pojo's hint ("%s %s IS NOW POJO"), the one power hint that shows
/// every time.
const POJO_HINT: u8 = 0x5D;

/// Weapon, armour and special bits and their hints, in the order the
/// pickup routine tests them: the first bit held wins.
const WEAPON_HINTS: [(u32, u8); 6] =
    [(0x8_0000, 0x25), (0x40_0000, 0x2F), (0x20_0000, 0x26), (0x10_0000, 0x30), (0x1000_0000, 0x56), (0x2000_0000, 0x57)];
/// A weapon with none of those: its element (1–4) picks an amulet.
const AMULET_HINTS: [u8; 4] = [0x28, 0x29, 0x2A, 0x2B];
const ARMOUR_HINTS: [(u32, u8); 7] = [
    (0x10_0000, 0x36),
    (0x1_0000, 0x23),
    (0x8_0000, 0x31),
    (0x2_0000, 0x34),
    (0x20_0000, 0x5B),
    (0x40_0000, 0x5C),
    (0x2000, 0x84),
];
const SPECIAL_HINTS: [(u32, u8); 19] = [
    (0x4, 0x24),
    (0x2, 0x27),
    (0x8, 0x33),
    (0x1, 0x35),
    (0x10, 0x51),
    (0x20, 0x52),
    (0x40, 0x53),
    (0x80, 0x54),
    (0x100, 0x58),
    (0x200, 0x59),
    (0x400, POJO_HINT),
    (0x2000, 0x62),
    (0x1000, 0x63),
    (0x8000, 0x64),
    (0x4000, 0x64),
    (0x8_0000, 0x71),
    (0x10_0000, 0x94),
    (0x20_0000, 0x95),
    (0x40_0000, 0x96),
];

impl Hint {
    /// The hint picking up a power-up of `subtype` (5 weapon, 6 armour, 7
    /// speed, 8 magic, 9 special) with `value` bits raises, if any.
    pub fn for_power(subtype: i32, value: u32) -> Option<Self> {
        let first = |table: &[(u32, u8)]| table.iter().find(|(bit, _)| value & bit != 0).map(|&(_, h)| h);
        let n = match subtype {
            5 => first(&WEAPON_HINTS).or_else(|| AMULET_HINTS.get((value & 0xF).checked_sub(1)? as usize).copied()),
            6 => first(&ARMOUR_HINTS),
            7 => Some(0x20),
            8 => Some(0x21),
            9 => first(&SPECIAL_HINTS),
            _ => None,
        }?;
        Some(Self::Power(n))
    }
}

/// The legendary items' announcer lines, items 1–11 (hints 0x72–0x7C).
const LEGENDARY_VOICES: [&str; 11] = [
    "S_SCIMITARVOX",
    "S_ICEAXEVOX",
    "S_LAMPVOX",
    "S_BELLOWSVOX",
    "S_SAVIORVOX",
    "S_SAVIORVOX",
    "S_BOOKVOX",
    "S_SAVIORVOX",
    "S_PARCHVOX",
    "S_LANTERNVOX",
    "S_JAVELINVOX",
];

impl Hint {
    /// The game's hint number.
    fn number(self) -> u8 {
        match self {
            Self::KillDeathWithMagic => 0,
            Self::UseKeyOnDoor => 1,
            Self::UseKeyOnChest => 2,
            Self::MagicFull => 3,
            Self::KeysFull => 4,
            Self::CollectMagicFirst => 6,
            Self::UseMagic => 7,
            Self::SaveKeys => 8,
            Self::Transporter => 9,
            Self::WaitForOthers => 0xB,
            Self::ShootPotion => 0xE,
            Self::EatMeat => 0xF,
            Self::EatFruit => 0x10,
            Self::CollectGold => 0x11,
            Self::SecretWalls => 0x14,
            Self::AvoidObjects => 0x15,
            Self::SomeBarrels => 0x1B,
            Self::PoisonedFood => 0x1C,
            Self::LevelUp => 0x22,
            Self::ThrowMagic => 0x5E,
            Self::MagicShield => 0x5F,
            Self::Legendary(n) => 0x71 + n.clamp(1, 11),
            Self::DeathDrainsExperience => 0x80,
            Self::DeathLeftAfterExperience => 0x81,
            Self::DeathDrainsHealth => 0x82,
            Self::DeathLeftAfterHealth => 0x83,
            Self::HealthFull => 0x85,
            Self::GeneralsCarryItems => 0x86,
            Self::ExplosionsDestroyItems => 0x87,
            Self::GasSpoilsFood => 0x88,
            Self::ChestsExplode => 0x89,
            Self::DefeatGargoyles => 0x8A,
            Self::Power(n) => n,
        }
    }

    /// The tutorial hints, which wait out the cool-down after a hint.
    fn waits(self) -> bool {
        matches!(self.number(), 0..0x1D | 0x2C | 0x2D | 0x37 | 0x50)
    }

    /// Text group, announcer line, and whether it's shown only once (the
    /// game's mode 0 hints repeat; the rest are remembered once shown).
    fn entry(self) -> (std::borrow::Cow<'static, str>, &'static str, bool) {
        if let Self::Legendary(n) = self {
            let i = usize::from(n.clamp(1, 11) - 1);
            return (format!("LEGEND_ITEMS{i:03}").into(), LEGENDARY_VOICES[i], true);
        }
        if let Self::Power(n) = self {
            let (_, group, line, once) = POWER_HINTS.iter().find(|h| h.0 == n).copied().unwrap_or((n, "", "", true));
            return (group.into(), line, once);
        }
        let (group, line, once) = match self {
            Self::UseKeyOnDoor => ("USEKEYOPENDOOR", "S_USEKEY", true),
            Self::UseKeyOnChest => ("USEKEYOPENCHEST", "S_USEKEY2", true),
            Self::MagicFull => ("FULLOFBOMBS", "S_MAGICFULL", true),
            Self::KeysFull => ("FULLOFKEYS", "S_KEYFULL", true),
            Self::CollectMagicFirst => ("COLLECTMAGICFIRST", "S_COLLECTPOT", true),
            Self::UseMagic => ("USEMAGIC2", "S_USEMAGIC2", true),
            Self::ThrowMagic => ("THROWMAGIC", "S_THROWMAGIC", true),
            Self::MagicShield => ("MAGICSHIELD", "S_SHIELDMAGIC", true),
            Self::SaveKeys => ("SAVEKEYS", "S_SAVEKEYS", true),
            Self::Transporter => ("TRANSPORTERSMOVEYOU", "S_TRANSPORTER", true),
            Self::WaitForOthers => ("HOWTOEXIT", "S_SAMEEXIT", true),
            Self::EatMeat => ("EATMEAT", "S_MEATGIVES", true),
            Self::EatFruit => ("EATFRUIT", "S_FRUITGIVES", true),
            Self::ShootPotion => ("SHOOTPOTIONLESSER", "S_SHOOTINGMAGIC", true),
            Self::CollectGold => ("COLLECTGOLD", "S_COLLECTGOLD", true),
            Self::SecretWalls => ("FOUNDSECRETWALLS", "S_MULTIPLEHITS", true),
            Self::AvoidObjects => ("AVOIDOBJECTS", "S_AVOID", true),
            Self::SomeBarrels => ("WOODBARREL", "S_SOMEBARRELS", true),
            Self::PoisonedFood => ("POISONEDFOOD", "S_POISONEDFOOD", true),
            Self::HealthFull => ("HEALTHFULL", "S_HEALTHFULL", true),
            Self::GeneralsCarryItems => ("GENSCARRY", "S_GENSCARRY", true),
            Self::ExplosionsDestroyItems => ("EXPDESTROY", "S_EXPDSTITMS", true),
            Self::GasSpoilsFood => ("GASPOISON", "S_GASFOODBAD", true),
            Self::ChestsExplode => ("CHESTSEXPL", "S_CHESTSEXPL", true),
            Self::DefeatGargoyles => ("DEFEATGAR", "S_DEFGRG4GLD", true),
            Self::LevelUp => ("LEVELUP", "S_GAINEDLEVEL", false),
            Self::KillDeathWithMagic => ("USEMAGIC", "S_USEMAGIC", true),
            Self::DeathDrainsExperience => ("DEATHDRAINEXP", "S_DEATHDRAINXP", true),
            Self::DeathDrainsHealth => ("DEATHDRAINHEALTH", "S_DEATHDRAINS", true),
            Self::DeathLeftAfterExperience => ("DEATHDIEEXP", "S_DIESAFTERXP", true),
            Self::DeathLeftAfterHealth => ("DEATHDIEHEALTH", "S_DIESAFTER", true),
            Self::Legendary(_) | Self::Power(_) => unreachable!(),
        };
        (group.into(), line, once)
    }
}

/// Asks for a hint for a player (by slot; past the last slot, for every
/// one, as the game's player 4); dropped if one is already up or it was
/// shown before.
#[derive(Message, Clone, Copy, Debug)]
pub struct ShowHint {
    pub hint: Hint,
    pub player: usize,
}

impl ShowHint {
    pub fn to(player: usize, hint: Hint) -> Self {
        Self { hint, player }
    }

    /// For every player (the game's player −1, shown as its player 4: the
    /// shared ink, in the middle of the screen).
    pub fn all(hint: Hint) -> Self {
        Self { hint, player: MAX_PLAYERS }
    }
}

/// The hint on screen: whose it is, its lines, font slot and scale, and
/// fields left.
struct Up {
    player: usize,
    lines: Vec<String>,
    slot: usize,
    scale: f32,
    fields: f32,
}

/// The hint on screen, those shown, and the tutorial hints' cool-down.
#[derive(Resource, Default)]
pub struct Hints {
    up: Option<Up>,
    /// By slot: the hints each player has seen (the game keeps them in the
    /// player's record).
    shown: [Vec<Hint>; MAX_PLAYERS],
    rom: Option<TextRom>,
    /// Fields before the next tutorial hint may show, and how many hints
    /// have set it this level.
    cooldown: f32,
    count: usize,
}

impl Hints {
    /// Whether a player (any, for [`ShowHint::ALL`]) has seen `hint` (so
    /// the next in a chain is used).
    pub fn seen(&self, player: usize, hint: Hint) -> bool {
        match self.shown.get(player) {
            Some(seen) => seen.contains(&hint),
            None => self.shown.iter().any(|s| s.contains(&hint)),
        }
    }

    /// The hint on screen, as one line (the debug overlay).
    pub fn text(&self) -> Option<String> {
        self.up.as_ref().map(|u| u.lines.join(" "))
    }
}

/// The wait the `count`th hint of a level sets before the next tutorial
/// hint (fields).
fn cooldown(count: usize) -> f32 {
    COOLDOWNS[count.min(COOLDOWNS.len() - 1)]
}

/// The box round `lines` (width of the widest, height as measured): its
/// left and top, size, and the centre its text is drawn about — the
/// panel's centre, moved to keep the box on screen.
fn place_box(player: usize, widest: f32, measured: f32) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let (w, h) = (widest + MARGIN_ACROSS, measured + MARGIN_DOWN);
    let (mut cx, mut cy) = if player < MAX_PLAYERS { (CENTRE.0 + PANEL_WIDTH * player as f32, CENTRE.1) } else { CENTRE_ALL };
    let mut x = cx - (w / 2.0).trunc();
    if x < 0.0 {
        cx -= x;
        x = 0.0;
    } else if x + w > SCREEN_RIGHT {
        cx -= x + w - SCREEN_RIGHT;
        x = SCREEN_RIGHT - w;
    }
    let mut y = cy - (h / 2.0).trunc();
    if y < SCREEN_TOP {
        cy += SCREEN_TOP - y;
        y = SCREEN_TOP;
    } else if y + h > PANELS_TOP {
        cy += PANELS_TOP - (y + h);
        y = PANELS_TOP - h;
    }
    ((x, y), (w, h), (cx, cy))
}

pub struct HintsPlugin;

impl Plugin for HintsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ShowHint>()
            .init_resource::<Hints>()
            .add_systems(Startup, load_text)
            .add_systems(Update, show_hints)
            .add_systems(PostUpdate, draw_hint.before(DrawBox).before(Flush2d));
    }
}

fn load_text(mut hints: ResMut<Hints>, mut game: ResMut<LoadedGame>) {
    let read = game.install.read("TEXT/ENGLISH.ROM").map_err(|e| e.to_string());
    match read.and_then(|b| TextRom::parse(&b).map_err(|e| e.to_string())) {
        Ok(rom) => hints.rom = Some(rom),
        Err(e) => warn!("no game text, hints disabled: {e}"),
    }
}

/// The colour names' order in `PLAYER_COLOR` (the player's `+0x04`).
const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
/// The Pojo's special bit: its name stands in for the hero's.
const POJO: u32 = 0x400;

/// A hint line's `%s %s` filled as the game fills it for the "it", shrink
/// and Pojo hints: the hero's colour and class names (`PLAYER_COLOR`,
/// `PLAYER_CLASS`) — or, while the hero is the Pojo, the whole line is the
/// Pojo's name (`POJO`), except in the Pojo's own hint.
fn fill_hero(line: &str, rom: &TextRom, hint: Hint, choice: Option<&PlayerChoice>, pojo: bool) -> String {
    if !line.contains("%s") {
        return line.to_string();
    }
    let name = |group: &str, i: Option<usize>| rom.group(group).and_then(|g| g.strings.get(i?)).cloned();
    if pojo && hint != Hint::Power(POJO_HINT) {
        return name("POJO", Some(0)).unwrap_or_else(|| "POJO".into());
    }
    let colour = choice.and_then(|c| COLOURS.iter().position(|p| c.variant.to_ascii_uppercase().starts_with(p)));
    let class = choice.and_then(|c| character::class_index(&c.class));
    let colour = name("PLAYER_COLOR", colour).unwrap_or_default();
    let class = name("PLAYER_CLASS", class).or_else(|| choice.map(|c| c.class.clone())).unwrap_or_default();
    line.replacen("%s", &colour, 1).replacen("%s", &class, 1).trim().to_string()
}

#[allow(clippy::too_many_arguments)]
fn show_hints(
    time: Res<Time>,
    mut hints: ResMut<Hints>,
    boxes: Res<MessageBox>,
    camera: Option<Res<PlayCamera>>,
    population: Option<Res<crate::population::LevelPopulation>>,
    mut requests: MessageReader<ShowHint>,
    mut voice: MessageWriter<QueueVoice>,
    party: Res<Party>,
) {
    // A level's load clears the box and the cool-down.
    if population.is_some_and(|p| p.is_changed()) {
        hints.up = None;
        hints.cooldown = 0.0;
        hints.count = 0;
    }
    // Play is frozen under the message box.
    if boxes.is_open() {
        requests.clear();
        return;
    }
    let fields = time.delta_secs() * 60.0;
    hints.cooldown = (hints.cooldown - fields).max(0.0);
    // The box's time runs out of camera cuts only.
    if !camera.is_some_and(|c| c.in_cut())
        && let Some(up) = hints.up.as_mut()
    {
        up.fields -= fields;
        if up.fields < 1.0 {
            hints.up = None;
        }
    }
    for &ShowHint { hint, player } in requests.read() {
        let (group, line, once) = hint.entry();
        // All of these share one priority, so one up blocks the next; the
        // tutorial ones wait out the cool-down.
        if hints.up.is_some() || (once && hints.seen(player, hint)) || (hint.waits() && hints.cooldown > 0.0) {
            continue;
        }
        // The hero it's about (the first player's for a hint for all).
        let (choice, state) = if player < MAX_PLAYERS { (party.choice(player), party.state(player)) } else { (party.choice(0), party.state(0)) };
        let pojo = state.is_some_and(|s| s.bits.special & POJO != 0);
        // The game fills a hint's `%d` with the hero's level.
        let level = state.map_or(1, |s| s.level).to_string();
        let Some(up) = hints.rom.as_ref().and_then(|rom| {
            let g = rom.group(&group)?;
            let lines = g
                .strings
                .iter()
                .map(|l| fill_hero(l, rom, hint, choice, pojo).replace("%d", &level))
                .collect::<Vec<_>>();
            let font = rom.fonts.get(g.font as usize).map_or(0, |f| gdl_formats::font::slot_for_rom_font(f));
            let fields = lines.len() as f32 * FIELDS_A_LINE + FIELDS_MORE;
            Some(Up { player, lines, slot: font, scale: g.scale[0], fields })
        }) else {
            continue;
        };
        info!("hint: {}", up.lines.join(" / "));
        hints.up = Some(up);
        match hints.shown.get_mut(player) {
            Some(seen) => seen.push(hint),
            None => hints.shown.iter_mut().for_each(|s| s.push(hint)),
        }
        hints.cooldown = cooldown(hints.count);
        hints.count += 1;
        voice.write(QueueVoice::announcer(line, VOICE_MOST_WAIT).gated());
    }
}

/// Draws the hint's box and lines over play (not on the front end's
/// screens or under a menu, and hidden during a camera cut).
#[allow(clippy::too_many_arguments)]
fn draw_hint(
    hints: Res<Hints>,
    frontend: Option<Res<Frontend>>,
    boxes: Res<MessageBox>,
    camera: Option<Res<PlayCamera>>,
    fonts: Option<Res<GameFonts>>,
    mut tex: Option<ResMut<UiTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut draw: ResMut<Draw2d>,
) {
    let (Some(up), Some(fonts), Some(tex)) = (hints.up.as_ref(), fonts, tex.as_deref_mut()) else { return };
    if frontend.is_some_and(|f| !f.playing() || f.menu_open()) || boxes.is_open() || camera.is_some_and(|c| c.in_cut()) {
        return;
    }
    let widest = up.lines.iter().map(|l| fonts.width(up.slot, up.scale, l)).fold(0.0, f32::max);
    let height = fonts.line_height(up.slot, up.scale).trunc();
    let ((x, y), (w, h), (cx, cy)) = place_box(up.player, widest, up.lines.len() as f32 * (height + MEASURE_GAP));
    if let Some(panel) = tex.get(PANEL, &mut images) {
        draw.image(&panel, x, y, w, h, Color::srgba(1.0, 1.0, 1.0, PANEL_ALPHA));
    }
    let [r, g, b] = INKS.get(up.player).copied().unwrap_or(INK_ALL);
    let ink = TextStyle::new(up.slot, up.scale, Color::srgb_u8(r, g, b));
    let step = height + LINE_GAP;
    let block = height + step * (up.lines.len() as f32 - 1.0);
    let mut line_y = (cy - (block / 2.0).trunc()).trunc();
    for l in &up.lines {
        draw.text(&fonts, &ink, -cx, line_y, l);
        line_y += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_hints_follow_the_pickup_routine() {
        // Gold armour (0x110000) is the invulnerability's hint 0x36.
        assert_eq!(Hint::for_power(6, 0x11_0000), Some(Hint::Power(0x36)));
        assert_eq!(Hint::for_power(6, 0x1_0000), Some(Hint::Power(0x23)));
        assert_eq!(Hint::for_power(6, 0x2008), Some(Hint::Power(0x84)));
        assert_eq!(Hint::for_power(9, 0x400), Some(Hint::Power(POJO_HINT)));
        assert_eq!(Hint::for_power(9, 0x4000), Some(Hint::Power(0x64)));
        assert_eq!(Hint::for_power(5, 0x8_0002), Some(Hint::Power(0x25)));
        assert_eq!(Hint::for_power(5, 0x3), Some(Hint::Power(0x2A)));
        assert_eq!(Hint::for_power(5, 0), None);
        assert_eq!(Hint::for_power(7, 0), Some(Hint::Power(0x20)));
        // Every hint the routine raises has its entry.
        for n in WEAPON_HINTS.iter().chain(&ARMOUR_HINTS).chain(&SPECIAL_HINTS).map(|h| h.1).chain(AMULET_HINTS).chain([0x20, 0x21]) {
            assert!(POWER_HINTS.iter().any(|h| h.0 == n), "hint {n:#x}");
        }
        let (_, _, once) = Hint::Power(POJO_HINT).entry();
        assert!(!once);
    }

    #[test]
    fn the_box_keeps_over_the_panel_and_on_screen() {
        // "COLLECT GOLD / TO BUY POWERUPS" in the 8 × 8 font: 120 wide,
        // two lines measured 16 each — a 184 × 48 box whose left would be
        // at 64 − 92, so it's moved to the screen's edge and its text
        // with it.
        let ((x, y), (w, h), (cx, cy)) = place_box(0, 120.0, 32.0);
        assert_eq!((x, y, w, h), (0.0, 226.0, 184.0, 48.0));
        assert_eq!((cx, cy), (92.0, 250.0));
        // A narrow one sits centred on the panel.
        let ((x, _), (w, _), (cx, _)) = place_box(0, 40.0, 16.0);
        assert_eq!((x, w, cx), (12.0, 104.0, 64.0));
    }

    #[test]
    fn tutorial_hints_wait_longer_each_time() {
        assert_eq!((0..7).map(cooldown).collect::<Vec<_>>(), [0.0, 120.0, 240.0, 420.0, 600.0, 600.0, 600.0]);
        assert!(Hint::CollectGold.waits() && Hint::UseKeyOnDoor.waits());
        assert!(!Hint::LevelUp.waits() && !Hint::HealthFull.waits() && !Hint::Power(0x23).waits());
    }

    #[test]
    fn hero_names_fill_the_line() {
        let group = |name: &str, strings: &[&str]| gdl_formats::text::TextGroup {
            name: name.into(),
            strings: strings.iter().map(|s| s.to_string()).collect(),
            font: 0,
            scale: [1.0; 2],
        };
        let rom = TextRom {
            groups: vec![
                group("PLAYER_COLOR", &["YELLOW", "BLUE", "RED", "GREEN"]),
                group("PLAYER_CLASS", &["WARRIOR", "VALKYRIE"]),
                group("POJO", &["POJO"]),
            ],
            fonts: Vec::new(),
            lists: Vec::new(),
        };
        let blue_valkyrie = PlayerChoice { class: "VAL".into(), variant: "BLU".into() };
        let pojo = Hint::Power(POJO_HINT);
        let shrink = Hint::Power(0x59);
        assert_eq!(fill_hero("%s %s", &rom, pojo, Some(&blue_valkyrie), false), "BLUE VALKYRIE");
        // As the Pojo: its own hint still names the hero, others say POJO.
        assert_eq!(fill_hero("%s %s", &rom, pojo, Some(&blue_valkyrie), true), "BLUE VALKYRIE");
        assert_eq!(fill_hero("%s %s", &rom, shrink, Some(&blue_valkyrie), true), "POJO");
        assert_eq!(fill_hero("IS NOW POJO", &rom, pojo, Some(&blue_valkyrie), true), "IS NOW POJO");
    }
}
