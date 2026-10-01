//! Online co-op for up to four players: deterministic lockstep over
//! [iroh](https://docs.rs/iroh) peer-to-peer QUIC.
//!
//! Every machine runs the whole game and draws its own camera. The
//! simulation steps at 30 Hz, and every machine simulates tick `T` with
//! every player's input for `T` — the same inputs everywhere — so the
//! network only carries inputs, a few control messages and checksums.
//!
//! # Shape
//!
//! A star: one machine hosts, the others ([clients]) connect to it only,
//! and it relays. The host makes an **invite code** — its iroh endpoint
//! address, `GDL<protocol>-<base32>` — for a friend to paste into
//! [`NetSession::join`]. iroh dials by public key, punches through home
//! NATs and falls back to n0's public relay servers, so no port
//! forwarding is needed.
//!
//! # Lockstep
//!
//! - Four player **slots**. The host gives each machine as many as it
//!   asks for (two pads on one machine take two), its own first.
//! - **Input delay** `D` (default 3 ticks): the input a machine samples
//!   as it simulates tick `t` is for tick `t + D`, which gives it `D`
//!   ticks to travel. The first `D` ticks are idle on every machine. The
//!   host may raise `D` mid-game (never lower it): each machine then
//!   repeats its current input over the ticks the raise opens up.
//! - Clients send their slots' inputs to the host as QUIC **datagrams**,
//!   each repeating every tick the host hasn't acknowledged yet, so a
//!   lost datagram is covered by the next one and nothing is resent on a
//!   timer of its own.
//! - The host gathers every slot's input for tick `T` and sends the
//!   **bundle** `[Option<PlayerInput>; 4]` to everyone — again repeating
//!   whatever a client hasn't acknowledged. Every machine, the host too,
//!   simulates `T` only once its bundle is known
//!   ([`NetSession::ready_inputs`]); a late bundle just holds the game,
//!   and [`NetSession::waiting_for`] says who it waits for. Empty slots
//!   are `None`; a machine that leaves or times out has its slots `None`
//!   from the first tick the host hadn't closed yet.
//! - **Checksums**: every 30 ticks each machine reports a hash of its
//!   state ([`NetSession::report_checksum`]); clients send theirs to the
//!   host, which compares and tells everyone [`NetEvent::Desync`] on a
//!   mismatch.
//! - **Control messages**: opaque bytes the game defines, sent reliably
//!   and in order on a QUIC stream ([`NetSession::send_control`]); the
//!   host relays them between clients.
//!
//! # Threads
//!
//! tokio and iroh live on a background thread with their own runtime; the
//! game's side is plain synchronous Rust: every call here returns at once
//! (only [`NetSession::host`] waits, for the endpoint to come up).
//!
//! # From the game
//!
//! Each fixed update (30 Hz):
//!
//! ```no_run
//! # use gdl_net::*;
//! # fn hash_state() -> u64 { 0 }
//! # fn simulate(_: &[Option<PlayerInput>; 4]) {}
//! # let (session, _invite) = NetSession::host(NetConfig::default()).unwrap();
//! # let mut tick = 0;
//! for event in session.poll_events() {
//!     // lobby updates, game control messages, desyncs…
//! }
//! for slot in session.local_slots() {
//!     session.set_local_input(slot, PlayerInput::default() /* this pad's state */);
//! }
//! if let Some(inputs) = session.ready_inputs(tick) {
//!     simulate(&inputs);
//!     session.report_checksum(tick, hash_state());
//!     tick += 1;
//! } else {
//!     let _waiting = session.waiting_for(); // "waiting for …"
//! }
//! ```
//!
//! See `docs/online.md` for the design, the invite flow, the messages and
//! the limits.
//!
//! [clients]: NetSession::join

mod invite;
mod lockstep;
mod transport;
mod wire;

use std::net::SocketAddr;
use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};

pub use invite::InviteError;

use lockstep::{Core, CoreConfig};
use transport::{Driver, Shared};

/// The network protocol's version: builds speaking another refuse each
/// other (the invite code says which it is for).
pub const PROTOCOL: u16 = 2;
/// The QUIC application protocol name.
pub const ALPN: &[u8] = b"gdl-coop/2";

/// A machine in the session: the host is [`HOST`], clients `1..=7`.
pub type PeerId = u8;
/// The host's peer id.
pub const HOST: PeerId = 0;
/// A simulation step (30 a second).
pub type Tick = u32;
/// Player slots.
pub const MAX_SLOTS: usize = 4;
/// Every slot's input for one tick (`None`: nobody plays that slot).
pub type Bundle = [Option<PlayerInput>; MAX_SLOTS];

/// One player's pad for one tick, quantized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlayerInput {
    pub stick: [i8; 2],
    pub c_stick: [i8; 2],
    pub buttons: u32,
}

/// A machine in the game and the slots it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    pub peer: PeerId,
    pub name: String,
    pub slots: Vec<u8>,
}

/// Who a control message is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Everyone but the sender.
    All,
    /// The host (the host itself included).
    Host,
    /// One machine (oneself included).
    Peer(PeerId),
}

/// What happened since the last [`NetSession::poll_events`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetEvent {
    /// In the game as `you`, playing `slots` (the host has this at once).
    /// `late`: the game was under way; the slots play from the host's next
    /// [`NetSession::restart`].
    Connected { you: PeerId, slots: Vec<u8>, late: bool },
    /// Joining didn't work: the host is unreachable, refused (full,
    /// started, another build), or the invite was wrong.
    Failed(String),
    PeerJoined { peer: PeerId, name: String, slots: Vec<u8> },
    /// A machine left or timed out; its slots are empty from `from_tick`
    /// on (when the game had started).
    PeerLeft { peer: PeerId, from_tick: Option<Tick> },
    /// The host started the game: tick 0 is next, the first `delay`
    /// ticks idle.
    Started { delay: u8 },
    /// The host started lockstep again at tick 0 (everyone in the roster
    /// playing); the run before is forgotten.
    Restarted { delay: u8 },
    /// The host raised the input delay.
    DelayChanged { delay: u8 },
    /// A game message.
    Control { from: PeerId, bytes: Vec<u8> },
    /// The machines' checksums for `tick` differ.
    Desync { tick: Tick },
    /// The host is gone (a client's session is over).
    Disconnected(String),
}

/// Where the connections may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relays {
    /// n0's public relays and address lookup: works across the internet,
    /// behind home NATs.
    Public,
    /// Direct addresses only (LAN, tests): no relay, no lookup.
    Disabled,
}

/// A session's settings.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// The player's name, shown to the others (32 characters kept).
    pub name: String,
    /// Slots this machine plays (pads on it), 0–4.
    pub local_players: u8,
    /// Input delay in ticks to start with (the host's counts).
    pub input_delay: u8,
    /// The host raises the delay from measured round trips, up to
    /// `max_input_delay`.
    pub auto_delay: bool,
    pub max_input_delay: u8,
    /// The game's build: machines whose builds differ refuse each other
    /// (lockstep needs the same simulation everywhere).
    pub game_version: String,
    pub relays: Relays,
    /// Bind to this address only (`127.0.0.1:0` in tests); all
    /// interfaces when `None`.
    pub bind: Option<SocketAddr>,
    /// A machine silent this long is gone.
    pub timeout: Duration,
    /// Checksums are compared every this many ticks.
    pub checksum_interval: u32,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            name: "Player".into(),
            local_players: 1,
            input_delay: 3,
            auto_delay: true,
            max_input_delay: 8,
            game_version: String::new(),
            relays: Relays::Public,
            bind: None,
            timeout: Duration::from_secs(10),
            checksum_interval: 30,
        }
    }
}

impl NetConfig {
    fn core(&self) -> CoreConfig {
        CoreConfig {
            name: self.name.chars().take(32).collect(),
            players: self.local_players.min(MAX_SLOTS as u8),
            delay: self.input_delay.max(1),
            max_delay: self.max_input_delay.max(self.input_delay.max(1)),
            auto_delay: self.auto_delay,
            game: self.game_version.clone(),
            timeout: self.timeout,
            checksum_interval: self.checksum_interval,
            packet_budget: transport::PACKET_BUDGET,
        }
    }
}

/// Why a session couldn't be made or a call refused.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error(transparent)]
    Invite(#[from] InviteError),
    #[error("couldn't start networking: {0}")]
    Setup(String),
    #[error("only the host can do that")]
    NotHost,
    #[error("the game has already started")]
    Started,
}

/// A co-op session: the host's or a client's. Dropping it leaves the
/// game (telling the others).
pub struct NetSession {
    shared: Arc<Shared>,
    driver: Option<Driver>,
}

impl NetSession {
    /// Hosts a game: brings up the endpoint (waiting a few seconds for a
    /// relay with [`Relays::Public`]) and returns the invite code to give
    /// the others.
    pub fn host(cfg: NetConfig) -> Result<(NetSession, String), NetError> {
        if cfg.local_players == 0 {
            return Err(NetError::Setup("the host must play at least one slot".into()));
        }
        let shared = Arc::new(Shared::new(Core::new_host(cfg.core(), Instant::now())));
        let (driver, invite) = transport::spawn_host(&cfg, shared.clone())?;
        Ok((NetSession { shared, driver: Some(driver) }, invite))
    }

    /// Joins the game an invite code names. Returns at once (a bad or
    /// mismatched code is an error now); [`NetEvent::Connected`] or
    /// [`NetEvent::Failed`] follows.
    pub fn join(invite: &str, cfg: NetConfig) -> Result<NetSession, NetError> {
        let addr = invite::decode(invite)?;
        let shared = Arc::new(Shared::new(Core::new_client(cfg.core(), Instant::now())));
        let driver = transport::spawn_client(&cfg, addr, shared.clone())?;
        Ok(NetSession { shared, driver: Some(driver) })
    }

    /// The host's current invite code (it can change as the network
    /// does: a relay found late, an address change).
    pub fn invite(&self) -> Option<String> {
        self.shared.invite.lock().ok().and_then(|i| i.clone())
    }

    /// A shorter invite code (about 60 characters) naming only the host's
    /// endpoint id: the friend's iroh looks the addresses up through n0's
    /// DNS, so both need [`Relays::Public`], and it can take a few seconds
    /// after hosting before the lookup finds the host.
    pub fn short_invite(&self) -> Option<String> {
        self.shared.short_invite.lock().ok().and_then(|i| i.clone())
    }

    fn core(&self) -> MutexGuard<'_, Core> {
        // A panic on the network thread mid-update leaves the engine as it
        // was; carry on with it.
        self.shared.core.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Everything that happened since the last call.
    pub fn poll_events(&self) -> Vec<NetEvent> {
        self.core().take_events()
    }

    /// The host starts the game: a machine joining after this waits, its
    /// slots kept, for the host's next [`NetSession::restart`].
    pub fn start(&self) -> Result<(), NetError> {
        let mut core = self.core();
        if !core.is_host() {
            return Err(NetError::NotHost);
        }
        if !core.start() {
            return Err(NetError::Started);
        }
        drop(core);
        self.shared.wake.notify_one();
        Ok(())
    }

    /// This machine's slot `slot`'s pad now. The latest sample before a
    /// tick is committed counts.
    pub fn set_local_input(&self, slot: u8, input: PlayerInput) {
        self.core().set_local_input(slot, input);
    }

    /// Every slot's input for `tick` — ask for ticks in order, from 0 —
    /// once the host's bundle for it is known; `None` means wait. The
    /// first time a tick is handed out, this machine's current samples are
    /// committed for `tick + input_delay` and sent.
    pub fn ready_inputs(&self, tick: Tick) -> Option<Bundle> {
        let inputs = self.core().ready_inputs(tick);
        if inputs.is_some() {
            self.shared.wake.notify_one();
        }
        inputs
    }

    /// This machine's checksum of its state after simulating `tick`; only
    /// every `checksum_interval`-th tick is compared.
    pub fn report_checksum(&self, tick: Tick, value: u64) {
        self.core().report_checksum(tick, value);
        self.shared.wake.notify_one();
    }

    /// Who the next tick waits for (empty when it doesn't wait): the
    /// machines whose inputs the host lacks, or the host itself when the
    /// bundles are slow to arrive.
    pub fn waiting_for(&self) -> Vec<PeerId> {
        self.core().waiting_for()
    }

    /// Sends a game message, reliably and in order.
    pub fn send_control(&self, to: Target, bytes: Vec<u8>) {
        self.core().send_control(to, bytes);
        self.shared.wake.notify_one();
    }

    /// The host starts lockstep again at tick 0: machines that joined since
    /// the start play from here, and a game that went out of sync can start
    /// again alike. Send what every machine needs to start from the same
    /// state (control messages) first: they arrive before the restart.
    pub fn restart(&self) -> Result<(), NetError> {
        let mut core = self.core();
        if !core.is_host() {
            return Err(NetError::NotHost);
        }
        core.restart();
        drop(core);
        self.shared.wake.notify_one();
        Ok(())
    }

    /// The host raises the input delay (it never comes down).
    pub fn raise_input_delay(&self, delay: u8) -> Result<(), NetError> {
        let mut core = self.core();
        if !core.is_host() {
            return Err(NetError::NotHost);
        }
        core.raise_delay(delay);
        drop(core);
        self.shared.wake.notify_one();
        Ok(())
    }

    pub fn input_delay(&self) -> u8 {
        self.core().delay()
    }

    /// Lockstep's run: 0 from the start, one more at each restart (the
    /// same on every machine once the restart reached it).
    pub fn epoch(&self) -> u8 {
        self.core().epoch()
    }

    /// Everyone in the game, this machine too.
    pub fn roster(&self) -> Vec<PeerInfo> {
        self.core().roster()
    }

    /// This machine's peer id once it's in.
    pub fn me(&self) -> Option<PeerId> {
        self.core().me()
    }

    /// The slots this machine plays.
    pub fn local_slots(&self) -> Vec<u8> {
        self.core().my_slots()
    }

    pub fn is_host(&self) -> bool {
        self.core().is_host()
    }

    pub fn started(&self) -> bool {
        self.core().started()
    }

    /// The round trip: a client's to the host, the host's slowest.
    pub fn rtt(&self) -> Option<Duration> {
        self.core().rtt()
    }

    /// Leaves the game, telling the others (dropping the session does the
    /// same).
    pub fn leave(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        if let Some(driver) = self.driver.take() {
            driver.stop();
        }
    }
}

impl Drop for NetSession {
    fn drop(&mut self) {
        self.shut_down();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session can live in a Bevy resource (shared between systems
    /// on any thread).
    #[test]
    fn the_session_is_send_and_sync() {
        fn send_sync<T: Send + Sync + 'static>() {}
        send_sync::<NetSession>();
        send_sync::<NetEvent>();
    }

    #[test]
    fn settings_are_clamped() {
        let cfg = NetConfig { name: "x".repeat(100), local_players: 9, input_delay: 0, max_input_delay: 0, ..NetConfig::default() };
        let core = cfg.core();
        assert_eq!((core.name.chars().count(), core.players, core.delay, core.max_delay), (32, 4, 1, 1));
    }
}
