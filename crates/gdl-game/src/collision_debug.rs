//! Debug overlay for level collision: every collision triangle drawn as a
//! translucent face with bright edges over the rendered level. `K` toggles
//! it; `GDL_DEBUG_COLLISION=1` starts with it on.
//!
//! Colours follow what the game's queries treat each face as (see
//! `docs/collision.md`): green = floor only (normal Y ≥ 0.866), yellow =
//! both floor and wall (0.5 ≤ Y < 0.866), red = wall only (|Y| < 0.5),
//! blue = facing down. Triangles of nodes that move are drawn at rest in
//! magenta.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use gdl_formats::LevelCollision;

use crate::world::LevelEntity;

pub struct CollisionDebugPlugin;

impl Plugin for CollisionDebugPlugin {
    fn build(&self, app: &mut App) {
        let visible = std::env::var("GDL_DEBUG_COLLISION").is_ok_and(|v| !v.is_empty() && v != "0");
        app.insert_resource(CollisionOverlay { visible }).add_systems(Update, toggle);
    }
}

#[derive(Resource)]
pub struct CollisionOverlay {
    pub visible: bool,
}

#[derive(Component)]
struct OverlayPart;

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<CollisionOverlay>,
    mut parts: Query<&mut Visibility, With<OverlayPart>>,
) {
    if crate::dev_keys() && keys.just_pressed(KeyCode::KeyK) {
        overlay.visible = !overlay.visible;
    }
    let want = if overlay.visible { Visibility::Visible } else { Visibility::Hidden };
    for mut v in &mut parts {
        v.set_if_neq(want);
    }
}

fn colour(normal: [f32; 3], moves: bool) -> [f32; 3] {
    let y = normal[1];
    match () {
        _ if moves => [1.0, 0.2, 1.0],
        _ if y >= 0.866 => [0.2, 1.0, 0.3],
        _ if y >= 0.5 => [1.0, 0.9, 0.2],
        _ if y > -0.5 => [1.0, 0.25, 0.2],
        _ => [0.3, 0.5, 1.0],
    }
}

/// Spawns the overlay for a level (hidden unless the overlay is on).
pub fn spawn_overlay(
    collision: &LevelCollision,
    visible: bool,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let (mut face_pos, mut face_col) = (Vec::new(), Vec::new());
    let (mut line_pos, mut line_col) = (Vec::new(), Vec::new());
    for (node, tri, v) in collision.world_triangles() {
        let normal = collision.triangles[tri].normal;
        let [r, g, b] = colour(normal, collision.nodes[node].moves());
        // Nudged off the surface along the normal so it doesn't z-fight
        // with the level geometry it traces.
        let lift = Vec3::from(normal) * 0.03;
        let p = v.map(|c| (Vec3::from(c) + lift).to_array());
        face_pos.extend(p);
        face_col.extend([[r, g, b, 0.28]; 3]);
        for (a, b2) in [(0, 1), (1, 2), (2, 0)] {
            line_pos.extend([p[a], p[b2]]);
            line_col.extend([[r, g, b, 1.0]; 2]);
        }
    }
    let visibility = if visible { Visibility::Visible } else { Visibility::Hidden };

    let faces = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, face_pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, face_col);
    commands.spawn((
        Mesh3d(meshes.add(faces)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            double_sided: true,
            depth_bias: 50.0,
            ..default()
        })),
        visibility,
        OverlayPart,
        LevelEntity,
    ));

    let lines = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, line_pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, line_col);
    commands.spawn((
        Mesh3d(meshes.add(lines)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            depth_bias: 100.0,
            ..default()
        })),
        visibility,
        OverlayPart,
        LevelEntity,
    ));
}
