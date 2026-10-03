//! Online co-op in the game (`docs/online.md`, "In the game"): up to four
//! players on their own machines over `gdl-net`, each with their own
//! screen, camera and settings, the game itself in deterministic lockstep.
//!
//! - **Hosting** brings a session up off the main thread (it waits a few
//!   seconds for a relay) and copies its invite code to the clipboard;
//!   **joining** takes the code from the clipboard.
//! - **The lobby** is the select screen (`frontend.rs`): each machine
//!   drives its own column — a new hero, or one saved on that machine —
//!   and the other columns show what their players pick ([`Message`]).
//!   With every player ready the host starts the game: it sends everyone
//!   the party ([`Message::Begin`]), and every machine builds the same
//!   party and loads the tower.
//! - **In play** every machine runs the whole game ([`Lockstep`]). A tick
//!   runs only once the host's bundle of every player's controls for it is
//!   in, at most one a frame, and never while a level change is settling:
//!   every machine runs the same ticks on the same controls, and the
//!   systems that run each frame see the same game between ticks. The
//!   game's clock (`Time<Virtual>`) moves with the ticks run. Each tick
//!   also runs [`NetTick`]: the message box (it freezes play for everyone,
//!   and any player puts its page away), the voice queues, the players who
//!   left, and every 30 ticks a hash of the game state the machines
//!   compare.
//! - **Starting again**: the host sends the party as its game has it and
//!   where to start ([`Message::Resync`]), and lockstep restarts from tick
//!   0 — for a player joining the game under way (at the tower), a player's
//!   changed hero (Manage Character) or the machines out of sync (the level
//!   under way).
//! - Each machine's camera follows its own hero, a teammate's while its
//!   own is out (`play_camera.rs`), or the host picks the shared co-op one.

use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use bevy::app::{RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy::ecs::schedule::{ExecutorKind, ScheduleLabel};
use bevy::prelude::*;
use gdl_formats::detmath::sync_bits;
use gdl_net::{Bundle, NetConfig, NetEvent, NetSession, PlayerInput, Relays, Target, Tick};
use serde::{Deserialize, Serialize};

use crate::level::LoadedGame;
use crate::party::{Inputs, MAX_PLAYERS, Party, SlotInput};
use crate::population::LevelPopulation;
use crate::saves::SavedCharacter;

/// The game's own lockstep revision, part of the build every machine must
/// share: raise it whenever the game steps differently.
const LOCKSTEP_REVISION: u32 = 4;

/// Frames without level work before the next tick may run: a level change
/// and its setup (systems that run as its population comes in, then as
/// their own resources do) take a few.
const SETTLE_FRAMES: u32 = 4;

/// Real time owed to the 30 Hz schedule is kept to this many ticks (a
/// machine behind catches up a tick a frame).
const MOST_OWED: u32 = 4;

/// After this long waiting on the network the screen says who for.
pub const WAIT_SHOWN: f32 = 0.5;

/// A machine that says nothing this long has left (its game crashed or its
/// network went): the others go on without it. A player who leaves or
/// quits says goodbye and is gone at once. The network thread answers
/// every 250 ms however busy the game is, so only a dead machine or
/// connection is this quiet.
const SILENT_LEAVES: Duration = Duration::from_secs(4);

pub struct OnlinePlugin;

impl Plugin for OnlinePlugin {
    fn build(&self, app: &mut App) {
        // Every machine runs a tick's systems in the same order: on one
        // thread, in the schedule's own order. (Several threads would run
        // the systems not ordered among themselves in whatever order they
        // came free, which can differ from machine to machine.)
        for schedule in [FixedUpdate.intern(), NetTick.intern()] {
            app.edit_schedule(schedule, |s| {
                s.set_executor_kind(ExecutorKind::SingleThreaded);
            });
        }
        app.init_resource::<Lockstep>()
            .init_resource::<LocalControls>()
            .init_schedule(NetTick)
            .add_message::<NewGame>()
            .add_systems(PreUpdate, poll)
            .add_systems(Update, fresh_game)
            .add_systems(RunFixedMainLoop, drive.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop))
            .add_systems(RunFixedMainLoop, run_net_tick.in_set(RunFixedMainLoopSystems::AfterFixedMainLoop))
            .add_systems(NetTick, (leave_gone, net_shop.after(leave_gone), test_desync.before(checksum), checksum.after(net_shop)))
            .add_systems(Last, settle);
    }
}

/// A network tick's own work, after the game's tick (or in place of it
/// while the message box freezes play).
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct NetTick;

/// A new game begins (online, every machine at once): what the game keeps
/// from level to level starts afresh, so every machine starts alike.
#[derive(Message, Clone, Copy, Debug, Default)]
pub struct NewGame;

/// Whether the game runs on the network's ticks, and where they are.
#[derive(Resource, Default)]
pub struct Lockstep {
    /// Online, from the game's start: ticks run only from bundles.
    pub on: bool,
    /// A network tick runs this frame (the game's, or a frozen one).
    pub ticked: bool,
    /// The game's own tick runs this frame (play isn't frozen).
    pub stepped: bool,
    /// The next tick.
    pub tick: Tick,
    /// Each slot's player is in this tick's bundle.
    pub present: [bool; MAX_PLAYERS],
    /// Each slot's controls last tick (presses are what's new).
    pub last_held: [u32; MAX_PLAYERS],
    /// Frames since level work (`level_work`, a population coming in).
    quiet: u32,
    /// Set by whatever changes level this frame (`exits.rs`, `world.rs`).
    pub level_work: bool,
    /// The front end holds the game (it's loading, not in play).
    pub held: bool,
    /// Real time owed to the 30 Hz schedule.
    owed: Duration,
    /// Real seconds the next tick has waited on the network.
    pub waited: f32,
    /// The clock was started afresh for this game.
    clock_reset: bool,
}

impl Lockstep {
    /// Starts lockstep for a new game: tick 0 next, the game's clocks from
    /// zero.
    pub fn begin(&mut self) {
        *self = Self { on: true, ..default() };
    }

    /// Whether a player pressed (newly held) any of `bits` this tick.
    pub fn pressed(&self, inputs: &Inputs, bits: u32) -> bool {
        (0..MAX_PLAYERS).any(|s| inputs.slots[s].held & bits & !self.last_held[s] != 0)
    }
}

/// Whether the game runs on the network's ticks (online play).
pub fn lockstep_on(lock: Res<Lockstep>) -> bool {
    lock.on
}

/// Whether the game runs on its own clock (alone, or local co-op).
pub fn lockstep_off(lock: Res<Lockstep>) -> bool {
    !lock.on
}

/// A game message between the machines (control messages, in order).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)] // a handful, in the lobby
pub enum Message {
    /// A player's select column as they pick, for the others' screens.
    Column { slot: u8, show: ColumnShow },
    /// A player's hero as they're ready with it; `None`: not ready.
    Hero { slot: u8, hero: Option<Hero> },
    /// The host starts the game with this party.
    Begin { heroes: Vec<(u8, Hero)> },
    /// The host starts again from the same place everywhere: this party
    /// (everyone's record as the host has it, players who joined since
    /// too), on `level` (the tower when someone joins or changes hero; the
    /// level under way after the machines went out of sync). Lockstep
    /// restarts right after.
    Resync { heroes: Vec<(u8, Hero)>, level: String },
    /// The host skipped `level`'s opening movie: it ends everywhere
    /// (`level_intro.rs`).
    SkipMovie { level: String },
}

/// A hero a player brings: a new one, or one saved on their machine (its
/// record goes along, so every machine plays it alike).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Hero {
    pub class: String,
    pub variant: String,
    pub name: String,
    pub saved: Option<SavedCharacter>,
}

/// What a select column shows, for a remote player's column.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColumnShow {
    pub step: u8,
    pub class: u8,
    pub colour: u8,
    pub name: String,
    pub letter: u8,
    pub selected: u8,
    pub ready: bool,
}

/// What the lobby hears, for the front end to act on.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // a handful, in the lobby
pub enum Lobby {
    /// A player came in (their column opens).
    Joined { slot: usize },
    /// A player left (their column closes).
    Left { slot: usize },
    Column { slot: usize, show: ColumnShow },
    Hero { slot: usize, hero: Option<Hero> },
    Begin(Vec<(usize, Hero)>),
    Resync(Vec<(usize, Hero)>, String),
}

/// Where the session is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Joining: waiting for the host's welcome.
    Joining,
    /// In the lobby (the select screen).
    Lobby,
    /// The game has begun.
    Playing,
    /// The session ended (why).
    Over(String),
}

/// The session: the host's or a client's.
#[derive(Resource)]
pub struct Online {
    session: NetSession,
    pub host: bool,
    /// The invite code (the host's; a client's, the one it joined with), and
    /// whether the host's went to the clipboard.
    pub invite: Option<String>,
    pub copied: bool,
    /// The slot this machine plays, once it's in.
    pub me: Option<usize>,
    pub phase: Phase,
    /// What came for the lobby.
    pub inbox: Vec<Lobby>,
    /// Each machine in the game and its slots.
    peers: Vec<(u8, Vec<u8>)>,
    /// A machine joined: this one sends its column and hero again.
    pub resend: bool,
    /// The machines' checksums first differed at this tick.
    pub desync: Option<Tick>,
    /// The host has started lockstep (tick 0 may run).
    started: bool,
    /// Short notices for the screen ("Player 2 left"), seconds left.
    pub notices: Vec<(String, f32)>,
    /// Joined while the game was under way: playing from the host's next
    /// resync.
    pub late: bool,
    /// A resync is on its way: no tick runs until lockstep restarts.
    pub awaiting_restart: bool,
    /// The host: players who joined late and are ready, waiting for the
    /// tower.
    pub joiners: Vec<(usize, Hero)>,
    /// The level whose opening movie the host skipped (`level_intro.rs`
    /// takes it).
    pub skip_movie: Option<String>,
}

impl Online {
    fn new(session: NetSession, host: bool, invite: Option<String>) -> Self {
        Self {
            session,
            host,
            invite,
            copied: false,
            me: None,
            phase: if host { Phase::Lobby } else { Phase::Joining },
            inbox: Vec::new(),
            peers: Vec::new(),
            resend: false,
            desync: None,
            started: false,
            notices: Vec::new(),
            late: false,
            awaiting_restart: false,
            joiners: Vec::new(),
            skip_movie: None,
        }
    }

    /// The host starts again from this party on `level`, everywhere.
    pub fn resync(&mut self, heroes: &[(usize, Hero)], level: &str) {
        let heroes = heroes.iter().map(|(s, h)| (*s as u8, h.clone())).collect();
        self.send(&Message::Resync { heroes, level: level.to_string() });
        self.awaiting_restart = true;
        if let Err(e) = self.session.restart() {
            warn!("online: can't restart: {e}");
        }
    }

    /// Sends a message to every other machine.
    pub fn send(&self, m: &Message) {
        match ron::to_string(m) {
            Ok(text) => self.session.send_control(Target::All, text.into_bytes()),
            Err(e) => warn!("online: can't encode {m:?}: {e}"),
        }
    }

    /// The host starts the game: everyone gets the party. Players joining
    /// after this come in at the host's next resync.
    pub fn start_game(&mut self, heroes: &[(usize, Hero)]) {
        let heroes = heroes.iter().map(|(s, h)| (*s as u8, h.clone())).collect();
        self.send(&Message::Begin { heroes });
        if let Err(e) = self.session.start() {
            warn!("online: can't start: {e}");
        }
    }

    /// The machines the next tick waits for, by their players' slots.
    pub fn waiting_slots(&self) -> Vec<usize> {
        let waiting = self.session.waiting_for();
        self.peers
            .iter()
            .filter(|(peer, _)| waiting.contains(peer))
            .flat_map(|(_, slots)| slots.iter().map(|&s| s as usize))
            .collect()
    }

    /// A short notice on the screen.
    pub fn notice(&mut self, text: impl Into<String>) {
        self.notices.push((text.into(), 4.0));
    }
}

/// Hosting under way: the session comes up on another thread.
#[derive(Resource)]
pub struct Opening {
    rx: Mutex<Receiver<Result<(NetSession, String), String>>>,
}

/// Why going online didn't work, for the front end to show.
#[derive(Resource, Clone, Debug)]
pub struct OnlineFailed(pub String);

/// The build every machine must share: the game's version, its lockstep
/// revision and the game disc's id.
fn game_version(game: &LoadedGame) -> String {
    // The revision too: a disc whose files differ would part the games.
    let disc = game.install.game_id.as_deref().unwrap_or("unknown");
    let revision = game.install.revision.map_or_else(|| "?".to_string(), |r| r.to_string());
    format!("gdl-game {} lockstep {LOCKSTEP_REVISION} disc {disc} rev {revision}", env!("CARGO_PKG_VERSION"))
}

/// The session's settings. `GDL_NET_LOCAL=1` (testing: two games on one
/// machine) keeps it to this machine's loopback, without relays.
fn config(game: &LoadedGame) -> NetConfig {
    let mut cfg = NetConfig {
        name: "Player".into(),
        local_players: 1,
        game_version: game_version(game),
        timeout: SILENT_LEAVES,
        ..default()
    };
    if std::env::var("GDL_NET_LOCAL").is_ok_and(|v| !v.is_empty() && v != "0") {
        cfg.relays = Relays::Disabled;
        cfg.bind = Some(std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
    }
    cfg
}

/// Starts hosting: the front end shows it's under way until [`Online`]
/// (or [`OnlineFailed`]) comes.
pub fn host(commands: &mut Commands, game: &LoadedGame) {
    let (tx, rx) = channel();
    let cfg = config(game);
    std::thread::Builder::new()
        .name("gdl-host".into())
        .spawn(move || {
            let _ = tx.send(NetSession::host(cfg).map_err(|e| e.to_string()));
        })
        .expect("spawn the hosting thread");
    commands.remove_resource::<OnlineFailed>();
    commands.insert_resource(Opening { rx: Mutex::new(rx) });
}

/// Joins the game an invite code names.
pub fn join(commands: &mut Commands, game: &LoadedGame, code: &str) -> Result<(), String> {
    let session = NetSession::join(code, config(game)).map_err(|e| e.to_string())?;
    commands.remove_resource::<OnlineFailed>();
    commands.insert_resource(Online::new(session, false, Some(code.to_string())));
    Ok(())
}

/// Leaves the online game (telling the others) and gives the game its own
/// clock back.
pub fn leave(commands: &mut Commands, lock: &mut Lockstep) {
    commands.remove_resource::<Online>();
    commands.remove_resource::<Opening>();
    *lock = Lockstep::default();
}

/// `GDL_INVITE_FILE` (testing): the invite goes to and comes from this
/// file instead of the clipboard.
fn invite_file() -> Option<std::path::PathBuf> {
    std::env::var_os("GDL_INVITE_FILE").filter(|v| !v.is_empty()).map(Into::into)
}

/// Puts the invite code on the clipboard.
pub fn copy_invite(code: &str) -> bool {
    if let Some(path) = invite_file() {
        return std::fs::write(path, code).is_ok();
    }
    arboard::Clipboard::new().and_then(|mut c| c.set_text(code.to_string())).is_ok()
}

/// An invite code from the clipboard, if it holds one.
pub fn paste_invite() -> Option<String> {
    let text = match invite_file() {
        Some(path) => std::fs::read_to_string(path).ok()?,
        None => arboard::Clipboard::new().ok()?.get_text().ok()?,
    };
    clean_invite(&text)
}

/// An invite code as pasted: spaces and line breaks dropped; `None` if it
/// isn't one.
fn clean_invite(text: &str) -> Option<String> {
    let code: String = text.split_whitespace().collect();
    code.to_ascii_uppercase().starts_with("GDL").then_some(code)
}

/// A stick axis to the wire's byte and back: every machine plays the
/// rounded value, the one that sent it too.
fn quantize(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

fn dequantize(q: i8) -> f32 {
    (f32::from(q) / 127.0).clamp(-1.0, 1.0)
}

pub fn to_net(i: &SlotInput) -> PlayerInput {
    PlayerInput {
        stick: [quantize(i.stick.x), quantize(i.stick.y)],
        c_stick: [quantize(i.c_stick.x), quantize(i.c_stick.y)],
        buttons: i.held,
    }
}

pub fn from_net(p: &PlayerInput) -> SlotInput {
    SlotInput {
        stick: Vec2::new(dequantize(p.stick[0]), dequantize(p.stick[1])),
        c_stick: Vec2::new(dequantize(p.c_stick[0]), dequantize(p.c_stick[1])),
        held: p.buttons,
    }
}

/// Hosting's result, and what the session says.
fn poll(mut commands: Commands, opening: Option<Res<Opening>>, online: Option<ResMut<Online>>, lock: Res<Lockstep>) {
    if let Some(opening) = opening {
        let got = opening.rx.lock().ok().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = got {
            commands.remove_resource::<Opening>();
            match result {
                Ok((session, invite)) => {
                    info!("online: hosting; invite {invite}");
                    let mut online = Online::new(session, true, Some(invite.clone()));
                    online.copied = copy_invite(&invite);
                    commands.insert_resource(online);
                }
                Err(e) => {
                    warn!("online: can't host: {e}");
                    commands.insert_resource(OnlineFailed(e));
                }
            }
        }
    }
    let Some(mut online) = online else { return };
    let online = &mut *online;
    for (_, left) in &mut online.notices {
        *left -= 1.0 / 60.0;
    }
    online.notices.retain(|(_, left)| *left > 0.0);
    // The host's code can change (a relay found late).
    if online.host
        && let Some(code) = online.session.invite()
        && online.invite.as_ref() != Some(&code)
    {
        online.copied = copy_invite(&code);
        online.invite = Some(code);
    }
    for event in online.session.poll_events() {
        match event {
            NetEvent::Connected { you, slots, late } => {
                info!("online: in as machine {you}, slots {slots:?}{}", if late { " (the game is under way)" } else { "" });
                online.late = late;
                online.me = slots.first().map(|&s| s as usize);
                online.peers.retain(|(p, _)| *p != you);
                online.peers.push((you, slots));
                for p in online.session.roster() {
                    if p.peer != you {
                        online.peers.retain(|(q, _)| *q != p.peer);
                        for &s in &p.slots {
                            online.inbox.push(Lobby::Joined { slot: s as usize });
                        }
                        online.peers.push((p.peer, p.slots));
                    }
                }
                if online.phase == Phase::Joining {
                    online.phase = Phase::Lobby;
                }
                online.resend = true;
            }
            NetEvent::Failed(why) => {
                warn!("online: joining failed: {why}");
                online.phase = Phase::Over(why);
            }
            NetEvent::PeerJoined { peer, name, slots } => {
                info!("online: {name} (machine {peer}) joined, slots {slots:?}");
                for &s in &slots {
                    online.inbox.push(Lobby::Joined { slot: s as usize });
                }
                online.peers.retain(|(p, _)| *p != peer);
                online.peers.push((peer, slots));
                online.resend = true;
            }
            NetEvent::PeerLeft { peer, from_tick } => {
                let slots = online.peers.iter().find(|(p, _)| *p == peer).map(|(_, s)| s.clone()).unwrap_or_default();
                info!("online: machine {peer} left (slots {slots:?}, from tick {from_tick:?})");
                online.peers.retain(|(p, _)| *p != peer);
                for &s in &slots {
                    online.notice(format!("Player {} left", s + 1));
                    // In play the bundles say when (`leave_gone`).
                    if !lock.on {
                        online.inbox.push(Lobby::Left { slot: s as usize });
                    }
                }
            }
            NetEvent::Started { delay } => {
                info!("online: lockstep starts (input delay {delay})");
                online.started = true;
            }
            NetEvent::Restarted { delay } => {
                info!("online: lockstep starts again (input delay {delay})");
                online.started = true;
                online.awaiting_restart = false;
                online.late = false;
                online.desync = None;
            }
            NetEvent::DelayChanged { delay } => info!("online: input delay now {delay} ticks"),
            NetEvent::Control { from, bytes } => {
                let decoded = std::str::from_utf8(&bytes).ok().and_then(|t| ron::from_str::<Message>(t).ok());
                match decoded {
                    Some(Message::Column { slot, show }) => online.inbox.push(Lobby::Column { slot: slot as usize, show }),
                    Some(Message::Hero { slot, hero }) => online.inbox.push(Lobby::Hero { slot: slot as usize, hero }),
                    Some(Message::Begin { heroes }) => {
                        online.inbox.push(Lobby::Begin(heroes.into_iter().map(|(s, h)| (s as usize, h)).collect()));
                    }
                    Some(Message::Resync { heroes, level }) => {
                        // The restart follows on the same stream: no tick
                        // until it comes.
                        online.awaiting_restart = true;
                        online.inbox.push(Lobby::Resync(heroes.into_iter().map(|(s, h)| (s as usize, h)).collect(), level));
                    }
                    Some(Message::SkipMovie { level }) => online.skip_movie = Some(level),
                    None => warn!("online: machine {from} sent something this build doesn't read"),
                }
            }
            NetEvent::Desync { tick } => {
                error!("online: the machines' games differ from tick {tick}");
                if online.desync.is_none() {
                    // The host starts the level again (`frontend.rs`).
                    online.notice("Out of sync, the level starts again");
                }
                online.desync.get_or_insert(tick);
            }
            NetEvent::Disconnected(why) => {
                warn!("online: disconnected: {why}");
                online.phase = Phase::Over(why);
            }
        }
    }
}

/// This machine's player's controls (`player.rs`), sent every frame, and a
/// command from its menus (`SlotInput::OPEN_SHOP` …) held on them for a
/// few frames so a tick takes it.
#[derive(Resource, Default, Clone, Copy)]
pub struct LocalControls(pub SlotInput, pub Option<(u32, u8)>);

/// Frames a menu command rides with the controls.
pub const COMMAND_FRAMES: u8 = 12;

/// Online, before the fixed loop: sends this machine's controls, takes the
/// next tick's bundle when it's due and the game is settled, and moves the
/// game's clock so the fixed loop runs exactly that tick.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive(
    mut lock: ResMut<Lockstep>,
    online: Option<Res<Online>>,
    real: Res<Time<Real>>,
    (mut virt, mut fixed): (ResMut<Time<Virtual>>, ResMut<Time<Fixed>>),
    mut inputs: ResMut<Inputs>,
    local: Res<LocalControls>,
    mut mouse: ResMut<crate::first_person::MouseLook>,
    (boxes, shop): (Res<crate::message_box::MessageBox>, Res<crate::shop::ShopScreen>),
) {
    lock.ticked = false;
    lock.stepped = false;
    if !lock.on {
        return;
    }
    let Some(online) = online else { return };
    if !virt.is_paused() {
        virt.pause();
    }
    if !lock.clock_reset {
        // Every machine's game clock starts at zero with the game.
        lock.clock_reset = true;
        let step = fixed.timestep();
        *fixed = Time::<Fixed>::from_duration(step);
    }
    if let Some(me) = online.me {
        online.session.set_local_input(me as u8, to_net(&local.0));
    }
    let step = fixed.timestep();
    lock.owed = (lock.owed + real.delta()).min(step * MOST_OWED);
    let settled = lock.quiet >= SETTLE_FRAMES && !lock.held;
    let mut run = 0;
    // A restart's ticks wait for the front end to start the game again
    // from the host's resync, which may come in the same frame.
    let resyncing = online.inbox.iter().any(|m| matches!(m, Lobby::Resync(..)));
    let started = online.started && !online.awaiting_restart && !resyncing;
    if started && settled && lock.owed >= step {
        match online.session.ready_inputs(lock.tick) {
            Some(bundle) => {
                // ready_inputs commits this sample for a future tick only
                // when it hands out a bundle. Retries keep the mouse pixels.
                mouse.take(step.as_secs_f32());
                take_bundle(&mut lock, &mut inputs, &bundle);
                lock.ticked = true;
                lock.owed -= step;
                lock.waited = 0.0;
                // The message box and the shop screens freeze play: the
                // tick is theirs alone.
                if !boxes.is_open() && !shop.is_open() {
                    lock.stepped = true;
                    run = 1;
                }
            }
            None => lock.waited += real.delta_secs(),
        }
    } else if started && settled {
        lock.waited = 0.0;
    }
    // The clock: the tick to run, then how far into the next one it is,
    // for drawing between ticks. While play is frozen or waiting, no time
    // passes.
    let between = if lock.ticked && run == 0 {
        fixed.overstep()
    } else {
        step.mul_f32((lock.owed.as_secs_f32() / step.as_secs_f32()).min(0.99))
    };
    let target = step * run + between;
    let advance = target.saturating_sub(fixed.overstep());
    virt.advance_by(advance);
}

/// A bundle into this tick's controls; a slot nobody plays has none.
fn take_bundle(lock: &mut Lockstep, inputs: &mut Inputs, bundle: &Bundle) {
    for (slot, input) in bundle.iter().enumerate() {
        lock.present[slot] = input.is_some();
        inputs.slots[slot] = input.as_ref().map(from_net).unwrap_or_default();
    }
}

/// After the fixed loop: a network tick's own work ([`NetTick`]).
fn run_net_tick(world: &mut World) {
    if !world.resource::<Lockstep>().ticked {
        return;
    }
    world.run_schedule(NetTick);
    let held: [u32; MAX_PLAYERS] = {
        let inputs = world.resource::<Inputs>();
        std::array::from_fn(|s| inputs.slots[s].held)
    };
    let mut lock = world.resource_mut::<Lockstep>();
    lock.last_held = held;
    lock.tick += 1;
}

/// A player whose machine left is out of the game from the first tick
/// without their controls: their hero goes.
fn leave_gone(
    lock: Res<Lockstep>,
    mut party: ResMut<Party>,
    heroes: Query<(Entity, &crate::player::Player)>,
    mut commands: Commands,
) {
    for slot in 0..MAX_PLAYERS {
        if lock.present[slot] || party.get(slot).is_none() {
            continue;
        }
        if let Some(m) = party.leave(slot) {
            info!("online: player {} ({}) left at tick {}", slot + 1, m.name, lock.tick);
        }
        for (e, p) in &heroes {
            if p.slot == slot {
                commands.entity(e).despawn();
            }
        }
    }
}

/// Online the shop screens run on the network's ticks with each player's
/// controls from the bundle: up and down (the D-pad or the stick), A
/// buys, X sells, B goes to EXIT. A player's Tower Menu command opens the
/// Shop or Inventory for everyone; when the screen ends the points bought
/// go back on the heroes and play goes on in the tower.
#[allow(clippy::too_many_arguments)]
fn net_shop(
    lock: Res<Lockstep>,
    inputs: Res<Inputs>,
    mut shop: ResMut<crate::shop::ShopScreen>,
    data: Option<Res<crate::shop::ShopData>>,
    mut party: ResMut<Party>,
    population: Option<Res<LevelPopulation>>,
    mut sticks: Local<[(bool, bool); MAX_PLAYERS]>,
) {
    use crate::combat::button;
    use crate::shop::{ShopKind, ShopOpen, ShopOutcome, ShopPress};
    let pressed = |slot: usize, bits: u32| inputs.slots[slot].held & bits & !lock.last_held[slot] != 0;
    let mut presses = [ShopPress::default(); MAX_PLAYERS];
    for (slot, press) in presses.iter_mut().enumerate() {
        let y = inputs.slots[slot].stick.y;
        let (was_up, was_down) = sticks[slot];
        let (up, down) = (y > 0.5, y < -0.5);
        sticks[slot] = (up, down);
        press.up = pressed(slot, button::DPAD_UP) || (up && !was_up);
        press.down = pressed(slot, button::DPAD_DOWN) || (down && !was_down);
        press.accept = pressed(slot, button::QUICK);
        press.sell = pressed(slot, button::MAGIC);
        press.back = pressed(slot, SlotInput::BACK);
    }
    if !shop.is_open() {
        // A Tower Menu's command, in the tower.
        let in_tower = population.as_ref().and_then(|p| crate::quest::level_of(&p.level)).is_some_and(|(realm, _)| realm == crate::quest::TOWER);
        let kind = if (0..MAX_PLAYERS).any(|s| pressed(s, SlotInput::OPEN_SHOP)) {
            Some(ShopKind::Shop)
        } else if (0..MAX_PLAYERS).any(|s| pressed(s, SlotInput::OPEN_INVENTORY)) {
            Some(ShopKind::Inventory)
        } else {
            None
        };
        if let (Some(kind), true, Some(data)) = (kind, in_tower, data.as_deref()) {
            let mut open = ShopOpen::new(kind);
            for (slot, state) in party.states() {
                open.bonus[slot] = state.bought;
            }
            info!("online: the {kind:?} screen opens at tick {}", lock.tick);
            shop.open(data, &party, open);
        }
        return;
    }
    let outcome = crate::shop::tick(&mut shop, 2.0, &presses, &mut party);
    if outcome != ShopOutcome::Continue {
        for slot in 0..MAX_PLAYERS {
            if let (Some(bought), Some(state)) = (shop.bonus(slot), party.state_mut(slot)) {
                state.bought = bought;
            }
        }
        shop.close();
        info!("online: the shop screen closes at tick {}", lock.tick);
    }
}

/// Every tick a multiple of the session's interval: a hash of the game's
/// state, which the host compares between the machines. Its parts go to
/// the log (`GDL_SYNC_LOG=1` every time; otherwise on a mismatch), to find
/// what differed.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn checksum(
    lock: Res<Lockstep>,
    online: Option<Res<Online>>,
    game: Res<LoadedGame>,
    party: Res<Party>,
    heroes: Query<&crate::player::Player>,
    monsters: Query<&crate::monsters::Monster>,
    critters: Query<&crate::critters::Critter>,
    projectiles: Query<&crate::projectiles::Projectile>,
    (monster_level, critter_level, items): (
        Option<Res<crate::monsters::MonsterLevel>>,
        Option<Res<crate::critters::CritterLevel>>,
        Option<Res<crate::items::LevelItems>>,
    ),
) {
    const INTERVAL: Tick = 30;
    let Some(online) = online else { return };
    if !lock.tick.is_multiple_of(INTERVAL) {
        return;
    }
    let hash = |f: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        f(&mut h);
        h.finish()
    };
    // Things in sets are summed: their order may differ between machines.
    let sum = |it: &mut dyn Iterator<Item = u64>| it.fold(0u64, u64::wrapping_add);
    let parts = [
        ("level", hash(&|h| game.current_name().hash(h))),
        ("party", hash(&|h| party.states().for_each(|(s, st)| (s, st.sync_hash()).hash(h)))),
        (
            "heroes",
            sum(&mut heroes.iter().map(|p| {
                hash(&|h| (p.slot, p.mover.position.map(sync_bits), sync_bits(p.mover.facing), p.look.on, sync_bits(p.look.yaw), sync_bits(p.look.pitch)).hash(h))
            })),
        ),
        (
            "monsters",
            sum(&mut monsters.iter().map(|m| {
                hash(&|h| (m.enemy, m.position.map(sync_bits), sync_bits(m.facing), sync_bits(m.hit_points)).hash(h))
            })),
        ),
        (
            "critters",
            sum(&mut critters.iter().map(|c| {
                hash(&|h| (c.position.map(sync_bits), sync_bits(c.yaw), sync_bits(c.hit_points)).hash(h))
            })),
        ),
        (
            "projectiles",
            sum(&mut projectiles.iter().map(|p| hash(&|h| p.position.to_array().map(sync_bits).hash(h)))),
        ),
        ("monster level", monster_level.map_or(0, |l| l.sync_hash())),
        ("critter level", critter_level.map_or(0, |l| l.sync_hash())),
        ("items", items.map_or(0, |i| i.sync_hash())),
    ];
    let total = hash(&|h| parts.iter().for_each(|(_, v)| v.hash(h)));
    online.session.report_checksum(lock.tick, total);
    let log = std::env::var("GDL_SYNC_LOG").is_ok_and(|v| !v.is_empty() && v != "0") || online.desync.is_some();
    if log {
        let list: Vec<String> = parts.iter().map(|(n, v)| format!("{n} {:04x}", v & 0xFFFF)).collect();
        // The run counts too (ticks start again at each restart).
        let at = u64::from(online.session.epoch()) * 100_000 + u64::from(lock.tick);
        info!("sync tick {at}: {:016x} ({})", total, list.join(", "));
    }
}

/// `GDL_DESYNC_AT=<tick>` (testing the recovery): on that tick of the first
/// run this machine's game goes its own way — player 1 finds a coin the
/// others don't.
fn test_desync(lock: Res<Lockstep>, online: Option<Res<Online>>, mut party: ResMut<Party>, mut at: Local<Option<Option<u32>>>) {
    let at = *at.get_or_insert_with(|| std::env::var("GDL_DESYNC_AT").ok().and_then(|v| v.parse().ok()));
    if at.is_some_and(|t| t == lock.tick) && online.is_some_and(|o| o.session.epoch() == 0)
        && let Some(s) = party.states_mut().next().map(|(_, s)| s)
    {
        warn!("online: GDL_DESYNC_AT: player 1 gets a coin here only");
        s.gold += 1;
    }
}

/// A new game online: what the game keeps between levels starts afresh on
/// every machine (the systems' own keep theirs: `NewGame`).
#[allow(clippy::too_many_arguments)]
fn fresh_game(
    mut new_game: MessageReader<NewGame>,
    mut voices: ResMut<crate::audio::VoiceQueues>,
    mut boxes: ResMut<crate::message_box::MessageBox>,
    mut snapshot: ResMut<crate::frontend::Snapshot>,
    mut pad: ResMut<crate::player::HeroPad>,
    mut stop: ResMut<crate::player_state::TimeStop>,
    mut scale: ResMut<crate::player_state::EnemyScale>,
    mut inputs: ResMut<Inputs>,
    mut shop: ResMut<crate::shop::ShopScreen>,
) {
    if new_game.read().count() == 0 {
        return;
    }
    // Its own random numbers start alike too.
    *shop = default();
    voices.clear();
    boxes.clear();
    *snapshot = default();
    *pad = default();
    *stop = default();
    *scale = default();
    *inputs = default();
}

/// At the frame's end: frames since level work.
fn settle(mut lock: ResMut<Lockstep>, population: Option<Res<LevelPopulation>>) {
    let busy = std::mem::take(&mut lock.level_work) || population.is_some_and(|p| p.is_changed()) || lock.held;
    lock.quiet = if busy { 0 } else { lock.quiet.saturating_add(1) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticks_round_trip_to_the_same_value_everywhere() {
        let i = SlotInput { stick: Vec2::new(0.5, -1.0), c_stick: Vec2::new(0.0, 2.0), held: 0x0300_0100 };
        let net = to_net(&i);
        assert_eq!(net.stick, [64, -127]);
        assert_eq!(net.c_stick, [0, 127]);
        let back = from_net(&net);
        assert_eq!(back.held, i.held);
        assert!((back.stick.x - 0.5).abs() < 0.01 && back.stick.y == -1.0 && back.c_stick.y == 1.0);
        // What a machine plays is what it would send again.
        assert_eq!(to_net(&back), net);
    }

    #[test]
    fn messages_survive_the_wire() {
        let hero = Hero { class: "WAR".into(), variant: "BLU".into(), name: "PELE".into(), saved: None };
        for m in [
            Message::Column { slot: 2, show: ColumnShow { step: 3, name: "AB".into(), ..default() } },
            Message::Hero { slot: 1, hero: Some(hero.clone()) },
            Message::Hero { slot: 1, hero: None },
            Message::Begin { heroes: vec![(0, hero.clone()), (3, hero)] },
            Message::SkipMovie { level: "levelA1".into() },
        ] {
            let text = ron::to_string(&m).unwrap();
            assert_eq!(ron::from_str::<Message>(&text).unwrap(), m);
        }
    }

    #[test]
    fn presses_are_what_a_player_newly_holds() {
        let mut lock = Lockstep { on: true, ..default() };
        let mut inputs = Inputs::default();
        inputs.slots[2].held = SlotInput::BACK;
        assert!(lock.pressed(&inputs, SlotInput::BACK));
        lock.last_held[2] = SlotInput::BACK;
        assert!(!lock.pressed(&inputs, SlotInput::BACK));
    }

    #[test]
    fn an_invite_is_taken_from_the_clipboard_text_without_its_spaces() {
        assert_eq!(clean_invite(" gdl1-abcd\n efgh ").as_deref(), Some("gdl1-abcdefgh"));
        assert_eq!(clean_invite("hello"), None);
    }
}
