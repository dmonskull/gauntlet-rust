//! What the power-ups hang on the hero (`docs/powers.md`, "The looks:
//! where they hang"): every tick the hero's power bits choose one model per
//! slot — the left and right wrists, the head (twice) and the body — and a
//! slot's model is put on, swapped or taken off to match. The wrist and
//! head models are plain model objects on the skeleton's nodes; the body's
//! are atrees with their own actions (the Pojo's follow the hero's). A
//! right-wrist model hides the held weapon; the Pojo hides the hero; the
//! body's model fades out over its power's last second.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::AnimFile;
use gdl_formats::texmod::TexMod;

use crate::actions::Action;
use crate::character::{self, Animator, CharacterData, CharacterModel, clip_end, clip_fps, loop_length};
use crate::combat::Hit;
use crate::fade::Fade;
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::model_mesh::{self, TextureCache};
use crate::player::Player;
use crate::player_state::{PlayerState, Power, PowerBits, PowersTick, power};
use crate::projectiles::HeroShot;

pub struct PowerLooksPlugin;

impl Plugin for PowerLooksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LookModels>().add_systems(FixedUpdate, wear_looks.after(PowersTick));
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

/// Slots of [`Worn::objects`] (left wrist, right wrist, head, head 2).
const RIGHT: usize = 1;
const HEAD_2_SLOT: usize = 3;

/// What the hero has on.
#[derive(Component, Default)]
struct Worn {
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
    state: Option<Res<PlayerState>>,
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<LookModels>,
    (mut shots, mut hits): (MessageReader<HeroShot>, MessageReader<Hit>),
    mut heroes: Query<(Entity, &Player, &Animator, Option<&mut Worn>)>,
    mut bodies: Query<(&mut Animator, &mut Fade), Without<Player>>,
    mut visibility: Query<&mut Visibility>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<LevelMaterial>>, ResMut<Assets<Image>>),
) {
    let threw: Vec<Entity> = shots.read().map(|s| s.hero).collect();
    let pecked: Vec<Entity> = hits.read().filter(|h| !h.ranged).map(|h| h.attacker).collect();
    let Some(state) = state else { return };
    let bits = state.bits;
    let (left_node, right_node) = character::class_index(&state.class)
        .and_then(|c| WRISTS.get(c))
        .copied()
        .unwrap_or(WRISTS[0]);
    for (hero, player, animator, worn) in &mut heroes {
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
    fn fades_in_the_last_second() {
        let p = |subtype, value, time| Power { subtype, value, amount: 0.0, time };
        assert_eq!(body_fade(&[p(power::SPECIAL, 0x1, 0.25)]), 0.75);
        assert_eq!(body_fade(&[p(power::SPECIAL, 0x1, 0.25), p(power::SPECIAL, 0x10, -1.0)]), 0.0);
        assert_eq!(body_fade(&[p(power::ARMOUR, FIRE_WALL, 5.0)]), 0.0);
        assert_eq!(body_fade(&[p(power::SPECIAL, 0x2, 0.5)]), 0.0);
    }
}
