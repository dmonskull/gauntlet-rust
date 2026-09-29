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

## Displacement per tick

Heading = stick angle + camera yaw (stick up moves away from the camera).
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

- Collision and floors (`WORLDS.PS2` collision, separate branch).
- Level player starts and the game's camera (`WDATA` `CAMS`/`BCAM`).
- Attacks, strafing, the shove, and blends between actions.
