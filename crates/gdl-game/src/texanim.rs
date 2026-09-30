//! Animated and scrolling textures (`gdl_formats::texmod`) — the level's,
//! and those of the banks its items and generators are drawn from:
//! flipbook frames step on the game's 30 Hz tick; scrolls are evaluated
//! every frame between ticks so they glide.

use std::collections::HashMap;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::texmod::{FirstFrame, TexMod, TexModKind};

use crate::level_material::LevelMaterial;
use crate::model_mesh::TextureCache;

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

/// A bank's texture modifiers on the materials drawing its textures
/// (`drawn`, by binding), set up the game's way (`docs/rendering.md`,
/// "Texture animation"): every flipbook first puts its first frame on its
/// texture — the last listed wins — and the free-running ones (owner −1)
/// then step on the frame counter, or scroll. The rest belong to actions
/// (`items.rs`).
pub fn bank_anims(
    texmods: &[TexMod],
    drawn: &HashMap<u16, Vec<Handle<LevelMaterial>>>,
    mut frames: impl FnMut(&TexMod) -> Option<Flipbook>,
    materials: &mut Assets<LevelMaterial>,
) -> Vec<TexAnim> {
    let mut anims = Vec::new();
    for m in texmods {
        let Some(drawn) = drawn.get(&m.binding) else { continue };
        let free = m.owner == -1;
        match m.kind {
            TexModKind::Frames(_) => {
                let Some(book) = frames(m) else { continue };
                for h in drawn {
                    let Some(material) = materials.get_mut(h) else { continue };
                    if let Some(Some(first)) = book.frames.first() {
                        material.diffuse = Some(first.clone());
                    }
                    material.widen_alpha(book.alpha);
                }
                if free {
                    anims.push(TexAnim::new(m.clone(), drawn.clone(), book.frames));
                }
            }
            TexModKind::ScrollU | TexModKind::ScrollV if free => anims.push(TexAnim::new(m.clone(), drawn.clone(), Vec::new())),
            _ => {}
        }
    }
    anims
}

/// A flipbook's frame images (missing ones `None`), and the blending the
/// most demanding of them needs.
pub struct Flipbook {
    pub frames: Vec<Option<Handle<Image>>>,
    pub alpha: AlphaMode,
}

/// A flipbook's frames: `count` bindings on from its first frame, in the
/// bank that has it — its own (`model`, through `cache`), or for a first
/// frame given by name that the bank lacks, the always-loaded `WEAPONS`
/// (`shared`).
pub fn flipbook_images(
    m: &TexMod,
    model: &ModelFile,
    cache: &mut TextureCache,
    shared: Option<(&ModelFile, &mut TextureCache)>,
    images: &mut Assets<Image>,
) -> Option<Flipbook> {
    let n = m.count.unsigned_abs();
    let run = |first: u16, cache: &mut TextureCache, images: &mut Assets<Image>| {
        let mut alpha = AlphaMode::Opaque;
        let frames = (0..n)
            .map(|k| {
                let (image, mode) = cache.get(first + k, images)?;
                alpha = match (alpha, mode) {
                    (_, AlphaMode::Blend) | (AlphaMode::Blend, _) => AlphaMode::Blend,
                    (_, AlphaMode::Mask(c)) | (AlphaMode::Mask(c), _) => AlphaMode::Mask(c),
                    _ => alpha,
                };
                Some(image)
            })
            .collect();
        Flipbook { frames, alpha }
    };
    match &m.kind {
        TexModKind::Frames(FirstFrame::Binding(first)) => Some(run(*first, cache, images)),
        TexModKind::Frames(FirstFrame::Named(name)) => {
            let find = |model: &ModelFile| model.texture_names.iter().find(|t| &t.name == name).map(|t| t.binding);
            if let Some(first) = find(model) {
                return Some(run(first, cache, images));
            }
            let (model, cache) = shared?;
            Some(run(find(model)?, cache, images))
        }
        _ => None,
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

    /// Adds animations for models made after the level (its monsters').
    pub fn extend(&mut self, anims: impl IntoIterator<Item = TexAnim>) {
        self.anims.extend(anims);
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
            _ => continue,
        };
        let offset = a.texmod.scroll(ticks);
        for h in &a.materials {
            if let Some(m) = materials.get_mut(h) {
                m.uv_offset[axis] = offset;
            }
        }
    }
}
