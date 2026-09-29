//! Animated and scrolling level textures (`gdl_formats::texmod`): flipbook
//! frames step on the game's 30 Hz tick; scrolls are evaluated every frame
//! between ticks so they glide.

use bevy::prelude::*;
use gdl_formats::texmod::{TexMod, TexModKind};

use crate::level_material::LevelMaterial;

pub struct TexAnimPlugin;

impl Plugin for TexAnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, step_frames).add_systems(Update, scroll);
    }
}

pub struct TexAnim {
    pub texmod: TexMod,
    /// Materials drawing the modified texture.
    pub materials: Vec<Handle<LevelMaterial>>,
    /// Flipbooks: the image for each frame (missing frames skipped).
    pub frames: Vec<Option<Handle<Image>>>,
    shown: Option<u32>,
}

impl TexAnim {
    pub fn new(texmod: TexMod, materials: Vec<Handle<LevelMaterial>>, frames: Vec<Option<Handle<Image>>>) -> Self {
        Self { texmod, materials, frames, shown: None }
    }
}

/// The current level's texture animations.
#[derive(Resource, Default)]
pub struct LevelTexAnims {
    pub anims: Vec<TexAnim>,
    ticks: u64,
}

impl LevelTexAnims {
    pub fn new(anims: Vec<TexAnim>) -> Self {
        Self { anims, ticks: 0 }
    }
}

fn step_frames(anims: Option<ResMut<LevelTexAnims>>, mut materials: ResMut<Assets<LevelMaterial>>) {
    let Some(mut anims) = anims else { return };
    anims.ticks += 1;
    let ticks = anims.ticks;
    for a in &mut anims.anims {
        if !matches!(a.texmod.kind, TexModKind::Frames(_)) {
            continue;
        }
        let frame = a.texmod.frame(ticks);
        if a.shown == Some(frame) {
            continue;
        }
        a.shown = Some(frame);
        let Some(Some(image)) = a.frames.get(frame as usize) else { continue };
        for h in &a.materials {
            if let Some(m) = materials.get_mut(h) {
                m.diffuse = Some(image.clone());
            }
        }
    }
}

fn scroll(fixed: Res<Time<Fixed>>, anims: Option<Res<LevelTexAnims>>, mut materials: ResMut<Assets<LevelMaterial>>) {
    let Some(anims) = anims else { return };
    let ticks = anims.ticks as f64 + fixed.overstep_fraction() as f64;
    for a in &anims.anims {
        let axis = match a.texmod.kind {
            TexModKind::ScrollU => 0,
            TexModKind::ScrollV => 1,
            TexModKind::Frames(_) => continue,
        };
        let offset = a.texmod.scroll(ticks);
        for h in &a.materials {
            if let Some(m) = materials.get_mut(h) {
                m.uv_offset[axis] = offset;
            }
        }
    }
}
