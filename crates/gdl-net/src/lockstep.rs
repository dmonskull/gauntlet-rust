//! The lockstep engine: pure logic, no sockets and no clock of its own.
//! The transport feeds it what arrives ([`Core::on_message`],
//! [`Core::on_link_up`], [`Core::on_link_down`]) and asks it what to send
//! ([`Core::poll`], with the time); the game asks it for each tick's
//! inputs. The in-memory tests below drive it with a simulated network.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use crate::wire::{BUNDLE_BYTES, HEADER_BYTES, INPUT_BYTES, Msg};
use crate::{Bundle, HOST, MAX_SLOTS, NetEvent, PROTOCOL, PeerId, PeerInfo, PlayerInput, Target, Tick};

/// The transport's name for a connection. A client's only link is
/// [`HOST_LINK`].
pub(crate) type LinkId = u64;
pub(crate) const HOST_LINK: LinkId = 0;

/// Something for the transport to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Out {
    Send { link: LinkId, msg: Msg },
    Close { link: LinkId, reason: String },
}

/// The engine's settings (the session's, less the transport's).
#[derive(Clone, Debug)]
pub(crate) struct CoreConfig {
    pub name: String,
    /// Slots this machine plays.
    pub players: u8,
    pub delay: u8,
    pub max_delay: u8,
    pub auto_delay: bool,
    /// Builds that differ here refuse each other.
    pub game: String,
    pub timeout: Duration,
    pub checksum_interval: u32,
    /// Bytes a lockstep datagram may take.
    pub packet_budget: usize,
}

/// A lost lockstep packet is repeated this soon.
const RESEND: Duration = Duration::from_millis(30);
/// Round-trip probes (they keep quiet links alive too).
const PING_EVERY: Duration = Duration::from_millis(250);
/// The host checks whether the input delay should go up this often.
const DELAY_CHECK: Duration = Duration::from_secs(1);
/// One tick at 30 Hz.
const TICK: Duration = Duration::from_micros(33_333);
/// Bundles kept behind the oldest still needed (for a re-asked tick).
const KEEP: Tick = 64;
/// Client peer ids go up to this (the host's waiting mask has a bit each).
const MAX_PEER: PeerId = 7;

struct Link {
    peer: Option<PeerId>,
    last_heard: Instant,
    last_sent: Instant,
    last_ping: Instant,
    /// Something new to tell (an acknowledgement, new inputs).
    dirty: bool,
    /// Host side: the client has every bundle below this.
    bundle_ack: Tick,
    /// Host side: bundles below this have been sent at least once.
    sent_upto: Tick,
    rtt: Option<Duration>,
}

impl Link {
    fn new(now: Instant) -> Self {
        Link {
            peer: None,
            last_heard: now,
            last_sent: now,
            last_ping: now,
            dirty: false,
            bundle_ack: 0,
            sent_upto: 0,
            rtt: None,
        }
    }
}

/// One machine's side of the session: the host's or a client's.
pub(crate) struct Core {
    cfg: CoreConfig,
    host: bool,
    me: Option<PeerId>,
    roster: BTreeMap<PeerId, PeerInfo>,
    started: bool,
    delay: u8,
    /// The session is over (left, failed or disconnected).
    over: bool,
    events: Vec<NetEvent>,
    outgoing: Vec<Out>,
    links: BTreeMap<LinkId, Link>,
    /// Latest local samples by slot; local inputs are committed for ticks
    /// below `committed_to`; the game has simulated every tick below
    /// `consumed`.
    samples: [PlayerInput; MAX_SLOTS],
    committed_to: Tick,
    consumed: Tick,
    /// Final bundles known here, and every tick below `final_to` has one.
    bundles: BTreeMap<Tick, Bundle>,
    final_to: Tick,
    my_checks: BTreeMap<Tick, u64>,
    pings: HashMap<u32, Instant>,
    next_ping: u32,
    // Host only.
    slot_owner: [Option<PeerId>; MAX_SLOTS],
    pending: BTreeMap<Tick, Bundle>,
    checks: BTreeMap<Tick, BTreeMap<PeerId, u64>>,
    desynced: BTreeSet<Tick>,
    last_delay_check: Instant,
    // Client only.
    outbox: BTreeMap<Tick, Vec<PlayerInput>>,
    input_ack: Tick,
    host_waiting: u8,
    host_final: Tick,
}

impl Core {
    fn new(cfg: CoreConfig, host: bool, now: Instant) -> Self {
        let delay = cfg.delay.max(1);
        Core {
            cfg,
            host,
            me: None,
            roster: BTreeMap::new(),
            started: false,
            delay,
            over: false,
            events: Vec::new(),
            outgoing: Vec::new(),
            links: BTreeMap::new(),
            samples: [PlayerInput::default(); MAX_SLOTS],
            committed_to: 0,
            consumed: 0,
            bundles: BTreeMap::new(),
            final_to: 0,
            my_checks: BTreeMap::new(),
            pings: HashMap::new(),
            next_ping: 0,
            slot_owner: [None; MAX_SLOTS],
            pending: BTreeMap::new(),
            checks: BTreeMap::new(),
            desynced: BTreeSet::new(),
            last_delay_check: now,
            outbox: BTreeMap::new(),
            input_ack: 0,
            host_waiting: 0,
            host_final: 0,
        }
    }

    /// The host's side: peer 0, playing the first `players` slots.
    pub(crate) fn new_host(cfg: CoreConfig, now: Instant) -> Self {
        let mut c = Core::new(cfg, true, now);
        let slots: Vec<u8> = (0..c.cfg.players.min(MAX_SLOTS as u8)).collect();
        for &s in &slots {
            c.slot_owner[usize::from(s)] = Some(HOST);
        }
        c.me = Some(HOST);
        c.roster.insert(HOST, PeerInfo { peer: HOST, name: c.cfg.name.clone(), slots: slots.clone() });
        c.events.push(NetEvent::Connected { you: HOST, slots });
        c
    }

    /// A client's side, before it reaches the host.
    pub(crate) fn new_client(cfg: CoreConfig, now: Instant) -> Self {
        Core::new(cfg, false, now)
    }

    // --- The game's side -------------------------------------------------

    pub(crate) fn is_host(&self) -> bool {
        self.host
    }

    pub(crate) fn me(&self) -> Option<PeerId> {
        self.me
    }

    pub(crate) fn started(&self) -> bool {
        self.started
    }

    pub(crate) fn delay(&self) -> u8 {
        self.delay
    }

    pub(crate) fn roster(&self) -> Vec<PeerInfo> {
        self.roster.values().cloned().collect()
    }

    /// This machine's slots, in order.
    pub(crate) fn my_slots(&self) -> Vec<u8> {
        self.me.and_then(|m| self.roster.get(&m)).map_or_else(Vec::new, |p| p.slots.clone())
    }

    /// The round trip: a client's to the host, the host's worst.
    pub(crate) fn rtt(&self) -> Option<Duration> {
        self.links.values().filter_map(|l| l.rtt).max()
    }

    pub(crate) fn take_events(&mut self) -> Vec<NetEvent> {
        std::mem::take(&mut self.events)
    }

    pub(crate) fn set_local_input(&mut self, slot: u8, input: PlayerInput) {
        if let Some(s) = self.samples.get_mut(usize::from(slot)) {
            *s = input;
        }
    }

    /// Every slot's input for `tick` once known; the first time a tick is
    /// handed out, this machine's slots' current samples are committed for
    /// `tick + delay`. Ticks must be asked for in order.
    pub(crate) fn ready_inputs(&mut self, tick: Tick) -> Option<Bundle> {
        if !self.started || self.over {
            return None;
        }
        if tick < self.consumed {
            return self.bundles.get(&tick).copied();
        }
        if tick > self.consumed {
            return None;
        }
        let bundle = *self.bundles.get(&tick)?;
        self.consumed = tick + 1;
        let upto = tick + 1 + Tick::from(self.delay);
        self.commit(upto);
        self.prune();
        Some(bundle)
    }

    /// The peers the next tick waits for (empty when it doesn't wait).
    pub(crate) fn waiting_for(&self) -> Vec<PeerId> {
        if !self.started || self.consumed < self.final_to {
            return Vec::new();
        }
        if self.host {
            let mut out: Vec<PeerId> = Vec::new();
            for s in 0..MAX_SLOTS {
                if let Some(p) = self.slot_owner[s]
                    && self.pending.get(&self.final_to).is_none_or(|b| b[s].is_none())
                    && !out.contains(&p)
                {
                    out.push(p);
                }
            }
            return out;
        }
        if self.final_to < self.host_final || self.host_waiting == 0 {
            return vec![HOST];
        }
        (0..=MAX_PEER).filter(|p| self.host_waiting & 1 << p != 0).collect()
    }

    pub(crate) fn report_checksum(&mut self, tick: Tick, value: u64) {
        if self.over || self.cfg.checksum_interval == 0 || !tick.is_multiple_of(self.cfg.checksum_interval) {
            return;
        }
        if self.host {
            self.my_checks.insert(tick, value);
            self.compare(tick);
        } else {
            self.my_checks.insert(tick, value);
            self.send(HOST_LINK, Msg::Checksum { tick, value });
        }
    }

    pub(crate) fn send_control(&mut self, to: Target, bytes: Vec<u8>) {
        let Some(me) = self.me else { return };
        if self.over {
            return;
        }
        if self.host {
            self.route_control(me, to, bytes);
            return;
        }
        match to {
            Target::Peer(p) if p == me => self.events.push(NetEvent::Control { from: me, bytes }),
            _ => self.send(HOST_LINK, Msg::Control { peer: target_code(to), bytes }),
        }
    }

    /// The host starts lockstep: the roster is fixed from here (late joins
    /// are refused) and every machine's first `delay` ticks are idle.
    pub(crate) fn start(&mut self) -> bool {
        if !self.host || self.started || self.over {
            return false;
        }
        self.started = true;
        let delay = self.delay;
        for link in self.welcomed_links() {
            self.send(link, Msg::Start { delay });
        }
        self.events.push(NetEvent::Started { delay });
        self.commit(Tick::from(delay));
        true
    }

    /// Raises the input delay (the host only; it never comes down).
    pub(crate) fn raise_delay(&mut self, delay: u8) {
        if !self.host || delay <= self.delay || self.over {
            return;
        }
        self.delay = delay.min(self.cfg.max_delay.max(self.delay));
        let delay = self.delay;
        for link in self.welcomed_links() {
            self.send(link, Msg::Delay { delay });
        }
        self.events.push(NetEvent::DelayChanged { delay });
    }

    /// Leaves for good, telling the others.
    pub(crate) fn leave(&mut self) {
        if self.over {
            return;
        }
        let links: Vec<LinkId> = self.links.keys().copied().collect();
        for link in links {
            self.send(link, Msg::Leave);
            self.outgoing.push(Out::Close { link, reason: "left".into() });
        }
        self.over = true;
    }

    // --- The transport's side --------------------------------------------

    pub(crate) fn on_link_up(&mut self, link: LinkId, now: Instant) {
        self.links.insert(link, Link::new(now));
        if !self.host {
            let msg = Msg::Hello {
                protocol: PROTOCOL,
                game: self.cfg.game.clone(),
                name: self.cfg.name.clone(),
                players: self.cfg.players,
            };
            self.send(link, msg);
        }
    }

    /// The transport couldn't reach the host.
    pub(crate) fn on_connect_failed(&mut self, reason: String) {
        if !self.over {
            self.over = true;
            self.events.push(NetEvent::Failed(reason));
        }
    }

    pub(crate) fn on_link_down(&mut self, link: LinkId, reason: &str) {
        let Some(l) = self.links.remove(&link) else { return };
        if self.host {
            self.drop_peer(l.peer);
        } else {
            self.lost_host(reason);
        }
    }

    pub(crate) fn on_message(&mut self, link: LinkId, msg: Msg, now: Instant) {
        let Some(l) = self.links.get_mut(&link) else { return };
        l.last_heard = now;
        match msg {
            Msg::Ping { id } => self.send(link, Msg::Pong { id }),
            Msg::Pong { id } => {
                if let Some(sent) = self.pings.remove(&id)
                    && let Some(l) = self.links.get_mut(&link)
                {
                    l.rtt = Some(now.saturating_duration_since(sent));
                }
            }
            msg if self.host => self.host_message(link, msg),
            msg => self.client_message(msg),
        }
    }

    /// What to send now: lockstep packets that are due, probes, and what
    /// the calls since the last poll queued.
    pub(crate) fn poll(&mut self, now: Instant) -> Vec<Out> {
        if !self.over {
            self.timeouts(now);
            self.pings(now);
            if self.host {
                self.auto_delay(now);
            }
            if self.started {
                self.lockstep_packets(now);
            }
        }
        std::mem::take(&mut self.outgoing)
    }

    // --- Inside ------------------------------------------------------------

    fn send(&mut self, link: LinkId, msg: Msg) {
        self.outgoing.push(Out::Send { link, msg });
    }

    fn welcomed_links(&self) -> Vec<LinkId> {
        self.links.iter().filter(|(_, l)| l.peer.is_some()).map(|(&k, _)| k).collect()
    }

    fn link_of(&self, peer: PeerId) -> Option<LinkId> {
        self.links.iter().find(|(_, l)| l.peer == Some(peer)).map(|(&k, _)| k)
    }

    /// Commits this machine's current samples for every tick below `upto`
    /// not yet committed (more than one when the delay went up).
    fn commit(&mut self, upto: Tick) {
        let slots = self.my_slots();
        while self.committed_to < upto {
            let t = self.committed_to;
            // The first `delay` ticks are idle on every machine.
            let idle = t < Tick::from(self.delay) && self.consumed == 0;
            let input = |s: u8| if idle { PlayerInput::default() } else { self.samples[usize::from(s)] };
            if self.host {
                let entry = self.pending.entry(t).or_insert([None; MAX_SLOTS]);
                for &s in &slots {
                    entry[usize::from(s)] = Some(input(s));
                }
            } else {
                let inputs: Vec<PlayerInput> = slots.iter().map(|&s| input(s)).collect();
                self.outbox.insert(t, inputs);
                if let Some(l) = self.links.get_mut(&HOST_LINK) {
                    l.dirty = true;
                }
            }
            self.committed_to += 1;
        }
        if self.host {
            self.finalize();
        }
    }

    /// The host closes every tick whose inputs are all in, in order.
    fn finalize(&mut self) {
        if !self.started {
            return;
        }
        // No owned slot would make every tick final at once.
        if self.slot_owner.iter().all(Option::is_none) {
            return;
        }
        loop {
            let t = self.final_to;
            let pending = self.pending.get(&t);
            let mut b: Bundle = [None; MAX_SLOTS];
            for (s, owner) in self.slot_owner.iter().enumerate() {
                if owner.is_some() {
                    match pending.and_then(|p| p[s]) {
                        Some(i) => b[s] = Some(i),
                        None => return,
                    }
                }
            }
            self.pending.remove(&t);
            self.bundles.insert(t, b);
            self.final_to += 1;
        }
    }

    fn prune(&mut self) {
        let oldest_needed = if self.host {
            self.links.values().filter(|l| l.peer.is_some()).map(|l| l.bundle_ack).min().unwrap_or(self.consumed).min(self.consumed)
        } else {
            self.consumed
        };
        let keep_from = oldest_needed.saturating_sub(KEEP);
        self.bundles = self.bundles.split_off(&keep_from);
        let checks_from = self.consumed.saturating_sub(KEEP * 10);
        self.my_checks = self.my_checks.split_off(&checks_from);
        self.checks = self.checks.split_off(&checks_from);
    }

    fn host_message(&mut self, link: LinkId, msg: Msg) {
        let peer = self.links.get(&link).and_then(|l| l.peer);
        match (msg, peer) {
            (Msg::Hello { protocol, game, name, players }, None) => self.hello(link, protocol, game, name, players),
            (Msg::Inputs { ack, base, slots, inputs }, Some(p)) => self.inputs(link, p, ack, base, slots, &inputs),
            (Msg::Checksum { tick, value }, Some(p)) => {
                self.checks.entry(tick).or_default().insert(p, value);
                self.compare(tick);
            }
            (Msg::Control { peer: code, bytes }, Some(p)) => self.route_control(p, target_of(code), bytes),
            (Msg::Leave, Some(_)) => {
                self.links.remove(&link);
                self.outgoing.push(Out::Close { link, reason: "left".into() });
                self.drop_peer(peer);
            }
            _ => {}
        }
    }

    fn hello(&mut self, link: LinkId, protocol: u16, game: String, name: String, players: u8) {
        let refuse = |c: &mut Core, reason: String| {
            c.send(link, Msg::Reject { reason: reason.clone() });
            c.outgoing.push(Out::Close { link, reason });
        };
        if protocol != PROTOCOL {
            return refuse(self, format!("the host speaks network protocol {PROTOCOL}, you speak {protocol}"));
        }
        if game != self.cfg.game {
            return refuse(self, format!("the host runs game build {:?}, you run {game:?}: both need the same version", self.cfg.game));
        }
        if self.started {
            return refuse(self, "the game has already started".into());
        }
        let free: Vec<u8> = (0..MAX_SLOTS as u8).filter(|&s| self.slot_owner[usize::from(s)].is_none()).collect();
        if usize::from(players) > free.len() {
            return refuse(self, format!("the game is full ({} free player slots, you bring {players})", free.len()));
        }
        let Some(peer) = (1..=MAX_PEER).find(|p| !self.roster.contains_key(p)) else {
            return refuse(self, "too many machines in the game".into());
        };
        let slots: Vec<u8> = free[..usize::from(players)].to_vec();
        for &s in &slots {
            self.slot_owner[usize::from(s)] = Some(peer);
        }
        let info = PeerInfo { peer, name: name.chars().take(32).collect(), slots };
        for other in self.welcomed_links() {
            self.send(other, Msg::Joined(info.clone()));
        }
        self.roster.insert(peer, info.clone());
        if let Some(l) = self.links.get_mut(&link) {
            l.peer = Some(peer);
        }
        let roster = self.roster();
        self.send(link, Msg::Welcome { you: peer, delay: self.delay, roster });
        self.events.push(NetEvent::PeerJoined { peer, name: info.name, slots: info.slots });
    }

    #[allow(clippy::too_many_arguments)]
    fn inputs(&mut self, link: LinkId, peer: PeerId, ack: Tick, base: Tick, slots: u8, inputs: &[PlayerInput]) {
        let theirs: Vec<u8> = self.roster.get(&peer).map_or_else(Vec::new, |p| p.slots.clone());
        if let Some(l) = self.links.get_mut(&link) {
            l.bundle_ack = l.bundle_ack.max(ack.min(self.final_to));
        }
        if usize::from(slots) != theirs.len() || theirs.is_empty() {
            return;
        }
        let mut new = false;
        for (k, chunk) in inputs.chunks(theirs.len()).enumerate() {
            let t = base + k as Tick;
            // Old ticks are final already; far-future ones are nonsense.
            if t < self.final_to || t > self.final_to + 10_000 {
                continue;
            }
            let entry = self.pending.entry(t).or_insert([None; MAX_SLOTS]);
            for (&s, &i) in theirs.iter().zip(chunk) {
                // The owner may have left meanwhile.
                if self.slot_owner[usize::from(s)] == Some(peer) && entry[usize::from(s)].is_none() {
                    entry[usize::from(s)] = Some(i);
                    new = true;
                }
            }
        }
        if new {
            if let Some(l) = self.links.get_mut(&link) {
                l.dirty = true;
            }
            self.finalize();
        }
    }

    /// The first tick the host still lacks `peer`'s inputs for.
    fn input_ack_of(&self, peer: PeerId) -> Tick {
        let theirs: Vec<usize> = (0..MAX_SLOTS).filter(|&s| self.slot_owner[s] == Some(peer)).collect();
        let mut t = self.final_to;
        while self.pending.get(&t).is_some_and(|b| theirs.iter().all(|&s| b[s].is_some())) {
            t += 1;
        }
        t
    }

    fn route_control(&mut self, from: PeerId, to: Target, bytes: Vec<u8>) {
        let me = self.me.unwrap_or(HOST);
        match to {
            Target::All => {
                if from != me {
                    self.events.push(NetEvent::Control { from, bytes: bytes.clone() });
                }
                let links: Vec<(LinkId, Option<PeerId>)> = self.links.iter().map(|(&k, l)| (k, l.peer)).collect();
                for (link, p) in links {
                    if p.is_some() && p != Some(from) {
                        self.send(link, Msg::Control { peer: from, bytes: bytes.clone() });
                    }
                }
            }
            Target::Host => self.events.push(NetEvent::Control { from, bytes }),
            Target::Peer(p) if p == me => self.events.push(NetEvent::Control { from, bytes }),
            Target::Peer(p) => {
                if let Some(link) = self.link_of(p) {
                    self.send(link, Msg::Control { peer: from, bytes });
                }
            }
        }
    }

    /// The host compares the checksums in for `tick`.
    fn compare(&mut self, tick: Tick) {
        let Some(&mine) = self.my_checks.get(&tick) else { return };
        let differs = self.checks.get(&tick).is_some_and(|m| m.values().any(|&v| v != mine));
        if differs && self.desynced.insert(tick) {
            for link in self.welcomed_links() {
                self.send(link, Msg::Desync { tick });
            }
            self.events.push(NetEvent::Desync { tick });
        }
    }

    /// A client is gone: its slots are empty from the next open tick.
    fn drop_peer(&mut self, peer: Option<PeerId>) {
        let Some(peer) = peer else { return };
        if self.roster.remove(&peer).is_none() {
            return;
        }
        for owner in self.slot_owner.iter_mut() {
            if *owner == Some(peer) {
                *owner = None;
            }
        }
        let from = self.started.then_some(self.final_to);
        for other in self.welcomed_links() {
            self.send(other, Msg::Left { peer, from });
        }
        self.events.push(NetEvent::PeerLeft { peer, from_tick: from });
        self.finalize();
    }

    fn client_message(&mut self, msg: Msg) {
        match msg {
            Msg::Welcome { you, delay, roster } => {
                if self.me.is_some() {
                    return;
                }
                self.me = Some(you);
                self.delay = delay.max(1);
                for p in roster {
                    if p.peer != you {
                        self.events.push(NetEvent::PeerJoined { peer: p.peer, name: p.name.clone(), slots: p.slots.clone() });
                    }
                    self.roster.insert(p.peer, p);
                }
                let slots = self.my_slots();
                self.events.push(NetEvent::Connected { you, slots });
            }
            Msg::Reject { reason } => {
                self.over = true;
                self.events.push(NetEvent::Failed(reason));
            }
            Msg::Joined(p) => {
                self.events.push(NetEvent::PeerJoined { peer: p.peer, name: p.name.clone(), slots: p.slots.clone() });
                self.roster.insert(p.peer, p);
            }
            Msg::Left { peer, from } => {
                if self.roster.remove(&peer).is_some() {
                    self.events.push(NetEvent::PeerLeft { peer, from_tick: from });
                }
            }
            Msg::Start { delay } => {
                if !self.started && self.me.is_some() {
                    self.started = true;
                    self.delay = delay.max(1);
                    self.events.push(NetEvent::Started { delay: self.delay });
                    self.commit(Tick::from(self.delay));
                }
            }
            Msg::Delay { delay } => {
                if delay > self.delay {
                    self.delay = delay;
                    self.events.push(NetEvent::DelayChanged { delay });
                }
            }
            Msg::Control { peer, bytes } => self.events.push(NetEvent::Control { from: peer, bytes }),
            Msg::Desync { tick } => self.events.push(NetEvent::Desync { tick }),
            Msg::Leave => {
                self.links.remove(&HOST_LINK);
                self.lost_host("the host left the game");
            }
            Msg::Bundles { ack, base, waiting, bundles } => {
                self.input_ack = self.input_ack.max(ack);
                self.outbox = self.outbox.split_off(&self.input_ack);
                self.host_waiting = waiting;
                self.host_final = self.host_final.max(base + bundles.len() as Tick);
                let before = self.final_to;
                for (k, b) in bundles.into_iter().enumerate() {
                    let t = base + k as Tick;
                    if t >= self.final_to {
                        self.bundles.entry(t).or_insert(b);
                    }
                }
                while self.bundles.contains_key(&self.final_to) {
                    self.final_to += 1;
                }
                if self.final_to != before
                    && let Some(l) = self.links.get_mut(&HOST_LINK)
                {
                    l.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn lost_host(&mut self, reason: &str) {
        if self.over {
            return;
        }
        self.over = true;
        if self.me.is_none() {
            self.events.push(NetEvent::Failed(format!("couldn't join: {reason}")));
        } else {
            self.roster.remove(&HOST);
            self.events.push(NetEvent::PeerLeft { peer: HOST, from_tick: None });
            self.events.push(NetEvent::Disconnected(reason.to_string()));
        }
        self.outgoing.push(Out::Close { link: HOST_LINK, reason: reason.to_string() });
    }

    fn timeouts(&mut self, now: Instant) {
        let late: Vec<LinkId> =
            self.links.iter().filter(|(_, l)| now.saturating_duration_since(l.last_heard) >= self.cfg.timeout).map(|(&k, _)| k).collect();
        for link in late {
            let peer = self.links.remove(&link).and_then(|l| l.peer);
            self.outgoing.push(Out::Close { link, reason: "timed out".into() });
            if self.host {
                self.drop_peer(peer);
            } else {
                self.lost_host("the host stopped answering");
            }
        }
    }

    fn pings(&mut self, now: Instant) {
        let due: Vec<LinkId> = self
            .links
            .iter()
            .filter(|(_, l)| (l.peer.is_some() || !self.host) && now.saturating_duration_since(l.last_ping) >= PING_EVERY)
            .map(|(&k, _)| k)
            .collect();
        for link in due {
            let id = self.next_ping;
            self.next_ping = self.next_ping.wrapping_add(1);
            self.pings.insert(id, now);
            if let Some(l) = self.links.get_mut(&link) {
                l.last_ping = now;
            }
            self.send(link, Msg::Ping { id });
        }
        // Unanswered probes are forgotten after a while.
        self.pings.retain(|_, sent| now.saturating_duration_since(*sent) < Duration::from_secs(10));
    }

    /// The host raises the delay to what the worst round trip needs: a
    /// tick's input has to reach the host and its bundle come back within
    /// `delay` ticks.
    fn auto_delay(&mut self, now: Instant) {
        if !self.cfg.auto_delay || now.saturating_duration_since(self.last_delay_check) < DELAY_CHECK {
            return;
        }
        self.last_delay_check = now;
        if let Some(rtt) = self.rtt() {
            let needed = ((rtt + RESEND / 2).as_secs_f32() / TICK.as_secs_f32()).ceil() as u8;
            if needed > self.delay {
                self.raise_delay(needed);
            }
        }
    }

    fn lockstep_packets(&mut self, now: Instant) {
        let budget = self.cfg.packet_budget.max(HEADER_BYTES + BUNDLE_BYTES);
        let links: Vec<LinkId> = self.links.keys().copied().collect();
        for link in links {
            let Some(l) = self.links.get(&link) else { continue };
            let quiet = now.saturating_duration_since(l.last_sent);
            if self.host {
                let Some(peer) = l.peer else { continue };
                let unacked = l.bundle_ack < self.final_to;
                let fresh = l.sent_upto < self.final_to;
                if !(l.dirty || fresh || (unacked && quiet >= RESEND)) {
                    continue;
                }
                let base = l.bundle_ack;
                let room = (budget - HEADER_BYTES) / BUNDLE_BYTES;
                let bundles: Vec<Bundle> = self.bundles.range(base..self.final_to).take(room).map(|(_, b)| *b).collect();
                let waiting = self.waiting_mask();
                let msg = Msg::Bundles { ack: self.input_ack_of(peer), base, waiting, bundles };
                let l = self.links.get_mut(&link).expect("link");
                l.dirty = false;
                l.sent_upto = self.final_to;
                l.last_sent = now;
                self.send(link, msg);
            } else {
                let unacked = self.outbox.range(self.input_ack..).next().is_some();
                if !(l.dirty || (unacked && quiet >= RESEND)) {
                    continue;
                }
                let slots = self.my_slots().len();
                let per_tick = (slots * INPUT_BYTES).max(1);
                let room = ((budget - HEADER_BYTES) / per_tick).min(255);
                let base = self.input_ack;
                let mut inputs = Vec::new();
                for (k, (&t, v)) in self.outbox.range(base..).take(room).enumerate() {
                    // A gap would shift every later tick: stop at it.
                    if t != base + k as Tick {
                        break;
                    }
                    inputs.extend_from_slice(v);
                }
                let msg = Msg::Inputs { ack: self.final_to, base, slots: slots as u8, inputs };
                let l = self.links.get_mut(&link).expect("link");
                l.dirty = false;
                l.last_sent = now;
                self.send(link, msg);
            }
        }
    }

    /// The host's peers it lacks inputs from for its next bundle, a bit
    /// each.
    fn waiting_mask(&self) -> u8 {
        let t = self.final_to;
        (0..MAX_SLOTS).fold(0u8, |m, s| match self.slot_owner[s] {
            Some(p) if p <= MAX_PEER && self.pending.get(&t).is_none_or(|b| b[s].is_none()) => m | 1 << p,
            _ => m,
        })
    }
}

/// A target's code on the wire.
fn target_code(t: Target) -> u8 {
    match t {
        Target::All => 0xFF,
        Target::Host => HOST,
        Target::Peer(p) => p,
    }
}

fn target_of(code: u8) -> Target {
    match code {
        0xFF => Target::All,
        HOST => Target::Host,
        p => Target::Peer(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic random source.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn chance(&mut self, p: f64) -> bool {
            (self.next() % 10_000) as f64 / 10_000.0 < p
        }
        fn below(&mut self, n: u64) -> u64 {
            if n == 0 { 0 } else { self.next() % n }
        }
    }

    /// How the simulated network treats packets.
    #[derive(Clone, Copy)]
    struct Weather {
        latency_ms: u64,
        jitter_ms: u64,
        /// Lockstep datagrams lost (reliable messages never are).
        loss: f64,
    }

    const CALM: Weather = Weather { latency_ms: 20, jitter_ms: 0, loss: 0.0 };

    fn cfg(name: &str, players: u8) -> CoreConfig {
        CoreConfig {
            name: name.into(),
            players,
            delay: 3,
            max_delay: 10,
            auto_delay: false,
            game: "test".into(),
            timeout: Duration::from_secs(5),
            checksum_interval: 30,
            packet_budget: 1000,
        }
    }

    /// The input a slot's owner gives at a tick: something every machine
    /// can check for.
    fn input_for(slot: u8, tick: Tick) -> PlayerInput {
        PlayerInput { stick: [slot as i8, (tick % 100) as i8], c_stick: [0, -(slot as i8)], buttons: tick * 4 + u32::from(slot) }
    }

    /// One machine: its engine and its game.
    struct Machine {
        core: Core,
        /// Ticks simulated, and the bundles they ran on.
        tick: Tick,
        ran: Vec<Bundle>,
        events: Vec<NetEvent>,
        /// The game's own clock: one tick every 33 ms.
        next_frame: Instant,
        /// Unplugged: nothing in or out.
        dead: bool,
        /// Its checksum lies at this tick.
        lie_at: Option<Tick>,
    }

    struct InFlight {
        at: Instant,
        /// Index of the receiving machine and the link it arrives on.
        to: usize,
        link: LinkId,
        msg: Msg,
        /// Reliable messages of one direction keep their order.
        seq: u64,
    }

    /// A star: machine 0 hosts; machine k (k ≥ 1) is the host's link k.
    struct World {
        now: Instant,
        machines: Vec<Machine>,
        flying: Vec<InFlight>,
        weather: Vec<Weather>,
        rng: Rng,
        seq: u64,
        /// Per direction: when its last reliable message lands (to keep
        /// them in order).
        last_reliable: HashMap<(usize, usize), Instant>,
    }

    impl World {
        fn new(clients: &[(u8, Weather)], host_players: u8) -> World {
            let now = Instant::now();
            let mut machines = vec![Machine::new(Core::new_host(cfg("host", host_players), now), now)];
            let mut weather = vec![CALM];
            for (k, &(players, w)) in clients.iter().enumerate() {
                machines.push(Machine::new(Core::new_client(cfg(&format!("c{}", k + 1), players), now), now));
                weather.push(w);
            }
            let mut world = World { now, machines, flying: Vec::new(), weather, rng: Rng(0x9E37_79B9_7F4A_7C15), seq: 0, last_reliable: HashMap::new() };
            for k in 1..world.machines.len() {
                world.machines[0].core.on_link_up(k as LinkId, now);
                world.machines[k].core.on_link_up(HOST_LINK, now);
            }
            world
        }

        /// Runs `ms` milliseconds.
        fn run(&mut self, ms: u64) {
            for _ in 0..ms {
                self.now += Duration::from_millis(1);
                self.deliver();
                for k in 0..self.machines.len() {
                    if !self.machines[k].dead {
                        self.machines[k].frame(self.now);
                    }
                    let out = self.machines[k].core.poll(self.now);
                    if !self.machines[k].dead {
                        self.post(k, out);
                    }
                }
            }
        }

        fn post(&mut self, from: usize, out: Vec<Out>) {
            for o in out {
                let Out::Send { link, msg } = o else { continue };
                let (to, arrive_link) = if from == 0 { (link as usize, HOST_LINK) } else { (0, from as LinkId) };
                let client = if from == 0 { to } else { from };
                let w = self.weather[client];
                if !msg.reliable() && self.rng.chance(w.loss) {
                    continue;
                }
                let mut at = self.now + Duration::from_millis(w.latency_ms + self.rng.below(w.jitter_ms + 1));
                if msg.reliable() {
                    let last = self.last_reliable.entry((from, to)).or_insert(at);
                    at = at.max(*last);
                    *last = at;
                }
                self.seq += 1;
                self.flying.push(InFlight { at, to, link: arrive_link, msg, seq: self.seq });
            }
        }

        fn deliver(&mut self) {
            let now = self.now;
            let mut due: Vec<InFlight> = Vec::new();
            let mut k = 0;
            while k < self.flying.len() {
                if self.flying[k].at <= now {
                    due.push(self.flying.swap_remove(k));
                } else {
                    k += 1;
                }
            }
            due.sort_by_key(|f| (f.at, f.seq));
            for f in due {
                let m = &mut self.machines[f.to];
                if !m.dead {
                    m.core.on_message(f.link, f.msg, now);
                }
            }
        }

        fn start(&mut self) {
            assert!(self.machines[0].core.start());
        }

        /// Every machine ran the same bundles as far as both got, and at
        /// least `ticks` of them.
        fn agree(&self, ticks: usize) {
            let host = &self.machines[0].ran;
            for (k, m) in self.machines.iter().enumerate().filter(|(_, m)| !m.dead) {
                assert!(m.ran.len() >= ticks, "machine {k} ran only {} ticks", m.ran.len());
                let n = m.ran.len().min(host.len());
                assert_eq!(&m.ran[..n], &host[..n], "machine {k} disagrees");
            }
        }
    }

    impl Machine {
        fn new(core: Core, now: Instant) -> Machine {
            Machine { core, tick: 0, ran: Vec::new(), events: Vec::new(), next_frame: now, dead: false, lie_at: None }
        }

        /// The game's fixed update: sample, then simulate the next tick if
        /// its inputs are in.
        fn frame(&mut self, now: Instant) {
            self.events.extend(self.core.take_events());
            if now < self.next_frame {
                return;
            }
            self.next_frame += TICK;
            for s in self.core.my_slots() {
                self.core.set_local_input(s, input_for(s, self.tick + Tick::from(self.core.delay())));
            }
            if let Some(b) = self.core.ready_inputs(self.tick) {
                self.ran.push(b);
                let mut sum = self.ran.len() as u64;
                if self.lie_at == Some(self.tick) {
                    sum ^= 0xBAD;
                }
                self.core.report_checksum(self.tick, sum);
                self.tick += 1;
            }
        }
    }

    #[test]
    fn two_machines_run_the_same_ticks() {
        let mut w = World::new(&[(1, CALM)], 1);
        w.run(200);
        assert!(w.machines[1].events.iter().any(|e| matches!(e, NetEvent::Connected { you: 1, .. })));
        assert!(w.machines[0].events.iter().any(|e| matches!(e, NetEvent::PeerJoined { peer: 1, .. })));
        w.start();
        w.run(10_000);
        w.agree(250);
        // Each slot's inputs are its owner's, the first `delay` ticks idle.
        let ran = &w.machines[1].ran;
        assert_eq!(ran[0], [Some(PlayerInput::default()), Some(PlayerInput::default()), None, None]);
        for (t, b) in ran.iter().enumerate().skip(3) {
            assert_eq!(b[0], Some(input_for(0, t as Tick)), "tick {t}");
            assert_eq!(b[1], Some(input_for(1, t as Tick)), "tick {t}");
        }
        assert!(w.machines.iter().all(|m| !m.events.iter().any(|e| matches!(e, NetEvent::Desync { .. }))));
    }

    #[test]
    fn lost_and_shuffled_datagrams_change_nothing() {
        let rough = Weather { latency_ms: 40, jitter_ms: 60, loss: 0.25 };
        let mut w = World::new(&[(1, rough), (1, CALM)], 2);
        w.run(300);
        w.start();
        // Round trips of 80–200 ms against a 100 ms delay stall it now and
        // then: it gets on, and identically everywhere.
        w.run(12_000);
        w.agree(150);
        for (t, b) in w.machines[0].ran.iter().enumerate().skip(3) {
            for s in 0..4u8 {
                assert_eq!(b[usize::from(s)], Some(input_for(s, t as Tick)), "tick {t} slot {s}");
            }
        }
    }

    #[test]
    fn three_machines_one_with_two_pads() {
        let mut w = World::new(&[(2, CALM), (1, Weather { latency_ms: 60, jitter_ms: 10, loss: 0.05 })], 1);
        w.run(300);
        assert_eq!(w.machines[1].core.my_slots(), vec![1, 2]);
        assert_eq!(w.machines[2].core.my_slots(), vec![3]);
        assert_eq!(w.machines[2].core.roster().len(), 3);
        w.start();
        w.run(8_000);
        w.agree(150);
    }

    #[test]
    fn a_full_game_refuses_more_players() {
        let mut w = World::new(&[(2, CALM), (2, CALM)], 1);
        w.run(300);
        // The second client wanted two slots; only one is left.
        assert!(w.machines[2].events.iter().any(|e| matches!(e, NetEvent::Failed(r) if r.contains("full"))), "{:?}", w.machines[2].events);
    }

    #[test]
    fn a_different_build_is_refused() {
        let now = Instant::now();
        let mut host = Core::new_host(cfg("h", 1), now);
        host.on_link_up(1, now);
        host.on_message(1, Msg::Hello { protocol: PROTOCOL, game: "other".into(), name: "x".into(), players: 1 }, now);
        let out = host.poll(now);
        assert!(out.iter().any(|o| matches!(o, Out::Send { msg: Msg::Reject { reason }, .. } if reason.contains("build"))), "{out:?}");
        assert!(out.iter().any(|o| matches!(o, Out::Close { link: 1, .. })));
    }

    #[test]
    fn a_dropped_peer_leaves_its_slot_empty() {
        let mut w = World::new(&[(1, CALM), (1, CALM)], 1);
        w.run(300);
        w.start();
        w.run(3_000);
        // Machine 2 is unplugged; the host's transport sees its link close.
        w.machines[2].dead = true;
        w.machines[0].core.on_link_down(2, "closed");
        w.run(5_000);
        let left = w.machines[1].events.iter().find_map(|e| match e {
            NetEvent::PeerLeft { peer: 2, from_tick } => *from_tick,
            _ => None,
        });
        let from = left.expect("the others hear it left") as usize;
        w.agree(200);
        let host = &w.machines[0].ran;
        assert!(host[from - 1][2].is_some() && host[from..].iter().all(|b| b[2].is_none()), "slot 2 empty from {from}");
        assert!(w.machines[0].core.waiting_for().is_empty());
    }

    #[test]
    fn a_silent_peer_times_out_and_the_rest_carry_on() {
        let mut w = World::new(&[(1, CALM), (1, CALM)], 1);
        w.run(300);
        w.start();
        w.run(2_000);
        let before = w.machines[0].ran.len();
        w.machines[2].dead = true;
        // Stalled meanwhile, waiting for machine 2.
        w.run(1_000);
        assert_eq!(w.machines[0].core.waiting_for(), vec![2]);
        assert!(w.machines[1].core.waiting_for().contains(&2), "{:?}", w.machines[1].core.waiting_for());
        w.run(6_000);
        assert!(w.machines[0].events.iter().any(|e| matches!(e, NetEvent::PeerLeft { peer: 2, .. })));
        assert!(w.machines[0].ran.len() > before + 50);
        w.agree(100);
    }

    #[test]
    fn a_mismatched_checksum_is_a_desync_for_everyone() {
        let mut w = World::new(&[(1, CALM), (1, CALM)], 1);
        w.machines[2].lie_at = Some(60);
        w.run(300);
        w.start();
        w.run(5_000);
        for (k, m) in w.machines.iter().enumerate() {
            let desyncs: Vec<Tick> = m.events.iter().filter_map(|e| match e {
                NetEvent::Desync { tick } => Some(*tick),
                _ => None,
            }).collect();
            assert_eq!(desyncs, vec![60], "machine {k}");
        }
    }

    #[test]
    fn control_messages_reach_their_targets() {
        let mut w = World::new(&[(1, CALM), (1, CALM)], 1);
        w.run(300);
        w.machines[1].core.send_control(Target::All, b"hi all".to_vec());
        w.machines[1].core.send_control(Target::Peer(2), b"hi two".to_vec());
        w.machines[2].core.send_control(Target::Host, b"to host".to_vec());
        w.machines[0].core.send_control(Target::Peer(1), b"from host".to_vec());
        w.run(300);
        let got = |k: usize| -> Vec<(PeerId, Vec<u8>)> {
            w.machines[k].events.iter().filter_map(|e| match e {
                NetEvent::Control { from, bytes } => Some((*from, bytes.clone())),
                _ => None,
            }).collect()
        };
        assert_eq!(got(0), vec![(1, b"hi all".to_vec()), (2, b"to host".to_vec())]);
        assert_eq!(got(1), vec![(0, b"from host".to_vec())]);
        assert_eq!(got(2), vec![(1, b"hi all".to_vec()), (1, b"hi two".to_vec())]);
    }

    #[test]
    fn raising_the_delay_mid_game_keeps_everyone_together() {
        let mut w = World::new(&[(1, Weather { latency_ms: 30, jitter_ms: 20, loss: 0.1 })], 1);
        w.run(300);
        w.start();
        w.run(2_000);
        w.machines[0].core.raise_delay(6);
        w.run(5_000);
        assert_eq!(w.machines[1].core.delay(), 6);
        assert!(w.machines[1].events.iter().any(|e| matches!(e, NetEvent::DelayChanged { delay: 6 })));
        w.agree(150);
    }

    #[test]
    fn the_host_raises_the_delay_for_a_slow_link() {
        let mut w = World::new(&[(1, Weather { latency_ms: 90, jitter_ms: 0, loss: 0.0 })], 1);
        for m in &mut w.machines {
            m.core.cfg.auto_delay = true;
        }
        w.run(300);
        w.start();
        w.run(4_000);
        // A 180 ms round trip needs 6 ticks.
        assert_eq!(w.machines[0].core.delay(), 6);
        assert_eq!(w.machines[1].core.delay(), 6);
        w.agree(60);
    }

    #[test]
    fn a_leaving_host_disconnects_its_clients() {
        let mut w = World::new(&[(1, CALM)], 1);
        w.run(300);
        w.start();
        w.run(500);
        w.machines[0].core.leave();
        w.run(200);
        let ev = &w.machines[1].events;
        assert!(ev.iter().any(|e| matches!(e, NetEvent::PeerLeft { peer: HOST, .. })));
        assert!(ev.iter().any(|e| matches!(e, NetEvent::Disconnected(_))));
        let t = w.machines[1].tick;
        assert!(w.machines[1].core.ready_inputs(t).is_none());
    }
}
