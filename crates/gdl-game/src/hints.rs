//! The game's on-screen hints ("USE KEY TO OPEN DOORS"): each is a text
//! group in `TEXT/ENGLISH.ROM` spoken by the announcer from the `VOICE1`
//! bank. Pickups, doors and transporters raise them by number through
//! [`ShowHint`]; `docs/items.md` has the table they come from. (The longer
//! messages — scrolls, the tower's notices — are the message box's,
//! `message_box.rs`.) The announcer's line waits in the announcer's voice
//! queue, dropped if more than half a second is queued ahead of it.
//! Stand-in: hints are drawn as plain centred text for a fixed time.

use bevy::prelude::*;
use gdl_formats::text::TextRom;

use crate::audio::QueueVoice;
use crate::character;
use crate::level::LoadedGame;
use crate::message_box::MessageBox;
use crate::player::PlayerChoice;
use crate::player_state::PlayerState;

/// How long a hint stays up. Stand-in: the game's hint timing isn't traced.
const HINT_SECONDS: f32 = 3.0;
/// A hint's line is dropped when it would wait longer than this, seconds.
const VOICE_MOST_WAIT: f32 = 0.5;

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
    /// 0x87: an explosion blows a powerup to pieces.
    ExplosionsDestroyItems,
    /// 0x88: poison gas spoils food.
    GasSpoilsFood,
    /// 0x89: a CHESTEXP explodes.
    ChestsExplode,
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
            Self::EatMeat => ("EATMEAT", "S_MEATGIVES", true),
            Self::EatFruit => ("EATFRUIT", "S_FRUITGIVES", true),
            Self::ShootPotion => ("SHOOTPOTIONLESSER", "S_SHOOTINGMAGIC", true),
            Self::CollectGold => ("COLLECTGOLD", "S_COLLECTGOLD", true),
            Self::SecretWalls => ("FOUNDSECRETWALLS", "S_MULTIPLEHITS", true),
            Self::AvoidObjects => ("AVOIDOBJECTS", "S_AVOID", true),
            Self::SomeBarrels => ("WOODBARREL", "S_SOMEBARRELS", true),
            Self::PoisonedFood => ("POISONEDFOOD", "S_POISONEDFOOD", true),
            Self::HealthFull => ("HEALTHFULL", "S_HEALTHFULL", true),
            Self::ExplosionsDestroyItems => ("EXPDESTROY", "S_EXPDSTITMS", true),
            Self::GasSpoilsFood => ("GASPOISON", "S_GASFOODBAD", true),
            Self::ChestsExplode => ("CHESTSEXPL", "S_CHESTSEXPL", true),
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

/// Asks for a hint; dropped if one is already up or it was shown before.
#[derive(Message, Clone, Copy, Debug)]
pub struct ShowHint(pub Hint);

/// The hint on screen, for the status overlay.
#[derive(Resource, Default)]
pub struct Hints {
    pub text: Option<String>,
    left: f32,
    shown: Vec<Hint>,
    rom: Option<TextRom>,
}

impl Hints {
    /// Whether `hint` has been shown (so the next in a chain is used).
    pub fn seen(&self, hint: Hint) -> bool {
        self.shown.contains(&hint)
    }
}

pub struct HintsPlugin;

impl Plugin for HintsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ShowHint>()
            .init_resource::<Hints>()
            .add_systems(Startup, load_text)
            .add_systems(Update, show_hints);
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
    mut requests: MessageReader<ShowHint>,
    mut voice: MessageWriter<QueueVoice>,
    choice: Option<Res<PlayerChoice>>,
    state: Option<Res<PlayerState>>,
) {
    // Play is frozen under the message box.
    if boxes.is_open() {
        requests.clear();
        return;
    }
    if hints.text.is_some() {
        hints.left -= time.delta_secs();
        if hints.left <= 0.0 {
            hints.text = None;
        }
    }
    for &ShowHint(hint) in requests.read() {
        let (group, line, once) = hint.entry();
        // All of these share one priority, so one up blocks the next.
        if hints.text.is_some() || (once && hints.seen(hint)) {
            continue;
        }
        let pojo = state.as_ref().is_some_and(|s| s.bits.special & POJO != 0);
        // The game fills a hint's `%d` with the hero's level.
        let level = state.as_ref().map_or(1, |s| s.level).to_string();
        let Some(text) = hints.rom.as_ref().and_then(|rom| {
            let lines = &rom.group(&group)?.strings;
            let lines: Vec<String> =
                lines.iter().map(|l| fill_hero(l, rom, hint, choice.as_deref(), pojo).replace("%d", &level)).collect();
            Some(lines.join(" "))
        }) else {
            continue;
        };
        // The font is ASCII-only.
        hints.text = Some(text.chars().filter(char::is_ascii).collect());
        info!("hint: {}", hints.text.as_deref().unwrap_or_default());
        hints.left = HINT_SECONDS;
        hints.shown.push(hint);
        voice.write(QueueVoice::announcer(line, VOICE_MOST_WAIT).gated());
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
