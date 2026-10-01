# The after-level screen, the shop and the inventory

Implemented in [`shop.rs`](../crates/gdl-game/src/shop.rs). Decoded from
`main.dol` (`r2 = 0x8034D100`, `r13 = 0x8034B4E0`); the item table is
`SHPDATA/SHOP.WAD`. The fonts, the 2D screen and the select screen are in
[frontend.md](frontend.md); where an exit sends the party, in
[items.md](items.md) "Exits".

## Entering (`FUN_8009a140`)

`FUN_8009a140(kind)` puts the game in mode `0x4012` with `r13-0x6eec` =
kind: **0 after a level**, **1 the Shop**, **2 the Inventory** (the Tower
Menu's items `0xE`/`0xF`, `FUN_80070c24`). It clears `r13-0x6ee0` (nobody
in) and `r13-0x6ef0` (the screen not built yet). For kinds 1 and 2 it first
asks for the tower's start entry nearest the camera's point
(`FUN_800a117c` → `r13-0x7cb8`, used when the tower starts again) and runs
`FUN_8007a4f4` (dying heroes back to state 1, health from the class
record's saved copy).

Then, after a normal exit (`r13-0x72dc` < 2) or one in the secret realm
(12): `r13-0x6ee0` = 1 when no player record is in state 1 or 5 (every
hero out — dead); the tower is loaded behind (`FUN_80053d1c`), the
`TRANSITION_SCREEN` sprite (512 × 320) covers it, the HUD panels are
rebuilt. After a secret exit (`r13-0x72dc` ≥ 2, not 12) `r13-0x6ee0` = 1:
no screen, the secret level comes next. `FUN_80054244` (mode `0x4010`)
calls `FUN_8009a140(0)` when the level just left goes back to the tower,
and mode `0x400E` (an ending movie) when `FUN_80019bec` returns 2.

In mode `0x4012` the play loop (`FUN_80054244`) runs `FUN_800980dc` every
frame: 0 → the players' update (`FUN_8007692c`) runs too and the screen
goes on; 2 with `r13-0x6ee0` = 0 → **the select screen in mode 2**
(`FUN_8008ffec(2)`, below); anything else → `FUN_80053530`, the next level
(the tower: for the Shop and Inventory, the tower starts again).

## The screen (`FUN_800980dc`)

Every frame: while the tower loads (`FUN_8008f790` = 0) the screen is
`TRANSITION_SCREEN` (`FUN_80053a7c(0)`) and `"Loading..."` shimmering at
(340, 260), scale 1. Once loaded it builds itself once (`FUN_80099b4c`,
`r13-0x6ef0` = 1), the HUD panels draw every frame (`FUN_8007496c(-1)`),
and with somebody in (`r13-0x6ee0` = 0) and no message box or menu
(`r13-0x7598`, `r13-0x70d0`) each player record in state **1 or 5** runs
its own step (`+0xA64`, below); a player at step 100 is set to state 5.
The screen is done once every such player is at 100, the tower has loaded
(`FUN_80053b74`) and the voice queues are empty (`FUN_8001538c(1)`); it
then returns 2 after a level, else 1. With nobody in it shows only the
transition screen and returns as soon as the tower is in.

### Columns (`FUN_80099b4c`)

Four 128-wide columns, x = `0x801226f8[p]` = 0, 128, 256, 384, centres
`0x80122708[p]` = 64, 192, 320, 448. Per column (depth: lower is in front):

| sprite | where | depth |
| --- | --- | --- |
| `S1_PLYR<p+1>` | (col, 0), 128 × 256 | 64000 |
| `S2_PLYR<p+1>` | (col, 256), 128 × 64 | 64000 |
| `S1_BORDER`, `S2_BORDER` | (col, 0), (col, 256); glow flag `0x4000` | 63900 |
| `SHOP_TOP_<colour>` (`YEL`, `BLU`, `RED`, `GRE` by the record's `+0x04`, `0x80122728`) | (col + 32, 0), its size (64 × 32); hidden for a player not in the game | 63800 |

and, for a player in state 1 or 5, the six sprites of the table at
`0x80122698` (name, x, y, depth − 64000), hidden until used:

| sprite | where | depth |
| --- | --- | --- |
| `SHP_GOLD`, `SHP_BONES`, `SHP_EXP` (128 × 256 heaps) | (col, 320) | 63990 |
| `SHOP_SCROLL_1` | (col, 0), 128 × 256 | 63980 |
| `SHOP_SCROLL_2` | (col, 256), 128 × 64 | 63980 |
| `S2_BORDER` | (col, 256) | 63960 |

and one sprite per shop item (its icon, the record's name, at (col + 20,
0) at its own size; hidden). Each page's title is drawn at the column's
centre, y 8, `font32` × 0.45 (`r2-0x544c`) in black (`0x000000`), on the
`SHOP_TOP` plate: `"Stats"` (`r2-0x5448`), `"Shop"` (`r2-0x53a4`) or
`"Inv"` (`r2-0x6344`). Every page's prompt is the A button's picture
(`r13-0x6cb0`, here `BUTTON_X`) and `"Continue"` shimmering (`FUN_8001eb80`,
× 0.5, `r2-0x5450`).

### A player's steps (`+0xA64`)

| step | what | next |
| --- | --- | --- |
| 0 | set up: the tally (`FUN_800998e4`), `+0xA6C` and the bought-stat counters `+0xA70..+0xA7C` = 0 | after a level 1; else the gold heap's target = gold, 6 |
| 1–3 | the tally (`FUN_80099694`) | A once the heaps are up → 4 |
| 4 | the level-up page (`FUN_80098c48(p, 0)`) | 20 after levelH4 (`r13-0x7230` = 8, `r13-0x7234` = 3), else 5 |
| 5 | the gold heap's target = gold | 6 |
| 6 | the Shop: show the scrolls, hide the icons, 7 (the same frame); the Inventory: remove the sprites | 7 / 9 |
| 7 | the shop's list (`FUN_800994fc`, `FUN_8009a280`) | EXIT → remove the sprites, 8 |
| 8 | the stats page (`FUN_80098c48(p, 1)`) | A → 9 |
| 9 | the Shop: done; else open the inventory (`FUN_8006c614`) with `S_STNDGLASS` (`FUN_8009bc98(2)`) | 100 / 10 |
| 10 | the inventory (`FUN_8006c64c`) | A → closing (`FUN_8006c5e4`), 11 |
| 11 | the inventory closing | when it's done (`FUN_8006c4d8`) → 12 → 100 |
| 20 | the final stats (`FUN_80098808`) | A → 21 → 100 |
| 100 | done: the player waits for the others | |

Steps 2 and 3 do what 1 does; nothing sets them. So after a level: the
tally, the level-up page (or the final stats after levelH4, and nothing
more), the shop, the stats page, the inventory. The Shop: the list and the
stats page. The Inventory: the inventory alone.

Pressing (each player's own pad, the "pressed this frame" word
`0x802407a8 + p × 0x3C`): A `0x2000000`, X `0x1000000`, B `0x8000000`, up
`0x30`, down `0xC0`. The pages' timer `+0xA6C` (low 16 bits) counts
fields (`r13-0x7584` a frame) while below `0xF000`; the level-up and stats
pages start it at 1, and A leaving a page sets it to 0.

## The tally (steps 1–3)

`FUN_800998e4` measures the level against the level record's full marks
(`r13-0x72bc`, the `LEVL` record of the level just played, `+0xE0` gold,
`+0xE4` kills, `+0xE8` experience; the boss levels have kill marks of 1 to
15 and gold marks of 3,000–20,000):

- **gold** = gold (`+0x1EC4`) − the kept copy's (`+0x224C` + class ×
  `0xF0`);
- **kills** = the class's monsters killed + generators destroyed
  (`+0xC10` and `+0xC20` + class × `0x1C`) − the kept copy's (`+0x205C`,
  `+0x206C`);
- **experience** = `+0x1EC0` − the kept copy's (`+0x1EDC` + class ×
  `0x18`).

"The kept copy" is the record as it was saved when the level started
(`FUN_8007a670`: `+0xA80..+0x1EB4` copied to `+0x1ECC..`). Each heap's
height is `gained × 208 / (mark + 1)` (C integer division), at least 64 and
at most 208 (`r13-0x7cf0` − `r13-0x7cf4` = 320 − 112).

The three are ranked tallest first (ties: gold, then experience, then
kills — the comparisons in `FUN_800998e4`) and rise in that order: each
heap starts at 20 (`r13-0x7cec`) and grows by 1.5 a field (`n + n >> 1`
for `n` fields) to its height; one rises only once those before it are
up, and until then it's hidden. A heap of height `h` is its texture's top
`h` rows (`FUN_800b2358`: u 0–1, v 0 to h/256) drawn from y 320 − h down to
320: it climbs out from behind the HUD row. While any heap of any player
rises, `S_TALLYSFX<realm>` (`0x80122c1c[r13-0x7244]`: the bank of the realm
just played, call 1, `0x1A0001` for realm 1 … `0x240001` for 11; none for
0, 12 and the tower) is played again whenever it isn't playing
(`FUN_8009f4e0`: `FUN_80015694`, volume `0xE0`), and stopped once none
rises.

The words (`FUN_80099694`), at x col + 16, `font32` × 0.5 (`r2-0x5450`):
`"%s: %d"` with `"Gold"` (y 32), `"Kills"` (y 52) and `"Exp."` (y 72)
(`0x80122738`, `0x80122744`) and the amounts — shimmering
(`FUN_8001eb80`) while its heap rises, white `font32` otherwise. With every
heap up: the A button (20 × 20) at (col + 16, 89) and `" Continue"` at
(col + 32, 92). A: `S_OPTMENUSEL` (`FUN_8009cc0c`: sound `0x11`), the heaps
hidden, the timer 0, step 4.

## The level-up and stats pages (`FUN_80098c48`)

Mode 0 after the tally, mode 1 after the shop. The level "before" is the
one the kept experience gives (`FUN_8007654c` of `+0x1EDC` + class ×
`0x18`: the level `L` with `(L − 1)(30L + 1000)` ≤ experience, from 61 on
`(L − 60) × 4600 + 165200`); in mode 0, with no level gained, the page
returns at once and isn't seen. Its first frame (mode 0) has the announcer
say `FUN_8009c314(p, 1)`: the hero's name `S_<colour><class>2`, `S_HAS`,
`S_GAINEDLEVEL` (a sentence, 3 s most wait, `FUN_8009f9e0`).

At x col + 8 (labels) and col + 88 (values), `font32` × 0.48
(`r2-0x5480`), white:

- `"Level %d"` (the level now) centred at y 32, × 0.75 — shimmering in
  mode 0;
- the rank centred at y 64, × 0.6: `LEGEND` at level 99, the class's name
  (`PLAYER_CLASS_LC`, by `+0x0C`) below 10, else the class's `CLASS_RANK`
  group at (level ÷ 10) ÷ 2;
- `"Strength"`, `"Armor"`, `"Magic"`, `"Speed"` at y 96, 116, 136, 156 with
  their values: before = the raw stat (`FUN_8007f104`) at the level before
  with the kept copy's bought points, less the points bought on this visit
  (`+0xA70`, `+0xA74`, `+0xA78`, `+0xA7C`); after = the stat now. A stat
  that changed lights up in turn for 60 fields — the label shimmering, the
  value switching to the new one (shimmering) from then on — starting at
  field 90 (mode 0) or 30 (mode 1);
- `"Max"` (y 188) and `"Health"` (y 204), the value at y 196: in mode 0
  they light up next for 60 fields, the value switching from the maximum
  before (now − 100 × levels gained, `r2-0x5408`) to now's
  (`FUN_80078510`: 100 × (level − 1) + 500, at most 9999); in mode 1 the
  maximum, plain;
- in mode 0 at levels 25 and 50, the class's magic potion line
  (`MAGIC_ATT1`/`MAGIC_ATT2`, string `+0x08` — the class less 8 above 7)
  centred at y 224 in `0xFF80C0`, `font32` at the group's 0.45. The groups
  hold four strings (warrior, valkyrie, wizard, archer: junk into silver /
  gold, traps, poison into fruit / meat, secret walls), so the other
  classes' lookup fails and nothing is drawn.

When the lights are done: A (`S_OPTMENUSEL`, timer 0) leaves; until then
the A button (16 × 16) at (col + 16, 280) and `"Continue"` at
(col + 40, 280).

**Bought points show twice.** The "before" value is worked out from the
kept copy's points (as the level started) less this visit's; a visit after
a level — nothing bought since the level started — shows a +10 as +20.
The game's arithmetic, kept here.

### Raw stats (`FUN_8007f104`)

`min(start + 5 × (level − 1), max) + bought`, at most 999 (`r2-0x5d08`),
for strength (`PDAT +0x28`), armour (`+0x38`), magic (`+0x40`) and speed
(`+0x30`), kept as floats at `+0xF4`, `+0xF8`, `+0xFC`, `+0x100`; the
bought points are the class record's floats `+0xA98`, `+0xA9C`, `+0xAA0`,
`+0xAA4` (+ class × `0x18`). The summoner (class 16, which plays as class
2 with `+0xF0` = `r13-0x7d94`) has 999 in all four.

## The shop

### Items (`SHPDATA/SHOP.WAD`, chunk `ITEM`)

Loaded by `FUN_8009b78c` (`"shpdata"`, `"shop.wad"`) into `r13-0x6ee4`
(count `r13-0x6ee8`), as the tower and select screen load
(`FUN_80053b9c`) and with the in-game icons (`FUN_8008010c`). `0x50`-byte
records:

| offset | field |
| --- | --- |
| `0x00` | icon: a texture name (empty for EXIT) |
| `0x20` | text, lines split by `\n` |
| `0x40` | f32 text scale (× 0.5) |
| `0x44` | type |
| `0x48` | price |
| `0x4C` | amount |

The 34 records, cheapest first:

| # | text | type | price | buys |
| --- | --- | --- | --- | --- |
| 0 | EXIT | 0 | 0 | leaves the shop |
| 1 | Cherry | 17 | 50 | 20 health |
| 2 | Key | 1 | 100 | a key |
| 3 | Levitation | 18 | 150 | special `1`, 60 s |
| 4 | Meat | 17 | 250 | 100 health |
| 5 | Potion | 3 | 250 | a potion of a random kind 1–4 (`FUN_800bcf9c(4)` + 1) |
| 6 | Growth | 19 | 300 | special `0x100`, 30 s |
| 7–10 | Fire / Electric / Light / Acid Amulet | 20–23 | 350 | weapon `1`/`2`/`3`/`4`, 90 s |
| 11–13 | Fire / Lightning / Acid Breath | 25–27 | 350 | special `0x10`/`0x40`/`0x20`, 5 uses |
| 14 | Super Shot | 24 | 400 | weapon `0x100000`, 5 uses |
| 15 | Reflect Shield | 9 | 400 | armour `0x20000`, 30 s |
| 16 | Electric Shield | 14 | 425 | armour `0x400000`, 15 s |
| 17 | Fire Shield | 15 | 425 | armour `0x200000`, 15 s |
| 18 | Phoenix | 10 | 450 | special `0x80`, 45 s |
| 19 | Rapid Fire | 11 | 450 | weapon `0x20000000`, 30 s |
| 20 | Hammer | 13 | 500 | weapon `0x10000000`, 3 uses |
| 21 | 3 Way Shot | 12 | 550 | weapon `0x80000`, 45 s |
| 22 | Invisible | 30 | 600 | special `4`, 15 s |
| 23 | Invulnerable | 31 | 600 | armour `0x10000`, 30 s |
| 24 | Xray Glasses | 32 | 650 | special `2`, 120 s |
| 25 | Gas Mask | 33 | 650 | armour `0x2008`, 15 s |
| 26 | Anti Death | 35 | 750 | armour `0x80000`, 120 s |
| 27 | Hand of Death | 36 | 1000 | special `0x200000`, 120 s |
| 28 | Health Vampire | 37 | 1000 | special `0x400000`, 120 s |
| 29 | Mikey Decoy | 38 | 1000 | special `0x100000`, 120 s |
| 30–33 | Add 10 to Strength / Speed / Armor / Magic | 5–8 | 1000 | 10 bought points |

`ORIGSHOP.WAD` is an older list (31 records, without 27–29); nothing loads
it.

### Buying (`FUN_8009a280`, A)

With gold (`+0x1EC4`) below the price: `S_NO` (`FUN_8009ccbc`: sound 10).
Otherwise, by type:

- **0** (and any type not below): leave the shop — the call returns 1.
- **1**: a key if keys (`+0x1EB8`) < 9 (`r13-0x7254`), else refused.
- **3**: a potion if potions (`+0x1EBC`) < 9 (`r13-0x7258`), its kind
  stored at `+0x3300` + count; else refused.
- **5–8**: 10 bought points (`FUN_8007f0a8` strength, `FUN_8007ef94`
  speed, `FUN_8007f04c` armour, `FUN_8007eff0` magic: the class record's
  float `+= 10` and the stats again), and this visit's counter
  (`+0xA70` strength, `+0xA7C` speed, `+0xA74` armour, `+0xA78` magic)
  `+= 10`.
- **17**: heals by the amount (`FUN_80078474`).
- **the rest** grant a power: `FUN_8007ee10(amount, duration, player,
  subtype, value)` — a held slot, as a pickup's ([powers.md](powers.md)):

  | type | amount | duration | subtype | value |
  | --- | --- | --- | --- | --- |
  | 2 | 0 | 90 | 5 | `0x200000` |
  | 4 | 0 | 30 | 9 | `0x100` |
  | 9 | 0 | 30 | 6 | `0x20000` |
  | 10 | 0 | 45 | 9 | `0x80` |
  | 11 | 0 | 30 | 5 | `0x20000000` |
  | 12 | 0 | 45 | 5 | `0x80000` |
  | 13 | 3 | −1 | 5 | `0x10000000` |
  | 14 | 0 | 15 | 6 | `0x400000` |
  | 15 | 0 | 15 | 6 | `0x200000` |
  | 16 | 0 | 20 | 6 | `0x110000` |
  | 18 | 0 | 60 | 9 | `1` |
  | 19 | 0 | 30 | 9 | `0x100` |
  | 20–23 | 0 | 90 | 5 | 1–4 |
  | 24 | 5 | −1 | 5 | `0x100000` |
  | 25, 26, 27 | 5 | −1 | 9 | `0x10`, `0x40`, `0x20` |
  | 28 | 4 | 40 | 9 | `0x10000` |
  | 29 | 0 | 15 | 9 | `0x200` |
  | 30 | 0 | 15 | 9 | `4` |
  | 31 | 0 | 30 | 6 | `0x10000` |
  | 32 | 0 | 120 | 9 | `2` |
  | 33 | 0 | 15 | 6 | `0x2008` |
  | 34 | 0 | 45 | 5 | `0x400000` |
  | 35 | 0 | 120 | 6 | `0x80000` |
  | 36, 37, 38 | 0 | 120 | 9 | `0x200000`, `0x400000`, `0x100000` |
  | 39 | 0 | 25 | 9 | `8` |

  (durations `r2-0x53e4`… in seconds; types 2, 4, 16, 28, 29, 34 and 39 are
  in no record.)

Bought: gold − price, and — unless it's EXIT (cursor 0) — `S_PICKUPMAGIC`
(`FUN_8009c870`: sound `0x26`) and the stats again (`FUN_8007c4f0`).
Refused: `S_NO`.

### Selling (X)

Only an item the hero owns (below). Keys − 1 or potions − 1 (the last
potion); a power: its slot (from `0x80122754`, below) emptied — time 0,
subtype 0, value 0, amount 0. Gold + price × 3 ÷ 4 (truncated),
`S_PICKUPMAGIC` and the stats again; nothing sold: `S_NO`. The stat points,
health, growth (19) and shrink (29) can't be sold.

**Owned** (flag 4, `FUN_8009ba60`/`FUN_8009bb04`/`FUN_8009ba9c`): a key or
potion while holding any; else the type's entry in `0x80122754` ({subtype,
value} by type: those of the grant table above, except 16, 19 and 29,
which have none) found in a power slot by `FUN_8009bb04`. That search walks
the 11 slots (`+0x130`, 16 bytes: f32 time, i32 subtype, f32 amount, u32
value) comparing subtype and value, but its pointer only moves on past a
slot whose time is above 0: from the first slot that is empty, or holds a
counted power (time −1: the hammer, the super shot, the breaths), it keeps
looking at that slot and finds nothing. So a counted power is never owned,
and nor is any power in a slot after an empty or counted one.

**Buyable** (flag 2, `FUN_8009b928`): gold ≥ price and, for a key, keys <
9; a potion, potions < 9; a stat, that raw stat < 999 (`r2-0x5368`);
health, health below its maximum.

An item neither owned nor buyable is **skipped** (`0x8028a300`): the cursor
passes over it, and it's drawn faded. The flags are worked out as the
screen is built and after every purchase or sale (items 1 on; EXIT never
skipped).

### The cursor (`+0xA68`)

Each frame, before buying: down (`FUN_800313d8`) moves to the next item,
past the list's length back to EXIT (0); up (`FUN_80031430`) to the one
before, from EXIT to the last item that's affordable or owned (the length
becomes that index + 1); either is repeated while the item it lands on is
skipped. The first move of a frame plays the cursor's tick: down
`S_SECRETCLOCK2`, up `S_SECRETCLOCK1` (`FUN_8009e6f4`: `r13-0x7cd4` =
`0x18`, `0x17`). The length starts as the number of items the hero can
afford. B (`FUN_8003105c`) jumps to EXIT. A and X only count while the list
is still. After a purchase or sale the bought row flashes red for 30 fields
(`0x8028a700`) and the cursor moves up while its item is unaffordable or
skipped.

The game also keeps a window "top" (`0x802891c0`) and the cursor's row in
it (`0x802891b0`) — 7 rows, scrolling at rows 3–4 — but the list's drawing
doesn't use them; the top only matters when a purchase moves the cursor
above it (the length is then recomputed, as for up from EXIT).

### The list (`FUN_8009b018`, `FUN_8009b65c`)

Each item's place: an icon (its sprite has a texture) adds 24, its text
(lines × trunc(32 × 0.5 × scale)) + 16. The list is laid out about the
cursor's item: that item at y 148, the middle of the window 72–224
(`r13-0x7cfc`, `r13-0x7cf8`) — but the first item no lower than 72, and
then the last no higher than 224. Every frame each icon moves toward its
place, when more than 1 away, by the scroll speed × fields: 2 a field
for a move, one more each further move while it still moves (the speed is
kept while anything moves, and A and X wait for it to stop); EXIT from B,
a wrap and the first frame jump straight there.

Each item, at its icon's y:

- the icon at (col + 20, y), its size;
- the price `"B:%d"` at (col + 58, y + 12) — owned: at y − 6, with the sale
  price `"S:%d"` (price × 3 ÷ 4) at y + 12 (`font32` × 0.5, a price of 0
  isn't shown);
- the text centred on the column from y + 32 (y + 12 without an icon),
  `font32` × 0.5 × the record's scale, line by line;
- the cursor's item shimmers (`FUN_8001eb80`, `FUN_8001ea94`); the others
  are black, red while flashing.

Outside the window an item fades: above 72 by (72 − y) × 510 / 64 (of 255,
the game's transparency, `r13-0x7d00` = 64), gone above 8; below 224 the
same down to 288. A skipped item inside the window is drawn at
transparency `0xA0`. With an item that isn't skipped above the window,
`MORE_UP` at (centre − 32, 32); below it, `MORE_DOWN` at (centre − 32,
280).

**The gold heap** (`FUN_800994fc`): `SHP_GOLD` stays up behind the scrolls,
its height 20 + gold × 188 ÷ (gold on entering + 1): spending lowers it by
1.5 a field, a sale raises it at once; with no gold it's hidden.

## The inventory (`FUN_8006c64c`)

The hero's quest pieces, in its column (x from the column; sprites at depth
3, `r2-0x6360`, at their textures' sizes), from the class record:

- `WINDOW_EMPTY` (64 × 256) at (0, 0), and in it the shards' pieces won
  (`FUN_800a1d98`: bit n of the record's realms beaten in the quest order
  `+0xDD4`, or the session's `+0x1EC8`): `LITCH_PIECE` (0, 86),
  `DRAGON_PIECE` (0, 124), `CHIMERA_PIECE` (0, 167), `PLAGUE_PIECE`
  (22, 131), `DRYDER_PIECE` (0, 106), `GENIE_PIECE` (25, 72),
  `YETTI_PIECE` (0, 148), `WRAITH_PIECE` (0, 63) for bits 1–8 (G, B, A,
  K, D, C, I, J; table `0x8011d0f0`);
- the legendary items by realm id (`FUN_800a1cb0`: bit n of `+0xDD8`),
  `<name>` held, `<name>_EMPTY` not: `SCIMITAR` (96, 32), `ICE_AX` (76, 32),
  `LAMP` (94, 58), `BILLOWS` (77, 56), `SOUL_SAVIOR` (98, 82), `BOOK`
  (54, 32), `FIRE_SCROLL` (56, 80), `LANTERN` (77, 81), `JAVILIN` (54, 56)
  for 1–5, 7, 9–11 (`0x8011d2c4`, `"%s%s"` + `"_empty"`);
- the gargoyle pieces `FANGS` (56, 116), `FEATHER` (56, 140), `CLAW`
  (56, 164) with `"%d"` / `"/%d"` — held (the record's `+0xDE8` i16s; below
  0 — the section open — or above, the need) and the need 12, 20, 28 —
  at x + 30, y + 2, `font32` × 0.5, white: the count right-aligned to the
  slash;
- the crystals `ORANGE_CRYSTLE` (6, 200), `RED_` (6, 216), `PURPLE_`
  (6, 232), `CYAN_` (6, 248), `GREEN_` (64, 200), `YELLOW_` (64, 216),
  `WHITE_` (64, 232), `BLACK_` (64, 248) for counters 1–8 (`+0xDF0`), counts
  the same way at x + 28, `font32` × 0.35, needs 15, 100, …, 250
  (`0x8012443c`).

Opening (`0x80273fe0` = 0), the timer `0x80273ff0` counting fields to 120:
with `t` = timer ÷ 120, each sprite circles in from 60 × (1 − t) away
(the angle `(x + y + trunc(180 t³)) mod 60` steps of 6°, tables
`0x8011cd30`/`0x8011ce20`), at (2 − t²) × its size, its transparency
255 × (1 − t²); the counts' 255 × (1 − t). Open (1): in place. A
(`S_OPTMENUSEL`) at any time closes it (2): over 15 fields each sprite
flies out from (64, 180) — moved 10 × (its offset from there) × t, × (1 +
5t) its size, transparency 255 × t — and the screen goes on when the
timer passes 14. The title is `"Inv"`; the A button (16 × 16) at
(col + 16, 280) and `"Continue"` (`font32` × 0.5) at (col + 40, 280) show
except while closing.

## The final stats (`FUN_80098808`)

After levelH4, in place of the shop: centred on the column, white
`font32` × 0.48, each line shimmering for its 60 fields and its value
(shimmering) from then on:

| fields | label (y) | value (y) |
| --- | --- | --- |
| 91–149 | `"Enemies Killed"` (60) | monsters killed (`+0xC10` + class × `0x1C`) (78) |
| 151–209 | `"Generators"` (98), `"Destroyed"` (116) | generators (`+0xC20`) (134) |
| 211–269 | `"Gold Found"` (154) | gold found (`+0xC24`) (172) |
| 271–329 | `"Total Playtime"` (192) | `"%d Days"`, `"%d Hours"`, `"%d Minutes"` (210, 228, 246) of the play time (`+0xC28`, fields: 5,184,000 a day) |

and `"Final Stats"` shimmering at y 32 (× 0.56); after 329 fields A
(`S_OPTMENUSEL`) ends it; the prompt as the other pages'.

## After the screen: the select screen in mode 2 (`FUN_8008ffec(2)`)

Every player record in state 5 (the ones who took part) goes to state 2
with the select step `+0x3338` = 1: **the character menu** in its column
(`0x80120fd4`: Save `0x3EA`, Change `0x3EB`, Load `0x3E9`, Quit `0x3EC`,
Done `0x3ED`), the cursor on Done. Players not in the game can join in
their columns as on any select screen. Done (`S_OPTMENUSEL`) makes the
player ready (state 3); when every joined player is ready the game starts
the tower (`FUN_80053530`). After levelH4 (`r13-0x724c` = 8,
`r13-0x7250` = 3) Change, Load and Quit are disabled. (Mode 1 is the
Tower Menu's Manage Character, mode 0 the title's Start.)

## Here

`shop.rs`: `ShopScreen` (a resource) with `open`, the per-frame `tick`
and the drawing; `ShopData` (the item table, the classes' `PDAT`, the
levels' full marks and `ENGLISH.ROM`) loads at start-up. The rules are pure
functions with tests: the item table and its effects, buying and selling on
a `PlayerState`, owning, the tally's heights and order, raw stats, the
level from experience, the list's layout.

Stand-ins and differences:

- `PlayerState` has no bought stat points, kill counts or final-stats
  totals; the screen takes them when it opens (`ShopOpen`) and gives the
  points back (`ShopScreen::bonus`). The level-start copy stands for the
  game's kept copy.
- The inventory reads the hero's quest progress as it is (the game reads
  the class record, merged from the session at each save).
- The three heaps share a depth in the game; they're drawn tallest at the
  back here (the order among equal depths isn't traced).
- Icons not in `SELECT/` (Hand of Death, Health Vampire, Mikey) come from
  `INVENTORY/`, the in-game power icons the game loads with the shop's
  items; whether that set is still loaded during the screen isn't traced.
- Not done: the loading screen and the tower loading behind
  (`TRANSITION_SCREEN`; the front end's), the HUD panels' text during the
  screen (`FUN_8007496c`; the panels' backing is drawn), the borders' glow
  flag, `FUN_8007a4f4`'s health restore, the start entry nearest the
  camera when the tower starts again.
