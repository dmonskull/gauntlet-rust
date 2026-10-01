//! The tower's own figures (`docs/items.md`, "The tower's wizard" and
//! "The tower's shards and runes"). As every tower level loads, the game
//! makes the idle wizard — `GWIZ`, from the tower's items bank — and stands
//! him on the lookout whose parameter is 0: the pedestal in front of the
//! heroes' start on `levelL1`. He goes through his first three actions in
//! turn, each played to its end. It also sets out what the heroes have
//! won: each shard (`SHARD<n>`) in the window frame over the door, each
//! runestone (`RUNE<n>`) in its place, the thirteenth apart — the effects
//! wound on to their last frames, their particles stopped. (The game sets
//! out only the pieces its wizard has announced, and announces new ones
//! in a scene that plays their effects in full; stand-in: without the
//! scenes, new pieces are set out with the rest.)

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::AnimFile;
use gdl_formats::population::LocatorKind;
use gdl_formats::texmod::TexMod;

use crate::message_box::{MessageBox, ShowMessage};
use crate::play_camera::{PlayCamera, StartCut};

use crate::character::{self, Animate, Animator, CharacterData, CharacterModel};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::mechanics::LevelNodes;
use crate::model_mesh::TextureCache;
use crate::party::Party;
use crate::population::{self, LevelPopulation};
use crate::projectiles;
use crate::quest;
use crate::tower_scenes;
use crate::world::LevelEntity;

pub struct TowerPlugin;

impl Plugin for TowerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                follow_levels.run_if(resource_exists_and_changed::<LevelPopulation>),
                place_wizard.run_if(resource_exists_and_changed::<LevelPopulation>),
                place_trophies
                    .run_if(resource_exists_and_changed::<LevelPopulation>)
                    .after(quest::seed_tests)
                    .after(follow_levels),
                arm_speeches
                    .run_if(resource_exists_and_changed::<LevelPopulation>)
                    .after(quest::seed_tests)
                    .after(follow_levels),
                speeches,
                wind_on.before(Animate),
                idle_wizard,
            ),
        )
        .init_resource::<TowerSpeeches>()
        .init_resource::<LevelTrail>();
    }
}

/// The last level the heroes finished (`r13-0x724c`/`r13-0x7250`): set as
/// a level ends with a hero still in it (`exits.rs`), forgotten as any
/// level but the tower starts. The tower's speeches and scenes depend on
/// it — where the heroes came back from, and not after dying or quitting.
#[derive(Resource, Default)]
pub struct LevelTrail {
    pub finished: Option<String>,
}

fn follow_levels(population: Res<LevelPopulation>, mut trail: ResMut<LevelTrail>) {
    if quest::level_of(&population.level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        trail.finished = None;
    }
}

/// The tower's realm.
const TOWER_REALM: u32 = 13;
/// His atree, in the tower's items bank.
const WIZARD_BANK: &str = "ITEMS/levelL";
const WIZARD_ATREE: &str = "GWIZ";
/// The lookout he stands on.
const WIZARD_LOOKOUT: u8 = 0;
/// He cycles through this many of his actions.
const IDLE_ACTIONS: usize = 3;

/// The idle wizard: the action he goes to next, and where his clip was a
/// frame ago (a looping clip is over when it starts round again).
#[derive(Component)]
struct TowerWizard {
    next: usize,
    last_frame: f32,
}

fn place_wizard(
    mut commands: Commands,
    population: Res<LevelPopulation>,
    mut game: ResMut<LoadedGame>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    if crate::quest::level_of(&population.level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        return;
    }
    let lookout = population
        .population
        .locators
        .iter()
        .find(|l| matches!(l.kind, LocatorKind::Lookout(_)) && l.param == WIZARD_LOOKOUT);
    let Some(lookout) = lookout else {
        warn!("{}: no lookout {WIZARD_LOOKOUT} for the wizard", population.level);
        return;
    };
    let Some(data) = projectiles::load_atree(&mut game, WIZARD_BANK, WIZARD_ATREE) else {
        warn!("{WIZARD_BANK}/{WIZARD_ATREE} didn't load");
        return;
    };
    let model = CharacterModel::build(&data, &mut meshes, &mut materials, &mut images);
    let transform = population::lookout_transform(lookout);
    let root = model.spawn(transform, &mut commands);
    commands.entity(root).insert((TowerWizard { next: 1, last_frame: 0.0 }, LevelEntity));
    info!("the wizard stands at {:?}", lookout.position);
}

/// Where the shards and runestones go: world nodes of `levelL1`.
const SHARD_PLACE: &str = "L1WINDOWFRAME";
const RUNE_PLACE: &str = "L1RUNEPLACE";
const RUNE13_PLACE: &str = "L1RUNE13";

/// A shard or runestone set out in the tower, to be wound on to its last
/// frame.
#[derive(Component)]
struct WindOn;

/// A shard's effect (1–8) and where it goes.
pub fn shard_piece(n: u8) -> (String, &'static str) {
    (format!("SHARD{n}"), SHARD_PLACE)
}

/// A runestone's effect (0–12) and where it goes.
pub fn rune_piece(stone: u8) -> (String, &'static str) {
    if stone == 12 { ("RUNE13".into(), RUNE13_PLACE) } else { (format!("RUNE{}", stone + 1), RUNE_PLACE) }
}

/// Where a world node of the level stands.
pub fn node_at(nodes: &LevelNodes, name: &str) -> Option<Vec3> {
    nodes.nodes.iter().position(|n| n.name == name).map(|i| Vec3::from(nodes.origin[i]))
}

/// As the tower loads: sets out the shards (bits 1–8 of
/// [`quest::boss_marks`]) and runestones (a bit each, 0–12) the wizard has
/// announced, wound on; then checks the hero's rank, marks everything held
/// as announced, and readies the wizard's scene for the rank and the
/// first piece that wasn't (`tower_scenes.rs`). The window's light shines
/// if all eight shards were announced before.
#[allow(clippy::too_many_arguments)]
fn place_trophies(
    mut commands: Commands,
    population: Res<LevelPopulation>,
    nodes: Option<Res<LevelNodes>>,
    mut party: ResMut<Party>,
    (mut trail, mut scene, mut light): (ResMut<LevelTrail>, ResMut<tower_scenes::Scene>, ResMut<quest::ShardLight>),
    mut game: ResMut<LoadedGame>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    *scene = tower_scenes::Scene::default();
    if quest::level_of(&population.level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        return;
    }
    let Some(nodes) = nodes else { return };
    if party.is_empty() {
        return;
    }
    // The tower shows what the party has: each player's shards and stones
    // put together (the game's tower load runs over every hero).
    let union = |f: fn(&crate::player_state::PlayerState) -> u32| party.states().fold(0, |bits, (_, s)| bits | f(s));
    let (shards, runes) = (union(|s| s.quest.shards_announced), union(|s| s.quest.runes_announced));
    light.0 = shards & quest::ALL_SHARDS == quest::ALL_SHARDS;
    let (marks, held) = (quest::boss_marks(union(|s| s.realms_beaten)), union(|s| s.runestone_bits()));
    let mut rank = None;
    for (_, member) in party.members_mut() {
        let (keep, new_rank) = tower_scenes::check_rank(&member.state, Some(&member.choice));
        if let Some(level) = keep {
            member.state.quest.rank_level = Some(level);
        }
        rank = rank.or(new_rank);
    }
    let announce = tower_scenes::announcement(marks, shards, held, runes, trail.finished.as_deref());
    // The thirteenth's absence is said once: the game forgets the
    // battlefield as it says it.
    if matches!(announce, Some(tower_scenes::Announce::Rune13No)) {
        trail.finished = None;
    }
    for (_, state) in party.states_mut() {
        state.quest.shards_announced |= marks;
        state.quest.runes_announced |= held;
    }
    let mut wanted: Vec<(String, &str)> = (1..=8u8).filter(|n| shards & (1 << n) != 0).map(shard_piece).collect();
    wanted.extend((0..13u8).filter(|n| runes & (1 << n) != 0).map(rune_piece));
    let piece = match announce {
        Some(tower_scenes::Announce::Shard(n)) => Some(shard_piece(n).0),
        Some(tower_scenes::Announce::Rune(i)) => Some(rune_piece(i).0),
        _ => None,
    };
    // The scenes' wizard is always ready: a hero can reach a new rank in
    // the tower.
    let mut names: Vec<&str> = wanted.iter().map(|(n, _)| n.as_str()).collect();
    names.push(SCENE_WIZARD);
    names.extend(piece.as_deref());
    let mut built = build_models(&mut game, &names, &mut meshes, &mut materials, &mut images);
    for (name, place) in wanted {
        let (Some(at), Some((model, _))) = (node_at(&nodes, place), built.get(&name)) else {
            warn!("{}: no {place} or {name} to set out", population.level);
            continue;
        };
        let root = model.spawn(Transform::from_translation(at), &mut commands);
        commands.entity(root).insert((WindOn, LevelEntity));
        info!("{name} set out at {place} {at:?}");
    }
    let Some((wizard, _)) = built.remove(SCENE_WIZARD) else {
        warn!("{WIZARD_BANK} has no {SCENE_WIZARD}: no scenes");
        return;
    };
    let wizard = tower_scenes::apparition(wizard, &mut materials);
    let piece = piece.and_then(|p| built.remove(&p)).map(|(m, particles)| (Arc::new(m), particles));
    if rank.is_some() || announce.is_some() {
        info!("the wizard will announce {rank:?} {announce:?}");
    }
    scene.arm(wizard, rank, announce, piece);
}

/// The wizard of the scenes: `WIZARD`, beside `GWIZ` in the tower's bank.
const SCENE_WIZARD: &str = "WIZARD";

/// Builds these atrees of the tower's items bank (reading it once), each
/// running its texture modifiers, with their particle systems.
pub fn build_models(
    game: &mut LoadedGame,
    names: &[&str],
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> HashMap<String, (CharacterModel, crate::effects::ParticleSystems)> {
    let mut out = HashMap::new();
    let Some((anim, texmods, model, textures)) = read_bank(game, WIZARD_BANK) else {
        warn!("{WIZARD_BANK} didn't load");
        return out;
    };
    let Some(first) = anim.atrees.first().cloned() else { return out };
    // One set of files and one texture cache for them all.
    let mut data = CharacterData {
        name: String::new(),
        class: String::new(),
        colour: String::new(),
        clips: Arc::new(first.clone()),
        skeleton: first,
        model,
        textures,
    };
    let mut cache = TextureCache::new(&data.model, &data.textures).sharing_materials();
    for &name in names {
        let Some(tree) = anim.atrees.iter().find(|a| a.name == name) else {
            warn!("{WIZARD_BANK} has no {name}");
            continue;
        };
        data.name = format!("{WIZARD_BANK}/{name}");
        data.clips = Arc::new(tree.clone());
        data.skeleton = tree.clone();
        let mut model = CharacterModel::build_with(&data, &mut cache, meshes, materials, images);
        model.run_texmods(&data, &texmods, &mut cache, images);
        let particles = crate::effects::particle_systems(&data, &mut cache, images, materials);
        out.insert(name.to_string(), (model, particles));
    }
    out
}

/// A bank's animation file (its atrees and texture modifiers), model and
/// textures.
fn read_bank(game: &mut LoadedGame, folder: &str) -> Option<(AnimFile, Vec<TexMod>, ModelFile, Vec<u8>)> {
    let bytes = game.install.read(&format!("{folder}/ANIM.PS2")).ok()?;
    let anim = AnimFile::parse(&bytes).ok()?;
    let texmods = TexMod::parse_all(&bytes).unwrap_or_default();
    let model = ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?;
    let textures = game.install.read(&format!("{folder}/textures.ngc")).ok()?;
    Some((anim, texmods, model, textures))
}

/// Winds a shard or runestone on to its action's end, as the game does
/// when it sets one out (start time back by the clip's frames at 1/30 s,
/// frame at the count): it holds its last frame.
fn wind_on(mut commands: Commands, mut animators: Query<(Entity, &mut Animator), With<WindOn>>) {
    for (e, mut a) in &mut animators {
        let frames = a.clips.actions.get(a.action).map_or(0, |x| x.frames);
        a.frame = character::clip_end(frames);
        commands.entity(e).remove::<WindOn>();
    }
}

/// The tower's speeches in the message box (`docs/items.md`): a new hero's
/// welcome (`WELCOMEMESSAGE`, five pages) and, back from Skorne's first
/// lair (`levelF2`), `GARMMESSAGE`. They show once the arrival's opening
/// shot is over; after the welcome the wizard points the way (GESTRIGHT)
/// under a cut to the tower's camera point 0xC6 held 50 × 6 fields, no
/// delay.
#[derive(Resource, Default)]
struct TowerSpeeches {
    pending: Vec<&'static str>,
    /// After the welcome's box: the wizard's gesture and its cut.
    gesture: bool,
    /// Tower loads this session.
    tower_loads: u32,
}

const WELCOME: &str = "WELCOMEMESSAGE";
const GARM: &str = "GARMMESSAGE";
/// Where the Garm speech follows from.
const SKORNE_LAIR: &str = "levelF2";
/// The welcome's camera point (a kind-9 locator's index) and its hold.
const WELCOME_CAMERA: i16 = 0xC6;
const WELCOME_HOLD: f32 = 50.0 * 6.0;
/// His gesture after the welcome.
const GESTURE: usize = 6;

/// On each level's arrival: in the tower, the speeches it owes. The game
/// welcomes a hero on the session's first tower load when no hero in the
/// game has any of its 16 records at player `+0xA90` above 0
/// — here, when the hero hasn't finished a realm's level.
fn arm_speeches(
    population: Res<LevelPopulation>,
    party: Res<Party>,
    trail: Res<LevelTrail>,
    mut speeches: ResMut<TowerSpeeches>,
) {
    let finished = trail.finished.clone();
    if quest::level_of(&population.level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        return;
    }
    let first = speeches.tower_loads == 0;
    speeches.tower_loads += 1;
    let fresh = !party.is_empty()
        && party.states().all(|(_, s)| {
            s.quest.finished.iter().enumerate().all(|(realm, &levels)| realm == TOWER_REALM as usize || levels == 0)
        });
    if first && fresh {
        speeches.pending.push(WELCOME);
    }
    if finished.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(SKORNE_LAIR)) {
        speeches.pending.push(GARM);
    }
}

fn speeches(
    mut speeches: ResMut<TowerSpeeches>,
    camera: Option<Res<PlayCamera>>,
    boxes: Res<MessageBox>,
    population: Option<Res<LevelPopulation>>,
    mut messages: MessageWriter<ShowMessage>,
    mut cuts: MessageWriter<StartCut>,
    mut wizards: Query<(&mut TowerWizard, &mut Animator)>,
) {
    if speeches.gesture && !boxes.is_open() {
        speeches.gesture = false;
        for (mut w, mut animator) in &mut wizards {
            animator.play(GESTURE);
            w.next = 0;
            w.last_frame = 0.0;
        }
        let point = population.as_ref().and_then(|p| {
            p.population.locators.iter().position(|l| l.kind == LocatorKind::Transmitter(9) && l.index == WELCOME_CAMERA)
        });
        match point {
            Some(locator) => {
                cuts.write(StartCut { locator, node: None, hold: Some(WELCOME_HOLD), delay: 0.0, extra: 0.0 });
            }
            None => warn!("the tower has no camera point {WELCOME_CAMERA:#x}"),
        }
    }
    if speeches.pending.is_empty() || boxes.is_open() || !camera.is_some_and(|c| c.settled()) {
        return;
    }
    for group in std::mem::take(&mut speeches.pending) {
        messages.write(ShowMessage::all(group));
        speeches.gesture |= group == WELCOME;
    }
}

/// Each of his actions plays out, then the next (0, 1, 2, 0…).
fn idle_wizard(mut wizards: Query<(&mut TowerWizard, &mut Animator)>) {
    for (mut w, mut animator) in &mut wizards {
        let looped = animator.frame < w.last_frame;
        w.last_frame = animator.frame;
        if animator.finished() || looped {
            let count = animator.clips.actions.len().clamp(1, IDLE_ACTIONS);
            // Past his third he goes back to the first (after a gesture).
            let next = if w.next >= count { 0 } else { w.next };
            animator.play(next);
            w.next = next + 1;
            w.last_frame = 0.0;
        }
    }
}
