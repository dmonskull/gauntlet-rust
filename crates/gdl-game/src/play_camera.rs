//! Drives the Bevy camera from the game's play camera (`camera_rig.rs`):
//! built from the level's camera points and its `WDATA` camera record when
//! the level loads, ticked at 30 Hz after the player, and interpolated
//! between ticks. `C` toggles the free-fly camera instead.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::mechanics::Mechanics;
use gdl_formats::population::LocatorKind;
use gdl_formats::{LevelCamera, LevelLight, WorldData};

use crate::camera::{FlyCamera, FreeLook};
use crate::camera_rig::{CameraPoint, CameraRig};
use crate::level::LoadedGame;
use crate::level_material::SceneLight;
use crate::player::{Player, PlayerTick};
use crate::population::LevelPopulation;
use crate::world::LevelGround;

/// The game's field of view: 60° across a 4:3 picture, i.e. this much
/// vertically. Kept vertical so wider screens see more to the sides.
fn vertical_fov() -> f32 {
    2.0 * (0.75 * 30f32.to_radians().tan()).atan()
}

pub struct PlayCameraPlugin;

impl Plugin for PlayCameraPlugin {
    fn build(&self, app: &mut App) {
        // GDL_FREE_CAMERA=1 starts in the free camera (level overviews).
        let free = std::env::var("GDL_FREE_CAMERA").is_ok_and(|v| !v.is_empty() && v != "0");
        app.insert_resource(FreeLook(free))
            .add_message::<StartCut>()
            .add_message::<Shake>()
            .add_systems(Startup, (load_level_cameras, set_fov, spawn_bars))
            .add_systems(Update, show_bars)
            .add_systems(FixedUpdate, tick.after(PlayerTick))
            .add_systems(
                Update,
                (
                    start.run_if(resource_exists_and_changed::<LevelPopulation>),
                    toggle,
                    place.run_if(resource_exists::<PlayCamera>),
                )
                    .chain(),
            );
    }
}

/// Each level's camera record and light, by lower-case level folder
/// (`levela1`).
#[derive(Resource, Default)]
struct LevelCameras(HashMap<String, (LevelCamera, LevelLight)>);

#[derive(Resource)]
pub struct PlayCamera {
    pub rig: CameraRig,
    /// Eye and target before the latest tick, for interpolation.
    previous: ([f32; 3], [f32; 3]),
    /// The level-start shot, while it lasts.
    intro: Option<Intro>,
    /// A trigger's camera cut, while it lasts.
    cut: Option<Cut>,
    /// The shake under way, and this tick's eye and target offsets.
    shake: Option<Shake>,
    shake_offset: ([f32; 3], [f32; 3]),
}

/// Shakes the camera (`docs/camera.md` "Shakes"): `what` 0 moves the
/// target, 1 the eye, 2 both, round a circle of radius `amplitude` that
/// turns 0.663 rad a field, after `delay` fields, for `fields`; a shake
/// with a lower `priority` doesn't replace one still going.
#[derive(Message, Clone, Copy, Debug)]
pub struct Shake {
    pub amplitude: f32,
    pub what: u8,
    pub delay: f32,
    pub fields: f32,
    pub priority: i32,
}

/// Radians the shake turns per field.
const SHAKE_TURN: f32 = 0.663_225_1;

/// Shows a trigger's camera point (`docs/camera.md` "Trigger cuts"): the
/// locator index into the level's locators, and the node the trigger moved.
#[derive(Message, Clone, Copy, Debug)]
pub struct StartCut {
    pub locator: usize,
    pub node: Option<usize>,
}

/// A camera cut: after 30 fields the view jumps to the camera point for its
/// time (40 fields, or 6 × the point's byte), held while the moved node
/// still moves, with black bars top and bottom; then play resumes, gliding
/// back like the level start. The hero can't be hurt meanwhile.
#[derive(Clone, Copy, Debug)]
struct Cut {
    delay: f32,
    eye: [f32; 3],
    target: [f32; 3],
    fields_left: f32,
    node: Option<usize>,
}

const CUT_DELAY: f32 = 30.0;
const CUT_FIELDS: f32 = 40.0;
const CUT_FIELDS_PER_STEP: f32 = 6.0;
/// The bars' height, of the 384-line screen.
const CUT_BAR: f32 = 80.0 / 384.0;

/// The level-start shot (`docs/camera.md` "Level start"): the entry's
/// starting camera holds for 91 video fields (any button skips it once
/// fewer than 45 remain), then eye and target glide a tenth of the way to
/// the play camera each tick until both are within 0.3 of it.
#[derive(Clone, Copy, Debug)]
struct Intro {
    eye: [f32; 3],
    target: [f32; 3],
    fields_left: f32,
}

const INTRO_FIELDS: f32 = 91.0;
const INTRO_SKIPPABLE: f32 = 45.0;
const INTRO_BLEND: f32 = 0.1;
const INTRO_DONE: f32 = 0.3;
const FIELDS_PER_TICK: f32 = 2.0;

impl PlayCamera {
    /// Eye and target to draw from this tick.
    fn view(&self) -> ([f32; 3], [f32; 3]) {
        if let Some(c) = self.cut.filter(|c| c.delay <= 0.0) {
            return (c.eye, c.target);
        }
        let (eye, target) = match self.intro {
            Some(i) => (i.eye, i.target),
            None => (self.rig.eye(), self.rig.target),
        };
        let (de, dt) = self.shake_offset;
        (std::array::from_fn(|i| eye[i] + de[i]), std::array::from_fn(|i| target[i] + dt[i]))
    }

    /// Whether a camera cut is showing (the hero can't be hurt then).
    pub fn in_cut(&self) -> bool {
        self.cut.is_some()
    }

}

/// The starting camera for the entry the heroes arrive at: the kind-1
/// locator with its index (entry 0's when there's none), looking along its
/// yaw and pitch at the players' distance.
fn intro_shot(population: &gdl_formats::Population, entry: i16, focus: [f32; 3]) -> Option<Intro> {
    let starting = |index: i16| population.locators.iter().find(|l| l.kind == LocatorKind::Transmitter(1) && l.index == index);
    let l = starting(entry).or_else(|| starting(0))?;
    let (eye, target) = locator_view(l, focus);
    Some(Intro { eye, target, fields_left: INTRO_FIELDS })
}

/// Eye and target of a camera point: looking along its yaw and pitch, at
/// `focus`'s distance.
fn locator_view(l: &gdl_formats::population::Locator, focus: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (yaw, pitch) = (l.rotation[1], -l.rotation[0]);
    let dir = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos());
    let eye = Vec3::from(l.position);
    let target = eye + dir * eye.distance(Vec3::from(focus)).max(1.0);
    (eye.to_array(), target.to_array())
}

fn load_level_cameras(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let mut cameras = LevelCameras::default();
    let wads: Vec<String> = game
        .install
        .files()
        .iter()
        .filter(|f| {
            let f = f.to_ascii_uppercase();
            f.starts_with("WDATA/") && f.ends_with(".WAD")
        })
        .cloned()
        .collect();
    for path in wads {
        match game.install.read(&path).map_err(|e| e.to_string()).and_then(|b| WorldData::parse(&b).map_err(|e| e.to_string())) {
            Ok(world) => {
                for level in &world.levels {
                    cameras
                        .0
                        .insert(level.folder().to_ascii_lowercase(), (world.cameras[level.camera].clone(), level.light));
                }
            }
            Err(e) => warn!("{path}: {e}"),
        }
    }
    commands.insert_resource(cameras);
}

fn set_fov(mut projections: Query<&mut Projection, With<Camera3d>>) {
    for mut p in &mut projections {
        match p.as_mut() {
            Projection::Perspective(p) => p.fov = vertical_fov(),
            Projection::Custom(c) => {
                if let Some(m) = c.get_mut::<crate::camera::MirroredPerspective>() {
                    m.0.fov = vertical_fov();
                }
            }
            Projection::Orthographic(_) => {}
        }
    }
}

fn start(
    mut commands: Commands,
    population: Res<LevelPopulation>,
    ground: Option<Res<LevelGround>>,
    cameras: Option<Res<LevelCameras>>,
    mut scene_light: ResMut<SceneLight>,
) {
    let Some(ground) = ground else { return };
    let records = cameras.and_then(|c| c.0.get(&population.level.to_ascii_lowercase()).cloned());
    // The level's light lights everything that isn't prelit.
    *scene_light = records.as_ref().map_or_else(SceneLight::default, |(_, light)| SceneLight::new(light));
    let record = records
        .map(|(c, _)| c)
        .unwrap_or(LevelCamera { mode: 0, pitch_limit: 0.35, bounds: None, near: 24.0, far: 32.0 });
    // Only plain camera points are picked by distance; starting, intro and
    // trigger cameras are chosen by events.
    let points: Vec<CameraPoint> = population
        .population
        .locators
        .iter()
        .filter(|l| l.kind == LocatorKind::Transmitter(2))
        .map(|l| CameraPoint { position: l.position, yaw: l.rotation[1], pitch: l.rotation[0] })
        .collect();
    let [lo, hi] = ground.0.bounds;
    let bounds = record.target_bounds(lo, hi);
    let focus = population.player_start().map_or([0.0; 3], |s| s.position);
    let rig = CameraRig::new(points, bounds, record.near, focus);
    let intro = intro_shot(&population.population, population.entry, focus);
    let previous = intro.map_or((rig.eye(), rig.target), |i| (i.eye, i.target));
    commands.insert_resource(PlayCamera { rig, previous, intro, cut: None, shake: None, shake_offset: ([0.0; 3], [0.0; 3]) });
}

#[allow(clippy::too_many_arguments)]
fn tick(
    camera: Option<ResMut<PlayCamera>>,
    players: Query<&Player>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut cuts: MessageReader<StartCut>,
    mut shakes: MessageReader<Shake>,
    population: Option<Res<LevelPopulation>>,
    mechanics: Option<Res<Mechanics>>,
) {
    let (Some(mut camera), Ok(player)) = (camera, players.single()) else { return };
    let camera = &mut *camera;
    camera.previous = camera.view();
    camera.rig.tick(player.mover.position);
    for s in shakes.read() {
        if camera.shake.is_none_or(|old| s.priority >= old.priority) {
            camera.shake = Some(*s);
        }
    }
    camera.shake_offset = ([0.0; 3], [0.0; 3]);
    if let Some(s) = camera.shake.as_mut() {
        s.fields -= FIELDS_PER_TICK;
        s.delay = (s.delay - FIELDS_PER_TICK).max(0.0);
        if s.fields < 0.0 {
            camera.shake = None;
        } else if s.delay <= 0.0 {
            let angle = SHAKE_TURN * s.fields;
            let d = [s.amplitude * angle.sin(), 0.0, s.amplitude * angle.cos()];
            let none = [0.0; 3];
            camera.shake_offset = match s.what {
                0 => (none, d),
                1 => (d, none),
                _ => (d, d),
            };
        }
    }
    for cut in cuts.read() {
        let Some(l) = population.as_ref().and_then(|p| p.population.locators.get(cut.locator)) else { continue };
        let (eye, target) = locator_view(l, player.mover.position);
        let fields = if l.param == 0 { CUT_FIELDS } else { CUT_FIELDS_PER_STEP * f32::from(l.param) };
        camera.cut = Some(Cut { delay: CUT_DELAY, eye, target, fields_left: fields, node: cut.node });
    }
    if let Some(cut) = camera.cut.as_mut() {
        if cut.delay > 0.0 {
            cut.delay -= FIELDS_PER_TICK;
            return;
        }
        cut.fields_left -= FIELDS_PER_TICK;
        let moving = cut.node.is_some_and(|n| mechanics.as_ref().is_some_and(|m| m.node_moving(n)));
        if cut.fields_left > 0.0 || moving {
            return;
        }
        // Back to play, gliding from the cut's view.
        let (eye, target) = (cut.eye, cut.target);
        camera.cut = None;
        camera.intro = Some(Intro { eye, target, fields_left: 1.0 });
    }
    let Some(intro) = camera.intro.as_mut() else { return };
    if intro.fields_left >= 2.0 {
        intro.fields_left -= FIELDS_PER_TICK;
        let pressed = keys.get_just_pressed().next().is_some() || pads.iter().any(|p| p.get_just_pressed().next().is_some());
        if intro.fields_left < INTRO_SKIPPABLE && pressed {
            intro.fields_left = 1.0;
        }
        return;
    }
    let (eye, target) = (camera.rig.eye(), camera.rig.target);
    let glide = |from: &mut [f32; 3], to: [f32; 3]| -> f32 {
        for i in 0..3 {
            from[i] += (to[i] - from[i]) * INTRO_BLEND;
        }
        Vec3::from(*from).distance(Vec3::from(to))
    };
    let (d1, d2) = (glide(&mut intro.eye, eye), glide(&mut intro.target, target));
    if d1 < INTRO_DONE && d2 < INTRO_DONE {
        camera.intro = None;
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut free_look: ResMut<FreeLook>,
    mut camera: Query<(&Transform, &mut FlyCamera)>,
) {
    if keys.just_pressed(KeyCode::KeyC) {
        free_look.0 = !free_look.0;
        if let Ok((transform, mut fly)) = camera.single_mut() {
            fly.sync(transform);
        }
    }
}

fn place(
    fixed: Res<Time<Fixed>>,
    free_look: Res<FreeLook>,
    play: Res<PlayCamera>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    if free_look.0 {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else { return };
    // `GDL_LOOK_AT="x,y,z[,distance[,yaw]]"` pins the camera on a point,
    // from above and behind it (−Z; `yaw` degrees turns the camera round
    // it) — testing: frame what's being checked.
    if let Some((at, distance, yaw)) = look_at() {
        let from = Quat::from_rotation_y(yaw.to_radians()) * Vec3::new(0.0, 0.6, -1.0).normalize();
        *transform = Transform::from_translation(at + from * distance).looking_at(at, Vec3::Y);
        return;
    }
    let t = fixed.overstep_fraction();
    let (now_eye, now_target) = play.view();
    let eye = Vec3::from(play.previous.0).lerp(Vec3::from(now_eye), t);
    let target = Vec3::from(play.previous.1).lerp(Vec3::from(now_target), t);
    *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}

/// The cut's black bars.
#[derive(Component)]
struct CutBar;

fn spawn_bars(mut commands: Commands) {
    for top in [true, false] {
        let mut node = Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0 * CUT_BAR), ..default() };
        if top {
            node.top = Val::Px(0.0);
        } else {
            node.bottom = Val::Px(0.0);
        }
        commands.spawn((CutBar, node, BackgroundColor(Color::BLACK), GlobalZIndex(50), Visibility::Hidden));
    }
}

fn show_bars(camera: Option<Res<PlayCamera>>, mut bars: Query<&mut Visibility, With<CutBar>>) {
    let on = camera.is_some_and(|c| c.cut.is_some_and(|cut| cut.delay <= 0.0));
    let want = if on { Visibility::Inherited } else { Visibility::Hidden };
    for mut v in &mut bars {
        v.set_if_neq(want);
    }
}

/// The `GDL_LOOK_AT` point, distance and yaw (degrees), if set.
fn look_at() -> Option<(Vec3, f32, f32)> {
    static LOOK: std::sync::OnceLock<Option<(Vec3, f32, f32)>> = std::sync::OnceLock::new();
    *LOOK.get_or_init(|| {
        let v: Vec<f32> = std::env::var("GDL_LOOK_AT").ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let at = Vec3::new(*v.first()?, *v.get(1)?, *v.get(2)?);
        Some((at, v.get(3).copied().unwrap_or(12.0), v.get(4).copied().unwrap_or(0.0)))
    })
}
