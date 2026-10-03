//! The tower wizard's scenes (`docs/items.md`, "The tower wizard's
//! scenes"): as the tower loads, a hero's new rank (every ten levels) and
//! the first shard or runestone won since the wizard last spoke are
//! announced. A second after the arrival's shot,
//! the wizard — `WIZARD`, a glowing apparition — stands at the lookout
//! nearest the heroes and the camera cuts to that lookout's point; two
//! seconds later his words type out in the bottom bar as he says them, and
//! three seconds after they're typed he goes: the piece's effect plays in
//! full at its place in the tower under a cut to it, and what follows
//! depends on what's now held — more shards to find, all eight (the
//! window's light comes on), a stone, all twelve, the thirteenth.

use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::population::{LocatorKind, PlacementParams};

use crate::audio::{EffectName, PlaySoundAt, QueueVoice};
use crate::character::CharacterModel;
use crate::effects::{EffectAt, EffectOn, ParticleSystems};
use crate::fade::Fade;
use crate::level_material::LevelMaterial;
use crate::mechanics::{LevelNodes, Mechanics};
use crate::message_box::{Captions, ShowCaption, TextFile};
use crate::play_camera::{PlayCamera, StartCut};
use crate::player::{Player, PlayerTick};
use crate::party::Party;
use crate::player_state::PlayerState;
use crate::population::{self, LevelPopulation};
use crate::quest::{self, ShardLight};
use crate::tower;
use crate::world::LevelEntity;

pub struct TowerScenesPlugin;

impl Plugin for TowerScenesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Scene>()
            .add_systems(
                FixedUpdate,
                run_scene.after(PlayerTick).after(crate::message_box::CaptionTick).before(crate::play_camera::tick),
            )
            .add_systems(Update, show_reveals);
    }
}

/// What the wizard announces (the game's `r13-0x6e8c` codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announce {
    /// A shard, 1–8: `NEWSHARDS` page n, `S_SHRD4<realm>`.
    Shard(u8),
    /// More to find: `MORESHARDS`, `S_CONTINUEVOX`.
    MoreShards,
    /// All eight: `ALLSHARDS`, `S_4KEYVOX`.
    AllShards,
    /// A runestone, 0–12 (12 the thirteenth): `NEWRUNES`, `S_FNDRUNEYOU`.
    Rune(u8),
    /// Back from the battlefield without the thirteenth: `RUNE13NO`.
    Rune13No,
    /// All twelve stones, the Underworld closed or open: `ALL12RUNESNO`,
    /// `ALL12RUNESYES`.
    Twelve { open: bool },
    /// The thirteenth: `RUNE13YES`.
    Rune13Yes,
    /// All twelve stones announced and the Desecrated Temple done: no
    /// words; the Underworld's portal is shown again.
    Underworld,
}

/// The shards' voice lines, 1–8.
const SHARD_VOICES: [&str; 9] =
    ["", "S_SHRD4TWN", "S_SHRD4MNT", "S_SHRD4CST", "S_SHRD4SKY", "S_SHRD4FOR", "S_SHRD4DES", "S_SHRD4ICE", "S_SHRD4DRM"];

impl Announce {
    /// Its text group and page (none: every page), and voice line.
    fn words(self) -> Option<(&'static str, Option<usize>, &'static str)> {
        Some(match self {
            Self::Shard(n) => ("NEWSHARDS", Some(usize::from(n)), SHARD_VOICES[usize::from(n).min(8)]),
            Self::MoreShards => ("MORESHARDS", None, "S_CONTINUEVOX"),
            Self::AllShards => ("ALLSHARDS", None, "S_4KEYVOX"),
            Self::Rune(_) => ("NEWRUNES", Some(0), "S_FNDRUNEYOU"),
            Self::Rune13No => ("RUNE13NO", None, "S_RUNE13NO"),
            Self::Twelve { open: false } => ("ALL12RUNESNO", None, "S_12RUNENO"),
            Self::Twelve { open: true } => ("ALL12RUNESYES", None, "S_12RUNEYES"),
            Self::Rune13Yes => ("RUNE13YES", None, "S_RUNE13YES"),
            Self::Underworld => return None,
        })
    }

    /// Seconds after the words are typed before he goes.
    fn wait(self) -> f32 {
        match self {
            Self::Shard(_) | Self::Rune(_) | Self::Rune13No => 3.0,
            _ => 2.0,
        }
    }

    /// The table of lookout camera points his view comes from: the
    /// shards' or the runes'.
    fn cameras(self) -> usize {
        match self {
            Self::Shard(_) | Self::MoreShards | Self::AllShards => 1,
            _ => 2,
        }
    }
}

/// A hero's new rank to announce: the level reached, the class (the
/// game's order) and colour (yellow, blue, red, green).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rank {
    pub level: u32,
    pub class: usize,
    pub colour: usize,
}

/// The classes' codes in their rank voice lines (`S_EXP10WAR`…).
const RANK_VOICE_CODES: [&str; 16] =
    ["WAR", "VAL", "WIZ", "ARC", "DWA", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR", "UNI", "MED", "HYE"];
/// The colours' and the classes' codes in the announcer's names for a hero
/// (`S_BLUWAR2`: "Blue Warrior", from the class's own bank).
const NAME_COLOUR_CODES: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
const NAME_CLASS_CODES: [&str; 16] =
    ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR", "UNI", "MED", "HYE"];
/// His rank sentence is dropped when it would wait longer than this,
/// seconds; a piece's line, `PIECE_MOST_WAIT`.
const RANK_MOST_WAIT: f32 = 5.0;
const PIECE_MOST_WAIT: f32 = 10.0;
/// The level-up flash (`WEAPONS`; the game's level-up puts it on the hero
/// too, `levelup.rs`) and the gem sparkle, by colour.
pub const LEVELUP_FLASHES: [&str; 4] = ["LEVELUP_YEL", "LEVELUP_BLU", "LEVELUP_RED", "LEVELUP_GRE"];
const RANK_SPARKLES: [&str; 4] = ["GETGEMYELLOW", "GETGEMBLUE", "GETGEMRED", "GETGEMGREEN"];

impl Rank {
    /// The line he says: `S_EXP<tens><class>`, `S_EXP99ALL` at 99.
    pub fn voice(self) -> String {
        if self.level >= 99 {
            return "S_EXP99ALL".into();
        }
        format!("S_EXP{}{}", self.level / 10 * 10, RANK_VOICE_CODES.get(self.class).copied().unwrap_or("WAR"))
    }

    /// The hero's name as the announcer says it first: `S_<colour><class>2`
    /// ("Blue Warrior").
    pub fn name_line(self) -> String {
        let colour = NAME_COLOUR_CODES.get(self.colour).copied().unwrap_or("YEL");
        format!("S_{colour}{}2", NAME_CLASS_CODES.get(self.class).copied().unwrap_or("WAR"))
    }

    /// What he says: the hero's name, then the rank line.
    pub fn sentence(self) -> QueueVoice {
        QueueVoice::announcer(self.name_line(), RANK_MOST_WAIT).then(self.voice())
    }

    /// His words: `NEWLEVEL` ("%s %s is now / a level %d %s!") with the
    /// colour, the class, the level and the rank — the class's list
    /// (`CLASS_RANK`) at (level ÷ 10) ÷ 2, or `LEGEND` at 99.
    pub fn words(self, english: &gdl_formats::text::TextRom) -> Option<String> {
        let format = english.get("NEWLEVEL", 0)?;
        let colour = english.get("PLAYER_COLOR_LC", self.colour)?;
        let class = english.get("PLAYER_CLASS_LC", self.class)?;
        let rank = if self.level >= 99 {
            english.get("LEGEND", 0)?
        } else {
            let list = english.list("CLASS_RANK")?;
            let group = english.groups.get(*list.groups.get(self.class)?)?;
            group.strings.get((self.level / 10 / 2) as usize)?.as_str()
        };
        Some(printf(format, &[colour, class, &self.level.to_string(), rank]))
    }
}

/// The game's `%s`/`%d` formatting, the arguments in turn.
fn printf(format: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' && matches!(chars.peek(), Some('s' | 'd')) {
            chars.next();
            out.push_str(args.next().copied().unwrap_or_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// What the tower announces as it loads: the first shard won but not
/// announced before (bits 1–8 of the marks), else — it wins — the first
/// stone so (0–12); with no new stone, back from finishing the
/// battlefield (`levelH3`) without the thirteenth announced, the
/// thirteenth's absence, and with all twelve announced and the Desecrated
/// Temple done (that wins), the Underworld's portal. The announced bits
/// are those before this load.
pub fn announcement(
    marks: u32,
    shards_announced: u32,
    runes: u32,
    runes_announced: u32,
    finished: Option<&str>,
) -> Option<Announce> {
    let new_shards = marks & !shards_announced;
    let new_runes = runes & !runes_announced;
    let mut out = (1..=8u8).find(|n| new_shards & (1 << n) != 0).map(Announce::Shard);
    if let Some(i) = (0..13u8).find(|i| new_runes & (1 << i) != 0) {
        return Some(Announce::Rune(i));
    }
    if finished.is_some_and(|p| p.eq_ignore_ascii_case(BATTLEFIELD)) && runes_announced & RUNE13 == 0 {
        out = Some(Announce::Rune13No);
    }
    if runes_announced & ALL_TWELVE == ALL_TWELVE && marks & TEMPLE_MARK != 0 {
        out = Some(Announce::Underworld);
    }
    out
}

/// The battlefield the thirteenth stone is found on.
const BATTLEFIELD: &str = "levelH3";
const RUNE13: u32 = 1 << 12;
const ALL_TWELVE: u32 = 0xFFF;
const ALL_THIRTEEN: u32 = 0x1FFF;
/// The Desecrated Temple's mark (the 9th of the tower's order), and the
/// shards with it, and with the Underworld's too.
const TEMPLE_MARK: u32 = 1 << 9;
const SHARDS_AND_TEMPLE: u32 = 0x3FE;
const SHARDS_TEMPLE_UNDERWORLD: u32 = 0x7FE;

/// The camera point tables for the wizard's lookouts (a kind-9 point's
/// index is the table's base + the lookout's parameter): heroes' ranks,
/// shards, runes. A missing point falls back to the table before.
const LOOKOUT_CAMERAS: [i16; 3] = [0xF0, 0xDC, 0xAA];
/// The places' camera points: the window, the rune place (and the
/// Underworld's portal), the thirteenth stone, the Desecrated Temple's
/// portal, Garm's Citadel's.
const WINDOW_CAMERA: i16 = 0xCA;
const RUNES_CAMERA: i16 = 0xC9;
const RUNE13_CAMERA: i16 = 0xCB;
const TEMPLE_CAMERA: i16 = 0xCC;
const CITADEL_CAMERA: i16 = 0xCD;
/// A place's cut: 40 fields, then 300 more the scene waits on.
const PLACE_HOLD: f32 = 40.0;
const PLACE_EXTRA: f32 = 300.0;
/// The lookout's cut holds until the scene ends it.
const FOREVER: f32 = 1_000_000.0;
/// Fields before the wizard appears, and from then to his words.
const START_DELAY: f32 = 60.0;
const WORDS_DELAY: f32 = 120.0;
/// Where his words type: the bottom bar.
const WORDS_Y: f32 = 312.0;
/// A cut is over when both its counts are under this.
const CUT_DONE: f32 = 5.0;
/// The chimes and knocks come when a place's extra count drops under
/// these (50 fields in; 80 for the thirteenth).
const CHIME_AT: f32 = 250.0;
const KNOCK13_AT: f32 = 220.0;
/// The scenes' sounds — the glass's chime, the stones' knocks and fall,
/// the shards' — play centred at this requested volume (twice a call's
/// own, `audio::PlaySoundAt`).
const CHIME_VOLUME: u8 = 0xFF;
/// Reveals fade in over this many fields.
const REVEAL_FIELDS: f32 = 180.0;
/// Fields and seconds a tick.
const FIELDS: f32 = 2.0;
const TICK: f32 = 1.0 / 30.0;

/// The seed a placed piece's particles are sprayed with.
const PIECE_SEED: u32 = 0x5EED;

/// The exits' destination codes the reveals work on: the Desecrated
/// Temple's, the Underworld's, Garm's Citadel's.
const TEMPLE_EXIT: i32 = 0x500;
const UNDERWORLD_EXIT: i32 = 0x600;
const CITADEL_EXIT: i32 = 0x803;

/// What the scene has made see-through: exits (by destination code) and
/// the window's light, each 0 whole … 1 clear.
#[derive(Default)]
struct Reveal {
    exits: Vec<(i32, f32)>,
    light: Option<f32>,
}

impl Reveal {
    /// Makes an exit clear (the game's transparency 255).
    fn clear(&mut self, code: i32) {
        self.set(code, 1.0);
    }

    fn set(&mut self, code: i32, amount: f32) {
        match self.exits.iter_mut().find(|(c, _)| *c == code) {
            Some(e) => e.1 = amount,
            None => self.exits.push((code, amount)),
        }
    }
}

/// The scene: what's announced and how far it's gone.
#[derive(Resource, Default)]
pub struct Scene {
    announce: Option<Announce>,
    /// Exits and the light made clear, and fading in.
    reveal: Reveal,
    /// The exit the fade under way brings in (none: only the light).
    fading: Option<i32>,
    /// Fields before the wizard appears.
    delay: f32,
    /// The wizard, while he's up.
    wizard: Option<Entity>,
    /// Fields since he appeared (his words start at 120).
    fields: f32,
    /// Whether his words and voice have started.
    spoken: bool,
    /// Seconds left once his words are typed.
    wait: f32,
    /// The steps after a piece is placed, as the game numbers them
    /// (`r13-0x6e7c`): 0x0A–0x10 shards, 0x14–0x1A stones, 0x1E–0x24 the
    /// thirteenth; 0x74, 0x7E, 0x88 a reveal fading in.
    follow: u8,
    /// A reveal's fields left.
    fade: f32,
    wizard_model: Option<Arc<CharacterModel>>,
    piece: Option<(Arc<CharacterModel>, ParticleSystems)>,
    /// A hero's new rank, announced first.
    rank: Option<Rank>,
    /// The rank's voice line, and whether its flash and sparkle came.
    rank_voice: Option<String>,
    flashed: bool,
    sparkled: bool,
}

/// After his words for a rank alone, he goes this many seconds later.
const RANK_WAIT: f32 = 0.5;
/// A rank's words stay at least this long (and until its voice is done).
const RANK_FIELDS: f32 = 360.0;
/// The flash and the sparkle come this far into them.
const FLASH_AT: f32 = 239.0;
const SPARKLE_AT: f32 = 269.0;

impl Scene {
    /// Readies the scene in the tower: the wizard's model, and what the
    /// tower announces as it loads (a rank, then a piece with its model).
    pub fn arm(
        &mut self,
        wizard: Arc<CharacterModel>,
        rank: Option<Rank>,
        what: Option<Announce>,
        piece: Option<(Arc<CharacterModel>, ParticleSystems)>,
    ) {
        *self = Self { wizard_model: Some(wizard), piece, ..default() };
        if let Some(what) = what {
            self.again(what);
        }
        self.rank = rank;
        if rank.is_some() && what.is_none() {
            self.wait = RANK_WAIT;
        }
        let at_once = what == Some(Announce::Underworld) && rank.is_none();
        self.delay = if at_once { 0.0 } else { START_DELAY };
    }

    /// Announces `what` (the wizard comes at once).
    fn again(&mut self, what: Announce) {
        self.announce = Some(what);
        self.delay = 0.0;
        self.fields = 0.0;
        self.spoken = false;
        self.wait = what.wait();
    }

    /// Whether the heroes' pads are held: from the announcement until the
    /// piece is placed.
    pub fn holds_input(&self) -> bool {
        self.announce.is_some() || self.rank.is_some()
    }

    /// Whether he's announcing something.
    fn active(&self) -> bool {
        self.announce.is_some() || self.rank.is_some()
    }
}

/// The wizard of the scenes as the game makes him (`0xC00880`): drawn
/// additively without depth writes.
pub fn apparition(model: CharacterModel, materials: &mut Assets<LevelMaterial>) -> Arc<CharacterModel> {
    for h in model.materials() {
        if let Some(m) = materials.get_mut(h) {
            m.alpha_mode = AlphaMode::Add;
            m.uv_offset.z = 1.0;
            m.depth_write = false;
        }
    }
    Arc::new(model)
}

/// A kind-9 camera point by index.
fn camera_point(population: &LevelPopulation, index: i16) -> Option<usize> {
    population.population.locators.iter().position(|l| l.kind == LocatorKind::Transmitter(9) && l.index == index)
}

/// A place's cut: 40 fields, then 300 more.
fn place_cut(population: &LevelPopulation, index: i16, cuts: &mut MessageWriter<StartCut>) {
    match camera_point(population, index) {
        Some(locator) => {
            cuts.write(StartCut { locator, node: None, hold: Some(PLACE_HOLD), delay: 0.0, extra: PLACE_EXTRA });
        }
        None => warn!("the tower has no camera point {index:#x}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_scene(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    camera: Option<ResMut<PlayCamera>>,
    (mut captions, mut caption_requests): (ResMut<Captions>, MessageWriter<ShowCaption>),
    (mut cuts, mut sounds, mut effects, mut riding): (
        MessageWriter<StartCut>,
        MessageWriter<PlaySoundAt>,
        MessageWriter<EffectAt>,
        MessageWriter<EffectOn>,
    ),
    mut voices: MessageWriter<QueueVoice>,
    (population, nodes): (Option<Res<LevelPopulation>>, Option<Res<LevelNodes>>),
    (players, playing, heroes): (Query<&Player>, Query<&EffectName>, Query<Entity, With<Player>>),
    mut party: ResMut<Party>,
    (mut light, mut meshes): (ResMut<ShardLight>, ResMut<Assets<Mesh>>),
    mechanics: Option<ResMut<Mechanics>>,
) {
    // Only in the tower, once it's readied (the wizard's model is built).
    if scene.wizard_model.is_none() {
        return;
    }
    let (Some(mut camera), Some(population), Some(nodes)) = (camera, population, nodes) else {
        return;
    };
    // Nothing starts under the arrival's opening shot.
    if camera.opening() {
        return;
    }
    let scene = &mut *scene;
    // A hero who reaches a new ten of levels in the tower hears of it.
    if !scene.active() && scene.follow == 0 {
        // Each player's check; the first new rank is announced.
        for (_, member) in party.members_mut() {
            let (keep, rank) = check_rank(&member.state, Some(&member.choice));
            if let Some(level) = keep {
                member.state.quest.rank_level = Some(level);
            }
            if let Some(rank) = rank
                && scene.rank.is_none()
            {
                info!("the wizard will announce {rank:?}");
                scene.rank = Some(rank);
                scene.wait = RANK_WAIT;
            }
        }
        if !scene.active() {
            return;
        }
    }
    if scene.active() {
        if scene.delay > 0.0 {
            scene.delay -= FIELDS;
            return;
        }
        let what = scene.announce;
        if scene.rank.is_some() || what != Some(Announce::Underworld) {
            if scene.wizard.is_none() {
                let cameras = if scene.rank.is_some() { 0 } else { what.map_or(0, Announce::cameras) };
                appear(scene, cameras, &population, &players, &mut commands, &mut cuts);
            }
            scene.fields += FIELDS;
            if scene.fields < WORDS_DELAY {
                return;
            }
            let since = scene.fields - WORDS_DELAY;
            if let Some(rank) = scene.rank {
                let hero = players.iter().next().map_or(Vec3::ZERO, |p| Vec3::from(p.mover.position));
                if !scene.spoken {
                    scene.spoken = true;
                    captions.clear();
                    let text = captions.english().and_then(|e| rank.words(e));
                    caption_requests.write(ShowCaption {
                        file: TextFile::English,
                        group: String::new(),
                        index: None,
                        y: WORDS_Y,
                        stay: true,
                        text: Some(text.unwrap_or_default()),
                    });
                    voices.write(rank.sentence());
                    scene.rank_voice = Some(rank.voice());
                    return;
                }
                let colour = rank.colour.min(3);
                // The flash and sparkle ride the hero.
                let rider = heroes.iter().next();
                if since > FLASH_AT && !scene.flashed {
                    scene.flashed = true;
                    let name = LEVELUP_FLASHES[colour];
                    match rider {
                        Some(on) => {
                            riding.write(EffectOn { name, bank: None, on, scale: 1.0 });
                        }
                        None => {
                            effects.write(EffectAt { name, bank: None, at: hero, facing: 0.0, scale: 1.0 });
                        }
                    }
                }
                if since > SPARKLE_AT && !scene.sparkled {
                    scene.sparkled = true;
                    let (name, bank) = (RANK_SPARKLES[colour], Some("POWERUPS"));
                    match rider {
                        Some(on) => {
                            riding.write(EffectOn { name, bank, on, scale: 1.0 });
                        }
                        None => {
                            effects.write(EffectAt { name, bank, at: hero, facing: 0.0, scale: 1.0 });
                        }
                    }
                }
                let voice_on = scene.rank_voice.as_ref().is_some_and(|v| playing.iter().any(|n| &n.0 == v));
                if captions.done() && since >= RANK_FIELDS && !voice_on {
                    // On to the piece, its words at once.
                    scene.rank = None;
                    scene.spoken = false;
                    scene.fields = WORDS_DELAY;
                    captions.clear();
                }
                return;
            }
            let Some(what) = what else {
                // A rank alone: he goes half a second later.
                scene.wait -= TICK;
                if scene.wait <= 0.0 {
                    leave(scene, &mut camera, &mut captions, &mut commands);
                }
                return;
            };
            if !scene.spoken {
                scene.spoken = true;
                if let Some((group, index, voice)) = what.words() {
                    captions.clear();
                    caption_requests.write(ShowCaption {
                        file: TextFile::Scroll,
                        group: group.into(),
                        index,
                        y: WORDS_Y,
                        stay: true,
                        text: None,
                    });
                    voices.write(QueueVoice::announcer(voice, PIECE_MOST_WAIT));
                }
                return;
            }
            if !captions.done() {
                return;
            }
            scene.wait -= TICK;
            if scene.wait > 0.0 {
                return;
            }
        }
        let Some(what) = what else { return };
        let progress = party.states().fold((0, 0), |(beaten, runes), (_, s)| (beaten | s.realms_beaten, runes | s.runestone_bits()));
        place(scene, what, progress, &population, &nodes, &mut camera, &mut captions, &mut light, &mut commands, &mut cuts, &mut sounds, &mut meshes);
        // The place's cut starts on the camera's tick: the steps after it
        // wait for the next.
        return;
    }
    let (hold, extra) = camera.cut_counts().unwrap_or((0.0, 0.0));
    let done = hold < CUT_DONE && extra < CUT_DONE;
    let sound = |name: &str, sounds: &mut MessageWriter<PlaySoundAt>| {
        sounds.write(PlaySoundAt::centred(name, CHIME_VOLUME));
    };
    scene.follow = match scene.follow {
        // All eight: the glass's chime, then the Desecrated Temple's portal
        // and the light fade in (stand-in: the portal's fade; the light
        // comes on at once).
        0x0A if extra < CHIME_AT => {
            sound("S_STNDGLASS", &mut sounds);
            0x0B
        }
        0x0B if done => {
            place_cut(&population, TEMPLE_CAMERA, &mut cuts);
            light.0 = true;
            scene.reveal.light = Some(1.0);
            scene.reveal.clear(TEMPLE_EXIT);
            scene.fading = Some(TEMPLE_EXIT);
            scene.fade = REVEAL_FIELDS;
            0x74
        }
        // More to find: the chime, then his words.
        0x0E if extra < CHIME_AT => {
            sound("S_STNDGLASS", &mut sounds);
            0x0F
        }
        0x0F if done => {
            scene.again(Announce::MoreShards);
            0
        }
        0x10 if done => {
            scene.again(Announce::AllShards);
            0
        }
        // A stone: its knock; after the twelfth, his words or the
        // Underworld's reveal.
        f @ (0x14 | 0x16 | 0x18) if extra < CHIME_AT => {
            sound("S_RUNEHIT", &mut sounds);
            f + 1
        }
        0x15 => {
            scene.reveal.clear(UNDERWORLD_EXIT);
            if done {
                place_cut(&population, RUNES_CAMERA, &mut cuts);
                scene.fading = Some(UNDERWORLD_EXIT);
                scene.fade = REVEAL_FIELDS;
                0x7E
            } else {
                0x15
            }
        }
        0x17 | 0x21 => 0,
        0x19 if done => {
            scene.again(Announce::Twelve { open: false });
            0
        }
        0x1A if done => {
            scene.again(Announce::Twelve { open: true });
            0
        }
        // The thirteenth: its knock; with all thirteen, Garm's Citadel's
        // reveal (its trigger 0xFF fires) and his words.
        f @ (0x1E | 0x20) if extra < KNOCK13_AT => {
            sound("S_RUNEHIT", &mut sounds);
            f + 1
        }
        0x1F if done => {
            place_cut(&population, CITADEL_CAMERA, &mut cuts);
            if let Some(mut m) = mechanics {
                m.fire(0xFF, false);
            }
            scene.reveal.clear(CITADEL_EXIT);
            scene.fading = Some(CITADEL_EXIT);
            scene.fade = REVEAL_FIELDS;
            0x88
        }
        0x24 if done => {
            scene.again(Announce::Rune13Yes);
            0
        }
        // A reveal fading in — the exit, and with the eighth shard the
        // light: see-through by what's left of the 180 fields — then the
        // step 100 below.
        f @ (0x74 | 0x7E | 0x88) => {
            scene.fade = (scene.fade - FIELDS).max(0.0);
            let clear = scene.fade / REVEAL_FIELDS;
            if let Some(code) = scene.fading {
                scene.reveal.set(code, clear);
            }
            if f == 0x74 {
                scene.reveal.light = Some(clear);
            }
            if scene.fade < 1.0 {
                if let Some(code) = scene.fading.take() {
                    scene.reveal.set(code, 0.0);
                }
                if f == 0x74 {
                    scene.reveal.light = Some(0.0);
                }
                f - 100
            } else {
                f
            }
        }
        f => f,
    };
}

/// The wizard appears at the lookout nearest the heroes, facing its way,
/// and the camera cuts to the lookout's point until he goes.
fn appear(
    scene: &mut Scene,
    cameras: usize,
    population: &LevelPopulation,
    players: &Query<&Player>,
    commands: &mut Commands,
    cuts: &mut MessageWriter<StartCut>,
) {
    let heroes: Vec<Vec3> = players.iter().map(|p| Vec3::from(p.mover.position)).collect();
    let centre = if heroes.is_empty() { Vec3::ZERO } else { heroes.iter().sum::<Vec3>() / heroes.len() as f32 };
    let lookout = population
        .population
        .locators
        .iter()
        .filter(|l| matches!(l.kind, LocatorKind::Lookout(_)))
        .min_by(|a, b| {
            let d = |l: &&gdl_formats::population::Locator| Vec3::from(l.position).distance_squared(centre);
            d(a).total_cmp(&d(b))
        });
    let Some(lookout) = lookout else {
        warn!("the tower has no lookout for the wizard");
        return;
    };
    if let Some(model) = &scene.wizard_model {
        let root = model.spawn(population::lookout_transform(lookout), commands);
        commands.entity(root).insert(LevelEntity);
        scene.wizard = Some(root);
    }
    let point = (0..=cameras)
        .rev()
        .find_map(|t| camera_point(population, LOOKOUT_CAMERAS[t] + i16::from(lookout.param)));
    match point {
        Some(locator) => {
            cuts.write(StartCut { locator, node: None, hold: Some(FOREVER), delay: 0.0, extra: 0.0 });
        }
        None => warn!("no camera point for the lookout {}", lookout.param),
    }
    info!("the wizard appears at lookout {} {:?}", lookout.param, lookout.position);
}

/// He goes: his model, his words and his cut.
fn leave(scene: &mut Scene, camera: &mut PlayCamera, captions: &mut Captions, commands: &mut Commands) {
    if let Some(w) = scene.wizard.take() {
        commands.entity(w).try_despawn();
    }
    captions.clear();
    camera.end_cut();
    scene.announce = None;
}

/// The hero's rank check: the level to keep (when it moved since the last
/// check; the first check only keeps it), and the rank to announce if it
/// has reached a new ten (or 99).
pub fn check_rank(state: &PlayerState, choice: Option<&crate::player::PlayerChoice>) -> (Option<u32>, Option<Rank>) {
    if state.quest.rank_level == Some(state.level) {
        return (None, None);
    }
    let before = state.quest.rank_level.unwrap_or(state.level);
    if !quest::rank_changed(before, state.level) {
        return (Some(state.level), None);
    }
    let rank = choice.and_then(|choice| {
        let class = crate::character::class_index(&choice.class)?;
        let colour = COLOURS.iter().position(|c| choice.variant.to_ascii_uppercase().starts_with(c)).unwrap_or(0);
        Some(Rank { level: state.level, class, colour })
    });
    (Some(state.level), rank)
}

/// Colour codes in their order (yellow, blue, red, green).
const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];

/// He goes, and the piece goes to its place (`docs/items.md`): its effect
/// plays in full under a cut to it, and the steps after it are chosen by
/// what's held now.
#[allow(clippy::too_many_arguments)]
fn place(
    scene: &mut Scene,
    what: Announce,
    (beaten, runes): (u32, u32),
    population: &LevelPopulation,
    nodes: &LevelNodes,
    camera: &mut PlayCamera,
    captions: &mut Captions,
    light: &mut ShardLight,
    commands: &mut Commands,
    cuts: &mut MessageWriter<StartCut>,
    sounds: &mut MessageWriter<PlaySoundAt>,
    meshes: &mut Assets<Mesh>,
) {
    leave(scene, camera, captions, commands);
    // The party's: every player's realms beaten and stones held.
    let marks = quest::boss_marks(beaten);
    // The piece's effect in full: its model, and its particles sprayed
    // there (a shard's; the stones have none).
    let mut set_out = |place: &str| {
        if let (Some(at), Some((model, particles))) = (tower::node_at(nodes, place), scene.piece.take()) {
            let root = model.spawn(Transform::from_translation(at), commands);
            commands.entity(root).insert(LevelEntity);
            let mut seed = PIECE_SEED;
            crate::effects::spray(&particles, at, 1.0, f32::INFINITY, &mut seed, commands, meshes);
        }
    };
    let sound = |name: &str, sounds: &mut MessageWriter<PlaySoundAt>| {
        sounds.write(PlaySoundAt::centred(name, CHIME_VOLUME));
    };
    scene.follow = match what {
        Announce::Shard(n) => {
            set_out(tower::shard_piece(n).1);
            place_cut(population, WINDOW_CAMERA, cuts);
            light.0 = false;
            if marks & quest::ALL_SHARDS == quest::ALL_SHARDS {
                sound("S_SHRDS127", sounds);
                scene.reveal.clear(TEMPLE_EXIT);
                0x0A
            } else {
                sound("S_SHRD8", sounds);
                0x0E
            }
        }
        Announce::Rune(12) => {
            set_out(tower::rune_piece(12).1);
            place_cut(population, RUNE13_CAMERA, cuts);
            scene.reveal.clear(CITADEL_EXIT);
            if runes & ALL_THIRTEEN == ALL_THIRTEEN { 0x1E } else { 0x20 }
        }
        Announce::Rune(_) | Announce::Rune13No | Announce::Underworld => {
            let stone = match what {
                Announce::Rune(i) => i,
                Announce::Rune13No => 13,
                _ => 14,
            };
            if stone < 13 {
                set_out(tower::rune_piece(stone).1);
                place_cut(population, RUNES_CAMERA, cuts);
                sound("S_RUNEFALL", sounds);
            }
            let nine = marks & SHARDS_AND_TEMPLE == SHARDS_AND_TEMPLE;
            let twelve = runes & ALL_TWELVE == ALL_TWELVE;
            let next = if !nine || !twelve {
                if twelve { 0x18 } else { 0x16 }
            } else if stone < 13 {
                0x14
            } else if marks & SHARDS_TEMPLE_UNDERWORLD != SHARDS_TEMPLE_UNDERWORLD {
                0x15
            } else {
                0
            };
            // With a step to follow, the Underworld's exit is made clear.
            if next != 0 {
                scene.reveal.clear(UNDERWORLD_EXIT);
            }
            next
        }
        // The words after a piece: nothing more goes anywhere.
        _ => 0,
    };
    info!("the wizard's scene placed {what:?}: step {:#x}", scene.follow);
}

/// An exit's destination as the game's code: realm × 0x100 + level.
fn exit_code(destination: &str) -> Option<i32> {
    crate::items::exit_destination(destination).map(|(realm, level)| (realm << 8 | level) as i32)
}

/// Draws what the scene made see-through: the exits by their destination
/// code (every model of the placement), and the window's light.
fn show_reveals(
    mut commands: Commands,
    scene: Res<Scene>,
    items: Option<Res<crate::items::LevelItems>>,
    models: Query<(Entity, &crate::population::PlacementIndex)>,
    pieces: Query<(Entity, &quest::TowerPiece)>,
    mut fades: Query<&mut Fade>,
) {
    if !scene.is_changed() {
        return;
    }
    let mut fade = |e: Entity, amount: f32, commands: &mut Commands| match fades.get_mut(e) {
        Ok(mut f) => {
            if f.amount != amount {
                f.amount = amount;
            }
        }
        Err(_) if amount > 0.0 => {
            commands.entity(e).try_insert(Fade::new(amount));
        }
        Err(_) => {}
    };
    if let Some(items) = items.as_deref() {
        for &(code, amount) in &scene.reveal.exits {
            let placements: Vec<usize> = items
                .views()
                .filter(|v| matches!(v.params, PlacementParams::Exit { destination: Some(d) } if exit_code(d) == Some(code)))
                .map(|v| v.placement)
                .collect();
            for (e, p) in &models {
                if placements.contains(&p.0) {
                    fade(e, amount, &mut commands);
                }
            }
        }
    }
    if let Some(amount) = scene.reveal.light {
        for (e, piece) in &pieces {
            if *piece == quest::TowerPiece::ShardLight {
                fade(e, amount, &mut commands);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: u32 = 1 << 1;
    const B: u32 = 1 << 2;

    #[test]
    fn the_first_new_piece_is_announced_a_stone_before_a_shard() {
        assert_eq!(announcement(G, 0, 0, 0, None), Some(Announce::Shard(1)));
        assert_eq!(announcement(G | B, G, 0, 0, None), Some(Announce::Shard(2)));
        assert_eq!(announcement(G, G, 0, 0, None), None);
        assert_eq!(announcement(G | B, G, 1 << 3, 0, None), Some(Announce::Rune(3)));
        assert_eq!(announcement(0, 0, RUNE13 | 0xFFF, 0xFFF, None), Some(Announce::Rune(12)));
    }

    #[test]
    fn ranks_every_ten_levels_and_at_ninety_nine() {
        assert!(!quest::rank_changed(1, 9));
        assert!(quest::rank_changed(9, 10));
        assert!(!quest::rank_changed(10, 19));
        assert!(quest::rank_changed(15, 32));
        assert!(quest::rank_changed(98, 99));
        assert!(!quest::rank_changed(99, 99));
        let rank = Rank { level: 20, class: 4, colour: 2 };
        assert_eq!(rank.voice(), "S_EXP20DWA");
        assert_eq!(rank.name_line(), "S_REDDWF2");
        assert_eq!(Rank { level: 99, ..rank }.voice(), "S_EXP99ALL");
        assert_eq!(printf("%s %s is now\na level %d %s!\n", &["Red", "Dwarf", "20", "Warrior"]), "Red Dwarf is now\na level 20 Warrior!\n");
    }

    /// The words come from `TEXT/ENGLISH.ROM` (real data).
    #[test]
    fn rank_words() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(bytes) = std::fs::read(std::path::Path::new(&root).join("TEXT/ENGLISH.ROM")) else {
            eprintln!("skipping: no ENGLISH.ROM");
            return;
        };
        let rom = gdl_formats::text::TextRom::parse(&bytes).unwrap();
        let words = |level, class, colour| Rank { level, class, colour }.words(&rom).unwrap();
        assert_eq!(words(10, 0, 1), "Blue Warrior is now\na level 10 Fighter!\n");
        assert_eq!(words(99, 3, 0), "Yellow Archer is now\na level 99 Legend!\n");
        assert!(words(40, 3, 3).starts_with("Green Archer is now\na level 40 Ranger"));
    }

    /// Every hero's name line and rank line is in the sound catalog (real
    /// data).
    #[test]
    fn rank_lines_are_in_the_catalog() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(bytes) = std::fs::read(std::path::Path::new(&root).join("AUDIO/AUDATPS2.ROM")) else {
            eprintln!("skipping: no AUDATPS2.ROM");
            return;
        };
        let catalog = gdl_formats::audio::AudioCatalog::parse(&bytes).unwrap();
        for class in 0..16 {
            for colour in 0..4 {
                for level in [10, 50, 90, 99] {
                    let rank = Rank { level, class, colour };
                    for line in [rank.name_line(), rank.voice()] {
                        assert!(catalog.find_sound(&line).is_some(), "{line}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_battlefield_and_the_underworld() {
        // Back from the battlefield without the thirteenth.
        assert_eq!(announcement(0x3FE, 0x3FE, 0xFFF, 0xFFF, Some("levelH3")), Some(Announce::Underworld));
        assert_eq!(announcement(0x1FE, 0x1FE, 0xFFF, 0xFFF, Some("levelH3")), Some(Announce::Rune13No));
        assert_eq!(announcement(0x1FE, 0x1FE, 0x1FFF, 0x1FFF, Some("levelH3")), None);
        // Twelve announced and the temple done: the Underworld's portal.
        assert_eq!(announcement(0x3FE, 0x3FE, 0xFFF, 0xFFF, None), Some(Announce::Underworld));
        assert_eq!(announcement(0x3FE, 0x3FE, 0x7FF, 0x7FF, None), None);
    }
}
