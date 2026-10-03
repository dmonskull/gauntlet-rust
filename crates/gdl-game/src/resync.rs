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
//! 1. each machine sends the host a **report**: the heroes as its game
//!    has them — where they stand, what they're doing, their records —
//!    and what its game has of the level;
//! 2. the host puts one game together: every hero from its own player's
//!    machine (so nobody's hero moves under them, and nobody loses what
//!    they saw their hero get or do), the rest of the level as the host
//!    has it, with whatever any machine's game has got further with —
//!    what was taken, opened, broken, let out, woken or killed there;
//! 3. every machine, the host too, takes that game over and plays on from
//!    the tick it stopped before.
//!
//! The wait is a round trip to the host and the few frames a level's
//! things take to come in; nothing starts again, and nobody is sent back.
//! Only machines whose games are on different levels can't be put
//! together: everyone then starts again on the level a game came to last.

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::prelude::*;
use gdl_net::Tick;
use serde::{Deserialize, Serialize};

use crate::audio::{VoiceQueues, VoicesSave};
use crate::breakables::BreakablesSave;
use crate::critters::CrittersSave;
use crate::hazards::{Hazards, HazardsSave};
use crate::items::ItemsSave;
use crate::level::LoadedGame;
use crate::loot::LootSave;
use crate::mechanics::{Mechanics, MechanicsSave};
use crate::message_box::{BoxSave, MessageBox};
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

/// `GDL_SYNC_AFTER=<ticks>` (testing what a late sync point must carry):
/// the host calls a sync point only this long after the games were found
/// to differ.
fn call_after() -> Tick {
    static AFTER: std::sync::OnceLock<Tick> = std::sync::OnceLock::new();
    *AFTER.get_or_init(|| std::env::var("GDL_SYNC_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

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
    /// The level this machine's game is on, and the tick it came to it.
    level: (String, Tick),
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
    /// The host: its own game at the point (and the tick it came to its
    /// level), and the reports as they come.
    state: Option<GameState>,
    entered: Tick,
    reports: Vec<(u8, Report)>,
    waited: f32,
}

/// What a machine tells the host at a sync point.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Report {
    /// The level it's on, and the tick its game came to it.
    level: String,
    entered: Tick,
    /// The heroes as its game has them: its own players' are taken as
    /// they are; the others' add what they got further with.
    heroes: Vec<HeroSave>,
    /// What its game has of the monsters.
    monsters: Option<MonstersSeen>,
    /// The level's items as it has them.
    items: Option<ItemsSave>,
    /// Its cameras (its own heroes' are taken).
    camera: Option<CameraSave>,
    /// The breakables it has standing.
    breakables: Option<BreakablesSave>,
    /// The critters as it has them: its statues woken, how hurt each
    /// critter is, how far into its death, a boss level's end.
    critters: Option<CrittersSave>,
    /// A boss's loot in flight.
    loot: LootSave,
    /// The message box it has up, whether a shop screen is, and its voice
    /// queues.
    boxes: BoxSave,
    shop: bool,
    voices: VoicesSave,
    /// The blasts it has in flight (their keys' sum).
    blasts: u64,
}

/// The game every machine takes over at a sync point.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct GameState {
    level: String,
    /// The host's game clock, in nanoseconds ([`set_clock`]).
    clock: u64,
    /// Every hero, each as its own player's machine has it.
    heroes: Vec<HeroSave>,
    /// The host's monsters, less what any machine's game has dead.
    monsters: Option<MonstersSave>,
    /// The host's items, with what any machine's heroes took or opened.
    items: Option<ItemsSave>,
    /// The host's critters, with what any machine's game has got further
    /// with (a statue woken, a critter dying or gone, a boss level's end).
    critters: Option<CrittersSave>,
    /// A boss's loot in flight on any machine.
    loot: LootSave,
    /// The host's cameras, each hero's own from its player's machine.
    camera: Option<CameraSave>,
    /// The host's triggers, movers, rotators and animated objects.
    mechanics: Option<MechanicsSave>,
    /// The breakables standing on every machine.
    breakables: Option<BreakablesSave>,
    /// The host's damage tiles and the heroes' guards against them.
    hazards: Option<HazardsSave>,
    /// The message box up: the host's, or failing that another machine's
    /// (play stops while one is up, so it's up for everyone).
    boxes: BoxSave,
    /// A shop screen stays up: only if it's up on every machine.
    shop: bool,
    /// The voice queues (a level's end waits on them): the host's, or
    /// failing lines there another machine's.
    voices: VoicesSave,
    /// The blasts in flight (the host's keys' sum), and whether every
    /// machine has the same: if not, none hurts any more.
    blasts: u64,
    blasts_differ: bool,
}

/// The game clock: the time the ticks read as now (the fixed loop's).
/// Every machine's starts at zero with the game and moves a tick at a
/// time, but a tick with a message box or a shop screen up doesn't move
/// it — so a box up on one machine alone leaves its clock behind for good,
/// and with it everything the game times by the clock.
fn clock(world: &World) -> u64 {
    world.resource::<Time<Fixed>>().elapsed().as_nanos() as u64
}

/// Sets the game clock (what's left over toward the next tick stays).
fn set_clock(world: &mut World, nanos: u64) {
    let elapsed = std::time::Duration::from_nanos(nanos);
    let mut fixed = world.resource_mut::<Time<Fixed>>();
    if fixed.elapsed() == elapsed {
        return;
    }
    let was = fixed.elapsed_secs_f64();
    info!("sync point: the game clock goes from {was:.3} s to {:.3} s", elapsed.as_secs_f64());
    let mut set = Time::<Fixed>::from_duration(fixed.timestep());
    set.advance_to(elapsed);
    set.accumulate_overstep(fixed.overstep());
    *fixed = set;
    crate::effects::shift_clock(world, elapsed.as_secs_f64() - was);
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
    let here = world.resource::<LoadedGame>().current_name().to_string();
    let mut resync = world.resource_mut::<Resync>();
    if let Some(since) = resync.since.as_mut() {
        *since += delta;
    }
    // Lockstep started again (a player joined at the tower): its ticks too.
    if resync.last.is_some_and(|last| now < last) {
        (resync.last, resync.strikes) = (None, 0);
    }
    // The level this machine's game is on, and since which tick.
    if resync.level.0 != here || now < resync.level.1 {
        resync.level = (here, now);
    }
    // The host's call: its controls carry it from a tick without it (the
    // point is the first tick with it) until the point comes.
    let differ = differ.is_some_and(|tick| synced.is_none_or(|at| tick >= at) && now >= tick + call_after());
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
        let (level, entered) = resync.level.clone();
        let mut point = Point { tick, state: None, entered, reports: Vec::new(), waited: 0.0 };
        let monsters = crate::monsters::save_synced(world);
        let items = crate::items::save_synced(world);
        let camera = crate::play_camera::save_synced(world);
        let breakables = Some(crate::breakables::save_synced(world));
        let critters = crate::critters::save_synced(world);
        let loot = crate::loot::save_synced(world);
        let boxes = world.resource::<MessageBox>().save_synced();
        let shop = world.resource::<crate::shop::ShopScreen>().is_open();
        let voices = world.resource::<VoiceQueues>().save_synced();
        let blasts = crate::effects::blasts_key(world);
        if host {
            let heroes = crate::player::save_synced(world);
            let mechanics = world.get_resource::<Mechanics>().map(Mechanics::save_synced);
            let hazards = world.get_resource::<Hazards>().map(Hazards::save_synced);
            let clock = clock(world);
            point.state = Some(GameState {
                level,
                clock,
                heroes,
                monsters,
                items,
                critters,
                loot,
                camera,
                mechanics,
                breakables,
                hazards,
                boxes,
                shop,
                voices,
                blasts,
                blasts_differ: false,
            });
        } else {
            // Its own players' heroes are theirs to say; its copies of the
            // others count for what they got further with here.
            let heroes = crate::player::save_synced(world);
            let report = Report {
                level,
                entered,
                heroes,
                monsters: monsters.map(|m| m.seen()),
                items,
                camera,
                breakables,
                critters,
                loot,
                boxes,
                shop,
                voices,
                blasts,
            };
            match ron::to_string(&report) {
                Ok(report) => {
                    debug!("online: sync report: {} bytes", report.len());
                    world.resource::<Online>().send_host(&Message::SyncReport { tick, report });
                }
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
    // (A machine that didn't report may have anything in flight.)
    state.blasts_differ = !all_in || point.reports.iter().any(|(_, r)| r.blasts != state.blasts);
    // Machines on another level can't be put together here: everyone
    // starts again (`frontend.rs`) on the level a machine went on to last
    // — a level one machine's game has finished isn't played again — each
    // hero with the record its own player's machine has.
    if point.reports.iter().any(|(_, r)| !r.level.eq_ignore_ascii_case(&state.level)) {
        let level = furthest_level((&state.level, point.entered), point.reports.iter().map(|(_, r)| (r.level.as_str(), r.entered)));
        warn!("online: sync point: the machines aren't on the same level; all go to {level}");
        online.restart_level = Some(level);
        merge_heroes(&mut state, &point.reports, &others);
        crate::player::load_records(world, &state.heroes);
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
    debug!("online: sync point's game: {} bytes", text.len());
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

/// Of the levels the machines' games are on (each with the tick its game
/// came to it), the one come to last: the host's unless another machine's
/// game went on later.
fn furthest_level<'a>(host: (&'a str, Tick), others: impl Iterator<Item = (&'a str, Tick)>) -> String {
    others.fold(host, |best, other| if other.1 > best.1 { other } else { best }).0.to_string()
}

/// One game from the host's and the machines' reports: each hero as its
/// own player's machine has it, keeping whatever the host's copy had got
/// further with (what only ever grows on a level).
fn merge(state: &mut GameState, reports: &[(u8, Report)], machines: &[(u8, Vec<u8>)]) {
    merge_heroes(state, reports, machines);
    for (machine, report) in reports {
        let Some((_, slots)) = machines.iter().find(|(m, _)| m == machine) else { continue };
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
        if let (Some(critters), Some(theirs)) = (&mut state.critters, &report.critters) {
            critters.keep_progress(theirs);
        }
        state.loot.keep_progress(&report.loot);
        if !state.boxes.busy() && report.boxes.busy() {
            state.boxes = report.boxes.clone();
        }
        state.shop &= report.shop;
        if !state.voices.busy() && report.voices.busy() {
            state.voices = report.voices.clone();
        }
    }
}

/// The heroes' part of [`merge`]: each from its own player's machine,
/// with what any other machine's copy of it had got further with (what
/// only grows: a realm won there marks every hero, a kill there earned
/// its experience).
fn merge_heroes(state: &mut GameState, reports: &[(u8, Report)], machines: &[(u8, Vec<u8>)]) {
    for (machine, report) in reports {
        let Some((_, slots)) = machines.iter().find(|(m, _)| m == machine) else { continue };
        for hero in &report.heroes {
            let Some(kept) = state.heroes.iter_mut().find(|h| h.slot == hero.slot) else { continue };
            if slots.contains(&hero.slot) {
                let mut own = hero.clone();
                own.record.keep_progress(&kept.record);
                *kept = own;
            } else {
                kept.record.keep_progress(&hero.record);
            }
        }
    }
}

/// What the last tick's systems left for the next to do — blows to land,
/// heroes to hurt or grab, missiles to launch, blasts to set off — is
/// dropped, on every machine alike: a machine alone in having one would
/// hurt, launch or blast what the others don't.
fn drop_pending(world: &mut World) {
    fn drop<M: bevy::ecs::message::Message>(world: &mut World) {
        if let Some(mut messages) = world.get_resource_mut::<Messages<M>>() {
            messages.clear();
        }
    }
    drop::<crate::combat::Hit>(world);
    drop::<crate::monsters::MonsterHit>(world);
    drop::<crate::monsters::DeathDrain>(world);
    drop::<crate::player_state::DamagePlayer>(world);
    drop::<crate::player_state::HurtHero>(world);
    drop::<crate::player_state::HealPlayer>(world);
    drop::<crate::player_state::SpendPower>(world);
    drop::<crate::player::GrabHero>(world);
    drop::<crate::player::ThrowHero>(world);
    drop::<crate::player::ReleaseHero>(world);
    drop::<crate::projectiles::HeroShot>(world);
    drop::<crate::projectiles::MonsterShot>(world);
    drop::<crate::breakables::BlastItem>(world);
    crate::effects::drop_pending(world);
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
    drop_pending(world);
    set_clock(world, state.clock);
    crate::projectiles::clear(world);
    if state.blasts_differ {
        crate::effects::spend_blasts(world);
    }
    crate::player::load_synced(world, &state.heroes);
    if let Some(monsters) = &state.monsters {
        crate::monsters::load_synced(world, monsters);
    }
    if let Some(items) = &state.items {
        crate::items::load_synced(world, items);
    }
    crate::loot::load_synced(world, &state.loot);
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
    if let (Some(save), Some(mut hazards)) = (&state.hazards, world.get_resource_mut::<Hazards>()) {
        hazards.load_synced(save);
    }
    world.resource_mut::<VoiceQueues>().load_synced(&state.voices);
    world.resource_mut::<MessageBox>().load_synced(&state.boxes);
    let mut shop = world.resource_mut::<crate::shop::ShopScreen>();
    if !state.shop && shop.is_open() {
        info!("sync point: the shop screen isn't up on every machine: it closes");
        shop.close();
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
    /// the host's copy had over it; a machine's copy of a hero it doesn't
    /// play adds only what grows (never its gold, health or place).
    #[test]
    fn a_hero_is_its_own_machines() {
        let mut state = GameState { level: "levelA1".into(), heroes: vec![hero(0, 10, 500), hero(1, 20, 900)], ..default() };
        let mut theirs = hero(0, 999, 800);
        theirs.record.realms_beaten = 0b100;
        let report = Report { level: "levelA1".into(), heroes: vec![hero(1, 75, 700), theirs], ..default() };
        merge(&mut state, &[(1, report)], &[(1, vec![1])]);
        let host = &state.heroes[0].record;
        assert_eq!((host.gold, host.experience, host.realms_beaten), (10, 800, 0b100));
        // The player's own gold; the experience at its most.
        assert_eq!((state.heroes[1].record.gold, state.heroes[1].record.experience), (75, 900));
    }

    /// Machines on different levels all go to the one a game came to last.
    #[test]
    fn the_level_gone_on_to_last_is_everyones() {
        // The client's game finished the boss level and is back at the
        // tower; the host's is still on it.
        assert_eq!(furthest_level(("levelB6", 0), [("levelL1", 1650)].into_iter()), "levelL1");
        // The host's went on; a client's hasn't yet.
        assert_eq!(furthest_level(("levelA2", 900), [("levelA1", 20), ("levelA2", 870)].into_iter()), "levelA2");
        // Both at once (different ways out): the host's.
        assert_eq!(furthest_level(("levelA2", 900), [("levelS1", 900)].into_iter()), "levelA2");
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
