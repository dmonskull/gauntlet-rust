//! Items the hero's blows break (`docs/mechanics.md`, "Breakables"):
//! barrels and barrel containers, exploding and poison barrels, shootable
//! (secret) walls, and hit switches.
//!
//! Each hittable item gets a [`Targetable`] entity (`TargetKind::Breakable`)
//! the combat search finds. A blow takes damage − armour (at least 1, in
//! whole points) off the item's hit points; what happens then depends on
//! the item: a container breaks open and releases what it holds (a new
//! item on the floor, with its model, that can't be picked up for 30
//! fields), a barrel breaks, an exploding barrel blasts everything near it,
//! a poison barrel lets out a cloud, a shootable wall is freed, a falling
//! obstacle (a wall that falls once shot down, a rock fall) falls
//! (`items.rs`), and a hit switch is pressed down its chain. An obstacle a blow leaves standing
//! flashes for one update (`flash.rs`).
//!
//! The blast and the poison cloud are the game's explosion effects
//! (`effects.rs`, `ExplosionAt`), owned by nobody: they grow over the
//! effect's life and hurt heroes, monsters and items (a barrel's blast
//! sets off the barrels near it).
//!
//! Blasts also reach what blows can't — chests and the powerups lying
//! about ([`BlastItem`]): an explosion (kind `0x400`) of 5 or more blows a
//! chest apart (a Death inside comes out, anything else is lost), sets a
//! CHESTEXP off, turns treasure to junk and blows other powerups to
//! pieces; poison gas (`0x800`) spoils food. A CHESTEXP, opened with a key
//! or set off, explodes once it's open (`docs/mechanics.md`, "Blows on
//! items").
//!
//! A blow's sounds are faded and panned at the item's centre raised 2
//! (`docs/audio-format.md`, "Positional sounds"): the barrels' breaking,
//! exploding and gas sounds at `0xE0`, `S_SECRETWALL` and a standing
//! obstacle's `S_WEAPONHITWOOD` at the calls' own; a CHESTEXP ticks
//! centred and explodes at its centre.
//!
//! Stand-ins: a monster inside (a Death) comes out at tier 1 straight away;
//! a shootable wall's in-between hits are silent (the level's own hit sound
//! isn't looked up); safe rocks (which break into pieces) aren't hittable;
//! the junk, spoiled food and wreck models show only where the level
//! built them (`ContentModels`).

use bevy::mesh::MeshTag;
use bevy::prelude::*;
use gdl_formats::population::{ItemClass, ItemType, PlacementParams, rotation_matrix};

use crate::audio::{CALL_VOLUME, PlaySoundAt};
use crate::combat::{Hit, TargetKind, Targetable};
use crate::effects::{EffectAt, Exploder, ExplosionAt};
use crate::flash::{self, FlashColours};
use crate::hints::{Hint, ShowHint};
use crate::items::{self, ItemTick, ItemView, LevelItems, USED};
use crate::mechanics::{self, Mechanics};
use crate::monsters::{MonsterLevel, NewMonster, spawn_monster};
use gdl_formats::enemy;
use crate::player::Player;
use crate::population::{ContentModels, ItemRig, LevelPopulation, PlacementIndex};
use crate::world::LevelEntity;

pub struct BreakablesPlugin;

impl Plugin for BreakablesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BlastItem>()
            .add_systems(
                Update,
                setup.after(items::build_items).run_if(resource_exists_and_changed::<LevelPopulation>),
            )
            .add_systems(
                FixedUpdate,
                (
                    hits.after(crate::player::PlayerTick).before(crate::damage::apply_hits),
                    flash_obstacles.after(hits),
                    // A chest set off explodes on the next tick.
                    (chest_explosions, blasted_items).chain().after(ItemTick),
                    let_out_of_chests.after(ItemTick),
                ),
            );
    }
}

/// A hittable item.
#[derive(Component)]
struct Breakable {
    placement: usize,
    hit_points: i32,
    armor: i8,
    /// Struck and still standing: an obstacle flashes on the next item
    /// update.
    flash: bool,
}

// Item type subtypes.
const BARREL: i32 = 0x2B;
const EXP_BARREL: i32 = 0x2C;
const POI_BARREL: i32 = 0x2D;
const WALL: i32 = 0x2A;
const HIT_SWITCH: i32 = 0x1F;
/// A container of this type breaks open when its hit points run out.
const BREAKS_OPEN: u16 = 0x200;
/// Blows of this kind do no damage.
const NO_DAMAGE: u32 = 0x800;
/// Fields before released contents can be picked up.
/// What a safe rock leaves as it breaks (the effect table's `0x1E`).
const ROCK_BROKEN_FX: &str = "GENDEST";
const RELEASE_DELAY: i32 = 30;
/// Blasts' damage (× the level's hazard scale): an exploding barrel's
/// fireball, a poison barrel's cloud, a CHESTEXP's (`effects.rs` has their
/// reach).
const EXPLOSION_DAMAGE: f32 = 30.0;
const POISON_DAMAGE: f32 = 10.0;
const CHEST_EXPLOSION_DAMAGE: f32 = 50.0;

// Blow kinds.
/// Magic (a potion's blast).
const MAGIC: u32 = 0x200;
/// An explosion (a barrel's, a CHESTEXP's, a suicide runner's).
const EXPLOSIVE: u32 = 0x400;
/// An explosive blow this strong blows chests and powerups apart.
const BLOWS_APART: f32 = 5.0;
/// Poison gas stronger than this spoils food.
const SPOILS: f32 = 2.0;

// Powerup subtypes.
const TREASURE: i32 = 1;
const KEY: i32 = 2;
const FOOD: i32 = 3;
const POTION: i32 = 4;
/// Runestones, the boss's key, the obelisk, legendary items, scrolls,
/// gems and gargoyle pieces: nothing blows them up.
const QUEST_PIECES: std::ops::RangeInclusive<i32> = 10..=16;
// Container subtypes (`BARREL` above is the barrel container too).
const SILVER_CHEST: i32 = 0x30;

/// Treasure blown to junk is worth this.
const JUNK_AMOUNT: i32 = 10;
/// Spoiled food: meat (2 hit points) turns bad, fruit to a green apple.
const BAD_MEAT: (&str, i32) = ("BADMEAT", -100);
const BAD_FRUIT: (&str, i32) = ("GAPPLE", -50);
/// Models left where things were blown apart (the realm's items bank),
/// and treasure's junk (`POWERUPS`).
const JUNK_MODEL: &str = "TREAS_JUNK";
const ITEM_WRECK: &str = "ITEMEXP0";
const SILVER_CHEST_WRECK: &str = "CHESTSEXP0";
const CHEST_WRECK: &str = "CHESTGEXP0";
/// The effects played where something is blown apart (`WEAPONS`): the
/// pieces and the smoke.
const PIECES_FX: &str = "CHESTDEST";
const SMOKE_FX: &str = "DESTSMOKE";
/// Monster type of Death.
const DEATH: i32 = 0x1E;
/// A blow on a standing barrel or obstacle.
const WOOD_HIT: &str = "S_WEAPONHITWOOD";

/// A blow's sounds play this far above the item's centre; the barrels'
/// at this requested volume.
const BLOW_SOUND_RISE: f32 = 2.0;
const BARREL_VOLUME: u8 = 0xE0;
/// A CHESTEXP's tick and explosion's requested volume.
const CHEST_EXP_VOLUME: u8 = 0xE0;

/// Barrel sounds by realm id (A–K = 1–11): breaking, exploding, gas.
fn barrel_sound(kind: &str, realm: usize) -> Option<String> {
    let letter = (b'A' + realm.checked_sub(1)? as u8) as char;
    (realm <= 11 && !matches!(letter, 'E' | 'F')).then(|| format!("S_BARREL_{kind}{letter}"))
}

fn setup(mut commands: Commands, items: Res<LevelItems>) {
    let mut count = 0;
    for view in items.views() {
        let class = view.ty.class;
        let subtype = view.ty.subtype;
        let hittable = view.live
            && view.ty.armor != -1
            && match class {
                ItemClass::Container => (BARREL..=POI_BARREL).contains(&subtype) && view.state < 1,
                ItemClass::Obstacle => subtype != items::SAFE_ROCK && !((BARREL..=POI_BARREL).contains(&subtype) && view.state >= 1),
                ItemClass::Trigger => subtype == HIT_SWITCH,
                _ => false,
            };
        if !hittable {
            continue;
        }
        let at = Vec3::from(view.shape.centre);
        debug!(
            "breakable {} {} ({class:?} {subtype:#x}) at {at}, {} hp, armour {}, holds {:?}",
            view.placement,
            view.ty.name,
            view.ty.hit_points,
            view.ty.armor,
            view.contents.map(|c| c.name.as_str())
        );
        commands.spawn((
            Transform::from_translation(at),
            Targetable::new(TargetKind::Breakable, view.shape.radius, view.ty.extent[1].max(0.5)),
            Breakable { placement: view.placement, hit_points: i32::from(view.ty.hit_points).max(1), armor: view.ty.armor, flash: false },
            LevelEntity,
        ));
        count += 1;
    }
    info!("breakables: {count}");
}

#[allow(clippy::too_many_arguments)]
fn hits(
    mut commands: Commands,
    mut messages: MessageReader<Hit>,
    mut breakables: Query<&mut Breakable>,
    items: Option<ResMut<LevelItems>>,
    contents: Option<Res<ContentModels>>,
    mut mechanics: Option<ResMut<Mechanics>>,
    mut level: Option<ResMut<MonsterLevel>>,
    players: Query<(), With<Player>>,
    mut explosions: MessageWriter<ExplosionAt>,
    (mut sounds, mut effects): (MessageWriter<PlaySoundAt>, MessageWriter<EffectAt>),
    mut hints: MessageWriter<ShowHint>,
    transforms: Query<&Transform>,
) {
    let Some(mut items) = items else { return };
    let incoming: Vec<Hit> = messages.read().filter(|h| h.target_kind == TargetKind::Breakable).cloned().collect();
    let realm = items.realm();
    let hazard_scale = level.as_ref().map_or(1.0, |l| l.tuning.hazard_damage);
    for hit in incoming {
        let Ok(mut b) = breakables.get_mut(hit.target) else { continue };
        if b.hit_points <= 0 {
            continue;
        }
        let Some(view) = items.view(b.placement) else { continue };
        let (class, subtype, flags, centre, pos) = (view.ty.class, view.ty.subtype, view.flags, view.shape.centre, view.shape.centre);
        let inside = view.contents.cloned();
        let nameless = view.ty.name.is_empty();
        let keys = match view.params {
            PlacementParams::Container { param, .. } => Some(i32::from(*param)),
            _ => None,
        };
        // The blow after armour (what the explosive test measures).
        let mut dealt = hit.damage;
        if hit.kind & NO_DAMAGE == 0 {
            if b.armor >= 0 {
                dealt -= f32::from(b.armor);
                if dealt <= 0.0 {
                    dealt = 1.0;
                }
            }
            b.hit_points = (b.hit_points - (dealt + 0.5) as i32).max(0);
        }
        let dead = b.hit_points == 0;
        // Where its sounds are.
        let blow_at = Vec3::from(centre) + Vec3::Y * BLOW_SOUND_RISE;
        // A hero's blow that does damage to a secret wall (a nameless
        // obstacle) tells of them.
        if class == ItemClass::Obstacle && nameless && hit.kind & NO_DAMAGE == 0 && !hit.ranged && players.contains(hit.attacker) {
            hints.write(ShowHint(Hint::SecretWalls));
        }
        // An obstacle a blow did damage and left standing flashes.
        if class == ItemClass::Obstacle && hit.kind & NO_DAMAGE == 0 && !dead && subtype != items::SAFE_ROCK {
            b.flash = true;
        }
        debug!("breakable {} ({class:?} {subtype:#x}) hit for {:.1}: {} left", b.placement, hit.damage, b.hit_points);
        let mut remove = dead;
        match class {
            ItemClass::Container => {
                let breaking_open = dead && flags & BREAKS_OPEN != 0;
                let death_inside = inside.as_ref().is_some_and(|t| t.enemy() == Some(DEATH));
                let blow = container_blow(subtype, hit.kind, dealt, breaking_open, death_inside);
                if blow != ItemBlow::Nothing {
                    let pose = items.view(b.placement).map_or_else(Transform::default, |v| item_pose(&v, &transforms));
                    let models = contents.as_deref();
                    let writers = (&mut effects, &mut sounds, &mut hints);
                    apply_blow(blow, b.placement, pose, &mut items, models, level.as_deref_mut(), &mut commands, writers);
                    remove = true;
                } else if breaking_open && flags & USED == 0 {
                    items.set_flags(b.placement, USED);
                    if subtype == BARREL {
                        if let Some(s) = barrel_sound("WOOD", realm) {
                            sounds.write(PlaySoundAt::faded(s, blow_at, BARREL_VOLUME));
                        }
                        hints.write(ShowHint(Hint::SomeBarrels));
                    }
                    // A monster inside (a Death) comes out where the container stood.
                    if let Some(ty) = inside.as_ref() {
                        let_out_monster(ty, pos, level.as_deref_mut(), &mut commands);
                    }
                    if let Some(ty) = inside.filter(|t| t.class != ItemClass::EnemyInfo) {
                        // Keys come as many as the container says.
                        let amount = (ty.class == ItemClass::Powerup && ty.subtype == 2).then(|| keys.unwrap_or(1).max(1));
                        let at = [pos[0], pos[1] - 1.0, pos[2]];
                        let name = ty.name.clone();
                        let placement = items.release(ty, at, rotation_matrix([0.0; 3]), amount, RELEASE_DELAY);
                        if let Some(models) = contents.as_ref() {
                            models.spawn(&name, Transform::from_translation(Vec3::from(at)), placement, &mut commands);
                        }
                        info!("breakable {} released {name}", b.placement);
                    }
                }
            }
            ItemClass::Obstacle => match subtype {
                WALL => {
                    if dead {
                        sounds.write(PlaySoundAt::faded("S_SECRETWALL", blow_at, CALL_VOLUME));
                        items.free(b.placement, &mut commands);
                        info!("secret wall {} broken", b.placement);
                    }
                }
                EXP_BARREL | POI_BARREL => {
                    if dead {
                        items.set_flags(b.placement, USED);
                        let poison = subtype == POI_BARREL;
                        let (kind, damage) = if poison { ("GAS", POISON_DAMAGE) } else { ("EXPLO", EXPLOSION_DAMAGE) };
                        if let Some(s) = barrel_sound(kind, realm) {
                            sounds.write(PlaySoundAt::faded(s, blow_at, BARREL_VOLUME));
                        }
                        // The game's explosion effect, owned by nobody:
                        // it hurts heroes, monsters and items (other
                        // barrels go up with it).
                        explosions.write(ExplosionAt {
                            owner: Entity::PLACEHOLDER,
                            at: Vec3::from(centre),
                            damage: damage * hazard_scale,
                            poison,
                            folder: None,
                            by: Exploder::Barrel,
                        });
                        info!("barrel {} {}", b.placement, if subtype == EXP_BARREL { "explodes" } else { "lets out gas" });
                    } else {
                        sounds.write(PlaySoundAt::faded(WOOD_HIT, blow_at, CALL_VOLUME));
                    }
                }
                _ => {
                    if dead {
                        items.set_flags(b.placement, USED);
                        if let Some(s) = barrel_sound("WOOD", realm) {
                            sounds.write(PlaySoundAt::faded(s, blow_at, BARREL_VOLUME));
                        }
                    } else {
                        sounds.write(PlaySoundAt::faded(WOOD_HIT, blow_at, CALL_VOLUME));
                    }
                }
            },
            ItemClass::Trigger => {
                if let Some(m) = mechanics.as_deref_mut() {
                    mechanics::hit_switch(m, b.placement);
                }
                remove = false;
                b.hit_points = i32::from(view_hit_points(&items, b.placement)).max(1);
            }
            _ => {}
        }
        if remove {
            commands.entity(hit.target).try_despawn();
        }
    }
}

/// A blast reached an item no blow can: a powerup lying about or a chest,
/// whose armour is below 0 (the blasts' item test, `effects.rs`, with
/// [`blast_reaches`] choosing them). Potions go off their own way and the
/// breakable [`Targetable`]s get [`Hit`]s.
#[derive(Message, Clone, Copy, Debug)]
pub struct BlastItem {
    pub placement: usize,
    /// The blast's kind bits and its damage there.
    pub kind: u32,
    pub damage: f32,
}

/// Whether a blast of `kind` reaches this item as a [`BlastItem`] — the
/// game's test of an effect against an item, for the items with armour
/// below 0: a live powerup (not taken) or container. Those blows don't
/// hurt (armour −1: chests, treasure, keys, powerups, quest pieces) take
/// only magic or an explosion; food (−2) takes any. (The game also lets
/// some effects pass powerups or barrels by their own flags; the blasts
/// leave those out.)
pub fn blast_reaches(v: &ItemView, kind: u32) -> bool {
    // A standing safe rock takes any blast but magic (a broken one none).
    if v.rock {
        return v.live && v.armor >= 0 && kind & MAGIC == 0;
    }
    // A critter's statue: any blast wakes it.
    if v.statue {
        return v.live;
    }
    if !v.live || v.ty.armor >= 0 {
        return false;
    }
    match v.ty.class {
        ItemClass::Powerup | ItemClass::Container => v.ty.armor != -1 || kind & (MAGIC | EXPLOSIVE) != 0,
        _ => false,
    }
}

/// What a blow does to a powerup or container, beyond its hit points (the
/// game's item damage routine, `docs/mechanics.md`, "Blows on items").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemBlow {
    Nothing,
    /// Treasure blown to junk (worth 10).
    Junk,
    /// A powerup blown to pieces.
    Destroyed,
    /// Food spoiled by gas: meat to bad meat, fruit to a green apple.
    Spoiled { meat: bool },
    /// A chest blown apart (the silver chest has its own wreck).
    ChestBlown { silver: bool },
    /// A CHESTEXP set off: open at once, it explodes on the next update.
    ChestArmed,
}

fn explosive(kind: u32, damage: f32) -> bool {
    kind & EXPLOSIVE != 0 && damage >= BLOWS_APART
}

/// A blow of `kind` and `damage` (after armour) on a powerup of `subtype`
/// with `hit_points` (food: 2 for meat, 1 for fruit).
fn powerup_blow(subtype: i32, hit_points: i16, kind: u32, damage: f32) -> ItemBlow {
    match subtype {
        // Keys and quest pieces stand; potions go off their own way.
        KEY | POTION => ItemBlow::Nothing,
        s if QUEST_PIECES.contains(&s) => ItemBlow::Nothing,
        TREASURE if explosive(kind, damage) => ItemBlow::Junk,
        TREASURE => ItemBlow::Nothing,
        FOOD if kind & NO_DAMAGE != 0 && damage > SPOILS => ItemBlow::Spoiled { meat: hit_points == 2 },
        _ if kind & NO_DAMAGE == 0 && explosive(kind, damage) => ItemBlow::Destroyed,
        _ => ItemBlow::Nothing,
    }
}

/// A blow on a container of `subtype`: one whose hit points ran out and
/// that breaks open (a barrel) does that instead; magic on a chest with a
/// Death inside kills the Death (not done here).
fn container_blow(subtype: i32, kind: u32, damage: f32, breaking_open: bool, death_inside: bool) -> ItemBlow {
    if kind & MAGIC != 0 && death_inside && subtype != BARREL {
        return ItemBlow::Nothing;
    }
    if breaking_open || !explosive(kind, damage) {
        return ItemBlow::Nothing;
    }
    if subtype == items::CHEST_EXP { ItemBlow::ChestArmed } else { ItemBlow::ChestBlown { silver: subtype == SILVER_CHEST } }
}

/// The item's pose: its model's, else standing at its centre.
fn item_pose(v: &ItemView, transforms: &Query<&Transform>) -> Transform {
    v.model
        .and_then(|m| transforms.get(m).ok())
        .copied()
        .unwrap_or_else(|| Transform::from_translation(Vec3::from(v.shape.centre)))
}

/// The pieces and the smoke where something was blown apart.
fn pieces(at: Vec3, effects: &mut MessageWriter<EffectAt>) {
    for name in [PIECES_FX, SMOKE_FX] {
        effects.write(EffectAt { name, bank: None, at, facing: 0.0, scale: 1.0 });
    }
}

/// Carries a blow out on the item numbered `placement`, standing at `pose`.
#[allow(clippy::too_many_arguments)]
fn apply_blow(
    blow: ItemBlow,
    placement: usize,
    pose: Transform,
    items: &mut LevelItems,
    models: Option<&ContentModels>,
    level: Option<&mut MonsterLevel>,
    commands: &mut Commands,
    (effects, sounds, hints): (&mut MessageWriter<EffectAt>, &mut MessageWriter<PlaySoundAt>, &mut MessageWriter<ShowHint>),
) {
    let Some(v) = items.view(placement) else { return };
    let (name, centre) = (v.ty.name.clone(), v.shape.centre);
    let death = v.contents.filter(|t| t.enemy() == Some(DEATH)).cloned();
    let open_action = v.actions.saturating_sub(1).min(2);
    info!("{name} ({placement}): {blow:?}");
    match blow {
        ItemBlow::Nothing => {}
        ItemBlow::Junk => {
            pieces(pose.translation, effects);
            swap_model(items, placement, JUNK_MODEL, pose, models, commands);
            items.set_amount(placement, JUNK_AMOUNT);
        }
        ItemBlow::Destroyed => {
            pieces(pose.translation, effects);
            wreck(items, placement, ITEM_WRECK, pose, models, commands);
            hints.write(ShowHint(Hint::ExplosionsDestroyItems));
        }
        ItemBlow::Spoiled { meat } => {
            let (model, amount) = if meat { BAD_MEAT } else { BAD_FRUIT };
            swap_model(items, placement, model, pose, models, commands);
            items.set_amount(placement, amount);
            hints.write(ShowHint(Hint::GasSpoilsFood));
        }
        ItemBlow::ChestBlown { silver } => {
            // A Death inside comes out; anything else is lost with it.
            if let Some(ty) = death {
                let_out_monster(&ty, centre, level, commands);
            }
            pieces(pose.translation, effects);
            wreck(items, placement, if silver { SILVER_CHEST_WRECK } else { CHEST_WRECK }, pose, models, commands);
        }
        ItemBlow::ChestArmed => {
            // Open at once and ticking: it goes off on the next update.
            items.set_flags(placement, USED | items::ALWAYS_ACTIVE);
            items.play(placement, open_action);
            items.set_state(placement, 2);
            sounds.write(PlaySoundAt::centred(items::CHEST_EXP_TICK, CHEST_EXP_VOLUME));
        }
    }
}

/// The item's model gives way to `name`'s (only if the level built one).
fn swap_model(
    items: &mut LevelItems,
    placement: usize,
    name: &str,
    pose: Transform,
    models: Option<&ContentModels>,
    commands: &mut Commands,
) {
    let Some(models) = models else { return };
    if models.spawn(name, pose, placement, commands).is_none() {
        debug!("no {name} model in this level");
        return;
    }
    if let Some(old) = items.take_model(placement) {
        commands.entity(old).try_despawn();
    }
}

/// The item is freed and `name`'s model left where it stood (bound to the
/// freed item, so nothing moves it).
fn wreck(
    items: &mut LevelItems,
    placement: usize,
    name: &str,
    pose: Transform,
    models: Option<&ContentModels>,
    commands: &mut Commands,
) {
    items.free(placement, commands);
    if let Some(models) = models
        && models.spawn(name, pose, placement, commands).is_none()
    {
        debug!("no {name} model in this level");
    }
}

/// A monster that was inside a container (a Death) comes out where the
/// container stood.
fn let_out_monster(ty: &ItemType, pos: [f32; 3], level: Option<&mut MonsterLevel>, commands: &mut Commands) {
    let (Some(id), Some(level)) = (ty.enemy().filter(|_| ty.class == ItemClass::EnemyInfo), level) else { return };
    let ai = enemy::enemy_stats(id).map_or(7, |s| s.default_ai);
    let new = NewMonster {
        enemy: id,
        tier: 1,
        ai,
        position: [pos[0], pos[1] - 1.0, pos[2]],
        facing: 0.0,
        generator: None,
        placed: true,
        awareness: None,
        freeze: 0.0,
        throw_rate: 1.0,
    };
    if spawn_monster(level, new, commands).is_some() {
        info!("a container lets out monster {id:#x}");
    }
}

/// Monsters a key-opened chest lets out (`items.rs`) come out where it
/// stood.
fn let_out_of_chests(mut commands: Commands, items: Option<ResMut<LevelItems>>, mut level: Option<ResMut<MonsterLevel>>) {
    let Some(mut items) = items else { return };
    for (ty, at) in items.take_let_out() {
        let_out_monster(&ty, at, level.as_deref_mut(), &mut commands);
    }
}

/// Blasts on chests and on powerups lying about.
fn blasted_items(
    mut commands: Commands,
    mut blasts: MessageReader<BlastItem>,
    items: Option<ResMut<LevelItems>>,
    contents: Option<Res<ContentModels>>,
    mut level: Option<ResMut<MonsterLevel>>,
    transforms: Query<&Transform>,
    (mut effects, mut sounds, mut hints): (MessageWriter<EffectAt>, MessageWriter<PlaySoundAt>, MessageWriter<ShowHint>),
) {
    let Some(mut items) = items else {
        blasts.clear();
        return;
    };
    for b in blasts.read() {
        // A blow on a critter's statue wakes it.
        if items.view(b.placement).is_some_and(|v| v.statue) {
            items.strike_statue(b.placement);
            continue;
        }
        // A safe rock takes the blow and shows its new stage; broken, it
        // leaves GENDEST (the game's effect `0x1E`).
        if let Some(v) = items.view(b.placement).filter(|v| v.rock) {
            let (pose, name) = (item_pose(&v, &transforms), v.ty.name.clone());
            if let Some(stage) = items.hit_rock(b.placement, b.damage) {
                info!("safe rock {} is now stage {stage}", b.placement);
                swap_model(&mut items, b.placement, &format!("{name}{stage}"), pose, contents.as_deref(), &mut commands);
                if stage == 0 {
                    effects.write(EffectAt { name: ROCK_BROKEN_FX, bank: None, at: pose.translation, facing: 0.0, scale: 1.0 });
                }
            }
            continue;
        }
        let Some(v) = items.view(b.placement).filter(|v| blast_reaches(v, b.kind)) else { continue };
        let blow = match v.ty.class {
            ItemClass::Powerup => powerup_blow(v.ty.subtype, v.ty.hit_points, b.kind, b.damage),
            _ => {
                let death_inside = v.contents.is_some_and(|t| t.enemy() == Some(DEATH));
                container_blow(v.ty.subtype, b.kind, b.damage, false, death_inside)
            }
        };
        if blow == ItemBlow::Nothing {
            continue;
        }
        let pose = item_pose(&v, &transforms);
        let models = contents.as_deref();
        let writers = (&mut effects, &mut sounds, &mut hints);
        apply_blow(blow, b.placement, pose, &mut items, models, level.as_deref_mut(), &mut commands, writers);
    }
}

/// A CHESTEXP that is open (its animation has reached state 2) explodes
/// where it stands: the game's effect `0x1D` (50 × the level's hazard
/// scale; `EXPCHEST` turned as the chest), the realm's barrel explosion
/// sound and hint `0x89`, and the chest is freed.
fn chest_explosions(
    mut commands: Commands,
    items: Option<ResMut<LevelItems>>,
    level: Option<Res<MonsterLevel>>,
    transforms: Query<&Transform>,
    mut explosions: MessageWriter<ExplosionAt>,
    (mut sounds, mut hints): (MessageWriter<PlaySoundAt>, MessageWriter<ShowHint>),
) {
    let Some(mut items) = items else { return };
    let open: Vec<(usize, Transform, [f32; 3])> = items
        .views()
        .filter(|v| v.live && v.ty.class == ItemClass::Container && v.ty.subtype == items::CHEST_EXP && v.state >= 2)
        .map(|v| (v.placement, item_pose(&v, &transforms), v.shape.centre))
        .collect();
    let hazard_scale = level.as_ref().map_or(1.0, |l| l.tuning.hazard_damage);
    for (placement, pose, centre) in open {
        let (facing, _, _) = pose.rotation.to_euler(EulerRot::YXZ);
        explosions.write(ExplosionAt {
            owner: Entity::PLACEHOLDER,
            at: pose.translation,
            damage: CHEST_EXPLOSION_DAMAGE * hazard_scale,
            poison: false,
            folder: None,
            by: Exploder::Chest { facing },
        });
        if let Some(s) = barrel_sound("EXPLO", items.realm()) {
            sounds.write(PlaySoundAt::faded(s, Vec3::from(centre), CHEST_EXP_VOLUME));
        }
        items.free(placement, &mut commands);
        hints.write(ShowHint(Hint::ChestsExplode));
        info!("CHESTEXP {placement} explodes");
    }
}

/// An obstacle's hit flash (the game's item flash count of 1): for one
/// item update its root object shows the level's `AAAWHITE` in place of
/// its own texture, and its whole model skips the lightmap (`flash.rs`).
/// Levels without one (levelT4) don't flash their obstacles.
#[allow(clippy::too_many_arguments)]
fn flash_obstacles(
    mut commands: Commands,
    mut breakables: Query<&mut Breakable>,
    models: Query<(Entity, &PlacementIndex, Option<&ItemRig>)>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    mut tags: Query<&mut MeshTag>,
    colours: Res<FlashColours>,
    mut lit: Local<Vec<Entity>>,
) {
    for root in lit.drain(..) {
        for e in children.iter_descendants(root) {
            if meshes.contains(e) {
                flash::set_tag(e, 0, &mut tags, &mut commands);
            }
        }
    }
    for mut b in &mut breakables {
        if !std::mem::take(&mut b.flash) {
            continue;
        }
        let Some(colour) = colours.level else { continue };
        let Some((root, _, rig)) = models.iter().find(|(_, p, _)| p.0 == b.placement) else { continue };
        // The root object: node 0 (or its flipbook's frame), or the whole
        // of a model without an atree.
        let own: Vec<Entity> = match rig {
            Some(rig) => {
                let holder = rig.flipbooks.iter().find(|(_, node, ..)| *node == 0).map(|(h, ..)| *h);
                rig.bones.first().into_iter().copied().chain(holder).collect()
            }
            None => vec![root],
        };
        for e in children.iter_descendants(root) {
            if !meshes.contains(e) {
                continue;
            }
            let is_own = own.iter().any(|&o| children.get(o).is_ok_and(|c| c.contains(&e)));
            let tag = if is_own { colour | flash::TAG_REPLACE | flash::TAG_NO_LIGHTMAP } else { flash::TAG_NO_LIGHTMAP };
            flash::set_tag(e, tag, &mut tags, &mut commands);
        }
        lit.push(root);
    }
}

fn view_hit_points(items: &LevelItems, placement: usize) -> i16 {
    items.view(placement).map_or(1, |v| v.ty.hit_points)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Shape;
    use gdl_formats::population::Population;

    const BARREL_BLAST: u32 = 0x421;
    const GAS: u32 = 0x800;

    #[test]
    fn explosions_blow_powerups_apart() {
        // Treasure turns to junk; keys, potions and quest pieces stand.
        assert_eq!(powerup_blow(TREASURE, 0, BARREL_BLAST, 5.0), ItemBlow::Junk);
        assert_eq!(powerup_blow(TREASURE, 0, BARREL_BLAST, 4.9), ItemBlow::Nothing);
        assert_eq!(powerup_blow(KEY, 0, BARREL_BLAST, 30.0), ItemBlow::Nothing);
        assert_eq!(powerup_blow(POTION, 1, BARREL_BLAST, 30.0), ItemBlow::Nothing);
        assert_eq!(powerup_blow(10, 0, BARREL_BLAST, 30.0), ItemBlow::Nothing);
        assert_eq!(powerup_blow(16, 0, BARREL_BLAST, 30.0), ItemBlow::Nothing);
        // Food and timed powerups go; magic alone does nothing.
        assert_eq!(powerup_blow(FOOD, 2, BARREL_BLAST, 5.0), ItemBlow::Destroyed);
        assert_eq!(powerup_blow(9, 0, BARREL_BLAST, 20.0), ItemBlow::Destroyed);
        assert_eq!(powerup_blow(9, 0, MAGIC | 1, 20.0), ItemBlow::Nothing);
    }

    #[test]
    fn gas_spoils_food() {
        assert_eq!(powerup_blow(FOOD, 2, GAS, 2.5), ItemBlow::Spoiled { meat: true });
        assert_eq!(powerup_blow(FOOD, 1, GAS, 2.5), ItemBlow::Spoiled { meat: false });
        assert_eq!(powerup_blow(FOOD, 1, GAS, 2.0), ItemBlow::Nothing);
        // Gas does nothing else.
        assert_eq!(powerup_blow(TREASURE, 0, GAS, 10.0), ItemBlow::Nothing);
        assert_eq!(powerup_blow(9, 0, GAS, 10.0), ItemBlow::Nothing);
    }

    #[test]
    fn explosions_blow_chests_apart() {
        assert_eq!(container_blow(0x2E, BARREL_BLAST, 5.0, false, false), ItemBlow::ChestBlown { silver: false });
        assert_eq!(container_blow(SILVER_CHEST, BARREL_BLAST, 5.0, false, false), ItemBlow::ChestBlown { silver: true });
        assert_eq!(container_blow(items::CHEST_EXP, BARREL_BLAST, 5.0, false, false), ItemBlow::ChestArmed);
        assert_eq!(container_blow(0x2E, BARREL_BLAST, 4.0, false, false), ItemBlow::Nothing);
        assert_eq!(container_blow(0x2E, 0x21, 50.0, false, false), ItemBlow::Nothing);
        // A barrel whose hit points ran out breaks open instead.
        assert_eq!(container_blow(BARREL, BARREL_BLAST, 20.0, true, false), ItemBlow::Nothing);
        // Magic with a Death inside is the Death's business.
        assert_eq!(container_blow(0x2E, MAGIC | EXPLOSIVE, 20.0, false, true), ItemBlow::Nothing);
        assert_eq!(container_blow(0x2E, BARREL_BLAST, 20.0, false, true), ItemBlow::ChestBlown { silver: false });
    }

    fn reaches(ty: &ItemType, kind: u32, live: bool) -> bool {
        let params = PlacementParams::None;
        let v = ItemView {
            placement: 0,
            ty,
            params: &params,
            shape: Shape { kind: 1, radius: 1.0, reach: 1.0, half: [0.0; 2], centre: [0.0; 3], axes: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] },
            flags: 0,
            state: 0,
            action: 0,
            done: false,
            actions: 0,
            live,
            armor: ty.armor,
            rock: false,
            statue: false,
            contents: None,
            model: None,
            floor_node: None,
        };
        blast_reaches(&v, kind)
    }

    fn item_type(class: ItemClass, subtype: i32, armor: i8) -> ItemType {
        ItemType {
            class,
            subtype,
            name: String::new(),
            choices: Vec::new(),
            extent: [1.0; 4],
            center_offset: [0.0; 3],
            value: 0,
            amount: 0,
            armor,
            hit_points: 0,
            flags: 0,
            duration: 0,
            raw: [0; 0x50],
        }
    }

    #[test]
    fn what_blasts_reach() {
        let chest = item_type(ItemClass::Container, 0x2E, -1);
        let treasure = item_type(ItemClass::Powerup, TREASURE, -1);
        let food = item_type(ItemClass::Powerup, FOOD, -2);
        // Chests and treasure: explosions and magic only; food: any blast.
        assert!(reaches(&chest, BARREL_BLAST, true) && !reaches(&chest, GAS, true));
        assert!(reaches(&treasure, BARREL_BLAST, true) && !reaches(&treasure, GAS, true));
        assert!(reaches(&food, BARREL_BLAST, true) && reaches(&food, GAS, true));
        // Magic reaches gold, but does nothing to it.
        assert!(reaches(&treasure, MAGIC | 1, true));
        assert_eq!(powerup_blow(TREASURE, 0, MAGIC | 1, 40.0), ItemBlow::Nothing);
        // Potions and barrels go their own ways; obstacles aren't these.
        assert!(!reaches(&item_type(ItemClass::Powerup, POTION, 0), BARREL_BLAST, true));
        assert!(!reaches(&item_type(ItemClass::Container, BARREL, 1), BARREL_BLAST, true));
        assert!(!reaches(&item_type(ItemClass::Obstacle, 0x2E, -1), BARREL_BLAST, true));
        // Taken or freed items aren't there.
        assert!(!reaches(&chest, BARREL_BLAST, false));
    }

    /// On the disc: chests, treasure and powerups take no blows (armour
    /// −1), food takes blasts only (−2) with 1 or 2 hit points (fruit or
    /// meat), and the spoiled and junk types exist (real data).
    #[test]
    fn item_armour_on_the_disc() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(dirs) = std::fs::read_dir(std::path::Path::new(&root).join("LEVELS")) else {
            eprintln!("skipping: no LEVELS");
            return;
        };
        let mut levels = 0;
        let mut names = std::collections::BTreeSet::new();
        for dir in dirs.flatten() {
            let Ok(bytes) = std::fs::read(dir.path().join("WORLDS.PS2")) else { continue };
            let Ok(pop) = Population::parse(&bytes) else { continue };
            levels += 1;
            for ty in &pop.item_types {
                names.insert(ty.name.clone());
                match (ty.class, ty.subtype) {
                    (ItemClass::Container, s) if s != BARREL => assert_eq!(ty.armor, -1, "{}", ty.name),
                    (ItemClass::Powerup, FOOD) => {
                        assert_eq!(ty.armor, -2, "{}", ty.name);
                        assert!(matches!(ty.hit_points, 1 | 2), "{}", ty.name);
                    }
                    (ItemClass::Powerup, POTION) => assert!(ty.armor >= 0, "{}", ty.name),
                    (ItemClass::Powerup, _) => assert_eq!(ty.armor, -1, "{}", ty.name),
                    _ => {}
                }
            }
        }
        if levels == 0 {
            eprintln!("skipping: no levels");
            return;
        }
        for name in [BAD_MEAT.0, BAD_FRUIT.0, JUNK_MODEL] {
            assert!(names.contains(name), "{name}");
        }
    }
}
