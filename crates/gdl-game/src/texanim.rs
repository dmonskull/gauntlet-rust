//! Animated and scrolling textures (`gdl_formats::texmod`) — the level's,
//! and those of the banks its items and generators are drawn from:
//! flipbook frames step on the game's 30 Hz tick; scrolls are evaluated
//! every frame between ticks so they glide.

use std::collections::HashMap;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::Atree;
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

/// Render flag `0x10` on a node: modifiers run on the nodes above it
/// don't reach it or anything under it.
const MOD_BOUNDARY: u32 = 0x10;

/// A texture modifier an animation runs, with its flipbook's frames.
struct RunMod {
    texmod: TexMod,
    book: Option<Flipbook>,
}

/// The texture modifiers a model's animations run, as every animated
/// object in the game does each step (`docs/rendering.md`, "Actions"): the
/// playing action's own on the first node and everything under it, then
/// each kind-3 node's on that node and everything under it. A flipbook sets
/// an object's one replaced texture (the last to run wins), a fade its
/// opacity; both stay until something changes them.
pub struct ModelMods {
    /// Per action, its modifiers.
    actions: Vec<Vec<RunMod>>,
    /// The nodes the actions' modifiers reach.
    action_reach: Vec<usize>,
    /// Kind-3 nodes, in the order the game walks them: the nodes each
    /// reaches and its modifier.
    nodes: Vec<(Vec<usize>, RunMod)>,
    /// The texture binding each of the model's materials draws.
    pub bindings: HashMap<AssetId<LevelMaterial>, u16>,
}

/// What the modifiers have left on an object: the texture put in place of
/// one of its bindings (with the blending its flipbook needs), and its
/// opacity from a fade (0–255).
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Look {
    pub texture: Option<(u16, Handle<Image>, AlphaMode)>,
    pub alpha: Option<u8>,
}

impl ModelMods {
    /// The modifiers `atree`'s actions and kind-3 nodes run, from
    /// `texmods` (the list of the file it came from); `book` finds a
    /// flipbook's frames, and `drawn` is the model's materials by binding.
    /// None when it runs none.
    pub fn new(
        atree: &Atree,
        texmods: &[TexMod],
        mut book: impl FnMut(&TexMod) -> Option<Flipbook>,
        drawn: &HashMap<u16, Vec<Handle<LevelMaterial>>>,
    ) -> Option<Self> {
        let mut run = |k: usize| texmods.get(k).map(|m| RunMod { texmod: m.clone(), book: book(m) });
        let actions: Vec<Vec<RunMod>> = atree.actions.iter().map(|a| a.texmods().filter_map(&mut run).collect()).collect();
        let nodes: Vec<(Vec<usize>, RunMod)> =
            atree.texmod_nodes.iter().filter_map(|&(n, k)| Some((reach(atree, n), run(k)?))).collect();
        if nodes.is_empty() && actions.iter().all(Vec::is_empty) {
            return None;
        }
        let bindings = drawn.iter().flat_map(|(&b, hs)| hs.iter().map(move |h| (h.id(), b))).collect();
        let action_reach = if atree.nodes.is_empty() { Vec::new() } else { reach(atree, 0) };
        Some(Self { actions, action_reach, nodes, bindings })
    }

    /// Runs the modifiers at `frame` of `action` (the animation's frame) on
    /// the looks of the model's nodes. The frame is rounded, and counted
    /// from the end on an action that runs backwards; the action's own
    /// modifiers wrap round a looping action, the nodes' don't.
    pub fn run(&self, atree: &Atree, action: usize, frame: f32, looks: &mut [Look]) {
        let Some(a) = atree.actions.get(action) else { return };
        let last = i32::from(a.frames) - 1;
        let mut f = ((frame + 0.5) as i32).min(last).max(0);
        if a.backwards() {
            f = last - f;
        }
        let mut g = f;
        for m in self.actions.get(action).into_iter().flatten() {
            let length = i32::from(m.texmod.count) * m.texmod.period as i32;
            if length < g && a.params[0] != 0 && length > 1 {
                g %= length;
            }
            m.apply(g, &self.action_reach, looks);
        }
        for (reach, m) in &self.nodes {
            m.apply(f, reach, looks);
        }
    }
}

impl RunMod {
    fn apply(&self, frame: i32, reach: &[usize], looks: &mut [Look]) {
        let m = &self.texmod;
        match m.kind {
            TexModKind::Frames(_) => {
                let Some(book) = &self.book else { return };
                let Some(Some(image)) = book.frames.get(m.action_frame(frame) as usize) else { return };
                for &n in reach {
                    if let Some(look) = looks.get_mut(n) {
                        look.texture = Some((m.binding, image.clone(), book.alpha));
                    }
                }
            }
            TexModKind::FadeIn | TexModKind::FadeOut => {
                // The game keeps how clear the object is as a byte.
                let t = m.fade(frame);
                let clear = if m.kind == TexModKind::FadeIn { 1.0 - t } else { t };
                let alpha = 255 - (clear * 255.0) as u8;
                for &n in reach {
                    if let Some(look) = looks.get_mut(n) {
                        look.alpha = Some(alpha);
                    }
                }
            }
            // Stand-in: an action's scrolls (texture wipes) aren't run.
            _ => {}
        }
    }
}

/// The nodes a modifier run on `node` reaches: the node, then its
/// children and theirs — none of them if the first child is a boundary
/// ([`MOD_BOUNDARY`]), and past it only the children that aren't.
fn reach(atree: &Atree, node: usize) -> Vec<usize> {
    let children = |p: usize| atree.nodes.iter().enumerate().filter(move |(_, n)| n.parent == Some(p)).map(|(i, _)| i);
    let mut out = vec![node];
    let mut stack = vec![node];
    while let Some(p) = stack.pop() {
        let kids: Vec<usize> = children(p).collect();
        if kids.first().is_none_or(|&c| atree.nodes[c].render_flags & MOD_BOUNDARY != 0) {
            continue;
        }
        for c in kids.into_iter().filter(|&c| atree.nodes[c].render_flags & MOD_BOUNDARY == 0) {
            out.push(c);
            stack.push(c);
        }
    }
    out
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
