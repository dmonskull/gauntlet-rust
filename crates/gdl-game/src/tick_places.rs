//! Where the game has its things as a tick starts.
//!
//! The game ticks 30 times a second and is drawn as often as the screen
//! allows: between ticks, everything it moves is drawn part-way from where
//! the tick before left it to where the last one did ([`Between`]). The
//! tick's own systems read transforms too — a hero's aim finds monsters by
//! theirs, a boss's blows land where its bones are — and must find
//! everything where the game has it, not where it was last drawn: that
//! depends on when the machine drew its last frame, and online every
//! machine must play a tick alike (`docs/online.md`, "What a tick may
//! read"). So as each tick starts, the systems that place things for
//! drawing place them at the last tick's state ([`TickPlaces`]), and the
//! transforms go down to their bones.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use bevy::transform::systems::{mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms};

pub struct TickPlacesPlugin;

/// How far from the tick before's state (0) to the last tick's (1)
/// everything is placed: 1 as a tick starts, the part of a tick gone by
/// when a frame is drawn.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Between(pub f32);

/// As a tick starts (`FixedPreUpdate`): the systems that place what the
/// game moves, in two steps with the transforms propagated after each.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum TickPlaces {
    /// Everything the game moves by its own state.
    Movers,
    /// What hangs from one of those (a hero in a critter's grip).
    Riders,
}

impl Plugin for TickPlacesPlugin {
    fn build(&self, app: &mut App) {
        let propagate = || (mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms).chain();
        app.insert_resource(Between(1.0))
            .configure_sets(FixedPreUpdate, (TickPlaces::Movers, TickPlaces::Riders).chain())
            .add_systems(
                FixedPreUpdate,
                (
                    at_the_tick.before(TickPlaces::Movers),
                    propagate().after(TickPlaces::Movers).before(TickPlaces::Riders),
                    propagate().after(TickPlaces::Riders),
                ),
            )
            .add_systems(RunFixedMainLoop, between_ticks.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop));
    }
}

fn at_the_tick(mut between: ResMut<Between>) {
    between.0 = 1.0;
}

/// After the frame's ticks: how far into the next one the frame is drawn.
fn between_ticks(fixed: Res<Time<Fixed>>, mut between: ResMut<Between>) {
    between.0 = fixed.overstep_fraction();
}
