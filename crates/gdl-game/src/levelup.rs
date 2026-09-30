//! What the game does when experience moves the hero's level
//! (`docs/items.md`, "Experience"): a rise raises hint `0x22` ("LEVEL %d
//! EXPERIENCE", `S_GAINEDLEVEL`) and the colour's `LEVELUP_<colour>`
//! flash rides the hero (the +100 health is `PlayerState::add_experience`);
//! a fall (a Death's drain) has the announcer say the hero's name and
//! `S_LOSTLEVEL`. Once per change, however many levels it covers.
//!
//! The level is watched each tick: a level start and a new hero (a load,
//! a new class) take the level as they find it, without either, and so
//! does the first second after them (what a level start sets up lands
//! then: the tests' `GDL_EXPERIENCE`).

use bevy::prelude::*;

use crate::audio::QueueVoice;
use crate::character;
use crate::effects::EffectOn;
use crate::hints::{Hint, ShowHint};
use crate::player::{Player, PlayerChoice};
use crate::player_state::{PlayerState, PowersTick};
use crate::population::LevelPopulation;
use crate::tower_scenes::{LEVELUP_FLASHES, Rank};

pub struct LevelUpPlugin;

impl Plugin for LevelUpPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, watch_level.after(PowersTick));
    }
}

/// The colours' order (`PlayerChoice::variant`'s first three letters).
const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
/// The lost level's line after the hero's name, and the sentence's longest
/// wait (seconds).
const LOST_LEVEL: &str = "S_LOSTLEVEL";
const LOST_MOST_WAIT: f32 = 3.0;
/// Ticks after a level start or a new hero during which the level is
/// taken as it is.
const SETTLE_TICKS: u32 = 30;
/// The Pojo is named as itself.
const POJO: u32 = 0x400;
const POJO_NAME: &str = "S_POJO2";

/// The hero's colour index, from its variant.
fn colour_of(choice: &PlayerChoice) -> usize {
    let variant = choice.variant.to_ascii_uppercase();
    COLOURS.iter().position(|c| variant.starts_with(c)).unwrap_or(0)
}

/// What a change of level brings, if anything.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    Rose,
    Fell,
}

fn change(before: u32, now: u32) -> Option<Change> {
    match now.cmp(&before) {
        std::cmp::Ordering::Greater => Some(Change::Rose),
        std::cmp::Ordering::Less => Some(Change::Fell),
        std::cmp::Ordering::Equal => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn watch_level(
    state: Option<Res<PlayerState>>,
    choice: Option<Res<PlayerChoice>>,
    population: Option<Res<LevelPopulation>>,
    heroes: Query<Entity, With<Player>>,
    mut seen: Local<Option<(Entity, u32, u32)>>,
    mut hints: MessageWriter<ShowHint>,
    mut flashes: MessageWriter<EffectOn>,
    mut voices: MessageWriter<QueueVoice>,
) {
    let (Some(state), Some(choice)) = (state, choice) else { return };
    let Some(hero) = heroes.iter().next() else {
        *seen = None;
        return;
    };
    let fresh = state.is_added() || choice.is_changed() || population.as_ref().is_some_and(|p| p.is_changed());
    let (before, ticks) = match *seen {
        Some((e, level, ticks)) if e == hero && !fresh => (level, ticks + 1),
        _ => {
            debug!("watching {hero:?} from level {}", state.level);
            *seen = Some((hero, state.level, 0));
            return;
        }
    };
    *seen = Some((hero, state.level, ticks));
    if ticks < SETTLE_TICKS {
        return;
    }
    let colour = colour_of(&choice);
    match change(before, state.level) {
        Some(Change::Rose) => {
            info!("level {before} → {}: the level-up flash", state.level);
            hints.write(ShowHint(Hint::LevelUp));
            flashes.write(EffectOn { name: LEVELUP_FLASHES[colour], bank: None, on: hero, scale: 1.0 });
        }
        Some(Change::Fell) => {
            info!("level {before} → {}: lost", state.level);
            let name = if state.bits.special & POJO != 0 {
                POJO_NAME.to_string()
            } else {
                let class = character::class_index(&choice.class).unwrap_or(0);
                Rank { level: state.level, class, colour }.name_line()
            };
            voices.write(QueueVoice::announcer(name, LOST_MOST_WAIT).then(LOST_LEVEL));
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_of_level() {
        assert_eq!(change(10, 11), Some(Change::Rose));
        assert_eq!(change(10, 13), Some(Change::Rose));
        assert_eq!(change(10, 9), Some(Change::Fell));
        assert_eq!(change(10, 10), None);
    }

    #[test]
    fn colours_by_variant() {
        let c = |v: &str| colour_of(&PlayerChoice { class: "WAR".into(), variant: v.into() });
        assert_eq!(c("BLU"), 1);
        assert_eq!(c("gre30"), 3);
        assert_eq!(c("YEL"), 0);
    }
}
