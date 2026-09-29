//! Free-fly camera for looking around a level: WASD to move, Space/Ctrl (or
//! E/Q) for up/down, hold the right mouse button to look, Shift to go fast,
//! mouse wheel to change speed.

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera).add_systems(Update, fly);
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
}

fn spawn_camera(mut commands: Commands) {
    let mut transform = Transform::default();
    let fly = FlyCamera::looking_at_bounds(Vec3::splat(-100.0), Vec3::splat(100.0), &mut transform);
    // No tonemapping: level colours are the game's own, already final.
    commands.spawn((Camera3d::default(), Tonemapping::None, transform, fly));
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
        fly.yaw -= motion.delta.x * 0.003;
        fly.pitch = (fly.pitch - motion.delta.y * 0.003).clamp(-1.54, 1.54);
    }
    transform.rotation = Quat::from_euler(EulerRot::YXZ, fly.yaw, fly.pitch, 0.0);

    let axis = |pos: &[KeyCode], neg: &[KeyCode]| {
        (pos.iter().any(|k| keys.pressed(*k)) as i32 - neg.iter().any(|k| keys.pressed(*k)) as i32) as f32
    };
    let forward = axis(&[KeyCode::KeyW, KeyCode::ArrowUp], &[KeyCode::KeyS, KeyCode::ArrowDown]);
    let right = axis(&[KeyCode::KeyD, KeyCode::ArrowRight], &[KeyCode::KeyA, KeyCode::ArrowLeft]);
    let up = axis(&[KeyCode::Space, KeyCode::KeyE], &[KeyCode::ControlLeft, KeyCode::KeyQ]);
    let dir = transform.forward() * forward + transform.right() * right + Vec3::Y * up;
    if dir != Vec3::ZERO {
        let boost = if keys.pressed(KeyCode::ShiftLeft) { 4.0 } else { 1.0 };
        transform.translation += dir.normalize() * fly.speed * boost * time.delta_secs();
    }
}
