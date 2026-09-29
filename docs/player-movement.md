# Player movement

Implemented in [`locomotion.rs`](../crates/gdl-game/src/locomotion.rs)
(pure, unit-tested) and [`player.rs`](../crates/gdl-game/src/player.rs)
(input, 30 Hz fixed tick, render interpolation, follow camera).

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
magnitude 0–1. `FUN_80088170` classifies the stick: magnitude 0 → idle,
≤ 0.75 → walk, above → run; the player update then requests READY, WALK1
or RUN1 (indices 0, 0x11, 0x13 in the action table at `0x80126430`).

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
| WALK*/RUN* | another gait | it | at end (now for attacks, hits) |

WALK1/2 and RUN1/2 are non-looping half strides (10–12 frames), so a walk
alternates the two clips. The blend time is 0 except when going back to
READY from most actions: 0.0667 s (`r2-0x4dcc`); `FUN_8000f534` then
lerps each bone's angles and offsets from the snapshot of the old pose
(`FUN_8000f788`/`FUN_8000f74c`) while the weight falls to 0. We slerp.
Standing still in READY for 600/1800 ticks switches to IDLE2/IDLE1 (not
implemented yet); the action's movement/turn factors (below) are set when
it starts, so a run's 1.3× applies from the first RUN1 frame.

`FUN_800ad42c` gives each action a category: 0 locomotion and misc, 1
defend/shove, 2 ATTSTART–ATTSLOW, 3 ATTQUICK1–3 and recoveries, 4 the
directional quick attacks, 5 ATT360, 6 steps/walks, 7 strafe attacks, 8 low
and power-A attacks, 9 throws, 10 fire, 11 the rest, 12 combos.

## Displacement per tick

Heading = stick angle + camera yaw (stick up moves away from the camera;
[camera.md](camera.md)). Players start at the level's entry-0 start
locator, facing its yaw, dropped onto the floor below.
Per tick:

```
d   = knockback × dt                    (knockback ×0.667 each tick)
d  += factor × dt × speed × magnitude × (sin heading, cos heading)
d.x, d.z clamped to ±1.5 × speed × dt
```

`factor` is set per action when it starts (`FUN_800ab898`): RUN1, RUN2,
SHIELD_RUN 1.3; strafe walks 0.667; SHOVE 1.5; WEBREACT 0.4; other
locomotion 1.0; most attacks 0, 0.25 or 0.5. The same function sets a turn
factor (`+0xA4C`), 1.0 for locomotion.

If knockback moves the player more than 0.05 units in a tick, stick input
more than 120° away from it is ignored.

## Facing

The body turns toward the stick heading at up to 5π rad/s × turn factor
(`r2-0x5ac0` = 15.708), shortest way round. Movement itself follows the
stick immediately; only the body lags.

## Not yet

- The player's own collision chain: movement goes through the generic
  actor mover ([collision.md](collision.md)) with a stand-in radius (1.0)
  and step (2.0) for now.
- Attacks, strafing, the shove, and blends between actions.
