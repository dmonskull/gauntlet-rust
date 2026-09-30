//! Characters: segmented models posed by a skeleton and animated by the
//! game's keyframe clips (`docs/animation-format.md`).

use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::{AnimFile, Atree, NodeKind, Track, rotation_matrix};
use gdl_install::GameInstall;

use crate::billboard::Billboard;
use crate::level_material::LevelMaterial;
use crate::model_mesh::{self, TextureCache};

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate.in_set(Animate));
    }
}

/// Posing characters for the frame (bones, and flipbook meshes and
/// materials swapped in).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Animate;

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

/// The game's index for a player class (its position in the class order).
pub fn class_index(class: &str) -> Option<usize> {
    CLASS_HAND_BONES.iter().position(|(c, _)| c.eq_ignore_ascii_case(class))
}

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

/// Height of blob shadows above the feet.
const SHADOW_LIFT: f32 = 0.1;

/// The meshes one model object is drawn with.
type PartMeshes = Vec<(Handle<Mesh>, Handle<LevelMaterial>)>;

/// A flipbook node's meshes: per action, the model objects for each frame.
struct Flipbook {
    /// The node's child entities that show the current frame's meshes.
    slots: Vec<Entity>,
    /// `frames[action][frame]` = the meshes of that frame's object; shared
    /// by every instance of the character.
    frames: Arc<Vec<Vec<PartMeshes>>>,
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
    blend: Blend,
}

/// Blending from the pose an action was interrupted in: the old pose's
/// weight falls from 1 to 0 over `duration` seconds.
#[derive(Default)]
enum Blend {
    #[default]
    None,
    /// Snapshot the bones on the next frame.
    Pending { duration: f32 },
    Active { from: Vec<Transform>, elapsed: f32, duration: f32 },
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
        self.play_blended(name, 0.0)
    }

    /// Starts `name` unless it's already playing, blending from the current
    /// pose over `blend` seconds.
    pub fn play_blended(&mut self, name: &str, blend: f32) -> bool {
        match self.clips.actions.iter().position(|a| a.name == name) {
            Some(i) => {
                if i != self.action || self.tracks.is_empty() {
                    self.play(i);
                    if blend > 0.0 {
                        self.blend = Blend::Pending { duration: blend };
                    }
                }
                true
            }
            None => false,
        }
    }

    /// The entity posing skeleton node `node` (its `GlobalTransform` is the
    /// node's world matrix as of the last frame).
    pub fn bone(&self, node: usize) -> Option<Entity> {
        self.bones.get(node).copied()
    }

    /// Name of the action playing.
    pub fn action_name(&self) -> &str {
        self.clips.actions.get(self.action).map_or("", |a| a.name.as_str())
    }

    /// A non-looping action has played out (see [`clip_end`]).
    pub fn finished(&self) -> bool {
        self.clips.actions.get(self.action).is_none_or(|a| !a.loops() && self.frame >= clip_end(a.frames))
    }
}

/// Frames a second an action with this rate plays at. The game gives each
/// frame rate / 30 × its clock's 1/30 s, i.e. rate / 900 seconds
/// (`docs/animation-format.md`): 30 plays at 30 frames a second, 60 at 15,
/// 45 at 20, 15 at 60. A rate of 0 plays at 30.
pub fn clip_fps(rate: u16) -> f32 {
    if rate == 0 { 30.0 } else { 900.0 / f32::from(rate) }
}

/// Where a clip ends, in frames: the game shows the nearest frame and is
/// done once that would be past the last one, half a frame after the last
/// frame comes up. A clip with no frames is over at once.
pub fn clip_end(frames: u16) -> f32 {
    (f32::from(frames) - 0.5).max(0.0)
}

/// A looping clip's length in frames: it starts over on the first 30 Hz
/// game tick at or past its end, so a 30-rate loop of `n` frames lasts `n`
/// ticks and a 60-rate one `2n − 1`.
pub fn loop_length(frames: u16, rate: u16) -> f32 {
    let per_tick = clip_fps(rate) / 30.0;
    ((clip_end(frames) / per_tick).ceil() * per_tick).max(per_tick)
}

/// Moves a clip's frame on by `dt` seconds: a looping clip wraps round (the
/// result says it did), any other stops at its end.
pub fn advance_clip(frame: &mut f32, dt: f32, frames: u16, rate: u16, loops: bool) -> bool {
    *frame += dt * clip_fps(rate);
    if frames == 0 {
        *frame = 0.0;
        return false;
    }
    if loops {
        let length = loop_length(frames, rate);
        if *frame >= length {
            *frame %= length;
            return true;
        }
    } else {
        *frame = frame.min(clip_end(frames));
    }
    false
}

/// The frame of a flipbook to show: the nearest, as the game rounds it.
pub fn shown_frame(frame: f32, frames: usize) -> usize {
    ((frame + 0.5) as usize).min(frames.saturating_sub(1))
}

/// A character's meshes, built once and shared by every copy spawned with
/// [`CharacterModel::spawn`] (monsters come and go by the dozen).
pub struct CharacterModel {
    /// Per skeleton node: rest offset and parent.
    nodes: Vec<(Vec3, Option<usize>)>,
    /// Per skeleton node: the meshes of its own model object, if drawn, and
    /// the camera-facing mode its render flags ask for.
    parts: Vec<(PartMeshes, Option<Billboard>)>,
    /// Flipbook nodes and their frames.
    flipbooks: Vec<(usize, Arc<Vec<Vec<PartMeshes>>>)>,
    /// A player's weapon, in its hand bone.
    weapon: Option<(usize, PartMeshes)>,
    shadow: PartMeshes,
    clips: Arc<Atree>,
    clip_bone: Vec<Option<usize>>,
    /// Rough rest-pose bounds in the root's space.
    pub bounds: (Vec3, Vec3),
}

impl CharacterModel {
    /// Builds every mesh `data` can show: each node's part, every flipbook
    /// frame, the weapon and the blob shadow.
    pub fn build(
        data: &CharacterData,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<LevelMaterial>,
        images: &mut Assets<Image>,
    ) -> Self {
        let mut cache = TextureCache::new(&data.model, &data.textures).sharing_materials();
        let mut bounds = (Vec3::MAX, Vec3::MIN);
        let object_index = |name: &str| data.model.objects.iter().position(|o| o.name == name);
        let mut build = |object: Option<usize>, flags: u32| -> PartMeshes {
            let Some(object) = object else { return Vec::new() };
            let instance = [(object, Vec3::ZERO, flags)];
            model_mesh::build_flagged(&data.model, &mut cache, instance, meshes, materials, images, &mut bounds)
                .into_iter()
                .map(|b| (b.mesh, b.material))
                .collect()
        };

        let mut parts = Vec::with_capacity(data.skeleton.nodes.len());
        let mut flipbooks = Vec::new();
        for (i, node) in data.skeleton.nodes.iter().enumerate() {
            // The game hides DUMMY nodes and nodes with the hidden render flag.
            // The rest draw with their render flags. Glows (CFXPGLOW: additive,
            // camera-facing) get their texture from the effects system at run
            // time (the instance's texture override), so they wait for effects.
            let visible = node.has_model() && !node.hidden() && node.name != "DUMMY" && !node.name.ends_with("GLOW");
            let own = if visible { object_index(&format!("{}{}", data.skeleton.name, node.name)) } else { None };
            parts.push((build(own, node.render_flags), Billboard::from_flags(node.render_flags)));
            if node.kind == NodeKind::Flipbook {
                let frames: Vec<Vec<PartMeshes>> = (0..data.clips.actions.len())
                    .map(|a| {
                        let Some(entry) = data.skeleton.flipbook_entry(i, a) else { return Vec::new() };
                        let Some(first) = object_index(&entry.first) else { return Vec::new() };
                        (0..entry.frames.max(1) as usize)
                            .filter(|k| first + k < data.model.objects.len())
                            .map(|k| build(Some(first + k), 0))
                            .collect()
                    })
                    .collect();
                flipbooks.push((i, Arc::new(frames)));
            }
        }

        // The weapon goes in the class's hand bone: WEAP_<colour>_HD1..3 by
        // player level, or WEAP_HOLD for classes without per-colour weapons.
        let weapon = if data.class.is_empty() {
            None
        } else {
            let hand = CLASS_HAND_BONES.iter().find(|(c, _)| *c == data.class).map_or("R_WRIST", |(_, b)| *b);
            data.skeleton.node_index(hand).map(|bone| {
                let weapon = object_index(&format!("WEAP_{}_HD1", data.colour)).or_else(|| object_index("WEAP_HOLD"));
                (bone, build(weapon, 0))
            })
        };
        // Blob shadow under the character.
        let shadow = build(object_index("SHADOWL1"), 0);

        // Skeletal nodes find their clip bone through the clips' own node of the
        // same name (players: the variant skeleton vs the class's shared clips;
        // monsters: the same atree).
        let clip_bone = data
            .skeleton
            .nodes
            .iter()
            .map(|n| data.clips.node_index(&n.name).and_then(|j| data.clips.clip_bone(j)))
            .collect();

        // Rest-pose joint positions widen the part-mesh bounds, which are in
        // each part's own (bone) space.
        let mut joints: Vec<Vec3> = Vec::with_capacity(data.skeleton.nodes.len());
        for n in &data.skeleton.nodes {
            let p = n.parent.map_or(Vec3::ZERO, |p| joints[p]) + Vec3::from(n.offset);
            bounds.0 = bounds.0.min(p);
            bounds.1 = bounds.1.max(p);
            joints.push(p);
        }

        Self {
            nodes: data.skeleton.nodes.iter().map(|n| (Vec3::from(n.offset), n.parent)).collect(),
            parts,
            flipbooks,
            weapon,
            shadow,
            clips: data.clips.clone(),
            clip_bone,
            bounds,
        }
    }

    /// Spawns a copy posed at `transform`, with an [`Animator`] on its root
    /// entity, which is returned.
    /// Every material the model draws with (its parts, flipbook frames and
    /// weapon; not the shadow). Built for this model alone.
    pub fn materials(&self) -> impl Iterator<Item = &Handle<LevelMaterial>> {
        let parts = self.parts.iter().flat_map(|(p, _)| p.iter());
        let frames = self.flipbooks.iter().flat_map(|(_, f)| f.iter().flatten().flatten());
        let weapon = self.weapon.iter().flat_map(|(_, p)| p.iter());
        parts.chain(frames).chain(weapon).map(|(_, m)| m)
    }

    pub fn spawn(&self, transform: Transform, commands: &mut Commands) -> Entity {
        let root = commands.spawn((transform, Visibility::default())).id();
        let attach = |parts: &PartMeshes, parent: Entity, commands: &mut Commands| {
            for (mesh, material) in parts {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), ChildOf(parent)));
            }
        };
        let mut bones: Vec<Entity> = Vec::with_capacity(self.nodes.len());
        for (i, &(offset, parent)) in self.nodes.iter().enumerate() {
            let parent = parent.map_or(root, |p| bones[p]);
            let bone = commands.spawn((Transform::from_translation(offset), Visibility::default(), ChildOf(parent))).id();
            let (parts, facing) = &self.parts[i];
            let target = match facing {
                Some(mode) if !parts.is_empty() => {
                    commands.spawn((Transform::default(), Visibility::default(), *mode, ChildOf(bone))).id()
                }
                _ => bone,
            };
            attach(parts, target, commands);
            bones.push(bone);
        }
        let flipbooks = self
            .flipbooks
            .iter()
            .map(|(i, frames)| {
                // Slots start with any frame's meshes so they carry the mesh and
                // material components the animator swaps each frame.
                let slots_needed = frames.iter().flatten().map(Vec::len).max().unwrap_or(0);
                let sample: Vec<_> = frames.iter().flatten().flatten().cloned().collect();
                let slots = (0..slots_needed)
                    .map(|s| {
                        let (mesh, material) = sample[s.min(sample.len() - 1)].clone();
                        let slot = (Mesh3d(mesh), MeshMaterial3d(material), Visibility::Hidden, ChildOf(bones[*i]));
                        commands.spawn(slot).id()
                    })
                    .collect();
                (*i, Flipbook { slots, frames: frames.clone(), shown: None })
            })
            .collect();
        if let Some((bone, parts)) = &self.weapon {
            attach(parts, bones[*bone], commands);
        }
        // Lifted off the floor it lies on (the collision floor can sit a
        // little below the drawn one).
        let lift = commands.spawn((Transform::from_xyz(0.0, SHADOW_LIFT, 0.0), Visibility::default(), ChildOf(root))).id();
        attach(&self.shadow, lift, commands);

        let mut animator = Animator {
            clips: self.clips.clone(),
            rest: self.nodes.iter().map(|n| n.0).collect(),
            bones,
            clip_bone: self.clip_bone.clone(),
            flipbooks,
            action: 0,
            frame: 0.0,
            tracks: Vec::new(),
            blend: Blend::None,
        };
        animator.play(0);
        commands.entity(root).insert(animator);
        root
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
    let model = CharacterModel::build(data, meshes, materials, images);
    let root = model.spawn(transform, commands);
    (root, model.bounds.0, model.bounds.1)
}

fn animate(
    time: Res<Time>,
    mut animators: Query<&mut Animator>,
    mut bones: Query<&mut Transform>,
    mut slots: Query<(&mut Mesh3d, &mut MeshMaterial3d<LevelMaterial>, &mut Visibility)>,
) {
    for mut a in &mut animators {
        let Some(action) = a.clips.actions.get(a.action) else { continue };
        let (frames, rate, loops) = (action.frames, action.rate, action.loops());
        advance_clip(&mut a.frame, time.delta_secs(), frames, rate, loops);
        // Tracks are sampled between keys; past the last frame they hold it.
        let sample_at = a.frame.min(f32::from(frames.saturating_sub(1)));

        if let Blend::Pending { duration } = a.blend {
            let from = a.bones.iter().map(|&b| bones.get(b).copied().unwrap_or_default()).collect();
            a.blend = Blend::Active { from, elapsed: 0.0, duration };
        }
        let weight = match &mut a.blend {
            Blend::Active { elapsed, duration, .. } => {
                *elapsed += time.delta_secs();
                (1.0 - *elapsed / *duration).max(0.0)
            }
            _ => 0.0,
        };
        let a = &mut *a;
        for (i, &bone) in a.bones.iter().enumerate() {
            let Ok(mut t) = bones.get_mut(bone) else { continue };
            let pose = match &a.tracks[i] {
                Some(track) => {
                    let pose = track.sample(sample_at);
                    let m = Mat4::from_cols_array(&rotation_matrix(pose.rotation, track.flags));
                    Transform {
                        translation: a.rest[i] + Vec3::from(pose.translation),
                        rotation: Quat::from_mat4(&m),
                        scale: Vec3::from(pose.scale),
                    }
                }
                None => Transform::from_translation(a.rest[i]),
            };
            *t = match &a.blend {
                Blend::Active { from, .. } if weight > 0.0 => Transform {
                    translation: pose.translation.lerp(from[i].translation, weight),
                    rotation: pose.rotation.slerp(from[i].rotation, weight),
                    scale: pose.scale.lerp(from[i].scale, weight),
                },
                _ => pose,
            };
        }
        if weight <= 0.0 && matches!(a.blend, Blend::Active { .. }) {
            a.blend = Blend::None;
        }

        let (action, frame) = (a.action, a.frame);
        for (_, book) in &mut a.flipbooks {
            let Some(frames) = book.frames.get(action) else { continue };
            let k = shown_frame(frame, frames.len());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_rates_are_time_per_frame() {
        assert_eq!(clip_fps(30), 30.0);
        assert_eq!(clip_fps(60), 15.0);
        assert_eq!(clip_fps(45), 20.0);
        assert_eq!(clip_fps(15), 60.0);
        assert_eq!(clip_fps(0), 30.0);
    }

    #[test]
    fn clips_end_half_a_frame_after_their_last() {
        // A grunt's 5-frame ATTACK1 at rate 60 lasts 4.5 frames of 1/15 s.
        let (mut frame, mut ticks) = (0.0, 0);
        while frame < clip_end(5) {
            advance_clip(&mut frame, 1.0 / 30.0, 5, 60, false);
            ticks += 1;
        }
        assert_eq!(ticks, 9);
        assert_eq!(frame, 4.5);
        assert_eq!(shown_frame(frame, 5), 4);
    }

    #[test]
    fn loops_restart_on_the_tick_past_their_end() {
        assert_eq!(loop_length(30, 30), 30.0);
        assert_eq!(loop_length(16, 60), 15.5);
        // A 30-frame walk at rate 30 comes round every 30 ticks.
        let mut frame = 0.0;
        let wraps = (0..90).filter(|_| advance_clip(&mut frame, 1.0 / 30.0, 30, 30, true)).count();
        assert_eq!(wraps, 3);
    }
}
