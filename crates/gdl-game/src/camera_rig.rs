//! The game's play camera, as pure logic (`docs/camera.md`).
//!
//! A level scatters camera points ("transmitters") over its map, each with a
//! yaw and a downward pitch. The camera looks at the players from the
//! nearest point's angles, switching points only when another is clearly
//! nearer to the players' feet and then turning over 50 ticks; it looks at
//! the players' top points, clamped to a box and smoothed over the last 9
//! ticks, from the level's distance.

use crate::locomotion::wrap;

/// Samples averaged for the target and the distance.
pub const SMOOTHING: usize = 9;
/// Ticks a turn to a new camera point takes.
pub const TURN_TICKS: f32 = 50.0;
/// A point takes over when its squared distance is at most this fraction of
/// the current point's — i.e. it is at most 2/3 as far away.
pub const SWITCH_RATIO: f32 = 4.0 / 9.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPoint {
    pub position: [f32; 3],
    /// Direction the camera faces (0 = +Z, π/2 = +X).
    pub yaw: f32,
    /// How far it looks down, radians (positive = down).
    pub pitch: f32,
}

#[derive(Clone, Debug)]
pub struct CameraRig {
    points: Vec<CameraPoint>,
    current: Option<usize>,
    bounds: ([f32; 3], [f32; 3]),
    near: f32,
    /// Facing, and pitch in the game's sign (negative looks down).
    pub yaw: f32,
    pub pitch: f32,
    turn: (f32, f32),
    turned: f32,
    samples: [[f32; 3]; SMOOTHING],
    distances: [f32; SMOOTHING],
    slot: usize,
    pub target: [f32; 3],
    pub distance: f32,
}

impl CameraRig {
    /// A camera at rest on `focus`, already facing the point nearest
    /// `feet`.
    pub fn new(points: Vec<CameraPoint>, bounds: ([f32; 3], [f32; 3]), near: f32, focus: [f32; 3], feet: [f32; 3]) -> Self {
        let p = clamp(focus, bounds);
        let mut rig = Self {
            points,
            current: None,
            bounds,
            near,
            yaw: 0.0,
            pitch: 0.0,
            turn: (0.0, 0.0),
            turned: TURN_TICKS,
            samples: [p; SMOOTHING],
            distances: [near; SMOOTHING],
            slot: 0,
            target: p,
            distance: near,
        };
        rig.current = rig.nearest(clamp(feet, bounds));
        (rig.yaw, rig.pitch) = rig.wanted();
        rig
    }

    /// The level's ordinary camera points.
    pub fn points(&self) -> &[CameraPoint] {
        &self.points
    }

    /// The box the target is kept in.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        self.bounds
    }

    pub fn current_point(&self) -> Option<CameraPoint> {
        self.current.map(|i| self.points[i])
    }

    /// One game tick following `focus` (the players' top points' centre)
    /// with points chosen by `feet` (their feet's centre).
    pub fn tick(&mut self, focus: [f32; 3], feet: [f32; 3]) {
        self.slot = (self.slot + 1) % SMOOTHING;
        self.samples[self.slot] = clamp(focus, self.bounds);

        // The nearest other point takes over only when clearly nearer.
        let p = clamp(feet, self.bounds);
        let before = self.current;
        match (self.current, self.nearest(p)) {
            (None, next) => self.current = next,
            (Some(cur), Some(next)) if dist_sq(p, self.points[next].position) <= SWITCH_RATIO * dist_sq(p, self.points[cur].position) => {
                self.current = Some(next)
            }
            _ => {}
        }
        if self.current != before {
            let (yaw, pitch) = self.wanted();
            self.turn = (wrap(yaw - self.yaw) / TURN_TICKS, wrap(pitch - self.pitch) / TURN_TICKS);
            self.turned = 0.0;
        }
        if self.turned < TURN_TICKS {
            self.yaw = wrap(self.yaw + self.turn.0);
            self.pitch = wrap(self.pitch + self.turn.1);
            self.turned += 1.0;
        }

        for axis in 0..3 {
            let pull: f32 = self.samples.iter().map(|s| s[axis] - self.target[axis]).sum();
            self.target[axis] += pull / SMOOTHING as f32;
        }
        self.distances[self.slot] = self.near;
        let pull: f32 = self.distances.iter().map(|d| d - self.distance).sum();
        self.distance += pull / SMOOTHING as f32;
    }

    /// Unit vector the camera looks along.
    pub fn direction(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [sy * cp, sp, cy * cp]
    }

    pub fn eye(&self) -> [f32; 3] {
        let d = self.direction();
        std::array::from_fn(|i| self.target[i] - d[i] * self.distance)
    }

    /// The angles the current point asks for (none: level with the horizon,
    /// facing +Z).
    fn wanted(&self) -> (f32, f32) {
        self.current_point().map_or((0.0, 0.0), |c| (wrap(c.yaw), -c.pitch))
    }

    /// Nearest point to `p`, other than the current one.
    fn nearest(&self, p: [f32; 3]) -> Option<usize> {
        (0..self.points.len())
            .filter(|&i| Some(i) != self.current)
            .min_by(|&a, &b| dist_sq(p, self.points[a].position).total_cmp(&dist_sq(p, self.points[b].position)))
    }
}

fn clamp(p: [f32; 3], (lo, hi): ([f32; 3], [f32; 3])) -> [f32; 3] {
    std::array::from_fn(|i| if p[i] < lo[i] { lo[i] } else { p[i].min(hi[i]) })
}

fn dist_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    const WIDE: ([f32; 3], [f32; 3]) = ([-1000.0; 3], [1000.0; 3]);

    fn point(x: f32, yaw: f32, pitch: f32) -> CameraPoint {
        CameraPoint { position: [x, 0.0, 0.0], yaw, pitch }
    }

    #[test]
    fn starts_on_the_nearest_point_looking_down_at_the_focus() {
        let rig = CameraRig::new(vec![point(0.0, 0.0, 0.6), point(50.0, 1.0, 0.3)], WIDE, 24.0, [5.0, 0.0, 0.0], [5.0, 0.0, 0.0]);
        assert_eq!(rig.current_point().unwrap().position[0], 0.0);
        assert!((rig.pitch + 0.6).abs() < 1e-6);
        let eye = rig.eye();
        assert!(eye[1] > 0.0 && eye[2] < 0.0, "above and behind: {eye:?}");
        let d = ((eye[0] - 5.0).powi(2) + eye[1].powi(2) + eye[2].powi(2)).sqrt();
        assert!((d - 24.0).abs() < 1e-4);
    }

    #[test]
    fn looks_at_the_top_point_and_picks_points_by_the_feet() {
        // The feet are nearer the first point, the top point (4.4 higher)
        // the second: the feet choose, the top is looked at.
        let high = CameraPoint { position: [5.0, 9.0, 0.0], yaw: 1.0, pitch: 0.3 };
        let (feet, top) = ([5.0, 0.0, 0.0], [5.0, 4.4, 0.0]);
        let mut rig = CameraRig::new(vec![point(0.0, 0.0, 0.6), high], WIDE, 24.0, top, feet);
        assert_eq!(rig.current_point().unwrap().position, [0.0; 3]);
        assert!((rig.target[1] - 4.4).abs() < 1e-6);
        rig.tick(top, feet);
        assert_eq!(rig.current_point().unwrap().position, [0.0; 3]);
        assert!((rig.target[1] - 4.4).abs() < 1e-6);
    }

    #[test]
    fn switches_only_when_clearly_nearer_then_turns_over_fifty_ticks() {
        let mut rig = CameraRig::new(vec![point(0.0, 0.0, 0.5), point(30.0, FRAC_PI_2, 0.5)], WIDE, 24.0, [0.0; 3], [0.0; 3]);
        // At x = 16 the second point is 14 away vs 16: not 2/3 as far.
        rig.tick([16.0, 0.0, 0.0], [16.0, 0.0, 0.0]);
        assert_eq!(rig.current_point().unwrap().position[0], 0.0);
        // At x = 20: 10 vs 20.
        rig.tick([20.0, 0.0, 0.0], [20.0, 0.0, 0.0]);
        assert_eq!(rig.current_point().unwrap().position[0], 30.0);
        for _ in 0..25 {
            rig.tick([20.0, 0.0, 0.0], [20.0, 0.0, 0.0]);
        }
        assert!((rig.yaw - FRAC_PI_2 * 26.0 / 50.0).abs() < 1e-4, "{}", rig.yaw);
        for _ in 0..40 {
            rig.tick([20.0, 0.0, 0.0], [20.0, 0.0, 0.0]);
        }
        assert!((rig.yaw - FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn target_is_clamped_and_smoothed() {
        let bounds = ([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]);
        let mut rig = CameraRig::new(vec![point(0.0, 0.0, 0.5)], bounds, 24.0, [0.0; 3], [0.0; 3]);
        rig.tick([100.0, 0.0, 0.0], [100.0, 0.0, 0.0]);
        // One of nine samples moved to the clamp edge.
        assert!((rig.target[0] - 10.0 / 9.0).abs() < 1e-5, "{:?}", rig.target);
        for _ in 0..200 {
            rig.tick([100.0, 0.0, 0.0], [100.0, 0.0, 0.0]);
        }
        assert!((rig.target[0] - 10.0).abs() < 1e-3);
    }
}
