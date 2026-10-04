//! The menus' arrow (`docs/frontend.md`, "The arrow"): the game's
//! `ICON_ARROW` model from the powerups bank, pointing at the selected
//! line.
//!
//! The game draws it in the 3D scene: its tip 1.1 in front of the camera
//! at the line's place on the screen, a twentieth of its size, turned with
//! the camera — its length along the camera's right — and rolled about
//! that length by the angle the menu gives (half a turn as it slides to
//! another line). Here the menus are a 2D layer over the picture, so the
//! arrow is drawn by a camera of its own into a small picture the menu
//! lays over its panel ([`MenuArrow::show`]): the same model, the same
//! place in the same frustum (the camera draws just the part of the
//! game's 512 × 384 view around the arrow), lit by the level's light as
//! the scene's camera would see it.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{CameraProjection, RenderTarget, SubCameraView};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::math::Vec3A;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Hdr;
use bevy::transform::TransformSystems;
use gdl_formats::ModelFile;
use gdl_formats::anim::AnimFile;
use gdl_formats::detmath::Det;

use crate::camera::FlyCamera;
use crate::font::{SCREEN_H, SCREEN_W, UiImage};
use crate::level::LoadedGame;
use crate::level_material::{LevelMaterial, SceneLight};
use crate::model_mesh::{self, TextureCache};

/// The bank and the model.
const BANK: &str = "POWERUPS";
const MODEL: &str = "ICON_ARROW";
/// How far in front of the camera its tip is, and its size.
const DEPTH: f32 = 1.1;
const SCALE: f32 = 0.05;
/// The game's view: 60° across its 4:3 picture.
const HALF_ACROSS: f32 = 30.0;
/// The part of the 2D screen its picture covers, about its tip: this far
/// to the left and right, and up and down.
const LEFT: f32 = 96.0;
const RIGHT: f32 = 16.0;
const HALF_HEIGHT: f32 = 40.0;
/// Picture pixels to a 2D screen pixel.
const DETAIL: u32 = 4;
/// Nothing else is drawn on this layer.
const LAYER: usize = 20;
const NEAR: f32 = 0.05;
/// What its picture is cleared to: the arrow's own pale metal with no
/// alpha, so its edges blend toward its colour (a gamma value, as the
/// level materials write).
const CLEAR: Color = Color::linear_rgba(0.5, 0.6, 0.52, 0.0);

pub struct MenuArrowPlugin;

impl Plugin for MenuArrowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build).add_systems(PostUpdate, place.before(TransformSystems::Propagate));
    }
}

/// The arrow's rig and this frame's place for it.
#[derive(Resource)]
pub struct MenuArrow {
    image: Handle<Image>,
    root: Entity,
    camera: Entity,
    materials: Vec<Handle<LevelMaterial>>,
    /// Where its tip is on the 2D screen and how far it's rolled, if a
    /// menu drew it this frame.
    wanted: Option<(Vec2, f32)>,
}

impl MenuArrow {
    /// The arrow with its tip at `tip` on the 2D screen, rolled by `turn`
    /// radians: its picture, and where on the 2D screen to draw it (left,
    /// top, width, height).
    pub fn show(&mut self, tip: Vec2, turn: f32) -> (UiImage, [f32; 4]) {
        self.wanted = Some((tip, turn));
        let size = Vec2::new(LEFT + RIGHT, 2.0 * HALF_HEIGHT);
        (UiImage { handle: self.image.clone(), size }, [tip.x - LEFT, tip.y - HALF_HEIGHT, size.x, size.y])
    }
}

/// The game's view of the part of its 2D screen about the arrow's tip:
/// its perspective (mirrored, `camera.rs`), cut down to the picture.
#[derive(Debug, Clone)]
struct Window {
    /// The tip on the 2D screen.
    tip: Vec2,
}

impl Window {
    /// Screen units a unit away from the middle is, a unit in front of the
    /// camera, across and up (the 2D screen's pixels are square).
    fn focal() -> f32 {
        0.5 * SCREEN_W / HALF_ACROSS.to_radians().dtan()
    }
}

impl CameraProjection for Window {
    fn get_clip_from_view(&self) -> Mat4 {
        // The picture's middle and half size in the full view's clip space.
        let middle = Vec2::new(self.tip.x + 0.5 * (RIGHT - LEFT), self.tip.y);
        let c = Vec2::new((middle.x - 0.5 * SCREEN_W) / (0.5 * SCREEN_W), -(middle.y - 0.5 * SCREEN_H) / (0.5 * SCREEN_H));
        let h = Vec2::new(0.5 * (LEFT + RIGHT) / (0.5 * SCREEN_W), HALF_HEIGHT / (0.5 * SCREEN_H));
        let across = Self::focal() / (0.5 * SCREEN_W);
        let up = Self::focal() / (0.5 * SCREEN_H);
        // Mirrored: the view's +X is the screen's left.
        Mat4::from_cols(
            Vec4::new(-across / h.x, 0.0, 0.0, 0.0),
            Vec4::new(0.0, up / h.y, 0.0, 0.0),
            Vec4::new(c.x / h.x, c.y / h.y, 0.0, -1.0),
            Vec4::new(0.0, 0.0, NEAR, 0.0),
        )
    }

    fn get_clip_from_view_for_sub(&self, _: &SubCameraView) -> Mat4 {
        self.get_clip_from_view()
    }

    fn update(&mut self, _width: f32, _height: f32) {}

    fn far(&self) -> f32 {
        100.0
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        // Wide enough for the whole view; the arrow's parts aren't culled.
        let corner = |x: f32, y: f32, z: f32| Vec3A::new(x * z, y * z, -z);
        [
            corner(1.0, -1.0, z_near),
            corner(1.0, 1.0, z_near),
            corner(-1.0, 1.0, z_near),
            corner(-1.0, -1.0, z_near),
            corner(1.0, -1.0, z_far),
            corner(1.0, 1.0, z_far),
            corner(-1.0, 1.0, z_far),
            corner(-1.0, -1.0, z_far),
        ]
    }
}

/// Where the arrow's root goes for a tip at `tip` rolled by `turn`, in the
/// arrow camera's space (the camera sits at the origin, unturned): the
/// game's camera space — right, up, forward — is its −X, +Y, −Z.
fn pose(tip: Vec2, turn: f32) -> Transform {
    let f = Window::focal();
    let (right, up) = (DEPTH * (tip.x - 0.5 * SCREEN_W) / f, -DEPTH * (tip.y - 0.5 * SCREEN_H) / f);
    Transform {
        translation: Vec3::new(-right, up, -DEPTH),
        // The game copies the camera's axes, then turns the model's Y and Z
        // about its X: Y' = Y cos − Z sin, Z' = Z cos + Y sin.
        rotation: Quat::from_rotation_y(std::f32::consts::PI) * Quat::from_rotation_x(-turn),
        scale: Vec3::splat(SCALE),
    }
}

/// Builds the arrow from the powerups bank, with its camera and picture.
fn build(
    mut commands: Commands,
    mut game: ResMut<LoadedGame>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<LevelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut read = |name: &str| game.install.read(&format!("{BANK}/{name}")).map_err(|e| e.to_string());
    let files = read("objects.ngc")
        .and_then(|o| ModelFile::parse(&o).map_err(|e| e.to_string()))
        .and_then(|m| Ok((m, read("textures.ngc")?, read("ANIM.PS2")?)))
        .and_then(|(m, t, a)| Ok((m, t, AnimFile::parse(&a).map_err(|e| e.to_string())?)));
    let (model, textures, anim) = match files {
        Ok(f) => f,
        Err(why) => {
            warn!("the menu arrow ({BANK}/{MODEL}): {why}");
            return;
        }
    };
    let Some(atree) = anim.atrees.iter().find(|a| a.name == MODEL) else {
        warn!("the menu arrow: no {MODEL} in {BANK}");
        return;
    };
    let layer = RenderLayers::layer(LAYER);
    let root = commands.spawn((Transform::default(), Visibility::Hidden, layer.clone())).id();
    let mut cache = TextureCache::new(&model, &textures);
    let mut bounds = (Vec3::MAX, Vec3::MIN);
    let mut bones: Vec<Entity> = Vec::with_capacity(atree.nodes.len());
    let mut own = Vec::new();
    for node in &atree.nodes {
        let parent = node.parent.map_or(root, |p| bones[p]);
        let bone = commands.spawn((Transform::from_translation(Vec3::from(node.offset)), Visibility::default(), ChildOf(parent))).id();
        bones.push(bone);
        if !node.has_model() || node.hidden() {
            continue;
        }
        let name = format!("{}{}", atree.name, node.name);
        let Some(object) = model.objects.iter().position(|o| o.name == name) else { continue };
        let at = [(object, Vec3::ZERO, node.render_flags)];
        for part in model_mesh::build_flagged(&model, &mut cache, at, &mut meshes, &mut materials, &mut images, &mut bounds) {
            // Lit by the level's light as the scene's camera sees it
            // (`place`), not as this camera does.
            if let Some(m) = materials.get_mut(&part.material) {
                m.own_light = true;
            }
            own.push(part.material.clone());
            commands.spawn((Mesh3d(part.mesh), MeshMaterial3d(part.material), ChildOf(bone), layer.clone(), NoFrustumCulling));
        }
    }
    if own.is_empty() {
        warn!("the menu arrow: {MODEL} has no parts in {BANK}");
        return;
    }
    let size = UVec2::new((LEFT + RIGHT) as u32, (2.0 * HALF_HEIGHT) as u32) * DETAIL;
    let image = images.add(Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None));
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera { order: -20, clear_color: ClearColorConfig::Custom(CLEAR), is_active: false, ..default() },
            RenderTarget::Image(image.clone().into()),
            Hdr,
            Tonemapping::None,
            Projection::custom(Window { tip: Vec2::ZERO }),
            Transform::default(),
            layer,
        ))
        .id();
    info!("the menu arrow: {} parts, size {:?}", own.len(), bounds.1 - bounds.0);
    commands.insert_resource(MenuArrow { image, root, camera, materials: own, wanted: None });
}

/// Puts the arrow where this frame's menu asked for it, lit as the scene's
/// camera would see it there; with no menu asking, its camera rests.
#[allow(clippy::type_complexity)]
fn place(
    arrow: Option<ResMut<MenuArrow>>,
    light: Res<SceneLight>,
    scene: Query<&GlobalTransform, With<FlyCamera>>,
    mut rig: Query<(&mut Transform, &mut Visibility)>,
    mut cameras: Query<(&mut Camera, &mut Projection)>,
    mut materials: ResMut<Assets<LevelMaterial>>,
) {
    let Some(mut arrow) = arrow else { return };
    let wanted = arrow.wanted.take();
    if let Ok((mut camera, mut projection)) = cameras.get_mut(arrow.camera) {
        if camera.is_active != wanted.is_some() {
            camera.is_active = wanted.is_some();
        }
        if let (Some((tip, _)), Projection::Custom(custom)) = (wanted, projection.as_mut())
            && let Some(window) = custom.get_mut::<Window>()
            && window.tip != tip
        {
            window.tip = tip;
        }
    }
    let Ok((mut transform, mut visibility)) = rig.get_mut(arrow.root) else { return };
    let Some((tip, turn)) = wanted else {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
        return;
    };
    *transform = pose(tip, turn);
    if *visibility != Visibility::Visible {
        *visibility = Visibility::Visible;
    }
    // The game's arrow is turned with the scene's camera, so the level's
    // light falls on it from where that camera sees the light: both
    // cameras' own spaces are the same space.
    let toward = scene.single().map_or(Quat::IDENTITY, |t| t.rotation().inverse()) * light.dir.truncate();
    let dir = toward.extend(light.dir.w);
    for handle in &arrow.materials {
        let stale = materials.get(handle).is_some_and(|m| m.light_dir != dir || m.light_color != light.color);
        if stale && let Some(m) = materials.get_mut(handle) {
            m.light_dir = dir;
            m.light_color = light.color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a point in the arrow camera's space lands on the 2D screen.
    fn on_screen(window: &Window, p: Vec3) -> Vec2 {
        let clip = window.get_clip_from_view() * p.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let left_top = Vec2::new(window.tip.x - LEFT, window.tip.y - HALF_HEIGHT);
        left_top + Vec2::new((ndc.x + 1.0) * 0.5 * (LEFT + RIGHT), (1.0 - ndc.y) * 0.5 * (2.0 * HALF_HEIGHT))
    }

    #[test]
    fn the_tip_is_drawn_where_the_menu_puts_it() {
        for tip in [Vec2::new(112.0, 176.0), Vec2::new(400.0, 40.0), Vec2::new(256.0, 192.0)] {
            let window = Window { tip };
            let at = on_screen(&window, pose(tip, 0.0).translation);
            assert!((at - tip).length() < 1e-3, "{tip} drawn at {at}");
        }
    }

    #[test]
    fn it_points_right_and_is_the_games_size() {
        let tip = Vec2::new(112.0, 176.0);
        let window = Window { tip };
        let root = pose(tip, 0.0);
        // A point a model unit back along its length (its tail's way) is
        // to the left on the screen: 1 × 0.05 / 1.1 of the focal length.
        let tail = on_screen(&window, root.transform_point(Vec3::new(-1.0, 0.0, 0.0)));
        let step = SCALE / DEPTH * Window::focal();
        assert!((tail - (tip - Vec2::new(step, 0.0))).length() < 1e-3, "{tail}");
        // The model's +Y is up the screen, its +Z away from the viewer.
        let above = on_screen(&window, root.transform_point(Vec3::new(0.0, 1.0, 0.0)));
        assert!((above - (tip - Vec2::new(0.0, step))).length() < 1e-3, "{above}");
        assert!(root.transform_point(Vec3::Z).z < root.translation.z);
    }

    #[test]
    fn it_rolls_about_its_length_top_toward_the_viewer() {
        // The game's turn: Y' = Y cos − Z sin, Z' = Z cos + Y sin — a
        // quarter turn brings the model's top edge to face the viewer.
        let root = pose(Vec2::new(112.0, 176.0), std::f32::consts::FRAC_PI_2);
        let top = root.transform_point(Vec3::Y) - root.translation;
        assert!(top.z > 0.0 && top.y.abs() < 1e-6 && top.x.abs() < 1e-6, "{top}");
        // Half a turn puts its back where its front was.
        let over = pose(Vec2::new(112.0, 176.0), std::f32::consts::PI);
        let back = over.transform_point(Vec3::Z) - over.translation;
        assert!(back.z > 0.0, "{back}");
    }
}
