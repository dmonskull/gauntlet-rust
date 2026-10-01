//! The pad's rumble (the game's Rumble Feature, `docs/combat.md`, "The
//! four schemes"): a blow that hurts the hero shakes its pad
//! (from the hero's damage routine, `docs/frontend.md`), harder and longer for
//! the heavier kinds — knock-down and blown-away blows (`0x10040`) at 0.8
//! for 30 fields, `0x120` at 0.6 for 20, `0x90` at 0.4 for 15, the rest at
//! 0.2 for 10 — while the option is on. Not during camera cuts, when no
//! harm comes to the hero, nor once it's dead.

use std::time::Duration;

use bevy::input::gamepad::{Gamepad, GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;

use crate::options::GameOptions;
use crate::party::Party;
use crate::player_state::{DamagePlayer, HurtHero};

pub struct RumblePlugin;

impl Plugin for RumblePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, rumble);
    }
}

/// The game's four strengths (of 1) and their lengths (fields) for a blow
/// of `kind`.
fn strength(kind: u32) -> (f32, f32) {
    if kind & 0x10040 != 0 {
        (0.8, 30.0)
    } else if kind & 0x120 != 0 {
        (0.6, 20.0)
    } else if kind & 0x90 != 0 {
        (0.4, 15.0)
    } else {
        (0.2, 10.0)
    }
}

fn rumble(
    options: Res<GameOptions>,
    party: Res<Party>,
    camera: Option<Res<crate::play_camera::PlayCamera>>,
    (mut hurts, mut hits): (MessageReader<HurtHero>, MessageReader<DamagePlayer>),
    pads: Query<Entity, With<Gamepad>>,
    mut requests: MessageWriter<GamepadRumbleRequest>,
) {
    let blows: Vec<(usize, u32)> = hurts
        .read()
        .filter(|h| h.amount > 0.0)
        .map(|h| (h.slot, h.kind))
        .chain(hits.read().filter(|h| h.amount > 0.0).map(|h| (h.slot, 0)))
        .collect();
    if !options.rumble || camera.is_some_and(|c| c.in_cut()) {
        return;
    }
    let solo = party.members().filter(|(_, m)| !m.devices.remote).count() == 1;
    for (slot, member) in party.members() {
        if !member.state.alive {
            continue;
        }
        let Some((level, fields)) =
            blows.iter().filter(|(s, _)| *s == slot).map(|&(_, kind)| strength(kind)).reduce(|a, b| if b.0 > a.0 { b } else { a })
        else {
            continue;
        };
        // The hit player's pad (alone, every pad nobody else holds).
        let intensity = GamepadRumbleIntensity { strong_motor: level, weak_motor: level };
        for gamepad in &pads {
            if member.devices.pad == Some(gamepad) || (solo && party.slot_of_pad(gamepad).is_none()) {
                requests.write(GamepadRumbleRequest::Add { gamepad, intensity, duration: Duration::from_secs_f32(fields / 60.0) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heavier_blows_shake_harder_and_longer() {
        assert_eq!(strength(0x40), (0.8, 30.0));
        assert_eq!(strength(0x20), (0.6, 20.0));
        assert_eq!(strength(0x10), (0.4, 15.0));
        assert_eq!(strength(0), (0.2, 10.0));
    }
}
