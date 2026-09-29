//! Going to a level by name — what an exit does with its level code
//! (`docs/items.md`). Turned into the world's relative [`ChangeLevel`].

use bevy::prelude::*;

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
    game: Res<LoadedGame>,
    mut change: MessageWriter<ChangeLevel>,
) {
    // Several at once: the last one wins.
    let Some(ChangeLevelTo(name)) = requests.read().last() else { return };
    match game.levels.iter().position(|l| l.name.eq_ignore_ascii_case(name)) {
        Some(to) => {
            change.write(ChangeLevel(to as isize - game.current as isize));
        }
        None => warn!("no level named {name} to go to"),
    }
}
