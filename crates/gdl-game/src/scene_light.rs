//! The scene's brightness. The game scales the 3D view's colours by
//! clamp(1 + offset, 0, 1), the offset going down to −0.8 while a boss's
//! intro darkens the level (`CritterLevel::light_offset`,
//! `docs/critters.md`); the HUD isn't touched. Drawn here as a black veil
//! over the 3D view, under the HUD, menus and text.
//!
//! The game multiplies gamma-space colours; Bevy blends in linear space,
//! so the veil lets through brightness^2.2 to look the same.

use gdl_formats::detmath::Det;
use bevy::prelude::*;

use crate::critters::CritterLevel;

pub struct SceneLightPlugin;

impl Plugin for SceneLightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct Veil;

fn spawn(mut commands: Commands) {
    commands.spawn((
        Veil,
        Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
        BackgroundColor(Color::NONE),
        // Below every other layer of 2D (text and panels sit at 0 and up).
        GlobalZIndex(-10),
        Pickable::IGNORE,
    ));
}

/// The veil's opacity for a scene brightness offset.
pub fn veil_alpha(offset: f32) -> f32 {
    let brightness = (1.0 + offset).clamp(0.0, 1.0);
    1.0 - brightness.dpowf(2.2)
}

fn update(critters: Option<Res<CritterLevel>>, mut veil: Query<&mut BackgroundColor, With<Veil>>) {
    let alpha = veil_alpha(critters.map_or(0.0, |c| c.light_offset()));
    let want = if alpha > 0.001 { Color::srgba(0.0, 0.0, 0.0, alpha) } else { Color::NONE };
    for mut bg in &mut veil {
        if bg.0 != want {
            bg.0 = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_intro_darkens_to_a_fifth() {
        assert_eq!(veil_alpha(0.0), 0.0);
        // −0.8: a fifth of the brightness, in gamma terms.
        assert!((1.0 - veil_alpha(-0.8) - 0.2f32.dpowf(2.2)).abs() < 1e-6);
        assert_eq!(veil_alpha(-2.0), 1.0);
    }
}
