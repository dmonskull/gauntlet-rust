//! The game's on-screen hints ("USE KEY TO OPEN DOORS"): each is a text
//! group in `TEXT/ENGLISH.ROM` spoken by the announcer from the `VOICE1`
//! bank. Pickups, doors and transporters raise them by number through
//! [`ShowHint`]; `docs/items.md` has the table they come from.
//!
//! Messages ([`ShowMessage`]) are the longer texts in `TEXT/SCROLL_E.ROM`:
//! scrolls, the tower's "you need 15 Orange Crystals…" and unlock notices.
//! They queue, and show before any hint. Stand-in: both are drawn as plain
//! centred text for a fixed time; the game's message box isn't built.

use bevy::prelude::*;
use gdl_formats::text::TextRom;

use crate::audio::PlaySound;
use crate::level::LoadedGame;

/// How long a hint stays up. Stand-in: the game's hint timing isn't traced.
const HINT_SECONDS: f32 = 3.0;
/// How long a message stays up (stand-in, as for hints).
const MESSAGE_SECONDS: f32 = 5.0;

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
    /// 0x11: more than 24 gold at once.
    CollectGold,
    /// 0x1C: poisoned food.
    PoisonedFood,
    /// 0x85: food at full health.
    HealthFull,
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
            Self::CollectGold => ("COLLECTGOLD", "S_COLLECTGOLD", true),
            Self::PoisonedFood => ("POISONEDFOOD", "S_POISONEDFOOD", true),
            Self::HealthFull => ("HEALTHFULL", "S_HEALTHFULL", true),
            Self::Legendary(_) => unreachable!(),
        };
        (group.into(), line, once)
    }
}

/// Asks for a hint; dropped if one is already up or it was shown before.
#[derive(Message, Clone, Copy, Debug)]
pub struct ShowHint(pub Hint);

/// Asks for a message: string `index` of a text group — `TEXT/SCROLL_E.ROM`
/// (`NEEDCRYSTALS`, `SCROLLSA1`…), else `TEXT/ENGLISH.ROM` (the wizard's
/// `DRAGON_SPEECH`…) — with an announcer line, shown for `seconds` (the
/// default 5 when none).
#[derive(Message, Clone, Debug)]
pub struct ShowMessage {
    pub group: String,
    pub index: usize,
    pub voice: Option<&'static str>,
    pub seconds: Option<f32>,
}

impl ShowMessage {
    pub fn new(group: impl Into<String>, index: usize) -> Self {
        Self { group: group.into(), index, voice: None, seconds: None }
    }

    pub fn voice(mut self, line: &'static str) -> Self {
        self.voice = Some(line);
        self
    }

    pub fn seconds(mut self, seconds: f32) -> Self {
        self.seconds = Some(seconds);
        self
    }
}

/// The hint or message on screen, for the status overlay.
#[derive(Resource, Default)]
pub struct Hints {
    pub text: Option<String>,
    left: f32,
    shown: Vec<Hint>,
    rom: Option<TextRom>,
    messages: Option<TextRom>,
    queued: std::collections::VecDeque<ShowMessage>,
    /// What's up is a message (hints wait for it).
    message_up: bool,
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
            .add_message::<ShowMessage>()
            .init_resource::<Hints>()
            .add_systems(Startup, load_text)
            .add_systems(Update, (show_messages, show_hints).chain());
    }
}

fn load_text(mut hints: ResMut<Hints>, mut game: ResMut<LoadedGame>) {
    let mut read = |path: &str| {
        game.install.read(path).map_err(|e| e.to_string()).and_then(|b| TextRom::parse(&b).map_err(|e| e.to_string()))
    };
    match read("TEXT/ENGLISH.ROM") {
        Ok(rom) => hints.rom = Some(rom),
        Err(e) => warn!("no game text, hints disabled: {e}"),
    }
    match read("TEXT/SCROLL_E.ROM") {
        Ok(rom) => hints.messages = Some(rom),
        Err(e) => warn!("no message text: {e}"),
    }
}

/// Shows queued messages one after another, each for its time.
fn show_messages(
    time: Res<Time>,
    mut hints: ResMut<Hints>,
    mut requests: MessageReader<ShowMessage>,
    mut voice: MessageWriter<PlaySound>,
) {
    hints.queued.extend(requests.read().cloned());
    if hints.message_up {
        hints.left -= time.delta_secs();
        if hints.left > 0.0 {
            return;
        }
        hints.message_up = false;
        hints.text = None;
    }
    while let Some(m) = hints.queued.pop_front() {
        let group = |rom: &Option<TextRom>| rom.as_ref().and_then(|r| r.group(&m.group)).and_then(|g| g.strings.get(m.index)).cloned();
        let text = group(&hints.messages).or_else(|| group(&hints.rom));
        let Some(text) = text else {
            warn!("no message {} {}", m.group, m.index);
            continue;
        };
        // The font is ASCII-only; the game's line breaks stay.
        hints.text = Some(text.replace('\r', "").chars().filter(|c| c.is_ascii()).collect());
        info!("message: {}", hints.text.as_deref().unwrap_or_default().replace('\n', " "));
        hints.left = m.seconds.unwrap_or(MESSAGE_SECONDS);
        hints.message_up = true;
        if let Some(line) = m.voice {
            voice.write(PlaySound(line.into()));
        }
        break;
    }
}

fn show_hints(
    time: Res<Time>,
    mut hints: ResMut<Hints>,
    mut requests: MessageReader<ShowHint>,
    mut voice: MessageWriter<PlaySound>,
) {
    if hints.message_up {
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
        voice.write(PlaySound(line.into()));
    }
}
