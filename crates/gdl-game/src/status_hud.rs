//! The hero's status as a debug line at the top left (F1 with
//! `GDL_DEV_KEYS=1`, or `GDL_DEBUG_HUD=1`; the game's own panel is `game_hud.rs`): health, level,
//! gold, keys, potions, running powerups, and the hint on screen (the
//! game's hint box is `hints.rs`).
//! Bevy's built-in font is ASCII-only, so all text here is ASCII.

use bevy::prelude::*;

use crate::player::Player;

use crate::hints::Hints;
use crate::party::Party;
use crate::player_state::PlayerState;

pub struct StatusHudPlugin;

impl Plugin for StatusHudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, (update, show_in_play));
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
        // Anchored to the top: bottom-anchored UI can fall below the visible
        // surface when the window is taller than the screen.
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(14.0), ..default() },
    ));
    commands.spawn((
        HintText,
        Text::new(""),
        TextFont { font_size: 26.0, ..default() },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(420.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::new_with_justify(Justify::Center),
    ));
}

fn update(
    party: Res<Party>,
    hints: Res<Hints>,
    players: Query<&Player>,
    mut shown_turbo: Local<Vec<i32>>,
    mut status: Query<&mut Text, (With<StatusText>, Without<HintText>)>,
    mut hint: Query<&mut Text, (With<HintText>, Without<StatusText>)>,
) {
    let turbos: Vec<i32> = party.members().map(|(slot, _)| players.iter().find(|p| p.slot == slot).map_or(0, |p| p.turbo as i32)).collect();
    if let Ok(mut text) = status.single_mut()
        && (party.is_changed() || turbos != *shown_turbo)
    {
        text.0 = party
            .states()
            .zip(&turbos)
            .map(|((slot, state), turbo)| format!("P{} {}   TURBO {turbo}", slot + 1, status_line(state)))
            .collect::<Vec<_>>()
            .join("\n");
        *shown_turbo = turbos;
    }
    if let Ok(mut text) = hint.single_mut()
        && hints.is_changed()
    {
        text.0 = hints.text().unwrap_or_default();
    }
}

type HudTexts = Or<(With<StatusText>, With<HintText>)>;

/// The status and hint lines only show while a level is played, not on
/// the title, select or loading screens, and only as a debugging aid now
/// that the game's own panel and hint box are drawn (`game_hud.rs`,
/// `hints.rs`): F1 (with `GDL_DEV_KEYS=1`) or `GDL_DEBUG_HUD=1` shows them.
fn show_in_play(
    frontend: Option<Res<crate::frontend::Frontend>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut options: ResMut<crate::options::GameOptions>,
    mut env: Local<Option<bool>>,
    mut texts: Query<&mut Visibility, HudTexts>,
) {
    let env = *env.get_or_insert_with(|| std::env::var("GDL_DEBUG_HUD").is_ok_and(|v| !v.is_empty() && v != "0"));
    if crate::dev_keys() && keys.just_pressed(KeyCode::F1) {
        options.debug_overlay = !options.debug_overlay;
    }
    let playing = frontend.is_none_or(|f| f.playing());
    for mut v in &mut texts {
        let want = if playing && (env || options.debug_overlay) { Visibility::Inherited } else { Visibility::Hidden };
        v.set_if_neq(want);
    }
}

fn status_line(s: &PlayerState) -> String {
    let health = if s.alive { format!("{:.0}/{:.0}", s.health, s.max_health()) } else { "DEAD".into() };
    let mut out = format!(
        "{}  LEVEL {} (XP {})   HEALTH {health}\nGOLD {}   KEYS {}   POTIONS {}",
        s.class, s.level, s.experience, s.gold, s.keys, s.potions.len()
    );
    let crystals: i32 = s.quest.crystals.iter().map(|&c| i32::from(c.max(0))).sum();
    if !s.runestones.is_empty() || crystals > 0 || s.quest.legendary != 0 {
        out += &format!("   RUNES {}   CRYSTALS {crystals}   LEGENDARY {:#x}", s.runestones.len(), s.quest.legendary);
    }
    for p in s.powers.iter().filter(|p| p.live()) {
        let state = match p.state {
            crate::player_state::SlotState::On => "on",
            crate::player_state::SlotState::Off => "off",
            _ => "held",
        };
        out += &format!("\n{} {:#x} {state}", power_name(p.subtype), p.value);
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
