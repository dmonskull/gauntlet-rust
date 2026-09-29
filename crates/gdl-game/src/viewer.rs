//! `--viewer`: look at one player class and play through its actions.
//! `[` / `]` change action, `Tab` / `Shift+Tab` change class.

use bevy::prelude::*;
use gdl_install::GameInstall;

use crate::camera::FlyCamera;
use crate::character::{self, Animator};
use crate::level_material::LevelMaterial;

pub struct ViewerPlugin;

#[derive(Resource)]
pub struct Viewer {
    pub install: GameInstall,
    pub classes: Vec<String>,
    pub class: usize,
    pub variant: String,
    pub action: Option<String>,
    status: String,
}

impl Viewer {
    pub fn new(install: GameInstall, class: Option<&str>, variant: Option<&str>, action: Option<&str>) -> Result<Self, String> {
        let classes = character::player_classes(&install);
        if classes.is_empty() {
            return Err("No player classes found under PLAYERS/.".into());
        }
        let class = match class {
            Some(c) => classes
                .iter()
                .position(|x| x.eq_ignore_ascii_case(c))
                .ok_or_else(|| format!("No class '{c}'. Classes: {}", classes.join(", ")))?,
            None => 0,
        };
        Ok(Self {
            install,
            classes,
            class,
            variant: variant.unwrap_or("BLU").to_ascii_uppercase(),
            action: action.map(str::to_string),
            status: String::new(),
        })
    }
}

#[derive(Component)]
struct ViewerCharacter;

#[derive(Component)]
struct ViewerText;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_text, respawn).chain())
            .add_systems(Update, (keys, show_status));
    }
}

fn spawn_text(mut commands: Commands) {
    commands.spawn((
        ViewerText,
        Text::new(""),
        TextFont { font_size: 15.0, ..default() },
        TextShadow::default(),
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(12.0), ..default() },
    ));
}

#[allow(clippy::too_many_arguments)]
fn respawn(
    mut commands: Commands,
    mut viewer: ResMut<Viewer>,
    old: Query<Entity, With<ViewerCharacter>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut camera: Query<(&mut Transform, &mut FlyCamera)>,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    let class = viewer.classes[viewer.class].clone();
    let variant = viewer.variant.clone();
    match character::load_player(&mut viewer.install, &class, &variant) {
        Ok(data) => {
            let root = character::spawn_character(
                &data,
                Transform::default(),
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut images,
            );
            commands.entity(root).insert(ViewerCharacter);
            if let Some(name) = viewer.action.clone() {
                commands.entity(root).insert(PendingAction(name));
            }
            viewer.status = data.name;
        }
        Err(e) => viewer.status = format!("{class}/{variant}: {e}"),
    }
    if let Ok((mut transform, mut fly)) = camera.single_mut() {
        *fly = FlyCamera::looking_at_bounds(Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 6.0, 5.0), &mut transform);
    }
}

/// Action requested on the command line, applied once the animator exists.
#[derive(Component)]
struct PendingAction(String);

#[allow(clippy::too_many_arguments)]
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut viewer: ResMut<Viewer>,
    mut animators: Query<(Entity, &mut Animator, Option<&PendingAction>)>,
    old: Query<Entity, With<ViewerCharacter>>,
    meshes: ResMut<Assets<Mesh>>,
    materials: ResMut<Assets<LevelMaterial>>,
    images: ResMut<Assets<Image>>,
    camera: Query<(&mut Transform, &mut FlyCamera)>,
) {
    for (e, mut a, pending) in &mut animators {
        if let Some(PendingAction(name)) = pending {
            a.play_named(name);
            commands.entity(e).remove::<PendingAction>();
        }
        let n = a.clips.actions.len();
        if keys.just_pressed(KeyCode::BracketRight) {
            let next = (a.action + 1) % n;
            a.play(next);
        }
        if keys.just_pressed(KeyCode::BracketLeft) {
            let prev = (a.action + n - 1) % n;
            a.play(prev);
        }
    }
    if keys.just_pressed(KeyCode::Tab) {
        let n = viewer.classes.len();
        let back = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        viewer.class = if back { (viewer.class + n - 1) % n } else { (viewer.class + 1) % n };
        viewer.action = None;
        respawn(commands, viewer, old, meshes, materials, images, camera);
    }
}

fn show_status(viewer: Res<Viewer>, animators: Query<&Animator>, mut text: Query<&mut Text, With<ViewerText>>) {
    let Ok(mut text) = text.single_mut() else { return };
    let action = animators
        .iter()
        .next()
        .map(|a| {
            let act = &a.clips.actions[a.action];
            format!(
                "{} ({}/{})  frame {:.1}/{}  rate {}{}",
                act.name,
                a.action + 1,
                a.clips.actions.len(),
                a.frame,
                act.frames,
                act.rate,
                if act.loops() { "  loop" } else { "" }
            )
        })
        .unwrap_or_default();
    text.0 = format!(
        "Character viewer: {}\n{action}\n\n[ ] action   Tab / Shift+Tab class   WASD / right-drag camera",
        viewer.status
    );
}
