//! What the power-ups hang on the hero (`docs/powers.md`, "The looks:
//! where they hang"): every tick the hero's power bits choose one model per
//! slot — the left and right wrists, the head (twice) and the body — and a
//! slot's model is put on, swapped or taken off to match. The wrist and
//! head models are plain model objects on the skeleton's nodes; the body's
//! are atrees with their own actions (the Pojo's follow the hero's). A
//! right-wrist model hides the held weapon; the Pojo hides the hero; the
//! body's model fades out over its power's last second.
//!
//! The x-ray glasses' sight is here too: the nearest container the hero
//! can see into goes see-through, with what it holds shown inside.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::AnimFile;
use gdl_formats::texmod::TexMod;

use gdl_formats::population::{ItemClass, ItemType, PlacementParams};

use crate::actions::Action;
use crate::audio::{CALL_VOLUME, PlaySoundAt};
use crate::billboard::Billboard;
use crate::character::{self, Animator, CharacterData, CharacterModel, clip_end, clip_fps, loop_length};
use crate::combat::Hit;
use crate::fade::Fade;
use crate::items::{ItemTick, LevelItems};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::model_mesh::{self, TextureCache};
use crate::monsters;
use crate::play_camera::PlayCamera;
use crate::player::Player;
use crate::party::Party;
use crate::player_state::{Power, PowerBits, PowersTick, power};
use crate::population::{ContentModels, LevelPopulation, XRAY_KEYS, XRAY_MONSTER, XRAY_SPRITE};
use crate::projectiles::HeroShot;
use crate::world::LevelEntity;

pub struct PowerLooksPlugin;

impl Plugin for PowerLooksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LookModels>()
            .init_resource::<Xray>()
            .add_systems(FixedUpdate, (wear_looks.after(PowersTick), xray.after(PowersTick).after(ItemTick)))
            .add_systems(Update, forget_xray.run_if(resource_exists_and_changed::<LevelPopulation>));
    }
}

const POWERUPS: &str = "POWERUPS";
const WEAPONS: &str = "WEAPONS";

/// Which of the hero's power bits a look answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bits {
    Weapon(u32),
    Armour(u32),
    Special(u32),
}

impl Bits {
    fn on(self, b: &PowerBits) -> bool {
        match self {
            Bits::Weapon(m) => b.weapon & m != 0,
            Bits::Armour(m) => b.armour & m != 0,
            Bits::Special(m) => b.special & m != 0,
        }
    }
}

/// A model a power puts on the hero: its name (a model object, or for the
/// body an atree) and the bank folder it's in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    name: &'static str,
    bank: &'static str,
}

const fn look(name: &'static str, bank: &'static str) -> Look {
    Look { name, bank }
}

/// The left wrist's models, the first whose bits are on hanging there.
const LEFT_WRIST: [(Bits, Look); 4] = [
    (Bits::Special(0x8000), look("BOSSGAUNTL", POWERUPS)),
    (Bits::Armour(0x2_0000), look("RF_SHLD", WEAPONS)),
    (Bits::Armour(0x20_0000), look("FW_SHLD", WEAPONS)),
    (Bits::Armour(0x40_0000), look("L_SHLD", WEAPONS)),
];

/// The right wrist's: Skorne's right gauntlet, the super crossbow, the
/// hammer.
const RIGHT_WRIST: [(Bits, Look); 3] = [
    (Bits::Special(0x4000), look("BOSSGAUNTR", POWERUPS)),
    (Bits::Weapon(0x10_0000), look("SUPERXBOW", WEAPONS)),
    (Bits::Weapon(0x1000_0000), look("HAMMER_HD", WEAPONS)),
];

/// The head's: Skorne's horns and mask, the halo, the gas mask, the x-ray
/// glasses.
const HEAD: [(Bits, Look); 5] = [
    (Bits::Special(0x1000), look("BOSSHORNS", POWERUPS)),
    (Bits::Special(0x2000), look("BOSSMASK", POWERUPS)),
    (Bits::Armour(0x8_0000), look("HEAD_HALO", POWERUPS)),
    (Bits::Armour(0x2000), look("HEAD_GAS", POWERUPS)),
    (Bits::Special(0x2), look("HEAD_XRAY", POWERUPS)),
];

/// The second head slot's: the Hand of Death, the Health Vampire.
const HAND_OF_DEATH: u32 = 0x20_0000;
const HEALTH_VAMPIRE: u32 = 0x40_0000;
const HEAD_2: [Look; 2] = [look("HEAD_HANDOFDEATH", POWERUPS), look("HEAD_HEALTHVAMP", POWERUPS)];

/// Where a body look hangs: on the hero's model (its root), its head node,
/// or the first child of its skeleton's root node (the pelvis).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum On {
    Model,
    Head,
    Pelvis,
}

const POJO: u32 = 0x400;
const FIRE_WALL: u32 = 0x20_0000;
/// Levitation (special `0x1`) lifts the hero's model this far above its
/// feet.
const LEVITATE: u32 = power::LEVITATE;
const LEVITATE_LIFT: f32 = 1.5;

/// The body's atrees, the first that applies riding the hero; the fire
/// wall's only while the hero runs with its shield up.
const BODY: [(Bits, Look, On); 7] = [
    (Bits::Special(POJO), look("POJO", POWERUPS), On::Model),
    (Bits::Armour(FIRE_WALL), look("FW_SHLD_ACTIVE", WEAPONS), On::Model),
    (Bits::Special(0x80), look("PHOENIX", POWERUPS), On::Model),
    (Bits::Special(0x10), look("HEAD_BREATHEF", POWERUPS), On::Head),
    (Bits::Special(0x20), look("HEAD_BREATHEA", POWERUPS), On::Head),
    (Bits::Special(0x40), look("HEAD_BREATHEE", POWERUPS), On::Head),
    (Bits::Special(0x1), look("WINGS", POWERUPS), On::Pelvis),
];

/// The hero's action while the fire wall's flames show.
const SHIELD_RUN: Action = Action(0x16);

/// The powers whose body models fade out in their last second: slots with
/// any of these special bits, or the fire wall.
const FADING_SPECIAL: u32 = 0x70_04F1;
/// A counted power's time, for the fade.
const COUNTED_TIME: f32 = 99_999.0;

/// Each class's wrist nodes (the game's per-class tables): left, right.
/// Classes past the table use the first's.
const WRISTS: [(&str, &str); 16] = [
    ("L_WRIST", "R_WRIST"),
    ("L_WRIST", "R_WRIST"),
    ("L_WRIST", "R_WRIST"),
    ("L_WRIST", "R_WRIST"),
    ("LEFTHAND", "RIGHTHAN"),
    ("LEFTHAND", "RIGHTHAN"),
    ("LEFTHAND", "RIGHTHAN"),
    ("LEFTHAND", "RHEND"),
    ("L_WRIST", "R_WRIST"),
    ("L_WRIST", "R_WRIST"),
    ("L_WRIST", "R_WRIST"),
    ("L_WRIST", "R_WRIST"),
    ("LEFTHAND", "RIGHTHAN"),
    ("LEFTHAND", "RIGHTHAN"),
    ("LEFTHAND", "RIGHTHAN"),
    ("LEFTHAND", "RHEND"),
];
const HEAD_NODE: &str = "HEAD";

/// The class's wrist nodes (by the game's class index; unknown classes
/// get the first class's).
fn wrists(class: Option<usize>) -> (&'static str, &'static str) {
    class.and_then(|c| WRISTS.get(c)).copied().unwrap_or(WRISTS[0])
}

/// The class's left wrist node (`L_WRIST` or `LEFTHAND`).
pub fn left_wrist(class: Option<usize>) -> &'static str {
    wrists(class).0
}

fn first_on<const N: usize>(table: &[(Bits, Look); N], bits: &PowerBits) -> Option<Look> {
    table.iter().find(|(b, _)| b.on(bits)).map(|(_, l)| *l)
}

/// The body's look for these bits while the hero plays `action`.
fn body_look(bits: &PowerBits, action: Action) -> Option<(Look, On)> {
    BODY.iter()
        .find(|(b, l, _)| b.on(bits) && (l.name != "FW_SHLD_ACTIVE" || action == SHIELD_RUN))
        .map(|&(_, l, on)| (l, on))
}

/// The second head slot: the game latches each of its two models when it
/// comes on (`latched`) and only swaps when one comes on afresh, so the
/// model already there stays while a latched power runs; both latches
/// clear with both powers off.
fn head_2(special: u32, latched: &mut [bool; 2], worn: Option<Look>) -> Option<Look> {
    for (k, bit) in [HAND_OF_DEATH, HEALTH_VAMPIRE].into_iter().enumerate() {
        if special & bit != 0 {
            if latched[k] {
                return worn;
            }
            latched[k] = true;
            return Some(HEAD_2[k]);
        }
    }
    *latched = [false; 2];
    None
}

/// The Pojo's action for the hero's action (the game's map: 1 RUN, 3
/// ATTPWR, 4 HIT, 5 DEATH, else 0 READY), 2 (ATTACK) on a peck; and
/// whether it plays through before the next may take over (the game's
/// mode 2).
fn pojo_action(hero: Action, pecked: bool) -> (usize, bool) {
    if pecked {
        return (2, true);
    }
    match hero.0 {
        0x6E => (3, true),
        0x08 | 0x11..=0x14 | 0x16 | 0x19 | 0x1A => (1, false),
        0x1B | 0x7F..=0x83 | 0x85 | 0x87 | 0x94 => (4, true),
        0x7E => (5, true),
        _ => (0, false),
    }
}

/// How faded the body's model is: 1 − the longest fading power's time
/// while that's under a second, else 0.
fn body_fade(powers: &[Power]) -> f32 {
    let longest = powers
        .iter()
        .filter(|p| p.active())
        .filter(|p| {
            (p.subtype == power::SPECIAL && p.value & FADING_SPECIAL != 0)
                || (p.subtype == power::ARMOUR && p.value & FIRE_WALL != 0)
        })
        .map(|p| if p.time < 0.0 { COUNTED_TIME } else { p.time })
        .reduce(f32::max);
    match longest {
        Some(t) if (0.0..1.0).contains(&t) => 1.0 - t,
        _ => 0.0,
    }
}

/// A look's meshes, built on first use.
#[derive(Clone)]
enum LookModel {
    /// A model object, hung on a node.
    Object(Arc<Vec<(Handle<Mesh>, Handle<LevelMaterial>)>>),
    /// An atree, animated on its own.
    Atree(Arc<CharacterModel>),
}

/// The looks' models by name (none: not found).
#[derive(Resource, Default)]
struct LookModels(HashMap<&'static str, Option<LookModel>>);

impl LookModels {
    fn get(
        &mut self,
        look: Look,
        atree: bool,
        game: &mut LoadedGame,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<LookModel> {
        self.0
            .entry(look.name)
            .or_insert_with(|| {
                let built = load(look, atree, game, meshes, materials, images);
                match &built {
                    Some(_) => debug!("power look {} built", look.name),
                    None => warn!("power look {} not found in {}", look.name, look.bank),
                }
                built
            })
            .clone()
    }
}

/// Model object names hold 15 characters (`HEAD_HANDOFDEAT`).
const OBJECT_NAME_LENGTH: usize = 15;

fn load(
    look: Look,
    atree: bool,
    game: &mut LoadedGame,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> Option<LookModel> {
    let folder = look.bank;
    let model = ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?;
    let textures = game.install.read(&format!("{folder}/textures.ngc")).ok()?;
    if !atree {
        let name: String = look.name.chars().take(OBJECT_NAME_LENGTH).collect();
        let object = model.objects.iter().position(|o| o.name == name)?;
        let mut cache = TextureCache::new(&model, &textures);
        let mut bounds = (Vec3::MAX, Vec3::MIN);
        let built =
            model_mesh::build_flagged(&model, &mut cache, [(object, Vec3::ZERO, 0)], meshes, materials, images, &mut bounds);
        return Some(LookModel::Object(Arc::new(built.into_iter().map(|b| (b.mesh, b.material)).collect())));
    }
    let bytes = game.install.read(&format!("{folder}/ANIM.PS2")).ok()?;
    let tree = AnimFile::parse(&bytes).ok()?.atrees.into_iter().find(|a| a.name == look.name)?;
    let texmods = TexMod::parse_all(&bytes).unwrap_or_default();
    let data = CharacterData {
        name: format!("{folder}/{}", look.name),
        class: String::new(),
        colour: String::new(),
        clips: Arc::new(tree.clone()),
        skeleton: tree,
        model,
        textures,
    };
    let mut cache = TextureCache::new(&data.model, &data.textures).sharing_materials();
    let mut built = CharacterModel::build_with(&data, &mut cache, meshes, materials, images);
    built.run_texmods(&data, &texmods, &mut cache, images);
    Some(LookModel::Atree(Arc::new(built)))
}

impl Worn {
    /// The body look's model root, when it's `name`'s (the Pojo's breath
    /// leaves from its node).
    pub(crate) fn body_model(&self, name: &str) -> Option<Entity> {
        self.body.filter(|(look, _)| look.name == name).map(|(_, e)| e)
    }
}

/// Slots of [`Worn::objects`] (left wrist, right wrist, head, head 2).
const RIGHT: usize = 1;
const HEAD_2_SLOT: usize = 3;

/// What the hero has on.
#[derive(Component, Default)]
pub(crate) struct Worn {
    /// Left wrist, right wrist, head, head 2: the look and the entity
    /// holding its meshes.
    objects: [Option<(Look, Entity)>; 4],
    /// The body's look and its model's root.
    body: Option<(Look, Entity)>,
    /// Seconds the body's action has left to play through.
    hold: f32,
    /// The second head slot's latches (Hand of Death, Health Vampire).
    latched: [bool; 2],
    /// The hero's own skeleton is hidden (the Pojo).
    skeleton_hidden: bool,
    /// Its weapon is hidden (a right-wrist model).
    weapon_hidden: bool,
}

/// Seconds `action` of `animator` takes to play once.
fn play_time(animator: &Animator, action: usize) -> f32 {
    animator.clips.actions.get(action).map_or(0.0, |a| {
        let frames = if a.loops() { loop_length(a.frames, a.rate) } else { clip_end(a.frames) };
        frames / clip_fps(a.rate)
    })
}

fn show(visibility: &mut Query<&mut Visibility>, entity: Entity, shown: bool) {
    if let Ok(mut v) = visibility.get_mut(entity) {
        *v = if shown { Visibility::Inherited } else { Visibility::Hidden };
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn wear_looks(
    mut commands: Commands,
    time: Res<Time>,
    party: Res<Party>,
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<LookModels>,
    (mut shots, mut hits): (MessageReader<HeroShot>, MessageReader<Hit>),
    mut heroes: Query<(Entity, &Player, &mut Animator, Option<&mut Worn>)>,
    mut bodies: Query<(&mut Animator, &mut Fade), Without<Player>>,
    mut visibility: Query<&mut Visibility>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<LevelMaterial>>, ResMut<Assets<Image>>),
) {
    let threw: Vec<Entity> = shots.read().map(|s| s.hero).collect();
    let pecked: Vec<Entity> = hits.read().filter(|h| !h.ranged).map(|h| h.attacker).collect();
    for (hero, player, mut animator, worn) in &mut heroes {
        let Some(state) = party.state(player.slot) else { continue };
        let bits = state.bits;
        let class = character::class_index(&state.class);
        let (left_node, right_node) = (left_wrist(class), wrists(class).1);
        // Levitation lifts the hero's model off its feet.
        let lift = if bits.special & LEVITATE != 0 { LEVITATE_LIFT } else { 0.0 };
        if animator.lift != lift {
            animator.lift = lift;
        }
        let animator = &*animator;
        let Some(mut worn) = worn else {
            commands.entity(hero).insert(Worn::default());
            continue;
        };
        let worn = &mut *worn;
        let bone = |name: &str| animator.node(name).and_then(|n| animator.bone(n));

        // The wrists and heads: plain model objects on their nodes.
        let head_2 = head_2(bits.special, &mut worn.latched, worn.objects[HEAD_2_SLOT].map(|(l, _)| l));
        let wanted = [
            (first_on(&LEFT_WRIST, &bits), left_node),
            (first_on(&RIGHT_WRIST, &bits), right_node),
            (first_on(&HEAD, &bits), HEAD_NODE),
            (head_2, HEAD_NODE),
        ];
        for (slot, (want, node)) in wanted.into_iter().enumerate() {
            if worn.objects[slot].map(|(l, _)| l) == want {
                continue;
            }
            if let Some((_, e)) = worn.objects[slot].take() {
                commands.entity(e).despawn();
            }
            let Some(look) = want else { continue };
            let Some(parent) = bone(node) else { continue };
            let Some(LookModel::Object(parts)) =
                models.get(look, false, &mut game, &mut meshes, &mut materials, &mut images)
            else {
                continue;
            };
            let holder = commands.spawn((Transform::default(), Visibility::default(), ChildOf(parent))).id();
            for (mesh, material) in parts.iter() {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), ChildOf(holder)));
            }
            worn.objects[slot] = Some((look, holder));
            debug!("power look {} on {node}", look.name);
        }

        // The body: an atree on the model, the head or the pelvis.
        let want = body_look(&bits, player.actions.action);
        if worn.body.map(|(l, _)| l) != want.map(|(l, _)| l) {
            if let Some((_, e)) = worn.body.take() {
                commands.entity(e).despawn();
            }
            worn.hold = 0.0;
            if let Some((look, on)) = want {
                let parent = match on {
                    On::Model => Some(hero),
                    On::Head => bone(HEAD_NODE),
                    On::Pelvis => {
                        animator.roots().next().and_then(|r| animator.first_child(r)).and_then(|n| animator.bone(n))
                    }
                };
                let model = models.get(look, true, &mut game, &mut meshes, &mut materials, &mut images);
                if let (Some(parent), Some(LookModel::Atree(model))) = (parent, model) {
                    let root = model.spawn(Transform::default(), &mut commands);
                    commands.entity(root).insert((ChildOf(parent), Fade::new(0.0)));
                    worn.body = Some((look, root));
                    debug!("power look {} on the body", look.name);
                }
            }
        }
        if let Some((look, e)) = worn.body
            && let Ok((mut body, mut fade)) = bodies.get_mut(e)
        {
            // Its action: the Pojo's by the hero's; the others' second on
            // a throw, if they have one.
            let (action, through) = if look.name == "POJO" {
                pojo_action(player.actions.action, pecked.contains(&hero))
            } else if threw.contains(&hero) && body.clips.actions.len() > 1 {
                (1, true)
            } else {
                (0, false)
            };
            worn.hold -= time.delta_secs();
            if action != body.action && worn.hold <= 0.0 {
                body.play(action);
                worn.hold = if through { play_time(&body, action) } else { 0.0 };
                debug!("power look {} plays action {action}", look.name);
            }
            fade.amount = body_fade(&state.powers);
        }

        // The Pojo hides the hero's skeleton; a right-wrist model its
        // weapon.
        let pojo = bits.special & POJO != 0;
        if pojo != worn.skeleton_hidden {
            worn.skeleton_hidden = pojo;
            for bone in animator.roots().filter_map(|r| animator.bone(r)) {
                show(&mut visibility, bone, !pojo);
            }
        }
        let armed = worn.objects[RIGHT].is_some();
        if armed != worn.weapon_hidden {
            worn.weapon_hidden = armed;
            for &e in animator.weapon() {
                show(&mut visibility, e, !armed);
            }
        }
    }
}

/// The x-ray glasses (special `0x2`, `docs/powers.md`).
const XRAY: u32 = 0x2;
/// How near a container has to be.
const XRAY_RANGE: f32 = 10.0;
/// The on-screen test's radius, × the container type's first extent.
const XRAY_SCREEN: f32 = 2.0;
/// The container's transparency (192 of 255).
const XRAY_FADE: f32 = 192.0 / 255.0;
/// What's inside is shown at this scale.
const XRAY_SCALE: f32 = 0.65;
const XRAY_SOUND: &str = "S_XRAY";
/// The hero's top point above its feet (`PDAT +0x50`, every class).
const HERO_TOP: f32 = 4.4;
/// The key powerup's subtype.
const KEY: i32 = 2;

/// What the x-ray shows inside a container holding `contents` (`keys`:
/// the container's count): the Death icon for a monster, the key ring for
/// more than one key, else the contents' own model; `None` for anything
/// else, which it doesn't see.
fn xray_model(contents: &ItemType, keys: i16) -> Option<&str> {
    match contents.class {
        ItemClass::EnemyInfo => Some(XRAY_MONSTER),
        ItemClass::Powerup if contents.subtype == KEY && keys > 1 => Some(XRAY_KEYS),
        ItemClass::Powerup => Some(contents.name.as_str()),
        _ => None,
    }
}

/// What the x-ray has on: the container (placement) and its model, the
/// model shown inside, and the holder at the container carrying it and
/// the see-through sprite.
#[derive(Resource, Default)]
struct Xray {
    container: Option<(usize, Option<Entity>)>,
    shown: Option<String>,
    holder: Option<Entity>,
}

/// A container the x-ray could see into: how far, which, its centre and
/// on-screen radius, its model and the model to show inside.
struct Seen<'a> {
    distance: f32,
    placement: usize,
    centre: [f32; 3],
    radius: f32,
    model: Option<Entity>,
    shown: &'a str,
}

/// A new level: what the x-ray had on went with the old one.
fn forget_xray(mut xray: ResMut<Xray>) {
    *xray = Xray::default();
}

/// Each tick, the nearest container within 10 of the hero that is still
/// shut and holds a powerup or a monster, if it's on screen: drawn
/// see-through, with the contents' model inside at 0.65 and the
/// see-through sprite over it; `S_XRAY` when the container or what's shown
/// changes. With nothing found, or the power off, it's put back.
#[allow(clippy::too_many_arguments)]
fn xray(
    mut commands: Commands,
    mut xray: ResMut<Xray>,
    party: Res<Party>,
    items: Res<LevelItems>,
    models: Option<Res<ContentModels>>,
    camera: Option<Res<PlayCamera>>,
    players: Query<&Player>,
    transforms: Query<&Transform>,
    mut fades: Query<&mut Fade>,
    mut sounds: MessageWriter<PlaySoundAt>,
) {
    // The first hero with the power on sees.
    let seer = players.iter().find(|p| party.state(p.slot).is_some_and(|s| s.bits.special & XRAY != 0));
    let on = seer.is_some();
    let hero = seer.map(|p| Vec3::from(p.mover.position));
    // The nearest container it could see into.
    let mut found: Option<Seen> = None;
    if let (true, Some(hero)) = (on, hero) {
        for v in items.views() {
            if v.ty.class != ItemClass::Container || !v.live || v.state != 0 {
                continue;
            }
            let keys = match v.params {
                PlacementParams::Container { param, .. } => *param,
                _ => 0,
            };
            let Some(model) = v.contents.and_then(|c| xray_model(c, keys)) else { continue };
            let distance = hero.distance(Vec3::from(v.shape.centre));
            if distance < found.as_ref().map_or(XRAY_RANGE, |f| f.distance) {
                let (placement, centre, model, shown) = (v.placement, v.shape.centre, v.model, model);
                found = Some(Seen { distance, placement, centre, radius: XRAY_SCREEN * v.ty.extent[0], model, shown });
            }
        }
    }
    let view = monsters::game_view(camera.as_deref());
    let target = found
        .filter(|f| monsters::on_screen(view.as_ref(), f.centre, f.radius))
        .map(|f| (f.placement, f.centre, f.model, f.shown.to_string()));

    let xray = &mut *xray;
    let mut fade = |entity: Option<Entity>, amount: f32, commands: &mut Commands| {
        let Some(e) = entity else { return };
        match fades.get_mut(e) {
            Ok(mut f) => f.amount = amount,
            Err(_) if amount > 0.0 => {
                commands.entity(e).try_insert(Fade::new(amount));
            }
            Err(_) => {}
        }
    };
    let Some((placement, centre, model, shown)) = target else {
        // Nothing to see: everything back as it was.
        if let Some((_, old)) = xray.container.take() {
            fade(old, 0.0, &mut commands);
        }
        if let Some(h) = xray.holder.take() {
            commands.entity(h).try_despawn();
        }
        xray.shown = None;
        return;
    };
    let mut changed = false;
    if xray.container.map(|(p, _)| p) != Some(placement) {
        if let Some((_, old)) = xray.container.take() {
            fade(old, 0.0, &mut commands);
        }
        fade(model, XRAY_FADE, &mut commands);
        xray.container = Some((placement, model));
        changed = true;
    }
    if changed || xray.shown.as_deref() != Some(shown.as_str()) {
        if let Some(h) = xray.holder.take() {
            commands.entity(h).try_despawn();
        }
        // The holder stands where the container's model does.
        let at = model.and_then(|m| transforms.get(m).ok()).copied().unwrap_or(Transform::from_translation(Vec3::from(centre)));
        let holder = commands.spawn((at, Visibility::default(), LevelEntity)).id();
        if let Some(models) = models.as_deref() {
            let inside = models.spawn_still(&shown, Transform::from_scale(Vec3::splat(XRAY_SCALE)), &mut commands);
            let sprite = models.spawn_still(XRAY_SPRITE, Transform::default(), &mut commands);
            for e in inside.into_iter().chain(sprite) {
                commands.entity(e).insert(ChildOf(holder));
            }
            if let Some(e) = sprite {
                commands.entity(e).insert(Billboard::Sprite);
            }
            if inside.is_none() {
                debug!("x-ray: no model {shown} to show in container {placement}");
            }
        }
        xray.holder = Some(holder);
        xray.shown = Some(shown);
        changed = true;
    }
    if changed {
        // At the hero's top point, panned only (`docs/audio-format.md`).
        if let Some(hero) = hero {
            sounds.write(PlaySoundAt::panned(XRAY_SOUND, hero + Vec3::Y * HERO_TOP, CALL_VOLUME));
        }
        debug!("x-ray: container {placement} shows {:?}", xray.shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(weapon: u32, armour: u32, special: u32) -> PowerBits {
        PowerBits { weapon, armour, special, ..default() }
    }

    #[test]
    fn first_that_applies() {
        let b = bits(0x10_0000, 0x22_0000, 0x8000);
        assert_eq!(first_on(&LEFT_WRIST, &b).unwrap().name, "BOSSGAUNTL");
        assert_eq!(first_on(&LEFT_WRIST, &bits(0, 0x60_0000, 0)).unwrap().name, "FW_SHLD");
        assert_eq!(first_on(&RIGHT_WRIST, &b).unwrap().name, "SUPERXBOW");
        assert_eq!(first_on(&HEAD, &bits(0, 0x8_2000, 0x2)).unwrap().name, "HEAD_HALO");
        assert_eq!(first_on(&HEAD, &bits(0, 0, 0)), None);
    }

    #[test]
    fn fire_wall_flames_only_on_the_shield_run() {
        let b = bits(0, FIRE_WALL, 0x1);
        assert_eq!(body_look(&b, Action(0x13)).unwrap().0.name, "WINGS");
        assert_eq!(body_look(&b, SHIELD_RUN).unwrap().0.name, "FW_SHLD_ACTIVE");
        assert_eq!(body_look(&bits(0, FIRE_WALL, POJO), SHIELD_RUN).unwrap().0.name, "POJO");
        assert_eq!(body_look(&bits(0, 0, 0x20), Action(0)).unwrap(), (look("HEAD_BREATHEA", POWERUPS), On::Head));
    }

    #[test]
    fn head_2_latches() {
        let mut latched = [false; 2];
        let vampire = head_2(HEALTH_VAMPIRE, &mut latched, None);
        assert_eq!(vampire, Some(HEAD_2[1]));
        // The Hand of Death comes on afresh: it takes the slot.
        let hand = head_2(HAND_OF_DEATH | HEALTH_VAMPIRE, &mut latched, vampire);
        assert_eq!(hand, Some(HEAD_2[0]));
        // It ends; the vampire is still latched, so the hand's model stays.
        assert_eq!(head_2(HEALTH_VAMPIRE, &mut latched, hand), hand);
        assert_eq!(head_2(0, &mut latched, hand), None);
        assert_eq!(latched, [false; 2]);
    }

    #[test]
    fn pojo_follows_the_hero() {
        assert_eq!(pojo_action(Action(0x13), false), (1, false));
        assert_eq!(pojo_action(Action(0x6E), false), (3, true));
        assert_eq!(pojo_action(Action(0x84), false), (0, false));
        assert_eq!(pojo_action(Action(0x85), false), (4, true));
        assert_eq!(pojo_action(Action(0x7E), false), (5, true));
        assert_eq!(pojo_action(Action(0x13), true), (2, true));
    }

    #[test]
    fn what_the_xray_shows() {
        let ty = |class, subtype, name: &str| ItemType {
            class,
            subtype,
            name: name.to_string(),
            choices: Vec::new(),
            extent: [1.0; 4],
            center_offset: [0.0; 3],
            value: 0,
            amount: 0,
            armor: 0,
            hit_points: 0,
            flags: 0,
            duration: 0,
            raw: [0; 0x50],
        };
        let key = ty(ItemClass::Powerup, KEY, "KEY");
        assert_eq!(xray_model(&key, 1), Some("KEY"));
        assert_eq!(xray_model(&key, 3), Some(XRAY_KEYS));
        assert_eq!(xray_model(&ty(ItemClass::Powerup, 3, "APPLE"), 2), Some("APPLE"));
        assert_eq!(xray_model(&ty(ItemClass::EnemyInfo, 0, "DEATH"), 0), Some(XRAY_MONSTER));
        assert_eq!(xray_model(&ty(ItemClass::Container, 0, "BAROBJ"), 0), None);
    }

    #[test]
    fn fades_in_the_last_second() {
        let p = |subtype, value, time| Power { subtype, value, amount: 0.0, time, state: crate::player_state::SlotState::On };
        assert_eq!(body_fade(&[p(power::SPECIAL, 0x1, 0.25)]), 0.75);
        assert_eq!(body_fade(&[p(power::SPECIAL, 0x1, 0.25), p(power::SPECIAL, 0x10, -1.0)]), 0.0);
        assert_eq!(body_fade(&[p(power::ARMOUR, FIRE_WALL, 5.0)]), 0.0);
        assert_eq!(body_fade(&[p(power::SPECIAL, 0x2, 0.5)]), 0.0);
    }
}
