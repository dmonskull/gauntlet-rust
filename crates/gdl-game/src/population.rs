//! Shows what populates a level — player starts, generators, monsters,
//! pickups, doors, triggers, exits (`docs/level-population.md`) — as
//! coloured markers and, where the item's model can be found, the model.
//!
//! `I` cycles models + markers / models / markers / hidden; `GDL_POPULATION`
//! (`all`, `models`, `markers`, `off`; default `models`) picks the starting view.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::{AnimFile, Atree};
use gdl_formats::population::{
    ENEMY_CODES, ItemClass, ItemType, LocatorKind, PlacementParams, PlayerStart, Population, rotation_matrix,
};
use gdl_install::GameInstall;

use crate::level::LevelData;
use crate::level_material::LevelMaterial;
use crate::billboard::Billboard;
use crate::model_mesh::{self, TextureCache};
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
    /// Flipbook nodes: the entity holding the frame shown, the frames and
    /// how they face the camera.
    pub flipbooks: Vec<(Entity, FlipbookFrames, Option<Billboard>)>,
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

/// Where the heroes arrive in the tower: its start `i` is for coming back
/// from realm `TOWER_ARRIVALS[i]` — the tower itself (a new game) at the
/// centre, then G, B, A, K, D, C, I, J, E, F, H beside their gates. The
/// realm is the last one played outside the tower.
const TOWER_ARRIVALS: [u32; 12] = [13, 7, 2, 1, 11, 4, 3, 9, 10, 5, 6, 8];
const TOWER_REALM: u32 = 13;

/// The start entry for arriving in `level` after last playing in realm
/// `last_realm`: in the tower, the one for that realm; elsewhere 0.
pub fn start_entry(level: &str, last_realm: Option<u32>) -> i16 {
    if crate::quest::level_of(level).is_none_or(|(realm, _)| realm != TOWER_REALM) {
        return 0;
    }
    let realm = last_realm.unwrap_or(TOWER_REALM);
    TOWER_ARRIVALS.iter().position(|&r| r == realm).map_or(0, |i| i as i16)
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
    let r = [-l.rotation[0], l.rotation[1] + std::f32::consts::PI, l.rotation[2]];
    let (cx, sx) = (r[0].cos(), -r[0].sin());
    let (cy, sy) = (r[1].cos(), -r[1].sin());
    let (cz, sz) = (r[2].cos(), -r[2].sin());
    // Row-major for row vectors; read as columns, the column-vector matrix.
    let m = [
        cy * cz - (sy * sx) * sz,
        cx * sz,
        sy * cz + (cy * sx) * sz,
        -cy * sz - (sy * sx) * cz,
        cx * cz,
        -sy * sz + (cy * sx) * cz,
        -sy * cx,
        -sx,
        cy * cx,
    ];
    Transform { translation: Vec3::from(l.position), rotation: Quat::from_mat3(&Mat3::from_cols_array(&m)), ..default() }
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
    if keys.just_pressed(KeyCode::KeyI) {
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

/// A folder's models and atrees (`objects.ngc`, `textures.ngc`, `ANIM.PS2`).
struct Source {
    model: ModelFile,
    textures: Vec<u8>,
    atrees: Vec<Arc<Atree>>,
}

impl Source {
    fn load(install: &mut GameInstall, dir: &str) -> Option<Self> {
        let model = ModelFile::parse(&install.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
        let textures = install.read(&format!("{dir}/textures.ngc")).ok()?;
        let atrees = install
            .read(&format!("{dir}/ANIM.PS2"))
            .ok()
            .and_then(|a| AnimFile::parse(&a).ok())
            .map_or_else(Vec::new, |a| a.atrees.into_iter().map(Arc::new).collect());
        Some(Self { model, textures, atrees })
    }
}

/// Where item models come from, in the game's search order: the realm's
/// items, the shared powerups, the level's own items
/// (`ITEMS/level<realm letter>`, `POWERUPS`, `ITEMS/<level>`); generators
/// also look in their monster's folder. Plain object lookups search the
/// level's own models too.
/// One model source: its models, textures and atrees.
type SourceRef<'a> = (&'a ModelFile, &'a [u8], &'a [Arc<Atree>]);

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
        let mut list: Vec<SourceRef> =
            items.iter().map(|s| (&s.model, s.textures.as_slice(), s.atrees.as_slice())).collect();
        let item_indices = (0..list.len()).collect();
        list.push((&level.model, &level.textures, &[]));
        let level_index = list.len() - 1;
        let mut monster_indices = HashMap::new();
        for (code, s) in monsters {
            monster_indices.insert(*code, list.len());
            list.push((&s.model, s.textures.as_slice(), s.atrees.as_slice()));
        }
        let objects = list
            .iter()
            .map(|(m, _, _)| m.objects.iter().enumerate().map(|(i, o)| (o.name.as_str(), i)).collect())
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
            let Some(atree) = self.list[s].2.iter().find(|a| a.name == name) else { continue };
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
        for suffix in ["", "L1", "ROOT"] {
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
    atree: Option<Arc<Atree>>,
    parts: Vec<(usize, Vec<model_mesh::BuiltMesh>, Option<Billboard>)>,
    /// Flipbook nodes and their frames by action (action 0's first frame
    /// is the node's part).
    flipbooks: Vec<(usize, FlipbookFrames)>,
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
        return BuiltModel { atree: None, parts: Vec::new(), flipbooks: Vec::new() };
    };
    let mut bounds = (Vec3::MAX, Vec3::MIN);
    let (file, _, _) = sources.list[r.source];
    let parts = r
        .parts
        .iter()
        .filter_map(|&(node, object)| {
            // Each atree node draws with its render flags (blending, facing),
            // like a character's; hidden nodes and glows (whose texture the
            // effects system sets at run time) don't draw.
            let flags = match &r.atree {
                Some(atree) => {
                    let n = &atree.nodes[node];
                    if n.hidden() || n.name.ends_with("GLOW") {
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
    BuiltModel { atree: r.atree, parts, flipbooks }
}

/// Spawns a built model at `transform` for placement `index`: the root,
/// then an entity per atree node (at its rest offset under its parent's,
/// so the atree's actions can pose it) or the plain parts.
fn spawn_built(model: &BuiltModel, transform: Transform, index: usize, view: PopulationView, commands: &mut Commands) -> Entity {
    let root = commands
        .spawn((transform, PopulationPart::Model, PlacementIndex(index), visibility(view, PopulationPart::Model), LevelEntity))
        .id();
    let attach = |commands: &mut Commands, parent: Entity, parts: &[model_mesh::BuiltMesh], facing: Option<Billboard>| {
        for p in parts {
            let e = commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(parent))).id();
            if let Some(b) = facing {
                commands.entity(e).insert((b, Transform::default()));
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
            // the frame can be swapped (`items.rs`).
            let holders: Vec<(usize, Entity, FlipbookFrames)> = model
                .flipbooks
                .iter()
                .map(|(node, frames)| {
                    let holder = commands.spawn((Transform::default(), Visibility::default(), ChildOf(bones[*node]))).id();
                    (*node, holder, frames.clone())
                })
                .collect();
            let mut flipbooks = Vec::new();
            for (node, parts, facing) in &model.parts {
                match holders.iter().find(|(n, _, _)| n == node) {
                    Some((_, holder, frames)) => {
                        attach(commands, *holder, parts, *facing);
                        flipbooks.push((*holder, frames.clone(), *facing));
                    }
                    None => attach(commands, bones[*node], parts, *facing),
                }
            }
            commands.entity(root).insert(ItemRig { atree: atree.clone(), bones, flipbooks });
        }
        None => {
            for (_, parts, facing) in &model.parts {
                attach(commands, root, parts, *facing);
            }
        }
    }
    root
}

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
        Some(spawn_built(model, transform, index, self.view, commands))
    }
}

/// Monster folder code for a generator's type, when it names a monster.
fn monster_code(ty: &ItemType) -> Option<&'static str> {
    let id = ty.enemy()?;
    ENEMY_CODES.iter().find(|e| e.0 == id).map(|e| e.2)
}

/// Model name the game gives a placement (generators draw
/// `GEN_<code><strength>`).
fn model_name(ty: &ItemType, placement: &gdl_formats::population::Placement) -> Option<String> {
    if placement.flags & 2 != 0 {
        return None; // placed without a model
    }
    if ty.class == ItemClass::Generator {
        let PlacementParams::Generator { strength, .. } = placement.params(ty.class) else { return None };
        return Some(format!("GEN_{}{}", monster_code(ty)?, strength.max(1)));
    }
    Some(placement.model_name(ty).to_string())
}

#[derive(Default)]
pub struct Spawned {
    pub markers: usize,
    pub models: usize,
    pub summary: String,
}

/// How far above the floor a dropped item sits.
const ITEM_FLOOR_GAP: f32 = 0.1;

/// Spawns the level's population as markers and models, tagged as level
/// entities so a level change clears them.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    level: &LevelData,
    install: &mut GameInstall,
    view: PopulationView,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    level_materials: &mut Assets<LevelMaterial>,
    marker_materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Spawned {
    let pop = &level.population;
    let realm_letter = level.name.strip_prefix("level").and_then(|s| s.chars().next()).unwrap_or('A');
    let item_dirs = [format!("ITEMS/level{realm_letter}"), "POWERUPS".to_string(), format!("ITEMS/{}", level.name)];
    let items: Vec<Source> = item_dirs.iter().filter_map(|d| Source::load(install, d)).collect();
    let mut codes: Vec<&'static str> = pop
        .placements
        .iter()
        .map(|p| pop.resolved_type(p))
        .filter(|t| t.class == ItemClass::Generator)
        .filter_map(monster_code)
        .collect();
    codes.sort();
    codes.dedup();
    let monsters: Vec<(&'static str, Source)> =
        codes.into_iter().filter_map(|c| Some((c, Source::load(install, &format!("MONSTERS/{c}"))?))).collect();
    let sources = Sources::new(level, &items, &monsters);
    let mut caches: Vec<TextureCache> = sources.list.iter().map(|(m, t, _)| TextureCache::new(m, t)).collect();
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

        let Some(name) = model_name(ty, placement) else { continue };
        let monster = monster_code(ty).and_then(|c| sources.monsters.get(c).copied());
        let key = format!("{name}/{}", monster.unwrap_or(usize::MAX));
        // Generators draw one model per strength level (`GEN_<code><n>`)
        // and step down as they're damaged: keep the lower levels' meshes.
        let tier_looks = match placement.params(ty.class) {
            PlacementParams::Generator { strength, .. } if ty.class == ItemClass::Generator => {
                monster_code(ty).map(|code| {
                    (1..=strength.max(1))
                        .map(|t| {
                            let n = format!("GEN_{code}{t}");
                            let k = format!("{n}/{}", monster.unwrap_or(usize::MAX));
                            let m = built.entry(k).or_insert_with(|| {
                                build_model(&sources, &mut caches, &n, monster, meshes, level_materials, images)
                            });
                            m.parts.iter().flat_map(|(_, p, _)| p.iter().map(|b| (b.mesh.clone(), b.material.clone()))).collect()
                        })
                        .collect::<Vec<_>>()
                })
            }
            _ => None,
        };
        let model = built
            .entry(key)
            .or_insert_with(|| build_model(&sources, &mut caches, &name, monster, meshes, level_materials, images));
        if model.parts.is_empty() {
            debug!("placement {index}: no model {name:?} ({:?} {})", ty.class, ty.name);
            continue;
        }
        let root = spawn_built(model, transform, index, view, commands);
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
    // A shut exit shows the items bank's `EXIT_OFF` (`items.rs`).
    if pop.placements.iter().any(|p| pop.resolved_type(p).class == ItemClass::Exit) {
        let model = build_model(&sources, &mut caches, crate::items::EXIT_OFF, None, meshes, level_materials, images);
        contents.models.insert(crate::items::EXIT_OFF.to_string(), model);
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
