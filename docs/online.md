# Online co-op (`crates/gdl-net`)

Up to four players on up to four machines over the internet, no port
forwarding. Each machine runs the whole game and renders its own camera;
the simulation is deterministic **lockstep**, so the network carries only
inputs, a few control messages and checksums. The crate is self-contained
(no Bevy, no game types): the game integrates it through a small sync API.

## Transport

[iroh](https://docs.rs/iroh/1.3.0) 1.3: peer-to-peer QUIC dialled by public
key, with hole punching and n0's public relay servers as fallback (the
`presets::N0` endpoint: n0 relays plus n0's DNS address lookup). ALPN
`gdl-coop/1`.

- **Star topology.** One machine hosts; clients connect to the host only
  and the host relays everything between them.
- Per connection: one bidirectional QUIC stream, opened by the client
  (its hello is the first frame), carrying the reliable messages as frames
  (`u32` length + message); and QUIC **datagrams** for the lockstep
  traffic. A lockstep packet goes on the stream instead when the
  connection has no datagrams or the packet is bigger than
  `max_datagram_size()` (never with the default budget).
- tokio and iroh run on a background thread (`gdl-net`, a two-worker
  runtime). The game's side is plain sync Rust behind a mutex; the network
  thread polls the engine every 5 ms and at once when the game commits an
  input or queues a message.

## Inviting and joining

1. The host calls `NetSession::host(cfg)` and gets an **invite code**:
   `GDL1-` + base32 of the host's endpoint address — its endpoint id, its
   home relay URL and its direct addresses — plus a CRC-16. About 150
   characters with a relay and two addresses. Lowercase, spaces and line
   breaks are accepted when pasted; a damaged code says so rather than
   dialling; a code from another protocol version says *"this invite is for
   network protocol 2; this build speaks protocol 1"*.
   `host()` waits up to 8 s for a relay (so the code can carry one); offline
   it gives a code with direct addresses only, fine on a LAN.
   `session.invite()` is kept current (a relay found later, a network
   change).
2. `session.short_invite()` is a ~60-character code of the endpoint id
   alone: the friend's iroh finds the addresses through n0's DNS. Both
   sides need `Relays::Public`; the host is findable a moment after it
   comes online.
3. The friend calls `NetSession::join(code, cfg)`: it returns at once, and
   `NetEvent::Connected { you, slots }` or `NetEvent::Failed(reason)`
   follows (unreachable, the game full or started, another build).
4. The lobby is the game's: `PeerJoined` / `PeerLeft` events, `roster()`,
   and control messages for anything else (ready flags, character picks,
   which level).
5. The host calls `start()`. No one joins after that.

## Lockstep

Ticks run at 30 Hz; every machine simulates tick `T` with the same
`[Option<PlayerInput>; 4]`.

- **Slots** 0–3. The host takes the first `local_players`, then each
  joining machine gets as many as its `local_players` asks for (two pads on
  one machine: two slots). Empty slots are `None` in every bundle.
- **Input delay `D`** (default 3 ticks, 100 ms). When the game simulates
  tick `t`, this machine's current pad samples are committed for tick
  `t + D` and sent. Ticks `0..D` are idle (neutral input) everywhere. The
  host can raise `D` (`raise_input_delay`, or `auto_delay` from measured
  round trips: `D ≥ (rtt + 15 ms) / 33.3 ms`, up to `max_input_delay`); it
  never comes down. A machine that raises its delay repeats its current
  input over the ticks the raise opens up. Machines may briefly run with
  different `D` — harmless, the host only collects "slot s, tick T".
- **Clients → host**: `Inputs { ack, base, slots, inputs }` datagrams, the
  client's inputs for every tick from the first the host hasn't
  acknowledged, so a lost datagram is covered by the next one.
- **Host → clients**: once every occupied slot's input for tick `T` is in
  (the host's own included), the bundle for `T` is final. `Bundles { ack,
  base, waiting, bundles }` datagrams repeat every bundle the client
  hasn't acknowledged. `ack` acknowledges the client's inputs, `waiting`
  lists the machines the host's next bundle waits for.
- Packets go when there's something new (an input, a bundle, an
  acknowledgement) and are repeated every 30 ms while anything is
  unacknowledged. Round trips are measured with pings every 250 ms.
- **Everyone, the host too, advances tick `T` only when its bundle is
  known.** A late bundle just holds the game; `waiting_for()` names who it
  waits for (the machines whose inputs the host lacks, or the host when the
  bundles are what's slow).
- **Leaving**: a machine that leaves (`leave()`, dropping the session) or
  goes silent for `timeout` (10 s) is dropped by the host; its slots are
  `None` from the first tick the host hadn't closed, and everyone gets
  `PeerLeft { peer, from_tick }`. When the host leaves, clients get
  `PeerLeft { peer: 0 }` and `Disconnected`.
- **Checksums**: every `checksum_interval` (30) ticks each machine reports
  a hash of its state; clients send theirs to the host, which compares and
  sends `Desync { tick }` to everyone (itself included) on a mismatch.

## Messages

Reliable (the stream), in order:

| message | way | meaning |
| --- | --- | --- |
| `Hello { protocol, game, name, players }` | C → H | first frame: who and how many pads |
| `Welcome { you, delay, roster }` | H → C | its peer id, the delay, everyone in the game |
| `Reject { reason }` | H → C | full, started, other protocol or build; then closed |
| `Joined(peer)`, `Left { peer, from }` | H → C | the roster changed |
| `Start { delay }` | H → C | lockstep begins at tick 0 |
| `Delay { delay }` | H → C | the delay went up |
| `Control { peer, bytes }` | both | game message; to the host `peer` is the target (`0xFF` all, `0` host, else a peer), from the host it's the sender |
| `Checksum { tick, value }` | C → H | state hash |
| `Desync { tick }` | H → C | checksums differed |
| `Leave` | both | leaving for good |

Unreliable (datagrams): `Inputs`, `Bundles` (above), `Ping { id }`,
`Pong { id }`.

Encoding: little-endian, a tag byte per message; an input is 8 bytes
(stick x, y, c-stick x, y as `i8`, buttons `u32`); a bundle is a slot mask
byte plus the present inputs (at most 33 bytes). Anything that doesn't
decode exactly is dropped.

## From the game (Bevy)

Keep the `NetSession` in a resource (it's `Send + Sync`) and the next tick
to simulate in another. In the fixed update (30 Hz):

```rust
fn net_tick(net: Res<Net>, mut sim: ResMut<SimTick>, pads: Res<LocalPads>, /* game state */) {
    for event in net.session.poll_events() { /* lobby, control, desync, disconnect */ }
    for slot in net.session.local_slots() {
        net.session.set_local_input(slot, pads.quantized(slot));
    }
    if let Some(inputs) = net.session.ready_inputs(sim.0) {
        // run the game's tick with inputs[0..4] (None: nobody in that slot)
        net.session.report_checksum(sim.0, /* hash of the game state */ 0);
        sim.0 += 1;
    } else {
        // show "waiting for …" from net.session.waiting_for()
    }
}
```

- The simulation must step **only** through `ready_inputs`, never because
  a FixedUpdate came round: a stall is time standing still for everyone.
  After a stall a machine can catch up by simulating more than one ready
  tick in a frame.
- Everything in the tick must be deterministic and fed only by the bundle:
  no wall-clock time, no unseeded randomness, no iteration over hash maps,
  identical float maths (the same build on every machine — builds that
  differ refuse each other through `game_version`).
- Each machine draws its own camera; local-only things (menus, sounds,
  particles that don't touch the state) can do what they like.

## Limits

- 4 player slots; at most 7 clients (peer ids 1–7). No joining after
  `start()`, and no reconnecting: a dropped machine's slots stay empty.
- The host is the hub: its uplink carries every client's bundles (about
  30 packets/s each, ~40–1000 bytes), and when it leaves the game ends.
- Lockstep packets are capped at 1000 bytes (29 bundles of four players,
  or 123 ticks of one client's single pad); iroh reported a
  `max_datagram_size()` of 1162 bytes on fresh connections (the QUIC
  minimum path MTU less overhead; MTU discovery can raise it). A client
  more than 29 ticks behind catches up 29 bundles per packet.
- Through a relay (when hole punching fails) round trips grow, so `D`
  goes up with `auto_delay`; every extra tick of delay is 33 ms of input
  lag for everyone.
- `NetSession::host` blocks up to 8 s waiting for a relay; dropping a
  session blocks up to 2 s for the goodbyes.
- The short code depends on n0's DNS service; the long one only on the
  relays (or on direct addresses).

## Tests

`cargo test -p gdl-net`: the engine with a simulated network (latency,
jitter, 25 % datagram loss and reordering, three machines with two pads on
one, a dropped and a silent machine, checksum mismatch, control routing,
delay raised by hand and from round trips, the host leaving), the wire
format and invite codes, and a real iroh host and client in one process
over loopback with relays off. `-- --ignored` adds a test through n0's
public relays with the long and the short codes.

## In the game (`crates/gdl-game/src/online.rs`)

**Menus.** Title → Start → *Local Game* (the game's own flow) or *Online
Game* → *Host Game* / *Join Game*. Hosting brings the session up on another
thread and copies the invite code to the clipboard (C on the lobby copies
it again); joining takes the code from the clipboard. Either way the lobby
is the select screen: this machine's devices drive its own column (New, or
Load a hero saved on this machine — its record goes to the others), the
other columns show what their players pick (`Message::Column`,
`Message::Hero`). With everyone ready the host presses Start: it sends the
party (`Message::Begin`) and starts lockstep; every machine builds the same
party, writes `NewGame` (what the game keeps between levels starts afresh)
and loads the tower.

**Lockstep.** `Lockstep.on` from then on. Each frame, before the fixed loop,
`drive` sends this machine's controls (`player::sample_online`: its devices,
neutral under a menu) and, when a tick is due by the wall clock and the game
is settled, takes the tick's bundle into `Inputs` and moves the paused
`Time<Virtual>` by exactly what makes the fixed loop run that one tick (and
how far into the next it is, for drawing between ticks). At most one tick a
frame, so the systems that run each frame see the state after every tick;
no tick while a level change settles (`level_work` from `exits.rs` /
`world.rs`, a new `LevelPopulation`, the front end loading: 4 quiet frames).
After the fixed loop the `NetTick` schedule runs: the message box (it
freezes play for everyone — the tick is then the box's alone — and any
player's B puts a page away), the voice queues (2 fields a tick; level
changes wait on them), players whose machine left (gone from the first
tick without their controls), and every 30 ticks the state hash
(`GDL_SYNC_LOG=1` logs its parts).

Online these run on the ticks instead of each frame: a hero's death
(`frontend::death`), the clips' frames (`character::advance_clips`; the
game reads them), the tower's speeches and unlock announcements, the
level-start shot's skip (any player's press). Each player's auto-aim,
auto-attack and Robotron style ride with their controls
(`SlotInput::AUTO_AIM` …), so every machine plays each hero by its own
player's settings; the items' on-screen test uses the game's views, not the
window's camera.

**Cameras.** The host chooses (Start → Camera; `GameOptions::online_cameras`,
riding with its controls as `SlotInput::OWN_CAMERAS`, so every machine
switches on the same tick): **Each Player** (the default) or **Overhead**,
the game's one co-op camera over the level for everyone, as in local co-op.
With each player's own, `PlayCamera` keeps one rig per hero, each following
its hero alone: a hero's stick turns by its own camera, the on-screen tests take
every standing hero's view, and each machine draws its own hero's
(`watching`) — a teammate's while its own is down or out, L / R picking
another. Cuts, the level-start shot and boss cameras stay everyone's.

**In play.** Start opens *Online Game*: Settings, Leave Game; play goes on
underneath. Dead heroes are out until the level ends, as the original's (their
gold from the level goes with the restored record). The screen says who the
game waits for after half a second, notices who left, and warns when the
machines' hashes differ.

**Not online yet**: joining after the start, Manage Character, the shops.

**Testing.** `tools/online_test.sh [seconds]` runs a host and a client on
this machine over loopback (`GDL_NET_LOCAL=1`, the invite through
`GDL_INVITE_FILE`), heroes picked by `GDL_ONLINE_HERO`, started by
`GDL_ONLINE_PLAYERS=2`, scripted sticks; it compares their hashes tick by
tick. `GDL_ONLINE_LEVEL=levelA1` starts the game on a level.

**Machines of different kinds.** Lockstep needs the same floating-point
results everywhere. The game's sines, cosines, arctangents and powers come
from `libm` (`gdl_formats::detmath::Det`: `x.dsin()` …) and Bevy's maths
from its `libm` feature, so a Mac and a Windows PC compute them alike; the
hash check would still warn ("Out of sync") if anything else differed.
