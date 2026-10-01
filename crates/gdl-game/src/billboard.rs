//! Instances that turn to face the camera, from their render flags'
//! facing mode (`docs/rendering.md`): bushes and trees turn about Y, sprites
//! and glows copy the camera's rotation.

use gdl_formats::detmath::Det;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use gdl_formats::world::render_flags;

pub struct BillboardPlugin;

impl Plugin for BillboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, face_camera.before(TransformSystems::Propagate));
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub enum Billboard {
    /// Turn about Y so +Z points at the camera.
    Yaw,
    /// Also tilt toward the camera, at most this far (radians; `None`:
    /// no limit).
    YawPitch(Option<f32>),
    /// Take the camera's own rotation.
    Sprite,
    /// Turn about Y so +Z points back along the camera's view (parallel to
    /// the screen, wherever on it): mode `0x2000000`, a critter's 3D
    /// health meter.
    ScreenYaw,
}

impl Billboard {
    pub fn from_flags(flags: u32) -> Option<Self> {
        match flags & render_flags::FACING_MASK {
            0x100_0000 => Some(Self::Yaw),
            0x200_0000 => Some(Self::ScreenYaw),
            0x300_0000 => Some(Self::YawPitch(None)),
            0x400_0000 => Some(Self::Sprite),
            0x500_0000 => Some(Self::YawPitch(Some(15f32.to_radians()))),
            0x600_0000 => Some(Self::YawPitch(Some(30f32.to_radians()))),
            0x700_0000 => Some(Self::YawPitch(Some(45f32.to_radians()))),
            _ => None,
        }
    }

    /// World rotation for an object at `at` seen from a camera at `eye`
    /// with rotation `camera`.
    pub fn rotation(self, at: Vec3, eye: Vec3, camera: Quat) -> Quat {
        let d = eye - at;
        let yaw = Quat::from_rotation_y(d.x.datan2(d.z));
        match self {
            Self::Yaw => yaw,
            Self::YawPitch(limit) => {
                let mut pitch = d.y.datan2(Vec2::new(d.x, d.z).length());
                if let Some(l) = limit {
                    pitch = pitch.clamp(-l, l);
                }
                yaw * Quat::from_rotation_x(-pitch)
            }
            Self::Sprite => camera,
            Self::ScreenYaw => {
                // The camera looks along its −Z.
                let back = camera * Vec3::Z;
                Quat::from_rotation_y(back.x.datan2(back.z))
            }
        }
    }
}

fn face_camera(
    camera: Query<&Transform, (With<Camera3d>, Without<Billboard>)>,
    parents: Query<&GlobalTransform>,
    mut billboards: Query<(&Billboard, &mut Transform, Option<&ChildOf>)>,
) {
    let Ok(cam) = camera.single() else { return };
    for (mode, mut t, parent) in &mut billboards {
        let parent = parent.and_then(|p| parents.get(p.parent()).ok()).map(|g| g.compute_transform());
        let world_at = parent.map_or(t.translation, |p| p.transform_point(t.translation));
        let world = mode.rotation(world_at, cam.translation, cam.rotation);
        t.rotation = match parent {
            Some(p) => p.rotation.inverse() * world,
            None => world,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_from_flags() {
        assert_eq!(Billboard::from_flags(0x1001800), Some(Billboard::Yaw));
        assert_eq!(Billboard::from_flags(0x4c01880), Some(Billboard::Sprite));
        assert_eq!(Billboard::from_flags(0x5c01880), Some(Billboard::YawPitch(Some(15f32.to_radians()))));
        assert_eq!(Billboard::from_flags(0x800), None);
    }

    #[test]
    fn yaw_points_plus_z_at_the_camera() {
        let r = Billboard::Yaw.rotation(Vec3::ZERO, Vec3::new(10.0, 5.0, 0.0), Quat::IDENTITY);
        let z = r * Vec3::Z;
        assert!((z - Vec3::X).length() < 1e-5, "{z}");
        let r = Billboard::YawPitch(Some(0.1)).rotation(Vec3::ZERO, Vec3::new(0.0, 10.0, 1.0), Quat::IDENTITY);
        let z = r * Vec3::Z;
        assert!((z.y - 0.1f32.dsin()).abs() < 1e-5, "{z}");
    }

    /// Mode 0x2000000 faces back along the view, wherever the object is.
    #[test]
    fn screen_yaw_faces_back_along_the_view() {
        assert_eq!(Billboard::from_flags(0x2000000), Some(Billboard::ScreenYaw));
        // A camera looking along +Z (its −Z turned half round), tilted down.
        let camera = Quat::from_rotation_y(std::f32::consts::PI) * Quat::from_rotation_x(-0.5);
        for at in [Vec3::ZERO, Vec3::new(30.0, 2.0, 5.0)] {
            let z = Billboard::ScreenYaw.rotation(at, Vec3::new(0.0, 10.0, -40.0), camera) * Vec3::Z;
            assert!((z - Vec3::NEG_Z).length() < 1e-5, "{z}");
        }
    }
}
