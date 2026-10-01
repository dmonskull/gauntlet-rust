//! Going to a level by name — what an exit does with its level code
//! (`docs/items.md`). Turned into the world's relative [`ChangeLevel`].
//!
//! The game ends a level only once its voice queues are empty: the play
//! mode keeps the level running while a line is playing or waiting, and a
//! level's start waits for them too (`docs/frontend.md`, "The wait for the
//! voices"). So a level change waits here while the queues are busy — an
//! exit, a boss level's end, the last hero out.
//!
//! As the level ends with the hero still in it — through an exit, or at a
//! boss level's end — the level is finished: marked in the hero's record
//! (the next level's exits open, `quest.rs`) and kept as the last level
//! finished (the tower's speeches, `tower.rs`). Quitting the level or every
//! hero out finishes nothing (`docs/items.md`, "Exits").

use bevy::prelude::*;

use crate::audio::VoiceQueues;
use crate::level::LoadedGame;
use crate::party::Party;
use crate::quest;
use crate::tower::LevelTrail;
use crate::world::ChangeLevel;

/// Loads the level with this folder name (`levelA6`), if the game has it.
#[derive(Message, Clone, Debug)]
pub struct ChangeLevelTo {
    pub level: String,
    /// The heroes leave still in the level, which finishes it.
    pub finishing: bool,
}

impl ChangeLevelTo {
    /// Going without finishing the level: a game starting, Quit Level, the
    /// last hero out.
    pub fn to(level: impl Into<String>) -> Self {
        Self { level: level.into(), finishing: false }
    }

    /// The heroes leave through an exit, or at a boss level's end.
    pub fn finishing(level: impl Into<String>) -> Self {
        Self { level: level.into(), finishing: true }
    }
}

pub struct ExitsPlugin;

impl Plugin for ExitsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChangeLevelTo>().add_systems(Update, change_level_to);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn change_level_to(
    mut requests: MessageReader<ChangeLevelTo>,
    mut pending: Local<Option<ChangeLevelTo>>,
    voices: Res<VoiceQueues>,
    game: Res<LoadedGame>,
    mut party: ResMut<Party>,
    mut trail: ResMut<LevelTrail>,
    mut change: MessageWriter<ChangeLevel>,
    mut lock: ResMut<crate::online::Lockstep>,
) {
    // Several at once: the last one wins.
    if let Some(request) = requests.read().last() {
        if voices.busy() && pending.is_none() {
            info!("{} waits for the voices", request.level);
        }
        *pending = Some(request.clone());
    }
    if voices.busy() {
        return;
    }
    let Some(request) = pending.take() else { return };
    let Some(to) = game.levels.iter().position(|l| l.name.eq_ignore_ascii_case(&request.level)) else {
        warn!("no level named {} to go to", request.level);
        return;
    };
    let leaving = &game.levels[game.current].name;
    // Each player still in play gets the level marked.
    if request.finishing
        && party.any(|s| s.alive)
        && let Some((realm, level)) = quest::level_of(leaving)
        && realm != quest::TOWER
    {
        info!("{leaving} finished");
        for (_, state) in party.states_mut().filter(|(_, s)| s.alive) {
            state.quest.finish_level(realm, level);
        }
        trail.finished = Some(leaving.clone());
    }
    change.write(ChangeLevel(to as isize - game.current as isize));
    // Online the next tick waits for the level to settle (`online.rs`).
    lock.level_work = true;
}
