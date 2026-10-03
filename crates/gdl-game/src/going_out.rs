//! A hero going out through a portal (`docs/items.md`, "Going out"): the
//! game's exit state, and the death light it shines on the hero.
//!
//! Once an exit has played its last action the hero stops: it spins
//! (3π a second) and sinks (0.12 a field) through the floor it stood on
//! until its top is a unit below, drawn through the death light, and after
//! 50 fields it's gone; a secret exit takes it at once. Blows don't reach
//! it meanwhile. The tower's portals and the realms' exits are the same.
//!
//! The death light is the game's timed texture effect with `DEATHLIGHT`'s
//! ten frames (`docs/rendering.md`, "Texture overrides"): a counter from
//! minus its step, up a step each tick, showing the frame it's at, round
//! once more from 0 at 10. The exit's steps 0.4; a boss level's end, as the
//! heroes teleport out, 0.5.

/// Video fields a 30 Hz tick.
const FIELDS_PER_TICK: i32 = gdl_formats::enemy::FIELDS_PER_TICK as i32;

/// Fields from the portal's last action to the hero gone.
pub const FIELDS: i32 = 50;
/// How far it sinks a field, and how fast it spins (radians a second).
const SINK_PER_FIELD: f32 = 0.12;
const SPIN: f32 = 3.0 * std::f32::consts::PI;
/// It sinks until its top is this far below the floor.
const BELOW: f32 = 1.0;

/// The exit's light step, and a boss level's end's.
pub const EXIT_LIGHT_STEP: f32 = 0.4;
pub const TELEPORT_LIGHT_STEP: f32 = 0.5;
/// The light's counter ends here (its ten frames), and goes round this
/// many more times.
const LIGHT_END: f32 = 10.0;
const LIGHT_REPEATS: u8 = 1;

/// The death light on a hero.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeathLight {
    counter: f32,
    step: f32,
    repeats: u8,
}

impl DeathLight {
    pub fn new(step: f32) -> Self {
        Self { counter: -step, step, repeats: LIGHT_REPEATS }
    }

    /// One tick on; whether it still shows.
    pub fn step(&mut self) -> bool {
        self.counter += self.step;
        if self.counter >= LIGHT_END {
            if self.repeats == 0 {
                return false;
            }
            self.counter = 0.0;
            self.repeats -= 1;
        }
        true
    }

    /// The frame it shows.
    pub fn frame(&self) -> usize {
        self.counter.max(0.0) as usize
    }
}

/// A hero going out through an exit.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GoingOut {
    /// Fields left until it's gone; none yet before its first tick.
    fields: Option<i32>,
    /// What it starts with: 50, or 0 through a secret exit.
    start: i32,
    /// The floor it stood on.
    floor: f32,
}

impl GoingOut {
    /// Standing on `floor`; gone after `fields`.
    pub fn new(floor: f32, fields: i32) -> Self {
        Self { fields: None, start: fields, floor }
    }

    /// One 30 Hz tick, after the hero's light has stepped: the first lights
    /// it (returned, to put on the hero); then it sinks and spins (feet
    /// `position`, `facing`, `half_height`, `dt` seconds) until gone.
    /// Whether it's still there.
    pub fn tick(
        &mut self,
        position: &mut [f32; 3],
        facing: &mut f32,
        half_height: f32,
        dt: f32,
        light: &mut Option<DeathLight>,
    ) -> bool {
        let fields = self.fields.get_or_insert_with(|| {
            *light = Some(DeathLight::new(EXIT_LIGHT_STEP));
            self.start
        });
        *fields -= FIELDS_PER_TICK;
        if *fields < 1 {
            *fields = 0;
            return false;
        }
        if self.floor < BELOW + 2.0 * half_height + position[1] {
            position[1] -= SINK_PER_FIELD * FIELDS_PER_TICK as f32;
            *facing += SPIN * dt;
        }
        true
    }

    /// Gone: its time is up.
    pub fn gone(&self) -> bool {
        self.fields == Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF_HEIGHT: f32 = 2.5;
    const TICK: f32 = 1.0 / 30.0;

    #[test]
    fn a_hero_sinks_and_spins_through_the_floor_then_is_gone() {
        let mut out = GoingOut::new(10.0, FIELDS);
        let (mut at, mut facing, mut light) = ([0.0, 10.0, 0.0], 0.0, None);
        let mut ticks = 0;
        while out.tick(&mut at, &mut facing, HALF_HEIGHT, TICK, &mut light) {
            ticks += 1;
            assert!(light.is_some());
        }
        // 24 ticks of it, gone on the 25th (50 fields, 2 a tick).
        assert_eq!(ticks, 24);
        assert!(out.gone());
        // 0.24 a tick: its top (feet + 5) ends under the floor.
        assert!((at[1] - (10.0 - 24.0 * 0.24)).abs() < 1e-4, "{at:?}");
        assert!(at[1] + 2.0 * HALF_HEIGHT < 10.0);
        // 3π a second: 1.2 turns.
        assert!((facing - 24.0 * SPIN * TICK).abs() < 1e-4);
    }

    #[test]
    fn a_secret_exit_takes_the_hero_at_once() {
        let mut out = GoingOut::new(0.0, 0);
        let (mut at, mut facing, mut light) = ([0.0; 3], 0.0, None);
        assert!(!out.tick(&mut at, &mut facing, HALF_HEIGHT, TICK, &mut light));
        assert_eq!(at, [0.0; 3]);
        assert!(out.gone());
    }

    #[test]
    fn the_light_shows_each_frame_then_goes_round_once_more() {
        // The exit's: frame 0 from the first step, 2.5 ticks a frame.
        let mut light = DeathLight::new(EXIT_LIGHT_STEP);
        let mut frames = Vec::new();
        while light.step() {
            frames.push(light.frame());
        }
        assert_eq!(&frames[..4], &[0, 0, 0, 1]);
        // Round twice. Single-precision steps of 0.4 fall just short of 10
        // after 26, so each pass holds frame 9 a tick longer.
        assert_eq!((frames[25], frames[26]), (9, 0));
        assert_eq!(frames.iter().filter(|&&f| f == 9).count(), 6);
        assert_eq!(frames.len(), 52);
        // A boss level's end: 0.5 a step, two ticks a frame.
        let mut light = DeathLight::new(TELEPORT_LIGHT_STEP);
        let mut n = 0;
        while light.step() {
            n += 1;
        }
        assert_eq!(n, 40);
    }
}
