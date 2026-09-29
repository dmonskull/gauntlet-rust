//! Characters: segmented models posed by a skeleton and animated by the
//! game's keyframe clips (`docs/animation-format.md`).

use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::{AnimFile, Atree, NodeKind, Track, rotation_matrix};
use gdl_install::GameInstall;

use crate::level_material::LevelMaterial;
use crate::model_mesh::{self, TextureCache};

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate);
    }
}

/// The game's player class order, and the bone each class holds its weapon
/// in (tables in `main.dol`; `docs/animation-format.md`).
const CLASS_HAND_BONES: [(&str, &str); 17] = [
    ("WAR", "R_WRIST"),
    ("VAL", "R_WRIST"),
    ("WIZ", "R_WRIST"),
    ("ARC", "R_WRIST"),
    ("DWF", "RIGHTHAN"),
    ("KNI", "RIGHTHAN"),
    ("SOR", "RIGHTHAN"),
    ("JES", "RHEND"),
    ("MIN", "R_WRIST"),
    ("FAL", "R_WRIST"),
    ("JAC", "R_WRIST"),
    ("TIG", "R_WRIST"),
    ("OGR", "RIGHTHAN"),
    ("UNI", "RIGHTHAN"),
    ("MED", "RIGHTHAN"),
    ("HYE", "RHEND"),
    ("SUM", "R_WRIST"),
];

/// Everything needed to spawn one player class in one colour/armour.
pub struct CharacterData {
    /// e.g. `ARC/BLU`.
    pub name: String,
    pub class: String,
    /// Colour folder prefix: `BLU`, `RED`, `YEL` or `GRE`.
    pub colour: String,
    /// The variant's own skeleton; its name prefixes the part models.
    pub skeleton: Atree,
    /// The class's shared actions and keyframes.
    pub clips: Arc<Atree>,
    pub model: ModelFile,
    pub textures: Vec<u8>,
}

/// Player classes on the disc (folders under `PLAYERS/`).
pub fn player_classes(install: &GameInstall) -> Vec<String> {
    let mut classes: Vec<String> = install
        .files()
        .iter()
        .filter_map(|f| {
            let mut p = f.split('/');
            let (Some(top), Some(class), Some(anim), Some(file)) = (p.next(), p.next(), p.next(), p.next()) else {
                return None;
            };
            (top.eq_ignore_ascii_case("PLAYERS")
                && anim.eq_ignore_ascii_case("ANIM")
                && file.eq_ignore_ascii_case("ANIM.PS2"))
            .then(|| class.to_string())
        })
        .collect();
    classes.sort();
    classes.dedup();
    classes
}

pub fn load_player(install: &mut GameInstall, class: &str, variant: &str) -> Result<CharacterData, String> {
    let read = |install: &mut GameInstall, path: String| install.read(&path).map_err(|e| format!("{path}: {e}"));
    let dir = format!("PLAYERS/{class}/{variant}");
    let model = ModelFile::parse(&read(install, format!("{dir}/objects.ngc"))?).map_err(|e| format!("{dir}/objects.ngc: {e}"))?;
    let textures = read(install, format!("{dir}/textures.ngc"))?;
    let skeleton = first_atree(&read(install, format!("{dir}/ANIM.PS2"))?).map_err(|e| format!("{dir}/ANIM.PS2: {e}"))?;
    let clips_path = format!("PLAYERS/{class}/ANIM/ANIM.PS2");
    let clips = first_atree(&read(install, clips_path.clone())?).map_err(|e| format!("{clips_path}: {e}"))?;
    if clips.clips.is_none() {
        return Err(format!("{clips_path} has no animation clips"));
    }
    Ok(CharacterData {
        name: format!("{class}/{variant}"),
        class: class.to_ascii_uppercase(),
        colour: variant.chars().take(3).collect::<String>().to_ascii_uppercase(),
        skeleton,
        clips: Arc::new(clips),
        model,
        textures,
    })
}

fn first_atree(data: &[u8]) -> Result<Atree, String> {
    AnimFile::parse(data)
        .map_err(|e| e.to_string())?
        .atrees
        .into_iter()
        .next()
        .ok_or_else(|| "no skeleton".to_string())
}

/// Monster folders on the disc (under `MONSTERS/`, excluding `*AUX` shared
/// data folders).
pub fn monster_names(install: &GameInstall) -> Vec<String> {
    let mut names: Vec<String> = install
        .files()
        .iter()
        .filter_map(|f| {
            let mut p = f.split('/');
            let (Some(top), Some(name), Some(file), None) = (p.next(), p.next(), p.next(), p.next()) else {
                return None;
            };
            (top.eq_ignore_ascii_case("MONSTERS") && file.eq_ignore_ascii_case("ANIM.PS2") && !name.ends_with("AUX"))
                .then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// A monster: its folder's model and the atree that animates its body — the
/// one named after the folder (bosses), else `<folder>1` (the first tier of
/// a regular monster), else the one with the most actions.
pub fn load_monster(install: &mut GameInstall, name: &str) -> Result<CharacterData, String> {
    let read = |install: &mut GameInstall, path: String| install.read(&path).map_err(|e| format!("{path}: {e}"));
    let dir = format!("MONSTERS/{name}");
    let model = ModelFile::parse(&read(install, format!("{dir}/objects.ngc"))?).map_err(|e| format!("{dir}/objects.ngc: {e}"))?;
    let textures = read(install, format!("{dir}/textures.ngc"))?;
    let anim = AnimFile::parse(&read(install, format!("{dir}/ANIM.PS2"))?).map_err(|e| format!("{dir}/ANIM.PS2: {e}"))?;
    let upper = name.to_ascii_uppercase();
    let pick = anim
        .atrees
        .iter()
        .position(|t| t.name == upper)
        .or_else(|| anim.atrees.iter().position(|t| t.name == format!("{upper}1")))
        .or_else(|| (0..anim.atrees.len()).max_by_key(|&i| anim.atrees[i].actions.len()))
        .ok_or_else(|| format!("{dir}/ANIM.PS2 has no atrees"))?;
    let tree = anim.atrees.into_iter().nth(pick).unwrap();
    Ok(CharacterData {
        name: format!("{name} ({})", tree.name),
        class: String::new(),
        colour: String::new(),
        skeleton: tree.clone(),
        clips: Arc::new(tree),
        model,
        textures,
    })
}

/// The meshes one model object is drawn with.
type PartMeshes = Vec<(Handle<Mesh>, Handle<LevelMaterial>)>;

/// A flipbook node's meshes: per action, the model objects for each frame.
struct Flipbook {
    /// The node's child entities that show the current frame's meshes.
    slots: Vec<Entity>,
    /// `frames[action][frame]` = the meshes of that frame's object.
    frames: Vec<Vec<PartMeshes>>,
    shown: Option<(usize, usize)>,
}

/// Drives a spawned character's bones.
#[derive(Component)]
pub struct Animator {
    pub clips: Arc<Atree>,
    /// One entity per skeleton node.
    bones: Vec<Entity>,
    rest: Vec<Vec3>,
    /// Skeleton node → clip bone.
    clip_bone: Vec<Option<usize>>,
    flipbooks: Vec<(usize, Flipbook)>,
    pub action: usize,
    pub frame: f32,
    tracks: Vec<Option<Track>>,
}

impl Animator {
    pub fn play(&mut self, action: usize) {
        self.action = action.min(self.clips.actions.len().saturating_sub(1));
        self.frame = 0.0;
        self.tracks = self
            .clip_bone
            .iter()
            .map(|b| b.and_then(|b| self.clips.track(b, self.action).ok().flatten()))
            .collect();
    }

    pub fn play_named(&mut self, name: &str) -> bool {
        match self.clips.actions.iter().position(|a| a.name == name) {
            Some(i) => {
                if i != self.action || self.tracks.is_empty() {
                    self.play(i);
                }
                true
            }
            None => false,
        }
    }
}

/// Spawns `data` posed at `transform`. Returns the root entity and rough
/// rest-pose bounds in the root's space (skeleton joints plus part meshes),
/// for framing a camera.
pub fn spawn_character(
    data: &CharacterData,
    transform: Transform,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> (Entity, Vec3, Vec3) {
    let root = commands.spawn((transform, Visibility::default())).id();
    let mut cache = TextureCache::new(&data.model, &data.textures);
    let mut bones: Vec<Entity> = Vec::with_capacity(data.skeleton.nodes.len());
    let mut bounds = (Vec3::MAX, Vec3::MIN);
    let object_index = |name: &str| data.model.objects.iter().position(|o| o.name == name);

    let mut build = |object: usize| {
        model_mesh::build(&data.model, &mut cache, [(object, Vec3::ZERO)], meshes, materials, images, &mut bounds)
    };
    let attach = |object: Option<usize>, parent: Entity, commands: &mut Commands, build: &mut dyn FnMut(usize) -> Vec<model_mesh::BuiltMesh>| -> bool {
        let Some(object) = object else { return false };
        for b in build(object) {
            commands.spawn((Mesh3d(b.mesh), MeshMaterial3d(b.material), ChildOf(parent)));
        }
        true
    };

    let mut flipbooks = Vec::new();
    for (i, node) in data.skeleton.nodes.iter().enumerate() {
        let parent = node.parent.map_or(root, |p| bones[p]);
        let bone = commands
            .spawn((Transform::from_translation(Vec3::from(node.offset)), Visibility::default(), ChildOf(parent)))
            .id();
        // The game hides DUMMY nodes and nodes with the hidden render flag;
        // glow objects (CFGLOW, CFXPGLOW) are drawn by effects, not yet here.
        let visible = node.has_model() && !node.hidden() && node.name != "DUMMY" && !node.name.ends_with("GLOW");
        if visible {
            attach(object_index(&format!("{}{}", data.skeleton.name, node.name)), bone, commands, &mut build);
        }
        if node.kind == NodeKind::Flipbook {
            let frames: Vec<Vec<PartMeshes>> = (0..data.clips.actions.len())
                .map(|a| {
                    let Some(entry) = data.skeleton.flipbook_entry(i, a) else { return Vec::new() };
                    let Some(first) = object_index(&entry.first) else { return Vec::new() };
                    (0..entry.frames.max(1) as usize)
                        .filter(|k| first + k < data.model.objects.len())
                        .map(|k| build(first + k).into_iter().map(|b| (b.mesh, b.material)).collect())
                        .collect()
                })
                .collect();
            // Slots start with any frame's meshes so they carry the mesh and
            // material components the animator swaps each frame.
            let slots_needed = frames.iter().flatten().map(Vec::len).max().unwrap_or(0);
            let sample: Vec<_> = frames.iter().flatten().flatten().cloned().collect();
            let slots = (0..slots_needed)
                .map(|s| {
                    let (mesh, material) = sample[s.min(sample.len() - 1)].clone();
                    commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), Visibility::Hidden, ChildOf(bone))).id()
                })
                .collect();
            flipbooks.push((i, Flipbook { slots, frames, shown: None }));
        }
        bones.push(bone);
    }

    // The weapon goes in the class's hand bone: WEAP_<colour>_HD1..3 by
    // player level, or WEAP_HOLD for classes without per-colour weapons.
    if !data.class.is_empty() {
        let hand = CLASS_HAND_BONES.iter().find(|(c, _)| *c == data.class).map_or("R_WRIST", |(_, b)| *b);
        if let Some(i) = data.skeleton.node_index(hand) {
            let weapon = object_index(&format!("WEAP_{}_HD1", data.colour)).or_else(|| object_index("WEAP_HOLD"));
            attach(weapon, bones[i], commands, &mut build);
        }
    }
    // Blob shadow under the character.
    attach(object_index("SHADOWL1"), root, commands, &mut build);

    // Skeletal nodes find their clip bone through the clips' own node of the
    // same name (players: the variant skeleton vs the class's shared clips;
    // monsters: the same atree).
    let clip_bone = data
        .skeleton
        .nodes
        .iter()
        .map(|n| data.clips.node_index(&n.name).and_then(|j| data.clips.clip_bone(j)))
        .collect();
    let mut animator = Animator {
        clips: data.clips.clone(),
        rest: data.skeleton.nodes.iter().map(|n| Vec3::from(n.offset)).collect(),
        bones,
        clip_bone,
        flipbooks,
        action: 0,
        frame: 0.0,
        tracks: Vec::new(),
    };
    animator.play(0);
    commands.entity(root).insert(animator);

    // Rest-pose joint positions widen the part-mesh bounds, which are in
    // each part's own (bone) space.
    let mut joints: Vec<Vec3> = Vec::with_capacity(data.skeleton.nodes.len());
    for n in &data.skeleton.nodes {
        let p = n.parent.map_or(Vec3::ZERO, |p| joints[p]) + Vec3::from(n.offset);
        bounds.0 = bounds.0.min(p);
        bounds.1 = bounds.1.max(p);
        joints.push(p);
    }
    (root, bounds.0, bounds.1)
}

fn animate(
    time: Res<Time>,
    mut animators: Query<&mut Animator>,
    mut bones: Query<&mut Transform>,
    mut slots: Query<(&mut Mesh3d, &mut MeshMaterial3d<LevelMaterial>, &mut Visibility)>,
) {
    for mut a in &mut animators {
        let Some(action) = a.clips.actions.get(a.action) else { continue };
        let (frames, rate, loops) = (action.frames as f32, action.rate.max(1) as f32, action.loops());
        a.frame += time.delta_secs() * rate;
        if frames > 1.0 {
            let last = frames - 1.0;
            if a.frame > last {
                a.frame = if loops { a.frame % last } else { last };
            }
        } else {
            a.frame = 0.0;
        }

        for (i, &bone) in a.bones.iter().enumerate() {
            let Ok(mut t) = bones.get_mut(bone) else { continue };
            *t = match &a.tracks[i] {
                Some(track) => {
                    let pose = track.sample(a.frame);
                    let m = Mat4::from_cols_array(&rotation_matrix(pose.rotation, track.flags));
                    Transform {
                        translation: a.rest[i] + Vec3::from(pose.translation),
                        rotation: Quat::from_mat4(&m),
                        scale: Vec3::from(pose.scale),
                    }
                }
                None => Transform::from_translation(a.rest[i]),
            };
        }

        let (action, frame) = (a.action, a.frame as usize);
        for (_, book) in &mut a.flipbooks {
            let Some(frames) = book.frames.get(action) else { continue };
            let k = frame.min(frames.len().saturating_sub(1));
            if book.shown == Some((action, k)) {
                continue;
            }
            book.shown = Some((action, k));
            let meshes = frames.get(k).map(Vec::as_slice).unwrap_or_default();
            for (s, &slot) in book.slots.iter().enumerate() {
                let Ok((mut mesh, mut material, mut vis)) = slots.get_mut(slot) else { continue };
                match meshes.get(s) {
                    Some((m, mat)) => {
                        mesh.0 = m.clone();
                        material.0 = mat.clone();
                        *vis = Visibility::Inherited;
                    }
                    None => *vis = Visibility::Hidden,
                }
            }
        }
    }
}
