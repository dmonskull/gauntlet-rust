//! What a landed blow does (`docs/combat.md`, `docs/monsters.md`): the
//! hero's `Hit` messages take hit points off monsters and generators.
//!
//! Monsters (the game's monster-damage routine): hit points go down by the damage;
//! below two thirds and one third of their full hit points they hit for
//! 0.667 / 0.333 of their damage; at 0 they die — their generator slot is
//! freed at once and the body plays out its death (`deaths.rs`). Generators take
//! whole hit points (the blow scaled by the hero's level against the
//! level's, less their armour, at least 1: `enemy::item_damage`), lose a
//! strength level for each item-type's worth (their monsters come out
//! weaker) and are gone at 0 with their model. Blows on them earn five
//! times a blow on one of their monsters.
//!
//! Monster blows (`MonsterHit`) hurt the hero through `DamagePlayer`.
//!
//! The flinch/knockdown and push the blow causes play out in the monster's
//! next tick (`monsters.rs`).
//!
//! Monster blows lose the hero's armour first (weak ones do nothing).
//!
//! A blow that does damage plays the monster's hit sound, the killing one
//! its death sound (`MonsterSounds`): the far versions for thrown blows.
//!
//! Stand-ins: the level-versus-player-level damage scale, the monster
//! resistances, elements and blocking while defending aren't applied; there are no
//! hit effects (the blood sprays) or score yet, and sounds aren't placed in 3D. What happens when
//! the hero dies is the front end's (`frontend.rs`).

use bevy::prelude::*;
use gdl_formats::enemy;

use crate::audio::PlaySound;
use crate::combat::{Hit, TargetKind, Targetable};
use crate::critters::{Critter, CritterLevel, CritterSphere};
use crate::deaths;
use crate::effects::EffectAt;
use crate::generators::Generator;
use crate::monsters::{self, Monster, MonsterHit, MonsterLevel};
use crate::player::Player;
use crate::player_state::{DamagePlayer, PlayerState};
use crate::player::PlayerTick;
use crate::population::{GeneratorLooks, PlacementIndex};

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (apply_hits.after(PlayerTick), hurt_hero));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_hits(
    mut commands: Commands,
    mut hits: MessageReader<Hit>,
    mut state: Option<ResMut<PlayerState>>,
    level: Option<Res<MonsterLevel>>,
    mut monsters: Query<&mut Monster>,
    mut generators: Query<&mut Generator>,
    models: Query<(Entity, &PlacementIndex, Option<&GeneratorLooks>, Option<&Children>)>,
    mut critters: Query<&mut Critter>,
    spheres: Query<&CritterSphere>,
    critter_level: Option<Res<CritterLevel>>,
    mut sounds: MessageWriter<PlaySound>,
    mut effects: MessageWriter<EffectAt>,
    heroes: Query<(), With<Player>>,
) {
    for hit in hits.read() {
        // Only a hero's blows earn experience (a monster's bomb or blast
        // earns nobody any).
        let by_hero = heroes.contains(hit.attacker);
        match hit.target_kind {
            TargetKind::Monster => {
                let Ok(mut m) = monsters.get_mut(hit.target) else { continue };
                // Already dead from an earlier blow this tick.
                if m.hit_points <= 0.0 {
                    continue;
                }
                m.take_hit(hit.damage, hit.kind, hit.push.to_array());
                if hit.damage > 0.0 {
                    m.hits = m.hits.saturating_add(1);
                    if let Some(s) = level.as_ref().and_then(|l| l.sounds.get(&m.enemy)) {
                        let name = if m.hit_points > 0.0 { s.hit(m.strength, m.hits, hit.ranged) } else { s.die(m.strength, hit.ranged) };
                        sounds.write(PlaySound(name.to_string()));
                    }
                }
                // Every blow earns experience; the killing one earns the
                // kill's.
                if by_hero && let (Some(state), Some(level)) = (state.as_mut(), level.as_ref()) {
                    let (at, scale) = level.experience;
                    let xp = enemy::experience(m.enemy, m.hit_points <= 0.0, state.level, at, scale);
                    let gained = state.add_experience(xp);
                    if gained > 0 {
                        info!("the hero reaches level {}", state.level);
                    }
                }
                if m.hit_points > 0.0 {
                    // Wounded monsters hit softer.
                    if let (Some(stats), Some(level)) = (enemy::enemy_stats(m.enemy), level.as_ref()) {
                        let full = stats.base_hit_points(&level.scales);
                        let base = stats.damage * level.scales.damage;
                        m.stats.damage = if m.hit_points <= 0.333 * full {
                            0.333 * base
                        } else if m.hit_points <= 0.667 * full {
                            0.667 * base
                        } else {
                            m.stats.damage
                        };
                    }
                    // The blow's effect: a blood spray, or the element's.
                    if hit.damage > 0.0
                        && let Some(name) = deaths::hit_effect(m.enemy, hit.kind)
                    {
                        let at = deaths::effect_origin(m.stats.step, m.centre(), hit.at);
                        let scale = deaths::die_effect_scale(m.enemy, m.stats.step);
                        effects.write(EffectAt { name, bank: None, at, facing: 0.0, scale });
                    }
                    debug!("monster {:?} hit for {:.1}: {:.1} left", hit.target, hit.damage, m.hit_points);
                    continue;
                }
                // Dead: its slot is free now, it can't be targeted, and the
                // body plays out its death (`deaths.rs`).
                if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
                    g.alive = g.alive.saturating_sub(1);
                }
                m.die(hit.kind, hit.at.to_array());
                commands.entity(hit.target).try_remove::<Targetable>();
                debug!("monster {:?} dies", hit.target);
            }
            TargetKind::Generator => {
                let Ok(mut g) = generators.get_mut(hit.target) else { continue };
                if g.hit_points <= 0.0 {
                    continue;
                }
                let hero_level = state.as_ref().map_or(1, |s| s.level);
                let was = g.hit_points;
                let at = level.as_ref().map_or(0.0, |l| l.experience.0);
                g.hit_points -= enemy::item_damage(hit.damage, g.armor, hero_level, at) as f32;
                // Five times a blow on one of its monsters.
                if by_hero && let (Some(state), Some(level)) = (state.as_mut(), level.as_ref()) {
                    let (at, scale) = level.experience;
                    let xp = enemy::generator_experience(g.enemy, g.hit_points <= 0.0, state.level, at, scale);
                    if state.add_experience(xp) > 0 {
                        info!("the hero reaches level {}", state.level);
                    }
                }
                // Its realm's sounds; none for destroying one on a boss level.
                if let Some(level) = level.as_ref()
                    && let Some((hurt, destroyed)) = enemy::generator_sounds(level.realm, g.enemy)
                    && g.hit_points < was
                {
                    if g.hit_points > 0.0 {
                        sounds.write(PlaySound(hurt));
                    } else if level.boss < 0 {
                        sounds.write(PlaySound(destroyed));
                    }
                }
                if g.hit_points <= 0.0 {
                    for (e, model, _, _) in &models {
                        if model.0 == g.placement {
                            commands.entity(e).try_despawn();
                        }
                    }
                    commands.entity(hit.target).try_despawn();
                    debug!("generator {} destroyed", g.placement);
                } else if g.hit_points_per_tier > 0.0 {
                    let tier = (g.hit_points / g.hit_points_per_tier).ceil().clamp(1.0, 3.0) as i32;
                    if tier != g.tier {
                        g.tier = tier;
                        // Its model steps down with it.
                        for (e, model, looks, children) in &models {
                            let (true, Some(looks)) = (model.0 == g.placement, looks) else { continue };
                            let Some(parts) = looks.0.get(tier as usize - 1) else { continue };
                            for c in children.into_iter().flatten() {
                                commands.entity(*c).try_despawn();
                            }
                            for (mesh, material) in parts {
                                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), ChildOf(e)));
                            }
                        }
                        debug!("generator {} drops to strength {tier}", g.placement);
                    }
                }
            }
            TargetKind::Object => {
                // Critters (`critters.rs`), on the body or a hit sphere:
                // block, armour, experience.
                let (target, sphere, part) = match spheres.get(hit.target) {
                    Ok(s) => (s.critter, s.node, s.part),
                    Err(_) => (hit.target, None, None),
                };
                let Ok(mut c) = critters.get_mut(target) else { continue };
                let mut hit_sounds = Vec::new();
                let level = critter_level.as_deref();
                let xp = c.take_hit(hit.damage, hit.kind, hit.push.to_array(), sphere, part, hit.ranged, level, &mut hit_sounds);
                info!("the hero hits a critter for {:.1}: {:.0} hit points left", hit.damage, c.hit_points);
                for s in hit_sounds {
                    sounds.write(PlaySound(s));
                }
                if let Some(state) = state.as_mut()
                    && state.add_experience(xp) > 0
                {
                    info!("the hero reaches level {}", state.level);
                }
            }
            _ => {}
        }
    }
}

/// The game's armour rule: a blow armour stops loses the armour value, and
/// one no stronger than the armour does nothing.
pub fn after_armor(damage: f32, armor: f32) -> f32 {
    if damage < 0.0 {
        -damage
    } else if damage > armor {
        damage - armor
    } else {
        0.0
    }
}

/// The monster type whose blows knock heroes down.
const KNOCKDOWN_MONSTER: i32 = 0x1D;

fn hurt_hero(
    mut hits: MessageReader<MonsterHit>,
    monsters: Query<&Monster>,
    mut players: Query<&mut Player>,
    mut damage: MessageWriter<DamagePlayer>,
) {
    for hit in hits.read() {
        let Ok(mut p) = players.get_mut(hit.player) else { continue };
        // The blow's kind, as the monster's attack sets it: a big monster's
        // strong attack knocks back, one type knocks down, small monsters'
        // blows only make the hero flinch.
        let (mut flags, mut push) = (0u32, Vec3::ZERO);
        if let Ok(m) = monsters.get(hit.monster) {
            let big = m.stats.step > monsters::BIG_STEP;
            if hit.strong && big {
                flags |= 0x10;
            }
            if m.enemy == KNOCKDOWN_MONSTER {
                flags |= 0x20;
            }
            if !big {
                flags |= 0x4000_0000;
            }
            if flags & 0x130 != 0 {
                let d = Vec3::from(p.mover.position) - Vec3::from(m.position);
                push = Vec3::new(d.x, 0.0, d.z).normalize_or_zero();
            }
        }
        let amount = after_armor(hit.damage, p.armor);
        if amount > 0.0 {
            p.queue_hit(amount, flags, push);
            damage.write(DamagePlayer { amount });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armour_takes_its_value_off_or_stops_the_blow() {
        assert_eq!(after_armor(5.0, 1.5), 3.5);
        assert_eq!(after_armor(1.0, 1.5), 0.0);
        assert_eq!(after_armor(-2.0, 1.5), 2.0);
    }
}
