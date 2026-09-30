//! Magic: the hero's potions and the effects they make (`docs/effects.md`).
//!
//! Tapping magic with a potion makes a blast around the hero that grows
//! over the effect's life and hurts every monster, object, generator and
//! breakable it reaches once, less the further out it gets; tapping twice
//! makes a shield that hurts whatever touches it; holding magic winds up
//! and throws the potion, which bursts where it lands. A potion's colour
//! is its element; the one matching the player's slot hits harder and
//! further. Damage goes out as [`Hit`] messages like any blow.
//!
//! The controls side (the magic intents, the double tap, the wind-up) is
//! [`MagicState`], stepped by `player.rs`; the actions chain in
//! `actions.rs`. `GDL_POTIONS=<n>[,<kind>]` hands the hero potions at each
//! level start.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::AnimFile;
use gdl_formats::pdata::PlayerStats;
use gdl_formats::texmod::TexMod;

use crate::audio::PlaySound;
use crate::character::{CharacterData, CharacterModel, clip_fps};
use crate::deaths;
use crate::combat::{self, Hit, TargetKind, Targetable, button};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::model_mesh::TextureCache;
use crate::monsters::{Monster, MonsterTick};
use crate::particles;
use crate::player::{Player, PlayerChoice};
use crate::damage::after_armor;
use crate::player_state::{DamagePlayer, PlayerState};
use crate::population::LevelPopulation;
use crate::projectiles;
use crate::world::{LevelEntity, LevelGround};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<UsePotion>()
            .add_message::<StrikePotion>()
            .add_message::<BlastAt>()
            .add_message::<EffectAt>()
            .add_message::<ExplosionAt>()
            .add_message::<NextStage>()
            .init_resource::<EffectModels>()
            .init_resource::<PotionCycle>()
            .add_systems(
                FixedUpdate,
                (use_potions, set_off_potions, spawn_blasts, spawn_explosions, tick_blasts, spawn_one_shots, tick_one_shots)
                    .chain()
                    .after(MonsterTick),
            )
            .add_systems(Update, (setup_level.run_if(resource_exists_and_changed::<LevelPopulation>), follow_blasts));
    }
}

/// The magic buttons.
pub const MAGIC_BUTTONS: u32 = button::MAGIC | button::MAGIC_SHIELD | button::THROW_MAGIC;

/// What the magic buttons ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagicIntent {
    /// MAGIC: a blast (tap), a shield (tap twice), a throw (hold).
    Magic,
    /// THROW_MAGIC: throw the potion (not on any GameCube scheme's buttons).
    Throw,
    /// MAGIC_SHIELD: the shield (likewise unmapped).
    Shield,
}

/// The magic half of the player record's control flags (`+0x956`) and the
/// throw's wind-up (`+0x958`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MagicState {
    pub flags: u16,
    /// Video fields the magic button was held into the throw.
    pub charge: f32,
}

impl MagicState {
    /// Asked for the shield (a second tap, or the shield button).
    pub const SHIELD: u16 = 2;
    /// Magic let go during MAGICS: it becomes a blast, not a throw.
    pub const RELEASED: u16 = 4;
    /// Magic let go during THROWPOTIONS: the wind-up stops.
    pub const THROW_RELEASED: u16 = 8;
    /// A potion was just used: magic does nothing until it's let go.
    pub const USED: u16 = 0x80;

    /// The magic intent for this tick's buttons (None while the last use
    /// is still held), then what it means for the magic action playing.
    pub fn observe(&mut self, held: u32, playing: u8, fields: f32) -> Option<MagicIntent> {
        let magic = held & MAGIC_BUTTONS;
        if magic == 0 && self.flags == Self::USED {
            self.flags = 0;
        }
        let intent = if magic == 0 || self.flags & Self::USED != 0 {
            None
        } else if held & button::THROW_MAGIC != 0 {
            Some(MagicIntent::Throw)
        } else if held & button::MAGIC_SHIELD != 0 {
            Some(MagicIntent::Shield)
        } else {
            Some(MagicIntent::Magic)
        };
        match playing {
            // MAGICS: let go, then pressed again: the shield.
            0x73 => {
                if intent == Some(MagicIntent::Magic) {
                    if self.flags & Self::RELEASED != 0 {
                        self.flags |= Self::SHIELD;
                    }
                } else {
                    self.flags |= Self::RELEASED;
                }
            }
            // THROWPOTIONS: winds up while magic is held.
            0x75 => {
                if matches!(intent, Some(MagicIntent::Magic | MagicIntent::Throw)) {
                    if self.flags & Self::THROW_RELEASED == 0 {
                        self.charge += fields;
                    }
                } else {
                    self.flags |= Self::THROW_RELEASED;
                }
            }
            _ => {}
        }
        intent
    }
}

/// A hero used a potion (`player.rs`, as MAGICR or THROWPOTIONR starts):
/// mode 0 blast, 1 shield, 2 and 3 thrown.
#[derive(Message, Clone, Copy, Debug)]
pub struct UsePotion {
    pub hero: Entity,
    pub feet: Vec3,
    pub facing: f32,
    pub mode: u8,
    /// Video fields the throw was wound up.
    pub charge: f32,
}

/// A potion lying on the floor struck by a hero's missile or blast (`by`
/// the hero), or reached by another potion's blast (by nobody).
#[derive(Message, Clone, Copy, Debug)]
pub struct StrikePotion {
    pub placement: usize,
    pub by: Option<Entity>,
}

/// A potion lying on the floor, which blows and blasts set off.
pub fn is_floor_potion(v: &crate::items::ItemView) -> bool {
    v.live && v.ty.class == gdl_formats::population::ItemClass::Powerup && v.ty.subtype == POTION
}

/// The powerup subtype of potions.
const POTION: i32 = 4;
/// A potion set off by a blow: its magic at 0.8 (`r2-0x67ec`), for nobody
/// — 40 × 0.8 damage out to 20 × 0.8 — and, struck by a hero, the hero's
/// own blast of it at 0.8 of their magic power.
const STRUCK_POWER: f32 = 0.8;
const NOBODYS_DAMAGE: f32 = 40.0 * STRUCK_POWER;
const NOBODYS_RADIUS: f32 = 20.0 * STRUCK_POWER;

/// A thrown potion burst (`projectiles.rs`).
#[derive(Message, Clone, Copy, Debug)]
pub struct BlastAt {
    pub owner: Entity,
    pub at: Vec3,
    pub kind: u32,
    pub damage: f32,
    pub radius: f32,
}

/// What a thrown potion becomes where it lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PotionBurst {
    pub kind: u32,
    pub damage: f32,
    pub radius: f32,
}

/// A potion's colour, from its kind's low four bits: 1 red (fire), 2 blue
/// (lightning), 3 yellow (light), 4 green (acid).
fn colour_index(kind: u32) -> usize {
    (kind & 0xF).clamp(1, 4) as usize - 1
}

/// The blast, shield and thrown-potion effects by colour, and their sounds
/// (the effect table's names; the sound ids' catalog names).
const BLAST_FX: [&str; 4] = ["MP_FIRE", "MP_ELEC", "MP_LIGHT", "MP_ACID"];
const SHIELD_FX: [&str; 4] = ["MS_FIRE", "MS_ELEC", "MS_LIGHT", "MS_ACID"];
const THROWN_FX: [&str; 4] = ["POT_RED_TW", "POT_BLU_TW", "POT_YEL_TW", "POT_GRE_TW"];
const POTION_SOUND: [&str; 4] = ["S_POTION2", "S_POTION1", "S_POTION3", "S_POTION4"];
const SHIELD_SOUND: [&str; 4] = ["S_SHIELD2", "S_SHIELD1", "S_SHIELD3", "S_SHIELD4"];
/// The light each effect gives (not drawn yet: the stand-in sphere takes
/// its colour): red, white, yellow, green.
const LIGHT: [[f32; 3]; 4] = [[2.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]];

/// The player slot whose potion colour it is (kind 3 player 1, 2 player 2,
/// 1 player 3, 4 player 4, counting from 0): that player's potions hit
/// harder and further.
pub fn favourite_player(kind: u32) -> Option<usize> {
    match kind & 0xF {
        3 => Some(0),
        2 => Some(1),
        1 => Some(2),
        4 => Some(3),
        _ => None,
    }
}

/// The hero's magic power (player `+0x10C`): 8–32 from the magic stat.
pub fn magic_power(stat: f32) -> f32 {
    8.0 + 0.001 * stat * 24.0
}

/// The magic power with what magic powers add, clamped to its range as
/// the game's stats routine does.
fn powered_magic(stat: f32, state: Option<&PlayerState>) -> f32 {
    (magic_power(stat) + state.map_or(0.0, |s| s.bits.magic)).clamp(magic_power(0.0), magic_power(1000.0))
}

/// A potion's effect: damage and radius, before it's spawned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PotionEffect {
    pub kind: u32,
    pub damage: f32,
    pub radius: f32,
}

/// The game's numbers for using a potion of `kind` (1–4) in `mode` by the
/// player in `slot` with magic power `power`: the blast deals 40 out to
/// the power; the shield 25 out to a quarter of it; the thrown potion's
/// burst 40 out to three quarters. The player's own colour: damage + 10%,
/// radius × 1.1.
pub fn potion_effect(kind: u32, mode: u8, slot: usize, power: f32, level: u32) -> PotionEffect {
    let (mut damage_scale, mut radius) = (1.0, power);
    if favourite_player(kind) == Some(slot) {
        radius *= 1.1;
        damage_scale += 0.1;
    }
    let mut kind = kind | 0x200;
    if level > 24 {
        kind |= 0x80_0000;
    }
    let (damage, radius) = match mode {
        1 => (25.0 * damage_scale, 0.25 * radius),
        2 | 3 => (40.0 * damage_scale, 0.75 * radius),
        _ => (40.0 * damage_scale, radius),
    };
    PotionEffect { kind, damage, radius }
}

/// How long an effect lasts: its clip's frames at the clip's frame rate
/// (rate / 900 s a frame; rate 0 plays at 30).
pub fn effect_life(frames: u16, rate: u16) -> f32 {
    f32::from(frames) / clip_fps(rate)
}

/// A growing blast's reach and damage share at `left` of `life` seconds
/// remaining: from a third of the radius (full damage × 1.005) out to all of
/// it (nothing), over the first two thirds of its life.
pub fn blast_front(left: f32, life: f32, radius: f32) -> Option<(f32, f32)> {
    let f = if left > life {
        return None;
    } else if life > 1.0 / 30.0 {
        left / life
    } else {
        1.0
    };
    (f > 0.33).then_some((radius * (0.33 + 1.0 - f), 1.5 * (f - 0.33)))
}

/// Blast, shield or burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlastShape {
    /// Grows from the point it went off.
    Grow,
    /// The shield: a steady radius around its hero.
    Aura(Entity),
}

/// A magic effect or explosion doing damage.
#[derive(Component)]
struct Blast {
    owner: Entity,
    shape: BlastShape,
    centre: Vec3,
    kind: u32,
    damage: f32,
    radius: f32,
    life: f32,
    age: f32,
    /// Until when each target is spared (fixed-clock seconds).
    spared: HashMap<Entity, f64>,
    /// What it does to heroes, and whether it hits generators and
    /// breakables (the game's effect flag 2: magic and gas do, a fireball
    /// doesn't).
    heroes: Heroes,
    items: bool,
    /// The stages still to come when it ends (the poison cloud's).
    then: &'static [Stage],
    /// Its model's scale, and how far below the centre it's drawn (kept
    /// by the stages after it).
    scale: Vec3,
    drop: f32,
}

/// What a blast does to heroes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heroes {
    /// Nothing (the heroes' own magic).
    Spared,
    /// Hurts them (a gas cloud).
    Hurt,
    /// Hurts them and throws them back (an explosion).
    Thrown,
}

/// A later stage of a chained effect.
#[derive(Clone, Copy, Debug)]
struct Stage {
    model: &'static str,
    /// It still hurts.
    blast: bool,
    /// How long it stays, when not its clip's length.
    hold: Option<f32>,
}

/// A chained effect's next stage, where the last one ended.
#[derive(Message, Clone, Copy, Debug)]
struct NextStage {
    owner: Entity,
    centre: Vec3,
    kind: u32,
    damage: f32,
    radius: f32,
    heroes: Heroes,
    items: bool,
    stages: &'static [Stage],
    scale: Vec3,
    drop: f32,
}

/// A monster blows up (a suicide runner, `monsters.rs`): the fireball —
/// EXPLOSION, its ring and the level's SUICIDEEXP — or, in the poison
/// realms, the gas cloud and SUICIDEEXP; its sound; and a blast that hurts
/// heroes as well as monsters (`docs/monsters.md`, "Suicide runners").
#[derive(Message, Clone, Debug)]
pub struct ExplosionAt {
    pub owner: Entity,
    pub at: Vec3,
    pub damage: f32,
    pub poison: bool,
    /// The monster folder the level's SUICIDEEXP is in.
    pub folder: Option<String>,
    /// An exploding or poison barrel's (the game's effects 0x18 and 0x19
    /// through its other explosion routine): bigger, hitting items too,
    /// no ring or sound of its own.
    pub barrel: bool,
}

/// The fireball: EXPLOSION blasts out to 6 with fire (kind `0x421`), and
/// EXPRING plays at 1.2 ×.
const EXPLOSION_FX: &str = "EXPLOSION";
const EXPLOSION_KIND: u32 = 0x421;
const EXPLOSION_RADIUS: f32 = 6.0;
const RING_FX: &str = "EXPRING";
const RING_SCALE: f32 = 1.2;
/// The gas cloud: POISONEXP1 then POISONEXP2 (held its 2 s) blast out to
/// 7.5 (kind `0x800`), then POISONEXP3 plays out; drawn 2.5 × wide and 1
/// lower.
const POISON_FX: &str = "POISONEXP1";
const POISON_KIND: u32 = 0x800;
const POISON_RADIUS: f32 = 7.5;
const POISON_SCALE: Vec3 = Vec3::new(2.5, 1.0, 2.5);
const POISON_DROP: f32 = 1.0;
static POISON_STAGES: [Stage; 2] =
    [Stage { model: "POISONEXP2", blast: true, hold: None }, Stage { model: "POISONEXP3", blast: false, hold: None }];
/// A poison barrel's cloud: out to 6.5, held 4 s, drawn 3.5 × wide.
static BARREL_POISON_STAGES: [Stage; 2] =
    [Stage { model: "POISONEXP2", blast: true, hold: Some(4.0) }, Stage { model: "POISONEXP3", blast: false, hold: None }];
const BARREL_POISON_RADIUS: f32 = 6.5;
const BARREL_POISON_SCALE: Vec3 = Vec3::new(3.5, 1.0, 3.5);
/// An exploding barrel's fireball: out to 12, drawn 1.75 × wide and 2
/// higher.
const BARREL_EXPLOSION_RADIUS: f32 = 12.0;
const BARREL_EXPLOSION_SCALE: Vec3 = Vec3::new(1.75, 1.0, 1.75);
const BARREL_EXPLOSION_LIFT: f32 = 2.0;
const SUICIDE_FX: &str = "SUICIDEEXP";
const EXPLOSION_SOUND: &str = "S_SUICIDE_BOMB";

/// An effect model: its meshes (which run its texture modifiers as it
/// plays: the fireball's FBALL_EXP, a gas cloud's POISON_GAS, the acid
/// blast's gas), its life (seconds) and its particle systems (their
/// values, material and direction).
#[derive(Clone)]
struct EffectModel {
    model: Arc<CharacterModel>,
    life: f32,
    particles: ParticleSystems,
}

/// An effect's particle systems (its atree's kind-4 nodes): their values,
/// material and direction.
pub type ParticleSystems = Arc<[(particles::Params, Handle<LevelMaterial>, Vec3)]>;

/// The particle systems of an atree, their textures from the model file
/// the atree's model is built from (through the same cache).
pub fn particle_systems(
    data: &CharacterData,
    cache: &mut TextureCache,
    images: &mut Assets<Image>,
    materials: &mut Assets<LevelMaterial>,
) -> ParticleSystems {
    data.skeleton
        .particles
        .iter()
        .map(|n| {
            let params = particles::Params::of(&n.record);
            let texture = n.record.texture();
            let binding = data.model.texture_names.iter().find(|t| t.name == texture).map(|t| t.binding);
            let image = binding.and_then(|b| cache.get(b, images)).map(|(i, _)| i);
            let material = params.material(image, materials);
            (params, material, Vec3::from(n.vector))
        })
        .collect()
}

/// Sprays an effect's particle systems once at `at` (scaled).
pub fn spray(systems: &ParticleSystems, at: Vec3, scale: f32, seed: &mut u32, commands: &mut Commands, meshes: &mut Assets<Mesh>) {
    for (k, (params, material, direction)) in systems.iter().enumerate() {
        *seed = seed.wrapping_add(0x9E37_79B9);
        let seed = *seed ^ k as u32;
        particles::spawn_burst(params.clone().scaled(scale), material.clone(), at, *direction, seed, commands, meshes);
    }
}

/// Effect models, loaded on first use: `WEAPONS`' by atree name, and the
/// few that live in monster folders (SUICIDEEXP) by folder and name.
#[derive(Resource, Default)]
struct EffectModels {
    weapons: HashMap<&'static str, Option<EffectModel>>,
    banks: HashMap<(String, &'static str), Option<EffectModel>>,
}

impl EffectModels {
    /// The model and its life (seconds) for `name`.
    fn get(
        &mut self,
        name: &'static str,
        game: &mut LoadedGame,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<(Arc<CharacterModel>, f32)> {
        self.effect(name, game, meshes, materials, images).map(|e| (e.model, e.life))
    }

    fn effect(
        &mut self,
        name: &'static str,
        game: &mut LoadedGame,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<EffectModel> {
        self.weapons.entry(name).or_insert_with(|| load_effect(game, "WEAPONS", name, meshes, materials, images)).clone()
    }

    /// An effect from a monster folder (`MONSTERS/<folder>`).
    fn effect_in(
        &mut self,
        folder: &str,
        name: &'static str,
        game: &mut LoadedGame,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<EffectModel> {
        self.effect_from(&format!("MONSTERS/{folder}"), name, game, meshes, materials, images)
    }

    /// An effect from a bank's folder (`POWERUPS`, `MONSTERS/DEM`).
    fn effect_from(
        &mut self,
        path: &str,
        name: &'static str,
        game: &mut LoadedGame,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<EffectModel> {
        let key = (path.to_string(), name);
        if let Some(e) = self.banks.get(&key) {
            return e.clone();
        }
        let e = load_effect(game, path, name, meshes, materials, images);
        self.banks.insert(key, e.clone());
        e
    }
}

/// The effect table's entries drawn see-through (transparency 96: the
/// sparks and the hit and die effects); every other effect is drawn as it
/// is (transparency 0).
const SEE_THROUGH: [&str; 12] = [
    "SPARKS", "HITCOL", "HITDIE", "BLOODHIT", "BLOODDIE", "BLOODFX1", "BLOODFX2", "FIREHIT", "FIREDIE", "ELECDIE",
    "LIGHTDIE", "ACIDDIE",
];

/// The effect table's depth bias for an effect: none for the breaths, the
/// bags and the bare FX nodes; −512 for SUICIDEEXP and the pickup
/// sparkles (`GETGEM<colour>`, `GETGARG`, `GETRUNE`); −128 for the rest.
fn effect_bias(name: &str) -> i16 {
    const NONE: [&str; 9] = [
        "NULLFX", "MAGICFX", "FIREBREATHE", "ACIDBREATHE", "ELECBREATHE", "L_SHLD_ACTIVE", "BOSS_BREATHE", "BAG_THROW",
        "BAG_HIT",
    ];
    if NONE.contains(&name) {
        0
    } else if name == SUICIDE_FX || name.starts_with("GETGEM") || name == "GETGARG" || name == "GETRUNE" {
        -512
    } else {
        -128
    }
}

/// Loads an effect's model, life and particle systems from `folder`. The
/// see-through ones get the game's transparency on their own materials
/// (their models are only ever used as effects), and all of them its
/// depth bias.
fn load_effect(
    game: &mut LoadedGame,
    folder: &str,
    name: &'static str,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> Option<EffectModel> {
    let bytes = game.install.read(&format!("{folder}/ANIM.PS2")).ok()?;
    let anim = AnimFile::parse(&bytes).ok()?;
    let index = anim.atrees.iter().position(|a| a.name.eq_ignore_ascii_case(name))?;
    let texmods = TexMod::parse_all(&bytes).unwrap_or_default();
    let tree = anim.atrees.into_iter().nth(index)?;
    let data = CharacterData {
        name: format!("{folder}/{name}"),
        class: String::new(),
        colour: String::new(),
        clips: Arc::new(tree.clone()),
        skeleton: tree,
        model: ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?,
        textures: game.install.read(&format!("{folder}/textures.ngc")).ok()?,
    };
    let life = data.clips.actions.first().map_or(1.0, |a| effect_life(a.frames, a.rate));
    // Its particle systems' textures and flipbook frames come from the same
    // files, through the cache its model is built with.
    let mut cache = TextureCache::new(&data.model, &data.textures).sharing_materials();
    let particles = particle_systems(&data, &mut cache, images, materials);
    let mut model = CharacterModel::build_with(&data, &mut cache, meshes, materials, images);
    model.run_texmods(&data, &texmods, &mut cache, images);
    let model = Arc::new(model);
    let (see_through, bias) = (SEE_THROUGH.contains(&name), effect_bias(name));
    let near = PerspectiveProjection::default().near;
    for h in model.materials() {
        if let Some(m) = materials.get_mut(h) {
            if see_through {
                m.uv_offset.w = 1.0 - deaths::EFFECT_ALPHA;
                if matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                    m.alpha_mode = AlphaMode::Blend;
                }
            }
            m.set_depth_bias(bias, near);
        }
    }
    Some(EffectModel { model, life, particles })
}

/// A one-off effect model where something happened (a monster's die
/// effect, `deaths.rs`): played once, at the game's effect transparency.
#[derive(Message, Clone, Copy, Debug)]
pub struct EffectAt {
    /// Its atree in `WEAPONS`, or in `bank` (a folder: `POWERUPS`).
    pub name: &'static str,
    pub bank: Option<&'static str>,
    pub at: Vec3,
    /// Heading, radians.
    pub facing: f32,
    pub scale: f32,
}

/// A one-off effect's seconds left.
#[derive(Component)]
struct OneShot(f32);

#[allow(clippy::too_many_arguments)]
fn spawn_one_shots(
    mut commands: Commands,
    mut requests: MessageReader<EffectAt>,
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<EffectModels>,
    mut seed: Local<u32>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for e in requests.read() {
        let effect = match e.bank {
            Some(bank) => models.effect_from(bank, e.name, &mut game, &mut meshes, &mut materials, &mut images),
            None => models.effect(e.name, &mut game, &mut meshes, &mut materials, &mut images),
        };
        let Some(effect) = effect else {
            debug!("effect {} has no model", e.name);
            continue;
        };
        play_effect(&mut commands, &effect, e.at, e.facing, Vec3::splat(e.scale), &mut seed, &mut meshes);
    }
}

/// Plays an effect once where something happened: its particle systems
/// burst there at its scale, and its model plays out its life.
fn play_effect(
    commands: &mut Commands,
    effect: &EffectModel,
    at: Vec3,
    facing: f32,
    scale: Vec3,
    seed: &mut u32,
    meshes: &mut Assets<Mesh>,
) {
    spray(&effect.particles, at, scale.x, seed, commands, meshes);
    let transform = Transform::from_translation(at).with_rotation(Quat::from_rotation_y(facing)).with_scale(scale);
    let entity = effect.model.spawn(transform, commands);
    commands.entity(entity).insert((OneShot(effect.life), LevelEntity));
}

fn tick_one_shots(mut commands: Commands, time: Res<Time>, mut shots: Query<(Entity, &mut OneShot)>) {
    for (e, mut left) in &mut shots {
        left.0 -= time.delta_secs();
        if left.0 <= 0.0 {
            commands.entity(e).try_despawn();
        }
    }
}

/// The next colour for a potion without one: 1, 2, 3, 4, 1…
#[derive(Resource, Default)]
struct PotionCycle(u32);

/// The hero's magic stat, per level.
#[derive(Resource)]
struct HeroMagic {
    stat: [f32; 2],
}

fn setup_level(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    choice: Option<Res<PlayerChoice>>,
    state: Option<ResMut<PlayerState>>,
    mut granted: Local<bool>,
) {
    let Some(choice) = choice else { return };
    let stats = game
        .install
        .read(&format!("PDATA/{}.WAD", choice.class))
        .ok()
        .and_then(|b| PlayerStats::parse(&b).ok().flatten());
    let magic = stats.map_or([400.0, 400.0], |s| [s.magic.start, s.magic.max]);
    commands.insert_resource(HeroMagic { stat: magic });
    // GDL_POTIONS=<n>[,<kind>]: potions to test with, once.
    if let (Some(mut state), false) = (state, *granted)
        && let Ok(spec) = std::env::var("GDL_POTIONS")
    {
        let mut parts = spec.split(',').map(str::trim);
        let n: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let kind: i32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        for i in 0..n {
            let k = if kind > 0 { kind } else { 1 + (i % 4) as i32 };
            state.take_potions(k, 1);
        }
        info!("GDL_POTIONS: the hero has {} potions", state.potions.len());
        *granted = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn use_potions(
    mut commands: Commands,
    mut uses: MessageReader<UsePotion>,
    mut state: Option<ResMut<PlayerState>>,
    magic: Option<Res<HeroMagic>>,
    mut cycle: ResMut<PotionCycle>,
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<EffectModels>,
    ground: Option<Res<LevelGround>>,
    mut sounds: MessageWriter<PlaySound>,
    mut blasts: MessageWriter<BlastAt>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for u in uses.read() {
        let level = state.as_ref().map_or(1, |s| s.level.max(1));
        // The last potion picked up; none (only by cheating) cycles colours.
        let mut kind = state.as_mut().and_then(|s| s.potions.pop()).map_or(0, |k| k.max(0) as u32);
        if kind & 0xF == 0 {
            kind |= cycle.0 % 4 + 1;
            cycle.0 += 1;
        }
        let stat = magic.as_ref().map_or(400.0, |m| locomotion::stat_at_level(m.stat[0], m.stat[1], level, 0.0));
        let e = potion_effect(kind, u.mode, 0, powered_magic(stat, state.as_deref()), level);
        let c = colour_index(kind);
        info!(
            "potion {} (mode {}): {:.1} damage out to {:.1}; {} left",
            kind & 0xF,
            u.mode,
            e.damage,
            e.radius,
            state.as_ref().map_or(0, |s| s.potions.len())
        );
        match u.mode {
            0 => {
                sounds.write(PlaySound(POTION_SOUND[c].into()));
                blasts.write(BlastAt { owner: u.hero, at: u.feet, kind: e.kind, damage: e.damage, radius: e.radius });
            }
            1 => {
                sounds.write(PlaySound(SHIELD_SOUND[c].into()));
                let effect = models.effect(SHIELD_FX[c], &mut game, &mut meshes, &mut materials, &mut images);
                let scale = (e.radius / 12.0).clamp(0.33, 1.0);
                spawn_blast(
                    &mut commands,
                    effect.as_ref(),
                    Blast {
                        owner: u.hero,
                        shape: BlastShape::Aura(u.hero),
                        centre: u.feet,
                        kind: e.kind,
                        damage: e.damage,
                        radius: e.radius,
                        life: SHIELD_LIFE,
                        age: 0.0,
                        spared: HashMap::new(),
                        heroes: Heroes::Spared,
                        items: true,
                        then: &[],
                        scale: Vec3::new(scale, 1.0, scale),
                        drop: 0.0,
                    },
                    c,
                );
            }
            _ => {
                // Thrown: up at 45° from 2 ahead and 4 up, faster the
                // longer magic was held.
                let fwd = Vec3::new(u.facing.sin(), 0.0, u.facing.cos());
                let speed = 1.5 * u.charge + 5.0;
                let start = u.feet + fwd * 2.0 + Vec3::Y * 4.0;
                let velocity = Vec3::new(fwd.x * 0.707, 0.707, fwd.z * 0.707) * speed;
                let model = models.get(THROWN_FX[c], &mut game, &mut meshes, &mut materials, &mut images);
                let burst = PotionBurst { kind: e.kind, damage: e.damage, radius: e.radius };
                let blocked = ground.as_deref().is_some_and(|g| projectiles::wall_between(&g.0, u.feet + Vec3::Y * 4.0, start, 0.5));
                if blocked {
                    blasts.write(BlastAt { owner: u.hero, at: start, kind: e.kind, damage: e.damage, radius: e.radius });
                } else {
                    projectiles::spawn_potion(&mut commands, model.as_ref().map(|m| &*m.0), u.hero, start, velocity, burst);
                }
            }
        }
    }
}

/// The shield lasts three seconds.
const SHIELD_LIFE: f32 = 3.0;

/// Potions lying on the floor that a blow or blast struck go off (the
/// game's item damage for them, then its item-hit routine for a hero's):
/// the potion is gone, its magic bursts there for nobody, and a hero who
/// struck it gets a blast of their own of it at 0.8 of their power, with
/// its sound and the hint that shooting magic has a lower effect.
#[allow(clippy::too_many_arguments)]
fn set_off_potions(
    mut commands: Commands,
    mut strikes: MessageReader<StrikePotion>,
    items: Option<ResMut<crate::items::LevelItems>>,
    state: Option<Res<PlayerState>>,
    magic: Option<Res<HeroMagic>>,
    mut cycle: ResMut<PotionCycle>,
    mut blasts: MessageWriter<BlastAt>,
    (mut sounds, mut hints): (MessageWriter<PlaySound>, MessageWriter<crate::hints::ShowHint>),
) {
    let Some(mut items) = items else {
        strikes.clear();
        return;
    };
    for s in strikes.read() {
        let Some(view) = items.view(s.placement).filter(is_floor_potion) else { continue };
        let at = Vec3::from(view.shape.centre);
        let value = view.ty.value.max(0) as u32;
        let mut colour = || {
            if value & 0xF != 0 {
                return value;
            }
            cycle.0 += 1;
            value | ((cycle.0 - 1) % 4 + 1)
        };
        let kind = colour();
        items.free(s.placement, &mut commands);
        info!("a potion is struck at {at:?}");
        blasts.write(BlastAt { owner: Entity::PLACEHOLDER, at, kind: kind | 0x200, damage: NOBODYS_DAMAGE, radius: NOBODYS_RADIUS });
        if let Some(hero) = s.by {
            let kind = colour();
            let level = state.as_ref().map_or(1, |s| s.level.max(1));
            let stat = magic.as_ref().map_or(400.0, |m| locomotion::stat_at_level(m.stat[0], m.stat[1], level, 0.0));
            let e = potion_effect(kind, 0, 0, STRUCK_POWER * powered_magic(stat, state.as_deref()), level);
            sounds.write(PlaySound(POTION_SOUND[colour_index(kind)].into()));
            blasts.write(BlastAt { owner: hero, at, kind: e.kind, damage: e.damage, radius: e.radius });
            hints.write(crate::hints::ShowHint(crate::hints::Hint::ShootPotion));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_blasts(
    mut commands: Commands,
    mut requests: MessageReader<BlastAt>,
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<EffectModels>,
    ground: Option<Res<LevelGround>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for b in requests.read() {
        let c = colour_index(b.kind);
        let effect = models.effect(BLAST_FX[c], &mut game, &mut meshes, &mut materials, &mut images);
        // Set on the floor under it; the model is drawn for a 32-unit blast.
        let mut at = b.at;
        if let Some(y) = ground.as_ref().and_then(|g| g.0.floor_height(at.to_array())) {
            at.y = y;
        }
        let life = effect.as_ref().map_or(1.5, |e| e.life);
        info!("magic blast at {at:?}: {:.1} damage out to {:.1} over {life:.2} s", b.damage, b.radius);
        spawn_blast(
            &mut commands,
            effect.as_ref(),
            Blast {
                owner: b.owner,
                shape: BlastShape::Grow,
                centre: at,
                kind: b.kind,
                damage: b.damage,
                radius: b.radius,
                life,
                age: 0.0,
                spared: HashMap::new(),
                heroes: Heroes::Spared,
                items: true,
                then: &[],
                scale: Vec3::splat((b.radius / 32.0).min(1.0)),
                drop: 0.0,
            },
            c,
        );
    }
}

/// A monster's explosion goes off (and a chained one's next stage
/// starts): its sound, its blast with its effect model, and the effects
/// that only show.
#[allow(clippy::too_many_arguments)]
fn spawn_explosions(
    mut commands: Commands,
    mut requests: MessageReader<ExplosionAt>,
    mut next: MessageReader<NextStage>,
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<EffectModels>,
    mut sounds: MessageWriter<PlaySound>,
    mut seed: Local<u32>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for e in requests.read() {
        if !e.barrel {
            sounds.write(PlaySound(EXPLOSION_SOUND.into()));
        }
        let (fx, kind, radius, heroes, then, scale, drop) = match (e.poison, e.barrel) {
            (true, false) => (POISON_FX, POISON_KIND, POISON_RADIUS, Heroes::Hurt, &POISON_STAGES[..], POISON_SCALE, POISON_DROP),
            (false, false) => (EXPLOSION_FX, EXPLOSION_KIND, EXPLOSION_RADIUS, Heroes::Thrown, &[][..], Vec3::ONE, 0.0),
            (true, true) => {
                (POISON_FX, POISON_KIND, BARREL_POISON_RADIUS, Heroes::Hurt, &BARREL_POISON_STAGES[..], BARREL_POISON_SCALE, 0.0)
            }
            (false, true) => (
                EXPLOSION_FX,
                EXPLOSION_KIND,
                BARREL_EXPLOSION_RADIUS,
                Heroes::Thrown,
                &[][..],
                BARREL_EXPLOSION_SCALE,
                -BARREL_EXPLOSION_LIFT,
            ),
        };
        let effect = models.effect(fx, &mut game, &mut meshes, &mut materials, &mut images);
        let life = effect.as_ref().map_or(1.0, |e| e.life);
        info!("{fx} at {:?}: {:.1} damage out to {radius:.1} over {life:.2} s", e.at, e.damage);
        let blast = Blast {
            owner: e.owner,
            shape: BlastShape::Grow,
            centre: e.at,
            kind,
            damage: e.damage,
            radius,
            life,
            age: 0.0,
            spared: HashMap::new(),
            heroes,
            items: e.poison || e.barrel,
            then,
            scale,
            drop,
        };
        spawn_blast(&mut commands, effect.as_ref(), blast, colour_index(kind));
        if !e.poison
            && !e.barrel
            && let Some(ring) = models.effect(RING_FX, &mut game, &mut meshes, &mut materials, &mut images)
        {
            play_effect(&mut commands, &ring, e.at, 0.0, Vec3::splat(RING_SCALE), &mut seed, &mut meshes);
        }
        if let Some(folder) = &e.folder
            && let Some(fx) = models.effect_in(folder, SUICIDE_FX, &mut game, &mut meshes, &mut materials, &mut images)
        {
            play_effect(&mut commands, &fx, e.at, 0.0, Vec3::ONE, &mut seed, &mut meshes);
        }
    }
    for s in next.read() {
        let Some((stage, rest)) = s.stages.split_first() else { continue };
        let effect = models.effect(stage.model, &mut game, &mut meshes, &mut materials, &mut images);
        let life = stage.hold.unwrap_or(effect.as_ref().map_or(1.0, |e| e.life));
        let blast = Blast {
            owner: s.owner,
            shape: BlastShape::Grow,
            centre: s.centre,
            kind: s.kind,
            damage: s.damage,
            radius: if stage.blast { s.radius } else { 0.0 },
            life,
            age: 0.0,
            spared: HashMap::new(),
            heroes: s.heroes,
            items: s.items,
            then: rest,
            scale: s.scale,
            drop: s.drop,
        };
        spawn_blast(&mut commands, effect.as_ref(), blast, colour_index(s.kind));
    }
}

/// The effect's model; without one, a stand-in: a translucent sphere in
/// the potion's light colour showing the blast's reach.
fn spawn_blast(commands: &mut Commands, effect: Option<&EffectModel>, blast: Blast, colour: usize) {
    let transform = Transform::from_translation(blast.centre - Vec3::Y * blast.drop).with_scale(blast.scale);
    let entity = match effect {
        Some(e) => e.model.spawn(transform, commands),
        None => commands.spawn((transform, Visibility::default())).id(),
    };
    commands.entity(entity).insert((blast, BlastColour(colour, effect.is_some()), LevelEntity));
}

#[derive(Component)]
/// The potion's colour, and whether the effect has its own model (then
/// no stand-in sphere).
struct BlastColour(usize, bool);

/// The stand-in reach sphere, a child of the blast.
#[derive(Component)]
struct ReachSphere;

#[allow(clippy::too_many_arguments)]
fn tick_blasts(
    mut commands: Commands,
    time: Res<Time>,
    mut blasts: Query<(Entity, &mut Blast)>,
    mut players: Query<(Entity, &mut Player)>,
    targets: Query<(Entity, &GlobalTransform, &Targetable, Option<&Monster>)>,
    mut hits: MessageWriter<Hit>,
    (mut hurt, mut stages): (MessageWriter<DamagePlayer>, MessageWriter<NextStage>),
    (items, mut struck): (Option<Res<crate::items::LevelItems>>, MessageWriter<StrikePotion>),
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    let bodies = projectiles::bodies(&targets);
    for (entity, mut b) in &mut blasts {
        let b = &mut *b;
        b.age += dt;
        let left = b.life - b.age;
        if left <= 0.0 {
            if let Some((_, rest)) = b.then.split_first() {
                stages.write(NextStage {
                    owner: b.owner,
                    centre: b.centre,
                    kind: b.kind,
                    damage: b.damage,
                    radius: b.radius,
                    heroes: b.heroes,
                    items: b.items,
                    stages: b.then,
                    scale: b.scale,
                    drop: b.drop,
                });
                debug!("blast stage over; {} to come", rest.len() + 1);
            }
            commands.entity(entity).despawn();
            continue;
        }
        if b.radius <= 0.0 {
            continue;
        }
        if let BlastShape::Aura(hero) = b.shape
            && let Ok((_, p)) = players.get(hero)
        {
            b.centre = Vec3::from(p.mover.position);
        }
        // Grow: reach and share by the time left; the shield: steady, full
        // damage, each target spared a second (at most what's left).
        let (reach, share, spare) = match b.shape {
            BlastShape::Grow => match blast_front(left, b.life, b.radius) {
                Some((r, s)) => (r, s, (left + 1.0 / 15.0).max(0.2)),
                None => continue,
            },
            BlastShape::Aura(_) => (b.radius, 1.0, left.clamp(0.2, 1.0)),
        };
        let damage = b.damage * share;
        let mut hit = |target: Entity, target_kind: TargetKind, b: &mut Blast| {
            if b.spared.get(&target).is_some_and(|&until| until > now) {
                return;
            }
            if damage > 2.0 {
                b.spared.insert(target, now + spare as f64);
            }
            info!("magic hits {target_kind:?} {target:?} for {damage:.1}");
            hits.write(Hit {
                target,
                attacker: b.owner,
                damage,
                kind: b.kind,
                push: Vec3::ZERO,
                at: b.centre,
                target_kind,
                ranged: true,
            });
        };
        // A critter once: its first sphere in reach, else its body.
        let reached: Vec<&projectiles::Body> =
            bodies.iter().filter(|body| (body.centre - b.centre).length() <= reach + body.radius).collect();
        for body in combat::one_per_critter(reached, |body| body.aim) {
            hit(body.entity, body.kind, b);
        }
        for (e, g, t, _) in &targets {
            if !b.items || !matches!(t.kind, TargetKind::Generator | TargetKind::Breakable) {
                continue;
            }
            let feet = g.translation();
            let across = Vec2::new(feet.x - b.centre.x, feet.z - b.centre.z).length();
            let dy = b.centre.y - (feet.y + 0.5 * t.height);
            if across <= t.radius + reach && dy.abs() <= 0.5 * t.height + reach {
                hit(e, t.kind, b);
            }
        }
        // Potions lying in its reach go off too.
        if b.items
            && let Some(items) = items.as_deref()
        {
            let by = players.contains(b.owner).then_some(b.owner);
            for v in items.views().filter(is_floor_potion) {
                let c = Vec3::from(v.shape.centre);
                let across = Vec2::new(c.x - b.centre.x, c.z - b.centre.z).length();
                if across <= v.shape.radius + reach && (b.centre.y - c.y).abs() <= v.shape.reach + reach {
                    struck.write(StrikePotion { placement: v.placement, by });
                }
            }
        }
        // A monster's explosion hurts heroes too: through their armour, and
        // a fireball throws them back (as a barrel's blast does).
        if b.heroes == Heroes::Spared {
            continue;
        }
        for (e, mut p) in &mut players {
            let feet = Vec3::from(p.mover.position);
            let centre = feet + Vec3::Y * projectiles::PLAYER_CENTRE;
            if (centre - b.centre).length() > reach + p.radius || b.spared.get(&e).is_some_and(|&until| until > now) {
                continue;
            }
            if damage > 2.0 {
                b.spared.insert(e, now + spare as f64);
            }
            let amount = after_armor(damage, p.armor);
            if amount <= 0.0 {
                continue;
            }
            if b.heroes == Heroes::Thrown {
                let away = (feet - b.centre).with_y(0.0).normalize_or_zero();
                p.queue_hit(amount, 0x10, away);
            }
            hurt.write(DamagePlayer { amount });
            info!("the blast hurts the hero for {amount:.1}");
        }
    }
}

/// The stand-in sphere's mesh and its four potion colours.
type SphereLook = (Handle<Mesh>, [Handle<StandardMaterial>; 4]);

/// Moves the shield with its hero and sizes the stand-in reach spheres.
#[allow(clippy::too_many_arguments)]
fn follow_blasts(
    mut commands: Commands,
    mut blasts: Query<(Entity, &Blast, &BlastColour, &mut Transform, Option<&Children>)>,
    mut spheres: Query<(&ReachSphere, &mut Transform), Without<Blast>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut sphere: Local<Option<SphereLook>>,
) {
    let (mesh, mats) = sphere
        .get_or_insert_with(|| {
            let mesh = meshes.add(Sphere::new(1.0).mesh().uv(24, 16));
            let mats = LIGHT.map(|[r, g, b]| {
                materials.add(StandardMaterial {
                    base_color: Color::srgba(r / 2.0, g / 2.0, b / 2.0, 0.25),
                    alpha_mode: AlphaMode::Add,
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            });
            (mesh, mats)
        })
        .clone();
    for (entity, b, colour, mut transform, children) in &mut blasts {
        transform.translation = b.centre - Vec3::Y * b.drop;
        let reach = match b.shape {
            BlastShape::Grow => blast_front(b.life - b.age, b.life, b.radius).map_or(0.0, |(r, _)| r),
            BlastShape::Aura(_) => b.radius,
        };
        // The sphere is in the blast's (scaled) space.
        let local = Vec3::splat(reach) / transform.scale.max(Vec3::splat(1e-3));
        let mut found = false;
        for child in children.into_iter().flatten() {
            if let Ok((_, mut t)) = spheres.get_mut(*child) {
                t.scale = local;
                found = true;
            }
        }
        if !found && !colour.1 {
            commands.spawn((
                ReachSphere,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mats[colour.0].clone()),
                Transform::from_scale(local),
                ChildOf(entity),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_power_spans_eight_to_thirty_two() {
        assert_eq!(magic_power(0.0), 8.0);
        assert!((magic_power(1000.0) - 32.0).abs() < 1e-4);
    }

    #[test]
    fn potions_hit_by_mode_and_colour() {
        let e = potion_effect(1, 0, 0, 20.0, 1);
        assert_eq!((e.damage, e.radius, e.kind), (40.0, 20.0, 0x201));
        // Yellow is player 1's: 10% more damage and reach.
        let e = potion_effect(3, 0, 0, 20.0, 1);
        assert!((e.damage - 44.0).abs() < 1e-4 && (e.radius - 22.0).abs() < 1e-4);
        let s = potion_effect(2, 1, 0, 20.0, 30);
        assert_eq!((s.damage, s.radius), (25.0, 5.0));
        assert_ne!(s.kind & 0x80_0000, 0);
        let t = potion_effect(4, 2, 0, 20.0, 1);
        assert_eq!((t.damage, t.radius), (40.0, 15.0));
    }

    #[test]
    fn blasts_grow_and_weaken() {
        let life = 2.0;
        let (r0, s0) = blast_front(life, life, 30.0).unwrap();
        assert!((r0 - 9.9).abs() < 1e-3 && (s0 - 1.005).abs() < 1e-3);
        let (r1, s1) = blast_front(1.0, life, 30.0).unwrap();
        assert!(r1 > r0 && s1 < s0);
        assert!(blast_front(0.6, life, 30.0).is_none());
        assert!((effect_life(37, 60) - 37.0 * 60.0 / 900.0).abs() < 1e-5);
        assert!((effect_life(30, 0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn tap_blasts_double_tap_shields_hold_throws() {
        let mut m = MagicState::default();
        assert_eq!(m.observe(button::MAGIC, 0, 2.0), Some(MagicIntent::Magic));
        // Let go during MAGICS, press again: shield.
        m.observe(0, 0x73, 2.0);
        assert_ne!(m.flags & MagicState::RELEASED, 0);
        m.observe(button::MAGIC, 0x73, 2.0);
        assert_ne!(m.flags & MagicState::SHIELD, 0);
        // Holding through THROWPOTIONS winds up until let go.
        let mut m = MagicState::default();
        m.observe(button::MAGIC, 0x75, 2.0);
        m.observe(button::MAGIC, 0x75, 2.0);
        m.observe(0, 0x75, 2.0);
        m.observe(button::MAGIC, 0x75, 2.0);
        assert_eq!(m.charge, 4.0);
        // After a use, magic is ignored until let go.
        let mut m = MagicState { flags: MagicState::USED, charge: 0.0 };
        assert_eq!(m.observe(button::MAGIC, 0, 2.0), None);
        assert_eq!(m.observe(0, 0, 2.0), None);
        assert_eq!(m.observe(button::MAGIC, 0, 2.0), Some(MagicIntent::Magic));
    }

    #[test]
    fn colours_favour_their_player() {
        assert_eq!(favourite_player(3), Some(0));
        assert_eq!(favourite_player(1), Some(2));
        assert_eq!(colour_index(0x203), 2);
    }
}
