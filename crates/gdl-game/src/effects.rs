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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::pdata::PlayerStats;

use crate::audio::PlaySound;
use crate::character::{CharacterData, CharacterModel, clip_fps};
use crate::deaths;
use crate::combat::{Hit, TargetKind, Targetable, button};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::locomotion;
use crate::model_mesh::TextureCache;
use crate::monsters::{Monster, MonsterTick};
use crate::particles;
use crate::player::{Player, PlayerChoice};
use crate::player_state::PlayerState;
use crate::population::LevelPopulation;
use crate::projectiles;
use crate::world::{LevelEntity, LevelGround};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<UsePotion>()
            .add_message::<BlastAt>()
            .add_message::<EffectAt>()
            .init_resource::<EffectModels>()
            .init_resource::<PotionCycle>()
            .add_systems(
                FixedUpdate,
                (use_potions, spawn_blasts, tick_blasts, spawn_one_shots, tick_one_shots).chain().after(MonsterTick),
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

/// A magic effect doing damage.
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
}

/// An effect model: its meshes, its life (seconds) and its particle
/// systems (their values, material and direction).
#[derive(Clone)]
struct EffectModel {
    model: Arc<CharacterModel>,
    life: f32,
    particles: Arc<[(particles::Params, Handle<LevelMaterial>, Vec3)]>,
}

/// Effect models, by atree name (loaded on first use from `WEAPONS`).
#[derive(Resource, Default)]
struct EffectModels(HashMap<&'static str, Option<EffectModel>>);

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
        self.0
            .entry(name)
            .or_insert_with(|| {
                let data: CharacterData = projectiles::load_atree(game, "WEAPONS", name)?;
                let life = data.clips.actions.first().map_or(1.0, |a| effect_life(a.frames, a.rate));
                // Its particle systems' textures come from the same files.
                let mut cache = TextureCache::new(&data.model, &data.textures);
                let particles = data
                    .skeleton
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
                    .collect();
                let model = Arc::new(CharacterModel::build(&data, meshes, materials, images));
                Some(EffectModel { model, life, particles })
            })
            .clone()
    }
}

/// A one-off effect model where something happened (a monster's die
/// effect, `deaths.rs`): played once, at the game's effect transparency.
#[derive(Message, Clone, Copy, Debug)]
pub struct EffectAt {
    /// Its atree in `WEAPONS`.
    pub name: &'static str,
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
    mut faded: Local<HashSet<&'static str>>,
    mut seed: Local<u32>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    for e in requests.read() {
        let Some(effect) = models.effect(e.name, &mut game, &mut meshes, &mut materials, &mut images) else {
            debug!("effect {} has no model", e.name);
            continue;
        };
        // Its particle systems: bursts where it happens, at its scale.
        for (k, (params, material, direction)) in effect.particles.iter().enumerate() {
            *seed = seed.wrapping_add(0x9E37_79B9);
            let seed = *seed ^ k as u32;
            particles::spawn_burst(params.clone().scaled(e.scale), material.clone(), e.at, *direction, seed, &mut commands, &mut meshes);
        }
        let (model, life) = (effect.model, effect.life);
        // The game draws effects part see-through (its models are only
        // ever used as effects, so their own materials change).
        if faded.insert(e.name) {
            for h in model.materials() {
                if let Some(m) = materials.get_mut(h) {
                    m.uv_offset.w = 1.0 - deaths::EFFECT_ALPHA;
                    if matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                        m.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
        }
        let transform = Transform::from_translation(e.at)
            .with_rotation(Quat::from_rotation_y(e.facing))
            .with_scale(Vec3::splat(e.scale));
        let entity = model.spawn(transform, &mut commands);
        commands.entity(entity).insert((OneShot(life), LevelEntity));
    }
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
        let e = potion_effect(kind, u.mode, 0, magic_power(stat), level);
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
                let model = models.get(SHIELD_FX[c], &mut game, &mut meshes, &mut materials, &mut images);
                let scale = (e.radius / 12.0).clamp(0.33, 1.0);
                spawn_blast(
                    &mut commands,
                    model.as_ref().map(|m| &*m.0),
                    Vec3::new(scale, 1.0, scale),
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
        let model = models.get(BLAST_FX[c], &mut game, &mut meshes, &mut materials, &mut images);
        // Set on the floor under it; the model is drawn for a 32-unit blast.
        let mut at = b.at;
        if let Some(y) = ground.as_ref().and_then(|g| g.0.floor_height(at.to_array())) {
            at.y = y;
        }
        let life = model.as_ref().map_or(1.5, |m| m.1);
        info!("magic blast at {at:?}: {:.1} damage out to {:.1} over {life:.2} s", b.damage, b.radius);
        spawn_blast(
            &mut commands,
            model.as_ref().map(|m| &*m.0),
            Vec3::splat((b.radius / 32.0).min(1.0)),
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
            },
            c,
        );
    }
}

/// The effect's model; without one, a stand-in: a translucent sphere in
/// the potion's light colour showing the blast's reach.
fn spawn_blast(commands: &mut Commands, model: Option<&CharacterModel>, scale: Vec3, blast: Blast, colour: usize) {
    let transform = Transform::from_translation(blast.centre).with_scale(scale);
    let entity = match model {
        Some(m) => m.spawn(transform, commands),
        None => commands.spawn((transform, Visibility::default())).id(),
    };
    commands.entity(entity).insert((blast, BlastColour(colour, model.is_some()), LevelEntity));
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
    players: Query<&Player>,
    targets: Query<(Entity, &GlobalTransform, &Targetable, Option<&Monster>)>,
    mut hits: MessageWriter<Hit>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    let bodies = projectiles::bodies(&targets);
    for (entity, mut b) in &mut blasts {
        let b = &mut *b;
        b.age += dt;
        let left = b.life - b.age;
        if left <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if let BlastShape::Aura(hero) = b.shape
            && let Ok(p) = players.get(hero)
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
        for body in &bodies {
            if (body.centre - b.centre).length() <= reach + body.radius {
                hit(body.entity, body.kind, b);
            }
        }
        for (e, g, t, _) in &targets {
            if !matches!(t.kind, TargetKind::Generator | TargetKind::Breakable) {
                continue;
            }
            let feet = g.translation();
            let across = Vec2::new(feet.x - b.centre.x, feet.z - b.centre.z).length();
            let dy = b.centre.y - (feet.y + 0.5 * t.height);
            if across <= t.radius + reach && dy.abs() <= 0.5 * t.height + reach {
                hit(e, t.kind, b);
            }
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
        transform.translation = b.centre;
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
