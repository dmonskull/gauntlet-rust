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
use crate::level::LoadedGame;
use crate::message_box::MessageBox;

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
    /// 0x71 + n: legendary item n (1–11) picked up — its name, spoken.
    Legendary(u8),
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
            Self::Legendary(_) => unreachable!(),
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

fn show_hints(
    time: Res<Time>,
    mut hints: ResMut<Hints>,
    boxes: Res<MessageBox>,
    mut requests: MessageReader<ShowHint>,
    mut voice: MessageWriter<QueueVoice>,
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
        let Some(text) = hints.rom.as_ref().and_then(|r| r.group(&group)).map(|g| g.strings.join(" ")) else {
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
