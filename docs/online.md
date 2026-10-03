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
   `GDL2-` (the protocol) + base32 of the host's endpoint address — its endpoint id, its
   home relay URL and its direct addresses — plus a CRC-16. About 150
   characters with a relay and two addresses. Lowercase, spaces and line
   breaks are accepted when pasted; a damaged code says so rather than
   dialling; a code from another protocol version says *"this invite is for
   network protocol 3; this build speaks protocol 2"*.
   `host()` waits up to 8 s for a relay (so the code can carry one); offline
   it gives a code with direct addresses only, fine on a LAN.
   `session.invite()` is kept current (a relay found later, a network
   change).
2. `session.short_invite()` is a ~60-character code of the endpoint id
   alone: the friend's iroh finds the addresses through n0's DNS. Both
   sides need `Relays::Public`; the host is findable a moment after it
   comes online.
3. The friend calls `NetSession::join(code, cfg)`: it returns at once, and
   `NetEvent::Connected { you, slots, late }` or `NetEvent::Failed(reason)`
   follows (unreachable, the game full, another build).
4. The lobby is the game's: `PeerJoined` / `PeerLeft` events, `roster()`,
   and control messages for anything else (ready flags, character picks,
   which level).
5. The host calls `start()`. A machine joining after that is welcomed
   `late`: its slots are kept for it, out of the game, until the host's
   next `restart()` (below).

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
  goes silent for `timeout` (10 s by default; the game sets 4 s) is
  dropped by the host; its slots are
  `None` from the first tick the host hadn't closed, and everyone gets
  `PeerLeft { peer, from_tick }`. When the host leaves, clients get
  `PeerLeft { peer: 0 }` and `Disconnected`.
- **Checksums**: every `checksum_interval` (30) ticks each machine reports
  a hash of its state; clients send theirs to the host, which compares and
  sends `Desync { tick }` to everyone (itself included) on a mismatch.
- **Restarting**: the host's `restart()` starts lockstep again from tick 0
  for everyone (`Restart { delay, epoch }`, then `NetEvent::Restarted`):
  late joiners' kept slots join the bundles, every machine forgets the run's
  inputs, bundles and checksums, and the run's number (`epoch`) rides on
  every input, bundle and checksum packet, so a straggler from the run
  before is dropped. The game sends its own message first saying where
  everyone starts from (in the game below).

## Messages

Reliable (the stream), in order:

| message | way | meaning |
| --- | --- | --- |
| `Hello { protocol, game, name, players }` | C → H | first frame: who and how many pads |
| `Welcome { you, delay, late, roster }` | H → C | its peer id, the delay, whether the game is under way, everyone in the game |
| `Reject { reason }` | H → C | full, other protocol or build; then closed |
| `Joined(peer)`, `Left { peer, from }` | H → C | the roster changed |
| `Start { delay }` | H → C | lockstep begins at tick 0 |
| `Restart { delay, epoch }` | H → C | lockstep begins again at tick 0, run `epoch` |
| `Delay { delay }` | H → C | the delay went up |
| `Control { peer, bytes }` | both | game message; to the host `peer` is the target (`0xFF` all, `0` host, else a peer), from the host it's the sender |
| `Checksum { epoch, tick, value }` | C → H | state hash |
| `Desync { tick }` | H → C | checksums differed |
| `Leave` | both | leaving for good |

Unreliable (datagrams): `Inputs`, `Bundles` (above, each with its run's
`epoch`), `Ping { id }`, `Pong { id }`.

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

- 4 player slots; at most 7 clients (peer ids 1–7). Joining after
  `start()` waits for the host's `restart()`; a dropped machine's slots stay
  empty until someone joins into them (a player who dropped out can join
  again that way).
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
The fixed tick and `NetTick` run on one thread, in the schedule's own order:
with several threads, systems not ordered among themselves would run in
whichever order they came free, which can differ between machines.
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

**In play.** Start opens *Online Game*: Settings, Camera, Invite, Quit
Level, Leave Game, and in the tower Shop, Inventory and Manage Character
too; play goes on underneath. **Quit Level** is the host's: the command
rides with its controls (`SlotInput::QUIT_LEVEL`) and on the tick that
takes it every machine leaves the level for the tower, as Quit Level does
in a game on one machine (a client's says "Only the host can quit the
level"). In first person a screen shows every player's panel in its slot
(not the empty ones). Dead heroes are out until the level ends, as the original's (their
gold from the level goes with the restored record). The screen says who the
game waits for after half a second and notices who left.

**Leaving.** Leave Game outside the tower asks with a warning: "Leaving
before reaching the tower will make you lose all progress in this level
for this character." The player who leaves plays on alone in their own
tower, not back at the title (`frontend::play_on_alone`): from a level
under way with the record their hero began the level with (the level
start's `Snapshot`), from the tower with the record as it is. The same
when the online game ends under them — the host left, or the network
went. A player who leaves — or quits the game, which drops
the session — says goodbye, and the host's ticks go straight on: checked
over loopback, 30 a second through the departure (the leaver's slot is
empty from the tick the host had reached). A machine that goes silent
(crashed, killed, its network gone) holds the others under "Waiting for
Player N..." until the session's timeout drops it; the game sets that to
4 s (`SILENT_LEAVES`), down from 10 — the network thread pings every
250 ms however busy the game is, so only a dead machine is that quiet.
Measured: a killed client held the host 4.8 s.

**Level intros.** The loading screens and movies play online too
([frontend.md](frontend.md), "Loading screens and movies"): no tick runs
while any machine is in one, and only the host skips a movie, for
everyone (`Message::SkipMovie`). The build string the machines compare
(`game_version`) carries the disc's game ID and revision, so a Rev 0 and
a Rev 1 copy don't play together.

**Shops.** A player's Shop or Inventory opens the screen for everyone: the
command rides with their controls for a few frames (`SlotInput::OPEN_SHOP`,
`OPEN_INVENTORY`) and the screen opens on the tick that takes it; the shop
screens (the after-level screen too) then run on the ticks with each
player's controls from the bundle (up/down, A buys, X sells, B to EXIT).

**Starting again** (`Message::Resync`, then the network's `restart()`).
The host sends every machine the party — each player's record as the
host's game has it, as a saved character — and where to start; every
machine builds that party, starts the game afresh (`NewGame`) and loads
the level, and lockstep runs again from tick 0. Ticks wait meanwhile (from
the resync until the front end has acted on it and the restart is in). It
starts again:

- **when someone joins the game under way**: their select screen shows only
  their own column ("The game is under way: you join at the tower"); with
  their hero ready they wait until the party is back in the tower, and
  everyone starts again there with them;
- **when a player changes hero** (Manage Character in the tower: Change,
  Load, Save, Done — their own column while the others play on): everyone
  starts again in the tower with the new hero; Quit leaves the game;
- **when the machines aren't on the same level** at a sync point (below):
  the one case their games can't be put together where they are.

**Sync points** (`resync.rs`). Every 30 ticks the machines compare a hash
of the game. Should they ever differ, nothing starts again: the host calls
a sync point. Its controls carry `SlotInput::SYNC`, and every machine
stops before the first tick whose bundle has it — the same tick everywhere,
whatever the network does. Then

1. each machine sends the host a report: its own players' heroes as its
   game has them (record, place, action, clip), what it has of the monsters
   (which are alive, which generators stand), its items and its heroes'
   cameras;
2. the host puts one game together: **each hero from its own player's
   machine** — nobody's hero moves under them, and nobody loses what they
   saw their hero get — with what only grows on a level (experience, quest
   pieces, runestones, tallies) at its most between the two copies; the
   host's monsters less those dead on any machine, a generator broken on
   any machine broken; the host's items, with a powerup taken and a door or
   key chest opened on any machine taken or opened; the host's critters and
   level counters; each hero's own camera from its machine;
3. every machine, the host too, takes that game over (as the same text,
   read back) and plays on from the tick it stopped before. Missiles in
   flight go; monsters are made again from the list, so every machine has
   the same ones in the same order.

The wait is a round trip to the host and four frames (measured over
loopback: 20–65 ms and the frames). A sync point that follows the last
within 150 ticks didn't hold — the games differ in something it doesn't
carry — and the host waits longer before the next (2 s doubling to 30 s),
so such games never stop every second. Not carried yet: a barrel or wall
broken, what a chest let out in another order, the movers
(`mechanics.rs`), a critter's move under way, the opening shot, cuts and
the boss camera.

**The lag sign** (`lag_sign.rs`). All the screen shows of the network: a
small medallion in the top right corner — a red gem sending out three
golden waves, lit one after another — while a tick has waited a quarter
of a second on another machine, and through a sync point. Painted in
code; no text.

**Testing.** `tools/online_test.sh [seconds]` runs a host and a client on
this machine over loopback (`GDL_NET_LOCAL=1`, the invite through
`GDL_INVITE_FILE`), heroes picked by `GDL_ONLINE_HERO`, started by
`GDL_ONLINE_PLAYERS=2`, scripted sticks; it compares their hashes tick by
tick. `GDL_ONLINE_LEVEL=levelA1` starts the game on a level; `HOST_PLAYERS=1`
starts the host alone, so the client joins the game under way;
`CLIENT_MENU` scripts the client's menus (Manage Character);
`CLIENT_DESYNC_AT=<tick>[:<what>]` puts the client's game out of sync then
(`GDL_DESYNC_AT`: `coin`, `hero`, `monster`, `kill`, `generator`, `item`,
`door`, `chest`); a sync point must put the games together, with no level
starting again, and every check after it must agree. `FIGHT=1`
hops the heroes among levelA1's grunts (`HOPS`) and has them throw every 9
ticks; `HOST_PREFIX` / `CLIENT_PREFIX` put a command before a side's game
(`taskpolicy -b`: slowed down).

**What a tick may read.** Only the game's own state, never where
something was last drawn. Between ticks everything the game moves is drawn
part-way from the tick before's state to the last one's
(`tick_places::Between`, the part of a tick gone by when the frame is
drawn), and that part depends on when each machine draws: two machines
never have the same. The tick's systems read transforms — a hero's aim and
blows find monsters by theirs (`combat::search`), a boss's blows and
missiles start from its bones, a held missile sits on its node — so as
each tick starts (`FixedPreUpdate`, `TickPlaces`) the same systems that
place things for drawing place them with `Between` at 1, where the last
tick left them, and Bevy's transform propagation runs then too, down to
the bones; the frame's own placing and propagation follow as before. The
clips' frames and a blend into a clip move on with the ticks online
(`character::advance_clips`), so the bones' own poses are the tick's too.

Before this the aim read the drawn places: in a fight the machines
disagreed within seconds (a throw aimed a hair apart) and the level
started again, over and over. `FIGHT=1 tools/online_test.sh` is that
fight, and `CLIENT_PREFIX="taskpolicy -b"` slows one side down (macOS) so
the two draw unevenly.

**The same maths everywhere.** Lockstep needs the same floating-point
results on every machine, a Mac's ARM processor and a PC's Intel one
alike:

- The game's sines, cosines, arctangents and powers come from `libm`
  (`gdl_formats::detmath::Det`: `x.dsin()` …) and glam's from Bevy's
  `libm` feature: the systems' own maths libraries round differently.
- glam works on four numbers at a time with the processor's own
  instructions, written apart for Intel and ARM. Read side by side (glam
  0.30.10, `f32/sse2` and `f32/neon`): products and sums of vectors,
  matrices and quaternions, cross products, the three-number dot product
  and length, and a rotation applied to a vector do the same steps in the
  same order on both — so do Bevy's transforms, which use only those. But
  the four-number dot product sums in a different order (a quaternion's
  or `Vec4`'s length, `normalize`, `Quat::lerp`, `from_rotation_arc`, a
  frustum's sphere test), `Vec3A::normalize` divides on one and multiplies
  by the reciprocal on the other, and the Intel `Quat::slerp` takes its
  sines from its own approximation. The game's `rotations.rs` has `slerp`
  and `arc` a number at a time, and `monsters::on_screen` is the frustum
  test; `Vec3` and `Vec2` are plain numbers everywhere.
- `f32::max` and `min` may return either of two equal inputs, and the
  processors differ on `-0.0` against `0.0`: the state hash takes every
  float through `detmath::sync_bits` (the zeros alike, every NaN alike),
  and `datan2` ignores a zero's sign.
- `crates/gdl-formats/tests/no_platform_maths.rs` keeps the platform's
  functions and those of glam's out of `crates/`.

(glam's `scalar-math` feature would make all of it plain numbers, but
Bevy 0.18's reflection doesn't build with it.)
