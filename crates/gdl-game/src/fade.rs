//! An object drawn part see-through: the game's transparency (object
//! `+0x53`, set over a model tree; `docs/rendering.md`), which blends the
//! object at 255 − transparency. Here a [`Fade`] on an entity draws its
//! meshes through blended copies of their materials while it's above 0 —
//! made the first time a mesh shows a material, dropped when it's whole
//! again — so opaque models can fade too (the tower's exits revealed,
//! `tower_scenes.rs`).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::character::Animator;
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

/// `level.wgsl`'s mode for a texture in place of a part's own, with its
/// coordinates from the normals (the chrome).
const CHROME_MODE: f32 = 3.0;

/// A hero's body (its model's meshes, not its shadow) drawn through copies
/// of its materials: with a texture in place of each part's own — the
/// game's texture override −3 with draw flag `0x80000`, which the chrome
/// power-ups use: coordinates from each point's normal, along the camera's
/// right and up — or through a death texture's frame (override −4, as a
/// dying monster: the death light as a hero goes out) — and/or see-through
/// by `fade` (the object's transparency / 255: invisibility). A part gets
/// its copy the first time it shows a material; one whose material changes
/// meanwhile is copied again.
#[derive(Default)]
pub struct BodyLook {
    /// What the copies show: the texture, whether through a death texture,
    /// and whether they're faded.
    shown: Option<(Option<AssetId<Image>>, bool, bool)>,
    fade: f32,
    /// The death texture's frame the copies show.
    frame: Option<AssetId<Image>>,
    /// A part's own material → its copy.
    copies: HashMap<AssetId<LevelMaterial>, Handle<LevelMaterial>>,
    /// A copy → the material it stands in for.
    originals: HashMap<AssetId<LevelMaterial>, Handle<LevelMaterial>>,
}

impl BodyLook {
    /// Shows `texture`, the death texture's frame `dying` and `fade` on the
    /// body's parts (none: their own again).
    pub fn show(
        &mut self,
        texture: Option<&Handle<Image>>,
        dying: Option<&Handle<Image>>,
        fade: f32,
        animator: &Animator,
        drawn: &mut Query<&mut MeshMaterial3d<LevelMaterial>>,
        materials: &mut Assets<LevelMaterial>,
    ) {
        let key = (texture.map(Handle::id), dying.is_some(), fade > 0.0);
        let wanted = (key.0.is_some() || key.1 || key.2).then_some(key);
        if wanted != self.shown {
            // Back on their own materials first.
            for &(_, e) in animator.meshes() {
                let Ok(mut m) = drawn.get_mut(e) else { continue };
                if let Some(own) = self.originals.get(&m.0.id()) {
                    m.0 = own.clone();
                }
            }
            self.copies.clear();
            self.originals.clear();
            self.shown = wanted;
            self.fade = fade;
            self.frame = dying.map(Handle::id);
        }
        if wanted.is_none() {
            return;
        }
        for &(_, e) in animator.meshes() {
            let Ok(mut m) = drawn.get_mut(e) else { continue };
            let own = m.0.id();
            if self.originals.contains_key(&own) {
                continue;
            }
            let copy = match self.copies.get(&own) {
                Some(c) => c.clone(),
                None => {
                    let Some(own_material) = materials.get(own) else { continue };
                    let mut copy = match dying {
                        Some(frame) => own_material.dissolving(frame.clone()),
                        None => own_material.clone(),
                    };
                    if let Some(texture) = texture {
                        copy.diffuse = Some(texture.clone());
                        copy.params.x = CHROME_MODE;
                    }
                    if fade > 0.0 {
                        // Solid parts blend, still hiding their own far
                        // sides.
                        if matches!(copy.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_)) {
                            copy.widen_alpha(AlphaMode::Blend);
                            copy.blend_depth_write = true;
                        }
                        copy.uv_offset.w = fade;
                    }
                    let c = materials.add(copy);
                    self.copies.insert(own, c.clone());
                    self.originals.insert(c.id(), m.0.clone());
                    c
                }
            };
            m.0 = copy;
        }
        if let Some(frame) = dying
            && self.frame != Some(frame.id())
        {
            self.frame = Some(frame.id());
            for c in self.copies.values() {
                if let Some(m) = materials.get_mut(c) {
                    m.lightmap = Some(frame.clone());
                }
            }
        }
        if fade != self.fade {
            self.fade = fade;
            for c in self.copies.values() {
                if let Some(m) = materials.get_mut(c) {
                    m.uv_offset.w = fade;
                }
            }
        }
    }
}
