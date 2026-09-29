//! What a landed blow does (`docs/combat.md`, `docs/monsters.md`): the
//! hero's `Hit` messages take hit points off monsters and generators.
//!
//! Monsters (the game's `FUN_8004e660`): hit points go down by the damage;
//! below two thirds and one third of their full hit points they hit for
//! 0.667 / 0.333 of their damage; at 0 they die — their generator slot is
//! freed at once and the body plays DEATH before it goes. Generators lose
//! a strength level for each item-type's worth of hit points (their
//! monsters come out weaker) and are gone at 0 with their model.
//!
//! Stand-ins: the level-versus-player-level damage scale and the monster
//! resistances (`FUN_8002f58c`) aren't applied; pushes aren't; there's no
//! hit reaction, sound or score yet.

use bevy::prelude::*;
use gdl_formats::enemy;

use crate::character::Animator;
use crate::combat::{Hit, TargetKind, Targetable};
use crate::generators::Generator;
use crate::monsters::{Monster, MonsterLevel};
use crate::player::PlayerTick;
use crate::population::PlacementModel;

pub struct DamagePlugin;

impl Plugin for DamagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, (apply_hits.after(PlayerTick), remove_bodies));
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
    level: Option<Res<MonsterLevel>>,
    mut monsters: Query<(&mut Monster, &mut Animator)>,
    mut generators: Query<&mut Generator>,
    models: Query<(Entity, &PlacementModel)>,
) {
    for hit in hits.read() {
        match hit.target_kind {
            TargetKind::Monster => {
                let Ok((mut m, mut animator)) = monsters.get_mut(hit.target) else { continue };
                m.hit_points -= hit.damage;
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
                commands.entity(hit.target).remove::<(Monster, Targetable)>().insert(Dying { left: BODY_TIME });
                debug!("monster {:?} dies", hit.target);
            }
            TargetKind::Generator => {
                let Ok(mut g) = generators.get_mut(hit.target) else { continue };
                g.hit_points -= hit.damage;
                if g.hit_points <= 0.0 {
                    for (e, model) in &models {
                        if model.0 == g.placement {
                            commands.entity(e).despawn();
                        }
                    }
                    commands.entity(hit.target).despawn();
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
            commands.entity(e).despawn();
        }
    }
}
