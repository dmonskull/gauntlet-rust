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
  `L` is `(L − 1)(30L + 1000)` up to 60, then `(L − 60) × 4600 + 165200`;
  each level gained adds 100 health. Nothing here awards experience yet.

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
breath uses). Stand-in: the game's clock that runs the slots down wasn't
traced — the runtime counts positive times down at one per second and
drops a slot at zero.

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
height, player, from, to, …)` for the items the move reaches; for each,
`FUN_8005f0e0` is the touch test and `FUN_8005d71c` the touch handler.

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
  extent 3 + `r` along its Z; 4 walls (`FUN_8005fd94`, not ported);
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
| CONTAINER | blocks. Locked (`0x10`, every chest) and closed: a key opens it (key −1, `S_CHEST`, flag `0x1`, contents released by `FUN_8005e8f8`); no key: hint 2. An opened gold chest (subtype `0x2F`) gives its gold and goes |
| GENERATOR | blocks while it has strength |
| DOOR | closed: if the hero moves toward it (hero's move · (door − hero) ≥ 0), a key opens it (key −1, flag `0x1`, door sound, walk-through this tick); no key: hint 1 and block. Opening: blocks |
| DAMAGETILE | while active (states 2/4): `FUN_80078560(amount × level factor)` with a per-hero cooldown |
| EXIT | stands in: sets the hero's bit in `+0xE0` (returns 2) |
| OBSTACLE | blocks, except crumbling floors (`0x28`, `0x35`, `0x31`: they start falling) |
| TRANSPORTER | stands in (returns 2); `FUN_80086e44` records it in the hero's `+0x8AC` |

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
(the hero flag `0x400`); not in the runtime. Sound ids are
`bank << 16 | call` in the catalog (`FUN_80015a30` → `FUN_80015cac`);
bank 0 is `COMMON`.

A picked-up item gets flag `0x100` and a timer of 8 fields (15 if a
player dropped it), after which the update frees it.

### Item animation

There's no spin or bob in code: **items animate through their atree**
(item `+0x6C`, `FUN_80011104(anim, action, mode)`). Every powerup's atree
has one looping action `ACTIVE` (potions 30 frames, gems 60, shields ~30;
treasure piles are still), so they turn and bob as keyframed. Doors have
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
`0x10`) and the rest break when hit (combat, not built). `FUN_8005e8f8`
releases the contents: a random type is resolved like a placement's; a
`CHEST GOLD` (`0x2F`) keeps the contents' amount as its own gold, taken by
touching it once open; `EXP BARREL` (`0x2C`) explodes (`FUN_8009d330`);
otherwise a new item of the contents type is made at the container,
dropped to the floor, with a 30-field pickup delay (keys take the
placement's count, `+0x34`). Stand-in: the runtime can't spawn a new
item's model yet, so a key-opened chest's contents are applied to the hero
at once, as if picked up.

### Exits

`FUN_800646e4` gives an exit `+0xDC` = its level code
(`FUN_80057b2c`: realm `<< 8 |` digit − 1), or −1 when the placement's
`+0x30` is non-zero (no code: a realm's last level), and forces flag
`0x40`. Touching one sets the hero's bit; the exit update (`FUN_800606e8`
case 9) steps the portal's actions while every living hero stands in it
(and back when they leave), and at the last state sets `r13-0x7328` to
14/15, which `FUN_8007809c` → `FUN_80086cc8` turns into the heroes' exit
state; `FUN_80077ccc` lifts and spins the hero for 50 fields and the level
changes to the code. Secret exits (subtype `0x32`, `SECRET_ICON`) go at
once, to the secret realm. `FUN_8005b5a4` switches exits off (`EXIT_OFF`
model, flag `0x8000`) until quest conditions are met — not in the
runtime.

The runtime steps the portal through its actions while the hero stands in
it and resets it when they leave, then waits 50 fields and sends
`ChangeLevelTo("level<code>")` (`exits.rs`), which becomes the world's
relative `ChangeLevel`. Stand-in: an exit without a code goes to the hub,
`levelL1`. The hero's lift and spin aren't drawn.

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
show one tick's slide at the jump (player.rs owns that).

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
| `0xF`, `0x10` | `EATMEAT`, `EATFRUIT` | `S_MEATGIVES`, `S_FRUITGIVES` |
| `0x11` | `COLLECTGOLD` | `S_COLLECTGOLD` |
| `0x1C` | `POISONEDFOOD` | `S_POISONEDFOOD` |
| `0x85` | `HEALTHFULL` | `S_HEALTHFULL` |
| `0x71` + n | legendary item n (1–11): `LEGEND_ITEMS000`–`010` ("THE SCIMITAR OF DECAPITATION", "THE LEGENDARY ICE AXE"…; `0x71` itself is `TURBOBOOST`), once | `VOICE2`: `S_SCIMITARVOX`, `S_ICEAXEVOX`, `S_LAMPVOX`, `S_BELLOWSVOX`, `S_SAVIORVOX` (5, 6, 8), `S_BOOKVOX`, `S_PARCHVOX`, `S_LANTERNVOX`, `S_JAVELINVOX` |

A hint's voice id is `bank << 16 | call` in the audio catalog's numbering
(`VOICE1` = 1, `VOICE2` = 2).

A hint up blocks others of the same priority (all 50). Stand-ins: each
is shown once per session, for 3 seconds.

## Not done

- Damage tiles (spikes, saw blades): their on/off cycle and damage.
- Trigger pads and what they move (bridges, lifts), rotators, traps.
- Breaking containers, releasing contents as items, quest effects of
  runestones, legendary items, gems and scrolls, exits switched off by
  quests, the shop.
- The announcer's class-name lines; the pickup score pop-ups
  (`FUN_8007fa7c`).
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
not ported).

## The tower's wizard

As any tower level (realm 13) loads, `FUN_80057020` calls `FUN_800a3fd8`,
which makes the idle wizard — the atree `GWIZ` (`r2-0x51a0`) from the
tower's items bank (`ITEMS/levelL`, `r13-0x7180`) — and stands him on the
lookout with param 0 (`FUN_80067194(0)`): on `levelL1` the pedestal at
(3.0, 2.0, −53.5), behind his podium, facing the heroes' start. In the
tower's mode (`0x4010`) its update (`FUN_800a20c4`) plays his actions 0, 1
and 2 in turn (READY, READING, THINKING), each to its end
(`r13-0x6e68`). His other actions (WELCOME, GOAWAY, GESTLEFT…) and the
scenes where the `WIZARD` model appears at a lookout to announce new
shards and runestones (`FUN_800a33c4`: `NewShards`, `MoreShards`,
`AllShards`, `NewRunes`, `Rune13No/Yes`, `All12RunesNo/Yes`; then
`FUN_800a39c4` sets the shard in `L1WINDOWFRAME` or the stone in
`L1RUNEPLACE` with a camera cut) aren't done yet. The runtime: `tower.rs`.

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

Here (`tower.rs`, `quest.rs`): the shards and runestones are set out as the
tower loads, wound on (`WindOn`), from the tower's items bank; the light
shows only once all eight shards are in (`quest::TowerPiece`, bits by
`quest::boss_marks`) — a new game's tower had it shining. Stand-in: there
are no announced bits and no scenes yet, so pieces just won are set out
wound on with the rest. Test with
`GDL_BEATEN=<realm bits>` and `GDL_RUNES=<stone bits>` (e.g. `0xE9E` for
the eight main bosses, `0x1FFF` for all thirteen stones) and
`GDL_LOOK_AT="3.5,0,-9.8,24"` (the rune place),
`"2.6,11,-79.9,26,180"` (the window) or `"4.1,-28.5,31.2,16"` (the
thirteenth).

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
| player `+0x1CD0` + slot × `0xE` | per realm id, a byte of levels entered |

**Pickups** (`FUN_8005de3c`, class 1):

- **15 gem**: the item's amount is its colour code, turned into a crystal
  counter when the item is built (`FUN_800646e4`: `0x8011c308` = 4, 2, 6,
  5, 1, 7, 8, 3, 10, 5); the counter goes up (`FUN_800a1af4`) while it's
  at least 0 and below its need; the HUD shows `SM_CRYSTAL_<colour>` and
  "n/need" for 3 s (`+0x928`/`+0x92C`, `FUN_80074b08`). Sound `S_PICKUPMAGIC`
  (`FUN_8009c870`, id `0x26`), a pickup notice (`FUN_8007fa7c`), a sparkle
  (`FUN_8009176c`).
- **16 gargoyle piece**: amount 0 fang, 1 feather, 2 claw; counts up to
  12, 20, 28 (`FUN_800a1850`, `0x8012455c`).
- **13 legendary item**: sets its bit; hint `0x71 + amount`. The amount
  (1–11: sword, ice axe, lamp, bellows, soul, …, book, parchment, sun lamp,
  javelin) is the realm id of the boss it's for; the ice axe (2) lies in
  levelA2 and is for the mountain's dragon.
- **14 scroll**: the item's amount is the placement's `+0x30`; shows
  message amount − 1 of the level's `SCROLLS<level>` group
  (`FUN_8006d7f4`).

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
thirteenth runestone, `0xFF` (`L1ELEV669`, which doesn't move: heights
0/0). `FUN_8005ff4c` takes every trigger with that id (not ones with
flags `0x8100`) and those chained after it: flags `|= 0x400`, on (`+0xC8`
and `+0xCA` = 2), its target's state `0x2F`, and a mover snapped to its on
height (`0x802571d0` = `0x802578d8`); a bridge-kind wall then fades out
over its first ticks. A counter at its need but not yet announced stays
shut until touched. Here: `Quest::tower_gates`, `Mechanics::fire_open` —
before, open gates were shut again on every return to the tower until
touched.

**Realms open** (`FUN_800a12cc`, by realm id): the tower always; E once
realms 1–8 of the quest order are beaten (`FUN_800a1d98` mask `0x1FE`); F
once 1–9 are and all twelve runestones are held (`FUN_800a1ec8` `0xFFF`); H
once 1–10 are (`0x7FE`); the others once their counter (`0x80124514`: A 3,
B 2, C 6, D 5, G 1, I 7, J 8, K 4) is open or at its need. The quest order
(`0x801244dc`): tower, G, B, A, K, D, C, I, J, E, F, H.

**Exits** (`FUN_8005b5a4`, on each level's load): an exit's `+0xDC` is its
destination (realm << 8 | level, 0 the first). For E and F it's shut unless
the realm is open; for H unless the realm is open and, for its fourth level,
all thirteen runestones are held (`0x1FFF`); everywhere else level n ≥ 1
needs bit n − 1 in the players' levels-entered byte for that realm (so the
first level is always open, behind its gate). A shut exit's model becomes
`EXIT_OFF` (from the realm's `ITEMS/level<X>` bank), its flags `+0xC4` =
`0x8000` (it goes nowhere), and the tower's `L1NSNC<letter><n>_ACTIVE`
node — its glowing trail — is hidden (instance flag `0x2`). Loading a level
(`FUN_800a1560`) sets that level's bit for each player.

**In this rewrite**: all of the above for one player. Stand-ins and gaps:
the messages draw as plain centred text for 5 s (the game's message box
isn't built); the pickup notices and sparkles aren't shown; progress isn't
saved between runs (there's no save yet); the level-record seen bits
(`+0xDDC…`) aren't kept. The gem/gargoyle count shows above the panel
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
