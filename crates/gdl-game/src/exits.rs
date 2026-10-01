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
//!
//! Levels are known to the game by id — a realm and an index into the
//! realm WAD's level records, whose names are the folders — and the
//! records aren't always in the folders' order ([`LevelIds`]): the tower's
//! second castle portal (`a2`) leads to `levelA6`, and finishing `levelA6`
//! opens the third.

use bevy::prelude::*;
use gdl_formats::{LevelOrder, WorldData};

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

/// Which folder each of the game's level ids loads, from every realm WAD's
/// level records (`docs/level-population.md`, "Exit codes"): exits name
/// levels by id, and a finished level is marked by its id.
#[derive(Resource, Clone, Debug, Default)]
pub struct LevelIds(pub LevelOrder);

impl LevelIds {
    /// A level folder's realm and index (the tower's included).
    pub fn id(&self, folder: &str) -> Option<(u32, u32)> {
        self.0.id(folder)
    }
}

pub struct ExitsPlugin;

impl Plugin for ExitsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChangeLevelTo>().add_systems(Startup, load_level_ids).add_systems(Update, change_level_to);
    }
}

fn load_level_ids(mut commands: Commands, mut game: ResMut<LoadedGame>) {
    let mut order = LevelOrder::default();
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
            Ok(world) => order.add(&world),
            Err(why) => warn!("{path}: {why}"),
        }
    }
    commands.insert_resource(LevelIds(order));
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
    ids: Option<Res<LevelIds>>,
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
    // Each player still in play gets the level marked, by its id.
    let id = match ids.as_deref() {
        Some(ids) => ids.id(leaving),
        None => quest::level_of(leaving),
    };
    if request.finishing
        && party.any(|s| s.alive)
        && let Some((realm, level)) = id
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
