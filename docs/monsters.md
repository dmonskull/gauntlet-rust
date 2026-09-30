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
swaps the model (`GEN_%s%d`, `GEN_SPECIAL%d`), and at 0 the generator is
freed and its monsters forget it. Experience: `docs/items.md`.

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
   `FUN_800467bc` clears the velocity and switches on the AI:
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
   - Others: 3 Death, 5/6 unaware, 8, 10, 12–31 special movers (thrower,
     bomber, suicide, ranged, fleeing…) — not traced.
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
- **AIs**: 7 and 2/4 are ported, and the throwing AIs `0x10`, `0x11`,
  `0x17`, `0x1A` ([projectiles.md](projectiles.md): facing, throwing,
  backing off, the throw pause); 0 uses 7's chase (its nine-angle search
  isn't); every other AI (suicide runners `0x12`, fireball casters
  `0x1C`/`0x1D`/`0x1F`, 3, 5/6, 14…) uses the chase too, and variants
  without a WALK or RUN animation stand still instead of gliding. The crowd
  penalty in target choice isn't applied (one player).
- **Unaware** monsters (AI 5/6) stand still.
- Zero level scales are read as 1 (above).
- Spot tests skip items (`FUN_8005ef98`); the placed-monster distance is
  from the player.
- Knockback, getting hit, dying, `+0x21C`, the leader monster
  (`r13-0x73b8`), `WALKTOREADY`/`READYTOWALK` transitions and the critter
  system aren't done.
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

**In this rewrite** (`deaths.rs`, the dying branch of `monsters.rs`,
`EffectAt` in `effects.rs`): all of the above for regular monsters (the
golem is a critter here). Stand-ins: the second stage's blend isn't
decoded — the frame multiplies the body's colour × 2 and its alpha, which
fits the textures (frame 0 is ~(180, 205, 170): white at × 2) — and it's
sampled with the body's own UVs; BLOODFX1/2 aren't drawn (their nodes are
the texture-animation kind, `FUN_80018304`, not decoded), nor any hit
effect; the depth bias isn't applied; the DEATHMAGIC switch isn't traced.

