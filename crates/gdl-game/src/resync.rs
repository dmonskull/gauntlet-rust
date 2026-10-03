//! Online: putting the machines' games back together when they've come
//! apart, without starting the level again (`docs/online.md`, "Sync
//! points").
//!
//! Every machine runs the whole game on the same controls, and every 30
//! ticks they compare a hash of it. Should the hashes ever differ, the
//! host calls a **sync point**: its controls carry [`SlotInput::SYNC`],
//! and every machine stops before the first tick whose bundle has it —
//! the same tick everywhere, whatever the network does. Then:
//!
//! 1. each machine sends the host a **report**: its own players' heroes
//!    as its game has them — where they stand, what they're doing, their
//!    records — and what its game has of the level;
//! 2. the host puts one game together: every hero from its own player's
//!    machine (so nobody's hero moves under them, and nobody loses what
//!    they saw their hero get or do), the rest of the level as the host
//!    has it, with what any machine's heroes took or opened;
//! 3. every machine, the host too, takes that game over and plays on from
//!    the tick it stopped before.
//!
//! The wait is a round trip to the host and the few frames a level's
//! things take to come in; nothing starts again, and nobody is sent back.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use gdl_net::Tick;
use serde::{Deserialize, Serialize};

use crate::breakables::BreakablesSave;
use crate::critters::CrittersSave;
use crate::items::ItemsSave;
use crate::level::LoadedGame;
use crate::mechanics::{Mechanics, MechanicsSave};
use crate::monsters::{MonstersSave, MonstersSeen};
use crate::online::{COMMAND_FRAMES, LocalControls, Lockstep, Message, Online};
use crate::party::SlotInput;
use crate::play_camera::CameraSave;
use crate::player::HeroSave;

pub struct ResyncPlugin;

impl Plugin for ResyncPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Resync>().add_systems(
            RunFixedMainLoop,
            sync_point.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop).after(crate::online::drive),
        );
    }
}

/// The host waits this long for the machines' reports before it goes on
/// with its own copy of their heroes. (A machine in the game is at most
/// its network delay behind: its controls for the tick are already in.)
const REPORTS_WAIT: f32 = 2.0;

/// A sync point this soon (ticks) after the one before means that one
/// didn't hold: the games differ in something a sync point doesn't carry.
const HELD_FOR: Tick = 150;

/// The sync points' state on this machine.
#[derive(Resource, Default)]
pub struct Resync {
    /// The host: its controls are calling a sync point that hasn't come.
    calling: bool,
    point: Option<Point>,
    /// Sync points done this game.
    pub done: u32,
    /// The tick the last one came before, and how many in a row haven't
    /// held: the host waits longer and longer before the next, so games
    /// that can't be put together don't stop every second.
    last: Option<Tick>,
    strikes: u32,
    /// Seconds since the last sync point, for the screen (`lag_sign.rs`).
    pub since: Option<f32>,
}

impl Resync {
    /// Ticks the host leaves after the last sync point before it calls
    /// another: none at first, then 2 s doubling up to 30 s.
    fn wait(&self) -> Tick {
        match self.strikes {
            0 => 0,
            n => (60u32 << (n - 1).min(4)).min(900),
        }
    }

    /// A sync point is under way on this machine.
    pub fn under_way(&self) -> bool {
        self.point.is_some()
    }
}

/// The sync point under way.
struct Point {
    tick: Tick,
    /// The host: its own game at the point, and the reports as they come.
    state: Option<GameState>,
    reports: Vec<(u8, Report)>,
    waited: f32,
}

/// What a machine tells the host at a sync point.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Report {
    /// The level it's on.
    level: String,
    /// Its own players' heroes.
    heroes: Vec<HeroSave>,
    /// What its game has of the monsters.
    monsters: Option<MonstersSeen>,
    /// The level's items as it has them.
    items: Option<ItemsSave>,
    /// Its cameras (its own heroes' are taken).
    camera: Option<CameraSave>,
    /// The breakables it has standing.
    breakables: Option<BreakablesSave>,
}

/// The game every machine takes over at a sync point.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct GameState {
    level: String,
    /// Every hero, each as its own player's machine has it.
    heroes: Vec<HeroSave>,
    /// The host's monsters, less what any machine's game has dead.
    monsters: Option<MonstersSave>,
    /// The host's items, with what any machine's heroes took or opened.
    items: Option<ItemsSave>,
    /// The host's critters.
    critters: Option<CrittersSave>,
    /// The host's cameras, each hero's own from its player's machine.
    camera: Option<CameraSave>,
    /// The host's triggers, movers, rotators and animated objects.
    mechanics: Option<MechanicsSave>,
    /// The breakables standing on every machine.
    breakables: Option<BreakablesSave>,
}

/// Runs the sync points: the host's call when the games differ, then at
/// the point each machine's report, the host's game put together from
/// them, and every machine taking it over.
fn sync_point(world: &mut World) {
    let Some(online) = world.get_resource::<Online>() else { return };
    let (host, differ) = (online.host, online.desync);
    let lock = world.resource::<Lockstep>();
    let (on, now, sync, synced, host_held) = (lock.on, lock.tick, lock.sync, lock.synced, lock.last_held[0]);
    if !on {
        return;
    }
    let delta = world.resource::<Time<Real>>().delta_secs();
    let mut resync = world.resource_mut::<Resync>();
    if let Some(since) = resync.since.as_mut() {
        *since += delta;
    }
    // Lockstep started again (a player joined at the tower): its ticks too.
    if resync.last.is_some_and(|last| now < last) {
        (resync.last, resync.strikes) = (None, 0);
    }
    // The host's call: its controls carry it from a tick without it (the
    // point is the first tick with it) until the point comes.
    let differ = differ.is_some_and(|tick| synced.is_none_or(|at| tick >= at));
    let due = resync.last.is_none_or(|last| now >= last + resync.wait());
    let calling = resync.calling;
    if host && sync.is_none() && (calling || (differ && due && host_held & SlotInput::SYNC == 0)) {
        world.resource_mut::<Resync>().calling = true;
        let mut local = world.resource_mut::<LocalControls>();
        let bits = local.1.map_or(0, |(bits, _)| bits) | SlotInput::SYNC;
        local.1 = Some((bits, COMMAND_FRAMES));
    }
    let Some(tick) = sync else {
        world.resource_mut::<Resync>().point = None;
        return;
    };

    // The point begins: this machine's report (the host's whole game).
    let mut resync = std::mem::take(&mut *world.resource_mut::<Resync>());
    if resync.point.as_ref().is_none_or(|p| p.tick != tick) {
        resync.calling = false;
        if let Some((bits, frames)) = world.resource::<LocalControls>().1 {
            let rest = bits & !SlotInput::SYNC;
            world.resource_mut::<LocalControls>().1 = (rest != 0).then_some((rest, frames));
        }
        let level = world.resource::<LoadedGame>().current_name().to_string();
        let mut point = Point { tick, state: None, reports: Vec::new(), waited: 0.0 };
        let monsters = crate::monsters::save_synced(world);
        let items = crate::items::save_synced(world);
        let camera = crate::play_camera::save_synced(world);
        let breakables = Some(crate::breakables::save_synced(world));
        if host {
            let heroes = crate::player::save_synced(world, false);
            let critters = crate::critters::save_synced(world);
            let mechanics = world.get_resource::<Mechanics>().map(Mechanics::save_synced);
            point.state = Some(GameState { level, heroes, monsters, items, critters, camera, mechanics, breakables });
        } else {
            let heroes = crate::player::save_synced(world, true);
            let report = Report { level, heroes, monsters: monsters.map(|m| m.seen()), items, camera, breakables };
            match ron::to_string(&report) {
                Ok(report) => world.resource::<Online>().send_host(&Message::SyncReport { tick, report }),
                Err(e) => warn!("online: can't encode the sync report: {e}"),
            }
        }
        info!("online: sync point before tick {tick}");
        resync.point = Some(point);
    }
    let point = resync.point.as_mut().expect("the point just began");
    point.waited += world.resource::<Time<Real>>().delta_secs();

    let done = if host { host_point(world, point) } else { client_point(world, point) };
    if done {
        let waited = point.waited;
        resync.point = None;
        resync.done += 1;
        resync.strikes = if resync.last.is_some_and(|last| tick < last + resync.wait() + HELD_FOR) { resync.strikes + 1 } else { 0 };
        resync.last = Some(tick);
        resync.since = Some(0.0);
        world.resource_mut::<Lockstep>().sync_done();
        world.resource_mut::<Online>().desync = None;
        info!("online: the games are together again from tick {tick} ({:.0} ms)", waited * 1000.0);
    }
    *world.resource_mut::<Resync>() = resync;
}

/// The host at a sync point: once every machine in the game has reported
/// (or the wait is up), it puts the game together, sends it and takes it
/// over itself. Returns whether the point is done.
fn host_point(world: &mut World, point: &mut Point) -> bool {
    let in_game = world.resource::<Lockstep>().held_back();
    let mut online = world.resource_mut::<Online>();
    for (machine, tick, text) in std::mem::take(&mut online.sync_reports) {
        if tick != point.tick || point.reports.iter().any(|(m, _)| *m == machine) {
            continue;
        }
        match ron::from_str::<Report>(&text) {
            Ok(report) => point.reports.push((machine, report)),
            Err(e) => warn!("online: machine {machine}'s sync report doesn't read: {e}"),
        }
    }
    // The machines whose players are in the game at this tick.
    let others: Vec<(u8, Vec<u8>)> =
        online.others().into_iter().filter(|(_, slots)| slots.iter().any(|&s| in_game[usize::from(s)])).collect();
    let all_in = others.iter().all(|(machine, _)| point.reports.iter().any(|(m, _)| m == machine));
    if !all_in && point.waited < REPORTS_WAIT {
        return false;
    }
    if !all_in {
        warn!("online: sync point: not every machine reported; their heroes are the host's copies");
    }
    let Some(mut state) = point.state.take() else { return true };
    // Machines on another level can't be put together here: the level
    // starts again for everyone (`frontend.rs`).
    if point.reports.iter().any(|(_, r)| !r.level.eq_ignore_ascii_case(&state.level)) {
        warn!("online: sync point: the machines aren't on the same level");
        online.restart_level = true;
        world.resource_mut::<Lockstep>().sync_dropped();
        return false;
    }
    merge(&mut state, &point.reports, &others);
    let text = match ron::to_string(&state) {
        Ok(text) => text,
        Err(e) => {
            warn!("online: can't encode the game: {e}");
            return true;
        }
    };
    online.send(&Message::SyncState { tick: point.tick, state: text.clone() });
    // The host takes the game as the others will read it.
    match ron::from_str::<GameState>(&text) {
        Ok(state) => apply(world, &state),
        Err(e) => warn!("online: the game doesn't read back: {e}"),
    }
    true
}

/// A client at a sync point: it takes the host's game over when it comes.
fn client_point(world: &mut World, point: &mut Point) -> bool {
    let mut online = world.resource_mut::<Online>();
    let Some((tick, text)) = online.sync_state.take() else { return false };
    if tick != point.tick {
        // An earlier point's (this machine wasn't at it).
        return false;
    }
    match ron::from_str::<GameState>(&text) {
        Ok(state) => apply(world, &state),
        Err(e) => warn!("online: the host's game doesn't read: {e}"),
    }
    true
}

/// One game from the host's and the machines' reports: each hero as its
/// own player's machine has it, keeping whatever the host's copy had got
/// further with (what only ever grows on a level).
fn merge(state: &mut GameState, reports: &[(u8, Report)], machines: &[(u8, Vec<u8>)]) {
    for (machine, report) in reports {
        let Some((_, slots)) = machines.iter().find(|(m, _)| m == machine) else { continue };
        for hero in &report.heroes {
            if !slots.contains(&hero.slot) {
                continue;
            }
            let Some(kept) = state.heroes.iter_mut().find(|h| h.slot == hero.slot) else { continue };
            let mut own = hero.clone();
            own.record.keep_progress(&kept.record);
            *kept = own;
        }
        if let (Some(monsters), Some(seen)) = (&mut state.monsters, &report.monsters) {
            monsters.keep_progress(seen);
        }
        if let (Some(items), Some(theirs)) = (&mut state.items, &report.items) {
            items.keep_progress(theirs);
        }
        if let (Some(camera), Some(theirs)) = (&mut state.camera, &report.camera) {
            camera.keep_own(theirs, slots);
        }
        if let (Some(breakables), Some(theirs)) = (&mut state.breakables, &report.breakables) {
            breakables.keep_progress(theirs);
        }
    }
}

/// Every machine takes the game over, alike: what the point doesn't carry
/// starts afresh everywhere (missiles in flight go).
fn apply(world: &mut World, state: &GameState) {
    for h in &state.heroes {
        let r = &h.record;
        info!("sync point: player {} at {:?}: health {:.0}, gold {}, keys {}, experience {}", h.slot + 1, h.place(), r.health, r.gold, r.keys, r.experience);
    }
    if let Some((monsters, generators)) = state.monsters.as_ref().map(MonstersSave::counts) {
        info!("sync point: {monsters} monsters, {generators} generators");
    }
    crate::projectiles::clear(world);
    crate::player::load_synced(world, &state.heroes);
    if let Some(monsters) = &state.monsters {
        crate::monsters::load_synced(world, monsters);
    }
    if let Some(items) = &state.items {
        crate::items::load_synced(world, items);
    }
    if let Some(critters) = &state.critters {
        crate::critters::load_synced(world, critters);
    }
    if let Some(camera) = &state.camera {
        crate::play_camera::load_synced(world, camera);
    }
    if let (Some(save), Some(mut mechanics)) = (&state.mechanics, world.get_resource_mut::<Mechanics>()) {
        mechanics.load_synced(save);
    }
    if let Some(breakables) = &state.breakables {
        crate::breakables::load_synced(world, breakables);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_state::PlayerState;

    fn hero(slot: u8, gold: u32, experience: u32) -> HeroSave {
        let mut record = PlayerState::new("WAR", None);
        record.gold = gold;
        record.experience = experience;
        HeroSave::of(slot, record)
    }

    /// Each hero comes from its own player's machine, with the progress
    /// the host's copy had over it; a machine's word on a hero it doesn't
    /// play counts for nothing.
    #[test]
    fn a_hero_is_its_own_machines() {
        let mut state = GameState { level: "levelA1".into(), heroes: vec![hero(0, 10, 500), hero(1, 20, 900)], ..default() };
        let report = Report { level: "levelA1".into(), heroes: vec![hero(1, 75, 700), hero(0, 999, 999)], ..default() };
        merge(&mut state, &[(1, report)], &[(1, vec![1])]);
        assert_eq!((state.heroes[0].record.gold, state.heroes[0].record.experience), (10, 500));
        // The player's own gold; the experience at its most.
        assert_eq!((state.heroes[1].record.gold, state.heroes[1].record.experience), (75, 900));
    }

    #[test]
    fn the_game_reads_back_as_sent() {
        let mut h = hero(2, 3, 4);
        h.record.health = 0.1 + 0.2;
        let state = GameState { level: "levelB6".into(), heroes: vec![h], ..default() };
        let text = ron::to_string(&state).unwrap();
        let back: GameState = ron::from_str(&text).unwrap();
        assert_eq!(back.heroes[0].record.health.to_bits(), state.heroes[0].record.health.to_bits());
        assert_eq!(ron::to_string(&back).unwrap(), text);
    }
}
