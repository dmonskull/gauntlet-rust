//! Characters: segmented models posed by a skeleton and animated by the
//! game's keyframe clips (`docs/animation-format.md`).

use std::sync::Arc;

use bevy::prelude::*;
use gdl_formats::ModelFile;
use gdl_formats::anim::{AnimFile, Atree, Track, rotation_matrix};
use gdl_install::GameInstall;

use crate::level_material::LevelMaterial;
use crate::model_mesh::{self, TextureCache};

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate);
    }
}

/// Everything needed to spawn one player class in one colour/armour.
pub struct CharacterData {
    /// e.g. `ARC/BLU`.
    pub name: String,
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
    Ok(CharacterData { name: format!("{class}/{variant}"), skeleton, clips: Arc::new(clips), model, textures })
}

fn first_atree(data: &[u8]) -> Result<Atree, String> {
    AnimFile::parse(data)
        .map_err(|e| e.to_string())?
        .atrees
        .into_iter()
        .next()
        .ok_or_else(|| "no skeleton".to_string())
}

/// Drives a spawned character's bones.
#[derive(Component)]
pub struct Animator {
    pub clips: Arc<Atree>,
    /// One entity per skeleton node.
    bones: Vec<Entity>,
    rest: Vec<Vec3>,
    /// Skeleton node → clip bone, matched by name.
    clip_bone: Vec<Option<usize>>,
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

/// Spawns `data` posed at `transform`; returns the root entity.
pub fn spawn_character(
    data: &CharacterData,
    transform: Transform,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<LevelMaterial>,
    images: &mut Assets<Image>,
) -> Entity {
    let root = commands.spawn((transform, Visibility::default())).id();
    let mut cache = TextureCache::new(&data.model, &data.textures);
    let mut bones: Vec<Entity> = Vec::with_capacity(data.skeleton.nodes.len());
    let mut bounds = (Vec3::MAX, Vec3::MIN);

    for node in &data.skeleton.nodes {
        let parent = node.parent.map_or(root, |p| bones[p]);
        let bone = commands
            .spawn((Transform::from_translation(Vec3::from(node.offset)), Visibility::default(), ChildOf(parent)))
            .id();
        let part = format!("{}{}", data.skeleton.name, node.name);
        if let Some(object) = data.model.objects.iter().position(|o| o.name == part) {
            for b in model_mesh::build(&data.model, &mut cache, [(object, Vec3::ZERO)], meshes, materials, images, &mut bounds) {
                commands.spawn((Mesh3d(b.mesh), MeshMaterial3d(b.material), ChildOf(bone)));
            }
        }
        bones.push(bone);
    }

    let clip_bone = data.skeleton.nodes.iter().map(|n| data.clips.node_index(&n.name)).collect();
    let mut animator = Animator {
        clips: data.clips.clone(),
        rest: data.skeleton.nodes.iter().map(|n| Vec3::from(n.offset)).collect(),
        bones,
        clip_bone,
        action: 0,
        frame: 0.0,
        tracks: Vec::new(),
    };
    animator.play(0);
    commands.entity(root).insert(animator);
    root
}

fn animate(time: Res<Time>, mut animators: Query<&mut Animator>, mut bones: Query<&mut Transform>) {
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
    }
}
