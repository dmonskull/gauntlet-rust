//! Drives the Bevy camera from the game's play camera (`camera_rig.rs`):
//! built from the level's camera points and its `WDATA` camera record when
//! the level loads, ticked at 30 Hz after the player, and interpolated
//! between ticks. `C` toggles the free-fly camera instead.
//!
//! Online each hero has a camera of its own as well, following it alone
//! as a player's camera does: each machine draws its own hero's (another
//! hero's while its own is out), each hero's stick turns by its own, and
//! the game's on-screen tests take every hero's. Cuts, the level-start
//! shot and a boss level's camera stay everyone's.

use gdl_formats::detmath::Det;
use std::collections::HashMap;

use bevy::prelude::*;

use crate::boss_camera::{self, BossCam, Hero};
use crate::critters::BossWatch;
use crate::mechanics::Mechanics;
use gdl_formats::population::LocatorKind;
use gdl_formats::{BossCamera, LevelCamera, LevelLight, WorldData};

use crate::camera::{FlyCamera, FreeLook};
use crate::camera_rig::{CameraPoint, CameraRig, RigSave};
use crate::level::LoadedGame;
use crate::level_material::SceneLight;
use crate::player::{Player, PlayerTick};
use crate::party::{MAX_PLAYERS, Party};
use crate::player_state::DEFAULT_HEAD;
use crate::population::LevelPopulation;
use crate::world::LevelGround;

/// The game's field of view: 60° across a 4:3 picture, i.e. this much
/// vertically. Kept vertical so wider screens see more to the sides.
fn vertical_fov() -> f32 {
    2.0 * (0.75 * 30f32.to_radians().dtan()).datan()
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
            .add_systems(FixedUpdate, tick.after(PlayerTick).after(crate::critters::watch_boss))
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

/// Each level's camera record, light and boss camera, by lower-case level
/// folder (`levela1`).
#[derive(Resource, Default)]
struct LevelCameras(HashMap<String, (LevelCamera, LevelLight, Option<BossCamera>)>);

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
    /// On a boss level: the boss camera, which drives the view instead of
    /// the play camera once the boss is made (`boss_camera.rs`), and the
    /// entry's starting camera point it opens from.
    boss: Option<BossCam>,
    boss_active: bool,
    start_point: Option<CameraPoint>,
    /// The level-start shot is still on (not a glide back from a cut).
    opening: bool,
    /// Online: each hero's own camera, by slot.
    own: [Option<OwnCamera>; MAX_PLAYERS],
    /// Online: the heroes standing (whose cameras the on-screen tests take).
    standing: [bool; MAX_PLAYERS],
    /// Online: whose camera this machine's screen shows (its own hero's, a
    /// teammate's while its own is out). The screen's, not the game's.
    pub watching: Option<usize>,
}

/// The play camera as a sync point carries it (`resync.rs`): the game's
/// own camera as the host has it — its rig, the opening shot, a cut, a
/// shake, a boss level's camera — and each hero's own rig as its player's
/// machine does (a hero's camera never jumps on the screen it's drawn
/// on). The game's camera is every machine's: the heroes' sticks turn by
/// it while it has the view, monsters wake by what it sees, and the
/// level's animated objects wait on its cuts.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CameraSave {
    shared: Option<RigAt>,
    own: Vec<(usize, RigAt)>,
    standing: [bool; MAX_PLAYERS],
    intro: Option<Intro>,
    cut: Option<Cut>,
    shake: Option<Shake>,
    shake_offset: ([f32; 3], [f32; 3]),
    boss: Option<crate::boss_camera::BossCamSave>,
    boss_active: bool,
    opening: bool,
}

/// A rig, and its eye and target before the latest tick.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RigAt {
    rig: RigSave,
    previous: ([f32; 3], [f32; 3]),
}

impl CameraSave {
    /// Each of `slots`' own cameras as `theirs` has it (those heroes'
    /// machine's).
    pub(crate) fn keep_own(&mut self, theirs: &CameraSave, slots: &[u8]) {
        for (slot, rig) in &theirs.own {
            if !slots.contains(&(*slot as u8)) {
                continue;
            }
            match self.own.iter_mut().find(|(s, _)| s == slot) {
                Some(mine) => mine.1 = rig.clone(),
                None => self.own.push((*slot, rig.clone())),
            }
        }
    }
}

pub(crate) fn save_synced(world: &World) -> Option<CameraSave> {
    let camera = world.get_resource::<PlayCamera>()?;
    let own = |(slot, o): (usize, &Option<OwnCamera>)| o.as_ref().map(|o| (slot, RigAt { rig: o.rig.save(), previous: o.previous }));
    Some(CameraSave {
        shared: Some(RigAt { rig: camera.rig.save(), previous: camera.previous }),
        own: camera.own.iter().enumerate().filter_map(own).collect(),
        standing: camera.standing,
        intro: camera.intro,
        cut: camera.cut,
        shake: camera.shake,
        shake_offset: camera.shake_offset,
        boss: camera.boss.as_ref().map(BossCam::save),
        boss_active: camera.boss_active,
        opening: camera.opening,
    })
}

pub(crate) fn load_synced(world: &mut World, save: &CameraSave) {
    let Some(mut camera) = world.get_resource_mut::<PlayCamera>() else { return };
    if let Some(shared) = &save.shared {
        camera.rig.load(&shared.rig);
        camera.previous = shared.previous;
    }
    for (slot, saved) in &save.own {
        if let Some(own) = camera.own.get_mut(*slot).and_then(Option::as_mut) {
            own.rig.load(&saved.rig);
            own.previous = saved.previous;
        }
    }
    camera.standing = save.standing;
    (camera.intro, camera.cut, camera.opening) = (save.intro, save.cut, save.opening);
    (camera.shake, camera.shake_offset) = (save.shake, save.shake_offset);
    if let (Some(boss), Some(saved)) = (camera.boss.as_mut(), &save.boss) {
        boss.load(saved);
    }
    camera.boss_active = save.boss_active;
}

impl PlayCamera {
    /// What the machines compare of the game's camera online
    /// (`online.rs`): where it looks from and at, and what has the view.
    pub fn sync_hash(&self) -> u64 {
        use gdl_formats::detmath::sync_bits;
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let (eye, target) = self.view();
        (eye.map(sync_bits), target.map(sync_bits), sync_bits(self.yaw())).hash(&mut h);
        self.cut.map(|c| (sync_bits(c.delay), sync_bits(c.fields_left), sync_bits(c.extra))).hash(&mut h);
        self.intro.map(|i| sync_bits(i.fields_left)).hash(&mut h);
        (self.boss_active, self.opening, self.standing).hash(&mut h);
        for own in self.own.iter().flatten() {
            (own.rig.eye().map(sync_bits), own.rig.target.map(sync_bits)).hash(&mut h);
        }
        h.finish()
    }
}

/// A hero's own camera (online): a rig following it alone, and its eye and
/// target before the latest tick.
#[derive(Clone)]
struct OwnCamera {
    rig: CameraRig,
    previous: ([f32; 3], [f32; 3]),
}

/// Shakes the camera (`docs/camera.md` "Shakes"): `what` 0 moves the
/// target, 1 the eye, 2 both, round a circle of radius `amplitude` that
/// turns 0.663 rad a field, after `delay` fields, for `fields`; a shake
/// with a lower `priority` doesn't replace one still going.
#[derive(Message, Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Shake {
    pub amplitude: f32,
    pub what: u8,
    pub delay: f32,
    pub fields: f32,
    pub priority: i32,
}

/// Radians the shake turns per field.
const SHAKE_TURN: f32 = 0.663_225_1;

/// Shows a camera point (`docs/camera.md` "Trigger cuts"): the locator
/// index into the level's locators, and the node the trigger moved. A
/// trigger's cut waits 30 fields and holds by the point's byte; the
/// game's scenes give their own hold and delay (`docs/camera.md`).
#[derive(Message, Clone, Copy, Debug)]
pub struct StartCut {
    pub locator: usize,
    pub node: Option<usize>,
    /// Fields it holds (none: 6 × the point's byte, or 40).
    pub hold: Option<f32>,
    /// Fields before the view changes.
    pub delay: f32,
    /// Fields it holds after that, which the tower's scenes wait on.
    pub extra: f32,
}

impl StartCut {
    /// A trigger's cut to its camera point.
    pub fn trigger(locator: usize, node: Option<usize>) -> Self {
        Self { locator, node, hold: None, delay: CUT_DELAY, extra: 0.0 }
    }
}

/// A camera cut: after 30 fields the view jumps to the camera point for its
/// time (40 fields, or 6 × the point's byte), held while the moved node
/// still moves, with black bars top and bottom; then play resumes, gliding
/// back like the level start. The hero can't be hurt meanwhile.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
struct Cut {
    delay: f32,
    eye: [f32; 3],
    target: [f32; 3],
    fields_left: f32,
    /// Counted once `fields_left` is spent.
    extra: f32,
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
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
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
    /// Whether the one camera has everyone's view: a cut, the level-start
    /// shot or the glide back, a boss level's camera.
    fn shared(&self) -> bool {
        self.cut.is_some_and(|c| c.delay <= 0.0) || self.intro.is_some() || self.boss_active
    }

    /// Whether the heroes have cameras of their own (online, the host's
    /// choice).
    pub fn per_hero(&self) -> bool {
        self.own.iter().any(Option::is_some)
    }

    fn own_of(&self, slot: usize) -> Option<&OwnCamera> {
        self.own.get(slot).and_then(Option::as_ref)
    }

    fn shaken(&self, (eye, target): ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
        let (de, dt) = self.shake_offset;
        (std::array::from_fn(|i| eye[i] + de[i]), std::array::from_fn(|i| target[i] + dt[i]))
    }

    /// The way a hero's stick turns: online its own camera's while that
    /// has the view, the game's camera's otherwise.
    pub fn yaw_of(&self, slot: usize) -> f32 {
        match self.own_of(slot) {
            Some(o) if !self.shared() => o.rig.yaw,
            _ => self.yaw(),
        }
    }

    /// What this machine's screen shows, and before the latest tick: online
    /// the watched hero's own camera while that has the view.
    pub fn screen_view(&self) -> ([f32; 3], [f32; 3]) {
        match self.watching.and_then(|s| self.own_of(s)) {
            Some(o) if !self.shared() => self.shaken((o.rig.eye(), o.rig.target)),
            _ => self.view(),
        }
    }

    fn screen_previous(&self) -> ([f32; 3], [f32; 3]) {
        match self.watching.and_then(|s| self.own_of(s)) {
            Some(o) if !self.shared() => o.previous,
            _ => self.previous,
        }
    }

    /// Personal render view; these presentation choices never alter the
    /// cameras used for monster activation or the shared boss bounds.
    pub fn view_for(&self, slot: usize, fraction: f32) -> ([f32; 3], [f32; 3]) {
        let (now, before) = match self.own_of(slot) {
            Some(o) if !self.shared() => (self.shaken((o.rig.eye(), o.rig.target)), o.previous),
            _ => (self.view(), self.previous),
        };
        let lerp = |a: [f32; 3], b: [f32; 3]| Vec3::from(a).lerp(Vec3::from(b), fraction).to_array();
        (lerp(before.0, now.0), lerp(before.1, now.1))
    }

    /// The way the screen's camera faces (the positional sounds' ear).
    pub fn screen_yaw(&self) -> f32 {
        self.watching.map_or_else(|| self.yaw(), |s| self.yaw_of(s))
    }

    /// The eyes and targets the game's on-screen tests look through: the
    /// play camera's; online each standing hero's own (every hero's with
    /// none standing) while they have the view.
    pub fn game_views(&self) -> Vec<([f32; 3], [f32; 3])> {
        let own: Vec<(usize, &OwnCamera)> = self.own.iter().enumerate().filter_map(|(s, o)| Some((s, o.as_ref()?))).collect();
        if own.is_empty() || self.shared() {
            return vec![(self.rig.eye(), self.rig.target)];
        }
        let any_standing = own.iter().any(|(s, _)| self.standing[*s]);
        own.iter().filter(|(s, _)| self.standing[*s] || !any_standing).map(|(_, o)| (o.rig.eye(), o.rig.target)).collect()
    }

    /// Eye and target to draw from this tick (the positional sounds' ear
    /// is its target, `audio.rs`).
    pub fn view(&self) -> ([f32; 3], [f32; 3]) {
        if let Some(c) = self.cut.filter(|c| c.delay <= 0.0) {
            return (c.eye, c.target);
        }
        let (eye, target) = match self.intro {
            Some(i) => (i.eye, i.target),
            None => self.play_view(),
        };
        let (de, dt) = self.shake_offset;
        (std::array::from_fn(|i| eye[i] + de[i]), std::array::from_fn(|i| target[i] + dt[i]))
    }

    /// The play camera's own view: the boss camera's on a boss level.
    fn play_view(&self) -> ([f32; 3], [f32; 3]) {
        match self.boss.as_ref().filter(|_| self.boss_active) {
            Some(b) => (b.eye(), b.target),
            None => (self.rig.eye(), self.rig.target),
        }
    }

    /// The way the camera faces, which the stick is turned by: the boss
    /// camera's on a boss level, the play camera's otherwise.
    pub fn yaw(&self) -> f32 {
        match self.boss.as_ref().filter(|_| self.boss_active) {
            Some(b) => b.yaw,
            None => self.rig.yaw,
        }
    }

    /// A hero's step, kept inside the boss camera's view on a boss level.
    pub fn keep_in_view(&self, feet: [f32; 3], centre: [f32; 3], step: [f32; 3]) -> [f32; 3] {
        match self.boss.as_ref().filter(|_| self.boss_active && self.cut.is_none()) {
            Some(b) => b.keep_in_view(feet, centre, step),
            None => step,
        }
    }

    /// Whether a camera cut is showing (the hero can't be hurt then).
    pub fn in_cut(&self) -> bool {
        self.cut.is_some()
    }

    /// Whether plain play has the camera: no cut, no opening shot or
    /// glide back.
    pub fn settled(&self) -> bool {
        self.cut.is_none() && self.intro.is_none()
    }

    /// Whether a fixed shot has the view: a cut showing its point, or the
    /// level-start shot's hold (not the glide back, which a personal view
    /// skips).
    pub fn showing_shot(&self) -> bool {
        self.cut.is_some_and(|c| c.delay <= 0.0) || self.intro.is_some_and(|i| i.fields_left >= 2.0)
    }

    /// Whether the level-start shot is still showing.
    pub fn opening(&self) -> bool {
        self.opening
    }

    /// The cut's hold and extra fields left (none without a cut).
    pub fn cut_counts(&self) -> Option<(f32, f32)> {
        self.cut.map(|c| (c.fields_left, c.extra))
    }

    /// Ends the cut: it's over on the next tick (unless another starts).
    pub fn end_cut(&mut self) {
        if let Some(c) = self.cut.as_mut() {
            c.fields_left = 0.0;
            c.extra = 0.0;
            c.node = None;
        }
    }

}

/// The starting camera for the entry the heroes arrive at: the kind-1
/// locator with its index (entry 0's when there's none), looking along its
/// yaw and pitch at the players' distance.
fn intro_shot(population: &gdl_formats::Population, entry: i16, focus: [f32; 3]) -> Option<Intro> {
    let (eye, target) = locator_view(starting_locator(population, entry)?, focus);
    Some(Intro { eye, target, fields_left: INTRO_FIELDS })
}

/// The entry's starting camera point: the kind-1 locator with its index,
/// else entry 0's.
fn starting_locator(population: &gdl_formats::Population, entry: i16) -> Option<&gdl_formats::population::Locator> {
    let starting = |index: i16| population.locators.iter().find(|l| l.kind == LocatorKind::Transmitter(1) && l.index == index);
    starting(entry).or_else(|| starting(0))
}

/// Eye and target of a camera point: looking along its yaw and pitch, at
/// `focus`'s distance.
fn locator_view(l: &gdl_formats::population::Locator, focus: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (yaw, pitch) = (l.rotation[1], -l.rotation[0]);
    let dir = Vec3::new(yaw.dsin() * pitch.dcos(), pitch.dsin(), yaw.dcos() * pitch.dcos());
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
                    let record = (world.cameras[level.camera].clone(), level.light, level.boss_camera);
                    cameras.0.insert(level.folder().to_ascii_lowercase(), record);
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
    party: Res<Party>,
    mut scene_light: ResMut<SceneLight>,
    (old, resume): (Option<Res<PlayCamera>>, Res<crate::player::Resume>),
) {
    let Some(ground) = ground else { return };
    // Back from Manage Character: no opening shot, the camera on the heroes
    // where they stood.
    let resumed = resume.on(&population.level).and_then(|spots| spots.iter().flatten().next().map(|(at, _)| *at));
    let records = cameras.and_then(|c| c.0.get(&population.level.to_ascii_lowercase()).cloned());
    // The level's light lights everything that isn't prelit.
    *scene_light = records.as_ref().map_or_else(SceneLight::default, |(_, light, _)| SceneLight::new(light));
    let boss_record = records.as_ref().and_then(|(_, _, b)| *b);
    let record = records
        .map(|(c, _, _)| c)
        .unwrap_or(LevelCamera { mode: 0, pitch_limit: 0.35, bounds: None, near: 24.0, far: 32.0 });
    // Only plain camera points are picked by distance; starting, intro and
    // trigger cameras are chosen by events.
    let points: Vec<CameraPoint> = population
        .population
        .locators
        .iter()
        .filter(|l| l.kind == LocatorKind::Transmitter(2))
        .map(|l| CameraPoint { position: l.position, yaw: l.rotation[1], pitch: l.rotation[0], param: l.param })
        .collect();
    let [lo, hi] = ground.0.bounds;
    let bounds = record.target_bounds(lo, hi);
    let feet = resumed.or_else(|| population.player_start().map(|s| s.position)).unwrap_or([0.0; 3]);
    let head = party.states().next().map_or(DEFAULT_HEAD, |(_, s)| s.head_height);
    let rig = CameraRig::new(points, bounds, record.near, top_point(feet, head), feet).with_far(record.far, record.pitch_limit);
    // A boss level with a boss camera opens with it instead of the
    // starting shot.
    let has_boss = population.population.locators.iter().any(|l| l.kind == LocatorKind::Boss);
    let boss = boss_record.filter(|_| has_boss).map(BossCam::new);
    let start_point = starting_locator(&population.population, population.entry)
        .map(|l| CameraPoint { position: l.position, yaw: l.rotation[1], pitch: l.rotation[0], param: l.param });
    let intro = if boss.is_some() || resumed.is_some() { None } else { intro_shot(&population.population, population.entry, feet) };
    let previous = intro.map_or((rig.eye(), rig.target), |i| (i.eye, i.target));
    // Online each hero's own camera comes with the host's choice (`tick`).
    let own = std::array::from_fn(|_| None);
    commands.insert_resource(PlayCamera {
        rig,
        previous,
        opening: intro.is_some(),
        intro,
        cut: None,
        shake: None,
        shake_offset: ([0.0; 3], [0.0; 3]),
        boss,
        boss_active: false,
        start_point,
        own,
        standing: [true; MAX_PLAYERS],
        watching: old.and_then(|c| c.watching),
    });
}

/// A hero's top point, which the camera looks at: the class's head height
/// above the feet (player `+0x54`, from `PDAT +0x50`; `docs/camera.md`).
fn top_point(feet: [f32; 3], head: f32) -> [f32; 3] {
    [feet[0], feet[1] + head, feet[2]]
}

/// The centre of the box round some points (`docs/camera.md`, "Update"):
/// what the camera follows with several players.
fn box_centre(points: impl Iterator<Item = [f32; 3]>) -> Option<[f32; 3]> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut any = false;
    for p in points {
        any = true;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    any.then(|| std::array::from_fn(|k| (lo[k] + hi[k]) / 2.0))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tick(
    camera: Option<ResMut<PlayCamera>>,
    players: Query<&Player>,
    party: Res<Party>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut cuts: MessageReader<StartCut>,
    mut shakes: MessageReader<Shake>,
    population: Option<Res<LevelPopulation>>,
    mechanics: Option<Res<Mechanics>>,
    (watch, fixed): (Res<BossWatch>, Res<Time<Fixed>>),
    (lock, inputs): (Res<crate::online::Lockstep>, Res<crate::party::Inputs>),
) {
    let Some(mut camera) = camera else { return };
    // Online the host's choice (riding with its controls, slot 0's): each
    // player's own camera, made from the game's where it is, or the game's
    // one co-op camera for everyone.
    if lock.on {
        let own = inputs.slots[0].held & crate::party::SlotInput::OWN_CAMERAS != 0;
        if own && !camera.per_hero() {
            let rig = camera.rig.clone();
            for slot in 0..MAX_PLAYERS {
                camera.own[slot] = party.get(slot).map(|_| OwnCamera { rig: rig.clone(), previous: (rig.eye(), rig.target) });
            }
        } else if !own && camera.per_hero() {
            camera.own = std::array::from_fn(|_| None);
        }
    }
    // The heroes it follows: those in play, else (all dead) every one.
    let heroes: Vec<Hero> = {
        let of = |p: &Player| {
            let s = party.state(p.slot);
            let feet = p.mover.position;
            Hero { feet, top: top_point(feet, s.map_or(DEFAULT_HEAD, |s| s.head_height)), half_height: s.map_or(2.5, |s| s.half_height) }
        };
        let living: Vec<Hero> = players.iter().filter(|p| party.state(p.slot).is_some_and(|s| s.alive)).map(of).collect();
        if living.is_empty() { players.iter().map(of).collect() } else { living }
    };
    let (Some(top), Some(feet)) = (box_centre(heroes.iter().map(|h| h.top)), box_centre(heroes.iter().map(|h| h.feet))) else {
        return;
    };
    let camera = &mut *camera;
    camera.previous = camera.view();
    // On a boss level the boss camera runs once the boss is made; the
    // play camera otherwise.
    let PlayCamera { rig, boss, boss_active, start_point, .. } = camera;
    match (boss.as_mut(), watch.spot) {
        (Some(boss), Some(spot)) => {
            let scene = boss_camera::Scene {
                heroes: &heroes,
                boss: watch.boss,
                spot,
                awake: watch.awake,
                ending: watch.ending,
                key: watch.key,
                wizard: watch.wizard,
                bounds: rig.bounds(),
                points: rig.points(),
                start: *start_point,
            };
            boss.tick(&scene, fixed.delta_secs());
            trace!(
                "boss camera: yaw {:.1}° pitch {:.1}° distance {:.1} margin {:.2} target {:?}",
                boss.yaw.to_degrees(),
                boss.pitch.to_degrees(),
                boss.distance,
                boss.margin,
                boss.target
            );
            *boss_active = true;
        }
        _ => {
            let framed: Vec<crate::camera_rig::Framed> = heroes.iter().map(|h| (h.top, h.feet)).collect();
            rig.tick(top, feet, &framed);
            *boss_active = false;
        }
    }
    // Online: each hero's own camera follows it alone.
    for p in &players {
        let Some(own) = camera.own.get(p.slot).and_then(Option::as_ref).map(|o| (o.rig.eye(), o.rig.target)) else { continue };
        let previous = camera.shaken(own);
        let s = party.state(p.slot);
        camera.standing[p.slot] = s.is_some_and(|s| s.alive);
        let feet = p.mover.position;
        let top = top_point(feet, s.map_or(DEFAULT_HEAD, |s| s.head_height));
        if let Some(o) = camera.own[p.slot].as_mut() {
            o.previous = previous;
            o.rig.tick(top, feet, &[(top, feet)]);
        }
    }
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
            let d = [s.amplitude * angle.dsin(), 0.0, s.amplitude * angle.dcos()];
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
        let (eye, target) = locator_view(l, feet);
        let fields = cut.hold.unwrap_or(if l.param == 0 { CUT_FIELDS } else { CUT_FIELDS_PER_STEP * f32::from(l.param) });
        camera.cut = Some(Cut { delay: cut.delay, eye, target, fields_left: fields, extra: cut.extra, node: cut.node });
    }
    if let Some(cut) = camera.cut.as_mut() {
        if cut.delay > 0.0 {
            cut.delay -= FIELDS_PER_TICK;
            return;
        }
        if cut.fields_left > 0.0 {
            cut.fields_left -= FIELDS_PER_TICK;
        } else {
            cut.extra -= FIELDS_PER_TICK;
        }
        let moving = cut.node.is_some_and(|n| mechanics.as_ref().is_some_and(|m| m.node_moving(n)));
        if cut.fields_left > 0.0 || cut.extra > 0.0 || moving {
            return;
        }
        // Back to play, gliding from the cut's view.
        let (eye, target) = (cut.eye, cut.target);
        camera.cut = None;
        camera.intro = Some(Intro { eye, target, fields_left: 1.0 });
    }
    let (eye, target) = camera.play_view();
    let Some(intro) = camera.intro.as_mut() else { return };
    if intro.fields_left >= 2.0 {
        intro.fields_left -= FIELDS_PER_TICK;
        // Online any player's press, as every machine sees it.
        let pressed = if lock.on {
            lock.pressed(&inputs, !crate::party::SlotInput::SETTINGS)
        } else {
            keys.get_just_pressed().next().is_some() || pads.iter().any(|p| p.get_just_pressed().next().is_some())
        };
        if intro.fields_left < INTRO_SKIPPABLE && pressed {
            intro.fields_left = 1.0;
        }
        return;
    }
    let glide = |from: &mut [f32; 3], to: [f32; 3]| -> f32 {
        for i in 0..3 {
            from[i] += (to[i] - from[i]) * INTRO_BLEND;
        }
        Vec3::from(*from).distance(Vec3::from(to))
    };
    let (d1, d2) = (glide(&mut intro.eye, eye), glide(&mut intro.target, target));
    if d1 < INTRO_DONE && d2 < INTRO_DONE {
        camera.intro = None;
        camera.opening = false;
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut free_look: ResMut<FreeLook>,
    mut camera: Query<(&Transform, &mut FlyCamera)>,
) {
    if crate::dev_keys() && keys.just_pressed(KeyCode::KeyC) {
        free_look.0 = !free_look.0;
        if let Ok((transform, mut fly)) = camera.single_mut() {
            fly.sync(transform);
        }
    }
}

pub(crate) fn place(
    fixed: Res<Time<Fixed>>,
    free_look: Res<FreeLook>,
    play: Res<PlayCamera>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
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
    let (now_eye, now_target) = play.screen_view();
    let (was_eye, was_target) = play.screen_previous();
    let eye = Vec3::from(was_eye).lerp(Vec3::from(now_eye), t);
    let target = Vec3::from(was_target).lerp(Vec3::from(now_target), t);
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
