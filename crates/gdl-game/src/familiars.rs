//! The heroes' familiars and the phoenix (`docs/projectiles.md`, "The
//! familiars and the phoenix"; `docs/powers.md`, "`0x80` phoenix").
//!
//! From level 30 a hero wears a familiar — `FAMILIAR1`, `FAMILIAR2` from
//! level 80, out of its effects bank — hanging from its model (not its
//! skeleton: the Pojo and levitation leave it be). On the tick after each
//! missile the hero lets go of, the familiar spits: it plays its ATTACK
//! and a `FAMILIAR_SPIT` flies. With the phoenix power the familiar hides
//! and the phoenix (its look rides the body, `power_looks.rs`) fires a
//! `PHOENIX_FBALL` instead. Both leave from the class's mouth point, aimed
//! as the throw was, lobbed to come down 21 units ahead for a speed of 50
//! but launched at 35 — the game's own mismatch, so they land short —
//! falling at 10 (straight on boss levels), and fly as the hero's missiles
//! for 3 s (`projectiles::spawn_hero_missile`).
//!
//! Stand-ins: the shot is aimed with the release tick's target search (the
//! game searches again on the tick it fires); the secret classes, which
//! have no effects bank of their own, use the one of the class eight
//! before them, as their thrown weapons do.

use gdl_formats::detmath::Det;
use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::AnimFile;
use gdl_formats::chunk::ChunkFile;
use gdl_formats::texmod::TexMod;

use crate::character::{self, Animator, CharacterData, CharacterModel};
use crate::level::LoadedGame;
use crate::level_material::LevelMaterial;
use crate::model_mesh::TextureCache;
use crate::monsters::MonsterLevel;
use crate::party::Party;
use crate::player::{Player, PlayerChoice};
use crate::player_state::PowersTick;
use crate::projectiles::{self, HeroShot};
use crate::texanim::{self, TexAnim};

pub struct FamiliarsPlugin;

impl Plugin for FamiliarsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FamiliarModels>().add_systems(FixedUpdate, familiars.after(PowersTick));
    }
}

/// The familiars by hero level: `FAMILIAR1` from 30, `FAMILIAR2` from 80.
const FAMILIARS: [(u32, &str); 2] = [(80, "FAMILIAR2"), (30, "FAMILIAR1")];
/// A familiar's shot, from the same bank.
const SPIT: &str = "FAMILIAR_SPIT";
/// The phoenix's fireball, from `WEAPONS`.
const FIREBALL: &str = "PHOENIX_FBALL";
const WEAPONS: &str = "WEAPONS";
/// The phoenix power (special bit): it hides the familiar and fires in
/// its place.
const PHOENIX: u32 = 0x80;
/// A familiar put on at level 99 sits 1.2 × as far out.
const TOP_LEVEL: u32 = 99;
const TOP_LEVEL_FAMILIAR: f32 = 1.2;
/// The class record (`PDAT`): where the familiar hangs and the mouth point
/// its shots leave from, in the hero's frame from its feet.
const PDAT_FAMILIAR: usize = 0x164;
const PDAT_MOUTH: usize = 0x170;
/// The familiar's actions.
const READY: usize = 0;
const ATTACK: usize = 1;

/// The shot: reach 21 (the throw's 15 + 200 × a wind-up of 0.03), a lob
/// solved for 50 but launched at 35, falling at 10 and aimed 0.5 below
/// the reach point (no fall, no drop on boss levels), radius 1.
const REACH: f32 = 21.0;
const LOB_SPEED: f32 = 50.0;
const SPEED: f32 = 35.0;
const GRAVITY: f32 = 10.0;
const DROP: f32 = -0.5;
const RADIUS: f32 = 1.0;
/// The phoenix's fireball: 10, fire and a strong knock (`0x11`).
const FIREBALL_DAMAGE: f32 = 10.0;
const FIREBALL_KIND: u32 = 0x11;
/// The crossbow's weapon bit: with it the aim is straight along the
/// facing.
const STRAIGHT: u32 = 0x10_0000;
/// The dwarf (class 4) aims 0.2 higher, as it throws.
const DWARF: usize = 4;

/// A familiar's spit: 0.1 × (level − 25) + 2.5 — 3 at 30, 8 at 80, 9.9 at
/// 99 — with kind 0.
fn spit_damage(level: u32) -> f32 {
    0.1 * (level as f32 - 25.0) + 2.5
}

/// Which familiar a hero of `level` wears, if any.
fn familiar_for(level: u32) -> Option<&'static str> {
    FAMILIARS.iter().find(|(from, _)| level >= *from).map(|(_, name)| *name)
}

/// Where a familiar's or the phoenix's shot starts and its velocity and
/// fall: from the mouth point (`mouth` × the model's `scale`, turned by
/// the facing) above the feet; aimed as the hero's throw (`hero_aim`, or
/// straight along the facing with the crossbow), 21 units out; lobbed for
/// a speed of 50 and let go at 35.
#[allow(clippy::too_many_arguments)]
fn shot(feet: Vec3, facing: f32, mouth: Vec3, scale: f32, aim: Vec3, targeted: bool, weapon: u32, dwarf: bool, boss: bool) -> (Vec3, Vec3, f32) {
    let start = feet + Quat::from_rotation_y(facing) * (mouth * scale);
    let aim = if weapon & STRAIGHT != 0 {
        Vec3::new(facing.dsin(), 0.0, facing.dcos())
    } else {
        projectiles::hero_aim(facing, aim, targeted, 0.0, dwarf, false)
    };
    let target = aim * REACH;
    let (gravity, drop) = if boss { (0.0, 0.0) } else { (GRAVITY, DROP) };
    let dir = projectiles::lob(Vec2::new(target.x, target.z), target.y + drop, LOB_SPEED, gravity);
    (start, dir * SPEED, gravity)
}

/// The familiar a hero wears.
#[derive(Component, Default)]
struct Familiar {
    /// Its name and model root.
    worn: Option<(&'static str, Entity)>,
    hidden: bool,
}

/// The familiars' models and the classes' points, loaded on first use.
#[derive(Resource, Default)]
struct FamiliarModels {
    models: HashMap<(String, &'static str), Option<Arc<CharacterModel>>>,
    /// The class code, and its familiar place and mouth point.
    points: HashMap<String, (Vec3, Vec3)>,
    /// `WEAPONS`' files: the textures an effects bank names but doesn't
    /// hold (the spit's `PIXIE_<colour>`, `WIZ_HEAD_<colour>` and their
    /// frames), found by name as the game does.
    weapons: Option<Option<(ModelFile, Vec<u8>)>>,
    /// The banks' running flipbooks on the models' materials, and the
    /// frame each shows.
    anims: Vec<(TexAnim, Option<u32>)>,
    /// Ticks since the first model was made, for the flipbooks.
    ticks: u64,
}

impl FamiliarModels {
    /// `name` from the first of `folders` that has it.
    fn get(
        &mut self,
        folders: &[String],
        name: &'static str,
        game: &mut LoadedGame,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<Arc<CharacterModel>> {
        let key = (folders.first()?.clone(), name);
        if let Some(model) = self.models.get(&key) {
            return model.clone();
        }
        let weapons = self.weapons.get_or_insert_with(|| {
            let model = ModelFile::parse(&game.install.read(&format!("{WEAPONS}/objects.ngc")).ok()?).ok()?;
            Some((model, game.install.read(&format!("{WEAPONS}/textures.ngc")).ok()?))
        });
        let built = folders.iter().find_map(|f| load(game, f, name, weapons.as_ref(), meshes, materials, images));
        let model = built.map(|(model, anims)| {
            self.anims.extend(anims.into_iter().map(|a| (a, None)));
            model
        });
        if model.is_none() {
            warn!("no {name} in {folders:?}");
        }
        self.models.insert(key, model.clone());
        model
    }

    /// Steps the banks' flipbooks by a tick.
    fn step(&mut self, materials: &mut Assets<LevelMaterial>) {
        self.ticks += 1;
        for (a, shown) in &mut self.anims {
            let frame = a.texmod.frame(self.ticks);
            if *shown == Some(frame) {
                continue;
            }
            *shown = Some(frame);
            let Some(Some(image)) = a.frames.get(frame as usize) else { continue };
            for h in &a.materials {
                if let Some(m) = materials.get_mut(h) {
                    m.diffuse = Some(image.clone());
                }
            }
        }
    }

    /// The class's familiar place and mouth point (`PDAT +0x164`,
    /// `+0x170`).
    fn points(&mut self, class: &str, game: &mut LoadedGame) -> (Vec3, Vec3) {
        if !self.points.contains_key(class) {
            let mut read = || {
                let bytes = game.install.read(&format!("PDATA/{class}.WAD")).ok()?;
                let file = ChunkFile::parse(&bytes).ok()?;
                let r = file.bytes("PDAT")?;
                let f = |at: usize| Some(f32::from_le_bytes(r.get(at..at + 4)?.try_into().ok()?));
                let v = |at: usize| Some(Vec3::new(f(at)?, f(at + 4)?, f(at + 8)?));
                Some((v(PDAT_FAMILIAR)?, v(PDAT_MOUTH)?))
            };
            let (at, mouth) = read().unwrap_or_else(|| {
                warn!("no familiar points for {class}");
                (Vec3::ZERO, Vec3::new(-1.0, 5.0, -1.0))
            });
            self.points.insert(class.to_string(), (at, mouth));
        }
        self.points.get(class).copied().unwrap_or((Vec3::ZERO, Vec3::ZERO))
    }
}

/// An atree from `folder`, built with its bank's texture modifiers: its
/// actions' and nodes', and the bank's running flipbooks (returned, to be
/// stepped) — their frames from `WEAPONS` when the bank only names them.
fn load(
    game: &mut LoadedGame,
    folder: &str,
    name: &str,
    weapons: Option<&(ModelFile, Vec<u8>)>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> Option<(Arc<CharacterModel>, Vec<TexAnim>)> {
    let bytes = game.install.read(&format!("{folder}/ANIM.PS2")).ok()?;
    let tree = AnimFile::parse(&bytes).ok()?.atrees.into_iter().find(|a| a.name.eq_ignore_ascii_case(name))?;
    let texmods = TexMod::parse_all(&bytes).unwrap_or_default();
    let data = CharacterData {
        name: format!("{folder}/{name}"),
        class: String::new(),
        colour: String::new(),
        clips: Arc::new(tree.clone()),
        skeleton: tree,
        model: ModelFile::parse(&game.install.read(&format!("{folder}/objects.ngc")).ok()?).ok()?,
        textures: game.install.read(&format!("{folder}/textures.ngc")).ok()?,
    };
    let mut cache = TextureCache::new(&data.model, &data.textures).sharing_materials();
    let mut model = CharacterModel::build_with(&data, &mut cache, meshes, materials, images);
    model.run_texmods(&data, &texmods, &mut cache, images);
    let drawn = cache.materials_by_binding();
    let mut shared = weapons.map(|(m, t)| (m, TextureCache::new(m, t)));
    let frames = |m: &TexMod| {
        let shared = shared.as_mut().map(|(m, c)| (*m, c));
        texanim::flipbook_images(m, &data.model, &mut cache, shared, images)
    };
    let anims = texanim::bank_anims(&texmods, &drawn, frames, materials);
    Some((Arc::new(model), anims))
}

/// The hero's effects banks: its own (`PLAYERS/<class>/SFX<colour>`), then
/// that of the class eight before it (the secret classes have none).
fn effects_banks(choice: &PlayerChoice) -> Vec<String> {
    const FIRST_EIGHT: [&str; 8] = ["WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES"];
    let colour: String = choice.variant.chars().take(3).collect::<String>().to_ascii_uppercase();
    let mut banks = vec![format!("PLAYERS/{}/SFX{colour}", choice.class)];
    if let Some(base) = character::class_index(&choice.class).map(|c| FIRST_EIGHT[c % 8])
        && base != choice.class
    {
        banks.push(format!("PLAYERS/{base}/SFX{colour}"));
    }
    banks
}

/// Every tick after the powers: each hero's familiar put on, swapped or
/// taken off by its level and hidden under the phoenix; the ATTACK on a
/// release; and on the tick after, the familiar's spit or the phoenix's
/// fireball.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn familiars(
    mut commands: Commands,
    mut shots: MessageReader<HeroShot>,
    mut pending: Local<Vec<HeroShot>>,
    (party, level): (Res<Party>, Option<Res<MonsterLevel>>),
    mut game: ResMut<LoadedGame>,
    mut models: ResMut<FamiliarModels>,
    mut heroes: Query<(Entity, &Player, &Transform, Option<&mut Familiar>)>,
    mut worn: Query<(&mut Animator, &mut Visibility), Without<Player>>,
    (mut meshes, mut materials, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<LevelMaterial>>, ResMut<Assets<Image>>),
) {
    // Last tick's releases fire now; this tick's wait for the next.
    let fire = std::mem::take(&mut *pending);
    pending.extend(shots.read().copied());
    models.step(&mut materials);
    let boss = level.as_ref().is_some_and(|l| l.boss >= 0);

    for (hero, player, _, familiar) in &mut heroes {
        let Some(member) = party.get(player.slot) else { continue };
        let (state, banks) = (&member.state, effects_banks(&member.choice));
        let phoenix = state.bits.special & PHOENIX != 0;
        let (at, _) = models.points(&member.choice.class, &mut game);
        let Some(mut familiar) = familiar else {
            commands.entity(hero).insert(Familiar::default());
            continue;
        };
        let familiar = &mut *familiar;

        // The familiar for the hero's level; put on where the class record
        // says (× 1.2 when put on at level 99), under the hero's model.
        let want = familiar_for(state.level);
        if familiar.worn.map(|(name, _)| name) != want {
            if let Some((_, e)) = familiar.worn.take() {
                commands.entity(e).try_despawn();
            }
            let model = want.and_then(|name| models.get(&banks, name, &mut game, &mut meshes, &mut materials, &mut images));
            if let (Some(name), Some(model)) = (want, model) {
                let scale = if state.level >= TOP_LEVEL { TOP_LEVEL_FAMILIAR } else { 1.0 };
                let root = model.spawn(Transform::from_translation(at * scale), &mut commands);
                commands.entity(root).insert(ChildOf(hero));
                familiar.worn = Some((name, root));
                familiar.hidden = false;
                info!("familiar {name} on at level {}", state.level);
            }
        }
        let Some((_, root)) = familiar.worn else { continue };
        let Ok((mut animator, mut visibility)) = worn.get_mut(root) else { continue };
        if familiar.hidden != phoenix {
            familiar.hidden = phoenix;
            *visibility = if phoenix { Visibility::Hidden } else { Visibility::Inherited };
        }
        // Shown, it spits as the hero lets go (ATTACK, then READY again).
        if !phoenix && pending.iter().any(|s| s.hero == hero) {
            animator.play(ATTACK);
        } else if animator.action == ATTACK && animator.finished() {
            animator.play(READY);
        }
    }

    // The shots, from where each hero is now.
    for s in fire {
        let Ok((hero, player, transform, familiar)) = heroes.get(s.hero) else { continue };
        let Some(member) = party.get(player.slot) else { continue };
        let (state, banks) = (&member.state, effects_banks(&member.choice));
        let phoenix = state.bits.special & PHOENIX != 0;
        let class = character::class_index(&member.choice.class);
        let (_, mouth) = models.points(&member.choice.class, &mut game);
        let has_familiar = familiar.is_some_and(|f| f.worn.is_some());
        if !phoenix && !has_familiar {
            continue;
        }
        let (name, folders, damage, kind) = if phoenix {
            (FIREBALL, vec![WEAPONS.to_string()], FIREBALL_DAMAGE, FIREBALL_KIND)
        } else {
            (SPIT, banks, spit_damage(state.level), 0)
        };
        let model = models.get(&folders, name, &mut game, &mut meshes, &mut materials, &mut images);
        let feet = Vec3::from(player.mover.position);
        let dwarf = class.is_some_and(|c| c % 8 == DWARF);
        let (start, velocity, gravity) =
            shot(feet, player.mover.facing, mouth, transform.scale.x, s.aim, s.targeted, player.weapon, dwarf, boss);
        projectiles::spawn_hero_missile(&mut commands, model.as_deref(), hero, start, velocity, gravity, RADIUS, damage, kind);
        info!("{name}: {damage:.1} from {start:?} at {velocity:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn familiars_by_level() {
        assert_eq!(familiar_for(29), None);
        assert_eq!(familiar_for(30), Some("FAMILIAR1"));
        assert_eq!(familiar_for(79), Some("FAMILIAR1"));
        assert_eq!(familiar_for(80), Some("FAMILIAR2"));
        assert_eq!(familiar_for(99), Some("FAMILIAR2"));
    }

    #[test]
    fn spit_grows_with_the_level() {
        assert!((spit_damage(30) - 3.0).abs() < 1e-5);
        assert!((spit_damage(80) - 8.0).abs() < 1e-5);
        assert!((spit_damage(99) - 9.9).abs() < 1e-5);
    }

    #[test]
    fn the_shot_leaves_the_mouth_and_lands_short() {
        // Facing +Z, aiming straight ahead: from (−1, 5, −1) above the feet.
        let mouth = Vec3::new(-1.0, 5.0, -1.0);
        let (start, v, g) = shot(Vec3::ZERO, 0.0, mouth, 1.0, Vec3::Z, false, 0, false, false);
        assert_eq!(start, mouth);
        assert_eq!(g, GRAVITY);
        assert!((Vec2::new(v.x, v.z).length() - SPEED).abs() < 1e-3);
        // Solved for 50 to come down 0.5 below 21 units out; at 35 it
        // comes down before that.
        let t = 21.0 / SPEED;
        let y = v.y * t - 0.5 * g * t * t;
        assert!(y < -0.5);
        // Boss levels: straight at the reach point, no fall.
        let (_, v, g) = shot(Vec3::ZERO, 0.0, mouth, 1.0, Vec3::Z, false, 0, false, true);
        assert_eq!(g, 0.0);
        assert!(v.y.abs() < 1e-4);
        // The crossbow aims along the facing whatever the target.
        let (_, v, _) = shot(Vec3::ZERO, 0.0, mouth, 1.0, Vec3::X, true, STRAIGHT, false, true);
        assert!(v.x.abs() < 1e-4 && v.z > 0.0);
    }
}
