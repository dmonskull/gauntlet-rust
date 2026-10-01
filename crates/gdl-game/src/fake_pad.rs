//! `GDL_FAKE_PAD="start@300,south@320-321,lstick=0:1@400-600"` (testing): a
//! pad plugged in by script, fed through Bevy's raw pad events as a real
//! one is, so co-op (joining, a second player's play) can be checked
//! without a second person. Each item is a button (`start`, `south`,
//! `east`, `west`, `north`, `select`, `lb`, `rb`, `lt`, `rt`, `up`,
//! `down`, `left`, `right`) or a stick (`lstick=x:y`, `rstick=x:y`) at a
//! frame or held over a range of frames (Update frames since start); the
//! pad connects on frame `GDL_FAKE_PAD_AT` (default 10). With
//! `GDL_FAKE_PAD_PLAYER=<class>` it joins the party at once as the next
//! player (testing co-op on a level picked from the command line; connect
//! it on frame 1 so it's there as the first level spawns its heroes).

use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent, RawGamepadAxisChangedEvent,
    RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;

pub struct FakePadPlugin;

impl Plugin for FakePadPlugin {
    fn build(&self, app: &mut App) {
        if let Some(script) = std::env::var("GDL_FAKE_PAD").ok().map(|s| parse(&s)) {
            let at = std::env::var("GDL_FAKE_PAD_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
            app.insert_resource(FakePad { script, connect_at: at, pad: None, frame: 0 }).add_systems(PreUpdate, drive);
        }
    }
}

/// What the scripted pad does over a range of frames (inclusive).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Act {
    Button(GamepadButton),
    Stick(bool, Vec2),
}

#[derive(Resource)]
struct FakePad {
    script: Vec<(Act, u64, u64)>,
    connect_at: u64,
    pad: Option<Entity>,
    frame: u64,
}

fn parse(spec: &str) -> Vec<(Act, u64, u64)> {
    spec.split(',')
        .filter_map(|item| {
            let (what, when) = item.trim().split_once('@')?;
            let (a, b) = when.split_once('-').unwrap_or((when, when));
            let (from, to) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
            let act = match what.trim().split_once('=') {
                Some((stick, v)) => {
                    let (x, y) = v.split_once(':')?;
                    Act::Stick(stick == "rstick", Vec2::new(x.parse().ok()?, y.parse().ok()?))
                }
                None => Act::Button(match what.trim() {
                    "start" => GamepadButton::Start,
                    "select" => GamepadButton::Select,
                    "south" | "a" => GamepadButton::South,
                    "east" | "b" => GamepadButton::East,
                    "west" | "x" => GamepadButton::West,
                    "north" | "y" => GamepadButton::North,
                    "lb" => GamepadButton::LeftTrigger,
                    "rb" => GamepadButton::RightTrigger,
                    "lt" => GamepadButton::LeftTrigger2,
                    "rt" => GamepadButton::RightTrigger2,
                    "up" => GamepadButton::DPadUp,
                    "down" => GamepadButton::DPadDown,
                    "left" => GamepadButton::DPadLeft,
                    "right" => GamepadButton::DPadRight,
                    other => {
                        warn!("GDL_FAKE_PAD: unknown {other}");
                        return None;
                    }
                }),
            };
            Some((act, from, to))
        })
        .collect()
}

/// Connects the pad, then presses and lets go as the script says, before
/// Bevy's pad update reads the raw events.
fn drive(
    mut fake: ResMut<FakePad>,
    mut commands: Commands,
    mut raw: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    (party, mut changes): (Res<crate::party::Party>, MessageWriter<crate::player_state::PartyChange>),
) {
    fake.frame += 1;
    let frame = fake.frame;
    if fake.pad.is_none() && frame >= fake.connect_at {
        let pad = commands.spawn_empty().id();
        fake.pad = Some(pad);
        let connection = GamepadConnection::Connected { name: "Scripted pad".into(), vendor_id: None, product_id: None };
        // As a pad backend does: the connection for the component's
        // insertion, and its raw event.
        connections.write(GamepadConnectionEvent::new(pad, connection.clone()));
        raw.write(RawGamepadEvent::Connection(GamepadConnectionEvent::new(pad, connection)));
        info!("GDL_FAKE_PAD: connected as {pad:?}");
        if let Ok(class) = std::env::var("GDL_FAKE_PAD_PLAYER")
            && let Some(slot) = party.free_slot()
        {
            let choice = crate::player::PlayerChoice { class: class.to_ascii_uppercase(), variant: "BLU".into() };
            let devices = crate::party::Devices { pad: Some(pad), ..default() };
            changes.write(crate::player_state::PartyChange::Set { slot, choice, name: "PELE".into(), saved: None, fresh: true, devices });
            info!("GDL_FAKE_PAD: joins as player {}", slot + 1);
        }
        return;
    }
    let Some(pad) = fake.pad else { return };
    for &(act, from, to) in &fake.script {
        let (on, off) = (frame == from, frame == to + 1);
        if !on && !off {
            continue;
        }
        match act {
            Act::Button(button) => {
                let value = if on { 1.0 } else { 0.0 };
                raw.write(RawGamepadEvent::Button(RawGamepadButtonChangedEvent { gamepad: pad, button, value }));
            }
            Act::Stick(right, v) => {
                let v = if on { v } else { Vec2::ZERO };
                let (x, y) = if right {
                    (GamepadAxis::RightStickX, GamepadAxis::RightStickY)
                } else {
                    (GamepadAxis::LeftStickX, GamepadAxis::LeftStickY)
                };
                raw.write(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent { gamepad: pad, axis: x, value: v.x }));
                raw.write(RawGamepadEvent::Axis(RawGamepadAxisChangedEvent { gamepad: pad, axis: y, value: v.y }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_reads_buttons_and_sticks() {
        let s = parse("start@300, south@320-321,lstick=0:1@400-600,bogus@5");
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], (Act::Button(GamepadButton::Start), 300, 300));
        assert_eq!(s[1], (Act::Button(GamepadButton::South), 320, 321));
        assert_eq!(s[2], (Act::Stick(false, Vec2::new(0.0, 1.0)), 400, 600));
    }
}
