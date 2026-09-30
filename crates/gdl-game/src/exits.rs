//! Going to a level by name — what an exit does with its level code
//! (`docs/items.md`). Turned into the world's relative [`ChangeLevel`].
//!
//! The game ends a level only once its voice queues are empty: the play
//! mode keeps the level running while a line is playing or waiting, and a
//! level's start waits for them too (`docs/frontend.md`, "The wait for the
//! voices"). So a level change waits here while the queues are busy — an
//! exit, a boss level's end, the last hero out.

use bevy::prelude::*;

use crate::audio::VoiceQueues;
use crate::level::LoadedGame;
use crate::world::ChangeLevel;

/// Loads the level with this folder name (`levelA6`), if the game has it.
#[derive(Message, Clone, Debug)]
pub struct ChangeLevelTo(pub String);

pub struct ExitsPlugin;

impl Plugin for ExitsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChangeLevelTo>().add_systems(Update, change_level_to);
    }
}

fn change_level_to(
    mut requests: MessageReader<ChangeLevelTo>,
    mut pending: Local<Option<String>>,
    voices: Res<VoiceQueues>,
    game: Res<LoadedGame>,
    mut change: MessageWriter<ChangeLevel>,
) {
    // Several at once: the last one wins.
    if let Some(ChangeLevelTo(name)) = requests.read().last() {
        if voices.busy() && pending.is_none() {
            info!("{name} waits for the voices");
        }
        *pending = Some(name.clone());
    }
    if voices.busy() {
        return;
    }
    let Some(name) = pending.take() else { return };
    match game.levels.iter().position(|l| l.name.eq_ignore_ascii_case(&name)) {
        Some(to) => {
            change.write(ChangeLevel(to as isize - game.current as isize));
        }
        None => warn!("no level named {name} to go to"),
    }
}
