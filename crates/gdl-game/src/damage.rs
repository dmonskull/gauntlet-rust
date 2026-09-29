//! What a landed blow does (`docs/combat.md`, `docs/monsters.md`): the
//! hero's `Hit` messages take hit points off monsters and generators.
//!
//! Monsters (the game's monster-damage routine): hit points go down by the damage;
//! below two thirds and one third of their full hit points they hit for
//! 0.667 / 0.333 of their damage; at 0 they die — their generator slot is
//! freed at once and the body plays DEATH before it goes. Generators lose
//! a strength level for each item-type's worth of hit points (their
//! monsters come out weaker) and are gone at 0 with their model.
//!
//! Monster blows (`MonsterHit`) hurt the hero through `DamagePlayer`.
//!
//! The flinch/knockdown and push the blow causes play out in the monster's
//! next tick (`monsters.rs`).
//!
//! Monster blows lose the hero's armour first (weak ones do nothing).
//!
//! Stand-ins: the level-versus-player-level damage scale, the monster
//! resistances, elements and blocking while defending aren't applied; there's no hit
//! sound, effect or score yet. When the hero dies it plays DEATH
//! and comes back at the level start at full health (lives and the
//! game-over flow aren't done).

use bevy::prelude::*;
use gdl_formats::enemy;

use crate::character::Animator;
use crate::combat::{Hit, TargetKind, Targetable};
use crate::generators::Generator;
use crate::monsters::{Monster, MonsterHit, MonsterLevel};
use crate::player::Player;
use crate::player_state::{DamagePlayer, PlayerState};
use crate::player::PlayerTick;
use crate::population::PlacementIndex;

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (apply_hits.after(PlayerTick), remove_bodies, hurt_hero, revive_hero));
    }
}

/// A monster that has died, playing out its death.
#[derive(Component)]
struct Dying {
    /// Seconds before the body is removed even if DEATH doesn't finish.
    left: f32,
}

/// How long a body may lie before it's removed.
const BODY_TIME: f32 = 4.0;

#[allow(clippy::too_many_arguments)]
fn apply_hits(
    mut commands: Commands,
    mut hits: MessageReader<Hit>,
    mut state: Option<ResMut<PlayerState>>,
    level: Option<Res<MonsterLevel>>,
    mut monsters: Query<(&mut Monster, &mut Animator)>,
    mut generators: Query<&mut Generator>,
    models: Query<(Entity, &PlacementIndex)>,
) {
    for hit in hits.read() {
        match hit.target_kind {
            TargetKind::Monster => {
                let Ok((mut m, mut animator)) = monsters.get_mut(hit.target) else { continue };
                // Already dead from an earlier blow this tick.
                if m.hit_points <= 0.0 {
                    continue;
                }
                m.take_hit(hit.damage, hit.kind, hit.push.to_array());
                // Every blow earns experience; the killing one earns the
                // kill's.
                if let (Some(state), Some(level)) = (state.as_mut(), level.as_ref()) {
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
                    debug!("monster {:?} hit for {:.1}: {:.1} left", hit.target, hit.damage, m.hit_points);
                    continue;
                }
                // Dead: its slot is free now; the body plays out.
                if let Some(mut g) = m.generator.and_then(|g| generators.get_mut(g).ok()) {
                    g.alive = g.alive.saturating_sub(1);
                }
                animator.play_named("DEATH");
                commands.entity(hit.target).try_remove::<(Monster, Targetable)>().try_insert(Dying { left: BODY_TIME });
                debug!("monster {:?} dies", hit.target);
            }
            TargetKind::Generator => {
                let Ok(mut g) = generators.get_mut(hit.target) else { continue };
                if g.hit_points <= 0.0 {
                    continue;
                }
                g.hit_points -= hit.damage;
                if g.hit_points <= 0.0 {
                    for (e, model) in &models {
                        if model.0 == g.placement {
                            commands.entity(e).try_despawn();
                        }
                    }
                    commands.entity(hit.target).try_despawn();
                    debug!("generator {} destroyed", g.placement);
                } else if g.hit_points_per_tier > 0.0 {
                    g.tier = (g.hit_points / g.hit_points_per_tier).ceil().clamp(1.0, 3.0) as i32;
                }
            }
            _ => {}
        }
    }
}

fn remove_bodies(
    time: Res<Time>,
    mut commands: Commands,
    mut bodies: Query<(Entity, &mut Dying, &Animator)>,
) {
    for (e, mut d, animator) in &mut bodies {
        d.left -= time.delta_secs();
        let done = animator.action_name() == "DEATH" && animator.finished();
        if d.left <= 0.0 || (done && d.left < BODY_TIME - 1.5) {
            commands.entity(e).try_despawn();
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

/// Stand-in for the game's "big monster" value (`docs/monsters.md`).
const BIG_MONSTER_RADIUS: f32 = 2.0;
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
            let big = m.stats.radius > BIG_MONSTER_RADIUS;
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

/// Seconds the hero lies dead before coming back.
const REVIVE_AFTER: f32 = 3.0;

fn revive_hero(
    time: Res<Time>,
    mut state: ResMut<PlayerState>,
    mut dead_for: Local<f32>,
    mut players: Query<(&mut Player, &mut Animator)>,
) {
    if state.alive {
        *dead_for = 0.0;
        return;
    }
    let Ok((mut p, mut animator)) = players.single_mut() else { return };
    if *dead_for == 0.0 {
        animator.play_named("DEATH");
    }
    *dead_for += time.delta_secs();
    if *dead_for >= REVIVE_AFTER {
        let (at, facing) = p.start;
        p.teleport(at, facing);
        state.health = state.max_health();
        state.alive = true;
        animator.play_named("READY");
        info!("the hero is back at the start");
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
