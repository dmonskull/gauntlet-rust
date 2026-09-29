//! The hero's status in an overlay at the bottom left: health, level,
//! gold, keys, potions, running powerups, and the hint on screen.
//! Bevy's built-in font is ASCII-only, so all text here is ASCII.

use bevy::prelude::*;

use crate::hints::Hints;
use crate::player_state::PlayerState;

pub struct StatusHudPlugin;

impl Plugin for StatusHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct StatusText;

#[derive(Component)]
struct HintText;

fn spawn(mut commands: Commands) {
    commands.spawn((
        StatusText,
        Text::new(""),
        TextFont { font_size: 22.0, ..default() },
        TextColor(Color::srgb(1.0, 0.92, 0.6)),
        TextShadow::default(),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(14.0), left: Val::Px(14.0), ..default() },
    ));
    commands.spawn((
        HintText,
        Text::new(""),
        TextFont { font_size: 26.0, ..default() },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(120.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::new_with_justify(Justify::Center),
    ));
}

fn update(
    state: Res<PlayerState>,
    hints: Res<Hints>,
    mut status: Query<&mut Text, (With<StatusText>, Without<HintText>)>,
    mut hint: Query<&mut Text, (With<HintText>, Without<StatusText>)>,
) {
    if let Ok(mut text) = status.single_mut()
        && state.is_changed()
    {
        text.0 = status_line(&state);
    }
    if let Ok(mut text) = hint.single_mut()
        && hints.is_changed()
    {
        text.0 = hints.text.clone().unwrap_or_default();
    }
}

fn status_line(s: &PlayerState) -> String {
    let health = if s.alive { format!("{:.0}/{:.0}", s.health, s.max_health()) } else { "DEAD".into() };
    let mut out = format!(
        "{}  LEVEL {} (XP {})   HEALTH {health}\nGOLD {}   KEYS {}   POTIONS {}",
        s.class, s.level, s.experience, s.gold, s.keys, s.potions.len()
    );
    if !s.runestones.is_empty() || !s.treasures.is_empty() {
        out += &format!("   RUNES {}   TREASURES {}", s.runestones.len(), s.treasures.len());
    }
    for p in &s.powers {
        out += &format!("\n{} {:#x}", power_name(p.subtype), p.value);
        if p.amount > 0.0 {
            out += &format!(" x{:.0}", p.amount);
        }
        if p.time > 0.0 {
            out += &format!("  {:.0}s", p.time);
        }
    }
    out
}

/// The game's names for the powerup subtypes.
fn power_name(subtype: i32) -> &'static str {
    match subtype {
        5 => "WEAPON",
        6 => "ARMOR",
        7 => "SPEED",
        8 => "MAGIC",
        9 => "SPECIAL",
        _ => "POWER",
    }
}
