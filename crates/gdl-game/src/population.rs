//! Shows what populates a level — player starts, generators, monsters,
//! pickups, doors, triggers, exits (`docs/level-population.md`) — as
//! coloured markers and, where the item's model can be found, the model.
//!
//! `I` cycles models + markers / models / markers / hidden; `GDL_POPULATION`
//! (`all`, `models`, `markers`, `off`; default `models`) picks the starting view.

use std::collections::HashMap;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::{AnimFile, Atree};
use gdl_formats::population::{
    ENEMY_CODES, ItemClass, ItemType, LocatorKind, PlacementParams, PlayerStart, Population, rotation_matrix,
};
use gdl_install::GameInstall;

use crate::level::LevelData;
use crate::level_material::LevelMaterial;
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

/// The drawn model of placement `.0` in the level's population (so what
/// happens to the item — a generator broken, a pickup taken — can find it).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlacementModel(pub usize);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum PopulationPart {
    Marker,
    Model,
}

/// The current level's population, and where its players start — for
/// whatever spawns the playable characters.
#[derive(Resource)]
pub struct LevelPopulation {
    pub level: String,
    pub population: Population,
}

impl LevelPopulation {
    /// The start for entry 0 (every level has one).
    pub fn player_start(&self) -> Option<PlayerStart> {
        self.population.player_start(0)
    }
}

/// Where a start puts the players: at its position, turned by its yaw the
/// way the game turns placed things (`rotation_matrix`). Which model axis
/// counts as "forward" for a player isn't confirmed yet; the game derives
/// items' facing from where their +Z axis ends up.
pub fn start_transform(start: &PlayerStart) -> Transform {
    Transform { translation: Vec3::from(start.position), rotation: game_rotation([0.0, start.yaw, 0.0]), ..default() }
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
    atrees: Vec<Atree>,
}

impl Source {
    fn load(install: &mut GameInstall, dir: &str) -> Option<Self> {
        let model = ModelFile::parse(&install.read(&format!("{dir}/objects.ngc")).ok()?).ok()?;
        let textures = install.read(&format!("{dir}/textures.ngc")).ok()?;
        let atrees = install
            .read(&format!("{dir}/ANIM.PS2"))
            .ok()
            .and_then(|a| AnimFile::parse(&a).ok())
            .map_or_else(Vec::new, |a| a.atrees);
        Some(Self { model, textures, atrees })
    }
}

/// Where item models come from, in the game's search order: the realm's
/// items, the shared powerups, the level's own items
/// (`ITEMS/level<realm letter>`, `POWERUPS`, `ITEMS/<level>`); generators
/// also look in their monster's folder. Plain object lookups search the
/// level's own models too.
struct Sources<'a> {
    list: Vec<(&'a ModelFile, &'a [u8], &'a [Atree])>,
    objects: Vec<HashMap<&'a str, usize>>,
    /// Indices into `list` of the item folders.
    items: Vec<usize>,
    level: usize,
    monsters: HashMap<&'static str, usize>,
}

impl<'a> Sources<'a> {
    fn new(level: &'a LevelData, items: &'a [Source], monsters: &'a [(&'static str, Source)]) -> Self {
        let mut list: Vec<(&ModelFile, &[u8], &[Atree])> =
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
    /// (each node's object is `<atree><node>`, at its rest offset), else an
    /// object named `name`, `name` + `L1` or `name` + `ROOT`.
    fn resolve(&self, name: &str, extra: Option<usize>) -> Option<(usize, Vec<(usize, Vec3)>)> {
        if name.is_empty() {
            return None;
        }
        let atree_order: Vec<usize> = self.items.iter().copied().chain(extra).collect();
        for &s in &atree_order {
            let Some(atree) = self.list[s].2.iter().find(|a| a.name == name) else { continue };
            let mut offsets: Vec<Vec3> = Vec::with_capacity(atree.nodes.len());
            let mut parts = Vec::new();
            for node in &atree.nodes {
                let at = node.parent.map_or(Vec3::ZERO, |p| offsets[p]) + Vec3::from(node.offset);
                offsets.push(at);
                if let Some(&o) = self.objects[s].get(format!("{}{}", atree.name, node.name).as_str()) {
                    parts.push((o, at));
                }
            }
            if !parts.is_empty() {
                return Some((s, parts));
            }
        }
        let object_order: Vec<usize> = atree_order.iter().copied().chain([self.level]).collect();
        for suffix in ["", "L1", "ROOT"] {
            let full = format!("{name}{suffix}");
            for &s in &object_order {
                if let Some(&o) = self.objects[s].get(full.as_str()) {
                    return Some((s, vec![(o, Vec3::ZERO)]));
                }
            }
        }
        None
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
    let mut built: HashMap<String, Vec<model_mesh::BuiltMesh>> = HashMap::new();

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
        let parts = match built.get(&key) {
            Some(parts) => parts,
            None => {
                let mesh_parts = match sources.resolve(&name, monster) {
                    Some((s, parts)) => {
                        let mut bounds = (Vec3::MAX, Vec3::MIN);
                        let (model, _, _) = sources.list[s];
                        model_mesh::build(model, &mut caches[s], parts, meshes, level_materials, images, &mut bounds)
                    }
                    None => Vec::new(),
                };
                built.entry(key).or_insert(mesh_parts)
            }
        };
        if parts.is_empty() {
            continue;
        }
        let root = commands
            .spawn((
                transform,
                PopulationPart::Model,
                PlacementModel(index),
                visibility(view, PopulationPart::Model),
                LevelEntity,
            ))
            .id();
        for p in parts {
            commands.spawn((Mesh3d(p.mesh.clone()), MeshMaterial3d(p.material.clone()), ChildOf(root)));
        }
        out.models += 1;
    }

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
