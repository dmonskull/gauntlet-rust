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

## Footsteps

There is no step timer or distance: a foot comes down as a stepping
action ends. Confirmed (`FUN_800ab898`, the action machine, and
`FUN_80080d3c`, the player update, which calls the machine at
`0x800835A8` and plays the step later in the same tick, `0x80084EEC`):

1. **The trigger.** Whenever the take-over reports anything
   (`FUN_80011134` returns `FUN_8000eb70`'s bits: the next action
   started, or the clip is at its end with its loop flag on or the same
   action asked for), the machine switches on the action that *was*
   playing (`+0x208`, read before the switch; jump table `0x80126B4C`)
   and sets a step bit in `+0x962` (u16): bit 1 for SHOVE (`0x08`),
   WALK1, RUN1 and SHIELD_RUN (`0x16`), bit 2 for WALK2 and RUN2. WALK1/2
   and RUN1/2 are non-looping half strides, so walking or running steps at
   the end of each, alternating the feet — also when an attack or a hit
   cuts one short, or READY follows. SHOVE and SHIELD_RUN loop: the
   machine turns the controller's loop flag (`+0xB4`) on for them (SHOVE
   as it plays; SHIELD_RUN where WALK1–RUN2 become SHIELD_RUN with a
   shield on, [powers.md](powers.md)), and each loop coming round steps
   with the first foot. Strafe steps, pivots, READY and the rest are
   silent.
2. **The step.** If either bit is set — in the movement block, so only
   while `+0x834` < 4 (the legendary weapon's throw) and the hero isn't
   attached (`+0x964 & 0x20`: a critter's grab, `FUN_800746c8`, or
   another player carrying it, `FUN_800747ac`) — the floor is picked,
   `FUN_8009e804(player, bit 2 set, floor)` plays unless the hero
   levitates (`+0x124 & 1`), and both bits are cleared.
3. **The floor**, in this order:
   - 4 **water** if `+0x8CC` > `+0x8B4`: `+0x8CC` is the water surface
     under the hero — the nearest hit of the floor check's first probe on
     a node kept apart (flag `0x200`, `r13-0x6fc0`/`-0x6fc4`,
     [collision.md](collision.md)), or the floor followed (`+0x8B4`) when
     there is none. The blob shadow (`SHADOWL1`, `+0x6C8`) is set by the
     same test: 0.1 above the floor, or 0.1 above the water with the
     texture `SPLASH` (`r13-0x6f10`) in place of its own (`FUN_800ba85c`
     −2 rather than −1). Not done here.
   - 3 **metal** with invulnerability (armour `+0x120 & 0x10000`): the
     chrome clanks whatever the floor.
   - 2 **stairs** if the floor node's flags (`+0x8C0`, the standing
     node's `+0x10`, copied with it by `FUN_8008764c`) have `0x8`.
   - 0 **rock** otherwise. Column 1 (wood) is never picked.
4. **The sound.** `FUN_8009e804` reads the table at `0x801231A4` (2 rows
   by foot × 5 floors, u32 common-bank ids, `0x0000` + the call):
   `S_STEPROCK1`, `S_STEPWOOD1`, `S_STEPSTAIR1`, `S_STEPMET1`,
   `S_STEPWATER1`, then the same with 2. It plays it with
   `FUN_80015828(id, player +0x44, 0x7F, 0x7D)`: at the hero's feet,
   volume 127, voice priority `0x7D`. That player scales the volume by
   `1.4 − d / 50` clamped to 0–1 (`r2-0x7db0`, `-0x7da8`), `d` being the
   distance to the nearest hero in play (`FUN_80063658`) — so a hero's
   own steps are always at full volume — and pans by the offset from the
   camera's focus (`0x8023F1BC`), normalised, along the camera's right
   (`0x8023F094`), times `min(|offset| / 20, 1)`, about the centre 127.5.

The levels' stairs (flag `0x8`) are the `…STAIRS…`/`…STEP…` nodes (46 in
levelL1, none in most levels). The water nodes (flag `0x200`) are
`A4WATER_LINE131`–`133`, levelC1's `C1WATER…`, `C4WATER`/`C4WATER1`,
`G3WATER01`, `I4WATER_POND`, `I4WATER_RIVER01` and `T1WATER`; a few
`0x202` nodes and some with a garbage flags word `0x67697274` (levelA2's
rock piles, levelG5's flame holders, the music books) are kept apart too.

Unused: `S_STEPWOOD1`/`2` (never picked) and the ice bank's
`S_STEPSNOW1`–`4` (no code or data names them). The heroes' table is the
only step table: the bosses' steps (`S_GEN%cSTEP1`/`2`, `S_GRG%cSTEP1`/`2`,
`S_YETISTEP`) are sounds of their moves' `SFXX` records
([critters.md](critters.md)), and other monsters have none in the catalog.

Here (`footsteps.rs`, after each player tick): the action a hero was
playing ends when `ActionState::action` changes, or when, the same action
playing, its clip's frame goes back (a loop coming round or the clip
played again); a stepping action's end plays the step by name
(`PlaySound`, at the call's volume, no pan). Water is
`LevelCollision::player_water` above `PlayerGround::floor` — probed at
the hero's feet after its move rather than at the move's destination
before the floor check; stairs are the standing node's
(`PlayerGround::node`) flag `0x8`; metal and levitation are the hero's
armour and special bits. Not modelled: the `+0x834` and attached checks
(nothing attaches a hero here). SHIELD_RUN steps only once the port
loops it as the game does (the shield swap's loop flag; its clip's own
loop bit is off).

## Not yet

- Falling out of a level returns the hero to the start (the game kills
  the player; lives aren't implemented).
- The shove, hit reactions, idles, and blends between actions other than
  into READY.
