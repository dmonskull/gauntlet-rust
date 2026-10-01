//! The settings' Frame Rate readout (not the game's): frames a second in
//! the window's top right corner, updated twice a second.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::options::GameOptions;

pub struct FrameRatePlugin;

impl Plugin for FrameRatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct FrameRateText;

fn spawn(mut commands: Commands) {
    commands.spawn((
        FrameRateText,
        Visibility::Hidden,
        Text::new(""),
        TextFont { font_size: 18.0, ..default() },
        TextColor(Color::srgb(1.0, 1.0, 0.6)),
        TextShadow::default(),
        Node { position_type: PositionType::Absolute, top: Val::Px(8.0), right: Val::Px(12.0), ..default() },
        GlobalZIndex(100),
    ));
}

fn update(
    options: Res<GameOptions>,
    diagnostics: Res<DiagnosticsStore>,
    time: Res<Time<Real>>,
    mut last: Local<f32>,
    mut text: Query<(&mut Text, &mut Visibility), With<FrameRateText>>,
) {
    let Ok((mut text, mut visibility)) = text.single_mut() else { return };
    visibility.set_if_neq(if options.frame_rate { Visibility::Inherited } else { Visibility::Hidden });
    if !options.frame_rate || time.elapsed_secs() - *last < 0.5 {
        return;
    }
    *last = time.elapsed_secs();
    if let Some(fps) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()) {
        text.0 = format!("{fps:.0} FPS");
    }
}
