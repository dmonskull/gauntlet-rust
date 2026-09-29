//! `--viewer`: look at one player class or monster and play through its
//! actions. `[` / `]` change action, `Tab` / `Shift+Tab` change character.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use gdl_install::GameInstall;

use crate::camera::FlyCamera;
use crate::character::{self, Animator};
use crate::level_material::LevelMaterial;

pub struct ViewerPlugin;

#[derive(Resource)]
pub struct Viewer {
    pub install: GameInstall,
    /// Player classes, or monster folders when `monsters` is set.
    pub classes: Vec<String>,
    pub class: usize,
    pub monsters: bool,
    pub variant: String,
    pub action: Option<String>,
    status: String,
}

impl Viewer {
    pub fn new(
        install: GameInstall,
        class: Option<&str>,
        monster: Option<&str>,
        variant: Option<&str>,
        action: Option<&str>,
    ) -> Result<Self, String> {
        let monsters = monster.is_some();
        let (classes, want, what) = if monsters {
            (character::monster_names(&install), monster, "monster")
        } else {
            (character::player_classes(&install), class, "class")
        };
        if classes.is_empty() {
            return Err(format!("No {what}s found on the disc."));
        }
        let class = match want {
            Some(c) => classes
                .iter()
                .position(|x| x.eq_ignore_ascii_case(c))
                .ok_or_else(|| format!("No {what} '{c}'. Choices: {}", classes.join(", ")))?,
            None => 0,
        };
        Ok(Self {
            install,
            classes,
            class,
            monsters,
            variant: variant.unwrap_or("BLU").to_ascii_uppercase(),
            action: action.map(str::to_string),
            status: String::new(),
        })
    }
}

#[derive(Component)]
struct ViewerCharacter;

/// Frames to wait before framing the camera on the drawn meshes.
#[derive(Resource)]
struct FrameAfter(u32);

/// Frames the camera on the world bounds of every visible mesh.
fn frame_camera(
    mut commands: Commands,
    wait: Option<ResMut<FrameAfter>>,
    parts: Query<(&Aabb, &GlobalTransform, &InheritedVisibility)>,
    mut camera: Query<(&mut Transform, &mut FlyCamera)>,
) {
    let Some(mut wait) = wait else { return };
    if wait.0 > 0 {
        wait.0 -= 1;
        return;
    }
    commands.remove_resource::<FrameAfter>();
    let (mut min, mut max) = (Vec3::MAX, Vec3::MIN);
    for (aabb, global, visible) in &parts {
        if !visible.get() {
            continue;
        }
        let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for corner in [-1.0f32, 1.0].into_iter().flat_map(|x| [-1.0f32, 1.0].map(move |y| (x, y))) {
            for z in [-1.0f32, 1.0] {
                let p = global.transform_point(c + h * Vec3::new(corner.0, corner.1, z));
                min = min.min(p);
                max = max.max(p);
            }
        }
    }
    if min.x > max.x {
        return;
    }
    if let Ok((mut transform, mut fly)) = camera.single_mut() {
        *fly = FlyCamera::framing(min, max, &mut transform);
    }
}

#[derive(Component)]
struct ViewerText;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_text, respawn).chain())
            .add_systems(Update, (keys, show_status, frame_camera));
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
    let loaded = if viewer.monsters {
        character::load_monster(&mut viewer.install, &class)
    } else {
        character::load_player(&mut viewer.install, &class, &variant)
    };
    match loaded {
        Ok(data) => {
            let (root, min, max) = character::spawn_character(
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
            // Rough framing now; `frame_camera` refines it once the posed
            // meshes have world bounds.
            if let Ok((mut transform, mut fly)) = camera.single_mut() {
                *fly = FlyCamera::looking_at_bounds(min, max, &mut transform);
            }
            commands.insert_resource(FrameAfter(3));
        }
        Err(e) => viewer.status = format!("{class}: {e}"),
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
        "Character viewer: {}\n{action}\n\n[ ] action   Tab / Shift+Tab character   WASD / right-drag camera",
        viewer.status
    );
}
