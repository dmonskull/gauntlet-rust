//! How a regular monster dies (`docs/monsters.md` "Deaths"). The killing
//! blow puts it in the game's dying state: no more AI, not a target, it
//! slides on its knockback and plays DEATH — a body without one its HIT2
//! knock-down, else READY's animation — while it's drawn through a ten-frame
//! death texture picked by what killed it (blood, fire, electricity, light,
//! acid, or a knight's or tree's own). The texture tints the body and eats
//! it away over 20 ticks, then it's gone. Small monsters (floor step at
//! most 2) have no death texture and go at once. Elemental kills, and
//! knights, also leave a die effect model where they fell.
//!
//! The monster tick (`monsters.rs`) runs the dying state; this module has
//! the game's choices, the textures and the drawing.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::texmod::{FirstFrame, TexMod, TexModKind};
use gdl_install::GameInstall;

use crate::character::Animate;
use crate::level_material::LevelMaterial;
use crate::model_mesh::TextureCache;
use crate::monsters::{BIG_STEP, Monster};

pub struct DeathsPlugin;

impl Plugin for DeathsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeathTextures>().add_systems(Update, dissolve_bodies.after(Animate));
    }
}

/// A death texture's frames.
pub type Frames = Arc<[Handle<Image>]>;

/// How far the death texture's counter runs: it starts at −0.5, steps 0.5
/// a tick and shows frame `counter` (truncated); the body goes when it
/// gets here (the game's timed-texture effect, set up by the kill with a
/// step of 0.5 and an end of 10).
pub const DEATH_STEPS: f32 = 10.0;
/// What the counter starts at and adds each 30 Hz tick.
pub const DEATH_START: f32 = -0.5;
pub const DEATH_STEP: f32 = 0.5;

const KNIGHT: i32 = 5;
const TREE: i32 = 0xB;
const ACID_BLOB: i32 = 0x15;
const GOLEM: i32 = 0x1D;

/// Which death texture a body goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathSet {
    /// `WEAPONS`' set for the killing blow's element (`kind & 0xF`): 0
    /// DEATHBLOOD, 1 DEATHFIRE, 2 DEATHELEC, 3 DEATHLIGHT, 4 DEATHACID.
    Element(usize),
    /// The level's DEATHALT: the tree's, else the knight's.
    Alt,
}

/// The light's set: the heroes go out through it (`going_out.rs`).
pub const LIGHT: DeathSet = DeathSet::Element(3);

/// The death texture a monster of type `enemy` with this floor step gets
/// when a blow of `kind` kills it (the game's monster damage routine):
/// knights and trees their own set for plain blows; any
/// other monster bigger than a step of 2 the set for the blow's element;
/// small monsters none. Elements above 4 (none of the game's blows) take
/// blood here.
pub fn death_set(enemy: i32, step: f32, kind: u32) -> Option<DeathSet> {
    let element = (kind & 0xF) as usize;
    match enemy {
        KNIGHT | TREE if element == 0 => Some(DeathSet::Alt),
        _ if step > BIG_STEP => Some(DeathSet::Element(if element <= 4 { element } else { 0 })),
        _ => None,
    }
}

/// The die effect model the game leaves (from its effect list, on a
/// kill): FIREDIE, ELECDIE, LIGHTDIE or ACIDDIE by the blow's element;
/// a plain kill leaves HITDIE on knights, a blood spray (BLOODFX2) on
/// anything else but trees and acid blobs, which leave nothing.
pub fn die_effect(enemy: i32, kind: u32) -> Option<&'static str> {
    match (kind & 0xF, enemy) {
        (0, KNIGHT | GOLEM) => Some("HITDIE"),
        (0, TREE | ACID_BLOB) => None,
        (0, _) => Some("BLOODFX2"),
        (1, _) => Some("FIREDIE"),
        (2, _) => Some("ELECDIE"),
        (3, _) => Some("LIGHTDIE"),
        (4, _) => Some("ACIDDIE"),
        _ => None,
    }
}

/// The effect a blow that doesn't kill leaves: a blood spray (BLOODFX1),
/// HITCOL on knights for plain blows; FIREHIT for fire, HITCOL for the
/// other elements; trees and acid blobs nothing for plain blows.
pub fn hit_effect(enemy: i32, kind: u32) -> Option<&'static str> {
    match (kind & 0xF, enemy) {
        (0, KNIGHT | GOLEM) => Some("HITCOL"),
        (0, TREE | ACID_BLOB) => None,
        (0, _) => Some("BLOODFX1"),
        (1, _) => Some("FIREHIT"),
        (2..=4, _) => Some("HITCOL"),
        _ => None,
    }
}

/// Where a monster's hit or die effect goes: at the blow for big ones
/// (a floor step of 4 or more), else at the monster (stand-in: its
/// centre, for the game's `+0x44` point).
pub fn effect_origin(step: f32, centre: Vec3, blow: Vec3) -> Vec3 {
    if step >= 4.0 { blow } else { centre }
}

/// The die effect's scale: half the monster's floor step; knights' (and
/// the golem's) are drawn at 1.
pub fn die_effect_scale(enemy: i32, step: f32) -> f32 {
    match enemy {
        KNIGHT | GOLEM => 1.0,
        _ => 0.5 * step,
    }
}

/// Effects draw at this alpha: the game sets their transparency to 96
/// (`+0x53` = 255 − 96).
pub const EFFECT_ALPHA: f32 = 159.0 / 255.0;

/// The death textures: `WEAPONS`' five (loaded once) and the level's
/// DEATHALT.
#[derive(Resource, Default)]
pub struct DeathTextures {
    by_element: Option<[Option<Frames>; 5]>,
    alt: Option<Frames>,
}

/// The element sets' names in `WEAPONS/ANIM.PS2`.
const ELEMENT_SETS: [&str; 5] = ["DEATHBLOOD", "DEATHFIRE", "DEATHELEC", "DEATHLIGHT", "DEATHACID"];

impl DeathTextures {
    pub fn frames(&self, set: DeathSet) -> Option<Frames> {
        match set {
            DeathSet::Element(e) => self.by_element.as_ref()?.get(e)?.clone(),
            DeathSet::Alt => self.alt.clone(),
        }
    }

    /// Loads `WEAPONS`' sets the first time.
    pub fn load_shared(&mut self, install: &mut GameInstall, images: &mut Assets<Image>) {
        if self.by_element.is_some() {
            return;
        }
        let sets = (|| {
            let model = ModelFile::parse(&install.read("WEAPONS/objects.ngc").ok()?).ok()?;
            let textures = install.read("WEAPONS/textures.ngc").ok()?;
            let texmods = TexMod::parse_all(&install.read("WEAPONS/ANIM.PS2").ok()?).ok()?;
            let mut cache = TextureCache::new(&model, &textures);
            Some(ELEMENT_SETS.map(|name| flipbook(&texmods, name, &model, &mut cache, images)))
        })();
        match &sets {
            Some(s) => info!("death textures: {:?}", s.iter().map(|f| f.as_ref().map_or(0, |f| f.len())).collect::<Vec<_>>()),
            None => warn!("death textures: WEAPONS isn't readable"),
        }
        self.by_element = Some(sets.unwrap_or_default());
    }

    /// The level's DEATHALT from the first monster folder (model, textures,
    /// texture modifiers) that has one: the tree's folders come first.
    pub fn set_alt<'a>(
        &mut self,
        folders: impl IntoIterator<Item = (&'a ModelFile, &'a [u8], &'a [TexMod])>,
        images: &mut Assets<Image>,
    ) {
        self.alt = folders.into_iter().find_map(|(model, textures, texmods)| {
            let mut cache = TextureCache::new(model, textures);
            flipbook(texmods, "DEATHALT", model, &mut cache, images)
        });
    }
}

/// A texture modifier's flipbook frames, decoded from its model file.
fn flipbook(
    texmods: &[TexMod],
    name: &str,
    model: &ModelFile,
    cache: &mut TextureCache,
    images: &mut Assets<Image>,
) -> Option<Frames> {
    let t = texmods.iter().find(|t| t.name == name)?;
    let first = match &t.kind {
        TexModKind::Frames(FirstFrame::Binding(b)) => *b,
        TexModKind::Frames(FirstFrame::Named(n)) => model.texture_names.iter().find(|x| x.name == *n)?.binding,
        _ => return None,
    };
    let count = u16::try_from(t.count).ok()?;
    let frames: Vec<Handle<Image>> = (0..count).filter_map(|k| cache.get(first + k, images).map(|(h, _)| h)).collect();
    (!frames.is_empty()).then(|| frames.into())
}

/// A dying monster's body drawn through its death texture: its own copies
/// of the materials it's drawn with, showing the counter's frame.
#[derive(Component)]
pub struct Dissolve {
    frames: Frames,
    shown: Option<usize>,
    /// Original material → this body's copy.
    copies: HashMap<AssetId<LevelMaterial>, Handle<LevelMaterial>>,
}

impl Dissolve {
    pub fn new(frames: Frames) -> Self {
        Self { frames, shown: None, copies: HashMap::new() }
    }
}

/// Puts each dying body's meshes on its copies of their materials (after
/// the animator has swapped in the flipbook frame's own) and moves the
/// copies to the counter's frame.
fn dissolve_bodies(
    mut bodies: Query<(Entity, &Monster, &mut Dissolve)>,
    children: Query<&Children>,
    mut drawn: Query<&mut MeshMaterial3d<LevelMaterial>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
) {
    for (root, m, mut d) in &mut bodies {
        let Some(frame) = m.death_frame() else { continue };
        let frame = frame.min(d.frames.len() - 1);
        let image = d.frames[frame].clone();
        if d.shown != Some(frame) {
            d.shown = Some(frame);
            for copy in d.copies.values() {
                if let Some(mat) = materials.get_mut(copy) {
                    mat.lightmap = Some(image.clone());
                }
            }
        }
        for e in children.iter_descendants(root) {
            let Ok(mut mat) = drawn.get_mut(e) else { continue };
            let id = mat.0.id();
            if d.copies.values().any(|c| c.id() == id) {
                continue;
            }
            let copy = match d.copies.get(&id) {
                Some(c) => c.clone(),
                None => {
                    let Some(own) = materials.get(id).map(|m| m.dissolving(image.clone())) else { continue };
                    let c = materials.add(own);
                    d.copies.insert(id, c.clone());
                    c
                }
            };
            mat.0 = copy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deaths_follow_the_games_choices() {
        // A grunt (step 3) killed by a plain blow bleeds; by fire, burns.
        assert_eq!(death_set(4, 3.0, 0), Some(DeathSet::Element(0)));
        assert_eq!(death_set(4, 3.0, 0x201), Some(DeathSet::Element(1)));
        // A rat (step 1.5) just goes.
        assert_eq!(death_set(3, 1.5, 0), None);
        // Knights and trees have their own for plain blows only.
        assert_eq!(death_set(KNIGHT, 3.0, 0), Some(DeathSet::Alt));
        assert_eq!(death_set(TREE, 3.0, 4), Some(DeathSet::Element(4)));
        assert_eq!(die_effect(4, 0), Some("BLOODFX2"));
        assert_eq!(die_effect(KNIGHT, 0), Some("HITDIE"));
        assert_eq!(die_effect(TREE, 0), None);
        assert_eq!(die_effect(TREE, 2), Some("ELECDIE"));
        assert_eq!(hit_effect(4, 0), Some("BLOODFX1"));
        assert_eq!(hit_effect(KNIGHT, 0), Some("HITCOL"));
        assert_eq!(hit_effect(4, 1), Some("FIREHIT"));
        assert_eq!(die_effect_scale(4, 3.0), 1.5);
        assert_eq!(die_effect_scale(KNIGHT, 3.0), 1.0);
    }

    #[test]
    fn the_counter_shows_each_frame_for_two_ticks() {
        // −0.5, then +0.5 a tick: frames 0..9 two ticks each, gone on the
        // 21st tick.
        let mut step = DEATH_START;
        let mut shown = Vec::new();
        loop {
            step += DEATH_STEP;
            if step >= DEATH_STEPS {
                break;
            }
            shown.push(step as usize);
        }
        assert_eq!(shown.len(), 20);
        assert_eq!(&shown[..4], &[0, 0, 1, 1]);
        assert_eq!(shown[19], 9);
    }
}
