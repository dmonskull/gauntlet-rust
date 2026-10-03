# Monsters and generators

Implemented in [`crates/gdl-formats/src/enemy.rs`](../crates/gdl-formats/src/enemy.rs)
(per-type stats, tiers, the realm's monster slots),
[`crates/gdl-formats/src/critter.rs`](../crates/gdl-formats/src/critter.rs)
(`CRITTER/*.WAD`, the bosses' data), `LevelTuning`/`RealmEnemy` in
[`world_data.rs`](../crates/gdl-formats/src/world_data.rs), and at run time
[`monsters.rs`](../crates/gdl-game/src/monsters.rs) and
[`generators.rs`](../crates/gdl-game/src/generators.rs).

Verified: every generator and placed monster in every level (6,999 of
them) resolves, through its realm's enemy list, to an atree for its tier in
the folders that level loads (`enemy::tests::every_real_level_monster_has_a_model`;
the 477 others are critter/boss types or Death, which this doesn't run).
All 18 `CRITTER` files parse and their `TYPE` spans tile the `MOVE`/`PTRN`/
`NODE` tables exactly. In game, levelA1's grunts walk out of their doors,
follow the hero around and swing at him; levelB1's generators make trolls.

## Two monster systems

- **Enemies** — grunts, demons, ghosts, rats…: everything generators make
  and most placed monsters. No data file: their stats are tables compiled
  into `main.dol`, their models `MONSTERS/<name>/`. The runtime here.
- **Critters** — the game's scripted-monster system: bosses, and the
  golem (29), Death (30), gargoyle (32) and general (33). Their behaviour is
  `CRITTER/<boss>.WAD` (loaded by `FUN_80040008`, spawned by
  `FUN_8003df60`); generators of those types call into it
  (`FUN_80060100`: types 0x1D, 0x20, 0x21 → `FUN_8003df60(3|7|8, …)`). Only
  parsed here, not run.

## Enemy type tables

34 four-byte entries each, indexed by enemy type id (the id in
`population::ENEMY_CODES`; 28 unused). Read by the enemy setup
`FUN_8004fd9c` / `FUN_8004ffbc` and the frame update `FUN_8004cfe0`:

| table | what | record field |
| --- | --- | --- |
| `0x8011b0b8` f32 | 2 × floor step (3/6/10/12) | `+0x23C` = × 0.5 (`r2-0x6ef0`) |
| `0x8011b140` f32 | collision radius (0.75–6) | `+0x238` |
| `0x8011b1c8` f32 | not traced (2/3.8/5) | `+0x224` |
| `0x8011b250` f32 | centre height (1.5/3/4) | `+0x230` |
| `0x8011b2d8` f32 | ground per video field (0.1; 0.12 demon, 0.02 acid blob) | speed table `0x80250810` = × fields × level `+0xB0`; `+0xB8` = 1 / it |
| `0x8011b360` f32 | damage (12/15/18…) | `+0xBC`, × level `+0xBC`, × the tier third |
| `0x8011b3e8` f32 | 0 except Death | `+0xC0` |
| `0x8011b470` f32 | hit points (21/30/46, 100 grm, 200 golem, 9999 "it") | `+0x200`, see tiers |
| `0x8011b7a0` i32 | default AI (7, 2 for sco/rat/mag, 19 golem, 3 Death, 27 it) | `+0x310` when none given |
| `0x8011b828`, `0x8011b8b0` f32 | 0 | `+0xC4`, `+0xC8` |
| `0x8011b938` f32 | turn per field: π/64 for every type | used by `FUN_8004cb20` |

`0x8011b4f8`, `0x8011b580`…`0x8011b718` are more per-type values (score?)
read by `FUN_8002f288`/`FUN_8002f400`; not traced. `0x8011bb04` holds the
avoidance angles 0, π/8 … 7π/8; `0x8011bb24` AI 0's search angles.

**Time**: the monster code counts video fields. `r13-0x7584` is fields since
the last frame (`FUN_8002eff4`: 2 at the game's 30 fps), so per-field rates
act twice per 30 Hz tick and timers count down by 2 a frame.

### Tiers

A monster's tier (1–3 from a generator's strength, the level from a placed
monster; 4–7 the special variants) scales it (`FUN_8004fd9c`):

- hit points = table × level `+0xAC` × 0.333 (`r2-0x6cf8`, the literal) ×
  tier, for types below 28; tiers above 3 count as 2 and AI 18 as 1;
  special types get the full table value.
- `+0x206` strength (`FUN_8004ffbc`): 3 if hit points > 0.667 × full
  (`r2-0x6cf0`), 2 if > 0.333 × full, 1 if > 0.
- damage = table × level `+0xBC`, then × 0.333 or × 0.667 when the hit
  points are at most the second or first third (not for Death).
- The model is the atree `"%s%d"` (`FUN_80051d84`): the monster's name and
  the tier (0 → 1), upper-cased: `GRU2`; tiers 4–7 use `"%s%c"` with the
  letters at `r13-0x7f0c + 4`: `A B S F` (`GRUA`: throwers, `GRUB`: bombers,
  `GRUS`: suicide runners). If the atree's missing it draws the object
  `<name><tier>L1`.

## A level's monsters: the realm's enemy list

The realm WAD's `ENMY` chunk (`0x18`-byte records: i32 type, i32 subtype,
name[16] that `FUN_8009d940` builds the monster's sound names from, e.g.
`"S_%s%sCLOSE"`) lists every enemy the
realm uses; each level record lists up to six of them at `+0x4C` (i16
indices, −1 none). `FUN_80057738` builds the level's eight enemy slots from
those (plus Death, type 0x1E, in a free one unless the level has a boss),
and `0x8025718c[subtype] = type` for subtypes below 6:

| subtype | slot | castle A1 | mountain B4 |
| --- | --- | --- | --- |
| 1 | small | rat | scorpion |
| 2 | main | grunt | — |
| 3 | elite | — | demon |
| 4 | special variants (tiers 4+) | — | troll (`TROAUX`) |
| 5 | critters | golem, general, gargoyle | golem, … |
| 9 | the boss | | |
| 11+ | loads `MONSTERS/<name><subtype − 10>` (hell: `WAR3`, `DEM3`…) | | |

`FUN_80057020` loads each with `FUN_80050af4(type, subtype)`: folder
`monsters/<name>aux` for subtype 4, `monsters/<name>%d` (subtype − 10) for
11+, else `monsters/<name>` (`FUN_80050d40` builds the same names).

**Level data only names placeholders** — `gru`, `kni`, `rat` (and a few
`tro`). `FUN_80050f18(type, tier)` swaps in the realm's own when a
generator or placed monster is built (`FUN_800646e4`):

- `rat` → the small slot;
- `gru`, `kni` → the special-variant slot for tier ≥ 4, else the main slot,
  else the elite slot, else unchanged;
- anything else unchanged; a type the level didn't load is refused (−5,
  `"Bad EnemyInfo (type %s) subtype %d"`).

Generators pass their `+0xDE` byte as the tier, which is their live count,
0, at that point — so a generator never makes special variants; placed
monsters pass their level.

**Special generators** (realms 5 and 6, E and F; `FUN_800646e4` before the
swap, `r13-0x7220` the realm id): every generator there — their levels
name the type `SPECIAL` — has its type renamed `CAT` (E) or `HEL` (F), its
monster type set to −2 or −3 and its strength `+0xE2` to 2 or 3 (after
its hit points, max and rate were taken from the placement's), and draws
`GEN_SPECIAL<strength>` from the realm's items (`GEN_SPECIAL0`–`2` in
`ITEMS/levelE`, `0`–`3` in `levelF`). The monster maker (`FUN_8004f41c`)
turns −2 into the next of a round shared by them all (`r13-0x73d4`, & 3):
types `0x8011BA64` [ice, imp, pla, zom] with AIs `0x8011BA74` [7, 7, 7, 7]
at tier 2; −3 into `0x8011BA84` [dem, war, gho, sky], AIs `0x8011BA94` [30,
30, 7, 7] at tier 3; a type the level didn't load is refused (−5). Every
other generator draws `GEN_<code><strength>` with its monster's code after
the swap above (an ice realm's `gru` generators draw `GEN_ICE<n>`).

Here (`generators.rs`, `population.rs` `GeneratorLook`): both. Before,
the population drew the placeholder's generator (`GEN_GRU…` in the ice
realm) and the special realms' generators weren't made at all.

## Hit and death sounds

`FUN_8009d940(slot, name)` runs for each realm `ENMY` record (not subtypes
5 and 9) and looks up sound ids by name (`FUN_8001801c`, catalog names are
15 characters, so longer names match their first 15). The name prefix:

- Types 1 2 4 5 7 8 10 11 13 14 16 17 19 20 23 24 25 26: `<name>1` for
  weak and `<name>2` for strong monsters when the subtype is below 10 and
  the level's `+0x44` boss is -1; otherwise `<name>2` for both.
- Type 0x1B: `<name>1`. Type 0x1D (golem): `GOL<realm letter>` (T → G,
  `FUN_80057a68`), and `KILL` instead of `DIE`; it also loads
  `S_GOL%cSTOMP`, `BORN`, `SWING`.
- Others: the name as is.

Sounds: `S_<p>DIECLOSE` / `S_<p>DIEFAR` (weak prefix → `0x8028b514` /
`0x8028b554`, strong → `0x8028b534` / `0x8028b574`); hits `S_<p>HITCLOSE` /
`HITFAR` for the weak prefix, and for the strong one `S_<p>HIT1CLOSE`,
`HIT2CLOSE`, `HIT1FAR`, `HIT2FAR` when both prefixes exist, else
`S_<p>HITCLOSE`/`HITFAR` for both. When the level's boss is 0x24,
0x25 or 0x29 every name is cut to 14 characters and `C`, `D` or `B` added
(`S_SKE2DIECLOSEC` on levelC5, `S_SPIDDIECLOSED` on levelD5,
`S_MAGDIECLOSEB` on levelG5). Types 2 8 0x11 0x13 0x18 0x19 and the plain
ones also get `S_<p>BITE`, type 0xB `S_<p>STRIKE` (`0x8028b654/674`; where
they play isn't traced).

`FUN_8004e660` counts a blow that does damage in `+0x204` and then plays
(`FUN_8009d6c0` hit, `FUN_8009d7b4` death; `param_7` 1 close, 2 far —
thrown missiles pass 2):

- hit: strength `+0x206` < 2 → the weak `HIT`; else `+0x204` < 2 → `HIT1`,
  else `HIT2`.
- death: strength < 2 → the weak `DIE`, else the strong one.

`+0x206` is set once when the monster is made (`FUN_8004ffbc` from its hit
points against the full ones), i.e. it follows the tier.

Runtime: `enemy::monster_sounds` builds the names, `damage.rs` plays the
close ones for hand blows and the far ones for missiles (`Hit::ranged`).
Stand-ins: sounds aren't positioned, and a
weak monster whose realm has only the strong set (the game leaves that
slot unset) uses the strong first-hit sound.

## Level tuning (level record, `0x10C` bytes)

`r13-0x72bc` points at the current level's record (set by `FUN_80058074`):

| offset | used for |
| --- | --- |
| `+0x44` | boss enemy type (−1 none) |
| `+0x4C` | 6 × i16 `ENMY` indices |
| `+0x8E` i16 | monster slots (`r13-0x73bc`): 13–25 alive at once |
| `+0xAC` | × monster hit points (critters' too, `FUN_8003e838`) |
| `+0xB0` | × monster speed |
| `+0xB4` | × monster awareness (30 units, `r2-0x6cb0`) |
| `+0xBC` | × monster damage |
| `+0xC0` | × the throw actions' timing (`FUN_800ab110`) |
| `+0xCC` | × generator hit points |
| `+0xD0` | × generator rate |
| `+0xD4` | × generator max |
| `+0xD8`, `+0xDC` | read by other item code (`FUN_800606e8`, `FUN_800646e4`); not traced |

The forest realm and some secret/test levels have 0 in all the scales.
**Stand-in**: we read 0 as 1 (taken literally their monsters would have no
hit points and notice nobody; what the game does there isn't confirmed).

## Generators

Built by `FUN_800646e4` (class 3) into the `0xF0`-byte item records
(`r13-0x71a8`):

| item field | from |
| --- | --- |
| `+0xD0` i16 hit points | type `+0x44` × strength × level `+0xCC` |
| `+0xD4` radius | × 4 (`r2-0x6650`) for the on-screen test |
| `+0xDC` i16 type | the monster name, substituted (above) |
| `+0xDE` alive count | 0 |
| `+0xDF` max | placement `+0x34`, 0 → 10/5/2 by strength (`0x8011c328`), × level `+0xD4` |
| `+0xE0` made | 0 |
| `+0xE2` strength | placement `+0x30` (< 1 → 1, `"Generator at %.1f %.1f %.1f has strength %d"`) |
| `+0xE3` AI | placement `+0x32`; negative → the type's default |
| `+0xE4` i16 wait | fields |
| `+0xE7` rate | placement `+0x36`, 0 → 5/10/15 (`0x8011c334`), × level `+0xD0` |
| `+0xE8` ramp | 0 |
| `+0xEC` yaw | `atan2(m20, m22)` of the placement matrix (its front) |

The item update `FUN_800606e8` runs an item's class code only while it's
on screen (`FUN_800b4ef4`, the item radius around the item: flag
`0x4000`) or always (flag `0x40`, from placement flag bit 0 — 1,308 of
4,721 generators). For a generator (case 3), once the game is running:

1. if alive < max, `FUN_800635a0`: count the wait down by the fields
   elapsed; when it's run out, spawn if a player is within 1000 units
   (`r2-0x6628`, i.e. always) and either it's on screen or no monster
   slot scan has come up full this frame (`r13-0x719c`).
2. `FUN_8004f41c(reach, position, type, strength, front, AI, item, …)`
   makes the monster (below); on success: wait = 6 (`r2-0x66f0`) × rate ×
   (1 + ramp) fields; ramp += 1 / (2 (`r2-0x67f0`) × max), back to 0
   past 1; alive and made + 1; the monster's facing = the generator's yaw
   + the spot's angle.

AI 15 generators (`+0xE3 == 0xF`) instead make one monster through
`FUN_80063430`. The debug display: `"Generators"` (`FUN_8002e650`).

### Blows on a generator (`FUN_8005c1c8`)

Item hits: for a generator hit by a player whose level record `+0x9C`
(experience level) is above 0, the damage is scaled by the hero's level
`L` against it: × (1 + 0.1 × (L − lvl)) above, × (1 − 0.01 × (lvl − L))
below, and at least 1. Then the item's armour `+0xCF` (the type's `+0x42`
byte; -1 = none) comes off, at least 1 left, and the result, rounded half
away from zero (`FUN_800bec14`), comes off the i16 hit points `+0xD0`.
The strength `+0xE2` is recomputed from the type's hit points × level
`+0xCC` (`per`): 3 above 2 × per, 2 above per, else 1, 0 at none; a change
swaps the model (`GEN_%s%d`, `GEN_SPECIAL%d`) — at 0 too, to the wreck
`GEN_<code>0`: when that model exists it stays, flag 1 cleared and armour
`0xFF` (no more blows); only without one is the item freed (`FUN_8005ba08`
around the `GEN_%s%d` lookups: `FUN_800674e0`, then `FUN_800b8684` and two
fallbacks) — and its monsters forget it. Here: `damage.rs`
(`GeneratorLooks::broken`). Experience: `docs/items.md`.

Sounds, by the realm id `r13-0x7220` (the level id >> 8: A–K = 1–11, S 12,
L 13, T 0): `FUN_8009bfac` plays the hit sound from `0x80123910[realm]`,
`FUN_8009c010` the destroy sound from `0x801238dc[realm]` (only when the
level has no boss, `r13-0x7764` < 0). They are `S_GENDAM<letter>` and
`S_GENKILL<letter>` for A–K (S and T none, L `S_DEFEATVOXB` on a hit); the
jungle's (J) generators of type 0x18 use `S_GENDAMWAR` / `S_GENKILLWAR`.

### Making a monster (`FUN_8004f41c`)

1. The AI: `FUN_8004f7e4(type, tier, ai)` — small monsters (sco, rat, sna,
   spi, mag, wol, dog, aci, han) get 2 or 4 at random unless given 2 or 4;
   the main types given AI 0 get 7 (tier 4 → 0x17, 5 → 0x11, 6 → 0x12;
   dem/sor/pla/wrm/war tier 3 → 0x1E); grm → 0x1F, golem → 0x13, Death →
   3, it → 0x1B. `FUN_8004ffbc` then maps 1 → 0 and 10 → 7.
2. A slot, `FUN_8004fc68`: a free record, else the worst one — score its
   target distance (`+0x27C`, 100000 when none), × 0.01 when dying,
   dormant or placed (`+0x2D8`), + 10000 when not near the screen; take the
   highest. It's refused if the victim is near the screen and the asker
   isn't a placed monster (generators on screen pass 0, off screen −1,
   placed monsters 1). The victim is killed first (`FUN_8004ef4c`) even if
   no spot is found.
3. Set up (`FUN_8004fd9c`/`FUN_8004ffbc`): stats above, awareness `+0x300`
   = 30 × level `+0xB4`, model (`FUN_80050580`), state 1, action START.
4. For a generator: try the eight spots from a random one
   (`FUN_800bcf9c(8)`): direction = the generator's front turned by
   `FUN_8004fb30` (0, π, −π/2, π/2, π/4, −π/4, 3π/4, −3π/4), at item
   extent[0] + the monster's radius out, from the generator's position
   raised by the monster's centre height. `FUN_8004f914` rejects a spot for
   good if a wall (monster radius) lies between, there's no floor (probe
   radius 0.1, 4 up to 10 down) or it's more than 6 from the start height;
   for now if a player or another monster (half radius) is in the way.
   Types 1, 4, 5, 7, 8, 10, 11, 14, 15, 19, 24, 25 only use spots 0, 4 and
   5 (mask `0xFFCE`). No spot: the slot is freed (−3).

## Placed monsters (ENEMYINFO)

`FUN_800646e4` case 4: `+0xDE` level (placement `+0x30`), `+0xDF` AI
(`+0x32`), `+0xE0` yaw, `+0xE8` range (f32 `+0x34`), `+0xEC` (`+0x38`).
`FUN_80060100`: when its spot is on screen (4 × its radius, `r2-0x672c`)
and within 50 (`r2-0x6728`) of `0x8023f1bc`, it's made once where it
stands (`FUN_8004f41c` with no generator), and the item removed. Range > 0
overrides the awareness (× level `+0xB4`). Level 0 (outside types 0x1E,
0x1F) makes it dormant (state 6), else levels below 4 freeze it 30 fields
(`+0x20C`). The data: 2,300 placed monsters, nearly all levels 4–6 (the
special variants) with AIs 16, 17, 18, 23, 26.

**Stand-in**: we measure the 50 from the player; `0x8023f1bc` is a global
position not traced (likely the camera's focus). No level-0 placements
exist on the disc.

## The monster record (`0x394` bytes at `0x802515e8`)

| offset | field |
| --- | --- |
| `+0x000` | enemy type |
| `+0x004` | matrix; `+0x034` position (feet) |
| `+0x054` | position used by the bump tests |
| `+0x064` | model instance; `+0x06C` animation controller |
| `+0x0B4` | state: 0 free, 1 active, 6 dormant, 8 dying |
| `+0x0BC` | damage; `+0x0B8` 1 / speed |
| `+0x0CC` / `+0x0D0` | current / requested action (indices into the names at `0x80126688`: READY 0, START 1, WALK 3, RUN 4, ATTACK1 0xC … ATTACK3 0x10, HIT1 0x1C, DEATH 0x20) |
| `+0x200` | hit points; `+0x206` i16 strength |
| `+0x20C` | freeze, fields |
| `+0x210` | velocity (per frame) |
| `+0x21C` | an offset above the floor (the mover subtracts it); not traced |
| `+0x230` / `+0x238` / `+0x23C` | centre height / radius / step |
| `+0x244` | facing; `+0x24C` heading; `+0x250` previous heading |
| `+0x25C` | knockback |
| `+0x274` i16 | target player; `+0x278` its score, `+0x27C` its distance |
| `+0x284` / `+0x288` | player / monster bumped this frame |
| `+0x290` | its generator (item) |
| `+0x294` | floor height |
| `+0x2CC` i16 / `+0x2CE` i16 / `+0x2D0` | player being struck / hits landed / hit flags |
| `+0x2D8` | placed; `+0x2DA` on screen; `+0x2DC` near the screen; `+0x2DE` aware |
| `+0x300` | awareness range |
| `+0x310` i16 | AI (`+0x314` the original) |
| `+0x324`, `+0x328`, `+0x32C`, `+0x334`, `+0x336` | avoid side, avoid timer (fields), wanderer turns, avoid step, stuck count |

## The frame update (`FUN_8004cfe0`)

Once a frame, for every slot: speeds for this frame (table × fields ×
level `+0xB0`); on screen (`FUN_800b4ef4`, radius 2 × monster radius) and
near the screen (+ 15). Then for each active monster:

1. **Target** (`FUN_80051660`): on the frame whose number & 7 matches the
   slot & 7, or when it has none, the nearest player within awareness
   (distance, plus 2 per monster already on that player when further than
   5 × radius); being within awareness sets `aware`.
2. **Hits** (`FUN_8004dec0`): when an attack animation finished since last
   frame (below), the struck player takes the damage (× 1.5 for ATTACK3;
   `FUN_80078560`), and hits landed + 1.
3. **AI** (`FUN_8004d8c0`): only when the target is within awareness or
   the monster is near the screen — otherwise it's asleep this frame.
   **The AI is the frame's**: the loop saves `+0x310` into `+0x312` before
   the monster's update and puts it back after (`+0x314` keeps the one
   that ran, and an AI that differs from last frame's is set up again,
   `FUN_800502fc`), so an AI that switches to another mid-frame (and runs
   it at once through `FUN_800467bc`) is back the next frame, unless it
   wrote `+0x312` too (the wanderers' 2 ↔ 4 swap, AI `0xD`'s chain
   breaking to 7). `FUN_800467bc` clears the velocity and switches on the
   AI:
   - **7** (`FUN_80047f04`, the chasers): no target or not aware → AI 5/6
     (by slot parity). Else, when no avoid timer runs: heading = angle to
     the player (`FUN_8002c780`: `atan2(dx, dz)`), or when a wall stopped
     it, that ∓ the avoid angle on the side whose point 30° off the facing
     is nearer the player (`FUN_8004cda0`), or when a monster did, the
     current heading ± it. A heading whose next step overlaps a monster or
     meets a wall (`FUN_8004c834`), or that undoes a turn of over 2°
     (`r2-0x6e40`), isn't taken; after 10 such frames it goes straight at
     the player. `FUN_8004cc84(1.0, heading)`: request WALK (RUN from
     1.25, `r2-0x6d68`) and, only while the current action is WALK, RUN or
     RUNATTACK, add speed × (sin, cos) of the heading. `FUN_8004cb20`
     turns the facing toward the heading by at most turn × fields (× 3,
     `r2-0x6e68`, running).
   - **2 / 4** (`FUN_8004710c`, `FUN_80047564`, the wanderers): target
     score ≤ 8 (`r2-0x6e20`) → AI 0. Else walk on along the heading; when
     the avoid timer (set to 20 by a wall or monster) runs out, turn π/4
     (`r2-0x6e18`; AI 2 one way, 4 the other), swapping AI after 4 turns;
     touching a player turns it at them.
   - **0** (`FUN_80046abc`): like 7, but searches nine offsets
     (`0x8011bb24`) for a free heading.
   - **5 / 6** (`FUN_800477ac`, `FUN_80047b58`, the unaware): walk the
     heading (`FUN_8004cc84(1.0, …)`). While the avoid timer runs it counts
     down, and when it runs out the heading turns a quarter (π/2,
     `r2-0x6e08`: 5 subtracts, 6 adds) and a turn is counted (to 4, round).
     A wall within radius + 0.5 ahead at 0.1 + radius above the feet
     (`FUN_8000d308`, any-hit walls, radius 0.1) or a next step that's
     blocked (`FUN_8004c834`) turns the heading a quarter at once and, if
     the timer is out, sets it to 20. **Most AIs hand an unaware frame
     over to them**: with no target or not `aware` (`+0x2DE`), AIs 0, 1,
     3, 7, 8, 10, `0xD`, `0xE`, `0x13`–`0x16` (`0x13` the golem's) and the
     running suicide runner switch to 5 or 6 by slot parity for the frame
     (`0x13` only once it has spotted a player near the screen) — so a
     chaser near the screen that hasn't noticed a player walks about, and
     chases once it has.
   - **Running from a charging runner**: the frame loop names a leader
     (`r13-0x73b8`): the first slot that is active, AI `0x12`, near the
     screen and running (action or request RUN), kept until it blows up.
     AIs 0, 1, 2, 4–8, 10, `0xC`–`0x10` and `0x16` first check it: when it
     is active, its player's distance is within their own awareness, and
     they aren't it, aren't placed (`+0x2D8`), have no avoid timer and are
     within 10 of it (distance² < 100, `r2-0x6e48`), they take AI `0x18`
     for the frame (`FUN_8004bbc4`): heading = the angle to the leader + π
     (straight away), `FUN_8004cc84(2.0, …)` — a RUN at twice their speed
     (with a blocked frame's heading nudged ±5°…±20° in turn,
     `0x8011b9c0`).
   - **3** Death's (below, "Death": chase as AI 0 with a target, run
     from a haloed hero, else AI 5/6).
   - Others: 8, 10, 12–31 special movers (thrower, bomber, suicide,
     ranged…) — not traced.
4. **Move** (`FUN_800445cc`): add knockback; walls and floor
   (`FUN_800453f0` → `FUN_80045b98`, below); position += velocity; then the
   bump tests along the move — the nearest player (`FUN_800465e8`: its
   cylinder, monster radius + 0.5 + the player's radius `+0x850`, height
   step + the player's `+0x854`) and other monsters (`FUN_800463d4`,
   radius + radius; `FUN_8002fa24` is the swept cylinder test; already
   overlapping only counts moving further in). Either one puts the
   monster back. A player: `FUN_800460a8` requests ATTACK1 (ATTACK3 when
   hits landed & 7 == 7) at that player. A monster: the chasers pick a
   side (`FUN_8004cf14`) and step the avoid angle (15 fields; 50 when
   giving up; flip side after 6 steps); others wait 20 (AI 0: 60).
5. **Animation** (`FUN_800ab110`): START, attacks and recoveries play out
   before switching; the switch from ATTACK1/2/4/5 to its `…R` sets hit
   flag 1, ATTACK3 → ATTACK3R flag 2 — that's when the hit lands. After
   ATTACK1R comes ATTACK2 if the monster has one. Locomotion switches at
   once (WALK ↔ READY through WALKTOREADY/READYTOWALK when present).
   Missing actions fall back: WALK ↔ RUN, ATTACK2/3/4/5 → ATTACK1, else
   READY's animation.

### Walls and floor

`FUN_800453f0`/`FUN_80045b98` are the actor mover of
[collision.md](collision.md) with monster values: the tests start 2
(`r2-0x6eb8`) above the feet (minus `+0x21C`); the wall test uses 1.5 ×
radius (`r2-0x6ee8`), and a wall push-out that fails zeroes the move; the
floor probe at the leading edge (+ direction × (radius + move)) has half
the radius and searches from step above to step + 5 (`r2-0x6eb0`) below;
a floor within 2 × (0.1 + radius + move) of the current one is taken
(re-probed under the destination when the rise is over 0.1 × the move),
otherwise the move is cancelled. Y follows the floor, down at most 16
per second (`r2-0x6ec0`); more than 5 below the floor it stood on
(`r2-0x6ea0`) kills it.

### Items in the way

The level's collision has no doors in it (a wall cast goes straight
through a gate): what holds a monster at a door, a chest or a generator
is the mover's last test, `FUN_8005d1f8(monster, from, to, moving)`, run
when neither a player nor a monster put it back (`from` its centre
`+0x54`, `to` that plus the move; `moving` its flag `+0x280`: the move
with its knockback over 0.001, `r2-0x6ef8`). A stopped monster is put
back, its move zeroed, and it steps round the item as round a monster.

- The test is the heroes' touch test (`FUN_8005f0e0`,
  [items.md](items.md) "Touching items") with radius 0.5 × the monster's
  (`r2-0x6790`) and half height 1.5 × that (`r2-0x6788`), against **one**
  item:
  - standing still (`moving` 0) with an item that stopped it last
    (`+0x28C`): that item again;
  - else, once its wait `+0x32C` (fields; counted down while moving) is
    under 1: `FUN_80062fdc(radius, to, …)` — over every item that's
    there (not free, `& 0x8100` clear), has a shape, is in this game
    (`+0xCD`), isn't a sound, a powerup still in its chest (`+0xE8`), or
    a damage tile that isn't on (state 2 or 4 with flag 1): the distance
    from `to` to its centre less its type's radius. The nearest of all
    sets the wait — (distance − radius) × 0.5 × `+0xB8` (1 / the ground
    it covers a tick), 30 at most, when positive: away from every item it
    doesn't look again until it could have reached one — and the nearest
    that's on screen (`0x4000`) or always active (`0x40`) is the one
    tested.
- A touched item stops it by class (`FUN_8005d3c4`):

| class | stops a monster |
| --- | --- |
| powerup, trigger, exit, transporter, rotator | never |
| container | unless the monster is a flier (types `0x1D`, `0x20`) |
| generator | while it stands (`+0xE2`) and doesn't make type `0x11`; a flier passes one no taller than 3 (`r2-0x6778`) |
| damage tile | every type but the fliers, Death (`0x1E`) and types 0 and 3 — which, on a tile that's on, take its damage instead (`FUN_8004e660`) |
| obstacle | all but rock falls `0x28`, leaves `0x31`, debris `0x33`, shot-down walls `0x34`, sinking rocks `0x35` |
| anything else (doors, statues, traps) | always (an open door isn't touched at all) |

Here: `LevelItems::stops_monster` (`items.rs`), called from the monster
tick after its bump tests, with `ItemWatch` on the monster for the last
item and the wait. Before this monsters walked through shut doors toward
a hero standing behind one. Not ported: types 0 and 3 taking a damage
tile's damage.

## Critter files (`CRITTER/*.WAD`)

Everything about critters is now in [critters.md](critters.md); this is
the summary.

A chunk file; `FUN_80040008` looks up eight tags (built from the bytes at
`r2-0x70cc`…) and byte-swaps each record, fixing their sizes:

| tag | record | notes |
| --- | --- | --- |
| `SFXX` | `0x50` | sounds |
| `DAMG` | `0x50` | |
| `MOVE` | `0x90` | |
| `PTRN` | `0x50` | |
| `NODE` | `0x50` | |
| `DESC` | `0x30` | one per file |
| `TYPE` | `0x140` | `"Critter Header has no types"` if none |
| `ADDA` | `0x30` | linked into its `TYPE` (`+0x134`, `"CRITTER: AddAnim has addto idx"`) |

`TYPE`: `+0xE4` f32 hit points (× level `+0xAC`, `FUN_8003e838`: 200
general … 6000 chimera, Skorne); `+0x110`, `+0x114`, `+0x118` i16 counts
each followed by an i16 first index — its `MOVE`, `PTRN` and `NODE`
records (the instance clears that many per-instance slots); `+0x11C` i16
the next part's type (the chimera's heads, spawned together by
`FUN_8003df60`). The first `0x50` bytes are a name buffer with leftover
tool text. Nothing else named.

## What the runtime does

- Level start: the realm's enemy list and tuning for the level
  (`WDATA/*.WAD`), generators and placed monsters from the population with
  the placeholder names swapped, and a model per (type, tier) needed —
  built once (`CharacterModel`) and instanced per monster.
- 30 Hz, after the player: generators (on screen by the game's play
  camera — a 60° × 45° view from its eye to its target, not the window —
  or always-active), placed monsters, then every monster: target, sleep
  unless in range or near the screen, AI 7 / 0 chase or 2 / 4 wander, turn,
  the monster mover, bumps, animation, and a `MonsterHit { monster,
  player, damage, strong }` message when an attack lands. Drawn
  interpolated between ticks.
- Monsters and generators are level entities; a level change clears them.

## Stand-ins and gaps

- **Player cylinder**: radius 1.0, height 5.0 for the bump tests (the
  player record's `+0x850`/`+0x854` aren't decoded; `player.rs` uses 1.0
  too).
- **AIs**: 7 and 2/4 are ported, the throwing AIs `0x10`, `0x11`,
  `0x17`, `0x1A` ([projectiles.md](projectiles.md): facing, throwing,
  backing off, the throw pause) and the suicide runners `0x12` (below); 0
  uses 7's chase (its nine-angle search isn't); every other AI (3, 5/6,
  14…; the fireball casters `0x1C`/`0x1D`/`0x1F` chase and fire on their
  blows) uses the chase too, and variants without a WALK or RUN animation
  stand still instead of gliding. The crowd penalty in target choice isn't
  applied (one player).
- **Unaware** monsters walk and turn as the game's AI 5/6 do, and the AIs
  above hand them unaware frames (`0x13`'s spotting gate isn't done).
  Monsters run from a charging suicide runner (the blocked-frame nudges
  aren't done).
- Zero level scales are read as 1 (above).
- Spot tests skip items (`FUN_8005ef98`); the placed-monster distance is
  from the player.
- Knockback, getting hit, dying, `+0x21C`, `WALKTOREADY`/`READYTOWALK`
  transitions and the critter system aren't done.
- Animations advance on the frame clock at the action's rate, as for
  players: each frame lasts rate / 900 s (`docs/animation-format.md`), so
  the many 60-rate attacks (grunts, knights, ghosts, imps…) play at 15
  frames a second, and the 60-rate walks (rats, imps, plague, skeletons)
  likewise.

## Hit reactions and knockback

`FUN_8004e660` (damage) takes the hit points off at once and accumulates
the blow into the monster: damage `+0x2A0`, kind bits `+0x2A4`, push
`+0x2A8..+0x2B0`. The monster's next update (`FUN_8004db94`, from
`FUN_8004cfe0`) turns that into a reaction:

- kind & `0x10160`, or damage > 10 (`r2-0x6d54`) with kind `0x200`:
  knocked down — action HIT2 (`0x1D`), knockback velocity (`+0x25C`) +=
  push × 40 (`r2-0x6d4c`; 20 when the monster's `+0x23C` exceeds 2, 2 for
  type `0x1D`, 0 for type `0x15`);
- otherwise kind & `0x10`: HIT1 (`0x1C`) with push × 8 (`r2-0x6d48`);
- otherwise: HIT1, no push.

After the reaction a monster left with hit points flashes: `AAAWHITE` over
its body for that update and the next ([rendering.md](rendering.md),
"Texture overrides"; `monsters.rs`, `flash.rs`).

The knockback speed is capped at 40 (`r2-0x6d40` = 40², `r2-0x6d38`).
Every update it keeps 0.8 of itself (`r2-0x6de0`), components under 0.01
stop (`r2-0x6ea8`), and upward speed falls at 100/s (`r2-28000`). While
HIT1, HIT2 or DEATH plays the monster skips its AI and only slides
(`FUN_8005a3b4`). Implemented in `monsters.rs` (`react`, `settle_knock`).
`+0x23C` is the floor step (half the step table: 1.5 for the small types,
3 and up for the rest), so "big" (above 2) is every type but sco, rat, sna,
spi, mag, wol, dog, aci and han; the same test gives big monsters' strong
blows the hero-knocking `0x10` (`docs/combat.md`), makes a monster a low
target for the hero at 2 or less, and decides whether it dissolves when it
dies (below).

## Deaths

The killing blow (`FUN_8004e660`, hit points at or below 0) plays the
death sound, sets the monster's state `+0xB4` to 8 and `+0x1FE` to the
attacker, frees its generator slot (`FUN_8004f240`), counts the kill, and
unless the attacker was −2:

- starts a timed texture effect on the body (`+0x1E4`, `FUN_80090a00(0.5,
  fx, texture, end, 0)`: counter −0.5, step 0.5, end 10): the golem (`0x1D`)
  hit plainly (`kind & 0xF` = 0) DEATHGOLEM with end 15; a tree (`0xB`) or
  knight (`5`) hit plainly DEATHALT; any other monster with a floor step
  (`+0x23C`) above 2 the texture for the blow's element (`0x80289198[kind &
  0xF]`); small monsters none;
- sets the body's draw priority to 999 (`FUN_800ba7c0`: object `+0x6A`,
  added to its sort key in `FUN_800c67a0`), so it draws last;
- spawns the kill's effect model (`FUN_80093e08`, below).

**The dying state** (state 8 in `FUN_8004cfe0`): the reaction and the move
run as usual (`FUN_8004db94`, `FUN_800445cc`: the killing blow's push, the
slide), the request is DEATH (`0x20`, priority 999) and the chooser
(`FUN_800ab110`) plays it — a body without DEATH gets the clip in its
action map's HIT2 slot (`+0x1BC`), its knock-down, and without that READY's.
The effect steps (`FUN_80090a48`: counter += 0.5 × fields / 2; at the end
it stops). While the action is HIT1, HIT2 or DEATH and the effect runs the
body stays; otherwise it's freed (`FUN_8004ef4c`). So a monster with a
death texture lasts 20 ticks after the kill, showing frame (int) counter:
frames 0–9 two ticks each; a small monster is gone at once.

**Drawing it** (`FUN_80090aec`): the texture override on the whole object
tree (`FUN_800ba85c`: `+0x5C` mode, `+0x58` texture): mode −4 with texture
first + (int) counter (−3 for the CHROMESILVER/CHROMEGOLD textures). The
draw (`FUN_800c3d60`) keeps each submesh's own texture and sets `0x8000000`,
which binds the override as a second texture stage (`FUN_800c6040`, in
`FUN_800c5894`). When the effect ends the mode goes back to −1.

**The textures** (set up with the effect list by `FUN_800972dc`):
texture-modifier records (`FUN_80010ab0`
looks one up by name in a bank's `ANIM.PS2` modifiers, 0x58-byte records,
and returns `+0x48`, the first frame's binding), ten frames each:

| index | name | frames (`WEAPONS`) |
| --- | --- | --- |
| 0 | DEATHBLOOD (DEATHMAGIC when `r13-0x72cc` is set; not traced) | DTH_BLOOD00–09 (DTH_YMAGIC) |
| 1 | DEATHFIRE | DTH_FIRE00–09 |
| 2 | DEATHELEC | DTH_PLASMA00–09 |
| 3 | DEATHLIGHT | DTH_LIGHT00–09 |
| 4 | DEATHACID | DTH_ACID00–09 |

DEATHALT comes from the tree's bank (`0x80250d34 + 4 × type`), else the
knight's (`MONSTERS/TRE` and `KNI` both have one); DEATHGOLEM from the
golem's. Each set runs from nearly white and solid to its colour (blood
red; fire yellow → orange → red; green; cyan; gold) and fully clear.

**The kill effect** (`FUN_80093e08(step, pos, parent, kind, 1, type)`;
the same call with 0 on every blow that doesn't kill): the effect by
element from `0x801225b0` (hits `0x8012259c`), knights and the golem
`0x80122588` (`0x80122574`), trees and acid blobs `0x801225d8`
(`0x801225c4`), indexing the effect list (`0x801218e0`, 0x28-byte entries:
name, depth bias, transparency; models from `WEAPONS`):

| element | hit | kill | knight/golem kill |
| --- | --- | --- | --- |
| 0 | BLOODFX1 | BLOODFX2 | HITDIE (hit: HITCOL) |
| 1 fire | FIREHIT | FIREDIE | FIREDIE |
| 2 electric | HITCOL | ELECDIE | ELECDIE |
| 3 light | HITCOL | LIGHTDIE | LIGHTDIE |
| 4 acid | HITCOL | ACIDDIE | ACIDDIE |

(BLOODHIT/BLOODDIE, list entries 4 and 5, are swapped for entries 6/7 and
8/9 in turn: BLOODFX1, BLOODFX2. Trees and acid blobs use entries 84/85 for
plain blows, which the list doesn't fill: nothing.) Scale: 0.5 × the step
(knights, the golem, or with `r13-0x72cc` set: 1); transparency 96
(`FUN_800ba9b0`: object `+0x53` = 255 − 96); depth bias −128 × scale
(`FUN_800baa74`, `+0x68`).

The effect goes where the blow landed for big monsters (step ≥ 4,
`r2-0x6dd8`), else at the monster's `+0x44` point.

BLOODFX1/2 are three particle systems each (atree node kind 4,
`docs/animation-format.md`): `BLOOD_SM` drops and `C_BLOOD` splats, 12–25
particles a second for 0.3 s (the records' first time; the second, 2 s, is
when the system is gone), living 0.3–0.8 s and falling.

**In this rewrite** (`deaths.rs`, the dying branch of `monsters.rs`, the
hit effects in `damage.rs`, `EffectAt` in `effects.rs`, bursts in
`particles.rs`): all of the above for regular monsters (the golem is a
critter here). The second stage is decoded ([rendering.md](rendering.md),
"Texture overrides"): the frame, sampled with the body's own coordinates,
takes the place of the body's colour — lighting × frame × 2 (frame 0 is
~(180, 205, 170): near white) — and its alpha shows where the body's own
is above 2/255. Stand-ins: `+0x44` is taken as the monster's centre; an
effect's scale scales its particles' sizes and speeds, and the node's
vector is taken as the spray direction; the depth bias isn't applied; the
DEATHMAGIC switch isn't traced.


## Suicide runners (AI `0x12`, `FUN_8004aa88`)

Placed grunts (and a few knights, lizards, a troll) at level 6 with AI 18
— on 40 levels, 13 on levelA1 and 31 on levelH1 (`cargo run -p
gdl-formats --example placements -- <game>/Gauntlet/LEVELS 0x12`). Their
model is the `S` variant (`GRUS`: READY, READYTOWALK, RUN only — a grunt
with a powder keg). Stats use tier 1 (`stat_tier`).

Each frame the heading is the angle to the target player (its `+0x44`,
or `+0x9E4` in player states ≥ 3). State `+0x316`:

- **0**: a target within the awareness (`+0x27C` ≤ `+0x300`) → state 1,
  timer `+0x31C` = 60 fields, run time `+0x320` = 0. It only turns.
- **1**: with a target and not yet in RUN (4): timer −= fields; below 1 it
  asks for READYTOWALK (9). The animation plays READYTOWALK out, then WALK
  — RUN when the model has no WALK (`FUN_800ab110` case 9). Once in RUN:
  state 2 and `S_SUICIDE_YELL` (`FUN_8009d5a4`: common sound `0x37`).
- **2**: run time += fields. No target, or not aware → AI 5/6 (by slot
  parity). Blocked by a monster (`+0x1FE` 2: the mover's block kind — 1
  wall, 2 monster, 3 item) with the bump wait `+0x358` running, it counts
  it down itself. While `+0x358` > 0 the heading gets the next offset of
  `0x8011b9c0` (±5°, ±10° … ±40°; `+0x30C` counts, at most 16) and the
  wait is zeroed; then, unless out of offsets and within 6° (`r2-0x6e28`)
  of the heading when the bumping began (`+0x254`), `FUN_8004cc84(1.5,
  heading)`: RUN (1.5 ≥ `r2-0x6d68` 1.25) at 1.5 × the speed.
- After the move: an item bump zeroes the wait; **run time ≥ 240 fields
  (`r2-0x6db8`) or a player bumped (`+0x284` ≥ 0) → it blows up**:
  `FUN_8004e660(999, monster, −2, kind 1, …)` (fire; −2: nobody, so no die
  effect, no sound, no experience). Else a bump that just began stores
  the heading in `+0x254` and restarts the offsets.

**The explosion** (`FUN_8004e660`'s kill path, for AI 18 however it died:
`FUN_800927f8(50 × level +0xBC, monster +0x54)` and `S_SUICIDE_BOMB`,
`FUN_8009d300`: sound `0x38`; a player's kill first stops the yell,
`FUN_8009d580`). Effects are the effect table's (`0x801218e0`, 0x28-byte
entries: name, `+0x20` depth bias, `+0x24` transparency):

- Realms 7 (G) and 11 (K): POISONEXP1 (`0x19`) as an area effect (flags
  `0x2B`: players, items, monsters), kind `0x800`, blast 7.5
  (`r2-0x5678`), then POISONEXP2 (`0x1A`) held 2 s (`+0x60`,
  `r2-0x5674`), then POISONEXP3 (`0x1B`) with no blast (the stage chain in
  `FUN_80094418`: `+0xAC`/`+0xAE`, flag `0x4000`, `FUN_80096d78` swaps the
  model in place); drawn 2.5 × 1 × 2.5 and 1 lower.
- Elsewhere: EXPLOSION (`0x16`, flags `0x29`: players and monsters, not
  items), kind `0x421`, blast 6 (`r2-0x566c`), scale 1; EXPRING (`0x1C`)
  at 1.2 with a light of radius 20 (`0x80121868` colour).
- Both: SUICIDEEXP (runtime entry `0x50`, depth bias −512), which
  `FUN_800972dc` takes from the first monster slot (not the special
  variants') whose bank has it.

The blast is the effects' area mode: it grows over the effect's life and
hits each target once (players through their guard).

**In this rewrite** (`monsters.rs` `suicide_ai` / `blow_up`, the
`ExplosionAt` handling in `effects.rs`): all of the above. Stand-ins: the
explosions' lights aren't cast; the blast hits each target once per stage
(the game spares monsters 3 s and heroes 0.25 s between hits). Heroes
take the blast with its own kind (`effects::blast_on_hero`, from the
effects update's area branch: under 5 damage it loses kinds `0x170` and
gains `0x1000000`; pushed by 0.25 × the way out from the centre, `r2-0x5578`),
so a fireball (`0x421`) knocks them down and the gas (`0x800`) only
makes them flinch; SUICIDEEXP's
`+0xB4` (0.5) isn't used. A running runner that loses its player walks
unaware for the frame (AI 5/6), and a charging one is the level's leader
that other monsters run from (AI `0x18`, "The frame update"). Test: `GDL_THROWER=15,0x12,6` on levelA1 with
`GDL_SHOT_CLOCK=ticks GDL_LOOK_AT="0,0,-9,18,90" GDL_SHOT_AT=96
GDL_SHOTS=12 GDL_SHOT_EVERY=3` films one running at the hero and going off.

## Death (type `0x1E`, AI 3)

Death is a monster record like the others (`0x802515e8` + slot ×
`0x394`), with its own branches in the target picker, the bump, the blow
routine and the dying frame. Confirmed from the decompile, with the
constants read from the DOL; the drain's sign trick also in the machine
code (`fneg` at `0x8004617c`). Level tuning scales as for any monster
(`+0xB0` speed, `+0xB4` awareness, `+0xBC` damage), except hit points.

**Where it comes from.** A level loads Death into a free enemy slot
unless it has a boss (`FUN_80057738`, above); it comes out of generators
naming it, placed ENEMYINFO items and containers holding it
([items.md](items.md), [mechanics.md](mechanics.md)). A placed one shows
the statue `DEATHSTATUE1`/`DEATHSTATUE2` (`MONSTERS/DEATH`; the item
update's class-4 case, `FUN_800606e8`) until its spot comes on screen
(`FUN_80060100`, "Placed monsters"); its tier is 2 when the placement's
count is non-zero, and it starts with a 30-field freeze (`+0x20C`), a
one-contact delay (`+0x2D4` = 1) and awareness 1000 (`r2-0x6708`).

**Stats** (the type tables, index 30): radius 1.5, centre height 3, step
3, ground 0.125 per field (the grunt's 0.1), damage 1 × level `+0xBC`
(not cut by the thirds), hit points **100** — not scaled by the level
(`FUN_8004fd9c` skips `+0xAC` for Death) nor by the tier (types from 28
on get the table value). Its strength `+0x206` is its tier (capped to 2
above 3; 1 for AI `0x12`), not the hit-point thirds: **tier 2 drains
experience**, any other tier health. The model is `DEATH1` / `DEATH2`
(`"%s%d"` by tier; `DEATH3` doesn't exist, so a tier-3 Death draws
`DEATH3L1`, also missing — not checked in play). Both atrees have only
READY (0 frames) and START (20), so every action it asks for plays READY's
animation (the chooser's fallback).

**Choosing a target** (`FUN_80051660`): as for other monsters — the "it"
hero (`r13-0x6FD4`, set by the tag monster type `0x1F`) when that hero is
in play and not invisible, else the nearest hero within awareness
(`+0x300`) with the crowd penalty — except that **a hero with the halo
(armour `0x80000`) is never Death's target**, not even as "it". Instead
the nearest haloed hero within awareness goes into `+0x328` (−1 when the
search starts over, every 8th frame or with no target).

**Moving** (AI 3, `FUN_80047350`, from the AI switch `FUN_800467bc`):

- With a target and aware (`+0x2DE`): AI 0 for the frame — the chase that
  tries nine offsets for a free heading (`FUN_80046abc`).
- Else with a haloed hero in `+0x328`: **it runs away** — heading = the
  angle to that hero (`FUN_8002c780`, from Death to the hero; the "it"
  decoy's position `+0x9E4` when the hero's `+0xA1C` is 3 or more) + π,
  wrapped to −π…π; `FUN_8004cc84(1.0, heading)` asks for WALK at its
  speed; turned by `FUN_8004cb20`; the velocity × 0.9 (`r2-0x6E10`); then
  the monster mover (`FUN_800445cc`). A blocked frame (`+0x358` > 0)
  nudges the next heading by the table at `0x8011B9C0` (±5°, ±10°, ±15°,
  ±20° in turn, index `+0x324`, up to 8), as the monsters fleeing a
  runner do.
- Else: AI 5/6 by slot parity (the unaware walk).

**The grab and the drain** (`FUN_800460a8`, the bump routine's Death
branch, run when the mover's bump test finds a hero — `+0x284` — in
Death's way; every frame they touch). Nothing happens while Death is
frozen (`+0x20C` ≥ 1) or the hero has the halo. Else, if the contact
delay `+0x2D4` is above 0 it counts down by 1; otherwise:

1. ATTACK1 is asked for (`FUN_800ad5ec(Death, 0xC)`; READY's clip plays).
2. The timer `+0x208` loses the frame's fields; at 0 or below it gains 3
   and a drain lands — **one drain per 3 fields**: 20 a second, two in
   every three frames at 30 fps.
3. The drain: a tier-2 Death takes experience (`FUN_80076144(hero, −1,
   −2)`: 1 × `(int)(0.01 × step)` experience, `step` = (level − 1) × 60 +
   1000 below level 61, else 4600 — a hundred drains are about one
   level; a level-99 hero loses nothing; losing enough drops the level,
   `FUN_800763d4`). Any other Death hurts: `FUN_80078560(−damage, hero,
   1, 0x1000, 0)`. The **negative** blow is the armour-piercing form: the
   resistance routine (`FUN_8002f58c`) takes a negative blow as its size
   without subtracting armour (`dVar5 = −dVar6`), gold armour (`0x100000`,
   a blow ≤ 1) and invulnerability still stop it, and kind `0x1000` has
   no element. Kind `0x1000` also arms the hero's `+0x898` for 1/15 s
   (`r2-0x5DF0`; poison's `0x800` arms it for 1 s): while it runs and
   the hero's intent is to stand, the hero plays STUN2 (`0x7A`, intent
   `0x20` from reaction class 100) standing still. **The hero isn't held**:
   any other intent (moving, attacking) acts normally, and walking out of
   Death's reach ends the contact.
4. The hint: `0x80` DEATHDRAINEXP ("DEATH DRAINS EXPERIENCE",
   `S_DEATHDRAINXP`) for tier 2, else `0x82` DEATHDRAINHEALTH ("DEATH
   LEAVES AFTER DRAINING 100 HEALTH", `S_DEATHDRAINS`) — raised on every
   drain; how often it shows is the hint system's (mode 3). `r13-0x7314` = 1; hits landed `+0x2CE` + 1.
5. If the hero lived (the damage routine returns 1 only when the hero
   died; the experience drain always 0), the drain effect goes on Death's
   model once (`+0x1E0`: `FUN_800911c4(model, strength, 0)` — runtime
   effect `0x60` `DEATH_EXP` for tier 2, else `0x5F` `DEATH_ARC`, both in
   `MONSTERS/DEATH`); it's freed (`FUN_80096fc8`) on a frame Death touches
   no hero. If the hero died, Death's hit points go to 0.
6. **Death pays for it**: its hit points lose its damage. Below 0 it has
   drunk its fill and **leaves**: `+0x320` = 1, `S_DEATHLAUGH` (`0x68`) at
   Death, hit points 0, state 8 (dying), the killer `+0x1FE` = the hero,
   unlinked from its generator (`FUN_8004f240`). So with damage 1 a
   health Death drains 100 health over 100 drains (5 s of contact) and
   leaves — the hint's "100 HEALTH" — and an experience Death takes about
   a level ("DEATH LEAVES AFTER DRAINING 1 LEVEL"). Otherwise
   `S_DEATHSUCK` (`0x66`) loops at Death (`FUN_800a045c`) and
   `r13-0x73E8` = 1; the frame update stops `S_DEATHSUCK`
   (`FUN_800a0438`) on a frame no Death drained.

**Blows on Death** (`FUN_8004e660`, the blow routine's Death branch; for
every blow — melee, missiles, blasts):

- Dying or gone (state 7/8): nothing.
- Dormant (state 6, the statue form — how a Death gets there isn't
  traced; placed Deaths don't): `S_WEAPONHITWOOD` (`0x3C`), the contact
  counter `+0x2D4` − 1, and at 0 it wakes: state 1, rebuilt
  (`FUN_80050580`), shown, `S_DEATHSHATTER` (`0x67`).
- **Magic** (kind `0x200`): hit points to 0 — killed outright. A hero
  above level 75 is healed by the hit points it had × (0.2 + 0.032 ×
  (level − 75)) (`r2-0x6D18`, `r2-0x6D10`; `FUN_80078474`, up to the
  hero's maximum).
- **Anything else takes exactly 1** hit point (`r2-0x6F10`), whatever its
  damage. From a hero without the halo it also raises hint 0 USEMAGIC
  ("USE MAGIC TO KILL DEATH", `S_USEMAGIC`) for that hero; from a haloed
  hero the point goes to the hero — +1 health (`+0x1EB4`, uncapped here),
  or for a tier-2 Death +1 × `(int)(0.01 × step)` experience
  (`FUN_80076144(hero, 1, −2)`).
- At 0 hit points: `S_DEATHDIE` (`0x65`, when the caller asks for
  sounds), state 8, the killer `+0x1FE`, unlinked; a kill counted for
  the hero (`+0xC10` + its weapon type `+0x0C` × `0x1C`). Returns 1.

So with 100 hit points a Death takes 100 non-magic blows; one magic blow
kills it.

**The halo's drain** (the player update `FUN_80080d3c`): when the hero
has the halo and this tick's target search (`FUN_800864b0`, the attacks'
search, out to its range) gives a Death less than 90° (`r2-0x5B78`) off
the hero's heading, every tick: the intent becomes 1 (stand — the hero
stops acting on the controls), `FUN_8004e660(1.0, Death, hero, 0, 0, 0,
1)` (the blow above: 1 hit point to the hero), `S_HALO` (`0x52`,
`FUN_8009e990`) on the first drain of this halo (latch `+0x95E`, cleared
when the halo ends), `S_DEATHDIE` at Death while it isn't playing,
`+0x128` |= 1 (|= 2 for a tier-2 Death), and `+0x95C` = 2, which turns
the intent into `0x1B` → DEATHGRABS (`0x1D`; then DEATHGRAB, DEATHGRABR
through the action machine). `+0x128` makes the stats routine put the
drain effect on the hero (`FUN_800911c4(hero model, 1 or 2, 0x10)`:
`DEATH_ARC` / `DEATH_EXP`) and stop `S_DEATHSUCK` when it clears. A
haloed hero thus empties a Death in 100 ticks (3.3 s), gaining 100
health or experience.

**Leaving and dying** (the frame update's state-8 branch for Death): no
death effect or blood — each frame Death rises 10 units a second
(`r2-0x6F08` × the frame's seconds) and its transparency (`+0x388`)
grows by 4 per field (`FUN_800ba9b0(model, t, 1)`) until it reaches 255 —
about 64 fields (1.07 s) from 0. Then, if it left after draining (`+0x320`), the hint to
the hero it drained (`+0x284`): `0x81` DEATHDIEEXP ("DEATH LEAVES AFTER
DRAINING 1 LEVEL", `S_DIESAFTERXP`) for tier 2, else `0x83`
DEATHDIEHEALTH ("DEATH DRAINS HEALTH", `S_DIESAFTER`) — the hint texts as
the game's table pairs them. The slot is freed (`FUN_8004ef4c`).

Not traced: the tier of a Death from a generator or container beyond the
usual tier rules; how a Death becomes dormant; `+0xC0` (1 for Death only,
0 for every other type).

**In this rewrite** (`monsters.rs`, `damage.rs`, `player.rs`): every level
without a boss loads Death; placed Deaths appear (tier 2 with the count
set, awareness 1000, a 30-field freeze, one contact's delay) with their
100 unscaled hit points; the target pick skips haloed heroes and notes
the nearest; AI 3 chases, runs from a haloed hero (× 0.9, the nudges
while blocked) or walks unaware; the drain on contact (20 a second:
health through `Player::take_blow` as the negative kind-`0x1000` blow, or
experience with `PlayerState::lose_experience`; `S_DEATHSUCK` following
the Death draining, stopped on a tick none drains; the drain effect; its
hint) and Death paying for it, leaving full with `S_DEATHLAUGH` (panned
at its feet, [audio-format.md](audio-format.md)); blows taking 1, magic killing it (the level-75 heal), the
halo's share and the use-magic hint; rising and fading as it goes, then
the leave hint; the halo's drain (the hero stands in DEATHGRABS →
DEATHGRAB → DEATHGRABR, `S_HALO` once, `S_DEATHDIE` going on at it, the
drain effect on the hero). Death moves by its AI whatever it plays (its
atrees have READY and START only). Stand-ins: the drain effects play
once per contact instead of holding until it ends; a hero killed by a
drain doesn't empty the Death; dormant Deaths (the statue) and a Death
with the suicide AI aren't. The STUN2 while standing drained is done
([combat.md](combat.md), "The stuns").
