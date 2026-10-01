# Items the hero touches, and the hero's own state

Implemented in [`player_state.rs`](../crates/gdl-game/src/player_state.rs)
(health, gold, keys, potions, powerups, `DamagePlayer`),
[`items.rs`](../crates/gdl-game/src/items.rs) (touch test, pickups, doors,
locked chests, exits, transporters, item animation),
[`exits.rs`](../crates/gdl-game/src/exits.rs) (`ChangeLevelTo`),
[`hints.rs`](../crates/gdl-game/src/hints.rs) (the game's tutorial hints)
and [`status_hud.rs`](../crates/gdl-game/src/status_hud.rs) (overlay).
[`population.rs`](../crates/gdl-game/src/population.rs) tags each model
with its placement (`PlacementIndex`) and builds atree models as a node
hierarchy (`ItemRig`) so their actions can play. The class record fields
used here are parsed in [`pdata.rs`](../crates/gdl-formats/src/pdata.rs).

Record layouts for item types and placements are in
[level-population.md](level-population.md); this file covers what happens
to them at run time.

## The player record

Four records of `0x335C` bytes at `0x802754c0` (`DAT_802754c0`); the
fields found here (offsets from the record):

| offset | field | evidence |
| --- | --- | --- |
| `+0x08` | class index (WAR, VAL, WIZ, ARC, DWF, KNI, SOR, JES…) | indexes the per-class sound tables below |
| `+0xE8` | state: 1 alive, 4 leaving by an exit, 5 left, 8 dying | `FUN_8007692c` switch; `FUN_80078560` sets 8 |
| `+0x130` | 11 powerup slots × `0x10`: f32 time left, i32 subtype, f32 amount, u32 value bits | `FUN_8007ee10` |
| `+0x850`, `+0x854` | radius against items, half height | `FUN_80079ed8`: `PDAT+0x4C`, `0.5 × PDAT+0x48` |
| `+0x8AC` | exit / transporter the hero stands in this tick | `FUN_80086e44` |
| `+0x8B0` | powerup touched this tick | `FUN_8005d71c`, consumed by `FUN_8005de3c` |
| `+0x93C`, `+0x940`, `+0x944` | transporter cooldown, transport timer, destination | `FUN_80086ab8` |
| `+0x1EB4` | health (f32) | `FUN_80078474` (heal), `FUN_80078560` (damage) |
| `+0x1EB8` | keys | `FUN_8005de3c` case 2, door/chest touch |
| `+0x1EBC` | potion count; the potions' kinds at `+0x3300` (9 × i32) | `FUN_8005de3c` case 4; reset in `FUN_80079b34` |
| `+0x1EC0` | experience | `FUN_80079ed8` |
| `+0x1EC4` | gold, capped at 99,999 | `FUN_800765e0` |
| `+0x3324` | level | `FUN_80078474`, `FUN_800763d4` |

`FUN_80079b34` resets a hero: 500 health, level 1, no experience, 0 gold
(2,500 in one game mode), no keys or potions. Keys and potions are capped
by `r13-0x7254` / `r13-0x7258`, which the level loader (`FUN_80058074`)
sets to 9. `DAT_80282310[class]` points at the class's `PDAT` record
(`FUN_8007f104` reads the stats through it): `+0x48` height (5.0 for every
class), `+0x4C` radius (1.5), `+0x58` powerup time factor (1.0–1.3; WIZ and
SOR 1.3).

### Health

- **Maximum** (`FUN_80078474`): `min(100 × (level − 1) + 500, 9999)`.
- **Healing** (`FUN_80078474`, food): a positive amount at full health is
  refused (returns 0; the food stays); otherwise added and capped.
- **Damage** (`FUN_80078560(amount, player, kind, flags, direction)`):
  only while alive; armour (`FUN_8002f58c`) and, for hits above 1, the
  level record's difficulty factor (`+0xA4`) reduce it first; below 1.0
  health the hero dies (state 8, health 0). Crossing 150 and 50 on the
  way down makes the announcer speak (`FUN_8009f82c`: class name +
  `S_BADLY` / `S_LIFEFORCE`).
- **Low-health warning** (`FUN_80077f50`, for the lowest-health hero):
  at ≤ 200 health `S_WARN` plays every 120 fields, 60 below 100, 30 below
  25, louder as health falls (`FUN_8009e9d0`).
- **No drain over time.** Every write to the health float was checked
  (`FUN_8004e660`, `FUN_80074f0c`, `FUN_80076144`, `FUN_8007826c`,
  `FUN_80078474`, `FUN_80078560`, `FUN_80078de8`, `FUN_80079094`,
  `FUN_80079b34`, `FUN_80079ed8`, `FUN_8007a4f4`–`FUN_8007a738`): none
  lowers it by time. The only callers of the damage function are attacks,
  hazards, damage tiles and poisoned food.
- **Levelling** (`FUN_80076144` → `FUN_800763d4`): experience for level
  `L` is `(L − 1)(30L + 1000)` up to 60, then `(L − 60) × 4600 + 165200`.
  When a change of experience moves the level (`FUN_800763d4` returns 1
  for a rise, −1 for a fall, however many levels): a rise raises hint
  `0x22` LEVELUP ("LEVEL %d EXPERIENCE", `S_GAINEDLEVEL`, mode 0: every
  time; `%d` is the hero's level, `r13-0x6E50` = `+0x3324`, set by
  `FUN_800a4268`), puts the colour's `LEVELUP_<colour>` flash on the hero
  (`FUN_80091ef4(0, +0x04)`: effect `0x80122628`[colour] = `0x39`–`0x3C`
  YEL/BLU/RED/GRE, attached to the hero model `+0x74` by `FUN_80093858`)
  and adds 100 health (`r2-0x5F88`); a fall (a Death's drain) has the
  announcer say `FUN_8009c314(player, −1)` → the sentence
  `FUN_8009f9e0(3 s, −1, player, S_LOSTLEVEL, −1)`: the hero's name
  (`S_<colour><class>2`, `S_POJO2` for the Pojo) then `S_LOSTLEVEL`. (A
  positive argument would say the name, `S_HAS`, `S_GAINEDLEVEL`; the
  level-up doesn't call it.)

  Here: `PlayerState::add_experience` (the heal) and `lose_experience`;
  `levelup.rs` watches the level each tick — the hint and the flash
  (`effects::EffectOn` on the hero) on a rise, the sentence on a fall —
  taking it as it finds it at a level start or a new hero and for the
  second after (the tests' `GDL_EXPERIENCE` lands then).

The runtime: `PlayerState` (a resource — the record outlives levels)
with `heal`, `damage`, `take_keys`, `use_key`, `take_potions`,
`grant_power`; other code hurts the hero with the `DamagePlayer { amount }`
message, applied in `FixedUpdate` after the player tick. **The sender
passes the amount after armour and difficulty**: those aren't applied
here. The death state is only recorded (`alive = false`, "DEAD" on the
overlay).

### Timed powerups

`FUN_8007ee10(amount, duration, player, subtype, value)`: time =
duration × the class's `PDAT+0x58`. The same subtype + value again adds its
amount and, if both times are positive, half the new time (a negative
time replaces it); otherwise it takes a free slot or the one nearest to
running out. Durations of −1 are powers counted by amount (crossbow shots,
breath uses).

**Each tick** the stats routine `FUN_8007c4f0` runs the eleven slots
(`+0x130`, `0x10` each; a slot counts while its time isn't 0 and its byte
`+0x1E0 + i` is 2): a positive time runs down by the tick (× 3 in one
mode, `r13-0x7788`; not at all in the tower, realm 13, or while
`r13-0x774c`/`r13-0x7340` hold) to 0, and — still counting on that tick —
adds up:

| subtype | adds |
| --- | --- |
| 5 weapon | its value to the weapon bits `+0x11C`; an element (value & 0xF: 1 fire, 2 lightning, 3 light, 4 acid) replaces the one there if none was set yet or it lasts longer |
| 6 armour | its value to the armour bits `+0x120` |
| 7 speed | its amount to the speed `+0x110`, and `0x10000` to the special bits |
| 8 magic | its amount to the magic power `+0x10C` |
| 9 special | its value to the special bits `+0x124`; the turbo power (`0x80000`) fills the meter by 100 (at most 100) and is spent |

and afterwards clamps the derived stats (strength, armour, magic, speed,
missile damage and speed) to their ranges ([player-movement.md](
player-movement.md)). The weapon bits are the kind of the hero's blows
and the start of its missiles' ([combat.md](combat.md)), so an element
power's blows burn, shock, light or dissolve what they kill.

The power-ups on the disc: weapon — `FIREICON` 1, `ELECICON` 2,
`LIGHTICON` 3, `ACIDICON` 4 (90 s), `MULTIICON` `0x80000`, `MULTI5ICON`
`0x400000` (45 s), `REFLECTICON` `0x200000` (60 s), `RAPIDFIRE`
`0x20000000` (30 s), `XBOWICON` `0x100000` (5 shots), `HAMMER_ICON`
`0x10000000` (3); armour — `INVULICON` `0x10000` (30 s), `INVULGICON`
`0x110000` (25 s), `RF_SHLD_ICON` `0x20000`, `FW_SHLD_ICON` `0x200000`,
`L_SHLD_ICON` `0x400000` (30 s), `HALOICON` `0x80000` (120 s), `GASMASK`
`0x2008` (60 s); speed — `BOOSTICON` +4 (40 s); special — `LEVITICON` 1,
`XRAYICON` 2, `INVISICON` 4, `TIMEICON` 8, `BREATHEF/A/E_ICON` `0x10` /
`0x20` / `0x40` (5 uses), `PHOENIX_ICON` `0x80`, `GROWPOT` `0x100`,
`SHRINKPOT` `0x200`, `POJOEGG` `0x400`, `TURBOPUP` `0x80000`.

**Attack overrides** (`FUN_80080d3c`, before the request switch): unless
a condition (`bVar5`, not traced) holds, a held power replaces the attack
the hero asks for — the first of: special `0x1000` or `0x2000` → 0x6E
ATTBREATHE; special `0x8000` → 0x67 ATTFIREL; special `0x4000` → 0x68;
weapon `0x100000` (the crossbow) → 0x6B SSHOT1; weapon `0x10000000` (the
hammer) → 0x70 ATTCHOP; special `& 0x70` (a breath) → 0x6E. The action
state machine (`FUN_800ab898`) then raises, as those actions hand over,
`0x2000` (0x67 → 0x69), `0x4000` (0x68 → 0x6A) and `0x800` (0x6B/0x6C →
0x6C/0x6D), which release the shots in [projectiles.md](projectiles.md),
"Hero release" (the crossbow's `0x800` spends a shot through
`FUN_8007ed38`). The chooser (`FUN_800ab898`) runs them: ATTFIREL (0x67) → 0x69 and
0x68 → 0x6A, ATTBREATHE (0x6E) → 0x6F, ATTCHOP (0x70) → 0x71 — each at
its clip's end (switch mode 0), or at once (mode 2) when a hit reaction,
knockdown or GRABBED (0x83–0x94) is asked for — to the same next action,
so the breath and the chop still go. Not a gauntlet's shot: 0x67–0x6A are
category 10, and any request above 0x72 while an action of category 1–10
plays is chosen as from READY (the chooser's prologue), so the reaction
takes over at once and the shot doesn't go (the `uVar6` test in the
gauntlets' case can't be reached). (The table's clip names run ATTFIREL,
ATTFIRELR, ATTFIRER, ATTFIRERR for 0x67–0x6A, so the left gauntlet plays
ATTFIREL then ATTFIRER, the right ATTFIRELR then ATTFIRERR.) The shot
events are raised by the chooser's hand-over switch on the old action,
which runs whenever the action (re)starts: 0x67 → 0x69 raises `0x2000`,
0x68 → 0x6A `0x4000`, and 0x6B or 0x6C → 0x6C or 0x6D `0x800`.

SSHOT1/2 (0x6B/0x6C), from the machine code: with SSHOT1 (the crossbow
shot) still asked for, the next action is SSHOT2 and the chooser writes 1
to the player's `+0xB4`; else, with a request of category 0 (the
locomotion and idle actions, `FUN_800ad42c`), SSHOTR (0x6D); else the
request itself in mode 1 (at the clip's end). `+0xB4` is the animation
slot's loop flag (the slot at `+0x7C`, its `+0x38`): each clip start
copies the clip's own loop flag into it (`FUN_8000ed70`), the chooser
overwrites it every tick (1 for READY, IDLE2_LOOP, SHOVE, the shield run
and SSHOT2 here, else 0), and at a clip's end the frame advance
(`FUN_8000ef18`) restarts the clip when it's set (else holds the last
frame). A restart counts as a start for the hand-over switch
(`FUN_8000eb70` bit 4), so **SSHOT2 repeats while the button is held and
every SSHOT1 → SSHOT2, SSHOT2 → SSHOT2 and SSHOT2 → SSHOTR raises `0x800`
— a bolt each** (SSHOT1 → SSHOTR also fires; a hand-over to any other
action doesn't). The case's third branch (any other request, category
1–12) sets mode 1 even over a knock-down's mode 2, so such a request
waits for the clip's end; a hit reaction (0x83–0x87, category 0) goes to
SSHOTR at once, with its bolt. Every clip set has the seven clips, and
SSHOT2's own loop flag is 0 in all eight — the chooser's flag is what
repeats it. Ported: all six results and their chaining (`actions.rs`:
`Next::again` is the chooser's loop flag, which `player.rs` turns into a
restart that counts as a hand-over; [powers.md](powers.md), "breaths");
the shots ([projectiles.md](projectiles.md), "Hero release"); the hammer's
blow on event `0x2000000` as ATTCHOPR starts (`FUN_80080d3c`): effect
`0x1C` EXPRING on the hero's model, damage 100 (`r2-0x5b60`) out to 35
(`r2-0x5b00`, `+0x614` = 0.1, `r2-0x5b10`, not traced) with kind `0x20`,
flags `0x2A` (monsters, items, area), a hammer use spent, a shake
(`FUN_800277ec(0.3, 0, 0, 30, 200)`) and `S_THUNDERHAMMER` (`0x50`) —
`effects.rs::spawn_chops`. Rapid fire (weapon `0x20000000`) isn't among these: the action state
machine sets the player's `+0xA8` to 0.75 (`r2-0x4da4`) instead of 1
(`r2-0x4da8`) while it's held and a throw-category action (9–10) plays —
as it does for every action under a speed power (special `0x10000`);
DEFEND2 gets 0.2 × armour (at least 0.25). `+0xA8` is the player's anim
instance (`+0x7C`) `+0x2C`, its frame length, which the action start
(`FUN_8000ed70`) recomputes — so how the 0.75 reaches the clip isn't
pinned down (not ported).

Here (`PlayerState::tick_powers`, `PowerBits`; `player.rs`
`apply_powers`): the adding up, the tower's hold, the weapon bits on
blows and missiles, speed, magic and the turbo fill; the multi-shots,
the crossbow's bolts and reflect's bouncing
([projectiles.md](projectiles.md)); the power attacks above (the
breaths, the gauntlets, the crossbow, the hammer); the armour, the
specials and the looks in [powers.md](powers.md). Not done yet: rapid
fire (above). Test with `GDL_POWERS="5:1,7:0:4:40"`
(subtype:value[:amount[:seconds]]; the crossbow `5:0x100000:5:-1`, the
left gauntlet `9:0x8000`).

## Items at run time

Items are `0xF0`-byte records (array pointer `r13-0x71a8`, count
`r13-0x71ac`), built by `FUN_800646e4` (see level-population.md). Fields
used here:

| offset | field |
| --- | --- |
| `+0x00` | item type pointer |
| `+0x04` | matrix (rows = the item's X, Y, Z axes; translation at `+0x34`) |
| `+0x54` | world centre: position + the type's centre offset, raised by 1 and turned with the item (`FUN_8005a400`) |
| `+0x64` | model instance; `+0x6C` animation instance |
| `+0xB4` | centre offset (type `+0x1C`, Y + 1.0) |
| `+0xC4` | flags (type `+0x46`): `0x1` used/active, `0x40` collides off screen, `0x100` picked up (freed when `+0xC6` runs out), `0x4000` on screen this frame, `0x8000` exit switched off; `0xFFFF` = free slot |
| `+0xC6` | timer, fields (1/60 s) |
| `+0xC8`, `+0xCA` | animation state reached, action playing |
| `+0xCB` | player who dropped it (−1 none) |
| `+0xCD` | not in this game (player-count gate): hidden, never touched |
| `+0xD4` | visibility radius: **2 ×** max(extent 0, extent 1) — not the touch radius |
| `+0xDC..` | class data: powerup value `+0xDC`, amount `+0xE0`, duration `+0xE4`, pickup delay `+0xEC`; exit code `+0xDC`, players-on-it mask `+0xE0`; transporter id/destination `+0xDC/+0xE0`, partner `+0xE4` (`FUN_80064600`) |

Item timers count **fields**: `r13-0x7584` is the number of 60 Hz video
fields since the last frame (2 at the game's 30 Hz; the frame timing in
`FUN_8002eff4` sets `dt = fields / 60`, `r2-0x742c` = 60).

The update, `FUN_800606e8`, first marks items on screen (`FUN_800b4ef4`
with the visibility radius → flag `0x4000`), runs the animation state
machine (below) and frees picked-up items, then runs each class's own
update.

### Touching items

The player update (`FUN_80080d3c`) asks `FUN_80086e44(radius, half
height, player, from, to, …)` for the items the move reaches — `from` is
the hero's collision centre `+0x64`, 2.5 above its feet, and `to` that plus
the move — and for each, `FUN_8005f0e0` is the touch test and
`FUN_8005d71c` the touch handler. So an item's centre is in reach from 3
below the hero's feet to 8 above them with the usual reach of 3 (the
rewrite had tested from the feet, `items::contact`).

**Touch test** (`FUN_8005f0e0`), with the hero's radius `r` and half
height `h` (`PDAT`: 1.5 and 2.5), the type's shape `+0x08` (u16) and
extents `+0x0C..+0x1C`, and the item's world centre `C`:

- skipped: free or picked-up items (`+0xC4` −1 or `& 0x8100`), items not
  in this game, shape 0, items neither on screen (`0x4000`) nor flagged
  `0x40`; open doors (`+0xC8` > 1, or 1 with timer > 30), burst barrels
  (obstacles 43–45 with state > 0), sounds;
- `R` = extent 0 (placed monsters: their range);
- horizontal distance from `C` within `R + r`; unless a sphere, the height
  difference within extent 1 + `h`;
- shape 1 upright cylinder: that's all; 2 sphere: 3D distance within
  `R + r`; 3 box: also within extent 2 + `r` along the item's X axis and
  extent 3 + `r` along its Z; 4 walls (`FUN_8005fd94`, the secret walls,
  obstacle `0x2A`): the hero's centre (`+0x64`, 2.5 above its feet) swept
  from `from` to `to`, radius `r`, against the level collision triangles
  the placement names (`+0x04` first, `+0x06` count → item `+0xC0/+0xC2`;
  stored in the item's frame), in the item's frame (`FUN_800bde10`) and
  within the sweep's height ± `r` (`FUN_8000e3b8`); a hit is a touch, and
  the hero is pushed out along the hit triangle's normal, level, until
  `r` from the hit point (`items::wall_contact`);
- result: `max(distance − R, 0)`, −1 for no touch;
- triggers, damage tiles, exits and transporters stop there; for the rest,
  if the hero already reached the item at `from` (always, for shape 1) and
  is moving away from its centre, it's no touch; and a push-back position
  is made: out of a box along its shallower axis; around anything else,
  along the tangent at `from` by the move's component on it.

Items shape 1 (cylinder) on the disc: powerups (extent 0.5, 2 — gold
1.25–1.5), generators (2, 5), barrels (1, 3), exits (3, 2), transporters
(1.6, 2); boxes: doors (3.7 radius, 5 high, 3.5 × 1.0), chests (3.9, 2,
1.2 × 1.0), damage tiles and trigger pads.

**Touch handler** (`FUN_8005d71c`) returns 0 walk through, 1 blocking
(the hero is moved to the push-back position), 2 standing in:

| class | on touch |
| --- | --- |
| POWERUP | if its pickup delay (`+0xEC`) is over and the hero hasn't touched a powerup this tick, it's the one picked up (`+0x8B0`) |
| CONTAINER | blocks. Locked (`0x10`, every chest) and closed: a key opens it (key −1, `S_CHEST`, flag `0x1`, contents let out by `FUN_8005e8f8`, "Containers" below); no key: hint 2. Open: a gold chest (subtype `0x2F`) gives its gold and goes; one holding what it let out is walked through and that is picked up |
| GENERATOR | blocks while it has strength |
| DOOR | closed: if the hero moves toward it (hero's move · (door − hero) ≥ 0), a key opens it (key −1, flag `0x1`, door sound, walk-through this tick); no key: hint 1 and block. Opening: blocks |
| DAMAGETILE | while active (states 2/4): `FUN_80078560(amount × level factor)` with a per-hero cooldown |
| EXIT | stands in: sets the hero's bit in `+0xE0` (returns 2) |
| OBSTACLE | blocks, except the falling ones (`0x28`, `0x35`, `0x31`: they're set off, below; `0x33`, `0x34` walked through) and a safe rock with `+0xDE` ≤ 0 |
| TRANSPORTER | stands in (returns 2); `FUN_80086e44` records it in the hero's `+0x8AC` |
| ENEMYINFO | a placed monster's item (a statue till its critter is made): with its range `+0xE8` ≥ 0 it's woken (`+0xE4 \|= 1`, [critters.md](critters.md) "Placed critters"); blocks within the type's radius (`+0x0C`) |

### Falling obstacles

Obstacles `0x28` (rock falls — `B3ROCKFALL_R7`… — and crumbling bits,
183 on the disc), `0x31` (falling leaves, D3/D4), `0x34` (walls that fall
once shot down: `A6SHOOTFALL_#0`…, F1, I1; shape 4, their own collision
triangles), `0x35` (sinking rocks: F1, F2, H2, H3, I2, I3, I5) and `0x33`
(E2's debris) fall once set off — item flag 1:

- **Touch** (`FUN_8005d71c`, class 10): `0x28` and `0x35` set off with
  `FUN_8009d154` and `0x31` with `FUN_8009d104` — the realm's sound,
  faded at the item at `0xE0` (`0x80123274`: A `S_FALLAWAY`, B
  `S_ROCKBREAK`, C `S_LIMBBREAKC`, D `S_LIMBBREAK`, E `S_ROCKBREAKE`, F
  `S_ROCKBREAKF` — F2 `S_ROCKBREAKF2` — G `S_ROCKBREAKG`, H
  `S_LIMBBREAKH`, I `S_ICEBREAK` — I5 `S_ICEBREAKY`; leaves `0x801232AC`:
  D `S_LEAFBREAK`, I `S_WOODBREAKI`); the hero walks on (returns 0).
  Their touch shape is a cylinder 15 across, 20 up and down (shape 1), so
  a hero walking toward one within 15 sets it off. `0x33` and `0x34` are
  walked through.
- **Blows** (`FUN_8005c1c8`, class 10, as the barrels): one that takes
  its last hit point sets off (flag 1, `S_BARREL_WOOD<realm>`) and stays;
  others sound `S_WEAPONHITWOOD` and flash.
- **E2's debris** (`0x33`, the item update's `0x80062670` case): each
  time the boss's stage counter `r13-0x71A0` moves on (`FUN_80063c44`, for
  boss `0x2A`) it's thrown: stage 1 up at 20 + random 10, 2 at 30 +
  random 15, 3 away from the boss's spot at 10 and up at 50 + random 100;
  then it falls like the rest.
- **The fall** (the item update, class 10, only while on screen or flag
  `0x40`): the item's matrix to angles (`FUN_800bd05c`, the locator
  builder's inverse), X += spin × `0x8011C354[slot & 7]` × dt and Z +=
  spin × `0x8011C354[~slot & 7]` × dt (steps −4…−1, 1…4), built again
  (`FUN_800bd344`); fall speed −= 2 a frame (leaves 1), position += speed
  × dt; spin 20° a second (sinking rocks 1°, leaves 10°). Below the kill
  height − 200 (`r13-0x7278`, `r2-0x6668`) it's freed. Its model and — a
  wall's — collision go with it.

(`items.rs`: `fall`, `fall_items`; the debris isn't thrown yet — the
boss's stage changes come from the critters' code.)

### Picking up (`FUN_8005de3c`)

At the end of the players' update each hero with a touched powerup picks
it up (heroes in rotating order, so two can't take the same one). By item
subtype, with the item's value (`+0x3C`), amount (`+0x40`; keys: the
placement's count) and duration (`+0x4A`):

| subtype | effect | sound | hint |
| --- | --- | --- | --- |
| 1 GOLD | gold += amount | `S_PICKUPMAGIC` (id `0x26`; the secret realm has per-player sounds) | `0x11` if > 24 |
| 2 KEY | keys += amount; if they don't all fit, as many as fit and the rest stays on the floor | `S_PICKUPKEY` | 2 (no doors in the level) / 8; ring full: 4 |
| 3 FOOD | heal by amount (refused at full health: stays, hint `0x85`); negative = poison damage | class's `S_<CLS>EATSFX`, 1 in 4 its voice `S_<CLS>EAT` (archer: one per fruit); poison `S_<CLS>POISON` | `0xF` (≥ 100), `0x10` (≥ 50), `0x1C` (poison) |
| 4 POTION | `amount` potions of kind `value`, while there's room | `S_PICKUPMAGIC` | 7, `0x5E`, `0x5F` in turn; full: 3 |
| 5–9 WEAPON, ARMOR, SPEED, MAGIC, SPECIAL | timed powerup (`FUN_8007ee10`) | `S_PICKUPSPECIAL`; `S_LEVITATEUP`, `S_GROW`, `S_SHRINK`, `S_POJO` for special bits 1, `0x100`, `0x200`, `0x400`; `S_PICKUPSHIELD` for armour `0x200000` | by value bit |
| 10 RUNESTONE | once per stone (amount = stone number) | `S_PICKUPRUNE` | `0x5A` if held |
| 13–16 | legendary item, scroll, gem, gargoyle piece: quest counters | `S_PICKUPMAGIC` | |

A Pojo (value `0x400`) hero eating a CHICKEN takes 100 damage instead
(the hero flag `0x400`). Poison food is a blow on the hero,
`FUN_80078560(amount, player, 0, 0x800, 0)`: through the resistance
routine with kind `0x800` (the gas mask and invulnerability stop it, the
gold armour heals) and into the reaction queue. Here: `items.rs` deals it
through `Player::take_blow` once the tick's pickups are done. Sound ids are
`bank << 16 | call` in the catalog (`FUN_80015a30` → `FUN_80015cac`);
bank 0 is `COMMON`.

A picked-up item gets flag `0x100` and a timer of 8 fields (15 if a
player dropped it), after which the update frees it.

**Blasts on powerups** ([mechanics.md](mechanics.md), "Blows on
items"): the hero's blows never reach powerups (armour −1; food −2), but
blasts do. An explosion (kind `0x400`) of 5 or more turns treasure into
`TREAS_JUNK`, worth 10, and blows food and timed powerups to pieces
(`ITEMEXP0` left, hint `0x87` EXPDESTROY); keys, potions (they go off)
and the quest pieces stand. Poison gas (`0x800`) above 2 spoils food:
meat becomes `BADMEAT` worth −100, fruit `GAPPLE` worth −50 — eaten as
poisoned food — with hint `0x88` GASPOISON. Here: `breakables.rs`
(`BlastItem`).

### Pickup notices

Besides its sound, hint and sparkle, a pickup shows a **plate** over the
player's panel: `FUN_8007fa7c(player, subtype, value)` takes the first
free of 24 slots (`0x80274F14`, `0x1C` bytes: state, player, subtype,
value, timer, two sprites; none free: nothing) for subtypes 1–10, 13, 15
and 16 — not scrolls. The pickup (`FUN_8005de3c`) passes gold's amount,
the keys taken (when not all fit, those left on the floor), food's health
as used (negative for poison; the Pojo's chicken −100; none when refused
at full health), 0 for potions and powers, and the item's amount for a
runestone, legendary item, gem or gargoyle piece; an opened gold chest
(`FUN_8005d71c`, subtype `0x2F`) passes 1 and its gold.

`FUN_8007f510`, every frame from the player update (`FUN_8007692c`,
outside modes `0x400D`, `0x4012`, `0x4016` and the level's opening
`r13-0x7340`), steps each slot by the fields elapsed (`r13-0x7584`):

1. **New**: two sprites (`FUN_800b3090`, textures from `STATIC`), 128
   wide, at the panel's x (`0x8011FA00[player]`: 0, 128, 256, 384):
   `S3` (128 × 16) at y 384, depth 63980, and under it at y 400, depth
   63979, the picture (128 × 64) — gold `GOLD` (`JUNK` when worth under
   11; `COINHUD` in the secret realm), keys `KEY` (`KEY_RING` from 2),
   food `MEAT` from 100 health, `FRUIT` from 0, `BADFRUIT` below 0,
   `BADMEAT` below −99, potions `MAGIC`, powers (5–9) `SPECIALS`,
   `RUNESTONE`, `LEGEND`, gems `CRYSTAL`, gargoyle pieces `GOLDNICON`.
2. **Rising**: up a pixel a field (the picture 16 below the strip) until
   the strip's top reaches 304 — the panel's own top — then a 90-field
   hold.
3. **Up**: until the hold runs out.
4. **Sinking**: down a pixel a field until the top is at 400 or lower;
   then the sprites are freed.

So a plate slides up from the screen's bottom edge over 80 fields
(1.3 s), covers the whole panel (in front of it: the panel's sprites are
at depth 64000) for 1.5 s and slides away over 96 fields; every slot moves
on its own, so quick pickups slide over each other, and every strip lies
behind every picture. The panel's text stays on top. The HUD's build at a
level's load (`FUN_8007bb40`) clears all 24; a player's own go when it's
taken out of the level (`FUN_80078de8`, Quit Level), when its death ends
(`FUN_80079094`: out of the level, or up again in the tower) and when it
quits (`FUN_80079418`).

**The runestone count.** A runestone none of the heroes held (class 10)
is set for all four players, restarts the key row (`r13-0x6fdc` = 300),
sparkles, and has the announcer count the stones the heroes in the game
hold (`FUN_8009f40c`, bits 0–12 of their `+0x1ECA`): 1 `S_RUNEFOUND1`;
2–12 `S_RUNE<n>` (`0x80122C4C`) then `S_RUNEFOUND2`; 13 nothing. Each
line is queued on the announcer's queue with no wait limit, unless a boss
level's end has begun (`r13-0x7790` ≥ 3; [frontend.md](frontend.md), "The
voice queues").

**The secret realm's coins**: each gold pickup there counts a coin for
every player in play, and the last one unlocks the level's secret
character — "The secret realm" below.

Here (`pickup_notices.rs`): the plates for player 1's panel from
`PickupNotice` messages — which the pickups in `items.rs` have yet to send
— moved by play's clock (stopped under the message box, and held under the
level's opening shot) and drawn over the panel, under the message box, not
under a menu; cleared at a level's
start, when the hero is out of the level and when it stands up again in
the tower after dying. Equal depths are drawn oldest first
(the game's order for them isn't traced). The count follows the stones
held: a new one in play, past a level start's first second (what the
load sets, and `GDL_RUNES`, count as held), queues its lines.

### Item animation

There's no spin or bob in code: **items animate through their atree**
(item `+0x6C`, `FUN_80011104(anim, action, mode)`). Every powerup's atree
has one action `ACTIVE` (potions 30 frames, gems 60, shields ~30;
treasure piles are still), so they turn and bob as keyframed.

**Which clips go round.** The animation state sits in the item at
`+0x70`; its `+0x34` (item `+0xA4`) is the clip's loop flag, which
`FUN_8000ef18` reads at the clip's end (round again, or hold the last
frame and flag it ended, `+0x36 = 0xFF`). `FUN_8000ed70` sets it from the
action's own flag (`+0x24`) whenever a clip starts. The model build
(`FUN_80065b4c`) creates the animation — its set-up (`FUN_8000e910`)
starts action 0 — then sets `+0xA4 = 1` and plays the item's action in
mode 2, which doesn't restart a clip still running with the same action:
so **an item's first action goes round until the item moves to another**,
whatever its own flag. The item update asks mode 0 while its state
(`+0xC8`) equals its action (`+0xCA`) — restart only for another action —
so a powerup, which never changes action, turns for good: the key's, key
ring's, scroll's and the reflect and boost icons' `ACTIVE` have no loop
flag of their own. An item made used (flag 1, without flag 4) starts
action 1, a restart, so its own flag rules. The trigger update clears
`+0xA4` every frame (pads never go round); the exit update sets it for
actions 0, 1 and 3 and clears it for the rest (`items.rs`,
`Item::loops`). Doors have
`CLOSE` / `ACTIV` (12 frames: the gate slides 6.5 units down) / `OPEN`;
chests `CLOSED` / `ACTIVE` / `OPEN`; exit portals `IDLE`, `READY`,
`ACTIVE1..3`; transporters a looping `ACTIVE`.

Once an item is used (flag `0x1`) with flag `0x4` in its type, the
update steps its action on: when the timer is out the action advances
(clamped at the last with flag `0x2`), and when that action ends the state
follows and the timer is set to the type's duration × 2 or the animation's
own length (`+0x80`, `+0x9C` of the animation instance — not decoded).
Stand-in: that timer's source isn't decoded, so the runtime moves to the
open action as soon as the opening one ends — a door stops blocking when
its gate has slid down (0.4 s), a chest is open after its 3.5 s lid
action.

`population.rs` builds atree models as one entity per node (`ItemRig`)
and `items.rs` plays the item's current action on them with the clip
tracks, the way characters are posed.

### Doors

`FUN_800646e4` counts doors (`r13-0x723c`; picked-up keys use it to choose
their hint). Doors aren't level collision: they're items with a box shape
(3.7 radius, 3.5 × 1.0 half widths), blocking through the touch test until
their animation reaches the open state. The door sound is
`table[realm][door subtype]` (`FUN_8009c8e0`, `0x80123434`): castle
`S_GATEA4/A2/A3/A1`, mountain `S_GATEB1`, desert `S_GATEC1/C2/C3/C1`,
forest `S_GATED1`, town `S_GATEWOODG` (subtype 3 `S_GATEMETG`), battle,
ice, dream (`S_GATEMETJ` for 3), sky; the temple, hell, secret and test
realms have none. No `LevelCollision` node is disabled: none is involved.

### Containers

Chests carry flag `0x10`: a key opens them on touch. Barrels (`0x206`, no
`0x10`) and the rest break when hit ([mechanics.md](mechanics.md), "Blows
on items"). Container subtypes: `0x2B` barrel (`BAROBJ`), `0x2C`
`CHESTEXP`, `0x2E` chest, `0x2F` gold chest (`CHESTG0`–`5`, the gold in
the model), `0x30` silver chest (`CHESTS`).

**The contents node.** After building a container's model
(`FUN_800646e4`, class 2) the game formats `%sNULL1` (`r2-0x6600`) with
the item's model name, finds that object (`FUN_800b8684`) and the model's
node drawing it (`FUN_800114e8`), keeps the node's instance as `+0xE4`
and hides it (instance flag 1). Only the chests have one — `CHESTNULL1`
and `CHESTSNULL1` in every realm's items, a 1.4 × 1.3 card: a placeholder
for where the contents go, never shown.

**Letting out** (`FUN_8005e8f8`, called as a key opens the chest; the
touch handler takes the key, sounds `S_CHEST` at it, sets flag 1 and
`+0xCB` = the player): a random contents type is resolved (choice =
(`r13-0x71B8` >> 5) + the item's slot, modulo the choices; the seed moves
on by `0x1B7` per pick and by 1 per item update, so it's effectively
random), then by the container's subtype:

- `0x30` (silver chest) holding gold becomes the level's gold chest: its
  type the last container type of subtype `0x2F` (`r13-0x71C8`), its
  model the realm items' `CHESTSG` (`r13-0x71BC`, `r2-0x65D8`; both found
  by `FUN_80067338`), `+0xE0` the gold;
- `0x2F` (gold chest): `+0xE0` = the gold;
- `0x2C` (`CHESTEXP`): flag `0x40`, `FUN_8009d330` (it ticks, then
  explodes);
- anything else: more than one key (the container's count `+0xEC` > 1)
  comes as the `KEYRING` type (`r2-0x674C`). A powerup, when the container
  has its contents node, is a new item made at the identity
  (`DAT_80127528`) and hung on the node (`FUN_800bb084`), scaled 0.2
  (instance flag 8 and `+0x40`..`+0x48` = `r2-0x6744`); the container's
  `+0xE8` and the item's point at each other. Otherwise the new item
  stands at the container and is dropped (`FUN_80064140`) — a barrel's
  `+0xCB` = `0xFE`. Keys get `+0xE0` = the count (at least 1), a scroll
  the count, and a powerup can't be picked up for 30 fields (`+0xEC`); a
  monster (class 4) is woken (`+0xE4 |= 1`).

**Growing** (the item update, class 2): while a container holds a hung
item (and isn't `0x2C`), the item's scale follows the container's state —
0.2 at 0; at 1, 0.8 × (frame + 1) / frames + 0.2 (`r2-0x66B0`,
`r2-0x66B8`, the opening clip's frame and length), or 1 if the clip is
under 2 frames; at 2 its own size (flag 8 off). The chest's `NULL1` rises
with a bounce as the lid opens (1.9 up at frame 24, then settling at
1.45), so the item comes up out of the chest growing to full size.

**Taking.** Open (state 2), a chest holding an item is walked through
(the touch handler returns 0) and the item becomes the hero's touched
powerup (`+0x8B0`) if there's none yet. The pickup (`FUN_8005de3c`) frees
the item after 8 fields (`+0xC6`, flag `0x100`), 15 when a player let it
out (`+0xCB` ≠ −1); a hung item goes back to the scene root, and its
container gets the same countdown and flag: **the chest goes with its
contents**. An opened gold chest gives its gold (hint `0x11` over 24) and
goes after 8 fields.

**Empty chests.** The item update frees an open container (state 2)
holding no item at once — but a barrel (`0x2B`) or a gold chest (`0x2F`);
a `CHESTEXP` explodes instead (`FUN_8009d210`, then freed). So a chest
that held a monster, or nothing, goes as it finishes opening.

In this rewrite (`items.rs` `release_contents`, `take_contents`,
`show_contents`; `population.rs` `CONTENTS_NODE`) as above, but: the
contents' random type is its first choice (`Population::resolve`, as
everywhere — so a silver chest's nested random always gives the first
treasure, `TREAS_JUNK`, and turns gold); a hung item stands at its
chest's centre rather than the origin and is only taken through the
chest; a monster let out comes out as a barrel's Death does
(`breakables.rs`).

### Exits

`FUN_800646e4` gives an exit `+0xDC` = its level code
(`FUN_80057b2c`: realm `<< 8 |` digit − 1), or −1 when the placement's
`+0x30` is non-zero (no code: a realm's last level), and forces flag
`0x40`. Touching one sets the hero's bit; the exit update (`FUN_800606e8`
case 9) steps the portal's actions while every living hero stands in it
(and back when they leave), and at the last state sets `r13-0x7328` to
14/15, which `FUN_8007809c` → `FUN_80086cc8` turns into the heroes' exit
state (`+0xE8` = 4). Secret exits (subtype `0x32`, `SECRET_ICON`) skip the
portal's actions: the touch sets `r13-0x72dc` = (code & 0xFF) + 3 and
`FUN_8008b92c` keeps the level they're in (`r13-0x6f74`, where the
secret realm's timer sends the heroes back). `FUN_8005b5a4` switches
exits off (`EXIT_OFF` model, flag `0x8000`) until quest conditions are
met ("Realms open", "Exits" below).

**Where an exit goes.** Only the tower's exits go where their code says.
The hero's destination `+0x830` is set by `FUN_80077ccc` from the code
(or, for a secret exit, realm `0xC` with the code's level), but the play
mode reads it only as the heroes leave the tower (`FUN_80054d18`: the
highest destination among the heroes in the game). Leaving any other
level (`FUN_80054244`, mode `0x4010`) with a normal exit (`r13-0x72dc` =
0) the next level is the tower (`r13-0x72b0`) — except from the secret
realm (`0xC`), E1 and F1, which go on to the next level of their realm
(current code + 1), and a level whose record `+0x44` names an ending
movie (`0x2B`/`0x2C`, `FUN_80019c80`), which plays it and then goes to
the tower. A secret exit goes to its secret level. The way back to the
tower passes the shop (`FUN_8009a140(0)`, mode `0x4012`: `SHOP_TOP_%s`,
`S1_PLYR_%d`, `SHP_GOLD`, `FUN_80099b4c`) unless every hero is out. So
G1's exit, coded `g2`, takes the heroes back to the tower, where G2's
portal is open now that G1 is finished ("A level finished" below). A
destination is a level id — realm and index — and the index picks its
realm WAD's level record, whose name is the folder loaded: the castle's,
dream's and sky's records aren't in their folders' order, so the tower's
`a2` portal leads to `levelA6`
([level-population.md](level-population.md), "Exit codes").

**Going out** (`FUN_8007692c` state 4, once no hero is still playing or
dying: `FUN_80077b84`, then `FUN_80077ccc` each update). The first update
sets the timer `+0x1F2` = 50 fields (0 through a secret exit), plays
`S_TUNNEL` at the first hero out's feet and stops the sound items
(`FUN_8009ca90`, `FUN_800a11c4`), keeps the floor under the hero
(`+0x8B4` → `+0x8B8`, from the floor check `FUN_800878a0`) and starts
the hero's timed texture effect with `DEATHLIGHT` (`r13-0x6f0c`, the
`WEAPONS` flipbook `DTH_LIGHT00`–`09`): `FUN_80090a00(0.4, +0x7DC,
DEATHLIGHT, 10, 1)` — a counter from −0.4, +0.4 an update, showing frame
`counter`, once more from 0 at 10 (`FUN_80090a48`; drawn with override
mode −4 by `FUN_80090aec`, as a dying monster's death texture). Every
update the timer counts down; at 0 the hero is out (state 5) and its
model hidden (`FUN_8002c450`); before that, while the floor is below
1 + 2 × the half height (`+0x854`) + its height, the hero sinks 0.12 a
field (`r2-0x5e58`; nothing sideways, `r2-0x5fac`) and spins 3π rad/s
(`r2-0x5e50` × the frame time, `FUN_800be7e8`: a turn about Y). The
classes' height (`PDAT +0x48`) is 5, so the hero sinks right through the
floor as its 50 fields run out. The damage routine hurts only heroes in
state 1, so a hero going out takes no blows. The tower's portals are
exits too: going into a realm looks the same. Nothing marks an arrival:
the heroes stand at the level's start (in the tower, beside the gates of
the realm last played, `population.rs` `start_entry`).

The portal's model (`EXIT_PORTAL`) shows its glow through the flipbook
node `XCOANIM` (under `TOPNODE`, which turns in `ACTIVE2`): nothing in
`IDLE` and `READY`; in `ACTIVE1` the purple column `EXIT_ACTIV12F02`… rises
round the hero (15 frames, 0 to 12.7 high, 1.9 across), `ACTIVE2` holds
`EXIT_ACTIVE13F2`, `ACTIVE3` runs `EXIT_ACTIV14F02`…. (The rewrite hung a
flipbook node's frames only on nodes with an object in the first action,
so the column never showed; `population.rs` `spawn_built` now hangs every
one.)

The runtime steps the portal through its actions while the hero stands in
it and resets it when they leave; at the last action the hero goes out
(`going_out.rs`, from `items.rs` as above, the light drawn through
`fade.rs` `BodyLook` like a dying monster) and 50 fields later (a secret
exit at once) `items.rs` sends `ChangeLevelTo` (`exits.rs`) for where the
exit goes (`exit_goes_to`). Stand-ins: no ending movies; the light's
second round isn't seen (the hero is gone first, as in the game). The
secret realm's timer and the way back from it: "The secret realm" below.

Sounds ([audio-format.md](audio-format.md), "Positional sounds"): while
a hero in play or going out has its exit count `+0x950` running (it
stands in an exit that isn't secret, `FUN_80086cc8`), `S_EXITFLAME` loops
at the first such hero's top point (`FUN_8007692c` → `FUN_8009ce48`,
0xE0), re-panned as it moves; the first hero out plays `S_TUNNEL` at its
feet (`FUN_80077ccc` → `FUN_8009ca90`, 0x7F) and from then every sound
item is stopped. Here the flame burns while the hero stands in an open
exit that isn't secret and as it goes out through one (stand-in: from the
first tick it stands there).

### The secret realm

Realm 12's nine levels, S1–S9 (`WDATA/SECRET.WAD`'s records, in folder
order), are reached only through secret exits: A6 → S3, B2 → S4, C2 →
S1, D3 → S2, G2 → S8, H2 → S5, I1 → S6, J3 → S7, K2 → S9 (and all nine in
a row on the test level T2, x 43 down to 11 at (x, 0, −56)). They have
no exits: a timer ends them.

**The timer.** A level record (`LEVL`) flags a timed level with `+0x00`
& 4 and gives its time at `+0x0C` (i16 seconds): only the secret
realm's, S1 70, S2 40, S3 45, S4 50, S5 130, S6 100, S7 55, S8 70, S9 60
(the other levels have 30 and no flags). As any level starts
(`FUN_80053530` → `FUN_800553b4`) the last-coin flag `r13-0x72e8` is
cleared and the level timer's four sprites are made (`0x80257000`:
`TIMER` at (1, 1), two windows on `TIMER_SAND`, `SAND_ANIM` at (63, 58)
— the hourglass the time stop borrows, [powers.md](powers.md) "`0x8`
time stop"); on a timed level they're shown, but for the falling
stream, and its time and the time left (`r13-0x72e4`, `r13-0x72e0`) are
set to the seconds + 0.99 (`r2-0x6bb8`, summed in double). Each frame of play
(`FUN_80054244` mode `0x4010` → `FUN_80054e78`, before the players'
update):

- once the opening shot is over (`r13-0x7340` = 0) the stream shows;
- with more left than the record's seconds + 1 (`r2-0x6c30`), the time
  and what's left are set to 5 (`r2-0x6c18`) — never met on the disc;
- on a timed level, with no message box or menu up (`r13-0x7598`, which
  the main loop also sets while `FUN_80070c24` has a menu open), no
  hint freeze (`r13-0x738c`) and the opening over, the time left loses
  the frame's seconds (`r13-0x7570`). Still above 0 (`r2-0x6c10`): each
  whole second it crosses (truncated), unless the last coin is in, plays
  the clock — `FUN_8009fe94(s)`: `S_SECRETCLOCK1` (`COMMON` `0x17`) for
  an even second, `S_SECRETCLOCK2` (`0x18`) an odd one, `S_SECRETCLOCKEN`
  (`0x19`) for 0, centred at 0x7F — and at 8 `S_TIMEISRUNNING`
  (`FUN_8009f2bc`, `0xC0085`), below 6, while no secret exit is being
  taken (`r13-0x72dc` = 0), `S_COUNT<s>` (`FUN_8009f3bc`, `0x80122C80[s]`
  = `0x2000B` − s), centred at 0xE0; all three played straight, not
  queued. The sand windows show the part gone, (60 × time − 60 × left)
  / (60 × time) (`r2-0x6c08`), as the time stop's do.
- At 0 or below: the four sprites are freed, the time left is 0,
  `r13-0x72dc` = 13 (`0xD`) and the pickup plates go (`FUN_8007fb00`).

(Under the mode flag `r13-0x7534 & 0x10` — not traced — a timed level's
time is 100.99 s, the time gone is drawn as `"%.1f"`, and on a level of
realm 12 its end sets `r13-0x72dc` = 2 and keeps the heroes' places.)

While a hero holds a time stop the player update (`FUN_8009fee8`, after
the timer) sets the same sprites to the time stop's time; when it ends
they stay shown in realm 12. A camera cut doesn't stop the timer.

**Out of time** (`r13-0x72dc` 13 … `0xFFFF`): the players' update sets
`r13-0x72f4` (as for any `r13-0x72dc` > 2) and `FUN_80086cc8` sends every
hero in play out (state 4, `+0x1F2` 0); `FUN_80077ccc` takes it at once —
no 50 fields when `r13-0x72dc` ≠ 0 — with `S_TUNNEL` at the first one's
feet, its model hidden, its destination `+0x830` = `r13-0x6f74`, the level
the secret exit was taken from. When the level ends (`FUN_80054244`), a
hero not out sends the party there — not to the tower, so no after-level
screen — and the secret level counts as finished (`FUN_800a1560`, as for
an exit); the load shows the plain `TRANSITION_SCREEN`
(`FUN_8001a630(0x78, 1)`, not the level's map). With every hero dead the
party goes to the tower as from any level.

**The kept level.** Taking a secret exit (the item update, exit class:
its players-on-it mask set, `r13-0x72dc` < 3, not yet used) sets
`r13-0x72dc` = (its code & 0xFF) + 3 and `FUN_8008b92c(item, player,
player +0x44)` keeps: the player and its place (`0x80284248`), the level
(`r13-0x6f74` = `r13-0x72d8`), the camera's state (`FUN_8008bc68`), every
item's type and flags `+0xC4` — `0xFFFF` for the exit itself, a free slot
and an opened container (class 2, `+0xC8` > 0) — and each trigger
target's state bytes `+0x16`/`+0x17` (`0x8025E3F0`, up to 150); the
plates go. Coming back (`FUN_80053530` with that `r13-0x72dc`): no save
(`FUN_8007a670` isn't called — nor at a secret level's start), then
`FUN_8008b7c0` lays the kept level on the fresh one: the first hero in
play at the kept place, the others beside it (`FUN_80080154(p, 2)`), the
camera as it was and no opening shot (`FUN_80026ca4(1)`); each item of
the kept type gets its flags back — freed ones freed, their models
hidden; a generator kept without flag 1 is destroyed (armour `0xFF`,
strength 0, freed) — any other is freed; the trigger targets get their
state bytes. Then `r13-0x6f74` = −1. Monsters aren't kept: the level's
generators start afresh and a placed monster made before (its item freed)
doesn't come again.

**The coins.** The secret levels' gold items are coins (`COIN_JACKAL` …
`COIN_UNI`: subtype 1, amount 0) — 100 on each level, 25 for each player
count, so one hero finds 25. The level's need `r13-0x7208` is the gold
items made as it's built for the players in the game (`FUN_80063fb0`:
class 1 subtype 1 that the player-count rule `FUN_80065d84` keeps). The
pickup (`FUN_8005de3c` class 1) adds the amount, plays the coin sound and
shows the `COINHUD` plate, sets `+0x95C` = 1 (which the player update
turns into its action choice `0xE` outside realm 12, `r13-0x7240` — not
traced further), and in realm 12 calls `FUN_800a1458` instead of raising
hint `0x11`:

- the character is `0x80124568[r13-0x7224]` by the level's index: S1–S9
  10, 11, 9, 8, 16, 12, 15, 14, 13 — the jackal, tigress, falconess,
  minotaur, sumner, ogre, hyena, medusa, unicorn (−1: none);
- every player in play (state 1) counts it (`+0x930` += 1) and gets the
  HUD's count: `+0x928` = `0x200` + the character, `+0x92C` = 60 s
  (`r2-0x5224`; a gem's is 3);
- once any has the need, each player in play gets the character's bit,
  1 << (character − 8), in `+0xA8C`, and the pickup queues
  `S_SECRETCHAR` (`FUN_8009f368`: `0x3B0025`, the announcer's queue, a
  second's most wait, refused once a boss level's end has begun), opens
  `AllCoins` page 0 for every player ("Congratulations! / You have
  unlocked a secret character!", that line its voice) and sets
  `r13-0x72e8` = 1 and the time left to 1 s (`r2-0x6804`): the level ends
  a second of play after the box is put away, unheard.

The count (`FUN_80074b08`, `+0x928` ≥ `0x200`): `"16_%sCOIN"` with the
class's code (`0x8011F878`; the sumner's `16_SUM`, `r2-0x6000`) from the
secret realm's items bank (`ITEMS/levelS`), 16 × 16 at (panel + 28, 288),
and `"%d/%d"` (`+0x930`, `r13-0x7208`) in `8Hifonts` × 1.5, white, at
(panel + 48, 292); it counts its 60 s down only in play with no menu
and no key row, which a secret level never starts. Every level start
resets `+0x930` to 0 and `+0x92C` to −1 (`FUN_80079ed8`). `+0xA8C` is
part of the character (`+0xA80…`, the memory card's): on the select
screen classes 8–15 can be picked with their bit, and the seventeenth
(the sumner) is passed over without bit 8 (`FUN_8008db78`).

**The opening.** Every level's opening shot zooms the level's name
(record `+0x14`, `font32`, white) in at the top (`FUN_8002a73c`,
`FUN_8002a574`: scale 0.025 up 0.025 a field to 2, y 48 − 16 × scale); a
timed level adds `GRAB_GOLD` ("GRAB COINS BEFORE TIME RUNS OUT",
`ENGLISH.ROM` group `0xB0`) in `0x160C03` centred at (256, 108) on a
`SCROLL_A` panel (its text's width + 60 by its height + 16), and queues
`S_GRAB` (`FUN_8009f2ec`, `0x10029`, the announcer, a second) as the name
reaches full size. (Record flag 1 would show `SHOTS_STUN` with
`S_SHOTSSTUN`; a realm's first level, `FIND_EXIT` with `S_FINDEXIT`, when
`r13-0x7668` is set.)

**Elsewhere in realm 12**: no key row as a level starts; the field count
`r13-0x732c` (the play time, likely) doesn't count; the players' `+0xA2C`
clock doesn't run (`FUN_80077f50`; neither traced); Quit Level is disabled
([frontend.md](frontend.md)); the sound items play × 4 on S1 ("Sound
items"). Record flag 8 (S5 alone): each hero in play carries a light in
its colour (the player update: `FUN_800c0dd8` at its place raised by
`r13-0x7d88`, `0x8011F9C0` + colour × 16), and `FUN_80067988` takes
another branch — not traced.

Here (`exits/secret_realm.rs`; the record's flag and seconds in
`gdl-formats` `WorldLevel::timed`): the timer and its sounds on the 30 Hz
tick (it waits for `PlayCamera::opening`; play's clock stops under menus
and the message box), the hourglass from it (`game_hud.rs`), the time up
sending the heroes out at once (`items::GoOut`) and back to the kept
level (or the tower when a secret level was started on its own — a
stand-in), the secret level finished; the kept level (`items.rs` at the
secret exit: the placements gone or let out, the chests opened, the exit,
the doors open, the placed monsters out) laid on as it's built — items
gone, doors opened again, destroyed generators gone, placed monsters made
before not made again, the heroes at the first one's place and facing.
The coins: counted for every living hero, the HUD's count, the unlock
(`PlayerState::secret_characters`, saved with the character), the line,
the message and the last second. Stand-ins and gaps: the movers' and
levers' states aren't kept (they start afresh: a lever pulled before can
be pulled again), nor the camera; the opening shot plays and the level
start saves the records unless `play_camera.rs` and `frontend.rs` ask
`SecretReturn::coming_back_to`; the select screen doesn't open the
unlocked classes yet (`secret_realm::class_open`); Quit Level isn't
disabled; the opening's banners, the plain transition screen, the gold
reaction and S5's lights aren't done; the plates aren't cleared at the
secret exit or the time up (the next level's start clears them).

### Transporters

`FUN_80064600` links each transporter to the one whose id is its
destination. `FUN_80086ab8`, each player tick: standing on a transporter
(`+0x8AC`) whose partner is live and on screen (`0x4000`), with the
cooldown clear, the hero freezes; if there's floor at the partner
(`FUN_8000d4b8`: 4 up to 10 down, the hero's radius) the realm's
transport sound plays (`0x80123834`: `S_TRANSPORTA/C/G/H/I/J/K/S3`) and a
60-field timer starts, counting down 2 per field (the hero fades out and
in); halfway the hero is moved to the partner (on its floor), the cooldown
set and hint 9 shown. The cooldown clears one tick after stepping off a
transporter.

The runtime does the same, with "on screen" tested against the play
camera's view (a stand-in for the game's sphere test). It sets the
player mover's position directly; the player's render interpolation may
show one tick's slide at the jump (player.rs owns that). The transport
sound is faded and panned at the partner (its `+0x34`; here where the
hero lands, under its centre).

### Sound items

Class 13 places a sound about the level: the placement's name is the
sound (`S_sfirea`, upper-cased for the catalog), `+0x30` its reach,
`+0x34` 0 for these (above 0, nameless, they pick the music instead),
`+0x38` flags. Each plays while a hero is near, louder the nearer, and is
re-volumed and re-panned every update — [audio-format.md](audio-format.md),
"The sound items". Here: `items.rs` (`ambient_sounds`), on the levels'
loops through `audio::LoopSoundAt`.

### Hints

`FUN_800a4268(hint, player, position)` shows a hint box and plays its
announcer line: table `0x80124668`, `0x1C` bytes per hint: priority, mode
(0 always, 2 once per player, 3 once), `ENGLISH.ROM` text group index,
string (−1 = the whole group), `VOICE1` sound id. The ones items use:

| hint | text group | voice |
| --- | --- | --- |
| 1 | `USEKEYOPENDOOR` | `S_USEKEY` |
| 2 | `USEKEYOPENCHEST` | `S_USEKEY2` |
| 3 | `FULLOFBOMBS` | `S_MAGICFULL` |
| 4 | `FULLOFKEYS` | `S_KEYFULL` |
| 7, `0x5E`, `0x5F` | `USEMAGIC2`, `THROWMAGIC`, `MAGICSHIELD` | `S_USEMAGIC2`, `S_THROWMAGIC`, `S_SHIELDMAGIC` |
| 8 | `SAVEKEYS` | `S_SAVEKEYS` |
| 9 | `TRANSPORTERSMOVEYOU` | `S_TRANSPORTER` |
| `0xB` | `HOWTOEXIT` ("ALL PLAYERS NEED / TO STEP ON THE EXIT"), once: a hero has stood in an exit for 6 fields (its `+0x254`, in the player update that ends at `0x80086cc8`) with more than one player in the game (`r13-0x7394` > 1), no level change under way (`r13-0x7764` < 0) and a normal exit (`r13-0x72dc` < 3) | `S_SAMEEXIT` |
| `0xF`, `0x10` | `EATMEAT`, `EATFRUIT` | `S_MEATGIVES`, `S_FRUITGIVES` |
| `0xE` | `SHOOTPOTIONLESSER` ("SHOOTING MAGIC / HAS A LOWER EFFECT"): a hero's missile or blast sets off a potion lying on the floor (`FUN_8002f400`) | `S_SHOOTINGMAGIC` |
| `0x11` | `COLLECTGOLD` | `S_COLLECTGOLD` |
| `0x14` | `FOUNDSECRETWALLS` ("MULTIPLE HITS DESTROY / SECRET WALLS"): a hero's melee blow (`FUN_8008615c`) that does damage to an obstacle whose type has no name (type `+0x28` = 0: the secret walls) | `S_MULTIPLEHITS` |
| `0x15` | `AVOIDOBJECTS` ("AVOID DANGEROUS OBJECTS"): a damage tile hurts the hero (`FUN_8005d71c` case 8) | `S_AVOID` |
| `0x1B` | `WOODBARREL` ("SOME BARRELS / CONTAIN ITEMS"): a barrel container (0x2B) breaks open (`FUN_8005c1c8`) | `S_SOMEBARRELS` |
| `0x1C` | `POISONEDFOOD` | `S_POISONEDFOOD` |
| `0x85` | `HEALTHFULL` | `S_HEALTHFULL` |
| `0x71` + n | legendary item n (1–11): `LEGEND_ITEMS000`–`010` ("THE SCIMITAR OF DECAPITATION", "THE LEGENDARY ICE AXE"…; `0x71` itself is `TURBOBOOST`), once | `VOICE2`: `S_SCIMITARVOX`, `S_ICEAXEVOX`, `S_LAMPVOX`, `S_BELLOWSVOX`, `S_SAVIORVOX` (5, 6, 8), `S_BOOKVOX`, `S_PARCHVOX`, `S_LANTERNVOX`, `S_JAVELINVOX` |

A hint's voice id is `bank << 16 | call` in the audio catalog's numbering
(`VOICE1` = 1, `VOICE2` = 2).

A hint up blocks others of the same priority (all 50). Its voice waits
in the announcer's voice queue and is dropped past a 0.5 s wait (with
one player, four hints are sentences naming the hero; none of those are
raised here) — [frontend.md](frontend.md), "The voice queues". The
eating lines above are the hero's own, in the heroes' queue (1 s).

**The record** (`0x80124664 + hint × 0x1C`): a freeze (`−1`: play stops
60 fields, `1`: 30 — `r13-0x738c`, held like the message box's
`r13-0x7598`; 0 on every item hint), priority, mode, text group, string
(−1: every string), voice id, and a word not traced. **Modes** (by the
"seen" bytes in each player's record, `+0x1CCC + hint`, which go to the
memory card with it; `FUN_800a4eec`): 0 always; 1 unless any player has
seen it; 2 unless this player has; 3 unless every player in the game
has. No hint shows during a camera cut (`r13-0x774c`).

**The cool-down** (`r13-0x6e54`, fields): after each hint the next of
hints 0–0x1C, 0x2C–0x2D, 0x37 and 0x50 waits `0x80124620[n]` — 0, 120,
240, 420, 600, then 600 each time (the table ends in −1 and the count
stops short of it) — counted from each level's load (`FUN_800a4d5c`
clears it); the attract loop's modes (`0x8003`, `0x8006`) wait 60.

**The box** (`FUN_800a4268`, drawn by `FUN_800a4874`; per player
`p` = 0–3, or 4 for all): the `SCROLL_A` sprite (`FUN_800b3090`)
`FUN_800b20cc(0x40)` — alpha `0x80 − 0x40/2` = 0x60 of 0x80 — the text's
widest line (`FUN_800a4dac`) + 64 wide and its height (`FUN_8001ed84`,
lines × (font height × scale + 8); 12 in the mode `r13-0x7338`) + 16
high, centred on the player's panel (`0x8011fa08[p]` = 64, 192, 320, 448;
y 250 — 256, 192 for all — or above a world point, 62 up, when one is
given) and moved to keep within x 0–511 and y 2–304, its text with it.
The lines are drawn in the group's font and scale (−1 font: the
group's; hints use `8Hi_fonts5`, slot 0's 8 × 8) 2 apart, centred on
the box (`FUN_8001f440`, y flag `0x1000`), in the player's ink
`0x8012464c[p]`: `0x1F1F00`, `0x00001F`, `0x1F0000`, `0x001F00` (dark
yellow, blue, red, green), `0x160C03` for all. It stays up (`r13-0x6e2c`)
60 fields a line + 30, counted outside cuts, where it's hidden.

Here (`hints.rs`): the box, ink, font, timing, cut rule and cool-down,
the box over the panel of the player the hint is for in that player's
ink (a hint for every player at (256, 192) in `0x160C03`). Stand-ins:
"seen" is mode 2 for every once-only hint (unless this player has seen
it), kept per player for the session, not in the character's record;
the barrels', critters' and Deaths' hints go to player 1.

## Not done

- Damage tiles (spikes, saw blades): their on/off cycle and damage.
- Trigger pads and what they move (bridges, lifts), rotators, traps.
- Breaking containers, releasing contents as items, quest effects of
  runestones, legendary items, gems and scrolls, exits switched off by
  quests, the shop.
- The announcer's class-name lines.
- Four players.

## Experience and levels

Blows on monsters earn experience (`FUN_8008625c` → `FUN_8002f288` →
`FUN_80076144`): the table value for the monster's type (`0x8011b580`
per landed blow, `0x8011b608` for the killing one — bosses up to 300; the
turbo variants at `0x8011b690`/`0x8011b718` are twice as much and also
fill the turbo meter by 0.025 × experience, not ported) × the level
record's `+0xA0`, divided by 1 + 0.1 × the levels the hero is above the
record's `+0x9C`. `FUN_800763d4` adds it (`+0x1EC0`) and raises the level
(`+0x3324`, at most 99) while the total reaches `L × ((L + 1) × 30 + 1000)`
(from level 60: `(L − 59) × 4600 + 165200`); each level re-derives the
stats through `FUN_8007f104` (+5 per stat per level, up to the class
maximum), which raises strength, armour, speed and — by 100 a level — the
health maximum. Blows on items go through `FUN_8002f400`: a blow on a
generator (item class 3) earns 5 × the hit value for its monster type
(`+0xDC`; -2 → 1, -3 → 2, other negatives → 0) and destroying it 5 × the
kill value, through the same `FUN_80076144` scaling (ported). A blow on a
potion (class 1 subtype 4) sets it off (`FUN_80076618`, the potion's magic;
[effects.md](effects.md), "Shooting potions").

## The tower's wizard

As any tower level (realm 13) loads, `FUN_80057020` calls `FUN_800a3fd8`,
which makes the idle wizard — the atree `GWIZ` (`r2-0x51a0`) from the
tower's items bank (`ITEMS/levelL`, `r13-0x7180`) — and stands him on the
lookout with param 0 (`FUN_80067194(0)`): on `levelL1` the pedestal at
(3.0, 2.0, −53.5), behind his podium, facing the heroes' start. In the
tower's mode (`0x4010`) its update (`FUN_800a20c4`) plays his actions 0, 1
and 2 in turn (READY, READING, THINKING), each to its end
(`r13-0x6e68`); after the welcome he gestures (action 6, below). The
scenes where the `WIZARD` model appears at a lookout to announce new
shards and runestones are below ("The tower wizard's scenes"). The
runtime: `tower.rs`, `tower_scenes.rs`.

**Pickup sparkles** (`FUN_8009176c(item position, kind)`): a gem sprays
effect `0x45` + its crystal counter (`GETGEMORANGE` … `GETGEMBLACK`,
`0x46`–`0x4D`, whether or not it counts), a gargoyle piece `GETGARG`
(`0x4E`, kind `0x100`), a new runestone `GETRUNE` (`0x4F`, kind `0x400`) —
`POWERUPS` atrees, depth bias −512, at the item (flags `0x80880`). Here:
`items.rs`, through `EffectAt` with its bank. (A runestone also restarts
the HUD's key/rune row timer `r13-0x6fdc` = 300 — `game_hud.rs` — and has
the announcer count the stones, `FUN_8009f40c` — "Pickup notices".)

## The tower's welcome

In the tower, once play is running after the arrival's opening shot
(`FUN_8007692c`: `r13-0x7000` armed at each level's load, a field counter
`r13-0x6fe8` held at 0 while `r13-0x733c`/`r13-0x7340` are set):

- a new game's first tower visit (`r13-0x6e98`, set by `FUN_800a32c0` on
  the session's first tower load when no hero in the game has any of its
  16 records at player `+0xA90` above 0) shows `WELCOMEMESSAGE`, all five
  pages, in the message box; then the idle wizard plays action 6
  (GESTRIGHT, `r13-0x6e68` = 6 with the restart flag `r13-0x6ea4`) and the
  camera cuts to the tower's camera point `0xC6` (`r13-0x7214`,
  `FUN_80066ab0`: hold 50 × 6 fields, no delay);
- back from `levelF2` (the last level finished `r13-0x724c`/`r13-0x7250`
  = realm 6, level 1 — "Exits" below) it shows `GARMMESSAGE`.

Here (`tower.rs`): both, with "new" read as the hero having finished no
realm's level (stand-in for the `+0xA90` records) and "play running" as
the camera settled (no opening shot, glide or cut).

## The tower's shards and runes

Beating a realm's boss (`FUN_8001b854` → `FUN_800a1d30`) sets, for every
hero in the game, the bit of the realm's place in the tower's order
(`0x801244dc`, found by `FUN_800a3360`): G, B, A, K, D, C, I, J are bits
1–8 — the eight shards — and E, F, H bits 9–11. As the tower loads,
`FUN_800a2ba8`, with the bits of every hero in the game:

- places each shard held: the effect `SHARD<n>` at the window frame's node
  (`L1WINDOWFRAME`, over the tower's door), wound on to its last frame
  (`docs/effects.md`, "Effects looked up by name"), its particle nodes
  stopped: the pane its flipbook ends on fills that part of the window,
  where a starfield shows until then;
- unless all eight are held, hides the light shining from the window
  (`L1XPLIGHTRAY01`) with everything under it: render flag 2, which the
  draw walk (`FUN_800c7d6c`) skips along with the node's children (flag
  1 hides only the node's own model);
- places each runestone held (the next word): `RUNE<n>` at `L1RUNEPLACE`
  for 1–12, and for the thirteenth (bit 12) fires trigger `0xFF` and
  shows `RUNE13` at `L1RUNE13`.

Each of these effects plays out over its action (82 frames for a rune,
104 for the thirteenth, 112 for a shard): dust clouds (`GARDUST` kind-3
flipbooks on `CFXCDU` parts, each from its own frame 38–43, their last
frame clear), a streak as the stone arrives (a flipbook run of frames
38–54), and the stone fading in (`TEXFADEIN<n>_18_19`); wound on, only
the stone in its slot is left.

What the tower sets out as it loads are the pieces already **announced**:
the character record keeps a second pair of words (player `+0x2220`
shards, `+0x2222` runestones, per record × `0xF0`), which the load reads
(`0x802776e0`/`0x802776e2`) rather than the ones won (`+0x1EC8`,
`+0x1ECA`). Then the tower's update (`FUN_800a20c4(1)`, at the end of the
load) ORs each hero's won bits into the record and into the announced
ones, and the first bit won but not announced before starts the wizard's
scene for it (`r13-0x6e8c` = shard n, or 100 + stone; the thirteenth
stone and "all twelve" have their own codes): his text and voice
(`FUN_800a33c4`), then `FUN_800a39c4` plays the piece's effect in full
(not wound on) with a camera cut to it, and — the eighth shard, the
twelfth stone — the reveals that follow (the light fading in, a realm's
portal).

Here (`tower.rs`, `quest.rs`): the announced shards and runestones
(`Quest::shards_announced`/`runes_announced`, saved with the character)
are set out as the tower loads, wound on (`WindOn`), from the tower's
items bank; the light shows once all eight shards were announced
(`quest::ShardLight`) — a new game's tower had it shining. Test with
`GDL_BEATEN=<realm bits>` and `GDL_RUNES=<stone bits>` (e.g. `0xE9E` for
the eight main bosses, `0x1FFF` for all thirteen stones) and
`GDL_LOOK_AT="3.5,0,-9.8,24"` (the rune place),
`"2.6,11,-79.9,26,180"` (the window) or `"4.1,-28.5,31.2,16"` (the
thirteenth).

## The tower wizard's scenes

`FUN_800a20c4(1)` (the end of the tower's load) picks what to announce:
for each hero, the record's shards and stones `|=` the won words, the
announced words (`+0x2220`/`+0x2222`) `|=` them too, and the first bit won
but not announced before is the code `r13-0x6e8c`: shard n (1–8), then
— overriding — stone i as 100 + i (0–12); with no new stone, 113
(`Rune13No`) when the last level finished was `levelH3`
(`r13-0x724c`/`-0x7250` = realm 8, level 2; cleared as it's announced)
and the thirteenth isn't announced, and 114 when all
twelve were announced and E is beaten (bit 9) — which calls
`FUN_800a39c4` at once. The timer `r13-0x6e90` is 3 s (`r2-0x5220`) for
these, 2 s (`r2-0x5210`) for the follow-ups; `FUN_80032824` blocks the
pads. The tower's wizard setup (`FUN_800a3fd8`) sets the start delay
`r13-0x6e80` = 60 fields.

`FUN_800a33c4`, each tower frame while the timer is up and no opening
shot runs:

- the delay counts down; then, once, the `WIZARD` atree
  (`r2-0x520c`, `FUN_80012f78(…, 0xC00880)`: additive, no depth writes —
  a glowing apparition, its READY a 49-frame flipbook loop) stands at the
  lookout nearest the heroes' centre (`FUN_8006f678`, `FUN_80067234`: any
  kind-8/10 locator), and the camera cuts to the lookout's point
  (`FUN_80066aec(kind, lookout param)`: kind 0 the heroes' ranks, 1
  shards, 2 stones; the tables `0xF0`/`0xDC`/`0xAA` + param, falling back
  to a lower kind — `docs/camera.md`, "Scene camera points"), held until
  ended;
- 120 fields on, his words type (`FUN_80019e64`, bank 0 = `SCROLL_E.ROM`,
  y 312 — `docs/frontend.md`, "Captions"): `NEWSHARDS` page n,
  `NEWRUNES` page 0, else the group's pages in turn (`MORESHARDS`,
  `ALLSHARDS`, `RUNE13NO`, `ALL12RUNESNO`/`YES`, `RUNE13YES`); his voice
  (`FUN_8009bec0`/`FUN_8009bd28`, the voice queue): `S_SHRD4TWN` …
  `S_SHRD4DRM` (`0x80123aa4`), `S_CONTINUEVOX`, `S_4KEYVOX`,
  `S_FNDRUNEYOU`, `S_RUNE13NO`, `S_12RUNENO`, `S_12RUNEYES`,
  `S_RUNE13YES`;
- once typed, the timer runs down; at 0 `FUN_800a39c4` takes him away,
  ends the cut (`FUN_8001be80`), frees the pads and places the piece: its
  effect in full (not wound on, flag `0x80000`) with a cut to its place
  (40 fields + 300 extra: `0xCA` window, `0xC9` rune place, `0xCB` the
  thirteenth) and a sound (`FUN_8009bc98`: `S_SHRDS127` with all eight
  shards, else `S_SHRD8`; `S_RUNEFALL` for a stone), and picks the next
  step `r13-0x6e7c`:
  - a shard: 10 (all eight in; the Desecrated Temple's exit, dest
    `0x500`, is made clear) or 14;
  - a stone (or codes 113/114, which place nothing): 22 without the eight
    shards and E (`0x3FE`) or twelve stones, 24 with twelve but not the
    rest, 20 with all, else 21 for 113/114 while F isn't beaten (`0x7FE`);
    the Underworld's exit (`0x600`) is made clear for any step;
  - the thirteenth (code 112): 30 with all thirteen, else 32; Garm's
    Citadel's exit (`0x803`) is made clear.

The steps (`FUN_800a20c4`), where "done" is the cut's hold and extra both
under 5: 10/14 at extra < 250 play `S_STNDGLASS` and go to 11/15; 11 when
done cuts to `0xCC`, shows the light (`L1XPLIGHTRAY01`) and fades it and
the temple's exit in over 180 fields (116 → 16); 15/16 when done announce
`MORESHARDS`/`ALLSHARDS`; 20/22/24 at extra < 250 play `S_RUNEHIT` and go
on (23 and 33 end); 21 when done cuts to `0xC9` and fades the Underworld's
exit in (126 → 26); 25/26 when done announce `ALL12RUNESNO`/`YES`;
30/32 at extra < 220 play `S_RUNEHIT`; 31 when done cuts to `0xCD`, fires
trigger `0xFF` (not snapped) and fades the citadel's exit in (136 → 36);
36 when done announces `RUNE13YES`. Each announcement brings the wizard
back at once (the delay is spent).

**Ranks** come first. `FUN_800a3d78` (at the load, and every tower frame
while nothing is announced) compares each hero's level now with the one
from its record's kept experience (`+0x7B7` by class, through
`FUN_8007654c`): a new ten of levels — or reaching 99 from below — sets
the hero's pending rank (`0x8028bcb4`), the timer to 0.5 s (`r2-0x51b0`)
and keeps the new experience. The wizard then appears with the heroes'
rank table (`0xF0` + lookout param) and, 120 fields on, for each such hero
in turn (`r13-0x7cb4`): `NEWLEVEL` ("%s %s is now / a level %d %s!") filled
with `PLAYER_COLOR_LC`, `PLAYER_CLASS_LC`, the level and the rank — the
class's `CLASS_RANK` group at (level ÷ 10) ÷ 2, `LEGEND` at 99 — typed
at 0.667 in `font32` at y 312 (`FUN_80019f5c`); his line `S_EXP<tens><class>`
(`0x80123540`: `WAR`, `VAL`, `WIZ`, `ARC`, `DWA`, `KNI`, `SOR`, `JES`,
`MIN`, `FAL`, `JAC`, `TIG`, `OGR`, `UNI`, `MED`, `HYE`), `S_EXP99ALL` at 99
(`FUN_8009c5b8`: a sentence of the announcer's queue, the hero's name
`S_<colour><class>2` — "Blue Warrior", `DWF` for the dwarf — then that
line, dropped past a 5 s wait; the pieces' lines past 10 s); after 239 fields the hero's `LEVELUP_<colour>` flash
(`FUN_80091ef4`, `0x80122628`, attached to the hero) and after 269 the
`GETGEM<colour>` sparkle (`FUN_8009176c`, `0x8012454c`: yellow 6, blue 4,
red 2, green 5 → effects `0x4B`, `0x49`, `0x47`, `0x4A` in `POWERUPS`); once
typed, 360 fields in and his line done, the next hero. The piece's words
(if any) then start at once, the wizard and his cut staying; with none
he goes 0.5 s later.

**The reveals** (`FUN_800a39c4` and the steps above): "made clear" is
the exit item found by its destination code (`FUN_8005b544(0x500 /
0x600 / 0x803)`) given transparency 255 (`FUN_800ba9b0(item +0x64, 0xFF,
1)`, over its model tree); step 11 also shows the light
(`L1XPLIGHTRAY01`, render flag 2 cleared) at transparency 255. The fade
steps (116, 126, 136) count `r13-0x6e78` down from 180 by the fields
elapsed and set both to (left × 255) / 180 — clear to solid over three
seconds — then drop to the step 100 below.

Here (`tower_scenes.rs`): all of the above for one hero, his lines in
the announcer's voice queue (`audio::QueueVoice`); the rank level
is kept as a level (`Quest::rank_level`, saved; a record without one is
checked from its level then). The reveals: the exits are made clear and
faded in, and the light with the temple's, through `fade.rs` (blended
copies of the models' materials while they're see-through; a texture
animation on a fading part holds its frame until it's whole). Test the
temple's with `GDL_BEATEN=0xE9E GDL_ANNOUNCED="0xFE:0"` (seven shards
announced before; `GDL_ANNOUNCED="<shard marks>:<stone bits>"`). Stand-ins:
a placed shard sprays its particle systems once from its place (the
effects' bursts; the game runs them along the effect's nodes). The flash
and sparkle ride the hero (`effects::EffectOn`: the model a child of the
hero, the particles bursting where it starts). The flash
(`LEVELUP_<colour>`: one `FF_K` node, render flags `0x1001800` — turned
about Y to the camera — a 4.4 × 6.3 quad from the feet up, ACTIVE 20
frames running a ten-frame flipbook of a dim blue sparkle, mean alpha
15 of 255) is drawn **additively** at depth bias −512: its maker
`FUN_80091ef4` gives the instance flags `0x880800` (`0x800000`: the
additive blend) and the effect table has −512 for `0x39`–`0x3C`; drawn
plainly blended at −128, as it was, it was a faint haze. In the rank
scene the wizard's camera also leaves the hero at the bottom edge,
mostly behind the panel. Not pinned: what the node's flag `0x1000`
(with `0x4000`, `FUN_800c67a0`'s other lighting set and the
draw context's bit 1) does to its light — ours lights it as any unlit
vertex. Test with `GDL_BEATEN`/
`GDL_RUNES`/`GDL_EXPERIENCE` on `levelL1` (a fresh hero gets the welcome
first: `GDL_MENU="b@400,b@440,b@480,b@520,b@560,b@600"`).

**The quest's messages, and where they're done** (text groups of
`TEXT/SCROLL_E.ROM` unless said):

| words | group | voice | when | here |
| --- | --- | --- | --- | --- |
| a shard | `NEWSHARDS` page n | `S_SHRD4<realm>` | the first tower visit after its boss | `tower_scenes.rs` |
| more to find, all eight | `MORESHARDS`, `ALLSHARDS` | `S_CONTINUEVOX`, `S_4KEYVOX` | once the shard is in the window (all eight: after the temple's portal fades in) | `tower_scenes.rs` |
| a runestone | `NEWRUNES` | `S_FNDRUNEYOU` | the first tower visit after it's found | `tower_scenes.rs` |
| all twelve | `ALL12RUNESNO`, `ALL12RUNESYES` | `S_12RUNENO`, `S_12RUNEYES` | once the twelfth is in place: Skorne not yet beaten (he must be banished first), or beaten (the Underworld's portal fades in first) | `tower_scenes.rs` |
| the thirteenth | `RUNE13YES`; `RUNE13NO` | `S_RUNE13YES`; `S_RUNE13NO` | in place with all thirteen (Garm's Citadel's portal fades in first); back from `levelH3` without it | `tower_scenes.rs` |
| a boss beaten | `ENGLISH.ROM`: `<BOSS>_SPEECH` ("…and recovered his shard"), `RUNE_PHRASE0`/`1`/`1B`/`2`, `SKORNE1_RUNE_YES`/`NO`, `SKORNE2_SPEECH`, `GARM2_SPEECH` | `S_DEFEATVOX<L>`, `S_RUNEVOX…` | the boss level's end ([critters.md](critters.md), "The end sequence") | `critters.rs` |
| a counter or wing open, a gate shut | `UNLOCKLEVEL`, `UNLOCKSECTION`; `NEEDCRYSTALS`, `NEEDGARGITEMS` | `S_CRYS4…`, `S_FNGS4WST`… | in the tower (below) | `quest.rs`, `mechanics.rs` |
| the welcome, Garm | `WELCOMEMESSAGE`, `GARMMESSAGE` | | the tower ("The tower's welcome") | `tower.rs` |
| a pickup | the plates, and the runestone count | `S_RUNEFOUND1`, `S_RUNE<n>` + `S_RUNEFOUND2` | any level ("Pickup notices") | `pickup_notices.rs` |
| a secret character | `ALLCOINS` | `S_SECRETCHAR` | the secret realm's last coin ("The secret realm") | `exits/secret_realm.rs` |

## Quest items and the tower's gates

Implemented in [`quest.rs`](../crates/gdl-game/src/quest.rs) (the rules and
the tower's announcements), `items.rs` (pickups, shut exits),
`mechanics.rs` (the gates) and `hints.rs` (the messages).

**The record.** Progress lives in the character record (the `0xF0`-byte
save slot at player `+0xCC`, from `+0xD18`; offsets below are the slot's,
the session copies are the player's `+0x1EC8…`):

| offset | what |
| --- | --- |
| `+0xDD4` (session `+0x1EC8`) | realms beaten, a bit per realm's place in the quest order (`FUN_8001b854` → `FUN_800a3360` → `FUN_800a1d30`) |
| `+0xDD6` (session `+0x1ECA`) | runestones, a bit per stone (`FUN_800a1e58`; the stone's item type `+0x40`) |
| `+0xDD8` | legendary items, a bit each (`FUN_800a1c34`) |
| `+0xDDC`/`+0xDDE`, `+0xDE0`/`+0xDE2` | per level record `+0x90`/`+0x92`: seen once / again (not used here) |
| `+0xDE8` i16 ×3 | gargoyle pieces (session `+0x2234`) |
| `+0xDEE` i16 ×9 | crystals by counter (session `+0x223A` in the tower) |
| player `+0x1CD0` + slot × `0xE` | per realm id, a byte of levels finished |

**Pickups** (`FUN_8005de3c`, class 1):

- **15 gem**: the item's amount is its colour code, turned into a crystal
  counter when the item is built (`FUN_800646e4`: `0x8011c308` = 4, 2, 6,
  5, 1, 7, 8, 3, 10, 5); the counter goes up (`FUN_800a1af4`) while it's
  at least 0 and below its need; the HUD shows `SM_CRYSTAL_<colour>` and
  "n/need" for 3 s (`+0x928`/`+0x92C`, `FUN_80074b08`). Sound `S_PICKUPMAGIC`
  (`FUN_8009c870`, id `0x26`), a plate (`FUN_8007fa7c`, "Pickup notices"), a sparkle
  (`FUN_8009176c`).
- **16 gargoyle piece**: amount 0 fang, 1 feather, 2 claw; counts up to
  12, 20, 28 (`FUN_800a1850`, `0x8012455c`).
- **13 legendary item**: sets its bit; hint `0x71 + amount`. The amount
  (1–11: sword, ice axe, lamp, bellows, soul, …, book, parchment, sun lamp,
  javelin) is the realm id of the boss it's for; the ice axe (2) lies in
  levelA2 and is for the mountain's dragon.
- **14 scroll**: the item's amount is the placement's `+0x30`
  (`FUN_800646e4`, unclamped; keys take theirs too, at least 1); shows
  message amount − 1 of the level's `SCROLLS<level>` group
  (`FUN_8006d7f4`). (The rewrite had given scrolls their type's amount,
  0, so none showed its text.)

In the tower, gems aren't placed at all once the orange counter is open
(`FUN_800646e4`).

**Counters** (`0x8012445c`, needs `0x80124438`, texts in
`TEXT/SCROLL_E.ROM`):

| counter | colour | needs | opens (realm) | voice |
| --- | --- | --- | --- | --- |
| 0 | tower | 0 | — | |
| 1 | orange | 15 | Forsaken Province (G, town) | `S_CRYS4TWN` |
| 2 | red | 100 | Mountain Kingdom (B) | `S_CRYS4MNT` |
| 3 | purple | 125 | Castle Stronghold (A) | `S_CRYS4CST` |
| 4 | blue | 150 | Sky Dominion (K) | `S_CRYS4SKY` |
| 5 | green | 175 | Forest Realm (D) | `S_CRYS4FOR` |
| 6 | yellow | 200 | Desert Lands (C) | `S_CRYS4DES` |
| 7 | white | 225 | Ice Domain (I) | `S_CRYS4ICE` |
| 8 | black | 250 | Dream World (J) | `S_CRYS4DRM` |

The tower levels hold 30 orange gems and 4 of each other colour across all
their placements (15 gem items are live in a one-player levelL1); the
welcome text says the tower has "enough Crystals to unlock … the Forsaken
Province". Gargoyle
sections: 0 west wing (12 Golden Snake Fangs), 1 east wing (20 Golden Eagle
Feathers), 2 lower tower / battle grounds (28 Golden Lion Claws); voices
`S_FNGS4WST`, `S_FTHS4WST`, `S_CLWS4BTL`.

**Unlocking** (`FUN_800a20c4`, the tower's update, realm 13 only): at
least 3 s (`r2-0x5200`) after the last one, each gargoyle section and then
each crystal counter whose best player has exactly what it needs is
announced — `UNLOCKSECTION`/`UNLOCKLEVEL` message n with its voice
(`FUN_8009bdf0`/`FUN_8009be58`, `0x80123aec`/`0x80123ac8`) — and set to −1
(open for good).

**Gates** (the trigger update in `FUN_800606e8`'s class code): a trigger
with flag `0x40` is a quest gate with its id byte (`+0xE2`). Touched while
shut — id below 100: crystal counter id not open (`FUN_800a1928`: −1, or a
player at its need); 101 and up: gargoyle section id − 101, 104+ counting as
2 (`FUN_800a1728`) — it shows `NEEDCRYSTALS`/`NEEDGARGITEMS` message id
(`FUN_800a200c`/`FUN_800a1f88`, at most every 10 s per gate,
`r2-0x5218`; `DEMOCLOSED` in the demo) and clears its touches. Its touches
(what's left) are copied to every trigger chained after it. In the tower
the gates are ids 1–8 on the realms' shield walls `L1XPTRAPW<letter>`
(bridge-kind movers that vanish while on), 101–103 on `L1TRAPWEASYA`,
`L1TRAPWMEDA`, `L1TRAPWHARD`, and 104 on the lift `L1LIFT01`.

**Opened as the tower loads** (`FUN_800a2ba8`): each gargoyle section and
then each crystal counter (need not 0) that some player has opened for
good (−1) has its gate triggers fired with `FUN_8005ff4c(id, 1)` — the
sections' `0x65` + section (the lower tower's also `0x68` and 199, the
elevator `L1ELEV02`), the counters' their number — and, with the
thirteenth runestone, `0xFF` (`L1ELEV669`, the H4 portal's platform, an
animated object raised to the gate's floor). `FUN_8005ff4c` takes every
trigger with that id (its id byte compared unsigned; not ones with flags
`0x8100`, so not the lift's 104, which needs standing on) and those
chained after it: flags `|= 0x400`, on (`+0xC8` and `+0xCA` = 2), its
target's state `0x2F`, and a mover snapped to its on height (`0x802571d0`
= `0x802578d8`) — or, for a target in the animated mode, its state and
previous state `0x2F`, node flag `0x200000` (play on) and, snapped,
`0x800000` with its animation's frame at its last (`FUN_80055cb8` finds
the node's animation): the snake, eagle and lion walls behind the gates
101–103 stand open as the tower loads. A bridge-kind wall then fades out
over its first ticks. A counter at its need but not yet announced stays
shut until touched. Here: `Quest::tower_gates`, `Mechanics::fire` —
before, open gates were shut again on every return to the tower until
touched, and then the animated walls played open over their first
seconds on every return.

**The lower tower's lift and elevator** (the item constructor
`FUN_800646e4`, in the tower only): a trigger with id 104 or 199 (the
lift `L1LIFT01`, the elevator `L1ELEV02`) whose target exists turns it on
as it's made once any player present has finished the battlefield's
first level (`FUN_800a1530(record, 8)`, realm 8's levels-finished byte,
bit 0; a record whose `+0xF0` is `r13-0x7d94` counts as having every
level): state and previous state `0x2F`, node flags `|= 0xA00000`, an
animated target's frame its last — the lift is down, the elevator heads
up from its off height. Here: `mechanics.rs`'s set-up (the record rule
left out).

**Realms open** (`FUN_800a12cc`, by realm id): the tower always; E once
realms 1–8 of the quest order are beaten (`FUN_800a1d98` mask `0x1FE`); F
once 1–9 are and all twelve runestones are held (`FUN_800a1ec8` `0xFFF`); H
once 1–10 are (`0x7FE`); the others once their counter (`0x80124514`: A 3,
B 2, C 6, D 5, G 1, I 7, J 8, K 4) is open or at its need. The quest order
(`0x801244dc`): tower, G, B, A, K, D, C, I, J, E, F, H.

**Exits** (`FUN_8005b5a4`, called only as the **tower** loads: its one
caller, the tower's setup `FUN_800a2ba8`, runs when the realm id
`r13-0x7220` is 0xD — a realm level's own exits are never switched off):
an exit's `+0xDC` is its destination (realm << 8 | level, 0 the first).
For E and F it's shut unless the realm is open; for H unless the realm
is open and, for its fourth level, all thirteen runestones are held
(`0x1FFF`); everywhere else level n ≥ 1 needs bit n − 1 in the players'
levels-finished byte for that realm (so the first level is always open,
behind its gate). A shut exit's model becomes
`EXIT_OFF` (from the realm's `ITEMS/level<X>` bank), its flags `+0xC4` =
`0x8000` (it goes nowhere), and the tower's `L1NSNC<letter><n>_ACTIVE`
node — its glowing trail — is hidden (instance flag `0x2`).

**A level finished.** The bit is set as a level *ends*, not as it loads.
The play mode (`FUN_80054244`, mode `0x4010`) ends the level when the
player update (`FUN_8007692c`) reports it over — the heroes gone through
an exit, a boss level's countdown out, or no hero left — and the voices
are quiet. Leaving the tower (current level = `r13-0x72b0`, `0xD00`) it
only forgets the last level finished (`r13-0x724c`/`r13-0x7250` = −1).
Leaving any other level, unless every hero in the game is out (state 0
or `0xB` — dead, or Quit Level's `FUN_80078de8`), it calls
`FUN_800a1560` with the level being left (`r13-0x7228`/`r13-0x722c`,
which the level loader `FUN_8005638c` copies from the level it loads):
that level becomes the last level finished, and for each player playing
or leaving by the exit (state 1, 4 or 5) its bit is set in the realm's
byte (and the level record's seen bits, `+0xDDC`/`+0xDE0`, first time or
again). So entering a level opens nothing; finishing it opens the next.
Starting any level but the tower (`FUN_80053d1c`, "Starting wave") also
forgets the last level finished.

Here (`exits.rs`): an exit and a boss level's end ask for the level change
as *finishing*; Quit Level, the last hero out and a game's start don't.
As the change goes ahead (after the voices) a finishing one, with the
hero alive and the level not the tower, sets the bit and the last level
finished (`tower.rs` `LevelTrail`, read by the GARM speech and the
thirteenth stone's announcement). Saves made before this rule kept the
levels *entered* (as `entered`); loading one drops those bits.

**In this rewrite** (the gates and messages): all of the above for one player; the messages show
in the game's message box (`docs/frontend.md`, "Message box"). Stand-ins
and gaps: the pickup plates ("Pickup notices") wait for the pickups to
send them; the level-record seen bits (`+0xDDC…`) aren't kept. The gem/gargoyle count shows above the panel
(`game_hud.rs`: `SM_CRYSTAL_<colour>` or `SM_FANGS`/`SM_FEATHERS`/
`SM_CLAWS` at panel x + 28, y 288, 16 × 16; "n/need" in font 1 at scale
1.5 at x + 48, y 292; `FUN_80074b08`). The game builds the icon's name
as `SM_CRYSTAL_<colour word>` (`SM_CRYSTAL_ORANGE`), but the art is named
with three letters (`SM_CRYSTAL_ORA`) and the lookup (`FUN_800b8354`) is a
30-letter `strncmp`, so the retail game never finds it (it gets texture 0);
we draw the matching icon.
Test aids: `GDL_CRYSTALS="<counter>:<n>,…"` sets counts at the start; the
town gate is at `GDL_WARP="21.9,-1.9,-79.4"` in levelL1 (frame it with
`GDL_LOOK_AT="22,-1,-84,18,180"`, the exit circle with
`"37,-6.5,-121,26,160"`).
