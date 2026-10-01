//! Shows what populates a level — player starts, generators, monsters,
//! pickups, doors, triggers, exits (`docs/level-population.md`) — as
//! coloured markers and, where the item's model can be found, the model.
//!
//! `I` (with `GDL_DEV_KEYS=1`) cycles models + markers / models / markers / hidden; `GDL_POPULATION`
//! (`all`, `models`, `markers`, `off`; default `models`) picks the starting view.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::enemy::LevelEnemies;
use gdl_formats::anim::{AnimFile, Atree};
use gdl_formats::texmod::{TexMod, TexModKind};
use gdl_formats::population::{
    ENEMY_CODES, ItemClass, ItemType, LocatorKind, PlacementParams, PlayerStart, Population, rotation_matrix,
};
use gdl_install::GameInstall;

use crate::level::LevelData;
use crate::level_material::LevelMaterial;
use crate::billboard::Billboard;
use crate::model_mesh::{self, TextureCache};
use crate::texanim::{self, TexAnim};
use crate::world::LevelEntity;

pub struct PopulationPlugin;

impl Plugin for PopulationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PopulationView::from_env()).add_systems(Update, toggle_view);
    }
}

/// Which parts of the population are drawn.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopulationView {
    All,
    Models,
    Markers,
    Hidden,
}

impl PopulationView {
    fn from_env() -> Self {
        match std::env::var("GDL_POPULATION").unwrap_or_default().to_ascii_lowercase().as_str() {
            "all" => Self::All,
            "markers" => Self::Markers,
            "off" | "none" | "0" | "hidden" => Self::Hidden,
            // Playing: the items as the game draws them, no debug markers.
            _ => Self::Models,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Models,
            Self::Models => Self::Markers,
            Self::Markers => Self::Hidden,
            Self::Hidden => Self::All,
        }
    }

    fn shows(self, part: PopulationPart) -> bool {
        match part {
            PopulationPart::Marker => matches!(self, Self::All | Self::Markers),
            PopulationPart::Model => matches!(self, Self::All | Self::Models),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "models + markers",
            Self::Models => "models",
            Self::Markers => "markers",
            Self::Hidden => "hidden",
        }
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum PopulationPart {
    Marker,
    Model,
}

/// On a placement's model: the index of its placement in
/// [`Population::placements`], so gameplay can find the model it drives.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlacementIndex(pub usize);

/// On a model built from an atree: one entity per atree node (in node
/// order, each a child of its parent node's), carrying that node's part at
/// its rest offset — for whatever plays the atree's actions on it.
#[derive(Component)]
pub struct ItemRig {
    pub atree: Arc<Atree>,
    pub bones: Vec<Entity>,
    /// Flipbook nodes: the entity holding the frame shown, the node, the
    /// frames and how they face the camera.
    pub flipbooks: Vec<(Entity, usize, FlipbookFrames, Option<Billboard>)>,
    /// What its actions do to its textures, and the parts drawing a
    /// texture they change: (entity, binding, the shared material).
    pub texmods: Option<Arc<ActionTexMods>>,
    pub texmod_parts: Vec<(Entity, u16, Handle<LevelMaterial>)>,
}

/// The texture modifiers each of an atree's actions runs, in order (a
/// force field's generator lighting up, a transporter's swirl), with each
/// flipbook's frame images (`docs/rendering.md`, "Texture animation").
pub struct ActionTexMods {
    pub actions: Vec<Vec<ActionTexMod>>,
}

/// One modifier an action runs, and its flipbook's frames (none for a
/// fade).
pub type ActionTexMod = (TexMod, Vec<Option<Handle<Image>>>);

impl ActionTexMods {
    /// The textures (bindings) their flipbooks change.
    fn bindings(&self) -> Vec<u16> {
        let mut out: Vec<u16> = self
            .actions
            .iter()
            .flatten()
            .filter(|(m, _)| matches!(m.kind, TexModKind::Frames(_)))
            .map(|(m, _)| m.binding)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A flipbook node's frames (a barrel's idle, breaking and broken looks):
/// the meshes of each frame of each action.
pub type FlipbookFrames = Arc<Vec<Vec<Vec<model_mesh::BuiltMesh>>>>;

/// The current level's population, and where its players start — for
/// whatever spawns the playable characters.
#[derive(Resource)]
pub struct LevelPopulation {
    pub level: String,
    pub population: Population,
    /// The start the heroes arrive at ([`start_entry`]).
    pub entry: i16,
}

/// The tower's order of the realms: its start `i` is for arriving from
/// realm `TOWER_ORDER[i]` — the tower itself (a new game) at the centre,
/// then G, B, A, K, D, C, I, J, E, F, H beside their gates (the realm is
/// the last one played outside the tower); a realm's boss beaten marks
/// the bit of its place (`quest::boss_marks`).
pub const TOWER_ORDER: [u32; 12] = [13, 7, 2, 1, 11, 4, 3, 9, 10, 5, 6, 8];
const TOWER_REALM: u32 = 13;

/// The start entry for arriving in `level` after last playing in realm
/// `last_realm`: in the tower, the one for that realm; elsewhere 0.
pub fn start_entry(level: &str, last_realm: Option<u32>) -> i16 {
    if crate::quest::level_of(level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        return 0;
    }
    let realm = last_realm.unwrap_or(TOWER_REALM);
    TOWER_ORDER.iter().position(|&r| r == realm).map_or(0, |i| i as i16)
}

impl LevelPopulation {
    /// The start the heroes arrive at (entry 0 when that one's missing).
    pub fn player_start(&self) -> Option<PlayerStart> {
        self.population.player_start(self.entry)
    }
}

/// Where a start puts the players: at its position, turned by its yaw the
/// way the game turns placed things (`rotation_matrix`). Which model axis
/// counts as "forward" for a player isn't confirmed yet; the game derives
/// items' facing from where their +Z axis ends up.
pub fn start_transform(start: &PlayerStart) -> Transform {
    Transform { translation: Vec3::from(start.position), rotation: game_rotation([0.0, start.yaw, 0.0]), ..default() }
}

/// Where a lookout (locator kinds 8 and 10) puts what stands on it: the
/// game turns its X angle round and adds half a turn to its Y angle, then
/// builds the matrix with its third Euler builder (sines negated), whose
/// yaw turns +Z toward (sin, cos) — the other way round from placements'
/// (`docs/level-population.md`, "Locators").
pub fn lookout_transform(l: &gdl_formats::population::Locator) -> Transform {
    let m = locator_matrix([-l.rotation[0], l.rotation[1] + std::f32::consts::PI, l.rotation[2]]);
    Transform { translation: Vec3::from(l.position), rotation: Quat::from_mat3(&Mat3::from_cols_array(&m)), ..default() }
}

/// The game's Euler builder for locators (lookouts, the boss's spot):
/// row-major for row vectors, so read as columns it's the column-vector
/// matrix. Its yaw turns +Z toward (sin, cos): row 2 is where +Z goes.
pub fn locator_matrix(r: [f32; 3]) -> [f32; 9] {
    let (cx, sx) = (r[0].cos(), -r[0].sin());
    let (cy, sy) = (r[1].cos(), -r[1].sin());
    let (cz, sz) = (r[2].cos(), -r[2].sin());
    [
        cy * cz - (sy * sx) * sz,
        cx * sz,
        sy * cz + (cy * sx) * sz,
        -cy * sz - (sy * sx) * cz,
        cx * cz,
        -sy * sz + (cy * sx) * cz,
        -sy * cx,
        -sx,
        cy * cx,
    ]
}

/// The angles [`locator_matrix`] builds `m` from — the game's own
/// matrix-to-angles routine for that builder, which a falling item's spin
/// goes through each update (`items.rs`): z from row 0's X and Y, x from
/// row 2's Y, y from row 2; within 1e-4 of straight up or down x is ±π/2,
/// z 0 and y from row 0.
pub fn locator_euler(m: [f32; 9]) -> [f32; 3] {
    if (1.0 - m[7].abs()).abs() < 1e-4 {
        let x = if m[7] <= 0.0 { -std::f32::consts::FRAC_PI_2 } else { std::f32::consts::FRAC_PI_2 };
        return [x, (-m[2]).atan2(m[0]), 0.0];
    }
    let z = (-m[1]).atan2(m[4]);
    let cz = z.cos();
    if cz == 0.0 {
        return if z <= 0.0 { [m[7].atan2(m[1]), (-m[5]).atan2(m[3]), z] } else { [m[7].atan2(-m[1]), m[5].atan2(-m[3]), z] };
    }
    let cx = m[4] / cz;
    [m[7].atan2(cx), (m[6] / cx).atan2(m[8] / cx), z]
}

fn game_rotation(euler: [f32; 3]) -> Quat {
    // Row-major for row vectors, read as columns = the column-vector matrix.
    Quat::from_mat3(&Mat3::from_cols_array(&rotation_matrix(euler)))
}

fn toggle_view(
    keys: Res<ButtonInput<KeyCode>>,
    mut view: ResMut<PopulationView>,
    mut parts: Query<(&PopulationPart, &mut Visibility)>,
) {
    if crate::dev_keys() && keys.just_pressed(KeyCode::KeyI) {
        *view = view.next();
    }
    if view.is_changed() {
        for (part, mut vis) in &mut parts {
            *vis = visibility(*view, *part);
        }
    }
}

fn visibility(view: PopulationView, part: PopulationPart) -> Visibility {
    if view.shows(part) { Visibility::Inherited } else { Visibility::Hidden }
}

/// What a marker stands for; picks its shape and colour.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Category {
    PlayerStart,
    Boss,
    Generator,
    Monster,
    Key,
    Gold,
    Food,
    Potion,
    Powerup,
    Container,
    Door,
    Trigger,
    Hazard,
    Exit,
    Transporter,
    Obstacle,
    Other,
}

impl Category {
    fn of(ty: &ItemType) -> Self {
        match ty.class {
            ItemClass::Generator => Self::Generator,
            ItemClass::EnemyInfo => Self::Monster,
            ItemClass::Powerup => match ty.subtype_name() {
                Some("KEY") => Self::Key,
                Some("GOLD") => Self::Gold,
                Some("FOOD") => Self::Food,
                Some("POTION") => Self::Potion,
                _ => Self::Powerup,
            },
            ItemClass::Container => Self::Container,
            ItemClass::Door => Self::Door,
            ItemClass::Trigger | ItemClass::Rotator => Self::Trigger,
            ItemClass::Trap | ItemClass::DamageTile => Self::Hazard,
            ItemClass::Exit => Self::Exit,
            ItemClass::Transporter => Self::Transporter,
            ItemClass::Obstacle => Self::Obstacle,
            _ => Self::Other,
        }
    }

    fn color(self) -> Color {
        match self {
            Self::PlayerStart => Color::srgb(0.1, 1.0, 0.2),
            Self::Boss => Color::srgb(1.0, 0.0, 0.4),
            Self::Generator => Color::srgb(1.0, 0.1, 0.1),
            Self::Monster => Color::srgb(1.0, 0.55, 0.0),
            Self::Key => Color::srgb(1.0, 1.0, 0.0),
            Self::Gold => Color::srgb(1.0, 0.8, 0.3),
            Self::Food => Color::srgb(0.6, 1.0, 0.4),
            Self::Potion => Color::srgb(0.2, 0.9, 1.0),
            Self::Powerup => Color::srgb(1.0, 0.3, 1.0),
            Self::Container => Color::srgb(0.65, 0.4, 0.2),
            Self::Door => Color::srgb(0.9, 0.9, 0.7),
            Self::Trigger => Color::srgb(0.5, 0.5, 0.6),
            Self::Hazard => Color::srgb(0.6, 0.0, 0.0),
            Self::Exit => Color::WHITE,
            Self::Transporter => Color::srgb(0.6, 0.3, 1.0),
            Self::Obstacle => Color::srgb(0.8, 0.7, 0.5),
            Self::Other => Color::srgb(0.3, 0.5, 1.0),
        }
    }

    /// Marker mesh, sitting on the origin (its base at y = 0).
    fn mesh(self) -> Mesh {
        match self {
            Self::PlayerStart => Cone { radius: 1.2, height: 3.0 }.into(),
            Self::Boss => Sphere::new(2.5).into(),
            Self::Generator => Cuboid::new(2.0, 2.0, 2.0).into(),
            Self::Monster => Capsule3d::new(0.6, 1.2).into(),
            Self::Exit => Torus::new(1.4, 2.2).into(),
            Self::Transporter => Torus::new(1.0, 1.6).into(),
            Self::Door => Cuboid::new(0.6, 3.0, 2.0).into(),
            Self::Trigger | Self::Hazard => Cylinder::new(1.0, 0.3).into(),
            Self::Container | Self::Obstacle => Cuboid::new(1.0, 1.0, 1.0).into(),
            _ => Sphere::new(0.6).into(),
        }
    }

    /// How far up to lift the marker so it sits on the floor.
    fn lift(self) -> f32 {
        match self {
            Self::PlayerStart => 1.5,
            Self::Boss => 2.5,
            Self::Generator => 1.0,
            Self::Monster => 1.2,
            Self::Exit | Self::Transporter => 0.4,
            Self::Door => 1.5,
            Self::Trigger | Self::Hazard => 0.15,
            Self::Container | Self::Obstacle => 0.5,
            _ => 0.6,
        }
    }
}

/// A folder's models, atrees and texture modifiers (`objects.ngc`,
/// `textures.ngc`, `ANIM.PS2`).
struct Source {
    model: ModelFile,
    textures: Vec<u8>,
    atrees: Vec<Arc<Atree>>,
    texmods: Vec<TexMod>,
}

impl Source {
    fn load(install: &mut GameInstall, dir: &str) -> Option<Self> {
        let model = ModelFile::parse(&install.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
        let textures = install.read(&format!("{dir}/textures.ngc")).ok()?;
        let anim = install.read(&format!("{dir}/ANIM.PS2")).ok();
        let atrees = anim
            .as_deref()
            .and_then(|a| AnimFile::parse(a).ok())
            .map_or_else(Vec::new, |a| a.atrees.into_iter().map(Arc::new).collect());
        let texmods = anim.as_deref().and_then(|a| TexMod::parse_all(a).ok()).unwrap_or_default();
        Some(Self { model, textures, atrees, texmods })
    }
}

/// Where item models come from, in the game's search order: the realm's
/// items, the shared powerups, the level's own items
/// (`ITEMS/level<realm letter>`, `POWERUPS`, `ITEMS/<level>`); generators
/// also look in their monster's folder. Plain object lookups search the
/// level's own models too.
/// One model source: its models, textures, atrees and texture modifiers.
#[derive(Clone, Copy)]
struct SourceRef<'a> {
    model: &'a ModelFile,
    textures: &'a [u8],
    atrees: &'a [Arc<Atree>],
    texmods: &'a [TexMod],
}

impl<'a> SourceRef<'a> {
    fn of(s: &'a Source) -> Self {
        Self { model: &s.model, textures: &s.textures, atrees: &s.atrees, texmods: &s.texmods }
    }
}

struct Sources<'a> {
    list: Vec<SourceRef<'a>>,
    objects: Vec<HashMap<&'a str, usize>>,
    /// Indices into `list` of the item folders.
    items: Vec<usize>,
    level: usize,
    monsters: HashMap<&'static str, usize>,
}

impl<'a> Sources<'a> {
    fn new(level: &'a LevelData, items: &'a [Source], monsters: &'a [(&'static str, Source)]) -> Self {
        let mut list: Vec<SourceRef> = items.iter().map(SourceRef::of).collect();
        let item_indices = (0..list.len()).collect();
        list.push(SourceRef { model: &level.model, textures: &level.textures, atrees: &[], texmods: &level.texmods });
        let level_index = list.len() - 1;
        let mut monster_indices = HashMap::new();
        for (code, s) in monsters {
            monster_indices.insert(*code, list.len());
            list.push(SourceRef::of(s));
        }
        let objects = list
            .iter()
            .map(|s| s.model.objects.iter().enumerate().map(|(i, o)| (o.name.as_str(), i)).collect())
            .collect();
        Self { list, objects, items: item_indices, level: level_index, monsters: monster_indices }
    }

    /// The model the game would draw for `name`: an atree of that name
    /// (each node's object is `<atree><node>`, posed by the node), else an
    /// object named `name`, `name` + `L1` or `name` + `ROOT`.
    fn resolve(&self, name: &str, extra: Option<usize>) -> Option<Resolved> {
        if name.is_empty() {
            return None;
        }
        let atree_order: Vec<usize> = self.items.iter().copied().chain(extra).collect();
        for &s in &atree_order {
            let Some(atree) = self.list[s].atrees.iter().find(|a| a.name == name) else { continue };
            let parts: Vec<(usize, usize)> = (0..atree.nodes.len())
                .filter_map(|i| {
                    // A flipbook node (a barrel's) shows its first action's
                    // first frame — the idle look — until it's animated.
                    let object = match atree.flipbook_entry(i, 0) {
                        Some(entry) => entry.first.clone(),
                        None => format!("{}{}", atree.name, atree.nodes[i].name),
                    };
                    self.objects[s].get(object.as_str()).map(|&o| (i, o))
                })
                .collect();
            if !parts.is_empty() {
                return Some(Resolved { source: s, atree: Some(atree.clone()), parts });
            }
        }
        let object_order: Vec<usize> = atree_order.iter().copied().chain([self.level]).collect();
        // The game appends as it goes (`docs/level-population.md`): the
        // name, then with `L1`, then with `L1ROOT` (the lizards' generators
        // are `GEN_LIZ<n>L1ROOT`).
        for suffix in ["", "L1", "L1ROOT"] {
            let full = format!("{name}{suffix}");
            for &s in &object_order {
                if let Some(&o) = self.objects[s].get(full.as_str()) {
                    return Some(Resolved { source: s, atree: None, parts: vec![(0, o)] });
                }
            }
        }
        None
    }
}

/// A model found for a name: its source, and either an atree's parts as
/// `(node, object)` or a single plain object (node 0, unused).
struct Resolved {
    source: usize,
    atree: Option<Arc<Atree>>,
    parts: Vec<(usize, usize)>,
}

/// A model's meshes, built once per name and shared by its placements:
/// per atree node (or one entry for a plain object), with how it faces the
/// camera.
struct BuiltModel {
    /// The source it came from (none when nothing was found).
    source: Option<usize>,
    atree: Option<Arc<Atree>>,
    parts: Vec<(usize, Vec<model_mesh::BuiltMesh>, Option<Billboard>)>,
    /// Flipbook nodes and their frames by action (action 0's first frame
    /// is the node's part).
    flipbooks: Vec<(usize, FlipbookFrames)>,
    /// What its atree's actions do to its textures.
    texmods: Option<Arc<ActionTexMods>>,
}

impl BuiltModel {
    /// Every mesh it can show: its parts and each flipbook frame.
    fn meshes(&self) -> impl Iterator<Item = &model_mesh::BuiltMesh> {
        let parts = self.parts.iter().flat_map(|(_, p, _)| p.iter());
        parts.chain(self.flipbooks.iter().flat_map(|(_, f)| f.iter().flatten().flatten()))
    }
}

/// A generator model's meshes for each strength level (index = level − 1),
/// for stepping it down as it's damaged.
#[derive(Component)]
pub struct GeneratorLooks(pub Vec<Vec<(Handle<Mesh>, Handle<LevelMaterial>)>>);

/// Builds the model the game would draw for `name` (see `Sources::resolve`).
#[allow(clippy::too_many_arguments)]
fn build_model(
    sources: &Sources,
    caches: &mut [TextureCache],
    name: &str,
    monster: Option<usize>,
    meshes: &mut Assets<Mesh>,
    level_materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> BuiltModel {
    let Some(r) = sources.resolve(name, monster) else {
        return BuiltModel { source: None, atree: None, parts: Vec::new(), flipbooks: Vec::new(), texmods: None };
    };
    let mut bounds = (Vec3::MAX, Vec3::MIN);
    let file = sources.list[r.source].model;
    let parts = r
        .parts
        .iter()
        .filter_map(|&(node, object)| {
            // Each atree node draws with its render flags (blending, facing),
            // like a character's; hidden nodes and a chest's contents node
            // (a placeholder card the game hides) don't draw. Unlike a
            // hero's `CFGLOW`, which the effects system textures at run
            // time, an item's glow is an ordinary part with its own texture
            // (the red gem's `CFXP_GLOW` ring, the green potion's
            // `XP_GLOW`; the others' names are cut short of "GLOW").
            let flags = match &r.atree {
                Some(atree) => {
                    let n = &atree.nodes[node];
                    if !part_drawn(n) {
                        return None;
                    }
                    n.render_flags
                }
                None => 0,
            };
            let at = [(object, Vec3::ZERO, flags)];
            let built = model_mesh::build_flagged(file, &mut caches[r.source], at, meshes, level_materials, images, &mut bounds);
            Some((node, built, Billboard::from_flags(flags)))
        })
        .filter(|(_, m, _)| !m.is_empty())
        .collect();
    // Flipbook nodes: every frame of every action, the frame objects in a
    // row from each action's first.
    let mut flipbooks = Vec::new();
    if let Some(atree) = &r.atree {
        for node in 0..atree.nodes.len() {
            if atree.flipbook_entry(node, 0).is_none() {
                continue;
            }
            let flags = atree.nodes[node].render_flags;
            let frames: Vec<Vec<Vec<model_mesh::BuiltMesh>>> = (0..atree.actions.len())
                .map(|a| {
                    let Some(entry) = atree.flipbook_entry(node, a) else { return Vec::new() };
                    let Some(&first) = sources.objects[r.source].get(entry.first.as_str()) else { return Vec::new() };
                    (0..usize::from(entry.frames.max(1)))
                        .filter(|k| first + k < file.objects.len())
                        .map(|k| {
                            let at = [(first + k, Vec3::ZERO, flags)];
                            model_mesh::build_flagged(file, &mut caches[r.source], at, meshes, level_materials, images, &mut bounds)
                        })
                        .collect()
                })
                .collect();
            flipbooks.push((node, Arc::new(frames)));
        }
    }
    // The modifiers each action runs (an action's `+0x2C` onward in its
    // bank's list).
    let texmods = r.atree.as_ref().and_then(|atree| {
        let list = sources.list[r.source].texmods;
        let actions: Vec<Vec<ActionTexMod>> = atree
            .actions
            .iter()
            .map(|a| {
                a.texmods()
                    .filter_map(|i| list.get(i))
                    .map(|m| {
                        let book = texanim::flipbook_images(m, file, &mut caches[r.source], None, images);
                        (m.clone(), book.map(|b| b.frames).unwrap_or_default())
                    })
                    .collect()
            })
            .collect();
        actions.iter().any(|l| !l.is_empty()).then(|| Arc::new(ActionTexMods { actions }))
    });
    BuiltModel { source: Some(r.source), atree: r.atree, parts, flipbooks, texmods }
}

/// Spawns a built model at `transform` for placement `index`: the root,
/// then an entity per atree node (at its rest offset under its parent's,
/// so the atree's actions can pose it) or the plain parts. Without an
/// index it's bound to no item and stays in its rest pose.
fn spawn_built(
    model: &BuiltModel,
    transform: Transform,
    index: Option<usize>,
    view: PopulationView,
    commands: &mut Commands,
) -> Entity {
    let root = commands.spawn((transform, PopulationPart::Model, visibility(view, PopulationPart::Model), LevelEntity)).id();
    if let Some(index) = index {
        commands.entity(root).insert(PlacementIndex(index));
    }
    // The parts drawing a texture its actions change, for `items.rs` to
    // give their own copies of the materials.
    let changed = model.texmods.as_ref().map_or_else(Vec::new, |t| t.bindings());
    let mut texmod_parts = Vec::new();
    let mut attach = |commands: &mut Commands, parent: Entity, parts: &[model_mesh::BuiltMesh], facing: Option<Billboard>| {
        for p in parts {
            let e = commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(parent))).id();
            if let Some(b) = facing {
                commands.entity(e).insert((b, Transform::default()));
            }
            if changed.contains(&p.diffuse) {
                texmod_parts.push((e, p.diffuse, p.material.clone()));
            }
        }
    };
    match &model.atree {
        Some(atree) => {
            let mut bones: Vec<Entity> = Vec::with_capacity(atree.nodes.len());
            for node in &atree.nodes {
                let parent = node.parent.map_or(root, |p| bones[p]);
                let at = Transform::from_translation(Vec3::from(node.offset));
                bones.push(commands.spawn((at, Visibility::default(), ChildOf(parent))).id());
            }
            // A flipbook node's frame hangs from a holder of its own, so
            // the frame can be swapped (`items.rs`) — also one showing
            // nothing until a later action (an exit's rising column).
            let flipbooks: Vec<(Entity, usize, FlipbookFrames, Option<Billboard>)> = model
                .flipbooks
                .iter()
                .map(|(node, frames)| {
                    let holder = commands.spawn((Transform::default(), Visibility::default(), ChildOf(bones[*node]))).id();
                    (holder, *node, frames.clone(), Billboard::from_flags(atree.nodes[*node].render_flags))
                })
                .collect();
            for (node, parts, facing) in &model.parts {
                match flipbooks.iter().find(|(_, n, _, _)| n == node) {
                    Some((holder, ..)) => attach(commands, *holder, parts, *facing),
                    None => attach(commands, bones[*node], parts, *facing),
                }
            }
            if index.is_some() {
                commands.entity(root).insert(ItemRig {
                    atree: atree.clone(),
                    bones,
                    flipbooks,
                    texmods: model.texmods.clone(),
                    texmod_parts,
                });
            }
        }
        None => {
            for (_, parts, facing) in &model.parts {
                attach(commands, root, parts, *facing);
            }
        }
    }
    root
}

/// The x-ray's models (`POWERUPS`): the see-through sprite drawn over the
/// container (camera-facing, drawn 800 depth units nearer), and what shows
/// inside for a monster and for more than one key.
pub const XRAY_SPRITE: &str = "SEETHRU";
pub const XRAY_MONSTER: &str = "DEATH_ICON";
pub const XRAY_KEYS: &str = "KEYRING";
const XRAY_SPRITE_BIAS: i16 = -800;
/// The key powerup's subtype.
const KEY: i32 = 2;
/// The gargoyle pieces' powerup subtype (`GARG<kind>`).
const GARGOYLE_PIECE: i32 = 16;
/// What a dying boss may throw (`loot.rs`).
const LOOT_ITEMS: [&str; 7] = ["COIN_BRONZE", "COIN_SILVER", "COIN_GOLD", "BGNTR_IC", "BMASK_IC", "BHORN_IC", "BGNTL_IC"];

/// The models a blast can bring (`breakables.rs`, `docs/mechanics.md`
/// "Blows on items"), and the items they come from: treasure's junk, the
/// spoiled meat and fruit, the wreck of a powerup blown up (food, timed
/// powerups — not keys, potions or quest pieces), of the silver chest and
/// of the other chests (not CHESTEXP, which explodes instead).
type Wanted = fn(&ItemType) -> bool;
const BLAST_MODELS: &[(&str, Wanted)] = &[
    ("TREAS_JUNK", |t| t.class == ItemClass::Powerup && t.subtype == 1),
    ("BADMEAT", |t| t.class == ItemClass::Powerup && t.subtype == 3),
    ("GAPPLE", |t| t.class == ItemClass::Powerup && t.subtype == 3),
    ("ITEMEXP0", |t| t.class == ItemClass::Powerup && !matches!(t.subtype, 1 | 2 | 4 | 10..=16)),
    ("CHESTSEXP0", |t| t.class == ItemClass::Container && t.subtype == 0x30),
    ("CHESTGEXP0", |t| t.class == ItemClass::Container && !matches!(t.subtype, 0x2C | 0x30)),
];

/// Whether an item model's atree node draws its part: all but hidden ones
/// and a chest's contents node.
fn part_drawn(n: &gdl_formats::anim::SkeletonNode) -> bool {
    !n.hidden() && n.name != CONTENTS_NODE
}

/// The node of a container's model that its contents hang on once it's
/// opened (`docs/items.md`, "Containers"): the game looks up the object
/// `<model>NULL1`, hides that node and keeps it for the contents. Only the
/// chests' models (`CHEST`, `CHESTS`) have one; its own part is a
/// placeholder card, never drawn.
pub const CONTENTS_NODE: &str = "NULL1";

/// The entity of a container model's contents node, if it has one.
pub fn contents_bone(rig: &ItemRig) -> Option<Entity> {
    rig.bones.get(rig.atree.node_index(CONTENTS_NODE)?).copied()
}

/// What a silver chest (`CHESTS`) holding gold becomes as it's opened: the
/// realm items' gold-filled silver chest.
pub const SILVER_GOLD_CHEST: &str = "CHESTSG";
/// The silver chest's container subtype.
const SILVER_CHEST: i32 = 0x30;
/// The gold powerup subtype.
const GOLD: i32 = 1;

/// Models of what the level's containers hold, by item type name, for
/// contents released at run time.
#[derive(Resource)]
pub struct ContentModels {
    models: HashMap<String, BuiltModel>,
    view: PopulationView,
}

impl ContentModels {
    /// Spawns the model of item type `name` for the released item numbered
    /// `index`; `None` if it has none.
    pub fn spawn(&self, name: &str, transform: Transform, index: usize, commands: &mut Commands) -> Option<Entity> {
        let model = self.models.get(name).filter(|m| !m.parts.is_empty())?;
        Some(spawn_built(model, transform, Some(index), self.view, commands))
    }

    /// Spawns item type `name`'s model in its rest pose, bound to no item
    /// (what the x-ray shows inside a container, `power_looks.rs`).
    pub fn spawn_still(&self, name: &str, transform: Transform, commands: &mut Commands) -> Option<Entity> {
        let model = self.models.get(name).filter(|m| !m.parts.is_empty())?;
        Some(spawn_built(model, transform, None, self.view, commands))
    }
}

/// Monster folder code for a generator's type, when it names a monster.
fn monster_code(ty: &ItemType) -> Option<&'static str> {
    let id = ty.enemy()?;
    ENEMY_CODES.iter().find(|e| e.0 == id).map(|e| e.2)
}

/// How a generator looks: `GEN_<code><strength>` with its monster's code
/// — the realm's own swapped in for a placeholder name, as the game builds
/// it (`docs/monsters.md`) — or, in the realms whose generators are all
/// special (E and F), `GEN_SPECIAL<strength>` from the realm's items, the
/// strength being the round's tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeneratorLook {
    Monster(&'static str, i32),
    Special(i32),
}

impl GeneratorLook {
    fn of(ty: &ItemType, placement: &gdl_formats::population::Placement, realm: u32, enemies: Option<&LevelEnemies>) -> Option<Self> {
        if ty.class != ItemClass::Generator {
            return None;
        }
        let PlacementParams::Generator { strength, .. } = placement.params(ty.class) else { return None };
        // Every generator of the special realms, whatever its type's name.
        if let Some((tier, _)) = crate::generators::special_generators(realm) {
            return Some(GeneratorLook::Special(tier));
        }
        let named = ty.enemy()?;
        let id = enemies.map_or(named, |e| e.substitute(named, 0));
        let code = gdl_formats::population::ENEMY_CODES.iter().find(|e| e.0 == id).map(|e| e.2)?;
        Some(GeneratorLook::Monster(code, i32::from(strength.max(1))))
    }

    /// Its model's name at strength `n`.
    fn name(self, n: i32) -> String {
        match self {
            GeneratorLook::Monster(code, _) => format!("GEN_{code}{n}"),
            GeneratorLook::Special(_) => format!("GEN_SPECIAL{n}"),
        }
    }

    fn strength(self) -> i32 {
        match self {
            GeneratorLook::Monster(_, n) | GeneratorLook::Special(n) => n,
        }
    }

    /// The monster folder its models are in (the realm's items: none).
    fn folder(self) -> Option<&'static str> {
        match self {
            GeneratorLook::Monster(code, _) => Some(code),
            GeneratorLook::Special(_) => None,
        }
    }
}

/// Model name the game gives a placement (generators draw
/// `GEN_<code><strength>`, [`GeneratorLook`]).
fn model_name(ty: &ItemType, placement: &gdl_formats::population::Placement, look: Option<GeneratorLook>) -> Option<String> {
    if placement.flags & 2 != 0 {
        return None; // placed without a model
    }
    if ty.class == ItemClass::Generator {
        let look = look?;
        return Some(look.name(look.strength()));
    }
    // A safe rock starts at its placement's count as its stage, and shows
    // it: `SAFEROCK3` (`docs/critters.md`, "Safe rocks").
    if let PlacementParams::Obstacle { subtype, count } = placement.params(ty.class) {
        let own = if subtype >= 1 { i32::from(subtype) } else { ty.subtype };
        if own == crate::items::SAFE_ROCK {
            return Some(format!("{}{count}", ty.name));
        }
    }
    Some(placement.model_name(ty).to_string())
}

#[derive(Default)]
pub struct Spawned {
    pub markers: usize,
    pub models: usize,
    pub summary: String,
    /// The banks' running texture animations on the models.
    pub texanims: Vec<TexAnim>,
}

/// How far above the floor a dropped item sits.
const ITEM_FLOOR_GAP: f32 = 0.1;

/// Spawns the level's population as markers and models, tagged as level
/// entities so a level change clears them.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    level: &LevelData,
    common: Option<&(ModelFile, Vec<u8>)>,
    install: &mut GameInstall,
    view: PopulationView,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    level_materials: &mut Assets<LevelMaterial>,
    marker_materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    enemies: Option<&LevelEnemies>,
) -> Spawned {
    let pop = &level.population;
    let realm = crate::quest::level_of(&level.name).map_or(0, |(r, _)| r);
    let look = |p: &gdl_formats::population::Placement| GeneratorLook::of(pop.resolved_type(p), p, realm, enemies);
    let realm_letter = level.name.strip_prefix("level").and_then(|s| s.chars().next()).unwrap_or('A');
    let item_dirs = [format!("ITEMS/level{realm_letter}"), "POWERUPS".to_string(), format!("ITEMS/{}", level.name)];
    let items: Vec<Source> = item_dirs.iter().filter_map(|d| Source::load(install, d)).collect();
    let mut codes: Vec<&'static str> = pop.placements.iter().filter_map(|p| look(p)?.folder()).collect();
    codes.sort();
    codes.dedup();
    let monsters: Vec<(&'static str, Source)> =
        codes.into_iter().filter_map(|c| Some((c, Source::load(install, &format!("MONSTERS/{c}"))?))).collect();
    let sources = Sources::new(level, &items, &monsters);
    let mut caches: Vec<TextureCache> = sources.list.iter().map(|s| TextureCache::new(s.model, s.textures)).collect();
    let mut built: HashMap<String, BuiltModel> = HashMap::new();

    let mut marker_assets: HashMap<Category, (Handle<Mesh>, Handle<StandardMaterial>)> = HashMap::new();
    let mut out = Spawned::default();
    let mut marker = |category: Category, transform: Transform, commands: &mut Commands, meshes: &mut Assets<Mesh>| {
        let (mesh, material) = marker_assets
            .entry(category)
            .or_insert_with(|| {
                let material = StandardMaterial { base_color: category.color(), unlit: true, ..default() };
                (meshes.add(category.mesh()), marker_materials.add(material))
            })
            .clone();
        let mut t = transform;
        t.translation.y += category.lift();
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            t,
            PopulationPart::Marker,
            visibility(view, PopulationPart::Marker),
            LevelEntity,
        ));
    };

    let mut counts: HashMap<Category, usize> = HashMap::new();
    for (index, placement) in pop.placements.iter().enumerate() {
        // Placements for more players (a second key, barrels for a bigger
        // party…) aren't made in a one-player game (`items.rs` doesn't
        // make their items either).
        if !placement.active_for(1) {
            continue;
        }
        let ty = pop.resolved_type(placement);
        let category = Category::of(ty);
        *counts.entry(category).or_default() += 1;
        // Items land on the floor under them (+0.1), like the game does at
        // level start, unless their type keeps its height.
        let mut position = placement.position;
        if !ty.keeps_height()
            && let Some(y) = level.collision.floor_height(position)
        {
            position[1] = y + ITEM_FLOOR_GAP;
        }
        let transform =
            Transform { translation: Vec3::from(position), rotation: game_rotation(placement.rotation), ..default() };
        marker(category, Transform::from_translation(transform.translation), commands, meshes);
        out.markers += 1;

        let generator = look(placement);
        let Some(name) = model_name(ty, placement, generator) else { continue };
        let monster = match generator {
            Some(g) => g.folder().and_then(|c| sources.monsters.get(c).copied()),
            None => monster_code(ty).and_then(|c| sources.monsters.get(c).copied()),
        };
        let key = format!("{name}/{}", monster.unwrap_or(usize::MAX));
        // Generators draw one model per strength level (`GEN_<code><n>`)
        // and step down as they're damaged: keep the lower levels' meshes.
        let tier_looks = generator.map(|g| {
            (1..=g.strength())
                .map(|t| {
                    let n = g.name(t);
                    let k = format!("{n}/{}", monster.unwrap_or(usize::MAX));
                    let m = built
                        .entry(k)
                        .or_insert_with(|| build_model(&sources, &mut caches, &n, monster, meshes, level_materials, images));
                    m.parts.iter().flat_map(|(_, p, _)| p.iter().map(|b| (b.mesh.clone(), b.material.clone()))).collect()
                })
                .collect::<Vec<_>>()
        });
        let model = built
            .entry(key)
            .or_insert_with(|| build_model(&sources, &mut caches, &name, monster, meshes, level_materials, images));
        if model.parts.is_empty() {
            debug!("placement {index}: no model {name:?} ({:?} {})", ty.class, ty.name);
            continue;
        }
        let root = spawn_built(model, transform, Some(index), view, commands);
        if model.atree.is_none()
            && let Some(looks) = tier_looks
        {
            commands.entity(root).insert(GeneratorLooks(looks));
        }
        out.models += 1;
    }

    // What containers hold, built now so a broken barrel's contents can
    // appear (`ContentModels`).
    let mut contents = ContentModels { models: HashMap::new(), view };
    for placement in &pop.placements {
        let ty = pop.resolved_type(placement);
        let PlacementParams::Container { contents: Some(c), .. } = placement.params(ty.class) else { continue };
        if c >= pop.item_types.len() {
            continue;
        }
        let inside = pop.resolve(c);
        if inside.name.is_empty() || contents.models.contains_key(&inside.name) {
            continue;
        }
        let model = build_model(&sources, &mut caches, &inside.name, None, meshes, level_materials, images);
        contents.models.insert(inside.name.clone(), model);
    }
    // A silver chest holding gold turns into the gold-filled one as it's
    // opened (`items.rs`).
    let silver_gold = pop.placements.iter().any(|p| {
        let ty = pop.resolved_type(p);
        let PlacementParams::Container { contents: Some(c), .. } = p.params(ty.class) else { return false };
        let inside = (c < pop.item_types.len()).then(|| pop.resolve(c));
        ty.class == ItemClass::Container
            && ty.subtype == SILVER_CHEST
            && inside.is_some_and(|t| t.class == ItemClass::Powerup && t.subtype == GOLD)
    });
    if silver_gold && !contents.models.contains_key(SILVER_GOLD_CHEST) {
        let model = build_model(&sources, &mut caches, SILVER_GOLD_CHEST, None, meshes, level_materials, images);
        contents.models.insert(SILVER_GOLD_CHEST.to_string(), model);
    }
    // A dead gargoyle leaves a gargoyle piece of its kind (`critters.rs`).
    let gargoyles = pop.placements.iter().any(|p| {
        let ty = pop.resolved_type(p);
        ty.class == ItemClass::EnemyInfo && ty.enemy() == Some(gdl_formats::enemy::GARGOYLE)
    });
    if gargoyles {
        for ty in pop.item_types.iter().filter(|t| t.class == ItemClass::Powerup && t.subtype == GARGOYLE_PIECE) {
            if !contents.models.contains_key(&ty.name) {
                let model = build_model(&sources, &mut caches, &ty.name, None, meshes, level_materials, images);
                contents.models.insert(ty.name.clone(), model);
            }
        }
    }
    // A safe rock shows each stage as it's broken down (`breakables.rs`).
    for placement in &pop.placements {
        let ty = pop.resolved_type(placement);
        let PlacementParams::Obstacle { subtype, .. } = placement.params(ty.class) else { continue };
        let own = if subtype >= 1 { i32::from(subtype) } else { ty.subtype };
        if own != crate::items::SAFE_ROCK {
            continue;
        }
        for stage in 0..=3 {
            let name = format!("{}{stage}", ty.name);
            if let std::collections::hash_map::Entry::Vacant(e) = contents.models.entry(name) {
                let model = build_model(&sources, &mut caches, e.key(), None, meshes, level_materials, images);
                e.insert(model);
            }
        }
    }
    // A boss's loot (`loot.rs`): its realm's coins or Skorne's pieces.
    if pop.locators.iter().any(|l| l.kind == LocatorKind::Boss) {
        for name in LOOT_ITEMS {
            if pop.item_types.iter().any(|t| t.name == name) && !contents.models.contains_key(name) {
                let model = build_model(&sources, &mut caches, name, None, meshes, level_materials, images);
                contents.models.insert(name.to_string(), model);
            }
        }
    }
    // A shut exit shows the items bank's `EXIT_OFF` (`items.rs`).
    if pop.placements.iter().any(|p| pop.resolved_type(p).class == ItemClass::Exit) {
        let model = build_model(&sources, &mut caches, crate::items::EXIT_OFF, None, meshes, level_materials, images);
        contents.models.insert(crate::items::EXIT_OFF.to_string(), model);
    }
    // What blasts turn things into or leave behind (`breakables.rs`):
    // treasure's junk and spoiled food (`POWERUPS`), and the wrecks of
    // powerups and chests (the realm's items bank) — each only where the
    // level has something it can come from.
    let mut types: Vec<ItemType> = pop.placements.iter().map(|p| pop.resolved_type(p).clone()).collect();
    for placement in &pop.placements {
        let ty = pop.resolved_type(placement);
        if let PlacementParams::Container { contents: Some(c), .. } = placement.params(ty.class)
            && c < pop.item_types.len()
        {
            types.push(pop.resolve(c).clone());
        }
    }
    let (mut blast_models, mut blast_triangles) = (Vec::new(), 0);
    for (name, wanted) in BLAST_MODELS {
        if contents.models.contains_key(*name) || !types.iter().any(wanted) {
            continue;
        }
        let model = build_model(&sources, &mut caches, name, None, meshes, level_materials, images);
        if model.parts.is_empty() {
            debug!("no {name} model for this level");
        }
        blast_triangles += model.meshes().map(|m| m.triangles).sum::<usize>();
        blast_models.push(*name);
        contents.models.insert(name.to_string(), model);
    }
    let ready: Vec<&str> =
        BLAST_MODELS.iter().map(|(n, _)| *n).filter(|n| contents.models.get(*n).is_some_and(|m| !m.parts.is_empty())).collect();
    info!("blast models {ready:?} ({blast_models:?} built for them: {blast_triangles} triangles)");
    // What the x-ray shows (`power_looks.rs`): the see-through sprite over
    // any container it can look into, the Death icon for a monster inside,
    // the key ring for more than one key — each only where a container
    // holds such a thing.
    let held: Vec<(ItemType, i16)> = pop
        .placements
        .iter()
        .filter_map(|p| match p.params(pop.resolved_type(p).class) {
            PlacementParams::Container { contents: Some(c), param } if c < pop.item_types.len() => {
                Some((pop.resolve(c).clone(), param))
            }
            _ => None,
        })
        .collect();
    let seen = |(t, _): &(ItemType, i16)| matches!(t.class, ItemClass::Powerup | ItemClass::EnemyInfo);
    let xray_wanted = [
        (XRAY_SPRITE, held.iter().any(seen)),
        (XRAY_MONSTER, held.iter().any(|(t, _)| t.class == ItemClass::EnemyInfo)),
        (XRAY_KEYS, held.iter().any(|(t, keys)| t.class == ItemClass::Powerup && t.subtype == KEY && *keys > 1)),
    ];
    let near = PerspectiveProjection::default().near;
    for (name, wanted) in xray_wanted {
        if !wanted || contents.models.contains_key(name) {
            continue;
        }
        let model = build_model(&sources, &mut caches, name, None, meshes, level_materials, images);
        if name == XRAY_SPRITE {
            for b in model.meshes() {
                if let Some(m) = level_materials.get_mut(&b.material) {
                    m.set_depth_bias(XRAY_SPRITE_BIAS, near);
                }
            }
        }
        debug!("x-ray model {name}: {} parts", model.parts.len());
        contents.models.insert(name.to_string(), model);
    }
    // Each bank's texture modifiers on the models drawn from it: the
    // item banks', the level's own and the generators' monster banks'.
    let mut drawn: Vec<HashMap<u16, Vec<Handle<LevelMaterial>>>> = vec![HashMap::new(); sources.list.len()];
    for model in built.values().chain(contents.models.values()) {
        let Some(s) = model.source else { continue };
        for b in model.meshes() {
            drawn[s].entry(b.diffuse).or_default().push(b.material.clone());
        }
    }
    let mut shared_cache = common.map(|(model, textures)| TextureCache::new(model, textures));
    for (s, source) in sources.list.iter().enumerate() {
        if drawn[s].is_empty() {
            continue;
        }
        let cache = &mut caches[s];
        let frames = |m: &TexMod| {
            let shared = common.map(|(model, _)| model).zip(shared_cache.as_mut());
            texanim::flipbook_images(m, source.model, cache, shared, images)
        };
        out.texanims.extend(texanim::bank_anims(source.texmods, &drawn[s], frames, level_materials));
    }
    commands.insert_resource(contents);

    for locator in &pop.locators {
        let category = match locator.kind {
            LocatorKind::PlayerStart => Category::PlayerStart,
            LocatorKind::Boss => Category::Boss,
            _ => continue, // camera points
        };
        *counts.entry(category).or_default() += 1;
        let mut t = Transform::from_translation(Vec3::from(locator.position));
        if category == Category::PlayerStart {
            // Cone tip along the start's facing (+Z turned by its yaw).
            let forward = game_rotation([0.0, locator.rotation[1], 0.0]) * Vec3::Z;
            t.rotation = Quat::from_rotation_arc(Vec3::Y, forward);
        }
        marker(category, t, commands, meshes);
        out.markers += 1;
    }

    let count = |c: Category| counts.get(&c).copied().unwrap_or(0);
    out.summary = format!(
        "{} starts, {} generators, {} monsters, {} keys, {} gold, {} food, {} potions, {} other powerups, \
         {} containers, {} doors, {} exits, {} transporters, {} triggers, {} hazards; {} models",
        count(Category::PlayerStart),
        count(Category::Generator),
        count(Category::Monster),
        count(Category::Key),
        count(Category::Gold),
        count(Category::Food),
        count(Category::Potion),
        count(Category::Powerup),
        count(Category::Container),
        count(Category::Door),
        count(Category::Exit),
        count(Category::Transporter),
        count(Category::Trigger),
        count(Category::Hazard),
        out.models,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_glows_draw_but_hidden_and_contents_nodes_dont() {
        let node = |name: &str, render_flags: u32| gdl_formats::anim::SkeletonNode {
            name: name.into(),
            offset: [0.0; 3],
            parent: Some(0),
            kind: gdl_formats::anim::NodeKind::Skeletal,
            node_flags: 0,
            render_flags,
            index: -1,
        };
        // The red gem's glow ring and the green potion's glow, like the
        // other gems' (named `CFXP_GLO`…).
        for name in ["CFXP_GLOW", "XP_GLOW", "CFXP_GLO", "XP_CRYSTA"] {
            assert!(part_drawn(&node(name, 0x04c0_1880)), "{name}");
        }
        assert!(!part_drawn(&node("XP_CRYSTA", gdl_formats::anim::RENDER_HIDDEN)));
        assert!(!part_drawn(&node(CONTENTS_NODE, 0)));
    }

    #[test]
    fn locator_angles_come_back() {
        for a in [[0.3, -1.1, 2.0], [0.0, 0.0, 0.0], [-1.2, 2.9, -0.4], [1.5, 0.2, 0.1]] {
            let m = locator_matrix(a);
            let back = locator_matrix(locator_euler(m));
            assert!(m.iter().zip(back).all(|(x, y)| (x - y).abs() < 1e-4), "{a:?}: {m:?} vs {back:?}");
        }
        // A placement's matrix goes through it unchanged too.
        let p = gdl_formats::population::rotation_matrix([0.4, 1.3, -0.7]);
        let back = locator_matrix(locator_euler(p));
        assert!(p.iter().zip(back).all(|(x, y)| (x - y).abs() < 1e-4), "{p:?} vs {back:?}");
    }

    #[test]
    fn locators_turn_the_other_way_from_placements() {
        // A yaw of 90°: the locator builder sends +Z to +X, the placement
        // builder to −X.
        let y = std::f32::consts::FRAC_PI_2;
        let l = locator_matrix([0.0, y, 0.0]);
        assert!((l[6] - 1.0).abs() < 1e-5 && l[8].abs() < 1e-5);
        let p = gdl_formats::population::rotation_matrix([0.0, y, 0.0]);
        assert!((p[6] + 1.0).abs() < 1e-5 && p[8].abs() < 1e-5);
        // Proper rotations (no mirror).
        let det = Mat3::from_cols_array(&locator_matrix([0.3, 1.2, -0.7])).determinant();
        assert!((det - 1.0).abs() < 1e-4);
    }
}
