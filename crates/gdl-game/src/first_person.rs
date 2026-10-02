//! Optional personal views. Preferences stay on the player's machine; the
//! look stick and mode travel with their inputs so movement remains lockstep.
//! Local co-op uses separate image targets (including the gamma pass) whenever
//! any local player chooses first person, composited under the existing HUD.

use crate::camera::{FlyCamera, FreeLook, MirroredPerspective};
use crate::character::Animator;
use crate::level_material::LevelMaterial;
use crate::locomotion::wrap;
use crate::party::{MAX_PLAYERS, Party, SlotInput};
use crate::play_camera::PlayCamera;
use crate::player::Player;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Hdr;
use bevy::transform::TransformSystems;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use gdl_formats::detmath::Det;
use std::collections::{HashMap, HashSet};

const TURN_RATE: f32 = 2.4;
const PITCH_LIMIT: f32 = 1.35;

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub on: bool,
    pub yaw: f32,
    pub pitch: f32,
    previous: (f32, f32),
}

impl Look {
    pub fn new(facing: f32) -> Self {
        Self { on: false, yaw: facing, pitch: 0.0, previous: (facing, 0.0) }
    }

    pub fn tick(&mut self, input: SlotInput, facing: f32, dt: f32) {
        let on = input.held & SlotInput::FIRST_PERSON != 0;
        if on != self.on {
            *self = Self::new(facing);
            self.on = on;
        }
        self.previous = (self.yaw, self.pitch);
        if on {
            let rate = if input.held & SlotInput::MOUSE_LOOK != 0 { 12.0 } else { TURN_RATE };
            self.yaw = wrap(self.yaw + input.c_stick.x * rate * dt);
            self.pitch = (self.pitch + input.c_stick.y * rate * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
    }

    pub fn direction(&self) -> Vec3 {
        Vec3::new(self.yaw.dsin() * self.pitch.dcos(), self.pitch.dsin(), self.yaw.dcos() * self.pitch.dcos())
    }

    fn rotation(&self, t: f32) -> Quat {
        let yaw = self.previous.0 + wrap(self.yaw - self.previous.0) * t;
        let pitch = self.previous.1 + (self.pitch - self.previous.1) * t;
        // The game's +Z facing and mirrored projection: +X is screen right.
        Quat::from_rotation_y(yaw + std::f32::consts::PI) * Quat::from_rotation_x(pitch)
    }
}

/// Mouse movement is displacement, not a held stick. Keep every frame's
/// pixels until a local tick or an online input commit consumes them.
#[derive(Resource, Default)]
pub(crate) struct MouseLook {
    pixels: Vec2,
}
impl MouseLook {
    pub fn sample(&self, dt: f32) -> Vec2 {
        (self.pixels * 0.003 / (12.0 * dt)).clamp(Vec2::splat(-1.0), Vec2::ONE)
    }
    pub fn take(&mut self, dt: f32) -> Vec2 {
        let sample = self.sample(dt);
        self.pixels -= sample * (12.0 * dt / 0.003);
        sample
    }
}

fn collect_mouse(
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
    mut look: ResMut<MouseLook>,
    party: Res<Party>,
    options: Res<crate::options::GameOptions>,
    fe: Res<crate::frontend::Frontend>,
    online: Option<Res<crate::online::Online>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let locals: Vec<_> = party.members().filter(|(_, m)| !m.devices.remote).collect();
    let active = fe.playing() && !fe.menu_open() && windows.single().is_ok_and(|w| w.focused)
        && locals.iter().any(|&(slot, m)| (m.devices.keyboard || locals.len() == 1)
            && options.player(if online.is_some() { 0 } else { slot }).first_person);
    if active {
        look.pixels += Vec2::new(motion.delta.x, -motion.delta.y);
    } else {
        look.pixels = Vec2::ZERO;
    }
}

/// This machine's ear follows its personal view. Local co-op still shares
/// its one speaker mix; this only changes the primary player's listening pose.
#[derive(Resource, Default)]
pub struct Listener(pub Option<(Vec3, Vec3)>);

pub struct FirstPersonPlugin;
impl Plugin for FirstPersonPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Views>()
            .init_resource::<Listener>()
            .init_resource::<MouseLook>()
            .add_systems(bevy::app::RunFixedMainLoop, collect_mouse
                .in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop)
                .before(crate::player::sample_online).before(crate::online::drive))
            .add_systems(Update, draw.after(crate::play_camera::place).after(crate::player::interpolate))
            .add_systems(PostUpdate, pose_gear.after(TransformSystems::Propagate));
    }
}

#[derive(Component)]
pub(crate) struct PersonalCamera;
#[derive(Component)]
struct CompositeCamera;
#[derive(Component)]
struct ViewPart;
struct GearPane {
    slot: usize,
    camera: Entity,
    node: Entity,
    image: Handle<Image>,
    reticle: Entity,
}

#[derive(Resource, Default)]
struct Views {
    layout: Vec<usize>,
    size: UVec2,
    cameras: Vec<Entity>,
    nodes: Vec<Entity>,
    images: Vec<Handle<Image>>,
    composite: Option<Entity>,
    gear: Vec<GearPane>,
    gear_size: UVec2,
    copies: HashMap<(usize, Entity), Entity>,
}

/// The normalized rectangle of each screen. Two players split left/right;
/// three or four use a two-by-two grid, ordered by occupied player slot.
pub(crate) fn rect(index: usize, count: usize) -> (Vec2, Vec2) {
    if count <= 1 {
        (Vec2::ZERO, Vec2::ONE)
    } else if count == 2 {
        (Vec2::new(index as f32 * 0.5, 0.0), Vec2::new(0.5, 1.0))
    } else {
        (Vec2::new((index % 2) as f32 * 0.5, (index / 2) as f32 * 0.5), Vec2::splat(0.5))
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    (mut views, mut listener): (ResMut<Views>, ResMut<Listener>),
    party: Res<Party>,
    options: Res<crate::options::GameOptions>,
    free: Res<FreeLook>,
    play: Option<Res<PlayCamera>>,
    fe: Res<crate::frontend::Frontend>,
    online: Option<Res<crate::online::Online>>,
    fixed: Res<Time<Fixed>>,
    mut windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    players: Query<(Entity, &Player, &Transform, &Animator), Without<FlyCamera>>,
    children: Query<&Children>,
    mut main: Query<(Entity, &mut Camera, &mut Transform, &mut Projection), With<FlyCamera>>,
    mut personal: Query<(&mut Transform, &mut RenderLayers), (With<PersonalCamera>, Without<Player>, Without<FlyCamera>)>,
    mut images: ResMut<Assets<Image>>,
    (mut gear_nodes, mut gear_cameras): (Query<&mut Visibility, Without<Player>>, Query<&mut Camera, (Without<FlyCamera>, Without<PersonalCamera>)>),
) {
    listener.0 = None;
    let Some(play) = play else { return };
    let Ok((window, mut cursor)) = windows.single_mut() else {
        return;
    };
    let Ok((main_entity, mut camera, mut transform, mut projection)) = main.single_mut() else {
        return;
    };
    let locals: Vec<_> = party.members().filter(|(_, m)| !m.devices.remote).map(|(s, _)| s).collect();
    let enabled = |slot| options.player(if online.is_some() { 0 } else { slot }).first_person;
    let split = online.is_none() && locals.len() > 1 && locals.iter().any(|&s| enabled(s)) && !free.0;
    let layout = if split { locals.clone() } else { Vec::new() };
    let size = UVec2::new(window.physical_width().max(1), window.physical_height().max(1));
    if views.layout != layout || (!layout.is_empty() && views.size != size) {
        for e in std::mem::take(&mut views.cameras).into_iter().chain(std::mem::take(&mut views.nodes)) {
            commands.entity(e).despawn();
        }
        if let Some(e) = views.composite.take() {
            commands.entity(e).despawn();
        }
        for h in views.images.drain(..) {
            images.remove(h.id());
        }
        views.layout = layout;
        views.size = size;
        if !views.layout.is_empty() {
            views.composite = Some(commands.spawn((Camera2d, Camera { order: 10, ..default() }, CompositeCamera, IsDefaultUiCamera)).id());
            let slots = views.layout.clone();
            for (i, _) in slots.iter().enumerate() {
                let (offset, extent) = rect(i, slots.len());
                let dimensions = (size.as_vec2() * extent).as_uvec2().max(UVec2::ONE);
                let image = images.add(Image::new_target_texture(dimensions.x, dimensions.y, TextureFormat::Rgba8UnormSrgb, None));
                let c = commands
                    .spawn((
                        Camera3d::default(),
                        Camera { order: -10 + i as isize, ..default() },
                        RenderTarget::Image(image.clone().into()),
                        Hdr,
                        Tonemapping::None,
                        Projection::custom(MirroredPerspective::default()),
                        Transform::default(),
                        RenderLayers::from_layers(&[0, 5]),
                        PersonalCamera,
                    ))
                    .id();
                let n = commands
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Percent(offset.x * 100.0),
                            top: Val::Percent(offset.y * 100.0),
                            width: Val::Percent(extent.x * 100.0),
                            height: Val::Percent(extent.y * 100.0),
                            ..default()
                        },
                        ImageNode::new(image.clone()),
                        GlobalZIndex(-100),
                    ))
                    .id();
                views.cameras.push(c);
                views.nodes.push(n);
                views.images.push(image);
            }
            info!("personal local screens: {:?}", views.layout.iter().map(|s| s + 1).collect::<Vec<_>>());
        } else {
            info!("shared/full-screen view restored");
        }
    }
    camera.is_active = !split;
    // A transparent, independently depth-tested picture keeps the equipped
    // arms from intersecting walls. It uses the same models, materials and
    // skeleton pose as the world hero; it has no combat or collision state.
    let gear_slots = if !locals.iter().any(|&s| enabled(s)) || free.0 {
        Vec::new()
    } else if split {
        locals.clone()
    } else {
        play.watching.or_else(|| locals.first().copied()).into_iter().collect()
    };
    if views.gear.iter().map(|p| p.slot).collect::<Vec<_>>() != gear_slots || views.gear_size != size {
        for pane in views.gear.drain(..) {
            commands.entity(pane.camera).despawn();
            commands.entity(pane.node).despawn();
            commands.entity(pane.reticle).despawn();
            images.remove(pane.image.id());
        }
        views.gear_size = size;
        for (i, slot) in gear_slots.into_iter().enumerate() {
            let count = if split { locals.len() } else { 1 };
            let (offset, extent) = rect(i, count);
            let dimensions = (size.as_vec2() * extent).as_uvec2().max(UVec2::ONE);
            let image = images.add(Image::new_target_texture(dimensions.x, dimensions.y, TextureFormat::Rgba8UnormSrgb, None));
            let camera = commands
                .spawn((
                    Camera3d::default(),
                    Camera { order: -5 + i as isize, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
                    RenderTarget::Image(image.clone().into()),
                    Hdr,
                    Tonemapping::None,
                    projection_for(true),
                    Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
                    RenderLayers::layer(10 + slot),
                ))
                .id();
            let node = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(offset.x * 100.0),
                        top: Val::Percent(offset.y * 100.0),
                        width: Val::Percent(extent.x * 100.0),
                        height: Val::Percent(extent.y * 100.0),
                        ..default()
                    },
                    ImageNode::new(image.clone()),
                    GlobalZIndex(10),
                    Visibility::Hidden,
                ))
                .id();
            let reticle = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent((offset.x + extent.x * 0.5) * 100.0),
                        top: Val::Percent((offset.y + extent.y * 0.5) * 100.0),
                        width: Val::Px(6.0),
                        height: Val::Px(6.0),
                        margin: UiRect::all(Val::Px(-3.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 0.93, 0.72, 0.85)),
                    BorderColor::all(Color::srgba(0.05, 0.03, 0.01, 0.75)),
                    GlobalZIndex(11),
                    Visibility::Hidden,
                    RenderLayers::layer(0),
                ))
                .id();
            views.gear.push(GearPane { slot, camera, node, image, reticle });
        }
    }
    // The hero's meshes are visible to everyone except their own eye-level
    // camera. Layer 5 is the overhead view; level geometry remains layer 0.
    for (root, p, _, _) in &players {
        let layers: Vec<_> = (1..=MAX_PLAYERS).filter(|&n| n != p.slot + 1).chain(std::iter::once(5)).collect();
        for child in children.iter_descendants(root) {
            commands.entity(child).insert(RenderLayers::from_layers(&layers));
        }
    }
    let watching = play.watching.or_else(|| locals.first().copied());
    let in_scene = !play.settled() || free.0 || !fe.playing();
    let keyboard_look = window.focused
        && locals.iter().any(|&s| party.get(s).is_some_and(|m| m.devices.keyboard) && party.state(s).is_some_and(|s| s.alive) && enabled(s))
        && !in_scene
        && !fe.menu_open();
    cursor.visible = !keyboard_look;
    cursor.grab_mode = if keyboard_look { CursorGrabMode::Locked } else { CursorGrabMode::None };
    let t = fixed.overstep_fraction();
    let view = |slot: usize| {
        let mine = players.iter().find(|(_, p, _, _)| p.slot == slot);
        if let Some((_, p, body, _)) =
            mine.filter(|(_, p, _, _)| enabled(slot) && p.look.on && party.state(slot).is_some_and(|s| s.alive) && !in_scene && p.going_out.is_none())
        {
            let head = party.state(slot).map_or(4.4, |s| s.head_height);
            let eye = body.translation + Vec3::Y * head * body.scale.y;
            (Transform::from_translation(eye).with_rotation(p.look.rotation(t)), true)
        } else {
            let (eye, target) = play.view_for(slot, t);
            (Transform::from_translation(Vec3::from(eye)).looking_at(Vec3::from(target), Vec3::Y), false)
        }
    };
    if !free.0
        && let Some(slot) = watching
    {
        let (pose, fp) = view(slot);
        // Leave the original scripted camera untouched for normal play.
        if fp {
            *transform = pose;
            let ahead = pose.rotation * Vec3::NEG_Z;
            let ahead = Vec3::new(ahead.x, 0.0, ahead.z).normalize_or(Vec3::Z);
            listener.0 = Some((pose.translation, Vec3::new(ahead.z, 0.0, -ahead.x)));
        }
        commands.entity(main_entity).insert(RenderLayers::from_layers(&[0, if fp { slot + 1 } else { 5 }]));
        *projection = projection_for(fp);
    }
    for pane in &views.gear {
        let (_, fp) = view(pane.slot);
        let show = fp && !fe.menu_open();
        for e in [pane.node, pane.reticle] {
            commands.entity(e).insert(UiTargetCamera(views.composite.unwrap_or(main_entity)));
            if let Ok(mut visibility) = gear_nodes.get_mut(e) {
                *visibility = if show { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
        if let Ok(mut camera) = gear_cameras.get_mut(pane.camera) {
            camera.is_active = show;
        }
    }
    for (i, &slot) in views.layout.iter().enumerate() {
        if let Some(&e) = views.cameras.get(i)
            && let Ok((mut transform, mut layers)) = personal.get_mut(e)
        {
            let (pose, fp) = view(slot);
            *transform = pose;
            *layers = RenderLayers::from_layers(&[0, if fp { slot + 1 } else { 5 }]);
            commands.entity(e).insert(projection_for(fp));
        }
    }
}

/// Copy only the forearms, hands and their equipped descendants. Dynamic
/// power weapons and texture/colour changes are picked up from their live
/// entities. Retain the exact retail attack pose, with only a presentation
/// translation/scaling to frame it in each pane. The camera itself stays still.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pose_gear(
    mut commands: Commands,
    mut views: ResMut<Views>,
    party: Res<Party>,
    players: Query<(&Player, &GlobalTransform, &Animator)>,
    children: Query<&Children>,
    joints: Query<&GlobalTransform, Without<ViewPart>>,
    sources: Query<(&Mesh3d, &MeshMaterial3d<LevelMaterial>, &GlobalTransform, &InheritedVisibility, Option<&MeshTag>), Without<ViewPart>>,
    mut copies: Query<
        (&mut Mesh3d, &mut MeshMaterial3d<LevelMaterial>, &mut Transform, &mut GlobalTransform, &mut Visibility),
        (With<ViewPart>, Without<Player>),
    >,
) {
    let mut present = HashSet::new();
    for (p, root, animator) in &players {
        let Some(member) = party.get(p.slot) else {
            continue;
        };
        if !views.gear.iter().any(|v| v.slot == p.slot) {
            continue;
        }
        let head = party.state(p.slot).map_or(4.4, |s| s.head_height).max(1.0);
        let pane_count = views.gear.len();
        let (_, extent) = rect(0, pane_count);
        let aspect = views.gear_size.x as f32 * extent.x / (views.gear_size.y as f32 * extent.y).max(1.0);
        // Narrow local panes move the arms slightly further from the lens.
        let depth = 1.35 + (1.3 - aspect).max(0.0) * 1.6;
        let anchor = Mat4::from_scale(Vec3::splat(4.4 / head * 0.7))
            * Mat4::from_translation(Vec3::new(0.0, -head + head * 0.14, depth * head / 4.4))
            * root.to_matrix().inverse();
        for (side, (arm, wrist)) in animator.first_person_arms(&member.choice.class).into_iter().enumerate() {
            let hand: HashSet<_> = std::iter::once(wrist).chain(children.iter_descendants(wrist)).collect();
            let sleeve = match (joints.get(arm), joints.get(wrist)) {
                (Ok(elbow), Ok(wrist)) => {
                    let from = anchor.transform_point3(elbow.translation());
                    let to = anchor.transform_point3(wrist.translation());
                    // The world animation was made to show a whole hero.
                    // Extend only its forearm to a shoulder below the lens,
                    // so no severed elbow floats in the view. The hand and
                    // weapon keep the exact animated pose and hit timing.
                    let shoulder = Vec3::new(if side == 0 { 0.85 } else { -0.85 }, -1.35, 0.12);
                    connect_arm(from, to, shoulder)
                }
                _ => Mat4::IDENTITY,
            };
            for source in std::iter::once(arm).chain(children.iter_descendants(arm)) {
                let Ok((mesh, material, pose, visible, tag)) = sources.get(source) else {
                    continue;
                };
                let key = (p.slot, source);
                present.insert(key);
                let matrix = anchor * pose.to_matrix();
                let matrix = if hand.contains(&source) { matrix } else { sleeve * matrix };
                let transform = Transform::from_matrix(matrix);
                let visibility = if visible.get() { Visibility::Inherited } else { Visibility::Hidden };
                let entity = match views.copies.get(&key).copied() {
                    Some(e) => {
                        if let Ok((mut m, mut mat, mut t, mut global, mut v)) = copies.get_mut(e) {
                            m.0 = mesh.0.clone();
                            mat.0 = material.0.clone();
                            *t = transform;
                            *global = GlobalTransform::from(matrix);
                            *v = visibility;
                        }
                        e
                    }
                    None => {
                        let e =
                            commands.spawn((mesh.clone(), material.clone(), transform, GlobalTransform::from(matrix), visibility, RenderLayers::layer(10 + p.slot), ViewPart)).id();
                        views.copies.insert(key, e);
                        e
                    }
                };
                commands.entity(entity).insert(MeshTag(tag.map_or(0, |t| t.0)));
            }
        }
    }
    views.copies.retain(|key, e| {
        if present.contains(key) {
            true
        } else {
            commands.entity(*e).despawn();
            false
        }
    });
}

/// Map an arm's elbow to the off-screen shoulder and retain its wrist.
/// Stretch along its length only; its width and the hand/weapon are unchanged.
fn connect_arm(elbow: Vec3, wrist: Vec3, shoulder: Vec3) -> Mat4 {
    let old = wrist - elbow;
    let new = wrist - shoulder;
    if old.length_squared() < 0.0001 || new.length_squared() < 0.0001 {
        return Mat4::IDENTITY;
    }
    let direction = old.normalize();
    let extension = new.length() / old.length() - 1.0;
    let stretch = Mat3::IDENTITY + Mat3::from_cols(direction * direction.x, direction * direction.y, direction * direction.z) * extension;
    Mat4::from_translation(shoulder)
        * Mat4::from_quat(Quat::from_rotation_arc(direction, new.normalize()))
        * Mat4::from_mat3(stretch)
        * Mat4::from_translation(-elbow)
}

fn projection_for(first: bool) -> Projection {
    let fov = if first { 70f32.to_radians() } else { 2.0 * (0.75 * 30f32.to_radians().tan()).atan() };
    Projection::custom(MirroredPerspective(PerspectiveProjection { fov, near: 0.1, ..default() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mouse_pixels_survive_frames_without_ticks_and_are_used_once() {
        let dt = 1.0 / 30.0;
        let mut motion = MouseLook::default();
        for delta in [Vec2::new(3.0, -1.0), Vec2::new(2.0, 4.0), Vec2::new(1.0, 0.0)] {
            motion.pixels += delta;
        }
        let peek = motion.sample(dt);
        assert_eq!(peek, motion.sample(dt), "waiting online doesn't consume motion");
        assert_eq!(peek, motion.take(dt));
        assert!(motion.take(dt).length() < 0.00001);
        assert!((peek * (12.0 * dt) - Vec2::new(6.0, 3.0) * 0.003).length() < 0.00001);
        motion.pixels = Vec2::new(300.0, 0.0);
        let total = motion.take(dt) + motion.take(dt) + motion.take(dt);
        assert!((total.x * (12.0 * dt) - 0.9).abs() < 0.00001, "fast turns retain their remainder");
    }
    #[test]
    fn looking_is_per_player_and_does_not_turn_back_on_toggle() {
        let mut a = Look::new(1.0);
        let b = Look::new(2.0);
        let input = SlotInput { held: SlotInput::FIRST_PERSON, c_stick: Vec2::ONE, ..default() };
        a.tick(input, 1.0, 1.0);
        assert!(a.on && !b.on);
        assert_eq!(b.yaw, 2.0);
        assert_eq!(a.pitch, PITCH_LIMIT);
        a.tick(SlotInput::default(), 0.5, 1.0);
        assert!(!a.on);
        assert_eq!(a.yaw, 0.5);
    }
    #[test]
    fn switching_off_stops_look_and_restores_the_classic_facing() {
        let mut look = Look::new(0.0);
        look.tick(SlotInput { held: SlotInput::FIRST_PERSON, c_stick: Vec2::ONE, ..default() }, 0.0, 0.1);
        assert!(look.on && look.pitch > 0.0);
        let classic = SlotInput { c_stick: Vec2::ONE, ..default() };
        look.tick(classic, 1.2, 0.1);
        assert!(!look.on);
        assert_eq!((look.yaw, look.pitch), (1.2, 0.0));
        look.tick(classic, 1.2, 0.1);
        assert_eq!((look.yaw, look.pitch), (1.2, 0.0), "the classic C-stick doesn't move a first-person gaze");
    }
    #[test]
    fn screen_rectangles_fit_and_do_not_overlap() {
        for n in 1..=4 {
            for i in 0..n {
                let (at, size) = rect(i, n);
                assert!(at.cmpge(Vec2::ZERO).all() && (at + size).cmple(Vec2::ONE).all());
                for j in 0..i {
                    let (b, bs) = rect(j, n);
                    assert!((at.cmpge(b + bs) | b.cmpge(at + size)).any());
                }
            }
        }
    }
    #[test]
    fn gaze_and_camera_point_the_same_way() {
        let mut look = Look::new(0.8);
        look.tick(SlotInput { held: SlotInput::FIRST_PERSON | SlotInput::MOUSE_LOOK, c_stick: Vec2::new(0.1, 0.05), ..default() }, 0.8, 1.0 / 30.0);
        assert!((look.yaw - 0.84).abs() < 0.00001);
        assert!((look.rotation(1.0) * Vec3::NEG_Z - look.direction()).length() < 0.00001);
    }
    #[test]
    fn framing_the_sleeve_preserves_the_animated_wrist() {
        let elbow = Vec3::new(0.8, -0.3, 1.4);
        let wrist = Vec3::new(0.7, -0.6, 2.0);
        let shoulder = Vec3::new(0.85, -1.35, 0.12);
        let matrix = connect_arm(elbow, wrist, shoulder);
        assert!((matrix.transform_point3(wrist) - wrist).length() < 0.00001);
        assert!((matrix.transform_point3(elbow) - shoulder).length() < 0.00001);
    }
}
