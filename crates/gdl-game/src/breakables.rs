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
//! a poison barrel lets out a cloud, a shootable wall is freed, and a hit
//! switch is pressed down its chain.
//!
//! Stand-ins: the blast and the poison cloud hurt once, at once, with the
//! missiles' blast falloff (the game's effects, which carry them, aren't
//! ported); hit flashes, hints 0x14 / 0x1B and chests blown apart by
//! explosive blows aren't done; a shootable wall's in-between hits are
//! silent (the level's own hit sound isn't looked up); safe rocks (which
//! break into pieces) aren't hittable.

use bevy::prelude::*;
use gdl_formats::population::{ItemClass, PlacementParams, rotation_matrix};

use crate::audio::PlaySound;
use crate::combat::{Hit, TargetKind, Targetable};
use crate::damage::after_armor;
use crate::items::{self, LevelItems, USED};
use crate::mechanics::{self, Mechanics};
use crate::monsters::{Monster, MonsterLevel};
use crate::player::Player;
use crate::player_state::DamagePlayer;
use crate::population::{ContentModels, LevelPopulation};
use crate::projectiles::blast_share;
use crate::world::LevelEntity;

pub struct BreakablesPlugin;

impl Plugin for BreakablesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            setup.after(items::build_items).run_if(resource_exists_and_changed::<LevelPopulation>),
        )
        .add_systems(FixedUpdate, hits.after(crate::player::PlayerTick).before(crate::damage::apply_hits));
    }
}

/// A hittable item.
#[derive(Component)]
struct Breakable {
    placement: usize,
    hit_points: i32,
    armor: i8,
}

// Item type subtypes.
const BARREL: i32 = 0x2B;
const EXP_BARREL: i32 = 0x2C;
const POI_BARREL: i32 = 0x2D;
const SAFE_ROCK: i32 = 0x29;
const WALL: i32 = 0x2A;
const HIT_SWITCH: i32 = 0x1F;
/// A container of this type breaks open when its hit points run out.
const BREAKS_OPEN: u16 = 0x200;
/// Blows of this kind do no damage.
const NO_DAMAGE: u32 = 0x800;
/// Fields before released contents can be picked up.
const RELEASE_DELAY: i32 = 30;
/// Blasts: damage (× the level's hazard scale) and radius.
const EXPLOSION: (f32, f32) = (30.0, 6.0);
const POISON: (f32, f32) = (10.0, 6.5);

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
                ItemClass::Obstacle => subtype != SAFE_ROCK && !((BARREL..=POI_BARREL).contains(&subtype) && view.state >= 1),
                ItemClass::Trigger => subtype == HIT_SWITCH,
                _ => false,
            };
        if !hittable {
            continue;
        }
        let at = Vec3::from(view.shape.centre);
        debug!("breakable {} {} ({class:?} {subtype:#x}) at {at}, {} hp, armour {}", view.placement, view.ty.name, view.ty.hit_points, view.ty.armor);
        commands.spawn((
            Transform::from_translation(at),
            Targetable::new(TargetKind::Breakable, view.shape.radius, view.ty.extent[1].max(0.5)),
            Breakable { placement: view.placement, hit_points: i32::from(view.ty.hit_points).max(1), armor: view.ty.armor },
            LevelEntity,
        ));
        count += 1;
    }
    info!("breakables: {count}");
}

#[allow(clippy::too_many_arguments)]
fn hits(
    mut commands: Commands,
    mut messages: ParamSet<(MessageReader<Hit>, MessageWriter<Hit>)>,
    mut breakables: Query<&mut Breakable>,
    items: Option<ResMut<LevelItems>>,
    contents: Option<Res<ContentModels>>,
    mut mechanics: Option<ResMut<Mechanics>>,
    level: Option<Res<MonsterLevel>>,
    monsters: Query<(Entity, &Monster)>,
    mut players: Query<(Entity, &mut Player)>,
    mut hurt: MessageWriter<DamagePlayer>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let Some(mut items) = items else { return };
    let incoming: Vec<Hit> = messages.p0().read().filter(|h| h.target_kind == TargetKind::Breakable).cloned().collect();
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
        let keys = match view.params {
            PlacementParams::Container { param, .. } => Some(i32::from(*param)),
            _ => None,
        };
        if hit.kind & NO_DAMAGE == 0 {
            let mut damage = hit.damage;
            if b.armor >= 0 {
                damage -= f32::from(b.armor);
                if damage <= 0.0 {
                    damage = 1.0;
                }
            }
            b.hit_points = (b.hit_points - (damage + 0.5) as i32).max(0);
        }
        let dead = b.hit_points == 0;
        debug!("breakable {} ({class:?} {subtype:#x}) hit for {:.1}: {} left", b.placement, hit.damage, b.hit_points);
        let mut remove = dead;
        match class {
            ItemClass::Container => {
                if dead && flags & BREAKS_OPEN != 0 && flags & USED == 0 {
                    items.set_flags(b.placement, USED);
                    if subtype == BARREL
                        && let Some(s) = barrel_sound("WOOD", realm)
                    {
                        sounds.write(PlaySound(s));
                    }
                    // (A monster inside, such as a Death, isn't released yet.)
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
                        sounds.write(PlaySound("S_SECRETWALL".into()));
                        items.free(b.placement, &mut commands);
                        info!("secret wall {} broken", b.placement);
                    }
                }
                EXP_BARREL | POI_BARREL => {
                    if dead {
                        items.set_flags(b.placement, USED);
                        let (kind, (damage, radius)) = if subtype == EXP_BARREL { ("EXPLO", EXPLOSION) } else { ("GAS", POISON) };
                        if let Some(s) = barrel_sound(kind, realm) {
                            sounds.write(PlaySound(s));
                        }
                        let at = Vec3::from(centre);
                        blast(at, damage * hazard_scale, radius, hit.attacker, &monsters, &mut players, &mut messages.p1(), &mut hurt);
                        info!("barrel {} {}", b.placement, if subtype == EXP_BARREL { "explodes" } else { "lets out gas" });
                    }
                }
                _ => {
                    if dead {
                        items.set_flags(b.placement, USED);
                        if let Some(s) = barrel_sound("WOOD", realm) {
                            sounds.write(PlaySound(s));
                        }
                    } else {
                        sounds.write(PlaySound("S_WEAPONHITWOOD".into()));
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

fn view_hit_points(items: &LevelItems, placement: usize) -> i16 {
    items.view(placement).map_or(1, |v| v.ty.hit_points)
}

/// A blast at `at`: every monster and the hero within `radius` (plus their
/// own radius) takes its share of `damage` (the missiles' falloff).
#[allow(clippy::too_many_arguments)]
fn blast(
    at: Vec3,
    damage: f32,
    radius: f32,
    attacker: Entity,
    monsters: &Query<(Entity, &Monster)>,
    players: &mut Query<(Entity, &mut Player)>,
    hits: &mut MessageWriter<Hit>,
    hurt: &mut MessageWriter<DamagePlayer>,
) {
    for (e, m) in monsters {
        let d = Vec3::from(m.position).distance(at) - m.stats.radius;
        if let Some(share) = blast_share(d, radius) {
            debug!("the blast hits monster {e:?} for {:.1}", damage * share);
            hits.write(Hit {
                target: e,
                attacker,
                damage: damage * share,
                kind: 0,
                push: Vec3::ZERO,
                at,
                target_kind: TargetKind::Monster,
                ranged: true,
            });
        }
    }
    for (_, mut p) in players.iter_mut() {
        let d = Vec3::from(p.mover.position).distance(at) - p.radius;
        if let Some(share) = blast_share(d, radius) {
            let amount = after_armor(damage * share, p.armor);
            if amount > 0.0 {
                let away = (Vec3::from(p.mover.position) - at).with_y(0.0).normalize_or_zero();
                p.queue_hit(amount, 0x10, away);
                hurt.write(DamagePlayer { amount });
                debug!("the blast hurts the hero for {amount:.1}");
            }
        }
    }
}
