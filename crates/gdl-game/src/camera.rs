//! Free-fly camera for looking around a level: WASD to move, Space/Ctrl (or
//! E/Q) for up/down, hold the right mouse button to look, Shift to go fast,
//! mouse wheel to change speed.

use bevy::camera::{CameraProjection, SubCameraView};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::math::Vec3A;
use bevy::prelude::*;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FreeLook>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, fly.run_if(|free: Res<FreeLook>| free.0));
    }
}

/// Whether the fly camera has the controls (otherwise something else, like
/// the player, drives the camera).
#[derive(Resource)]
pub struct FreeLook(pub bool);

impl Default for FreeLook {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Component)]
pub struct FlyCamera {
    yaw: f32,
    pitch: f32,
    /// Units per second.
    speed: f32,
}

impl FlyCamera {
    /// Frames an axis-aligned box from above and in front, the way the game
    /// looks down on its levels, and returns the matching controller state.
    pub fn looking_at_bounds(min: Vec3, max: Vec3, transform: &mut Transform) -> Self {
        let (center, extent) = if min.x <= max.x {
            ((min + max) / 2.0, (max - min).max_element().max(1.0))
        } else {
            (Vec3::ZERO, 100.0)
        };
        let eye = center + Vec3::new(0.0, extent * 0.45, extent * 0.55);
        *transform = Transform::from_translation(eye).looking_at(center, Vec3::Y);
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        Self { yaw, pitch, speed: (extent * 0.25).clamp(10.0, 500.0) }
    }
}

impl FlyCamera {
    /// Fits a box in view (vertical FOV 45°, 16:9) from slightly above the
    /// front — for looking at one character rather than a whole level.
    pub fn framing(min: Vec3, max: Vec3, transform: &mut Transform) -> Self {
        let center = (min + max) / 2.0;
        let size = max - min;
        let fit = size.y.max(size.x.max(size.z) / 1.6).max(0.5);
        let distance = fit * 0.5 / (22.5f32.to_radians()).tan() * 1.15;
        let eye = center + Vec3::new(0.0, 0.3, 1.0).normalize() * (distance + size.z * 0.5);
        *transform = Transform::from_translation(eye).looking_at(center, Vec3::Y);
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        Self { yaw, pitch, speed: (fit * 0.5).clamp(1.0, 500.0) }
    }

    /// Looks down at `target` from behind it (against its +Z) and above.
    pub fn behind(target: &Transform, transform: &mut Transform) -> Self {
        let back = -(target.rotation * Vec3::Z);
        let eye = target.translation + back * 22.0 + Vec3::Y * 18.0;
        *transform = Transform::from_translation(eye).looking_at(target.translation, Vec3::Y);
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        Self { yaw, pitch, speed: 20.0 }
    }
}

impl FlyCamera {
    /// Takes over from wherever the camera is now.
    pub fn sync(&mut self, transform: &Transform) {
        (self.yaw, self.pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
    }
}

/// The game's picture. Its data is right-handed (Y up, counter-clockwise
/// fronts), but it shows it through a left-handed view: a character faces
/// +Z with its right hand at +X (`R_WRIST` x > 0), and on the tower's
/// start the orange crystals (+X) are to the hero's left. A plain
/// right-handed render is the mirror image of that, so the 3D camera draws
/// with our perspective flipped left to right (`docs/rendering.md`,
/// "Handedness"): the level materials then take clockwise triangles as
/// front faces, and the stick's right is +X when facing +Z.
#[derive(Debug, Clone, Default)]
pub struct MirroredPerspective(pub PerspectiveProjection);

/// Flips clip space left to right.
fn mirror() -> Mat4 {
    Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))
}

impl CameraProjection for MirroredPerspective {
    fn get_clip_from_view(&self) -> Mat4 {
        mirror() * self.0.get_clip_from_view()
    }

    fn get_clip_from_view_for_sub(&self, sub_view: &SubCameraView) -> Mat4 {
        mirror() * self.0.get_clip_from_view_for_sub(sub_view)
    }

    fn update(&mut self, width: f32, height: f32) {
        self.0.update(width, height);
    }

    fn far(&self) -> f32 {
        self.0.far()
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        // The same frustum, seen the other way round.
        self.0.get_frustum_corners(z_near, z_far)
    }
}

fn spawn_camera(mut commands: Commands) {
    let mut transform = Transform::default();
    let fly = FlyCamera::looking_at_bounds(Vec3::splat(-100.0), Vec3::splat(100.0), &mut transform);
    // No tonemapping: level colours are the game's own, already final.
    let projection = Projection::custom(MirroredPerspective::default());
    commands.spawn((Camera3d::default(), projection, Tonemapping::None, transform, fly));
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut camera: Query<(&mut Transform, &mut FlyCamera)>,
) {
    let Ok((mut transform, mut fly)) = camera.single_mut() else {
        return;
    };

    if scroll.delta.y != 0.0 {
        fly.speed = (fly.speed * 1.15f32.powf(scroll.delta.y)).clamp(1.0, 2000.0);
    }
    if buttons.pressed(MouseButton::Right) {
        // The picture is mirrored: dragging right turns the other way.
        fly.yaw += motion.delta.x * 0.003;
        fly.pitch = (fly.pitch - motion.delta.y * 0.003).clamp(-1.54, 1.54);
    }
    transform.rotation = Quat::from_euler(EulerRot::YXZ, fly.yaw, fly.pitch, 0.0);

    let axis = |pos: &[KeyCode], neg: &[KeyCode]| {
        (pos.iter().any(|k| keys.pressed(*k)) as i32 - neg.iter().any(|k| keys.pressed(*k)) as i32) as f32
    };
    let forward = axis(&[KeyCode::KeyW, KeyCode::ArrowUp], &[KeyCode::KeyS, KeyCode::ArrowDown]);
    let right = axis(&[KeyCode::KeyD, KeyCode::ArrowRight], &[KeyCode::KeyA, KeyCode::ArrowLeft]);
    let up = axis(&[KeyCode::Space, KeyCode::KeyE], &[KeyCode::ControlLeft, KeyCode::KeyQ]);
    // Mirrored picture: the screen's right is the camera's left.
    let dir = transform.forward() * forward + transform.left() * right + Vec3::Y * up;
    if dir != Vec3::ZERO {
        let boost = if keys.pressed(KeyCode::ShiftLeft) { 4.0 } else { 1.0 };
        transform.translation += dir.normalize() * fly.speed * boost * time.delta_secs();
    }
}
