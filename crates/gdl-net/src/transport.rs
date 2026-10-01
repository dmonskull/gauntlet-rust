//! The iroh side: a background thread running a tokio runtime, an
//! endpoint, and a task per connection. It feeds what arrives to the
//! lockstep engine and sends what the engine asks for; the game reaches
//! the engine through the shared mutex.
//!
//! Each connection has one bidirectional stream, opened by the client
//! (whose first frame is its hello): reliable messages as frames of a
//! `u32` length and the message. Lockstep packets go as QUIC datagrams,
//! or as frames on the stream when the connection has no datagrams or a
//! packet is too big for one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, RelayMode};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::lockstep::{Core, HOST_LINK, LinkId, Out};
use crate::wire::Msg;
use crate::{ALPN, NetConfig, NetError, Relays, invite};

/// Bytes a lockstep datagram may take: under the "little over a
/// kilobyte" QUIC guarantees on any path.
pub(crate) const PACKET_BUDGET: usize = 1000;
/// The longest a reliable frame may be.
const MAX_FRAME: usize = 1 << 20;
/// How long the host waits for a relay before giving out an invite with
/// direct addresses only (the invite updates when one turns up).
const ONLINE_WAIT: Duration = Duration::from_secs(8);
/// Reaching the host, and a new connection's handshake.
const CONNECT_WAIT: Duration = Duration::from_secs(20);
/// The engine is polled at least this often.
const POLL_EVERY: Duration = Duration::from_millis(5);
/// Leaving: how long the goodbyes get.
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// What the game's thread and the network thread share.
pub(crate) struct Shared {
    pub(crate) core: Mutex<Core>,
    /// Wakes the network thread to send now (an input committed, a
    /// control message queued).
    pub(crate) wake: Notify,
    /// The host's invite codes: the full one, and the one of its endpoint
    /// id alone.
    pub(crate) invite: Mutex<Option<String>>,
    pub(crate) short_invite: Mutex<Option<String>>,
}

impl Shared {
    pub(crate) fn new(core: Core) -> Self {
        Shared { core: Mutex::new(core), wake: Notify::new(), invite: Mutex::new(None), short_invite: Mutex::new(None) }
    }

    fn core(&self) -> std::sync::MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The network thread, for the session to stop.
pub(crate) struct Driver {
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Option<oneshot::Sender<()>>,
}

impl Driver {
    /// Leaves the game and waits for the thread (bounded by `CLOSE_WAIT`).
    pub(crate) fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// What the connection tasks tell the main loop.
enum LinkEvent {
    Up { link: LinkId, conn: Connection, frames: mpsc::UnboundedSender<Vec<u8>> },
    Packet { link: LinkId, bytes: Vec<u8> },
    Down { link: LinkId, reason: String },
    ConnectFailed(String),
}

struct LinkHandle {
    conn: Connection,
    frames: mpsc::UnboundedSender<Vec<u8>>,
}

fn runtime() -> Result<tokio::runtime::Runtime, NetError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("gdl-net-io")
        .enable_all()
        .build()
        .map_err(|e| NetError::Setup(format!("tokio runtime: {e}")))
}

async fn bind(cfg: &NetConfig, host: bool) -> Result<Endpoint, NetError> {
    let mut b = match cfg.relays {
        Relays::Public => Endpoint::builder(presets::N0),
        Relays::Disabled => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
    };
    if host {
        b = b.alpns(vec![ALPN.to_vec()]);
    }
    if let Some(addr) = cfg.bind {
        b = b.clear_ip_transports().bind_addr(addr).map_err(|e| NetError::Setup(format!("bind address {addr}: {e}")))?;
    }
    b.bind().await.map_err(|e| NetError::Setup(format!("couldn't open the network endpoint: {e}")))
}

/// Starts the host's thread; returns once its invite code is known.
pub(crate) fn spawn_host(cfg: &NetConfig, shared: Arc<Shared>) -> Result<(Driver, String), NetError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<String, NetError>>();
    let (stop_tx, stop_rx) = oneshot::channel();
    let cfg = cfg.clone();
    let thread = std::thread::Builder::new()
        .name("gdl-net".into())
        .spawn(move || {
            let rt = match runtime() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            rt.block_on(async move {
                let ep = match bind(&cfg, true).await {
                    Ok(ep) => ep,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                if cfg.relays == Relays::Public {
                    let _ = tokio::time::timeout(ONLINE_WAIT, ep.online()).await;
                }
                let code = invite::encode(&ep.addr());
                *shared.invite.lock().unwrap_or_else(|e| e.into_inner()) = Some(code.clone());
                *shared.short_invite.lock().unwrap_or_else(|e| e.into_inner()) = Some(invite::encode(&EndpointAddr::new(ep.id())));
                let _ = ready_tx.send(Ok(code));
                run(ep, shared, stop_rx, None).await;
            });
        })
        .map_err(|e| NetError::Setup(format!("network thread: {e}")))?;
    let code = ready_rx.recv().map_err(|_| NetError::Setup("the network thread stopped".into()))??;
    Ok((Driver { thread: Some(thread), stop: Some(stop_tx) }, code))
}

/// Starts a client's thread, which dials the host.
pub(crate) fn spawn_client(cfg: &NetConfig, host: EndpointAddr, shared: Arc<Shared>) -> Result<Driver, NetError> {
    let (stop_tx, stop_rx) = oneshot::channel();
    let cfg = cfg.clone();
    let thread = std::thread::Builder::new()
        .name("gdl-net".into())
        .spawn(move || {
            let rt = match runtime() {
                Ok(rt) => rt,
                Err(e) => {
                    shared.core().on_connect_failed(e.to_string());
                    return;
                }
            };
            rt.block_on(async move {
                match bind(&cfg, false).await {
                    Ok(ep) => run(ep, shared, stop_rx, Some(host)).await,
                    Err(e) => shared.core().on_connect_failed(e.to_string()),
                }
            });
        })
        .map_err(|e| NetError::Setup(format!("network thread: {e}")))?;
    Ok(Driver { thread: Some(thread), stop: Some(stop_tx) })
}

/// The network thread's main loop: connections in, packets both ways, the
/// engine polled every few milliseconds and whenever the game wakes it.
async fn run(ep: Endpoint, shared: Arc<Shared>, mut stop: oneshot::Receiver<()>, dial: Option<EndpointAddr>) {
    let host = dial.is_none();
    let (tx, mut rx) = mpsc::unbounded_channel::<LinkEvent>();
    let mut links: HashMap<LinkId, LinkHandle> = HashMap::new();
    if let Some(addr) = dial {
        tokio::spawn(dial_host(ep.clone(), addr, tx.clone()));
    }
    let mut next_link: LinkId = HOST_LINK + 1;
    let mut poll = tokio::time::interval(POLL_EVERY);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_addr_check = Instant::now();
    loop {
        tokio::select! {
            _ = &mut stop => break,
            incoming = ep.accept(), if host => match incoming {
                Some(incoming) => {
                    tokio::spawn(accept_link(incoming, next_link, tx.clone()));
                    next_link += 1;
                }
                None => break,
            },
            Some(ev) = rx.recv() => handle(ev, &shared, &mut links),
            _ = poll.tick() => {}
            _ = shared.wake.notified() => {}
        }
        // Everything else that has arrived, then what to send.
        while let Ok(ev) = rx.try_recv() {
            handle(ev, &shared, &mut links);
        }
        let out = shared.core().poll(Instant::now());
        dispatch(out, &mut links);
        // The host's address can change (a relay found late, a new
        // network): keep the invite current.
        if host && last_addr_check.elapsed() >= Duration::from_secs(1) {
            last_addr_check = Instant::now();
            let code = invite::encode(&ep.addr());
            let mut current = shared.invite.lock().unwrap_or_else(|e| e.into_inner());
            if current.as_deref() != Some(code.as_str()) {
                *current = Some(code);
            }
        }
    }
    // Leaving: the goodbyes, a moment for them to go, then the endpoint.
    let out = {
        let mut core = shared.core();
        core.leave();
        core.poll(Instant::now())
    };
    dispatch(out, &mut links);
    // (A lost goodbye costs nothing: the closed connection says the same.)
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = tokio::time::timeout(CLOSE_WAIT, ep.close()).await;
}

fn handle(ev: LinkEvent, shared: &Shared, links: &mut HashMap<LinkId, LinkHandle>) {
    let now = Instant::now();
    match ev {
        LinkEvent::Up { link, conn, frames } => {
            links.insert(link, LinkHandle { conn, frames });
            shared.core().on_link_up(link, now);
        }
        LinkEvent::Packet { link, bytes } => {
            if let Some(msg) = Msg::decode(&bytes) {
                shared.core().on_message(link, msg, now);
            }
        }
        LinkEvent::Down { link, reason } => {
            links.remove(&link);
            shared.core().on_link_down(link, &reason);
        }
        LinkEvent::ConnectFailed(reason) => shared.core().on_connect_failed(reason),
    }
}

fn dispatch(out: Vec<Out>, links: &mut HashMap<LinkId, LinkHandle>) {
    for o in out {
        match o {
            Out::Send { link, msg } => {
                let Some(h) = links.get(&link) else { continue };
                let bytes = msg.encode();
                let as_datagram = !msg.reliable() && h.conn.max_datagram_size().is_some_and(|max| bytes.len() <= max);
                if as_datagram && h.conn.send_datagram(Bytes::from(bytes.clone())).is_ok() {
                    continue;
                }
                let _ = h.frames.send(bytes);
            }
            Out::Close { link, .. } => {
                // Dropping the frame sender lets the writer flush what's
                // queued (a refusal, a goodbye) and then close.
                links.remove(&link);
            }
        }
    }
}

async fn dial_host(ep: Endpoint, addr: EndpointAddr, tx: mpsc::UnboundedSender<LinkEvent>) {
    let conn = match tokio::time::timeout(CONNECT_WAIT, ep.connect(addr, ALPN)).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => {
            let _ = tx.send(LinkEvent::ConnectFailed(format!("couldn't reach the host: {e}")));
            return;
        }
        Err(_) => {
            let _ = tx.send(LinkEvent::ConnectFailed("couldn't reach the host: timed out".into()));
            return;
        }
    };
    match conn.open_bi().await {
        Ok((send, recv)) => start_link(conn, send, recv, HOST_LINK, tx),
        Err(e) => {
            let _ = tx.send(LinkEvent::ConnectFailed(format!("couldn't open a stream to the host: {e}")));
        }
    }
}

async fn accept_link(incoming: Incoming, link: LinkId, tx: mpsc::UnboundedSender<LinkEvent>) {
    let Ok(Ok(conn)) = tokio::time::timeout(CONNECT_WAIT, incoming).await else { return };
    if conn.alpn() != ALPN {
        conn.close(1u32.into(), b"wrong protocol");
        return;
    }
    // The client's first frame (its hello) opens the stream here.
    match tokio::time::timeout(CONNECT_WAIT, conn.accept_bi()).await {
        Ok(Ok((send, recv))) => start_link(conn, send, recv, link, tx),
        _ => conn.close(1u32.into(), b"no hello"),
    }
}

/// Announces the link, then starts its reader, datagram reader, writer and
/// close watcher (in that order, so the engine knows the link before
/// anything from it arrives).
fn start_link(conn: Connection, send: SendStream, recv: RecvStream, link: LinkId, tx: mpsc::UnboundedSender<LinkEvent>) {
    let (frames_tx, frames_rx) = mpsc::unbounded_channel();
    if tx.send(LinkEvent::Up { link, conn: conn.clone(), frames: frames_tx }).is_err() {
        return;
    }
    tokio::spawn(write_frames(send, frames_rx, conn.clone()));
    tokio::spawn(read_frames(recv, link, tx.clone()));
    tokio::spawn(read_datagrams(conn.clone(), link, tx.clone()));
    tokio::spawn(async move {
        let reason = conn.closed().await;
        let _ = tx.send(LinkEvent::Down { link, reason: reason.to_string() });
    });
}

async fn write_frames(mut send: SendStream, mut frames: mpsc::UnboundedReceiver<Vec<u8>>, conn: Connection) {
    while let Some(f) = frames.recv().await {
        let len = (f.len() as u32).to_le_bytes();
        if send.write_all(&len).await.is_err() || send.write_all(&f).await.is_err() {
            return;
        }
    }
    // The link was closed on this side: let the last frames land, then go.
    let _ = send.finish();
    let _ = tokio::time::timeout(CLOSE_WAIT, send.stopped()).await;
    conn.close(0u32.into(), b"bye");
}

async fn read_frames(mut recv: RecvStream, link: LinkId, tx: mpsc::UnboundedSender<LinkEvent>) {
    loop {
        let mut len = [0u8; 4];
        if recv.read_exact(&mut len).await.is_err() {
            return;
        }
        let n = u32::from_le_bytes(len) as usize;
        if n > MAX_FRAME {
            return;
        }
        let mut bytes = vec![0; n];
        if recv.read_exact(&mut bytes).await.is_err() || tx.send(LinkEvent::Packet { link, bytes }).is_err() {
            return;
        }
    }
}

async fn read_datagrams(conn: Connection, link: LinkId, tx: mpsc::UnboundedSender<LinkEvent>) {
    while let Ok(d) = conn.read_datagram().await {
        if tx.send(LinkEvent::Packet { link, bytes: d.to_vec() }).is_err() {
            return;
        }
    }
}
