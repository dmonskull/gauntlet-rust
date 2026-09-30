//! The tower wizard's scenes (`docs/items.md`, "The tower's wizard's
//! scenes"): as the tower loads, the first shard or runestone won since
//! the wizard last spoke is announced. A second after the arrival's shot,
//! the wizard — `WIZARD`, a glowing apparition — stands at the lookout
//! nearest the heroes and the camera cuts to that lookout's point; two
//! seconds later his words type out in the bottom bar as he says them, and
//! three seconds after they're typed he goes: the piece's effect plays in
//! full at its place in the tower under a cut to it, and what follows
//! depends on what's now held — more shards to find, all eight (the
//! window's light comes on), a stone, all twelve, the thirteenth.

use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::population::LocatorKind;

use crate::audio::PlaySound;
use crate::character::CharacterModel;
use crate::level_material::LevelMaterial;
use crate::mechanics::{LevelNodes, Mechanics};
use crate::message_box::{Captions, ShowCaption, TextFile};
use crate::play_camera::{PlayCamera, StartCut};
use crate::player::{Player, PlayerTick};
use crate::player_state::PlayerState;
use crate::population::{self, LevelPopulation};
use crate::quest::{self, ShardLight};
use crate::tower;
use crate::world::LevelEntity;

pub struct TowerScenesPlugin;

impl Plugin for TowerScenesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Scene>()
            .add_systems(FixedUpdate, run_scene.after(PlayerTick).before(crate::play_camera::tick));
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

/// What the tower announces as it loads: the first shard won but not
/// announced before (bits 1–8 of the marks), else — it wins — the first
/// stone so (0–12); with no new stone, back from the battlefield
/// (`levelH3`) without the thirteenth announced, the thirteenth's absence,
/// and with all twelve announced and the Desecrated Temple done (that
/// wins), the Underworld's portal. The announced bits are those before
/// this load.
pub fn announcement(
    marks: u32,
    shards_announced: u32,
    runes: u32,
    runes_announced: u32,
    previous: Option<&str>,
) -> Option<Announce> {
    let new_shards = marks & !shards_announced;
    let new_runes = runes & !runes_announced;
    let mut out = (1..=8u8).find(|n| new_shards & (1 << n) != 0).map(Announce::Shard);
    if let Some(i) = (0..13u8).find(|i| new_runes & (1 << i) != 0) {
        return Some(Announce::Rune(i));
    }
    if previous.is_some_and(|p| p.eq_ignore_ascii_case(BATTLEFIELD)) && runes_announced & RUNE13 == 0 {
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
/// Reveals fade in over this many fields.
const REVEAL_FIELDS: f32 = 180.0;
/// Fields and seconds a tick.
const FIELDS: f32 = 2.0;
const TICK: f32 = 1.0 / 30.0;

/// The scene: what's announced and how far it's gone.
#[derive(Resource, Default)]
pub struct Scene {
    announce: Option<Announce>,
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
    piece: Option<Arc<CharacterModel>>,
}

impl Scene {
    /// Arms the scene for `what`, with the wizard's model and the piece's.
    pub fn arm(&mut self, what: Announce, wizard: Arc<CharacterModel>, piece: Option<Arc<CharacterModel>>) {
        *self = Self { wizard_model: Some(wizard), piece, ..default() };
        self.again(what);
        self.delay = if what == Announce::Underworld { 0.0 } else { START_DELAY };
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
        self.announce.is_some()
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
    (mut cuts, mut sounds): (MessageWriter<StartCut>, MessageWriter<PlaySound>),
    population: Option<Res<LevelPopulation>>,
    nodes: Option<Res<LevelNodes>>,
    players: Query<&Player>,
    state: Option<Res<PlayerState>>,
    mut light: ResMut<ShardLight>,
    mechanics: Option<ResMut<Mechanics>>,
) {
    if scene.announce.is_none() && scene.follow == 0 {
        return;
    }
    let (Some(mut camera), Some(population), Some(nodes), Some(state)) = (camera, population, nodes, state) else {
        return;
    };
    // Nothing starts under the arrival's opening shot.
    if camera.opening() {
        return;
    }
    let scene = &mut *scene;
    if let Some(what) = scene.announce {
        if scene.delay > 0.0 {
            scene.delay -= FIELDS;
            return;
        }
        if what != Announce::Underworld {
            if scene.wizard.is_none() {
                appear(scene, what, &population, &players, &mut commands, &mut cuts);
            }
            scene.fields += FIELDS;
            if scene.fields < WORDS_DELAY {
                return;
            }
            if !scene.spoken {
                scene.spoken = true;
                if let Some((group, index, voice)) = what.words() {
                    captions.clear();
                    caption_requests.write(ShowCaption { file: TextFile::Scroll, group: group.into(), index, y: WORDS_Y, stay: true });
                    sounds.write(PlaySound(voice.into()));
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
        place(scene, what, &state, &population, &nodes, &mut camera, &mut captions, &mut light, &mut commands, &mut cuts, &mut sounds);
        // The place's cut starts on the camera's tick: the steps after it
        // wait for the next.
        return;
    }
    let (hold, extra) = camera.cut_counts().unwrap_or((0.0, 0.0));
    let done = hold < CUT_DONE && extra < CUT_DONE;
    let sound = |name: &str, sounds: &mut MessageWriter<PlaySound>| {
        sounds.write(PlaySound(name.into()));
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
        0x15 if done => {
            place_cut(&population, RUNES_CAMERA, &mut cuts);
            scene.fade = REVEAL_FIELDS;
            0x7E
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
            scene.fade = REVEAL_FIELDS;
            0x88
        }
        0x24 if done => {
            scene.again(Announce::Rune13Yes);
            0
        }
        // A reveal fading in, then the step 100 below.
        f @ (0x74 | 0x7E | 0x88) => {
            scene.fade -= FIELDS;
            if scene.fade < 1.0 { f - 100 } else { f }
        }
        f => f,
    };
}

/// The wizard appears at the lookout nearest the heroes, facing its way,
/// and the camera cuts to the lookout's point until he goes.
fn appear(
    scene: &mut Scene,
    what: Announce,
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
    let point = (0..=what.cameras())
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

/// He goes, and the piece goes to its place (`docs/items.md`): its effect
/// plays in full under a cut to it, and the steps after it are chosen by
/// what's held now.
#[allow(clippy::too_many_arguments)]
fn place(
    scene: &mut Scene,
    what: Announce,
    state: &PlayerState,
    population: &LevelPopulation,
    nodes: &LevelNodes,
    camera: &mut PlayCamera,
    captions: &mut Captions,
    light: &mut ShardLight,
    commands: &mut Commands,
    cuts: &mut MessageWriter<StartCut>,
    sounds: &mut MessageWriter<PlaySound>,
) {
    if let Some(w) = scene.wizard.take() {
        commands.entity(w).try_despawn();
    }
    captions.clear();
    camera.end_cut();
    scene.announce = None;
    let marks = quest::boss_marks(state.realms_beaten);
    let runes = state.runestone_bits();
    let mut set_out = |place: &str| {
        if let (Some(at), Some(model)) = (tower::node_at(nodes, place), scene.piece.take()) {
            let root = model.spawn(Transform::from_translation(at), commands);
            commands.entity(root).insert(LevelEntity);
        }
    };
    let sound = |name: &str, sounds: &mut MessageWriter<PlaySound>| {
        sounds.write(PlaySound(name.into()));
    };
    scene.follow = match what {
        Announce::Shard(n) => {
            set_out(tower::shard_piece(n).1);
            place_cut(population, WINDOW_CAMERA, cuts);
            light.0 = false;
            if marks & quest::ALL_SHARDS == quest::ALL_SHARDS {
                sound("S_SHRDS127", sounds);
                0x0A
            } else {
                sound("S_SHRD8", sounds);
                0x0E
            }
        }
        Announce::Rune(12) => {
            set_out(tower::rune_piece(12).1);
            place_cut(population, RUNE13_CAMERA, cuts);
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
            if !nine || !twelve {
                if twelve { 0x18 } else { 0x16 }
            } else if stone < 13 {
                0x14
            } else if marks & SHARDS_TEMPLE_UNDERWORLD != SHARDS_TEMPLE_UNDERWORLD {
                0x15
            } else {
                0
            }
        }
        // The words after a piece: nothing more goes anywhere.
        _ => 0,
    };
    info!("the wizard's scene placed {what:?}: step {:#x}", scene.follow);
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
