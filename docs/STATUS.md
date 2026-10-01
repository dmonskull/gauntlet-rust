# Status: how close the rewrite is to the original

Assessed 2026-10-01 on `master` after the level audit's second round
(`04b2745`; the first estimate, on `ef1f596`, was 81 % / 76 %). Update
this file when an area moves; [HANDOFF.md](HANDOFF.md) has the
day-to-day state and the job queue.

## The numbers

| measure | estimate |
| --- | --- |
| **Faithful to the original, one player** (what a single hero plays through) | **≈ 82 %** |
| **Done overall** (everything the original does, 2–4 player co-op included, every level checked, no known issues) | **≈ 77 %** |

These are judgements, not measurements. Each area below gets a weight (how
much of the game it is) and a share done (what's confirmed against the
binary and ported, against what's known to be left: the code's 62 labelled
stand-ins and the gaps listed under "What's left"). One player's figure
leaves co-op out.

| area | weight | done | what holds it back |
| --- | --- | --- | --- |
| Disc, files and formats | 6 | 97 % | a few WORLDS/WDATA words, the `SNDS` chunk |
| Level look (geometry, lightmaps, blending, texture animation, particles) | 10 | 88 % | action texture wipes, the hand and weapon glows, effect lights, exact particle maths, shadows |
| Hero movement, camera, cuts | 7 | 95 % | the step cut under the boss camera, co-op framing |
| Hero combat, magic, power-ups | 10 | 88 % | rapid fire's rate, missile streaks, magic element models, the legendary weapon's throw |
| Monsters and generators | 9 | 75 % | most AIs run the chase stand-in; no actor-vs-actor collision |
| Bosses and critters | 9 | 72 % | grabs, effect-slot contact damage, missile lifetimes, the boss key, stumps, the health meter |
| Items, pickups, doors, exits, hazards | 6 | 91 % | random item types, the shop, the secret realm's coins, the keys' turning traced |
| Level mechanics (triggers, lifts, animated objects, secret walls, falls) | 8 | 84 % | bursting objects, E2's debris, subtype 1 rotators; the tour's re-run pending |
| What each level places and hides | 5 | 82 % | 47 of 67 levels not yet audited; the stray objects likely the debug markers (now off) |
| Every level checked start to exit | 5 | 25 % | only A1's exit to A6 checked; triggers toured on A1–C1 and D1–D4 |
| Quest, tower, saving | 5 | 88 % | the secret realm's coins, per-class records, memory card screens |
| Front end, menus, HUD, hints | 7 | 75 % | shop, inventory, options, attract loop, the hints' plates |
| Audio | 5 | 85 % | music switching and ducking, menu sounds, footstep pan |
| Co-op (2–4 players) | 6 | 3 % | not started beyond the data and the waiting panels |
| Speed and stability | 2 | 85 % | the animated objects' cost unmeasured; warm-up hitches |

## What's left

- Online co-op: joining after the start, Manage Character and the shops
  online; recovering from an out-of-sync game (today it only warns).

In order, top first. Each line names its doc, where the decoding is.

### 1. Now: regressions and blockers

Fixed 2026-10-01: A1's lifts 402/403 (a teleport swept the item touch
test across secret walls, `8c39b5f`); the crystal pickup's sparkle that
never ended (effect emitters now stop with their effect, `9aa1881`);
keys that turned once and stopped (`d2fa6d4`, a stand-in: see
[items.md](items.md) "Item animation"); the developer keys (`I` swapped
every item for a debug marker — a likely source of the user's "stray
objects" —, `K`, `C`, `[`/`]`, `M`, `N`, F1) are off unless
`GDL_DEV_KEYS=1` (`04b2745`). The second audit round (`728a4ff`)
explained or fixed every tour lead from A4 to D4: items now drop with
the movers at their start heights and ride them, touches reach from the
hero's centre; C1 433 and D2 307 are unreachable in the original too.

- Re-run the tour on every level on `04b2745` (`tools/tour.sh`; running),
  then the all-levels smoke test (`tools/smoke.sh`).
- Check the secret walls block and break in game (A1: `GDL_WARP=
  "55.5,0.15,59.1"`, A1SHOOTW1; the first try's camera sat inside a
  pillar).
- Look at characters after the track-flag fix (`flags & 0x0FFF`), which
  changed 32 character bones ([animation-format.md](animation-format.md)).
- Measure the frame rate with the 1,608 animated objects (`GDL_FPS=1`).
- Done since: keys turn by the game's rule (`dd18546`); hints in the game's box (`cd179ab`); levels E1–I5 audited and bursting objects burst (`e231026`; their blast still to add in `effects.rs`).

### 2. Level fidelity (the user's top priority)

- Static audit, level by level, of what the level build makes, skips and
  hides, and every trigger's link ([level-population.md](level-population.md),
  [mechanics.md](mechanics.md); `level_audit` example): E1–E2, F1–F2,
  G1–G5, H1–H4, I1–I5 (the helper, round 3), then A5, A6, J1–J6, K1–K5,
  L1–L3, S1–S9, T1–T3, DEMO1 (done: A1–A4, B1–B6, C1–C5, D1–D5).
- Pin down the stray objects the user saw on the early levels: likely
  the debug markers (`I`, now off in play); ask the user to confirm.
- Walk each level from its start to its exit: levers, lifts, doors, keys,
  the exit to the next level. Done so far: A1's exit leads to A6.
- Bursting animated objects (node type `0x50000`: H1's fire, the I realm's
  minecarts): burst with their effect and sound as their loop comes
  round, then hide.
- E2's debris (`0x33`), thrown as the boss's stage counter moves on.
- Subtype 1 rotators; monsters holding a mover still by standing on it.
- Random item types: the game picks one each build; the port takes the
  first choice ([level-population.md](level-population.md)).
- Exits with no destination code go to the tower (stand-in).
- Whether an animated object's collision scales with it (unconfirmed).

### 3. Monsters ([monsters.md](monsters.md) "Stand-ins and gaps")

- The AIs still on the chase stand-in: 0's nine-angle search, 3, 5/6
  beyond unaware walking, 14, `0x13`'s spotting gate, the fireball
  casters' own movement, and the rest of the AI table.
- Actor-vs-actor collision and pushing ([collision.md](collision.md)).
- The crowd penalty in target choice (co-op), the blocked-frame nudges
  round a charging runner, `WALKTOREADY`/`READYTOWALK`.
- The kiting thrower's wall-bump angles and leader logic
  ([projectiles.md](projectiles.md)).

### 4. Bosses and critters ([critters.md](critters.md) "Stand-ins and gaps")

- Grabs (`DAMG` kind 7): decoded, hero side on branch `wip-grabs`; test on
  I5 (`GDL_WARP="9.6,-4,-45"`).
- The effect slots' contact damage for kinds 0 and 4 (the lich's aura).
- Critter missiles' lifetime (3 s stand-in), trails and spin.
- The boss key's effect and pick-up (a 5 s stand-in returns to the
  tower); where the heroes go after a boss.
- Head stumps, the parts' look nodes, the health meter (`GMETER`),
  breakable `NODE`s, dropping held items, shadows.
- Critter blows on monsters, pushing players aside, critter-vs-critter
  and -monster collision, elemental resistances, multi-target weighting,
  the distance-from-home condition.
- The legendary weapon's throw and the heroes' highlights.
- A missile or blast waking a statue; the dropped key's toss.

### 5. Hero, combat and power-ups ([items.md](items.md), [powers.md](powers.md), [combat.md](combat.md))

- Rapid fire's rate; the hero missiles' streaks (`WEP_STREAK`) and the
  magic classes' element models.
- Turbo filling from experience; the sorceress's combo's 95; co-op combos.
- The hand glows and the `WEAPONS` and heroes' banks' running modifiers;
  the head-2 sparkle; the node spheres' flashes.
- The heroes' step cut under the boss camera; footsteps' pan and the
  shadow on water.

### 6. Look ([rendering.md](rendering.md), [effects.md](effects.md))

- Action texture scrolls (the wipes) and the texture-shift maths.
- The effects' own lights (a translucent sphere stands in where an effect
  has no model); the effects' depth bias everywhere; the no-blood switch.
- The particle library's exact emitter maths.

### 7. Front end, HUD and hints ([frontend.md](frontend.md) "Stand-ins")

- The shop and inventory screens; Options, Game Options, Compass and
  Controls changing what they list.
- The attract loop (movies, scroll screens, credits, demo play) and the
  title's 30 s timeout.
- Menu sounds, the spinning 3D arrow, border glows, inline icons, the menu
  fade.
- The memory card screens; one record per class.
- Hints drawn as the game draws them, for its time (plain centred text now).
- The secret realm's coin count and `ALLCOINS` (the secret character).

### 8. Audio ([audio-format.md](audio-format.md) "Not done / unconfirmed")

- Music track switching, ducking and priorities.
- The per-level `SNDS` names; the voice queues' pan and 12-voice limit.

### 9. Co-op (2–4 players)

- Joining from the select screen and the waiting panels, a hero per pad,
  the camera framing everyone, the 2–4 player placements, damage and
  boss scaling by player count, co-op combos, shared pickups and keys.

### 10. Speed and stability

- Warm-up hitches in a level's first frames (a pre-warm skips culling for
  3 frames; unverified).
- The frame rate with every animated object posed each tick.
- Keep the all-levels smoke test passing after each merge.

### 11. Formats

- `WDATA/*.WAD`, the per-realm resources (loaded by `FUN_8005a094`, parsed
  by `FUN_80058074`: cameras, enemies, maps, 14 named realm types): the
  chunk directory, level names, camera, audio and enemy records and the
  monster tuning are parsed; the rest isn't.
- `WORLDS.PS2` header words 4 and 6.
- The monster and actor type data the actor movers read (collision
  radius, step height); the players' is decoded.

## How the work is checked

- `tools/smoke.sh`: every level runs 120 s with the hero attacking; stops
  at a panic or a missing screenshot.
- `tools/tour.sh <levels>`: one run per level that stands the hero on each
  one-player trigger in turn (`GDL_HOPS`), attacking at each; prints which
  switched on and whether what they move arrived.
- `cargo run -p gdl-formats --example level_audit -- <game>/Gauntlet
  <level> --triggers --anim --falls`: what the level build changes.
- `cargo run -p gdl-formats --example floors -- <level folder> x z`: the
  floors down a line (check every warp point with it).
