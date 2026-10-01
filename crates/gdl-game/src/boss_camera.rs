//! The boss levels' camera (`docs/critters.md`, "Boss camera"), as pure
//! logic. On a level with a boss and a `BCAM` record the game drives the
//! view with it instead of the play camera:
//!
//! - it starts at the entry's starting camera point, looking at the heroes;
//! - before the boss wakes it looks at the heroes' centre from the nearest
//!   camera point's angles (the starting point's while the opening lasts),
//!   backing off or closing in to keep them in frame;
//! - awake, it looks at the boss (the key, then the wizard at the end)
//!   along the way from the heroes furthest from it, stays off the boss's
//!   facing when the heroes are outside its cone, keeps the heroes and the
//!   boss in frame between the record's distances, and tilts with the
//!   distance.
//!
//! Yaw, distance and pitch ease toward their goals with the game's speed
//! limits and accelerations, and the target glides after its goal.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_6, PI};

use gdl_formats::BossCamera;

use crate::camera_rig::CameraPoint;
use crate::locomotion::wrap;

/// The game's view, which the fit keeps things inside: 60° across a 4:3
/// picture, near plane 1.
const TAN_ACROSS: f32 = 0.577_350_26;
const TAN_UP: f32 = 0.75 * TAN_ACROSS;
const NEAR: f32 = 1.0;
/// A margin when nothing is measured (`r2-0x7b7c`).
const NO_MARGIN: f32 = 1.0e20;

/// The distance steps: out by 10 while something is outside the view, out
/// by 2 × (2.5 − margin) under a margin of 2 (2.25 while already backing
/// off), in by 2 × (margin − 2.5) over 2.5 (over 2.25 while already
/// closing in; awake: by the excess over 2.5, past 4).
const OUT_JUMP: f32 = 10.0;
const SNUG: f32 = 2.0;
const SNUG_HELD: f32 = 2.25;
const EASY: f32 = 2.5;
const WIDE_AWAKE: f32 = 4.0;
const STEP: f32 = 2.0;
/// Before the boss wakes the camera backs off only within 1.5 × the
/// record's far distance, and goes no further than twice that.
const ASLEEP_FAR: f32 = 1.5;
/// The opening is over once the distance is within this of its goal.
const OPENING_DONE: f32 = 5.0;
/// Awake, the pitch waits while the distance is this far off its goal.
const PITCH_WAITS: f32 = 10.0;
/// The fit radius at the key and the wizard, and where there's no boss.
const KEY_RADIUS: f32 = 5.0;
const WIZARD_RADIUS: f32 = 4.0;

/// The easing (`r13-0x7f78`…`-0x7f64`, `r2-0x7af0`, `r2-0x7af4`): yaw and
/// pitch at most π/2 a second, speeding up by π/6 and π/12 a second each
/// second; the distance 200 out and 50 in a second, by 75; within a tenth
/// of a second's top speed of the goal they stop.
const YAW_SPEED: f32 = FRAC_PI_2;
const YAW_ACCEL: f32 = FRAC_PI_6;
const PITCH_SPEED: f32 = FRAC_PI_2;
const PITCH_ACCEL: f32 = FRAC_PI_6 / 2.0;
const DIST_SPEED: f32 = 200.0;
const DIST_IN: f32 = 0.25;
const DIST_ACCEL: f32 = 75.0;
const STOP_BAND: f32 = 0.1;
/// The target's glide: its way turns at most 20 a
/// second (by 20), keeps its progress while it turns less than this cosine,
/// and a goal within 0.001 is taken outright.
const GLIDE_TURN: f32 = 20.0;
const GLIDE_KEEP: f32 = 0.965;
const GLIDE_SNAP: f32 = 0.001;
/// A nearer camera point takes over when it's within 0.667 of the current
/// one's distance (across the ground).
const POINT_SWITCH: f32 = 0.667;
/// The heroes are kept inside the view as seen from this share of the
/// camera's far limit (`r13-0x7f7c`): 1.5 × the asleep far distance before
/// the boss wakes, the far distance awake.
const KEEP_IN_VIEW: f32 = 0.85;

/// A hero as the camera sees it.
#[derive(Clone, Copy, Debug)]
pub struct Hero {
    pub feet: [f32; 3],
    /// The top point (`+0x54`): what's framed and looked at.
    pub top: [f32; 3],
    /// Half its height: its margin radius.
    pub half_height: f32,
}

/// The boss as the camera sees it.
#[derive(Clone, Copy, Debug)]
pub struct Boss {
    pub position: [f32; 3],
    /// Where it was made.
    pub spawn: [f32; 3],
    /// Its facing (0 = +Z).
    pub yaw: f32,
    /// Its cylinder's height (`TYPE +0x78`): its margin radius.
    pub height: f32,
    /// At 0 hit points (`r13-0x7780`).
    pub dying: bool,
}

/// The level as the boss camera sees it this tick.
pub struct Scene<'a> {
    pub heroes: &'a [Hero],
    pub boss: Option<Boss>,
    /// The boss locator, for when the boss is gone.
    pub spot: [f32; 3],
    pub awake: bool,
    /// The end sequence (`r13-0x7790`), and the key and wizard in it.
    pub ending: bool,
    pub key: Option<[f32; 3]>,
    pub wizard: Option<[f32; 3]>,
    /// The level camera's target box.
    pub bounds: ([f32; 3], [f32; 3]),
    /// The level's ordinary camera points, and the entry's starting one.
    pub points: &'a [CameraPoint],
    pub start: Option<CameraPoint>,
}

#[derive(Clone, Debug)]
pub struct BossCam {
    record: BossCamera,
    started: bool,
    /// The opening (`r13-0x7340`): angles from the starting point.
    pub opening: bool,
    /// The record's run-time bits: 0x200 backing off, 0x100 closing in.
    moving: u32,
    pub target: [f32; 3],
    /// The glide (`+0xB0`…`+0xD8`): from here, along this way, this far.
    anchor: [f32; 3],
    way: [f32; 3],
    way_speed: [f32; 3],
    progress: f32,
    progress_speed: f32,
    pub yaw: f32,
    yaw_speed: f32,
    pub pitch: f32,
    pitch_speed: f32,
    pub distance: f32,
    distance_speed: f32,
    /// Where the distance lies between near and far (0..1).
    fraction: f32,
    /// The smallest margin last measured.
    pub margin: f32,
    point: Option<usize>,
    /// The boss's last facing (kept when it's gone).
    facing: f32,
    /// How far behind the target the heroes' view limit is (`+0xDC`).
    limit: f32,
}

impl BossCam {
    pub fn new(record: BossCamera) -> Self {
        Self {
            record,
            started: false,
            opening: false,
            moving: 0,
            target: [0.0; 3],
            anchor: [0.0; 3],
            way: [0.0; 3],
            way_speed: [0.0; 3],
            progress: 0.0,
            progress_speed: 0.0,
            yaw: 0.0,
            yaw_speed: 0.0,
            pitch: 0.0,
            pitch_speed: 0.0,
            distance: 0.0,
            distance_speed: 0.0,
            fraction: 0.0,
            margin: 0.0,
            point: None,
            facing: 0.0,
            limit: KEEP_IN_VIEW * ASLEEP_FAR * record.far_asleep,
        }
    }

    /// The way the camera looks (+Z turned by yaw, then pitch).
    pub fn direction(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, sp, cy * cp]
    }

    pub fn eye(&self) -> [f32; 3] {
        let d = self.direction();
        std::array::from_fn(|i| self.target[i] - d[i] * self.distance)
    }

    /// One game tick of `dt` seconds.
    pub fn tick(&mut self, scene: &Scene, dt: f32) {
        if let Some(b) = scene.boss {
            self.facing = b.yaw;
        }
        if !self.started {
            self.start(scene);
        } else if scene.awake {
            self.awake(scene, dt);
        } else {
            self.asleep(scene, dt);
        }
        self.yaw = wrap(self.yaw);
        self.pitch = wrap(self.pitch);
        self.started = true;
    }

    /// The first tick: at the starting camera point, looking at the
    /// heroes' centre.
    fn start(&mut self, scene: &Scene) {
        self.opening = true;
        let centre = heroes_centre(scene.heroes, None).unwrap_or(scene.spot);
        self.target = centre;
        self.anchor = centre;
        self.way = [0.0; 3];
        self.way_speed = [0.0; 3];
        self.progress = 0.0;
        self.progress_speed = 0.0;
        self.yaw_speed = 0.0;
        self.pitch_speed = 0.0;
        self.distance_speed = 0.0;
        let from = scene.start.map_or_else(|| sub(centre, [0.0, -10.0, 20.0]), |p| p.position);
        self.look(sub(centre, from));
    }

    /// Distance, yaw and pitch straight from the eye along `v`.
    fn look(&mut self, v: [f32; 3]) {
        self.distance = length(v);
        (self.yaw, self.pitch) = angles(v);
    }

    /// Before the boss wakes.
    fn asleep(&mut self, scene: &Scene, dt: f32) {
        let limit = ASLEEP_FAR * self.record.far_asleep;
        self.limit = KEEP_IN_VIEW * limit;
        let centre = heroes_centre(scene.heroes, Some(scene.bounds)).unwrap_or(self.target);
        let point = if self.opening { scene.start } else { self.nearest_point(centre, scene.points) };
        let (goal_yaw, goal_pitch) = point.map_or((0.0, 0.0), |p| (wrap(p.yaw), -p.pitch));
        let margin = self.heroes_margin(scene.heroes);
        self.margin = margin;
        let wanted = self.step_distance(margin, limit, false);
        let goal = if wanted < self.record.near_asleep { self.record.near_asleep } else { wanted.min(STEP * limit) };
        if self.opening && (goal - self.distance).abs() < OPENING_DONE {
            self.opening = false;
        }
        self.target = centre;
        self.anchor = centre;
        self.ease_view(goal_yaw, goal, goal_pitch, dt);
    }

    /// The boss awake.
    fn awake(&mut self, scene: &Scene, dt: f32) {
        self.opening = false;
        let r = self.record;
        self.limit = KEEP_IN_VIEW * r.far;
        let look = lerp(r.look_near, r.look_far, self.fraction);
        let dying = scene.boss.is_some_and(|b| b.dying);
        // What it looks at and frames, how big that is, and how far the
        // target may jump before the camera turns to it at once.
        let (goal_target, framed, radius, jump) = if let (true, Some(w)) = (scene.ending, scene.wizard) {
            (add(w, r.look_wizard), w, WIZARD_RADIUS, 2)
        } else if let Some(k) = scene.key {
            (add(k, r.look_key), k, KEY_RADIUS, 2)
        } else if !scene.ending && !dying {
            match scene.boss {
                None => {
                    let t = add(scene.spot, look);
                    (t, t, KEY_RADIUS, 1)
                }
                Some(b) => {
                    let base = if r.flags & 0x10 != 0 {
                        heroes_centre_with(scene.heroes, b.position)
                    } else if r.flags & 1 != 0 {
                        b.position
                    } else {
                        b.spawn
                    };
                    (add(base, look), b.position, b.height, 1)
                }
            }
        } else {
            (self.target, self.target, KEY_RADIUS, 1)
        };
        let jumped = if dt > 0.0 && DIST_SPEED * dt < length(sub(self.target, goal_target)) { jump } else { 0 };

        let goal_yaw = self.awake_yaw(scene);

        let mut far = r.far;
        if scene.ending {
            far *= STEP;
        }
        let margin = self.heroes_margin(scene.heroes).min(self.margin_of(framed, radius));
        self.margin = margin;
        let wanted = self.step_distance(margin, far, true);
        let goal = if wanted < r.near { r.near } else { wanted.min(STEP * far) };
        let span = r.far - r.near;
        self.fraction = if span <= 0.01 { 1.0 } else { ((self.distance - r.near) / span).clamp(0.0, 1.0) };
        let goal_pitch = if (self.distance - goal).abs() >= PITCH_WAITS {
            self.pitch
        } else {
            -(self.fraction * (r.pitch_far - r.pitch_near) + r.pitch_near)
        };

        let eye = self.eye();
        self.glide(goal_target, dt);
        if jumped == 0 {
            self.ease_view(goal_yaw, goal, goal_pitch, dt);
        } else {
            // A jump: the eye stays and turns to the new target.
            let v = sub(self.target, eye);
            let d = length(v);
            (self.yaw, self.pitch) = angles(v);
            if self.distance < d || jumped > 1 {
                self.distance = d;
            }
        }
    }

    /// The awake yaw: along the way from the heroes furthest from the
    /// target (the target's own when there are none), turned to whichever
    /// side is nearer the camera (unless flag 2), or off the boss's facing
    /// by the record's offset when that way is outside the boss's cone.
    fn awake_yaw(&self, scene: &Scene) -> f32 {
        let r = &self.record;
        let way = if r.flags & 0x20 == 0 {
            heroes_way(scene, self.target, r.flags & 8 != 0, self.yaw)
        } else {
            let centre = if r.flags & 8 != 0 {
                scene.boss.map_or_else(|| heroes_centre(scene.heroes, None), |b| Some(heroes_centre_with(scene.heroes, b.position)))
            } else {
                heroes_centre(scene.heroes, None)
            };
            centre.and_then(|c| {
                let v = sub(self.target, c);
                let (len, unit) = flat_unit(v);
                (len >= 1.0).then_some(unit)
            })
        };
        let Some(mut way) = way else { return self.yaw };
        let facing = [self.facing.sin(), 0.0, self.facing.cos()];
        let cos_cone = r.yaw_offset.cos();
        let mut cone = dot_flat(way, facing);
        if self.started {
            let (_, camera) = flat_unit(sub(self.eye(), self.target));
            let mut score;
            if r.flags & 4 == 0 {
                cone = dot_flat(way, facing);
                score = if cone < cos_cone { -1.0 } else { dot_flat(camera, way) };
            } else {
                cone = -1.0;
                score = -1.0;
            }
            if r.flags & 2 == 0 {
                let other = [-way[0], -way[1], -way[2]];
                let other_cone = dot_flat(other, facing);
                let other_score = if other_cone < cos_cone { -1.0 } else { dot_flat(camera, other) };
                if score < other_score {
                    way = other;
                    cone = other_cone;
                    score = other_score;
                }
            }
            let _ = score;
        }
        if cos_cone <= cone {
            wrap(way[0].atan2(way[2]) + PI)
        } else {
            let f = facing[0].atan2(facing[2]);
            let side = way[2] * facing[0] - way[0] * facing[2];
            let y = if side < 0.0 { f + r.yaw_offset } else { f - r.yaw_offset };
            wrap(y + PI)
        }
    }

    /// The distance the margin asks for: out by 10 when something is
    /// outside the view, out while snug (within `limit`), in while roomy;
    /// the direction it went is remembered for the in-between band.
    fn step_distance(&mut self, margin: f32, limit: f32, awake: bool) -> f32 {
        let (was_out, was_in) = (self.moving & 0x200 != 0, self.moving & 0x100 != 0);
        let d = self.distance;
        let near_limit = d < limit;
        let (moving, d) = if margin < 0.0 {
            (0x200, d + OUT_JUMP)
        } else if margin < SNUG && near_limit || margin < SNUG_HELD && near_limit && was_out {
            (0x200, d + STEP * (EASY - margin))
        } else if awake {
            if margin > WIDE_AWAKE || margin > EASY && was_in { (0x100, d - (margin - EASY)) } else { (0, d) }
        } else if margin > EASY {
            (0x100, d - STEP * (margin - EASY))
        } else if margin > SNUG_HELD && was_in {
            (0x100, d - STEP * (margin - SNUG_HELD))
        } else {
            (0, d)
        };
        self.moving = moving;
        d
    }

    fn ease_view(&mut self, yaw: f32, distance: f32, pitch: f32, dt: f32) {
        self.yaw = ease(self.yaw, yaw, -YAW_SPEED, YAW_SPEED, YAW_ACCEL, STOP_BAND, &mut self.yaw_speed, true, dt);
        let (lo, hi) = (-DIST_IN * DIST_SPEED, DIST_SPEED);
        self.distance = ease(self.distance, distance, lo, hi, DIST_ACCEL, STOP_BAND, &mut self.distance_speed, false, dt);
        self.pitch = ease(self.pitch, pitch, -PITCH_SPEED, PITCH_SPEED, PITCH_ACCEL, STOP_BAND, &mut self.pitch_speed, true, dt);
    }

    /// The target glides after `to`: from its anchor along a way that turns
    /// only slowly, the progress easing out to the goal's distance; a way
    /// turning too far starts over from where the target is.
    fn glide(&mut self, to: [f32; 3], dt: f32) {
        let delta = sub(to, self.anchor);
        let len = length(delta);
        if len <= GLIDE_SNAP {
            self.target = to;
            self.anchor = to;
            self.way = [0.0; 3];
            self.way_speed = [0.0; 3];
            self.progress = 0.0;
            self.progress_speed = 0.0;
            return;
        }
        let unit = scale(delta, 1.0 / len);
        let (lo, hi) = (-DIST_SPEED, DIST_SPEED);
        if self.progress > 0.0 {
            if dot(unit, self.way) >= GLIDE_KEEP {
                self.progress = ease(self.progress, len, lo, hi, DIST_ACCEL, 0.0, &mut self.progress_speed, false, dt);
            } else {
                self.progress = 0.0;
                self.anchor = self.target;
            }
            for ((way, speed), goal) in self.way.iter_mut().zip(&mut self.way_speed).zip(unit) {
                *way = ease(*way, goal, -GLIDE_TURN, GLIDE_TURN, GLIDE_TURN, 0.0, speed, false, dt);
            }
        } else {
            self.progress = ease(self.progress, len, lo, hi, DIST_ACCEL, 0.0, &mut self.progress_speed, false, dt);
            self.way = unit;
            self.way_speed = [0.0; 3];
        }
        self.target = add(self.anchor, scale(self.way, self.progress));
    }

    /// The nearest ordinary camera point across the ground; another takes
    /// over only when it's within 0.667 of the current one's distance.
    fn nearest_point(&mut self, at: [f32; 3], points: &[CameraPoint]) -> Option<CameraPoint> {
        let ground = |p: &CameraPoint| (at[0] - p.position[0]).hypot(at[2] - p.position[2]);
        let best = (0..points.len())
            .filter(|&i| Some(i) != self.point)
            .min_by(|&a, &b| ground(&points[a]).total_cmp(&ground(&points[b])));
        match (self.point, best) {
            (None, b) => self.point = b,
            (Some(cur), Some(b)) if ground(&points[b]) <= POINT_SWITCH * ground(&points[cur]) => self.point = Some(b),
            _ => {}
        }
        self.point.and_then(|i| points.get(i).copied())
    }

    /// A hero's step under the boss camera (the doc's "Players"): the four
    /// sides of the game's view, from its limit distance behind the target;
    /// a step ending outside one (the centre's end, the feet's for the
    /// bottom side) and heading out through it slides along it across the
    /// ground, keeping its rise and fall.
    pub fn keep_in_view(&self, feet: [f32; 3], centre: [f32; 3], step: [f32; 3]) -> [f32; 3] {
        let forward = self.direction();
        let Some(right) = normalize(cross([0.0, 1.0, 0.0], forward)) else { return step };
        let up = cross(forward, right);
        let apex = sub(self.target, scale(forward, self.limit));
        let side = |edge: [f32; 3], tan: f32| sub(edge, scale(forward, tan));
        let sides = [
            (side(right, TAN_ACROSS), centre),
            (side(scale(right, -1.0), TAN_ACROSS), centre),
            (side(up, TAN_UP), centre),
            (side(scale(up, -1.0), TAN_UP), feet),
        ];
        let mut out = step;
        for (normal, from) in sides {
            if dot(sub(add(from, out), apex), normal) <= 0.0 || dot(out, normal) <= 0.0 {
                continue;
            }
            let Some(across) = normalize([normal[0], 0.0, normal[2]]) else { continue };
            let along = dot(out, across);
            out = sub(out, scale(across, along));
        }
        out[1] = step[1];
        out
    }

    /// The smallest margin of the heroes' top points.
    fn heroes_margin(&self, heroes: &[Hero]) -> f32 {
        heroes.iter().map(|h| self.margin_of(h.top, h.half_height)).fold(NO_MARGIN, f32::min)
    }

    /// How far `p` is inside the game's view (past the near plane and
    /// inside each side), less `radius` — in the view of the last tick.
    fn margin_of(&self, p: [f32; 3], radius: f32) -> f32 {
        let forward = self.direction();
        let right = normalize(cross([0.0, 1.0, 0.0], forward)).unwrap_or([1.0, 0.0, 0.0]);
        let up = cross(forward, right);
        let v = sub(p, self.eye());
        let (x, y, z) = (dot(v, right), dot(v, up), dot(v, forward));
        let (cos_across, cos_up) = (cos_of(TAN_ACROSS), cos_of(TAN_UP));
        [
            z - NEAR - radius,
            cos_across * (x + z * TAN_ACROSS) - radius,
            cos_across * (z * TAN_ACROSS - x) - radius,
            cos_up * (y + z * TAN_UP) - radius,
            cos_up * (z * TAN_UP - y) - radius,
        ]
        .into_iter()
        .fold(NO_MARGIN, f32::min)
    }
}

/// The heroes' centre: the middle of the box round their top points, kept
/// to `bounds` when given. None without heroes.
fn heroes_centre(heroes: &[Hero], bounds: Option<([f32; 3], [f32; 3])>) -> Option<[f32; 3]> {
    let (lo, hi) = heroes_box(heroes)?;
    let c: [f32; 3] = std::array::from_fn(|i| 0.5 * (lo[i] + hi[i]));
    Some(match bounds {
        Some((min, max)) => std::array::from_fn(|i| c[i].clamp(min[i], max[i].max(min[i]))),
        None => c,
    })
}

/// The middle of the box round the heroes' top points and `boss`.
fn heroes_centre_with(heroes: &[Hero], boss: [f32; 3]) -> [f32; 3] {
    let (lo, hi) = heroes_box(heroes).unwrap_or((boss, boss));
    std::array::from_fn(|i| 0.5 * (lo[i].min(boss[i]) + hi[i].max(boss[i])))
}

fn heroes_box(heroes: &[Hero]) -> Option<([f32; 3], [f32; 3])> {
    let first = heroes.first()?;
    Some(heroes.iter().fold((first.top, first.top), |(lo, hi), h| {
        (std::array::from_fn(|i| lo[i].min(h.top[i])), std::array::from_fn(|i| hi[i].max(h.top[i])))
    }))
}

/// The way from the target toward the heroes (the doc's "yaw" rule):
/// the two furthest from it across the ground (the boss counted too with
/// `with_boss`), bisected — the bisector nearer the first's side, or when
/// the two are almost opposite, the one nearer the camera's yaw.
fn heroes_way(scene: &Scene, target: [f32; 3], with_boss: bool, camera_yaw: f32) -> Option<[f32; 3]> {
    let mut ways: Vec<(f32, [f32; 3])> = Vec::new();
    let boss = scene.boss.filter(|_| with_boss).map(|b| b.position);
    for from in boss.into_iter().chain(scene.heroes.iter().map(|h| h.feet)) {
        let (len, unit) = flat_unit(sub(target, from));
        if len > 0.0 {
            ways.push((len, unit));
        }
    }
    ways.sort_by(|a, b| b.0.total_cmp(&a.0));
    let yaw = match ways.as_slice() {
        [] => return None,
        [(_, one)] => one[0].atan2(one[2]),
        [(_, first), (_, second), ..] => {
            let (a, b) = (first[0].atan2(first[2]), second[0].atan2(second[2]));
            let mut mid = wrap(0.5 * (a + b));
            if wrap(mid - a).abs() > FRAC_PI_2 {
                mid = wrap(mid + PI);
            }
            if wrap(a - b).abs() > (160.0f32).to_radians() && wrap(camera_yaw - mid).abs() > FRAC_PI_2 {
                mid = wrap(mid + PI);
            }
            mid
        }
    };
    Some([-yaw.sin(), 0.0, -yaw.cos()])
}

/// Eases `value` toward `goal` (the doc's easing): speeds up by `accel` a
/// second while it can still stop in time and slows otherwise, within
/// `min..max` a second; reaches the goal outright when this step would get
/// there; stops when it and the gap are within `band` s of top speed.
#[allow(clippy::too_many_arguments)]
fn ease(value: f32, goal: f32, min: f32, max: f32, accel: f32, band: f32, speed: &mut f32, wraps: bool, dt: f32) -> f32 {
    let gap = if wraps { wrap(goal - value) } else { goal - value };
    let dv = accel * dt;
    let near = dt * band * max;
    let v = *speed;
    let new = if band <= 0.0 || near <= v.abs() || near <= gap.abs() {
        if dt * (v.abs() + dv) <= gap.abs() {
            let s = if v.abs() / accel < gap.abs() / v.abs() {
                if gap <= 0.0 { v - dv } else { v + dv }
            } else if v <= 0.0 {
                (v + dv).min(0.0)
            } else {
                (v - dv).max(0.0)
            };
            if s < min { min } else { s.min(max) }
        } else {
            gap / dt
        }
    } else {
        0.0
    };
    *speed = new;
    value + new * dt
}

/// Yaw and pitch of `v` (0 = +Z; pitch negative looking down).
fn angles(v: [f32; 3]) -> (f32, f32) {
    (v[0].atan2(v[2]), v[1].atan2(v[0].hypot(v[2])))
}

fn cos_of(tan: f32) -> f32 {
    1.0 / (1.0 + tan * tan).sqrt()
}

/// Across the ground: the length and the unit way (y 0).
fn flat_unit(v: [f32; 3]) -> (f32, [f32; 3]) {
    let len = v[0].hypot(v[2]);
    if len > 0.0 { (len, [v[0] / len, 0.0, v[2] / len]) } else { (0.0, [1.0, 0.0, 0.0]) }
}

fn dot_flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[2] * b[2]
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + t * (b[i] - a[i]))
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|x| x * s)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = length(a);
    (l > 1e-6).then(|| scale(a, 1.0 / l))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 30.0;

    fn record() -> BossCamera {
        BossCamera {
            flags: 1,
            yaw_offset: 18f32.to_radians(),
            near: 40.0,
            near_asleep: 25.0,
            far: 85.0,
            far_asleep: 30.0,
            pitch_near: 15f32.to_radians(),
            pitch_far: 9f32.to_radians(),
            look_near: [0.0, -5.0, 10.0],
            look_far: [0.0, -20.0, 0.0],
            look_key: [0.0, 5.0, 0.0],
            look_wizard: [0.0, -5.0, 0.0],
        }
    }

    fn hero(at: [f32; 3]) -> Hero {
        Hero { feet: at, top: [at[0], at[1] + 4.4, at[2]], half_height: 2.5 }
    }

    fn scene<'a>(heroes: &'a [Hero], points: &'a [CameraPoint], boss: Option<Boss>, awake: bool) -> Scene<'a> {
        Scene {
            heroes,
            boss,
            spot: [0.0; 3],
            awake,
            ending: false,
            key: None,
            wizard: None,
            bounds: ([-1000.0; 3], [1000.0; 3]),
            points,
            start: Some(CameraPoint { position: [0.0, 30.0, -60.0], yaw: 0.0, pitch: 0.4, param: 0 }),
        }
    }

    #[test]
    fn easing_speeds_up_holds_its_limit_and_lands_on_the_goal() {
        let mut speed = 0.0;
        let mut d = 0.0;
        d = ease(d, 100.0, -50.0, 200.0, 75.0, 0.1, &mut speed, false, DT);
        assert!((speed - 2.5).abs() < 1e-4 && d > 0.0, "{speed} {d}");
        for _ in 0..600 {
            d = ease(d, 100.0, -50.0, 200.0, 75.0, 0.1, &mut speed, false, DT);
            assert!(speed <= 200.0);
        }
        assert!((d - 100.0).abs() < 0.5, "{d}");
        // Angles take the short way round.
        let mut s = 0.0;
        let y = ease(3.0, -3.0, -FRAC_PI_2, FRAC_PI_2, 0.5, 0.1, &mut s, true, DT);
        assert!(y > 3.0 || y < -3.0, "{y}");
    }

    #[test]
    fn it_opens_at_the_starting_point_then_backs_off_to_frame_the_heroes() {
        let heroes = [hero([0.0, 0.0, 0.0])];
        let points = [CameraPoint { position: [0.0, 20.0, -30.0], yaw: 0.0, pitch: 0.5, param: 0 }];
        let mut cam = BossCam::new(record());
        let s = scene(&heroes, &points, None, false);
        cam.tick(&s, DT);
        let eye = cam.eye();
        assert!((eye[1] - 30.0).abs() < 1e-3 && (eye[2] + 60.0).abs() < 1e-3, "{eye:?}");
        assert!(cam.opening);
        for _ in 0..300 {
            cam.tick(&s, DT);
        }
        // A lone hero is framed well inside the view, between the asleep
        // near distance and the limit, from the point's angles.
        assert!(!cam.opening);
        assert!(cam.distance >= 25.0 && cam.distance <= 90.0, "{}", cam.distance);
        assert!(cam.margin > 0.0, "{}", cam.margin);
        assert!((cam.pitch + 0.5).abs() < 0.05, "{}", cam.pitch);
    }

    #[test]
    fn a_hero_can_walk_along_the_views_edge_but_not_out() {
        let heroes = [hero([0.0, 0.0, 0.0])];
        let boss = Boss { position: [0.0, 0.0, 30.0], spawn: [0.0, 0.0, 30.0], yaw: PI, height: 12.0, dying: false };
        let mut cam = BossCam::new(record());
        let s = scene(&heroes, &[], Some(boss), true);
        for _ in 0..300 {
            cam.tick(&s, DT);
        }
        // Far to the side, outside the view: a step further out is turned
        // along the edge; a step back in is untouched.
        let right = normalize(cross([0.0, 1.0, 0.0], cam.direction())).unwrap();
        let far = scale(right, 400.0);
        let feet = add(cam.target, far);
        let centre = add(feet, [0.0, 2.5, 0.0]);
        let out = scale(right, 1.0);
        let kept = cam.keep_in_view(feet, centre, out);
        assert!(length(kept) < 0.99, "{kept:?}");
        let back = scale(right, -1.0);
        assert_eq!(cam.keep_in_view(feet, centre, back), back);
        // Inside the view nothing changes.
        assert_eq!(cam.keep_in_view(cam.target, add(cam.target, [0.0, 2.5, 0.0]), out), out);
    }

    #[test]
    fn awake_it_looks_at_the_boss_from_the_heroes_side() {
        let heroes = [hero([0.0, 0.0, -30.0])];
        let boss = Boss { position: [0.0, 0.0, 0.0], spawn: [0.0, 0.0, 0.0], yaw: PI, height: 12.0, dying: false };
        let mut cam = BossCam::new(record());
        let s = scene(&heroes, &[], Some(boss), true);
        cam.tick(&s, DT);
        for _ in 0..400 {
            cam.tick(&s, DT);
        }
        // Looking from the hero (−Z) toward the boss: along +Z.
        assert!(wrap(cam.yaw).abs() < 0.1, "{}", cam.yaw);
        let eye = cam.eye();
        assert!(eye[2] < -30.0, "{eye:?}");
        assert!(cam.distance >= 40.0 && cam.distance <= 170.0, "{}", cam.distance);
    }
}
