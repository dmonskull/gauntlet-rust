# Critters: bosses, golems, gargoyles, generals

"Critter" is the game's name for its scripted-monster system. Each critter is
driven by a `CRITTER/<name>.WAD` data file rather than the compiled-in monster
tables (see [monsters.md](monsters.md) for regular monsters).

The file format is parsed in
[`crates/gdl-formats/src/critter.rs`](../crates/gdl-formats/src/critter.rs),
which has typed records and real-data tests over all 18 files. The
runtime is [`crates/gdl-game/src/critters.rs`](../crates/gdl-game/src/critters.rs).
So far it runs the placed golem only; see [In this rewrite](#in-this-rewrite).

Addresses are in `main.dol`; `r2`/`r13` are as in [INDEX.md](INDEX.md).
Critters keep time in seconds: `r13-0x756c` is the current time and
`r13-0x7570` is seconds per frame (1/30 at the game's rate). `r13-0x7584`
is video fields per frame, used by a few field counters.

## Loading

The per-level loader `0x8005638c` walks the level's eight enemy slots
(`0x802571ac`, built by `0x80057738`; [monsters.md](monsters.md)). For each
critter enemy type it loads one file with `0x8003f2c4`, which calls the
loader `0x80040008` (in `critter.rs`, `file_for_enemy`):

| enemy | file |
| --- | --- |
| 0x1D golem | `golemI.wad` in realm 9 (ice), `golemF.wad` in realm 6 (hell), else `golem.wad` |
| 0x20 gargoyle | `gar_%s.wad` (`0x80111fec`), `%s` taken from the realm `ENMY` record's name buffer at +0x10: `eagl`, `lion` or `serp` (`RealmEnemy::variant`) |
| 0x21 general | `general.wad` |
| 0x22 … 0x2C | `dragon`, `chimera`, `djinn`, `drider`, `pboss`, `yeti`, `wraith`, `lich`, `skorne1`, `skorne2`, `garm` |

Each level's boss is its `LEVL +0x44` enemy type, listed with subtype 9:

| level | boss |
| --- | --- |
| A5 | chimera |
| B6 | dragon |
| C5 | djinn |
| D5 | drider |
| E2 | skorne1 |
| F2 | skorne2 |
| G5 | lich |
| H4 | garm |
| I5 | yeti |
| J5 | wraith |
| K5 | pboss |

Most regular levels load the golem, general and gargoyle as subtype 5.

The loader `0x80040008` looks up the eight chunk tags from the bytes at
`r2-0x70cc`: `SFXX DAMG MOVE PTRN NODE DESC TYPE ADDA`. It byte-swaps every
record field by field, which is what fixes each record's size and field
widths. The header it fills keeps, as (count, pointer) word pairs:

| words | table |
| --- | --- |
| 4, 5 | `TYPE` (`"Critter Header has no types"` when empty) |
| 6, 7 | `DESC` |
| 8, 9 | `ADDA` |
| 10, 11 | `MOVE` |
| 12, 13 | `PTRN` |
| 14, 15 | `NODE` |
| 16, 17 | `DAMG` |
| 18, 19 | `SFXX` |

After swapping, the loader clears each `DESC +0x22` to −1 and each
`TYPE +0x134` to 0. It then links every `ADDA` record into the list at
its type's `TYPE +0x134` (`"CRITTER: AddAnim has addto idx"`).

`0x8003f69c` then runs `0x8003f754` for each type:

- `TYPE +0x130` is set to the file and `TYPE +0x120` to its `DESC` record.
- The type registers in `0x8024b9d4[class × 6 + subtype]` (`TYPE +0x52`
  subtype). `0x8003f734` reads that registry back.

Model paths (`0x8003f32c`, `0x8003f4ec`, `0x8003f754`) depend on the class
(`model_folder` in `critter.rs`):

- golem (3) and general (8): `monsters/%s/%s` (`0x80111a20`), the `DESC`
  name plus the realm's items folder (`r13-0x72c0 + 4`, e.g. `levelA`),
  giving `MONSTERS/GOLEM/levelA`;
- gargoyle (7): `monsters/%s_%s` (`0x80111a30`), giving `MONSTERS/GAR_EAGL`;
- bosses: `monsters/%s` (`0x80111a40`).

`0x8003f90c` resolves the atree. Its name is `"%s%s"` (`r2-0x70d4`): the
`DESC` prefix (+0x10) followed by the `TYPE` name (+0x00), giving
`GOLEM1`, `DRAGON` or `CHIMEAGLE` (`"Critter can not find atree %s"`). A
type with `TYPE +0x11E` ≥ 0 shares that parent's atree instead
(`"Child critter defined before parent"`). The same function resolves:

- the `ADDA` animations, and
- the node names at `TYPE +0x20/+0x30/+0x40` into `+0x56/+0x58/+0x5A`.

`0x8003fb64` (via `0x8003fae8`) resolves each move:

- its animation name (`MOVE +0x20`) into `+0x0C`. A missing one logs
  `"Critter %s unable to find anim %s"` and plays action 0.
- its node name (`+0x30`) into `+0x0E`. A missing node is −1, which means
  the root.

It also resolves `NODE +0x14`, and loads the `SFXX` effects and sounds the
moves and `DAMG` records use (`0x8003feb0`, which follows `SFXX +0x04`
chains). A `DAMG` kind 6 anywhere sets `r13-0x74b0`. Finally it sets
`TYPE +0x124/+0x128/+0x12C` to the type's `MOVE`, `PTRN` and `NODE` runs.

The retail data has a few references that don't resolve. The test
`every_real_critter_animation_resolves` lists them:

- the eagle and lion gargoyles name `SERPTORSO`;
- some realm golem models lack `TAUNT`, `BLOCK`, `ATTACK1R`, `FOOTR#0`,
  `ULEGR#n`;
- PBOSS names `BODY1_TENT_COLLI`.

## Classes (`DESC +0x20`)

| class | critters | update |
| --- | --- | --- |
| 3 | golem | `0x800395bc` |
| 4 | the bosses | `0x800399f0` |
| 7 | gargoyles | `0x800395bc` |
| 8 | the general | `0x800395bc` |

Death (enemy 0x1E) is a regular monster, not a critter.

## Instances

Instances are 16 records of `0xAE0` bytes at `0x80240bd4`. The allocator
is `0x8003e200` (`"Too many Critter Insts"`); `r13-0x7494` counts the slots
in use. This is the same "object" table the hero's target search walks
([combat.md](combat.md): state `+0x08` ≥ 2, hit points `+0x4B0` > 0,
centre `+0x5C`, radius from type `+0x7C`), so a critter is a
`TargetKind::Object`.

| offset | field |
| --- | --- |
| `+0x000` i16 | slot index |
| `+0x002` i16 | serial number |
| `+0x004` | its `TYPE` |
| `+0x008` | state: 0 new, 1 dying, 2 set by the boss intro, 3 active (≥ 2 can be hit) |
| `+0x00C` | matrix |
| `+0x02C` | its forward row |
| `+0x03C` | root position |
| `+0x04C` | aim point: root plus `TYPE +0xB4` up |
| `+0x05C` | centre: root plus the matrix times `TYPE +0xC0` |
| `+0x06C` | model |
| `+0x074` | animation controller |
| `+0x086` | animation index |
| `+0x088` i16 | frame count |
| `+0x090` f32 | frame |
| `+0x0B4` | model nodes (0x28 bytes each) |
| `+0x0BC` | `ADDA` instances |
| `+0x0C0` | root model node |
| `+0x0C4` | shadow |
| `+0x0C8`, `+0x0CC`, `+0x0DC` | look and effect nodes |
| `+0x0D0` | the current move's node |
| `+0x0F8` / `+0x0FC` | yaw it was made facing / current yaw |
| `+0x100`..`+0x10C` | look-node angles |
| `+0x110` / `+0x114` | anger / 1 ÷ anger |
| `+0x118` i16 | current move (index within the type's moves) |
| `+0x11A` i16 | next move |
| `+0x11C` / `+0x11E` / `+0x120` i16 | running pattern / chosen pattern / pattern step |
| `+0x122` | switched this frame |
| `+0x124` i16 | the current move's target player |
| `+0x126` i16 | target picked by the attack choice |
| `+0x128` i16 | player held by a grab |
| `+0x12A` i16 | number of targets |
| `+0x12C` | up to 4 target entries, 0x24 bytes each: player, cos, horizontal distance, score, weight, direction x/y/z |
| `+0x1BC`..`+0x1C8` | per player (0x10 each): damage dealt to it and when, damage taken from it and when |
| `+0x1FC` | a target point |
| `+0x20C` / `+0x20E` | blows done / sounds done this move (bits 1, 2) |
| `+0x214` | the move's hold time (bosses) |
| `+0x218` | per move: when it last ended |
| `+0x318` | per pattern: when it last started |
| `+0x398` / `+0x3C8` / `+0x428` | the move node's matrix / position / previous position |
| `+0x3D8`, `+0x418` | spawn matrix and position |
| `+0x438` | floor point |
| `+0x44C` | health meter |
| `+0x44E` / `+0x44F` | parts made / parts alive |
| `+0x450` | health-meter model (`GMETER`) |
| `+0x49C` | home |
| `+0x4AC` | distance from home |
| `+0x4B0` | hit points |
| `+0x4B4` | damage taken lately (roar) |
| `+0x4B8` | blow kinds taken |
| `+0x4BC` | push taken |
| `+0x4CC` | knockback velocity |
| `+0x4DC` | time of the last blow |
| `+0x4F8` | per `NODE` (0x5C each): record, model node, `+0x534` position, `+0x548` flash, `+0x54C` hit points, `+0x550` damage taken |
| `+0xAB8` | the `NODE` last hit |
| `+0xABA` | kill effect |
| `+0xABC` | flash |
| `+0xABE` | fade |
| `+0xAC4` / `+0xAC6` | frozen / stunned (fields) |
| `+0xAC8` | effect timer |
| `+0xACC` | item to drop |
| `+0xAD0` | placed range |
| `+0xAD4` | path point |
| `+0xAD8` / `+0xADC` | next part / parent |

## Spawning (`0x8003df60(class, subtype, matrix)`)

`0x8003df60` looks the type up in the registry (`"No Critter type %d
subtype %d loaded"`) and allocates an instance. Then:

1. `0x8003e838` sets up the stats:
   - hit points = `TYPE +0xE4` × the level's `+0xAC`;
   - the `MOVE`/`PTRN`/`NODE` timers and the per-player records are
     cleared.
2. `0x8003e300` places it:
   - it builds the model and yaw from the matrix;
   - it drops the root onto the floor below: probe `0x8000d4b8` from 4
     above to 1000 below, then add `TYPE +0xB0`;
   - home = `TYPE +0xA0` if `+0xA4` < 999, else the spawn point.
3. `0x8003f108` builds the `ADDA` models. `0x8003ee10` sets up the `NODE`
   spheres: hit points = `NODE +0x44` × the body's.
4. `0x8003e6e8` builds the health meter.
5. For each `TYPE +0x11C` child it builds another instance that shares the
   root matrix, attached to the parent's model node. The chimera's heads
   are made this way.

Callers:

- **Boss locator** (kind 6, `0x80066258`): `0x8001bb54` stores the spot
  (`0x8023e034`) and calls `0x8003df60(4, 0, matrix)`. The boss instance
  goes to `r13-0x7760`.
- **Placed critters** (`ENEMYINFO`, the placed-monster case of the item
  update, `0x80060100`):
  - 0x1D calls `0x8003df60(3,0,…)`, 0x20 calls `(7,0,…)`, 0x21 calls
    `(8,0,…)`.
  - A placed golem or gargoyle starts as a statue model, `GOL_STATUE` or
    `GAR_STATUE` (`0x800646e4`, which also drops item flag 1).
  - The statue only comes alive once item `+0xE4` bit 1 is set. A trigger
    with flag `0x2000` sets that bit on an item it finds near itself
    (`0x800606e8`). The statue's animation (`+0xC4`/`+0xCA`) then plays,
    and when its field timer `+0xC6` runs out the critter is made.
  - The placement's range (`+0xE8`) × level `+0xB4` becomes `+0xAD0`, and
    its drop item (`+0xEE`) becomes `+0xACC`.
  - Level-data survey: 38 levels name the golem.
  - Not ported: the trigger wake-up needs the trigger system, so a
    runtime will need a stand-in (e.g. waking when the player comes near,
    named as such).

## The update

`0x8004cfe0` (the monster frame update) calls `0x80038c30`. That counts
the frame (`r13-0x749c`) and the live players (`r13-0x74a4`), then runs
`0x80038cf4` for every live instance that isn't a child part.

`0x80038cf4`:

1. Counts the parts still alive (`+0x44F`). Refreshes the matrix, aim
   point, centre and `NODE` sphere positions.
2. `0x80039384` turns blows into knockback (not bosses); see "Taking
   damage".
3. `0x800394e0` clears the damage-taken record (`+0x4B4`, `+0x4B8`,
   `+0x4DC`) when either:
   - the last blow is more than 3 s old (`r2-0x7290`), or
   - the current move is ROAR or a hit reaction (kinds 0x40–0x7E).
4. Moves the health meter.
5. A state-3 body whose parts are all dead dies itself. Death sets state 1
   (see "Death and victory").
6. Dispatches on class:
   - 4 → `0x800399f0`
   - 3, 7, 8 → `0x800395bc`
   - anything else → `0x8003c324` plus a floor snap
7. The class update returning 0 means the critter was removed. Otherwise
   it updates the `ADDA` animations and refreshes the matrices again.

### Anger (`+0x110`)

anger = (1 − hp / (1 + full hp)) × 4.5 (`r2-0x723c`) + 0.5 (`r2-0x7240`)

Anger runs from 0.5 at full health to 5 near death. It gates moves
(conditions `[4]`/`[5]`) and scales projectile speed.

### Targets

`0x80036b88` (golem, gargoyle, general) runs on every live player that
isn't invisible (player `+0x124 & 4`):

- It scores the player with `0x800371b8(TYPE +0x80 condition, player
  position +0x64)`.
- A player hit by a critter within the last 0.25 s (`+0x8E8` > now)
  scores × 1000 (`r2-0x7200`).
- When following a path (`+0xAD4`), a player beyond `+0xAD0` is ignored.
- It keeps the one best target in `+0x12C`.

Bosses use `0x80036ed4` instead:

- up to four targets, sorted by score (`0x800370d4`);
- only scores under 1e21 count;
- scores are weighted by how much damage each player dealt versus took
  (`+0x1BC`/`+0x1C4`).

Every target chosen adds 1.0 to a per-player count (`0x802409a0`).
`0x80036d18` uses those counts to spread a multi-part boss's parts over
different players.

Scoring, `0x800371b8(critter, condition, point)`: the vector from the
centre (`+0x5C`) to the point, with its height zeroed, is normalised to
give the horizontal distance and direction. Then:

- anger < `[4]`, or `[4]` < `[5]` ≤ anger → 1.2e21
- distance < `[0]` → 1.01e21
- 0 < `[1]` < distance → 1.02e21
- 0 < `[7]` < |dy| → 1.03e21
- turn the forward row by −`[2]`; cos(that, direction) < `[3]` → 1.1e21
- otherwise the score is distance ÷ |cos|, or distance × 2 (`r2-0x71dc`)
  when cos ≤ 0.5.

`0x80036a74` re-scores a stored target the same way, times its weight
(`+0x10` of the entry). `0x80036970(critter, condition, fallback)` picks
the best stored target for a condition:

- A positive condition `[6]` is a limit on the critter's distance from
  home (`+0x4AC`).
- Without fallback, scores ≥ 1e21 fail.
- With fallback, it falls through to the parent part's choice.

### Choosing a move

Each frame `+0x11A` (next) is reset. It is then filled by the first of
these that finds something:

| step | function | picks |
| --- | --- | --- |
| forced | `0x8003bd0c` | no current move, or state 0/2 → INIT (kind 0). After INIT → START (0x10). Dying → DEATH (0x11). The current move's `next` (`+0x54`). Bosses leave START for READY, and the intro state (`r13-0x725c`) forces READY/ROAR. Then: blow kinds `& 0x120` → KNOCKDOWN (0x42) if `0x100`, else KNOCKBACK (0x41); damage taken lately ≥ 50 (`r2-0x7128`) × `0x8011a90c[players]` (1.0 for one player) → ROAR (0x22); kinds `& 0x10` → FLINCH (0x40). Clears kinds `0x130`. |
| block | `0x8003ba14` | a BLOCK (0x23) move whose condition finds a player who is attacking: `0x80074564(player, 1)`, player `+0x8F0` > 10 or 2/5/10 |
| attack | `0x8003b6f0` | the next step of a running pattern; else the least recently used pattern or attack (kinds 0x7F–0xEF) that is off cooldown and whose condition finds a target (below) |
| movement | `0x8003b594` | movement moves (0x30–0x39; only when `TYPE +0x5C` has `0x10000`, which the loader sets when the type has any). The one whose condition scores the target point lowest (< 1e21) and that is off cooldown. With no targets and a path point, the first one with a speed. 0x38 needs a target. |
| idle | `0x8003c8a0` | TAUNT (0x21) when anger < 0.8 (`r2-0x7168`) and it's ready, else READY (0x20) |

`0x8003c8a0(critter, kind, mode)` finds a move of a kind, skipping moves
with flag 4. A move is off cooldown when its last end (`+0x218[i]`) plus
its cooldown (`MOVE +0x80`) has passed. The mode decides what it accepts:

- mode 0: only moves that are ready;
- mode 1: the one closest to ready. If there is none, it logs `"Critter
  can not find move type %d"` and falls back to READY.
- mode 2: the first found.

Attack choice (`0x8003b6f0`) in detail:

- **Continuing a pattern:** while a pattern runs (`+0x11C`) and its next
  step (`PTRN +0x20 + 2 × (step + 1)`) is ≥ 0, that step is taken.
- **Patterns:** a pattern is eligible when all of these hold:
  - it isn't the one that just ran;
  - it isn't flagged `0x1000`;
  - if flagged 2, all parts are alive;
  - its cooldown has passed (`+0x318[i] + PTRN +0x14 ≤ now`);
  - its condition (`PTRN +0x30`) finds a target.

  Among eligible patterns, the smallest `+0x318` wins; the start value is
  999999 (`r2-0x7130`).
- **Attacks:** an attack is eligible when all of these hold:
  - it isn't the current move;
  - its flags don't include 4;
  - if flagged 2, all parts are alive;
  - if flagged 0x10, its node, and its `next` move's node, still exist;
  - it is off cooldown;
  - its condition finds a target.

  An attack beats the chosen pattern or attack when its `+0x218` is older.
  On a tie it still wins when no pattern is chosen and the transition from
  the chosen attack to it (below) is 2. A grab (0x81) is forced while a
  player is held.
- The pattern's first move goes to `+0x11A`, with `+0x11E` = pattern and
  `+0x126` = target.

Bosses with parts (`0x800399f0`) also give each part a move:

- A part follows the boss's pattern step.
- Otherwise it gets `0x8003bb40` (forced), then attacks, then TAUNT/READY.
- When a part is attacking, the boss plays kind 1 (TOGETHER).
- `0x8003b3e4` steps through all moves in order, skipping kind 0xF0. This
  runs under the debug flag `r13-0x7534 & 0x80`.

### Switching (`0x8003c324`)

Next = the pattern's step if one is running, else `+0x11A`. The mode for
the animation switch:

- No next move: keep the current animation, mode 0.
- Next ≠ current, next priority (`MOVE +0x08`) ≥ `0xF00`, and the current
  move's transition (`+0x56`) ≠ 0: mode 3, switch at once.
- Else, while the hold time (`+0x214`) hasn't passed: keep the current
  move, mode 0.
- Else the mode is `0x8003c7ec(current, next)`, 3 with no current move.
  It is keyed on the **current** move's `+0x56`:

  | `+0x56` | mode |
  | --- | --- |
  | 0 | 0 |
  | 0x14 (20, most moves) | 1 if next priority & ~0xFF ≤ current's, else 2 |
  | 0x3C | 1 if next priority < current's, else 2 |
  | 0x50 (INIT) | 1 if next priority < 1, else 2 |
  | 0x5A | 2 |
  | other | 2 if current priority < next's, else 1 |

- Different move, mode 0, hold passed and the animation ended → mode 1.

The animation controller `0x80011104` → `0x80011134` → `0x8000eb70` applies
the mode. The "ended" flag is `+0x36 & 0xFF == 0xFF`, tested by
`0x8000eb54`.

| mode | switches when |
| --- | --- |
| 0 | ended and different |
| 1 | ended |
| 2 | ended or different |
| 3 | always |

It returns bit 1 when it switched. `+0x122` = switched.

On a switch, `0x8003c614`:

- **Pattern bookkeeping:**
  - Outside a pattern, it records the move's end: `+0x218[i]` = now +
    (frames − 2) × 1/30 (`r2-0x7120`). Cooldowns run from there.
  - Starting a pattern sets `+0x318[p]` = now, `+0x11C` = p, step 0.
  - Inside one, it steps it; a step < 0 or past 8 ends it.
- **Boss intro:** START ending moves the intro state 1 → 2; ROAR moves it
  3 → 4.
- Sets `+0x118` = next and `+0x214` = 0.

With no switch and no next, the current move becomes −1 once its
animation ends.

Bosses (`0x800399f0`) hold a move for `MOVE +0x8C` seconds (`+0x214`).
DEATH with a hold fades the body out over the last 0.5 s and is removed
when the hold ends.

### Doing the move

The class updates run these for the current move (bosses also for each
part), in order:

1. `0x8003b0e4`: the move's target (`+0x124`). On a switch (or with none)
   it is the attack's pick (`+0x126`), else `0x80036970(move condition,
   fallback)`, or none while stunned. A switch also:
   - clears `+0x20C`/`+0x20E`;
   - sets the move node (`+0xD0` = model node `MOVE +0x0E`, else the
     root);
   - saves the node's previous position (`+0x428`) and computes its
     matrix (`+0x398`) and position (`+0x3C8`).
2. `0x8003b218(frame)` fires blows and sounds. `0x8003c034` gives the blow
   bits for this frame by move kind:
   - 0x80, 0x83, 0x86 (sweeps): blow 1 every frame in
     [`+0x40`, `+0x50`], blow 2 in [`+0x44`, `+0x52`].
   - 0x81 (grab): blow 1 in its window, blow 2 once from `+0x44`.
   - 0x85: in the window, every `+0x4C` frames.
   - 0x88: once from `+0x40`, at the target's position (`+0x1FC`);
     blow 2 after it.
   - Others: each once, from `+0x40` / `+0x44`.

   Each set bit calls `0x8003c9b0(DAMG +0x48 or +0x4A, bit, first)`, where
   first means the bit wasn't set before. A `DAMG` with flag `0x4000` stops
   while the effect timer `+0xAC8` runs. The `SFXX` at `+0x58` starts once
   at frame ≥ `+0x5A`, and `+0x5C` at frame ≥ `+0x5E` (`0x8003d6f8`).
3. `0x8003a8dc` moves the critter.
   - **Speed** = `MOVE +0x84` × level `+0xB0` × dt, along the forward row
     (flag `TYPE 0x40`: along `+0x3F8`). The direction depends on the kind:

     | kind | direction |
     | --- | --- |
     | 0x35 | backwards |
     | 0x32 | left |
     | 0x33 | right |
     | 0x36 | diagonal |
     | 0x38 | toward `+0x1FC` |

   - **Knockback** (`+0x4CC`) × dt is added. Knockback then keeps 0.8 of
     itself per frame (`r2-0x7168`), components under 0.01 stop, and
     upward speed falls at 100/s (`r2-0x7134`).
   - **Bosses** stay within `TYPE +0xAC` of home (a box with `TYPE` flag
     `0x20`); `+0x4AC` is the distance from home.
   - **Other classes** collide:
     - walls from the centre with radius `TYPE +0x7C` (`0x80035320`), or
       with the solid `NODE` spheres (`TYPE` flag `0x100`);
     - floor probe with radius 1, from `TYPE +0x78` above to −(`+0x78` +
       3) below, at the leading edge. The rise must be ≤ 2 × (radius +
       move), or the move is cancelled; it falls at most 16/s;
     - `0x80034e78`, `0x800350c8` (pushes players out: 2 × overlap), and
       `0x80034c14`;
     - other critters' cylinders (`0x80037414`).
4. `0x8003ae64` turns, at `MOVE +0x88` × dt (halved while stunned):
   - toward the target's position;
   - without `TYPE` flag `0x400`, within `TYPE +0xCC` of the yaw it was
     made facing;
   - toward home with `MOVE` flag `0x20`;
   - toward the path point with no target.
5. `0x80035c20` turns the look nodes (`+0xC8`/`+0xCC`) toward the
   target's `+0x54`. The limits are `TYPE +0x60..+0x74`, at 1.946 rad/s
   (`r2-0x7248`). This is skipped during START, INIT, DEATH and moves
   flagged 1.

Class 3/7/8 (`0x800395bc`): when DEATH's animation ends, it drops its item
(`0x8003a750`: `+0xACC`, or a random gargoyle drop) and is removed
(`0x8003e964`).

### What a `DAMG` does (`0x8003c9b0`)

`DAMG` damage is × the level's `+0xBC`. The kinds, with their
setup-function numbers:

- **0 — sphere** (`0x8003633c`). The sphere sits at the move node's
  position plus `DAMG +0x20` rotated by the node's matrix, radius `+0x0C`.
  It is swept from last frame to this one against each live, hittable
  player's cylinder (`0x80078da0`; player `+0x850`/`+0x854` + radius,
  `0x8002fa24`).
  - A player can be hit once the critter-hit timer `+0x8E8` has passed.
  - It calls `0x80078560(damage, player, 1, DAMG +0x04 | 0x1000000 if
    DAMG +0x42 has an effect, push)`. Push = 0.5 × (node movement + unit
    direction to the player with y = 1).
  - The player's timer is set to now + 0.25 s. The damage is added to the
    per-player record.
  - `0x80035908` hits regular monsters (state 1/6, or 8 during
    `r13-0x731c`; not type 0x1F) the same way (`0x8004e660`).
  - Non-bosses deal half damage while `r13-0x7320` < 1.
  - On first frames it also spawns the projectile/effect as for kind 1.
- **1 — projectile** (`0x8003cfbc(…, 0, node position)`).
  - **Start:** `DAMG +0x20` in the critter's space plus the node position.
  - **Effect:** the `SFXX +0x40` effect is the projectile (`0x8003d6f8` →
    `0x8003db7c` → `0x8009418c`).
  - **Flags:** `0x801` (bosses) or `0x809`, then:
    - kind 1: | 6; kind 2: | 0x30; kinds 3, 5, 6, 8: | 0x20;
    - `DAMG` flag 0x40: & ~6; 0x1000: & ~1; 0x2000: | 0x400;
    - blow bit 0x20000 carries over.
  - **Stats:** damage = `+0x2C` × level `+0xBC`. It also gets the `+0x18`
    parameter, radius `+0x0C`, kind `+0x04`, hit effect `+0x42`, and trails
    `+0x44`/`+0x46` lasting `+0x3C`.
  - **Speed:** min `+0x30` ≤ 0 means a still effect lasting `+0x08`.
    Otherwise speed = min + (clamp(anger, 0.5, 1.5) − 0.5) × 0.75 × (max
    `+0x34` − min).
  - **Direction:**
    - flag 4: the full forward row;
    - flag 1 with a target: at the target (`+0x54`) from the projectile;
    - otherwise forward with y = −0.5.
  - **Aim:** without flag 8 the direction is solved for an arc under
    gravity `+0x38` (`0x80030a9c`). Kind 1 is then turned by `+0x14` ±
    spread `+0x48`/2 at random, and tilted by `+0x1C` with flag 8.
  - **Launch:** `0x80093688(gravity, life +0x08, …, velocity)`.
- **2, 3 — attached or ground effect** (`0x8003cfbc(…, 1, 0)`): the same
  effect with damage, attached to the move node. Kind 3 is the golem and
  garm stomp ring, radius `+0x0C` (55–75).
- **4 — breath cone** (`0x80036050`). A segment from the node point along
  the node's direction, turned by `+0x14`/`+0x1C`, of length `+0x0C`.
  Players between `+0x10` and `+0x0C` horizontally, within `+0x08` + their
  cylinder, and with a clear line (`0x8005fb34`) take the damage. The hit
  timer is as for kind 0.
- **5, 6 — generator effects**: on the level's special generators
  (`0x802409f0`, found by `0x80063efc`).
- **7 — grab**: bit 1 grabs the player (`0x800746c8`) into `+0x128`; bit 2
  throws them with damage (`0x800366e4`, kind `0x8050`).
- **8 — projectile at a point**: aimed at `+0x1FC`.
- **9 — body-specific**: `0x8001a914`.

## Taking damage (`0x800382c0`)

`0x800382c0(damage, critter, player, kind, point, push, effects)` is
reached from the hero's blows (`0x8008625c`), from explosions
(`0x800357c8`, on floor types `0xF0000`), and others.

1. It needs a type and state ≥ 2.
2. **BLOCK** (current move 0x23): the kind loses `0x130` and damage is ×
   0.25 (`r2-0x7228`). A block effect plays once when the move's second
   sound frame is ≥ 1000.
3. **Armour and resistances:** `0x8002f58c(TYPE +0xBC armour, damage,
   kind, TYPE +0xE0 resistances)` — armour is taken off, and a blow no
   stronger than it does nothing ([combat.md](combat.md)).
4. **Scaling:** a fixed value in one mode (`0x80256f60` == 3). Non-bosses
   take × 2 while `r13-0x7320` < 1.
5. `+0x4B4` += damage.
6. **Bosses** outside the intro take × `0x8011a920[players]`: 1, 1, 0.5,
   0.3, 0.2 for 0–4 players.
7. **Experience** (for a player's blow): min(damage, hp) ÷ (1 + full hp) ×
   `TYPE +0xE8`, × players for bosses, handed to that player
   (`0x80076144`).
   - The per-player damage record is updated.
   - Kind `0x800000` pushes the player (`0x8007826c`).
   - Non-bosses scale damage down for heroes above the level's experience
     level: `+0x9C`, by 0.02 per level, at least 0.1.
8. **Node spheres:** without kinds `0x100320`, a blow on a `NODE` sphere
   (`+0xAB8`) is × `NODE +0x40`, capped at the node's remaining hit points.
   A breakable node (flag 2) that runs out:
   - fires its `DAMG +0x12`;
   - with flag 4, hides its model subtree (`0x8003ecdc`) and drops moves
     that used it.
9. **Damage > 0:**
   - kinds are ORed into `+0x4B8`, push is added to `+0x4BC`, and
     `+0x4DC` = now;
   - hit points go down, and a child's also come off its parent;
   - blows on a parent are shared out over its living parts (× 0.5 ÷
     their count);
   - the hit effect plays (`TYPE +0xF4`, or `+0xF6` for effect kind 2; or
     `0x80093e08` by element);
   - the body or node flashes (2 frames).
10. **At 0 hit points:**
    - state 1, and every player gets 0.2 (`r2-0x71a8`) × `TYPE +0xE8`
      (`0x80036658`);
    - a parent's parts all die with it;
    - a boss sets `r13-0x7780` (`0x8001ba1c`);
    - the killer's kill count goes up.

**Knockback** (`0x80039384`, not bosses): push × a factor is added to
`+0x4CC`, capped at 40 (`r2-0x7178`), and the push is cleared. The factor
is:

| blow kinds | factor |
| --- | --- |
| `0x10140` | 10 (`r2-0x7194`) |
| `0x20` | 7.5 |
| `0x10` | 5 (`r2-0x7270`) |
| dead | 20 |
| golem | 5 less (`r2-0x7188`) |

## Death and victory

- **Removal:** `0x8003e964` removes an instance and its parts, effects,
  shadow and models. For a boss it also calls `0x8002c450` and
  `0x8001b854`:
  - each player plays victory (`0x800a1d30`);
  - the boss key effect (`BOSSKEY2`) appears at the boss's position +
    `TYPE +0xD0` (`0x8009eb78`);
  - `r13-0x7784` = 1.
- **When removal happens:** bosses are removed when DEATH's hold ends;
  other classes when DEATH's animation ends.
- **Boss wake-up** (`0x800399f0`, state 0):
  - Without `TYPE` flag `0x80`, a 2 s timer starts (`r13-0x74b4`).
  - When it runs out, the boss wakes (state 3) once the nearest target is
    within `TYPE +0xEC` (always, if that's ≤ 0).
  - Waking calls `0x8001b7f8`, which starts the boss camera
    (`r13-0x7788`, `0x8002c4f0`) and sets `r13-0x720c`/`-0x7210`.
  - The same happens when a boss stands on floor flagged `0x10`.

## Boss intro and camera (pointers, not traced)

The intro state `r13-0x725c`:

- `0x80057020` sets it to 1 at level start for the first player who
  hasn't seen this boss (`0x800a1cb0`).
- The critter code moves it 1 → 2 when START ends.
- The level update (around `0x80056c00`) moves 2 → 3 after a delay:
  `r2-0x6ae0`, or `r2-0x6b00` for bosses 0x23, 0x29 and 0x2A.
- ROAR moves it 3 → 4.
- The level update moves 4 → 5, stamping the time.
- After another delay it goes to 6; PBOSS swaps its eyeball model
  (`PBOSSEYEBALL`).
- A dead boss sets 99.

The forced-move code reads it (READY during 1–2, ROAR at 3, READY at 3–5
for boss 0x23).

The boss camera reads `WDATA` `BCAM`:

- `"BossCamStartCalc called with no boss"` is at decompile line ≈ 11135.
- The `"BCAM Y %.0f P %.0f D %.2f…"` and `"BCAM DY %.2f DD %.2f DP
  %.2f"` debug prints follow at ≈ 11240–11900, from about `0x8001bbcc`
  on.
- [camera.md](camera.md) has the play camera it replaces.

## Record layouts

Little-endian on the disc (the game byte-swaps). Field names match
`critter.rs`.

**`DESC`** (`0x30`):

| offset | field |
| --- | --- |
| `+0x00` | name[16] (the `MONSTERS/` folder) |
| `+0x10` | atree prefix[16] |
| `+0x20` i16 | class |
| `+0x22` i16 | runtime: loaded file |
| `+0x24` i16 | runtime: load state |
| `+0x26` i16 | runtime: frame stamp |
| `+0x28` | runtime: model set |

**`TYPE`** (`0x140`):

| offset | field |
| --- | --- |
| `+0x00` | name[32] (atree suffix) |
| `+0x20`, `+0x30` | look nodes |
| `+0x40` | effect node |
| `+0x50` i16 | `DESC` |
| `+0x52` i16 | subtype (−1 part only) |
| `+0x54` | — |
| `+0x56`/`+0x58`/`+0x5A` i16 | resolved nodes |
| `+0x5C` u32 | flags: 1 shadow, 2 `NODE` hit spheres, 4 health meter, 8 meter variant, 0x10 look node from the model, 0x20 leash is a box, 0x40 move along `+0x3F8`, 0x80 no wake timer, 0x100 walls by spheres, 0x400 face target freely, 0x800 `GMETER`, 0x1000 model flag, 0x10000 has movement moves (set by the loader) |
| `+0x60`..`+0x74` | look limits (6 × f32) |
| `+0x78` | height |
| `+0x7C` | radius |
| `+0x80` | target condition (8 × f32) |
| `+0xA0` | home (used if `+0xA4` < 999) |
| `+0xAC` | leash |
| `+0xB0` | hover above the floor (the dragon flies at 18.5) |
| `+0xB4` | aim height |
| `+0xB8` | not traced (20/50) |
| `+0xBC` | armour |
| `+0xC0` | centre offset |
| `+0xCC` | max turn from the yaw it was made facing |
| `+0xD0` | key offset |
| `+0xDC` | not traced (2π/3) |
| `+0xE0` | resistances |
| `+0xE4` | hit points |
| `+0xE8` | experience |
| `+0xEC` | wake distance |
| `+0xF0` | not traced |
| `+0xF4`/`+0xF6` i16 | hit `SFXX` |
| `+0xF8`..`+0xFE` i16 | meter parameters |
| `+0x100`..`+0x108` | meter offset |
| `+0x110`/`+0x114`/`+0x118` | i16 count + i16 first for `MOVE`/`PTRN`/`NODE` |
| `+0x11C` i16 | child |
| `+0x11E` i16 | parent (shares atree) |
| `+0x120`..`+0x138` | runtime pointers: `DESC`, moves, patterns, nodes, file, `ADDA` list, atree |

**`MOVE`** (`0x90`):

| offset | field |
| --- | --- |
| `+0x00` i32 | kind (table below) |
| `+0x04` u32 | flags: 1 no look / not interrupted, 2 needs all parts, 4 disabled, 8 animation flag, 0x10 needs its node, 0x20 face home |
| `+0x08` i32 | priority |
| `+0x0C`/`+0x0E` i16 | runtime anim/node |
| `+0x10` | name[16] |
| `+0x20` | anim[16] |
| `+0x30` | node[16] (fewer than 2 characters: none) |
| `+0x40`/`+0x44` i32 | hit frames |
| `+0x48`/`+0x4A` i16 | `DAMG` |
| `+0x4C` | repeat step (0x85) |
| `+0x50`/`+0x52` i16 | window ends |
| `+0x54` i16 | next |
| `+0x56` i16 | transition |
| `+0x58`/`+0x5A`, `+0x5C`/`+0x5E` i16 | (`SFXX`, frame) × 2 |
| `+0x60` | condition (8 × f32) |
| `+0x80` | cooldown (s) |
| `+0x84` | speed (units/s) |
| `+0x88` | turn (rad/s) |
| `+0x8C` | hold (s) |

Move kinds:

| kind | meaning |
| --- | --- |
| 0x00 | INIT |
| 0x01 | together |
| 0x10 | START |
| 0x11 | DEATH |
| 0x20 | READY |
| 0x21 | TAUNT |
| 0x22 | ROAR |
| 0x23 | BLOCK |
| 0x30–0x39 | movement: 0x32 left, 0x33 right, 0x34 walk, 0x35 back, 0x36 diagonal, 0x37 on, 0x38 to point |
| 0x40 | flinch |
| 0x41 | knockback |
| 0x42 | knockdown |
| 0x7F–0xEF | attacks: 0x80/0x83/0x86 sweeps, 0x81 grab, 0x85 repeat, 0x88 aimed |
| 0xF0 | skip |

**`PTRN`** (`0x50`):

| offset | field |
| --- | --- |
| `+0x00` | name[16] |
| `+0x10` u16 | flags: 2 needs all parts, 0x1000 never picked alone |
| `+0x12` | — |
| `+0x14` | cooldown |
| `+0x18` | bytes |
| `+0x20` | 8 × i16 moves (−1 ends) |
| `+0x30` | condition |

**`NODE`** (`0x50`):

| offset | field |
| --- | --- |
| `+0x00` | node[16] (empty: root) |
| `+0x10` u16 | flags: 2 breakable, 4 hide when broken, 8 solid |
| `+0x12` i16 | `DAMG` on break |
| `+0x14` i16 | runtime node |
| `+0x16` i16 | model flag |
| `+0x18`, `+0x1C` | tool priority values |
| `+0x20` | offset |
| `+0x2C` | radius |
| `+0x30` | second node, or `+n`/`-n` model steps |
| `+0x40` | damage scale |
| `+0x44` | hit-point share |

**`DAMG`** (`0x50`):

| offset | field |
| --- | --- |
| `+0x00` i16 | kind 0–9 |
| `+0x02` u16 | flags: 1 aim at target, 4 along the body, 8 straight, 0x40, 0x800, 0x1000, 0x2000, 0x4000 |
| `+0x04` u32 | blow kind bits |
| `+0x08` | life (projectile) / thickness (cone) |
| `+0x0C` | radius / length |
| `+0x10` | min range |
| `+0x14` | yaw |
| `+0x18` | param |
| `+0x1C` | pitch |
| `+0x20` | offset |
| `+0x2C` | damage |
| `+0x30`/`+0x34` | speed min/max |
| `+0x38` | gravity |
| `+0x3C` | trail life |
| `+0x40`..`+0x46` i16 | `SFXX`: projectile, hit, trails |
| `+0x48` | spread |

**`SFXX`** (`0x50`):

| offset | field |
| --- | --- |
| `+0x00` u32 | flags (0x100/0x200 model effects, 0x400 hide body, 0x801 attach, 0x40/0x80 at node/spawn, 0xF000000 particle systems `0x8003dd88`, 0x40000 kill effect, 0x2 shake, 0x20 generator) |
| `+0x04` i32 | next |
| `+0x08`/`+0x0C` | runtime effect/sound |
| `+0x10` | effect[16] |
| `+0x20` | sound[16] (`%c` = realm letter) |
| `+0x30` | offset |
| `+0x3C` | life |
| `+0x40` | scale |
| `+0x44`/`+0x46` i16 | effect parameters |
| `+0x48` | model node |
| `+0x4C` | size |

**`ADDA`** (`0x30`):

| offset | field |
| --- | --- |
| `+0x00` i16 | type |
| `+0x02` u16 | flags (1 attach to a node) |
| `+0x04` | runtime anim |
| `+0x08` | runtime next |
| `+0x10` | anim name |
| `+0x18` | node name |
| `+0x20` | offset |

## Constants

| where | value | use |
| --- | --- | --- |
| `r2-0x723c`, `r2-0x7240` | 4.5, 0.5 | anger |
| `r2-0x7168` | 0.8 | TAUNT below this anger; knockback decay |
| `r2-0x7120` | 1/30 | frame time for move ends |
| `r2-0x7128` | 50 | ROAR damage |
| `r2-0x7228` | 0.25 | BLOCK damage; player hit immunity |
| `r2-0x7290` | 3 | damage-taken memory |
| `r2-0x7200` | 1000 | recently hit player score factor |
| `r2-0x71dc` | 2 | target behind factor |
| `r2-0x71f8` | 0.75 | projectile speed span |
| `r2-0x7108` | 1.5 | anger cap for projectile speed |
| `r2-0x71a8` | 0.2 | kill experience |
| `r2-0x7194`, `r2-0x7190`, `r2-0x7270`, `r2-0x7198` | 10, 7.5, 5, 20 | knockback |
| `r2-0x7188` | 5 | golem knockback reduction |
| `r2-0x7178`, `r2-0x7180` | 40, 1600 | knockback cap |
| `r2-0x7134` | 100 | knockback fall |
| `r2-0x7248` | 1.946 | look-node turn rate |
| `r2-0x7130` | 999999 | attack-choice start |
| `r2-0x7268` | 1e21 | movement-choice start |
| `r2-0x7220`…`r2-0x7204` | 2e21 … 1.01e21 | rejection scores |
| `0x8011a90c`, `0x8011a920` | tables | roar threshold and boss damage scale by player count |

## In this rewrite

[`critters.rs`](../crates/gdl-game/src/critters.rs) runs the **placed
golem** (enemy 0x1D) on the 30 Hz tick, interpolated for drawing. Bosses,
the gargoyle and the general aren't run yet.

- **Loading.** When the level's monster state is set up, the level's golem
  slot loads its file (`golem`, `golemF` or `golemI` by realm) and builds its
  body model from `MONSTERS/golem/level<realm>`. It also builds the
  `GOL_STATUE` atree from the same folder.
- **Statues.** Each placed golem (`ENEMYINFO` for one player) stands as
  `GOL_STATUE` on the floor below its placement.
  - `mechanics.rs` now lists the placements of wake triggers (flag `0x2000`)
    that come on (`Mechanics::woken`).
  - Each wakes the nearest statue whose horizontal distance, less the item
    type's radius, is under 10.
  - The level-data survey `every_real_statue_and_its_wake_trigger` finds
    377 placed critters on the disc, only 13 with a wake trigger in reach
    (A1, C2, G2, G3, I1, J1, K1). Most statues are scenery, as in the game.
- **Waking.** A woken statue plays `ACTIVE`, then the golem replaces it.
  It gets hit points `TYPE +0xE4` × the level's `+0xAC` and state 0.
- **Each tick** it:
  1. turns blows taken into knockback, and forgets damage after 3 s or
     during ROAR and hit reactions;
  2. scores the players with `TYPE +0x80`, a player hit in the last 0.25 s
     scoring × 1000;
  3. updates anger, and goes to state 3;
  4. picks a move: forced, then block, then attack (least recently used,
     with cooldowns and conditions), then movement (lowest score), then
     TAUNT/READY;
  5. switches by the transition table and animation modes, recording move
     ends for cooldowns;
  6. aims the move at its target and lands blows on their frames;
  7. plays the move's `SFXX` sounds (`%c` = the realm letter);
  8. walks at `MOVE +0x84` × the level's `+0xB0` in the move's direction,
     plus knockback. It takes the walls from 2 units up with its radius,
     probes the floor at the leading edge (from its height to −height − 3,
     rise ≤ 2 × (radius + move)), drops at most 16/s, and stops short of
     the hero;
  9. turns at `MOVE +0x88`.
- **Blows** (`DAMG`):
  - Kind 0 sweeps the sphere on the move's node (the bone's world matrix,
    offset `+0x20`, radius `+0x0C`) from last tick to this against the
    hero's cylinder.
  - A hit deals `+0x2C` × the level's `+0xBC` through `queue_hit` and
    `DamagePlayer`, less the hero's armour. The push is 0.5 × (node
    movement + unit(to hero, y = 1)), and the hero can't be hit by a
    critter again for 0.25 s.
- **Blows on it** (`damage.rs`, `TargetKind::Object` → `Critter::take_hit`):
  - blocking takes × 0.25 and no knock kinds;
  - `TYPE +0xBC` armour comes off (`after_armor`);
  - the hero earns min(damage, hp) ÷ (1 + full) × `TYPE +0xE8`, plus 0.2 ×
    `+0xE8` for the kill;
  - the kinds and push feed the reactions and knockback;
  - the `TYPE +0xF4` hit sound plays.
- **Death.** At 0 hit points it loses `Targetable`, plays DEATH, and is
  removed when that ends.
- **Verified** in levelJ1 and levelC2:
  - Stepping on the wake trigger wakes the statue next to it.
  - The golems play INIT, START, TURN/WALK, then block, attack in rotation
    (ATTACK1L → ATTACK1R → ATTACK2 → ATTACK3 → ATTACK4), roar and taunt.
    They hit the hero (8.5–58.5 per blow) and knock him down.
  - The hero's blows take them down (blocked ones barely scratch them),
    and they play DEATH and go.
- **Testing aids:**
  - `GDL_WAKE_STATUES=<range>` wakes statues when the hero comes that
    close;
  - `GDL_CRITTER_HP=<scale>` scales critter hit points;
  - `GDL_CRITTER_SHOT=<png>` saves a screenshot a few ticks into the first
    critter death.

### Stand-ins and gaps

- **Wake-up.**
  - The wake search only looks at statues. The game takes the nearest
    `ENEMYINFO` item of any kind, if flagged on screen or always active.
  - The statue's own animation states (`+0xC4`/`+0xCA`/`+0xC6`) are
    reduced to "play ACTIVE, then spawn".
- **The stomp ring** (`DAMG` kind 3, ATTACK4) hurts heroes within its
  radius at once. In the game it's a damaging effect in the projectile
  table, lasting `+0x08` s.
- **Projectile kinds** (1, 2, 8), breath cones (4), generator effects (5,
  6), grabs (7) and kind 9 aren't done.
- **Timing.** Critter time starts at 0 each level. Moves that have never
  run count as long ago, so they're ready; the game's clock runs from boot.
- **Only one target is tracked.** Condition `[6]` (distance from home) and
  the multi-target weighting are bosses' and aren't done.
- **Not done:**
  - look nodes, the health meter (`GMETER`), hit effects and flashes,
    fading, shadows;
  - breakable `NODE`s: blows land on the body, and `TYPE` flag 2's node
    spheres aren't used;
  - dropping `+0xACC` items;
  - critter blows on monsters (`0x80035908`);
  - pushing players aside (`0x800350c8`) — the golem only stops short;
  - critter-vs-critter and critter-vs-monster collision;
  - the level-vs-hero-level damage scale;
  - elemental resistances (`TYPE +0xE0`).
- **Player "attacking"** (for BLOCK) is the hero's action being in an
  attack or defend category, rather than the game's intent test
  (`0x80074564`).
- **Node positions** come from the bones' last drawn transforms, one frame
  behind. The critter's own animation clock (frame and end, at the clip's
  rate) drives the logic.

## Boss work in progress

`critters.rs` now also runs one-part bosses. It was checked on levelB6's dragon.

**What runs:**
- **Spawn.** The level's boss type (`LEVL +0x44`) loads its file and spawns
  at the boss locator (kind 6), dropped onto the floor below and raised by
  `TYPE +0xB0`. That call is `0x8001bb54` → `0x8003df60(4, 0, …)`.
  - Hit points are `TYPE +0xE4` × level `+0xAC`.
  - Home is `TYPE +0xA0`, or where it spawned.
  - It is leashed to `TYPE +0xAC` of home (a box with `TYPE` flag 0x20) and
    ignores the level's collision.
- **Waking** (`0x800399f0`, state 0). It plays INIT, waits 2 s (none with
  `TYPE` flag 0x80), then wakes once its targets are within `TYPE +0xEC`.
- **Targets.** It tracks up to 4 players that pass `TYPE +0x80`, with
  weight 1, and so only one player here (`0x80036ed4`).
- **Moves.**
  - After START the forced move is READY.
  - Patterns run with their cooldowns (`PTRN +0x14`); a pattern's steps
    follow one another.
  - A move is held for `MOVE +0x8C` past its animation.
  - DEATH is removed when its hold ends. There is no fade (stand-in).
- **Blows:**
  - DAMG kinds 1, 2 and 8 fire missiles through
    `projectiles::spawn_critter_missile`.
    - They start at the move node plus `+0x20` in the critter's space.
    - Speed is min + (clamp(anger, 0.5, 1.5) − 0.5) × 0.75 × (max − min).
    - They are aimed at the target's centre on the flatter ballistic arc
      under `+0x38` (flag 8: straight).
    - Kind 1 adds yaw `+0x14` ± spread `+0x48`.
    - The model is the effect atree named by `SFXX +0x40` (e.g.
      `FBALL_LOOP`), drawn at scale `+0x08`. It draws nothing without the
      effects system, so a glowing sphere of half the hit radius stands in.
  - Kind 4 is a breath cone: the node's forward axis turned by
    `+0x14`/`+0x1C`, length `+0x0C`, thickness `+0x08`, from `+0x10`
    outward. It hits every frame of a 0x83 window, subject to the 0.25 s
    guard.
- **Hit spheres.** Critters with `TYPE` flag 2 are hit on `NODE` spheres
  (`CritterSphere` entities following their bones; `damage.rs` maps them
  back to the critter). Each blow is × `NODE +0x40`, capped at
  `NODE +0x44` × hit points per sphere.
- **Bosses taking damage.** Damage is × `0x8011a920[players]` (1.0 for one
  player), experience is × players, and there is no knockback. The roar
  threshold is 50 × `0x8011a90c[players]`.
- **Boss death** sets `CritterLevel::boss_dead` and logs where the key would
  drop (`TYPE +0xD0`, `0x8001b854`).

**Checked on B6:**
- The dragon wakes and plays START (its ROAR animation), then READY, then
  fireballs, claws, breath, stomp and wing.
- Its missiles, claws, breath and stomp hit the hero.
- The hero's thrown axe and blows hit its spheres.
- It dies and is removed.
- Screenshots were taken with `GDL_CRITTER_SHOT_ON=missile|hurt|death` and
  `GDL_CRITTER_SHOT_DELAY`.

**Next:**
- Parts (`TYPE +0x11C` children, the chimera's heads). The spawn is
  `0x8003df60` with children linked by `+0xAD8`/`+0xADC`. They share the
  body (`0x80036d18`) and patterns (`0x8003b6f0`, `0x8003bb40`).
- The boss key as a real item. This needs a hook to build extra item
  models.
- The intro state machine (`r13-0x725c`, `0x80056c00`–`0x80057020`) and the
  boss camera (`BCAM`, around `0x8001bbcc`).
- Kinds 5, 6, 7 and 9, the look nodes, and breakable nodes.
- Checking the golem against hit spheres: blows on its BALL/HANDR spheres
  run out at 0.25 × its hit points.
