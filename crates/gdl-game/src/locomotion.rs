//! Player locomotion: how the stick turns into movement, facing and the
//! idle/walk/run actions. Pure logic stepped at the game's 30 Hz tick; the
//! numbers are the game's own (`docs/player-movement.md`).

use std::f32::consts::PI;

/// The game simulates at 30 ticks per second.
pub const TICK_HZ: f64 = 30.0;

/// Stick magnitudes above this run; at or below (but above zero) walk.
pub const RUN_THRESHOLD: f32 = 0.75;
/// Turning speed toward the stick direction, radians per second (5π),
/// scaled by the current action's turn factor.
pub const TURN_RATE: f32 = 5.0 * PI;
/// Per axis, one tick's movement never exceeds this × speed × dt.
pub const MAX_STEP: f32 = 1.5;
/// Knockback velocity keeps this fraction each tick.
pub const KNOCKBACK_DECAY: f32 = 0.667;
/// While being knocked back faster than this (per tick, squared), stick
/// input pointing more than 120° away from the knockback is ignored.
const KNOCKBACK_RESIST_SQ: f32 = 0.0025;
const KNOCKBACK_RESIST_ANGLE: f32 = 2.094_395_1;

/// Movement speed range the speed stat (0–999) maps onto, units/second.
pub const SPEED_MIN: f32 = 5.0;
pub const SPEED_MAX: f32 = 12.5;
/// Highest value any stat reaches.
pub const STAT_CAP: f32 = 999.0;
/// Each character level adds this to every stat's starting value.
pub const STAT_PER_LEVEL: f32 = 5.0;

/// A stat's value at a character level: the class's start plus 5 per level
/// past the first, never above the class maximum; bonuses (armour, runes)
/// go on top, capped at 999.
pub fn stat_at_level(start: f32, max: f32, level: u32, bonus: f32) -> f32 {
    let base = (start + level.saturating_sub(1) as f32 * STAT_PER_LEVEL).min(max);
    (base + bonus).min(STAT_CAP)
}

/// Units per second for a speed stat; speed boosts add on top, and the
/// result stays within the game's range.
pub fn move_speed(speed_stat: f32, boost: f32) -> f32 {
    let base = SPEED_MIN + 0.001 * speed_stat * (SPEED_MAX - SPEED_MIN);
    (base + boost).clamp(SPEED_MIN, SPEED_MAX)
}

/// What the player's stick asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    Idle,
    Walk,
    Run,
}

impl Gait {
    pub fn from_magnitude(magnitude: f32) -> Self {
        if magnitude <= 0.0 {
            Gait::Idle
        } else if magnitude <= RUN_THRESHOLD {
            Gait::Walk
        } else {
            Gait::Run
        }
    }

    /// The player action the game plays for this gait.
    pub fn action(self) -> &'static str {
        match self {
            Gait::Idle => "READY",
            Gait::Walk => "WALK1",
            Gait::Run => "RUN1",
        }
    }

    /// Movement and turn factors the game gives the gait's action when it
    /// starts: running covers 1.3× the ground.
    pub fn factors(self) -> (f32, f32) {
        match self {
            Gait::Run => (1.3, 1.0),
            Gait::Idle | Gait::Walk => (1.0, 1.0),
        }
    }
}

/// Stick input already turned into the world: `heading` is the direction to
/// move (radians; 0 = +Z, π/2 = +X), `magnitude` is 0–1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stick {
    pub heading: f32,
    pub magnitude: f32,
}

/// One player's movement state.
#[derive(Clone, Copy, Debug)]
pub struct Mover {
    pub position: [f32; 3],
    /// Direction the body faces (same convention as `Stick::heading`).
    pub facing: f32,
    /// Units per second, from `move_speed`.
    pub speed: f32,
    /// Knockback velocity, units per second, decaying each tick.
    pub knockback: [f32; 3],
    pub gait: Gait,
}

impl Mover {
    pub fn new(position: [f32; 3], facing: f32, speed: f32) -> Self {
        Self { position, facing, speed, knockback: [0.0; 3], gait: Gait::Idle }
    }

    /// Advances one tick. Returns the displacement the tick wants; the
    /// caller resolves it against collision and writes the result back to
    /// `position`.
    pub fn step(&mut self, stick: Stick, dt: f32) -> [f32; 3] {
        let (move_factor, turn_factor) = self.gait.factors();
        let mut magnitude = stick.magnitude.clamp(0.0, 1.0);

        let mut d = self.knockback.map(|v| v * dt);
        let pushed_sq = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if pushed_sq >= KNOCKBACK_RESIST_SQ && magnitude > 0.0 {
            let off = wrap(d[0].atan2(d[2]) - stick.heading);
            if !(-KNOCKBACK_RESIST_ANGLE..=KNOCKBACK_RESIST_ANGLE).contains(&off) {
                magnitude = 0.0;
            }
        }

        let run = move_factor * dt * self.speed * magnitude;
        let limit = MAX_STEP * self.speed * dt;
        d[0] = (d[0] + run * stick.heading.sin()).clamp(-limit, limit);
        d[2] = (d[2] + run * stick.heading.cos()).clamp(-limit, limit);
        self.knockback = self.knockback.map(|v| v * KNOCKBACK_DECAY);

        if magnitude > 0.0 {
            let max_turn = TURN_RATE * dt * turn_factor;
            let turn = wrap(stick.heading - self.facing).clamp(-max_turn, max_turn);
            self.facing = wrap(self.facing + turn);
        }
        self.gait = Gait::from_magnitude(magnitude);
        d
    }
}

/// Wraps an angle into (-π, π].
pub fn wrap(a: f32) -> f32 {
    let a = (a + PI).rem_euclid(2.0 * PI) - PI;
    if a == -PI { PI } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / TICK_HZ as f32;

    #[test]
    fn speed_stat_spans_the_games_range() {
        assert_eq!(move_speed(0.0, 0.0), 5.0);
        assert!((move_speed(1000.0, 0.0) - 12.5).abs() < 1e-5);
        assert!((move_speed(400.0, 0.0) - 8.0).abs() < 1e-5);
        assert_eq!(move_speed(999.0, 10.0), SPEED_MAX);
    }

    #[test]
    fn stats_grow_five_per_level_up_to_the_class_max() {
        assert_eq!(stat_at_level(300.0, 600.0, 1, 0.0), 300.0);
        assert_eq!(stat_at_level(300.0, 600.0, 11, 0.0), 350.0);
        assert_eq!(stat_at_level(300.0, 600.0, 99, 0.0), 600.0);
        assert_eq!(stat_at_level(900.0, 999.0, 99, 200.0), 999.0);
    }

    #[test]
    fn gait_thresholds() {
        assert_eq!(Gait::from_magnitude(0.0), Gait::Idle);
        assert_eq!(Gait::from_magnitude(0.3), Gait::Walk);
        assert_eq!(Gait::from_magnitude(0.75), Gait::Walk);
        assert_eq!(Gait::from_magnitude(0.76), Gait::Run);
    }

    #[test]
    fn running_uses_the_run_factor_after_the_first_tick() {
        let mut m = Mover::new([0.0; 3], 0.0, 8.0);
        let stick = Stick { heading: 0.0, magnitude: 1.0 };
        let first = m.step(stick, DT);
        assert!((first[2] - 8.0 * DT).abs() < 1e-6, "{first:?}");
        assert_eq!(m.gait, Gait::Run);
        let second = m.step(stick, DT);
        assert!((second[2] - 1.3 * 8.0 * DT).abs() < 1e-6, "{second:?}");
    }

    #[test]
    fn facing_turns_at_five_pi_per_second() {
        let mut m = Mover::new([0.0; 3], 0.0, 8.0);
        m.step(Stick { heading: PI / 2.0, magnitude: 1.0 }, DT);
        assert!((m.facing - 5.0 * PI * DT).abs() < 1e-5);
        for _ in 0..3 {
            m.step(Stick { heading: PI / 2.0, magnitude: 1.0 }, DT);
        }
        assert!((m.facing - PI / 2.0).abs() < 1e-5);
    }

    #[test]
    fn knockback_decays_and_resists_opposing_input() {
        let mut m = Mover::new([0.0; 3], 0.0, 8.0);
        m.knockback = [0.0, 0.0, 30.0];
        // Pushing straight against the knockback does nothing.
        let d = m.step(Stick { heading: PI, magnitude: 1.0 }, DT);
        assert!((d[2] - 1.5 * 8.0 * DT).abs() < 1e-6, "clamped to the step limit: {d:?}");
        assert!((m.knockback[2] - 30.0 * KNOCKBACK_DECAY).abs() < 1e-4);
    }

    #[test]
    fn wrap_keeps_angles_in_range() {
        assert!((wrap(3.0 * PI) - PI).abs() < 1e-5);
        assert!((wrap(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-5);
    }
}
