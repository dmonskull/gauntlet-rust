//! Developer overlay: which game is loaded, current level, audio, controls.
//! Hidden by default; F1 toggles it with `GDL_DEV_KEYS=1` (`GDL_DEBUG_HUD=1` starts with it on).
//! Bevy's built-in font is ASCII-only, so keep all text here ASCII.

use bevy::prelude::*;

use crate::audio::AudioStatus;
use crate::level::LoadedGame;
use crate::population::{LevelPopulation, PopulationView};
use crate::world::CurrentLevelStats;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud).add_systems(Update, (toggle_hud, update_hud));
    }
}

#[derive(Component)]
struct HudText;

fn toggle_hud(keys: Res<ButtonInput<KeyCode>>, mut hud: Query<&mut Visibility, With<HudText>>) {
    if crate::dev_keys()
        && keys.just_pressed(KeyCode::F1)
        && let Ok(mut v) = hud.single_mut()
    {
        *v = if *v == Visibility::Hidden { Visibility::Inherited } else { Visibility::Hidden };
    }
}

fn spawn_hud(mut commands: Commands) {
    let shown = std::env::var("GDL_DEBUG_HUD").is_ok_and(|v| !v.is_empty() && v != "0");
    commands.spawn((
        HudText,
        if shown { Visibility::Inherited } else { Visibility::Hidden },
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
    ));
}

fn update_hud(
    game: Res<LoadedGame>,
    stats: Option<Res<CurrentLevelStats>>,
    audio: Res<AudioStatus>,
    view: Res<PopulationView>,
    population: Option<Res<LevelPopulation>>,
    mut text: Query<&mut Text, With<HudText>>,
) {
    let Some(stats) = stats else { return };
    if !stats.is_changed() && !game.is_changed() && !audio.is_changed() && !view.is_changed() {
        return;
    }
    let Ok(mut text) = text.single_mut() else { return };

    let install = &game.install;
    let mut s = format!(
        "{} [{}] - {}\n{}\n\n{} ({}/{}): ",
        install.title.as_deref().unwrap_or("Gauntlet: Dark Legacy"),
        install.game_id.as_deref().unwrap_or("?"),
        install.source_kind(),
        game.summary_line(),
        stats.name,
        game.current + 1,
        game.levels.len(),
    );
    match &stats.error {
        Some(e) => s += &format!("failed to load: {e}"),
        None => {
            s += &format!(
                "{} objects, {} triangles in {} meshes, {} collision triangles",
                game.levels[game.current].objects, stats.triangles, stats.meshes, stats.collision_triangles
            )
        }
    }
    let summary = &game.levels[game.current];
    if summary.unsupported_textures > 0 {
        s += &format!(" ({} textures in unsupported formats)", summary.unsupported_textures);
    }
    s += &format!("\nmusic: {}{}", audio.music, if audio.muted { " (muted)" } else { "" });
    if !audio.last_sound.is_empty() {
        s += &format!("  sound: {}", audio.last_sound);
    }
    if stats.error.is_none() {
        s += &format!("\n{} placements: {}", summary.placements, stats.population);
        if let Some(start) = population.as_ref().filter(|p| p.level == stats.name).and_then(|p| p.player_start()) {
            let [x, y, z] = start.position;
            s += &format!("\nplayer start: {x:.1} {y:.1} {z:.1}, yaw {:.0} deg", start.yaw.to_degrees());
        }
        s += &format!("\npopulation shown: {}", view.label());
    }
    for (name, why) in &game.failures {
        s += &format!("\nlevel {name} failed: {why}");
    }
    s += "\n\nWASD/stick move  Shift walk  J attack  L power  H turbo/defend  O strafe  C free camera  [ ] level  I population  K collision  M mute music  N next sound\nfree camera: WASD fly  Space/Ctrl up/down  Shift fast  right-drag look  wheel speed";
    text.0 = s;
}
