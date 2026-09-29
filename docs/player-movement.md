# Player movement

Implemented in [`locomotion.rs`](../crates/gdl-game/src/locomotion.rs)
and [`actions.rs`](../crates/gdl-game/src/actions.rs) (pure, unit-tested)
and [`player.rs`](../crates/gdl-game/src/player.rs) (input, 30 Hz fixed
tick, render interpolation, follow camera). Attacks are in
[combat.md](combat.md).

The game updates each player once per tick in `FUN_80080d3c`; the tick is
`r13-0x7570` seconds (1/30). Constants below are `r2` doubles/floats read
from `main.dol`.

## Stats → speed

`FUN_8007f104` sets each player's four raw stats from `PDAT`
([chunk-files.md](chunk-files.md)): `min(start + (level − 1) × 5, max) +
bonus`, capped at 999. Player `+0xF4..+0x100` hold strength
(`PDAT+0x28`), armour (`+0x38`), magic (`+0x40`) and speed (`+0x30`).

`FUN_8007c4f0` derives the working values each tick as
`min + 0.001 × raw × (max − min)`, clamped, with the ranges in `r13`
data (`-0x7d84..`):

| raw stat | player field | range |
| --- | --- | --- |
| strength | `+0x104` | 5 – 20 |
| armour | `+0x108` | 0 – 5 |
| magic | `+0x10C` | 8 – 32 |
| speed | `+0x110` | 5 – 12.5 units/s |

Speed power-ups (effect type 7) add straight to `+0x110` before the clamp.

## Stick → intent → action

`FUN_80032b94` turns each pad into polar form: angle (`atan2(x, y)`) and
magnitude 0–1. `FUN_80088170` classifies the controls into an intent
([combat.md](combat.md)); with no button that matters, the stick decides:
magnitude 0 → idle, ≤ 0.75 → walk, above → run, and the player update
requests READY, WALK1 or RUN1 (indices 0, 0x11, 0x13 in the action table
at `0x80126430`). Holding strafe (R) instead requests the strafe steps.

## Action chaining

`FUN_800ab898` picks the next action from the playing one (`+0x208`) and
the requested one (`+0x20C`), plus how it takes over, which
`FUN_8000eb70` applies: 2 = now (if different), 0 = when the playing clip
has finished (if different), 1 = when it has finished. For locomotion:

| playing | requested | next | when |
| --- | --- | --- | --- |
| READY | anything | it | now |
| WALK1 | WALK1 | WALK2 | at end |
| WALK2 | WALK1 | WALK1 | at end |
| RUN1 | RUN1 | RUN2 | at end |
| RUN2 | RUN1 | RUN1 | at end |
| WALK*/RUN* | another gait | it | at end (now for anything past index 0x1A or of another category: attacks, hits; from RUN2 only HITREACT or another category) |
| STRAFE_WLK?1 | its group's first step | that group's second step | at end (now for other categories) |
| STRAFE_WLK?2 | its group's first step | it | at end |

The strafe groups are front/back (F1 → F2, B1 → B2, and F1 asked for B1
gives B2) and left/right. The attack actions are in
[combat.md](combat.md); `actions.rs` ports the whole state machine for
the actions reachable so far.

WALK1/2 and RUN1/2 are non-looping half strides (10–12 frames), so a walk
alternates the two clips. The blend time is 0 except when going back to
READY: 0.0667 s (`r2-0x4dcc`), except after `0x56`–`0x93`, HITREACT,
`0x81`/`0x82` and the archer's ATTQUICK2R; `FUN_8000f534` then
lerps each bone's angles and offsets from the snapshot of the old pose
(`FUN_8000f788`/`FUN_8000f74c`) while the weight falls to 0. We slerp.
Standing still in READY for 600/1800 ticks switches to IDLE2/IDLE1 (not
implemented yet). The movement/turn factors (below) are set by the state
machine each tick for the action playing when it runs (`iVar12`, before any
switch); the update computes the step before calling it and turns after,
so a new action's movement factor applies from its second tick and its
turn factor from its first.

`FUN_800ad42c` gives each action a category: 0 locomotion and misc, 1
defend/shove, 2 ATTSTART–ATTSLOW, 3 ATTQUICK1–3 and recoveries, 4 the
directional quick attacks, 5 ATT360, 6 steps/walks, 7 strafe attacks, 8 low
and power-A attacks, 9 throws, 10 fire, 11 the rest, 12 combos.

## Displacement per tick

Heading = stick angle + camera yaw (stick up moves away from the camera;
[camera.md](camera.md)). Players start at the level's entry-0 start
locator, facing its yaw, dropped onto the floor below. Each tick's move
goes through the player's own collision (`LevelCollision::move_player`,
[collision.md](collision.md) "Moving a player").
Per tick:

```
d   = knockback × dt                    (knockback ×0.667 each tick)
d  += factor × dt × speed × magnitude × (sin heading, cos heading)
d.x, d.z clamped to ±1.5 × speed × dt
```

`factor` (`+0xA48`) is set per action by `FUN_800ab898`: RUN1, RUN2,
SHIELD_RUN 1.3; strafe walks 0.667; SHOVE 1.5; WEBREACT 0.4; other
locomotion 1.0; attacks per the table in [combat.md](combat.md). The same
function sets a turn factor (`+0xA4C`), 1.0 for locomotion, and a stick
scale (`+0xA50`), 0 while defending or casting.

If knockback moves the player more than 0.05 units in a tick, stick input
more than 120° away from it is ignored.

## Facing

The body turns toward a desired facing at up to 5π rad/s × turn factor
(`r2-0x5ac0` = 15.708), shortest way round: the stick heading, or the
current facing (no turn) with the stick released or while strafing or
defending; attacking in place aims it at the target
([combat.md](combat.md)). Movement itself follows the stick immediately;
only the body lags.

## Not yet

- Falling out of a level returns the hero to the start (the game kills
  the player; lives aren't implemented).
- The shove, hit reactions, idles, and blends between actions other than
  into READY.
