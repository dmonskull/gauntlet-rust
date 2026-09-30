//! An object drawn part see-through: the game's transparency (object
//! `+0x53`, set over a model tree; `docs/rendering.md`), which blends the
//! object at 255 − transparency. Here a [`Fade`] on an entity draws its
//! meshes through blended copies of their materials while it's above 0 —
//! made the first time a mesh shows a material, dropped when it's whole
//! again — so opaque models can fade too (the tower's exits revealed,
//! `tower_scenes.rs`).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::level_material::LevelMaterial;

pub struct FadePlugin;

impl Plugin for FadePlugin {
    fn build(&self, app: &mut App) {
        // After the animation and item systems that swap materials (Update).
        app.add_systems(PostUpdate, draw_fades);
    }
}

/// How faded an entity's model is: 0 whole … 1 gone.
#[derive(Component, Default)]
pub struct Fade {
    pub amount: f32,
    shown: f32,
    /// A mesh's own material → this entity's faded copy of it.
    copies: HashMap<AssetId<LevelMaterial>, Handle<LevelMaterial>>,
    /// A copy → the material it stands in for.
    originals: HashMap<AssetId<LevelMaterial>, Handle<LevelMaterial>>,
}

impl Fade {
    pub fn new(amount: f32) -> Self {
        Self { amount, ..default() }
    }
}

fn draw_fades(
    mut fades: Query<(Entity, &mut Fade)>,
    children: Query<&Children>,
    mut drawn: Query<&mut MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
) {
    for (root, mut f) in &mut fades {
        let f = &mut *f;
        if f.amount <= 0.0 {
            if f.copies.is_empty() {
                continue;
            }
            // Whole again: back on their own materials.
            for e in children.iter_descendants(root) {
                let Ok(mut m) = drawn.get_mut(e) else { continue };
                if let Some(own) = f.originals.get(&m.0.id()) {
                    m.0 = own.clone();
                }
            }
            f.copies.clear();
            f.originals.clear();
            f.shown = 0.0;
            continue;
        }
        let changed = f.amount != f.shown;
        f.shown = f.amount;
        for e in children.iter_descendants(root) {
            let Ok(mut m) = drawn.get_mut(e) else { continue };
            let id = m.0.id();
            if f.originals.contains_key(&id) {
                continue;
            }
            let copy = match f.copies.get(&id) {
                Some(c) => c.clone(),
                None => {
                    let Some(mut own) = materials.get(id).cloned() else { continue };
                    // Solid parts blend (still hiding their own far sides);
                    // glows stay as they are.
                    if matches!(own.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                        own.widen_alpha(AlphaMode::Blend);
                        own.blend_depth_write = true;
                    }
                    own.uv_offset.w = f.amount;
                    let c = materials.add(own);
                    f.copies.insert(id, c.clone());
                    f.originals.insert(c.id(), m.0.clone());
                    c
                }
            };
            m.0 = copy;
        }
        if changed {
            for c in f.copies.values() {
                if let Some(m) = materials.get_mut(c) {
                    m.uv_offset.w = f.amount;
                }
            }
        }
    }
}
