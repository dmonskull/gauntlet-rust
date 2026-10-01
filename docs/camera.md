# The play camera

Implemented in [`camera_rig.rs`](../crates/gdl-game/src/camera_rig.rs)
(pure, unit-tested) and [`play_camera.rs`](../crates/gdl-game/src/play_camera.rs).
Addresses are in `main.dol`; `r2`/`r13` as in [INDEX.md](INDEX.md).

## Data

**Camera points** are the level's locators ([level-population.md](level-population.md))
of kinds 1–4 and 9, which the game's messages call "transmitters".
`FUN_80066258` copies them into a table at `0x802587e4` (0x28 bytes each,
at most 256): position, pitch (`+0x10`), yaw (`+0x14`), the locator's
`+1` byte and a state byte:

| kind | state | use |
| --- | --- | --- |
| 2 | 0 | ordinary point, picked by distance (≈100 per level) |
| 1 | 3 | an entry's starting camera, by the locator index |
| 3, 4 | 1 | pitch negated; the level-intro path |
| 9 | 2 | switched to by the trigger whose id is the locator index |

Kinds 1, 2 and 9 store `yaw + π`; the update subtracts π again, so the
camera faces the locator's own yaw (0 = +Z, π/2 = +X).

**Level camera record**: `WDATA/<realm>.WAD` chunk `CAMS`, 0x6C bytes,
chosen by the level's `LEVL` `+0x58` ([chunk-files.md](chunk-files.md),
`world_data.rs`). Read by `FUN_8002cf2c`, `FUN_80026ca4`, `FUN_8006ed50`:

| offset | field |
| --- | --- |
| `+0x00` | i16 mode (0 in every retail level) |
| `+0x08` | pitch limit with several players (radians) |
| `+0x0C`, `+0x18` | target box min, max |
| `+0x24` | u8: box is set; if 0, `FUN_80026ca4` derives it from the level bounds (`WORLDS.PS2` words 9–14) as min + (8, 0, 8), max + (−8, 4, −8) |
| `+0x2C`, `+0x30` | near and far distance |
| `+0x34` | i16, copied to `r13-0x73bc` (not traced) |

## Update (`FUN_8006df34`, each tick)

1. Focus: the centre of the box round the players' **top points**
   (`FUN_8006f8f0(…, 5)`: player `+0x54`, the feet + the class's `PDAT
   +0x50`, 4.4 for every class), clamped to the target box. It goes into a
   9-slot ring (`r13-0x7e30` = 9).
2. Point choice (`FUN_8006fcdc`), from the players' **feet** instead
   (`FUN_8006f8f0(…, 2)`: `+0x44`, clamped the same way): the nearest
   state-0 point other than the current one takes over when its squared
   distance is ≤ 0.4444 (`r2-0x6280`, (2/3)²) × the current point's. With
   no current point, the nearest wins.
3. Angles (`FUN_80070144`): target yaw = point yaw, target pitch = −point
   pitch (with several players, no shallower than −pitch limit). When the
   point changes the camera turns linearly over 50 ticks (`r13-0x7e14`),
   `30 × dt` steps per frame.
4. Target += mean(ring − target); distance likewise over a 9-slot ring of
   `FUN_8006ed50`'s distance — the record's near distance for one player,
   a fit of every player into view (between near and far) for several.
5. Direction = (sin yaw cos pitch, sin pitch, cos yaw cos pitch)
   (`FUN_800be0fc`/`FUN_800be070` rotating +Z); eye = target − direction ×
   distance.

Field of view (`+0x10C`, `r2-0x6234`) is 60°, horizontal at 4:3
(`FUN_800bbe60` converts with 0.75 × height scale): 46.8° vertical. We keep
the vertical angle, so widescreen shows more at the sides.

Player movement is relative to the camera's yaw (stick up = the way the
camera faces; [player-movement.md](player-movement.md)).

## What we do differently

- The camera ticks at 30 Hz and is interpolated between ticks for drawing.
- (Fixed: it had looked at the hero's feet, 4.4 lower than the game — the
  tower's start showed the wizard's pedestal cut off at the top.)
- At level start it snaps to the nearest point instead of turning from
  wherever it was.

## Several players (`FUN_8006ed50`)

The update (`FUN_8006df34`) picks the preferred distance (`+0xDC`): the
record's near × 0.8 (`r13-0x7e18`) for one player (never used: the fit
returns near), and for several the current camera point's byte (its
locator's `+1`) or, when that's 0, the record's far × 0.9375
(`r13-0x7e1c`). `FUN_8006ed50` then gives the distance that goes in the
9-slot ring: with near = far, near; with one player (`r13-0x7394`, the
count; `r13-0x706c` never set), near; with several, for each of the
camera's four view-side planes (`+0x40`, 4 × 16 bytes, inward normals)
and each player in state 1 or 4 without flag `0x20` (`+0x964`), the
distance that puts its top point (`+0x54`) and feet (`+0x44`) on the
plane's inner side looking at the focus along the view —
(focus · n − p · n) / (dir · n), `r2-0x62a8` = 1 — the largest of those
("needed"), then: when the players fit within near − 10 (`r13-0x7e28`)
and the preferred distance is past near, near; otherwise needed + 10, or
the preferred distance once needed is past preferred − 10, or needed + 4
(`r13-0x7e24`) once past preferred − 4. No far limit. With several
players the target pitch is no shallower than the record's pitch limit
(`+0x08`).

Here (`camera_rig.rs` `fit`): as above, the planes being the 4:3 view's
sides (60° across, 46.8° up) through the eye along the current yaw and
pitch — the game's `+0x40` planes aren't traced, so their margins (if
any) aren't known. The heroes framed are those alive (all of them when
none is).

## Not yet

- Starting (kind 1), intro path (3/4) and trigger (9) cameras.
- Camera modes other than 0. (Boss levels use the boss camera:
  [critters.md](critters.md) "Boss camera".)

## Level start

`FUN_80026ca4` sets up the opening shot for the entry's starting camera
(the kind-1 locator whose index is the entry, `r13-0x71e4`): the eye at the
locator, facing its yaw (+π twice, so its own yaw) and −pitch, looking at a
point as far along that direction as the players are from it; a countdown
of 91 video fields (`r13-0x733c` = `0x5B`) and the intro flag
(`r13-0x7340`). The director (`FUN_80023e84`) counts it down by the fields
elapsed; once fewer than 45 remain any button (`& 0x20000FF`) cuts it to
1. Then each update eye and target move 0.1 (`r2-0x7798`) of the way to
where the play camera wants them, until both are within 0.3
(`r2-0x77f8`), and play begins (players' `+0x91C` = 4). Ported in
`play_camera.rs` (`Intro`); the hero isn't held still during the shot.

## Trigger cuts

A trigger whose id matches a kind-9 camera point's index (the locator
loader `FUN_80066258` links them with `FUN_80066c7c`; the trigger keeps
the point in `+0xEE`) shows that point when it comes on with a player on
it (`FUN_8001be98(node, &point.pos, &point.rot, point byte, 0x1E, 0)`):

- `r13-0x774c` = 2 (a cut is on). While it is, the hero's damage routine
  refuses every blow (`FUN_80078560` checks it first) and triggers keep
  their touches; the pads are cleared and not read (`FUN_80032824` sets
  `r13-0x7500`, which the input read `FUN_80032b94` honours) until the cut
  ends (`FUN_80032788`).
- Its arguments (`FUN_8001be98(node, pos, rot, hold, delay, extra)`): the
  hold is 6 × the given byte, 40 fields for 0, or for ever (1,000,000)
  when negative, until `FUN_8001be80` ends it; `delay` fields pass before
  the view changes; after the hold, `extra` fields more (`r13-0x7740`,
  which the tower's scenes watch). The tower's scenes cut with their own
  holds and no delay.
- `r13-0x7748` = 30 fields: the delay before the view changes.
- `r13-0x7744` = 6 × the point's byte, or 40 fields when it's 0: how long
  the view holds (counted down by fields in `FUN_8001bc14`-ish, the cut
  update around `0x8001bd00`); it also holds while the node the trigger
  moved is moving (node flag `0x8000000`).
- The camera stands at the point, turned by −(its yaw + π) about Y, then
  its pitch about X — the same view as the level-start camera.
- Two black bars are drawn over the 512 × 384 screen: lines 0–80 and
  304–384 (`FUN_800b2dc4(…, 0, 0, 0x200, 0x50)` and `(…, 0, 0x130,
  0x200, 0x50)`).
- When it ends the camera goes back to following the players.

In this rewrite (`play_camera.rs`, `StartCut` sent by `mechanics.rs`): the
same delay, time, hold while moving and bars; the hero takes no damage
and his pads aren't read meanwhile (they were until now); the way back glides like the level start (the game's own
return isn't traced). On levelA1 `GDL_WARP="-33.8,0,26.6"` stands the
hero on the switch of `A1ELEV666`, which cuts to the elevator rising out
of the water.

### Scene camera points

In the tower (realm 13) the locator loader gives some kind-9 points to the
tower's scenes instead of triggers (`FUN_80066c7c`, by index): `0xC6` the
welcome's (`r13-0x7214`), `0xCA` the window (`r13-0x71f0`, shards),
`0xCC` the E realm's portal (`r13-0x71ec`), `0xC9` the rune place,
`0xCB` the thirteenth rune, `0xCD` H's fourth level (`0x802587a0`…a8);
`0xF0`–`0xFD`, `0xDC`–`0xE9` and `0xAA`–`0xB7` the wizard's lookouts, one
table each for the heroes' ranks, shards and runes (`0x802586F8` + kind ×
`0x38`, `FUN_80066aec`, falling back to a lower kind's).

## Shakes

`FUN_800277ec(amplitude, what, delay, fields, priority)` starts a shake
(`r13-0x7688..-0x7674`) unless one with a higher priority is still going;
`FUN_800275bc(eye, target)` applies it each frame: after `delay` fields,
for `fields` fields, the target (`what` 0), the eye (1) or both (2) move
round a circle of radius `amplitude` in X/Z, at angle 0.6632 (`r2-0x76f8`)
× the fields left. Triggers with flag 0x1000 shake the target by 0.1 for
180 fields (priority 100); the other callers use 0x5A/0x1E fields and
priorities 100/200 (not traced). `play_camera::Shake` does the same.
