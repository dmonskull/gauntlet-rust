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

1. Focus: the players' centre (`FUN_8006f8f0`), clamped to the target box.
   It goes into a 9-slot ring (`r13-0x7e30` = 9).
2. Point choice (`FUN_8006fcdc`): the nearest state-0 point other than the
   current one takes over when its squared distance is ≤ 0.4444 (`r2-0x6280`,
   (2/3)²) × the current point's. With no current point, the nearest wins.
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
- At level start it snaps to the nearest point instead of turning from
  wherever it was.

## Not yet

- Starting (kind 1), intro path (3/4) and trigger (9) cameras.
- Several players: the fit distance and pitch limit.
- Boss cameras (`BCAM`), camera modes other than 0.

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
