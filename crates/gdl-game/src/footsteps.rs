//! The heroes' footsteps (`docs/player-movement.md`, "Footsteps"). The
//! game has no step timer or distance: a foot comes down as one of the
//! stepping actions ends, when another action takes over or it starts
//! over. SHOVE, WALK1, RUN1 and SHIELD_RUN put down the first foot, WALK2
//! and RUN2 the second, so a walk or a run (two half-stride clips) steps
//! twice a stride and the looping shove and shield run once a loop.
//!
//! The sound is the foot's from the step table, by what's underfoot:
//! water (a water surface above the floor being followed), else metal
//! while invulnerable (the chrome clanks), else stairs (a floor node with
//! flag `0x8`), else rock. The table's wood column is never picked. A
//! levitating hero makes no sound. A step plays at the hero's feet at the
//! call's own volume, panned by where the feet are from the camera's focus
//! and faded by the distance to the nearest hero — none, for the hero's
//! own (`audio::PlaySoundAt`).

use bevy::prelude::*;
use gdl_formats::PlayerCollision;

use crate::actions::Action;
use crate::audio::{CALL_VOLUME, PlaySoundAt};
use crate::character::Animator;
use crate::damage::resists::INVULNERABLE;
use crate::player::{Player, PlayerTick};
use crate::player_state::power::LEVITATE;
use crate::world::LevelGround;

pub struct FootstepsPlugin;

impl Plugin for FootstepsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, step.after(PlayerTick));
    }
}

/// What's underfoot: the step table's columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Floor {
    Rock = 0,
    /// Never picked by the game.
    #[allow(dead_code)]
    Wood = 1,
    Stairs = 2,
    Metal = 3,
    Water = 4,
}

/// The step sounds, by foot and by [`Floor`] (the game's table: common
/// sounds `0x1C 0x20 0x1E 0x1A 0x22` and `0x1D 0x21 0x1F 0x1B 0x23`).
const STEPS: [[&str; 5]; 2] = [
    ["S_STEPROCK1", "S_STEPWOOD1", "S_STEPSTAIR1", "S_STEPMET1", "S_STEPWATER1"],
    ["S_STEPROCK2", "S_STEPWOOD2", "S_STEPSTAIR2", "S_STEPMET2", "S_STEPWATER2"],
];

/// Collision node flag `0x8`: the levels' `…STAIRS…` and `…STEP…` nodes.
const STAIRS: u32 = 0x8;

/// Running with a shield on the arm.
const SHIELD_RUN: Action = Action(0x16);

/// The foot an action puts down as it ends: the first (0) or the second
/// (1); none for the rest.
fn foot(action: Action) -> Option<usize> {
    match action {
        Action::SHOVE | Action::WALK1 | Action::RUN1 | SHIELD_RUN => Some(0),
        Action::WALK2 | Action::RUN2 => Some(1),
        _ => None,
    }
}

/// What the step sounds on: water over everything, then the chrome, then
/// stairs.
fn floor(water: bool, invulnerable: bool, stairs: bool) -> Floor {
    if water {
        Floor::Water
    } else if invulnerable {
        Floor::Metal
    } else if stairs {
        Floor::Stairs
    } else {
        Floor::Rock
    }
}

fn sound(foot: usize, floor: Floor) -> &'static str {
    STEPS[foot.min(1)][floor as usize]
}

/// A hero's action and its clip's frame, as of a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stride {
    action: Action,
    clip: usize,
    frame: f32,
}

/// The action that ended between two ticks: the one playing before, if
/// another has taken over, or if it started over (its clip's frame went
/// back: a loop coming round, or the clip played again).
fn ended(before: Stride, now: Stride) -> Option<Action> {
    if now.action != before.action {
        Some(before.action)
    } else if now.clip == before.clip && now.frame < before.frame {
        Some(now.action)
    } else {
        None
    }
}

/// Each tick after the heroes': a step for every stepping action that
/// ended.
fn step(
    ground: Option<Res<LevelGround>>,
    heroes: Query<(Entity, &Player, &Animator)>,
    mut seen: Local<Vec<(Entity, Stride)>>,
    mut sounds: MessageWriter<PlaySoundAt>,
) {
    seen.retain(|(e, _)| heroes.contains(*e));
    for (hero, p, animator) in &heroes {
        let now = Stride { action: p.actions.action, clip: animator.action, frame: animator.frame };
        let before = match seen.iter_mut().find(|(e, _)| *e == hero) {
            Some((_, s)) => std::mem::replace(s, now),
            None => {
                seen.push((hero, now));
                continue;
            }
        };
        let Some(action) = ended(before, now) else { continue };
        let Some(foot) = foot(action) else { continue };
        if p.special_bits & LEVITATE != 0 {
            continue;
        }
        let (water, stairs) = ground.as_ref().map_or((false, false), |g| {
            let level = &g.0;
            let body = PlayerCollision::default();
            let water = level.player_water(p.mover.position, &body).is_some_and(|w| w.point[1] > p.ground.floor);
            let stairs = p.ground.node.and_then(|n| level.nodes.get(n)).is_some_and(|n| n.flags & STAIRS != 0);
            (water, stairs)
        });
        let name = sound(foot, floor(water, p.armour_bits & INVULNERABLE != 0, stairs));
        debug!("{} ends: {name}", action.name());
        sounds.write(PlaySoundAt::faded(name, Vec3::from(p.mover.position), CALL_VOLUME));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_actions_and_their_feet() {
        assert_eq!(foot(Action::WALK1), Some(0));
        assert_eq!(foot(Action::WALK2), Some(1));
        assert_eq!(foot(Action::RUN1), Some(0));
        assert_eq!(foot(Action::RUN2), Some(1));
        assert_eq!(foot(Action::SHOVE), Some(0));
        assert_eq!(foot(SHIELD_RUN), Some(0));
        // Standing, strafing, SHIELD_READY and attacking make none.
        for a in [Action::READY, Action::STRAFE_WLKF1, Action::STRAFE_WLKL2, Action(0x15), Action::ATTQUICK1] {
            assert_eq!(foot(a), None, "{}", a.name());
        }
    }

    #[test]
    fn the_floor_picks_the_sound() {
        assert_eq!(sound(0, floor(false, false, false)), "S_STEPROCK1");
        assert_eq!(sound(1, floor(false, false, true)), "S_STEPSTAIR2");
        assert_eq!(sound(0, floor(false, true, true)), "S_STEPMET1");
        assert_eq!(sound(1, floor(true, true, true)), "S_STEPWATER2");
        assert_eq!(sound(1, Floor::Wood), "S_STEPWOOD2");
    }

    #[test]
    fn a_step_as_an_action_ends_or_starts_over() {
        let at = |action, clip, frame| Stride { action, clip, frame };
        // A half stride hands over to the next.
        assert_eq!(ended(at(Action::WALK1, 3, 11.5), at(Action::WALK2, 4, 0.0)), Some(Action::WALK1));
        // Still going, or held at its end.
        assert_eq!(ended(at(Action::WALK1, 3, 2.0), at(Action::WALK1, 3, 3.0)), None);
        assert_eq!(ended(at(Action::WALK1, 3, 11.5), at(Action::WALK1, 3, 11.5)), None);
        // A loop coming round.
        assert_eq!(ended(at(Action::SHOVE, 7, 19.0), at(Action::SHOVE, 7, 1.0)), Some(Action::SHOVE));
    }

    /// Every step sound is in the catalog (real data).
    #[test]
    fn step_sounds_are_in_the_catalog() {
        let root = std::env::var("GAUNTLET_ASSET_ROOT")
            .unwrap_or_else(|_| "/Users/dmonskull/Desktop/GauntletDarkLegacy/Gauntlet".into());
        let Ok(bytes) = std::fs::read(std::path::Path::new(&root).join("AUDIO/AUDATPS2.ROM")) else {
            eprintln!("skipping: no AUDATPS2.ROM");
            return;
        };
        let catalog = gdl_formats::audio::AudioCatalog::parse(&bytes).unwrap();
        for name in STEPS.iter().flatten() {
            assert!(catalog.find_sound(name).is_some(), "{name}");
        }
    }
}
