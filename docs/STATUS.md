# Status: how close the rewrite is to the original

Assessed 2026-10-01 on `master` after the level audit's second round
(`04b2745`; the first estimate, on `ef1f596`, was 81 % / 76 %); local
co-op re-scored after it landed (3 % → 65 %). Online co-op is an addition,
not counted. Update
this file when an area moves; [HANDOFF.md](HANDOFF.md) has the
day-to-day state and the job queue.

## The numbers

| measure | estimate |
| --- | --- |
| **Faithful to the original, one player** (what a single hero plays through) | **≈ 82 %** |
| **Done overall** (everything the original does, 2–4 player co-op included, every level checked, no known issues) | **≈ 80 %** |

These are judgements, not measurements. Each area below gets a weight (how
much of the game it is) and a share done (what's confirmed against the
binary and ported, against what's known to be left: the code's 62 labelled
stand-ins and the gaps listed under "What's left"). One player's figure
leaves co-op out.

| area | weight | done | what holds it back |
| --- | --- | --- | --- |
| Disc, files and formats | 6 | 97 % | a few WORLDS/WDATA words, the `SNDS` chunk |
| Level look (geometry, lightmaps, blending, texture animation, particles) | 10 | 88 % | action texture wipes, the hand and weapon glows, effect lights, exact particle maths, shadows |
| Hero movement, camera, cuts | 7 | 97 % | the step cut under the boss camera |
| Hero combat, magic, power-ups | 10 | 88 % | rapid fire's rate, missile streaks, magic element models, the legendary weapon's throw |
| Monsters and generators | 9 | 75 % | most AIs run the chase stand-in; no actor-vs-actor collision |
| Bosses and critters | 9 | 72 % | grabs, effect-slot contact damage, missile lifetimes, the boss key, stumps, the health meter |
| Items, pickups, doors, exits, hazards | 6 | 92 % | random item types, the shop, the keys' turning traced, the secret realm's kept movers and camera |
| Level mechanics (triggers, lifts, animated objects, secret walls, falls) | 8 | 84 % | bursting objects, E2's debris, subtype 1 rotators; the tour's re-run pending |
| What each level places and hides | 5 | 90 % | every level audited statically (the last round: A5, A6, J, K, L, S, T, DEMO1); random item types take the first choice; the stray objects likely the debug markers (now off) |
| Every level checked start to exit | 5 | 25 % | only A1's exit to A6 checked; triggers toured on A1–C1 and D1–D4 |
| Quest, tower, saving | 5 | 90 % | the unlocked secret characters on the select screen, per-class records, memory card screens |
| Front end, menus, HUD, hints | 7 | 75 % | shop, inventory, options, attract loop, the hints' plates |
| Audio | 5 | 85 % | music switching and ducking, menu sounds, footstep pan |
| Co-op (2–4 players) | 6 | 65 % | the after-level progress and shop screen (and joining there), co-op combos, the monsters' crowd penalty, the exits' wait-for-the-others hint |
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
  "Auditing the levels"; [mechanics.md](mechanics.md); `level_audit`
  example): done for all 67 (the helper's fourth round: A5, A6, J1–J6,
  K1–K5, L1–L3, S1–S9, T1–T4, DEMO1). That round's fixes: the tower's
  portals go where the realm WADs' level records say (`a2` → `levelA6`,
  `a6` → the boss `levelA5`, `j4`–`j6` → J6, J4, J5, `k1`–`k4` → K2, K3,
  K4, K1) and a finished level is marked by that id; triggers sharing an
  id keep it only on the first, and a camera point reaches only ids up to
  127 (the tower's pedestal pad cut to the wizard's camera 240 each time
  the hero stepped on it; C3's 426 cut too). J4's 335/366/422/429 and
  L1's 81/83 are as the game has them. Saved heroes keep their bits:
  each still names the portal finished, now the level the game puts
  behind it.
- The secret levels (S1–S9) end by their timer, now in the runtime
  (`exits/secret_realm.rs`, [items.md](items.md) "The secret realm"): the
  hourglass, the clock and the count, the coins and the unlock, and the
  way back to the level whose secret exit was taken, as it was left.
  Left: the movers' and levers' states and the camera kept with it (the
  game keeps them; here they start afresh), no opening shot and no save
  on the way back (hooks in `play_camera.rs`/`frontend.rs`:
  `SecretReturn::coming_back_to`), Quit Level disabled there, the
  opening's banners (`GRAB_GOLD`, `S_GRAB`; every level's name), S5's
  lights (record flag 8).
- The tower (also the fourth round): the gates it opens as it loads now
  have their animated walls at their last frame (they played open over
  their first seconds on every return), the lower lift and elevator start
  on once H1 is finished (the item set-up's rule, [items.md](items.md),
  "Quest items and the tower's gates"); the rule's record check
  (`+0xF0` against `r13-0x7d94`: every level counted) isn't ported.
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
- The select screen opening the secret characters a hero has unlocked
  (`secret_realm::class_open`; the coins, count and `ALLCOINS` are in).

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
