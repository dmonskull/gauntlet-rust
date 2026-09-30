# The front end: fonts, the 2D screen, menus, death and game over

Implemented in [`gdl-formats/src/font.rs`](../crates/gdl-formats/src/font.rs)
(font files and slots), [`gdl-game/src/font.rs`](../crates/gdl-game/src/font.rs)
(the 2D layer and text) and [`gdl-game/src/frontend.rs`](../crates/gdl-game/src/frontend.rs)
(title, character select, pause menus, death, GAME OVER). The text ROM
chunks the fonts and strings come from are in [chunk-files.md](chunk-files.md).

Verified: all 16 `FONTS/*.FNT` files parse; the 12 whose texture is on the
disc have every glyph inside it (`GAUNT` has no texture anywhere; the three
`KANJI*` pages are 32×32 placeholders on the US disc).

## Fonts

### `FONTS/<name>.fnt`

Loaded by `FUN_80020e38(slot, name, space_width)` as `"fonts/%s.fnt"`,
byte-swapped, and turned into a blit font by `FUN_800b5ed8` (its assert:
`"MBNewFont: MBNewBlit failed"`). Little-endian:

| offset | field |
| --- | --- |
| `0x00` | name pointer (0 on disc; the loader stores the name) |
| `0x04` | flags: low byte = glyph height (every glyph); `0x100` = digits-only font |
| `0x08` | glyph pointer (0 on disc; the loader points it at `0x0C`) |
| `0x0C` | 16-byte glyphs `{u32 code, u32 width, u32 x, u32 y}` up to one with code 0 |

`FUN_800b5ed8` builds a table indexed by code (size: highest code + 1), each
entry a blit of the rectangle `x..x+width × y..y+height` of the texture
with the font's own name (looked up with `FUN_800b8324`; UVs divide by the
texture's size from its name record). Glyph `n` is on the texture `n /
page` bindings after the named one (`FUN_800b2148`); only the Japanese
fonts have more than one page.

Drawing (`FUN_800b52f0` width, `FUN_800b55dc` draw): the pen moves by each
glyph's width × x-scale, no extra spacing; a character with no glyph is
skipped, except a space, which moves by the slot's space width
(`0x8029dda8[slot]`); `*` followed by `A`–`Z` is an inline button icon one
line high (`*D`..`*X` pick the `BUTTON_*` textures in `r13-0x6cb0`..`-0x6ccc`;
not drawn here yet). In a digits-only font (none on the disc) other ASCII
is skipped, `.` and `-` draw as `:` and `;`, and a byte ≥ `0x80` is a
colour byte followed by the character. Line step = height × y-scale;
multi-line text splits at `\n`, at most 16 lines (`FUN_80020c40`). A text
x below 0 centres the string on `-x` (`FUN_800b5d78`). Text is drawn after
all sprites of the frame. Colours are PS2-style (`FUN_800b5b58`: halved,
`0x80` = full), alpha is a transparency (`FUN_800b5bb0`: `0x80 − v/2`).

The glow draw flag `0x4000` grows each glyph quad by `r13-0x7c4c` (2.0)
screen pixels on every side; text drawn with it uses the `FONT32_GLOW`
texture (`r13-0x771c`), which has `FONT32`'s layout.

### Slots

`FUN_80020d5c` loads slots 1–12 at boot from the tables at `0x80118538`
(names) and `0x8011856c` (space widths); slot 0 isn't read from disc — its
font is built into the executable at `0x80237420`, byte-for-byte
`FONTS/FONT8X8.FNT`. Slots 10–12 read up to 256 glyphs (`0xD2` per page
for 10 and 11, 100 for 12), the rest 128. Slot 13 is loaded per screen
(`FUN_80020cf8`: `credits`, `gaunt`, `shopatt9`).

| slot | font | space | texture (in `STATIC/` unless noted) |
| --- | --- | --- | --- |
| 0 | `font8x8` | 8 | `FONT8X8` 128×64 |
| 1 | `8Hifonts` | 8 | `8HIFONTS` 128×128 (has ©, ™, ® as codes 3, 0x12, 0x14) |
| 2 | `bars` | 4 | `BARS` |
| 3 | `arrows` | 8 | `ARROWS` |
| 4 | `score` | 9 | `SCORE` |
| 5 | `scoratt` | 12 | `SCORATT` |
| 6 | `font32` | 16 | `FONT32` 256×256; also `FONT32_PARCH`, `FONT32_GLOW`, `FONT32GAR0..5` (128×128) with the same layout |
| 7 | `initials` | 12 | `INITIALS` 128×256 |
| 8 | `scoratt8` | 8 | `SCORATT8` |
| 9 | `namefont` | 8 | `NAMEFONT` |
| 10–12 | `kanji10a`, `kanji10b`, `kanji20a` | 10, 10, 20 | 32×32 placeholders |
| (13) | `credits` / `gaunt` / `shopatt9` | 8 | `CREDITS/CREDITS`; none; `SELECT/SHOPATT9` |

A string group's `font` number indexes the text ROM's `FONT` chunk, whose
names map to slots ([chunk-files.md](chunk-files.md)); `ENGLISH.ROM`'s
first font name matches no slot, so those groups draw in `font8x8`.

The runtime draws a missing or unreadable font in Bevy's built-in font at
the same height (`FALLBACK_HEIGHT`).

## The 2D screen

Everything 2D is laid out in a 512 × 384 screen (text y is rejected past
`0x17F`, x past `0x1FF`), shown here letterboxed to the largest 4:3 area of
the window over the 3D view. Sprites (`FUN_800b32bc(0, texture, x, y, w, h)`,
`-1` = the texture's size) and text are re-issued every frame; this
runtime does the same with a draw list (`Draw2d`) shown by a pool of Bevy
UI image nodes. UI textures are sRGB, clamped, bilinear.

## Game modes

`r13-0x7380` is the game's mode: `0x8000`–`0x8009` the attract loop
(`FUN_80014ac0` steps through the table at `0x80117bd8`: movies
`0x8001/2`, scroll screens `0x8004`, demo play `0x8003/6/8`, credits
`0x8000`, title `0x8009`), `0x400B` character select, `0x400C` → `0x4010`
play, `0x4014` GAME OVER, `0x4016` final stats, dispatched every frame by
`FUN_80054244`.

## Title (`0x8009`)

`FUN_80013b30` (init: `"Load titlescreen."`, `"Init titlescreen done."`),
`FUN_800137bc` (update), `FUN_80013d20` (attract timer):

- Loads `TITLE/`; draws `TITLE00`..`TITLE03` (`"%s%02d"`) at the
  positions in `0x80117c90`: (0,0), (256,0), (0,256), (256,256) — a
  512 × 384 picture.
- `GLOWCROP_00` sprite at (192, 0), 128 × 128, stepping through 10
  bindings, one per 4 fields; its transparency falls from 255 to 0 over
  the first 60 fields.
- `"Press Start"` via `FUN_8001eb80` at x −256 (centred on 256), y 320:
  the shimmer text — `font32` (`r13-0x7f2c` = 6) at scale 1, a glow pass
  in `0x8200EA` (`r13-0x7f3c`) whose alpha follows a triangle over
  `r13-0x7f34` = 40 fields up and down plus `r13-0x7f30` = 5 at rest
  (half to full opacity), then the same text in white.
- Start opens the title menu (`FUN_80070ba4` → menu `0x8011d684`). After
  30 s without input (`r13-0x78a0` = `0x708` fields) the attract loop
  resumes.
- Choosing Start sets `r13-0x7014`; the title then shows `"Loading..."`,
  loads the realm data (`FUN_8005a094`, which also sets the tower level
  `r13-0x72b0` = `0xD00`) and enters select with `FUN_8008ffec(0)`.

## Menus

`FUN_80073b20(menu, player)` opens a menu (stack of up to 4 at
`0x80274820`, depth `r13-0x7dcc`; only the top one is drawn),
`FUN_80073c0c` lays it out, `FUN_8007268c` reads input (returns the
chosen item's id, −1/−2 for back, 3/4/7/8 for left/right/up/down),
`FUN_80072a38` draws it, `FUN_80070c24` dispatches by menu id.

Menu record (words): 0 id, 1 title scale (1.2), 2 title, 3 items' x
(≥ 0 left edge, < 0 centred on −x), 4 items' y (−1 centres the list on
192), 7 item list, 8 flags, 9 button-hint y, 10–12 colour triplets
(plain, selected-from, selected-to), 13 item scale, 14 arrow model
(`ICON_ARROW`), 16 arrow x offset (−16), 17 panel texture (`SCROLL_A`),
18–21 panel x (−1 centred), y, w, h, 22–26 logo sprite (`LOGO_BURN1`, x,
y, w, h). Items are `0x24` bytes: text, id, extra space below, value text,
…, word 8 < 0 = disabled (drawn at half alpha, skipped by the cursor).

Flags: `0x1` "Back" hint (`BUTTON_TRI`), `0x2` "Select" (`BUTTON_X`), `0x4`
"Change", `0x8` "Center" — spaced 512 / (n + 1) apart at the hint y,
`font32` × 0.667 (`r13-0x7dbc`); `0x20` fade in/out over 30 fields
(`r13-0x7dc8`); `0x40` letters in `FONT32_PARCH` (plain items then white);
`0x80` plain items flicker through `FONT32GAR0..5` from field 10 to 22
after opening; `0x10` "Player %d" label.

The menu font is `r13-0x7df8` = 6 (`font32`); line step 32 × item scale.
The selected item is drawn twice: `FONT32_GLOW` in the selected-to colour
with the glow flag and a pulsing alpha (`r13-0x7dc4` = 1: glow mode; period
`r13-0x7e00` = 40, `r13-0x7dfc` = 5), then white `FONT32`. The title is
centred on the panel at panel y + `r13-0x7dec` (58), white. The logo steps
through `LOGO_BURN1..5`, one per 8 fields.

| record | id | title | items (id) | notes |
| --- | --- | --- | --- | --- |
| `0x8011d684` | 2 | — | Start (`0xB`), Options (`0xC`) | title menu: x −256, y 304, no panel; colours (180,50,10) / white / (130,0,234) |
| `0x8011d820` | 5 | Options | Audio (`0x11`), Game Options (`0x10`), Compass (`0x14`), Controls (`0x15`) | |
| `0x8011d9e0` | 3 | Tower Menu | Settings (`0xC`), Manage Character (`0xD`), Shop (`0xE`), Inventory (`0xF`), Quit Game (`0x25`) | pause in the tower (current level = `r13-0x72b0`) |
| `0x8011dcac` | 4 | Game Menu | Settings (`0xC`), Quit Level (`0x26`) | pause elsewhere; Quit Level disabled in the secret realm |
| `0x8011db58` | 6 | Settings | Audio, Compass, Controls | from the Tower Menu |
| `0x8011de00` | 7 | Settings | Audio, Controls | from the Game Menu |
| `0x8011df54` | 9 | Game Options | Difficulty (`0x12`), Multiplayer Mode (`0x13`) | |
| `0x8011e190` | 8 | Audio | Music Volume (`0x17`), Sfx Volume (`0x18`), Mono (`0x19`, value `Stereo`) | y 108, item scale 0.8, 52 below each slider |
| `0x8011e728` | `0xC` | Compass | Hide (`0x20`), Show (`0x1F`) | |
| `0x8011e904` | `0xD` | Controls | Style, Rumble Feature, Auto Aim, Auto Attack (`0x21`–`0x24`) | |
| `0x8011e2e4` | `0x13` | Quit Game? | No (−1), Yes (`0x25`) | panel (96, 64) 320 × 220 |
| `0x8011e0a8` | `0x14` | Abort Level? | No (−1), Yes (`0x26`) | |

The in-game menus (Options and after) share flags `0x4F3`, panel
`SCROLL_A` at (16, 8) 480 × 360, items at x 128, colours (92,26,3) /
white / (130,0,234), hints at y 304, logo at (290, 142) 224 × 172.

Volume sliders (`FUN_80072330`, `FUN_80072454`): under the item, at the
items' x, a bar `r13-0x7dac` = 264 wide: `MARKER_LEFT` at x − 52,
`MARKER_RIGHT` at x + 264 − 24, `EMPTY_BAR` 264 × 32 at y + 11, `PINK_BAR`
(264 × value / 255) × 32 at y + 15, the `slider` knob at the fill's end − 20,
y + 2; items not selected are dimmer. Left/right move the value 0–255 by
the fields elapsed while held.

What the choices do (`FUN_80070c24`): Start → select; Options / Settings /
sub-menu items → the sub-menu; Manage Character → back to the select
screen at the character menu (`FUN_8008ffec(1)`); Shop / Inventory →
`FUN_8009a140(1/2)`; Quit Game → Yes sets `r13-0x7004`, which switches to
GAME OVER; Quit Level → Yes sets `r13-0x7008`, which takes every hero out
of the level (`FUN_80078de8`: state `0xB`) so the level ends and the party
returns to the tower.

## Character select (`0x400B`)

`FUN_8008ffec` (enter), `FUN_8008be04` (update, per player),
`FUN_8008dd2c` (per-player legends), `FUN_8008fc88` / `FUN_8008f828`
(class card). The tower is loaded behind it (`FUN_80053b9c`,
`"LoadTowerAndSelect Timeout"`); the 2D art covers y 0–320 and the tower
shows below.

Four 128-wide columns at x `0x80120e50` = 0, 128, 256, 384, each with 11
sprite slots at `0x80120e70` (x, y, depth): 0 `S1_PLYRn` (0,0), 1
`S2_PLYRn` (0,256), 2 `S12_WEAP_<class & 7>` (0,0), 3/4 `S12_<class>_<colour>`
(0,28), 5/6 (0,160), 7 `SELSCRN_QUESTMARK` (64,160), 8 `<class>_NAME`
(8,272), 9 `S1_BORDER` (0,0, glow flag), 10 `S2_BORDER` (0,256). A player
not in the game shows slots 0, 1, 9, 10 (`FUN_800903a4`).

Per player (`+0x3338` step, record at `0x802754c0 + p × 0x335C`):

| step | what | legend (`font8x8` × 1.2 at `r13-0x7d20`, 19 px buttons) |
| --- | --- | --- |
| 0 | menu `0x80120f68`: New (1000), Load (1001, disabled without saves) | Select, Back |
| 3 | name entry (`FUN_8005a864`): "Enter / Your / Name" (`font32` × 0.8) at y 64, 90, 116 | U/D Change, L/R Edit, Accept, Cancel at y 180, 200, 220, 240 |
| 4 | class and colour | L/R Change at y 232, Select at 252 |
| 1 | menu `0x80120fd4`: Save, Change, Load, Quit, Done | |
| 2 | menu `0x801210ac` Yes / No, "Character / Not Saved / Quit Anyway?" | |

The select menus use the record at `0x801214b8`: centred on the column
(x = −(column + 64)), y −128 (centred on 128), scale 0.667, no arrow,
colours (180,50,10) / white / (130,0,234).

Name entry: up/down step the letter through `0`–`9`, `@`, `A`–`Z`, `_`
(wrapping); A takes it — on `@` the name is done; six letters also end it;
L/R rub out the last letter; B cancels. Letters are drawn in `initials` ×
0.9 at y 340 from x = `r13-0x7ec8[p]` − 34 (8 for player 1), 18 apart, in
the player's colour (`0x8011c2e8`: yellow, sky blue, red, green); the
pending letter blinks grey/white every 16 fields, empty places are `_`.
An empty name becomes one of 16 (`0x8011c2a8`: LARRY, PELE, CHUCK, …).
The finished name blinks for 60 fields (`initials` × 0.75, centred).

Class: left/right step through the 16 classes (the 17th, `SUM`, only when
unlocked — `FUN_8008db78`); classes 8–15 need the character's unlock bit
(`+0xA8C`), otherwise they show `S12_<class>_SHADW` and the question mark
and can't be picked. Up/down cycle the colour (`YEL`, `BLU`, `RED`, `GRE`).
A new character starts as class 6 (Sorceress): `FUN_80079b34` sets the
class from a loop counter that always ends at 9, which its switch maps to
6. The colour byte of a new record is 0 (yellow).

Class card (`FUN_8008f828`): `ATTS_DESC` labels (`font32` × 0.5)
right-aligned at column + 81, y 160 + 16 × i; values `%03d` in `initials`
× 0.5 at column + 84, y 162 + 16 × i: class start value (`PDAT +0x28`,
`+0x30`, `+0x38`, `+0x40`) + per-character points + 5 × (level − 1), capped
at 999; the highest glows with `ATT_GLOW` behind it. `"Level %d"` centred
at y 292.

Picking an open class makes the player ready (state 3); when every joined
player is ready the game starts (`FUN_80053530`) in the tower level
`0xD00` — realm 13 (`L`), level 0: **`levelL1`**. Backing out of the
New/Load menu with nobody else joined returns to the title.

## Saving

The game keeps a single save file, `Gauntlet Save Data`, on the memory
card in Slot A, holding every character's record (`MEMCARD.C`; the
strings "The Gauntlet Dark Legacy Save File / on the Memory Card in Slot A
is corrupt", "Gauntlet will now create a Save File.", "Save File
Created!"). At boot it checks the card and makes the file when it's
missing; without a usable card saving is disabled, and New/Load's Load is
disabled while there are no saved characters. The character menu (step 1
above) saves the joined hero's record (Save) or swaps in a saved one
(Load); Quit with an unsaved character asks "Character / Not Saved / Quit
Anyway?" (step 2).

In this rewrite (`saves.rs`) the file is `characters.ron` in the
platform's data folder (macOS `~/Library/Application Support/`, Windows
`%APPDATA%`, else `$XDG_DATA_HOME` or `~/.local/share`, then
`GauntletDarkLegacyRust/`); `GDL_SAVE_DIR` puts it elsewhere. A record
holds the name, class and colour, level, experience, health, gold, keys,
potions, runestones, realms beaten and the quest's progress (crystals,
gargoyle items, legendary items, levels entered). Saving a name that's
already there replaces it. Load lists the saved characters ("NAME Lv n",
the first 10, `font32` × 0.5) in the player's column; choosing one
makes a fresh hero of its class and colour with the record laid on it,
then goes to the tower. Quit asks first only when the hero differs from
its saved record. A hero started from the command line (`--level`,
`--character`) is named LARRY, as a blank name entry takes one of the
sixteen.

## In-game HUD

Implemented in [`game_hud.rs`](../crates/gdl-game/src/game_hud.rs); the old
text line (`status_hud.rs`) only shows with F1 or `GDL_DEBUG_HUD=1`.

Each player has a 128-wide panel along the bottom of the 2D screen, at x
`0x8011fa00[player]` = 0, 128, 256, 384 (centres `0x8011fa08` = 64, 192,
320, 448). `FUN_8007bb40` builds all four at level load (`FUN_8007bca4`
per player) and loads `KEY_ICON` and the potion icons (`0x8011f49c`:
`POTION_ICON_RED`, `RED`, `BLU`, `YEL`, `GRE`, indexed by the potion's
kind). `FUN_80075cac` sets the panel's textures from the player's state
(`FUN_8007605c`: 0 not in, 1 playing, 2 ready, 3 naming, 6 picking a
class, 10 out of the level); `FUN_80074f0c` draws the per-frame parts
(from the player update); `FUN_80074cf8` shows the legendary-key and
rune rows; `FUN_80074850` hides a panel. Sprites are kept from frame to
frame (shown or hidden); text is issued anew every frame.

Sprites (x from the panel's left; depth 64000, `r2-0x6010`, unless
noted — lower is in front):

| sprite | position, size | texture |
| --- | --- | --- |
| `0x80275460` | (0, 304), 128 wide | `S3`; `BK_RUNE_STONE_02` while playing |
| `0x80275464` | (0, 320), 128 wide | `S4` tinted `0x8011f9a0[colour]` (joined) / `0x8011f9b0` (not); `S4_<class>` while playing |
| `0x80275468` | (0, 320), 128 wide | `S4_FRAME` |
| `0x8027546c` | (0, 344), 148 wide | (not set here) |
| `0x80275470` | (6, 357), 20 × 20, depth 63990 | `coin` |
| `0x80275474` | (61, 357), 20 × 20, depth 63990 | `heart` |
| `0x802753a0` × 12 | (15 + 8i + i/3, 306) | `SM_RUNE_<blu,red,yel,gre>_<01..03>`, shown for bit i of the runestones held (`+0x1ECA`) |
| `0x80275320` × 8 | (12 + 12i, 300), the texture's size | `SM_KEY_<c>`, c = `0x8011fdc0[i]`: blu, red, yel, gre, gre, red, yel, blu; the legendary-key row (below) |
| `0x802752e0` × 4 | (26, 322 + 3i) | — |
| `0x80275270` × 7 | table `0x8011f414` (name, x, y, depth; stride `0x14`): `trbo_full_new` ×2 (0, 304), `trbo_glint`, `turbo_glow_new`, `black_bar` ×2, `trbo_gleem1` (80, 310) | the turbo meter |
| `0x80275260` | centre x − 14, y −323 (off screen), hidden | `BTMBK_LEVL`: the level plate, shown only in the mode `r13-0x7338` |
| `0x80274894` | (8, 340), 16 × 16 | `RUNE13` (`r2-0x5d68`): the thirteenth runestone held (below) |
| `0x802748a4` | (104, 338), 16 × 16 | `QUEST_ICON` (set by `FUN_80075cac`; each boss level's `ITEMS/<level>` has its own, the realm's legendary item — levelB6's is the ice axe): that item brought to the boss (below) |

Text (`FUN_80074f0c`, in the player's colour `0x8011f990`: `0xFFFF80`,
`0x87CEEB`, `0xFFC0E0`, `0x80FF80`, unless noted):

- the name (`+0xA80`) in `initials` × 0.667 centred on the panel at y 339;
  `"LV %d"` in `8Hifonts` (white) centred at y 326;
- gold (`FUN_8007572c`, `+0x1EC4`, capped 99,999) `"%d"` in the `score`
  font right-aligned to x 60, y 359; health (`+0x1EB4`, shown ≤ 9999)
  right-aligned to x 116, y 359;
- keys (`+0x1EB8`) when any: `KEY_ICON` at (8, 323) and the count in
  `score` × 0.8 at (26, 327); potions (`+0x1EBC`) when any: the last
  potion's icon at (102, 323) and the count at (92, 327);
- a player out of the level (below): "Wait In Tower" / "Quit Game", or
  "IN TOWER".

**The legendary-key row** (`FUN_80074cf8`, every frame from
`FUN_8007496c(-1)`): `r13-0x6fdc` is set to 300 as a level starts, except
in the secret realm (`FUN_80053530`, realm 12), and when a runestone none
of the players holds is picked up (`FUN_8005de3c`, class 10); a new mode
(`FUN_80015084`), character select (`FUN_8008ffec`) and the HUD's build
(`FUN_8007bb40`) zero it. While it is at least 1 and no menu is open
(`r13-0x7060`, the open menu's id from `FUN_80073b20`, is 0) it counts
down by the fields elapsed (not below 0) and, for each player once a
frame (`+0x966` & 2, cleared by
`FUN_80054140`): in state 1, 2, 4 or 5 key i is shown for bit i of the
class record's `+0xDD4` — the realms beaten by their place in the tower's
order (`0x801244dc`: tower, G, B, A, K, D, C, I), merged in from the
session's `+0x1EC8` when the record is saved (`FUN_8007a9e0`) — and in
other states (dying, out) the row is hidden; in state 1 or 5 (outside the
attract loop) the runestone slots are set again from `+0x1ECA`. Otherwise
(run out, or a menu open, which also holds the count) every key is
hidden: the row shows for 5 s. The pickup count (`FUN_80074b08`: its icon
16 × 16 at (28, 288), "n/need" in `8Hifonts` × 1.5 white at (48, 292))
shows, and its 3 s (`+0x92C`) run, only in play (`0x4010`) with no menu
and `r13-0x6fdc` < 1: it waits for the row.

**The icons** (`FUN_80074f0c`): while the player's health (`+0x1EB4`) is
above 0 and no player menu is up (`r13-0x70b4`), `QUEST_ICON` shows in
play (`0x4010`) outside the tower while the player's `+0x834` ≠ 0 — 1 for
the first hero bringing the realm's legendary item as a boss level starts
(`FUN_80057020`; critters.md, "The boss intro"), 2–4 as the item is used
and the legendary weapon thrown, back to 0 when the weapon is released
(`FUN_80080d3c`) or the player goes out (`FUN_80078de8`) — and `RUNE13` in
modes `0x400F`–`0x4010` while the runestones held (`+0x1ECA`) have bit 12
(`0x1000`: the thirteenth, `RUNEE1`). With no health left (or that menu
up) the icons keep their look and the turbo meter's seven sprites are
hidden. The panel's rebuild (`FUN_80075cac` → `FUN_80074850`) hides both.

**Out of the level** (state `0xB`, `FUN_80074f0c`):

- In play (`0x4010`) with a player still playing (`r13-0x739c`: players
  in state 1 or 5, counted every frame by `FUN_80054140`) and the out
  player's `+0x3338` = 1 (set as it goes out, `FUN_80079094`): the A
  button's picture (`r13-0x6cb0`) at (6, 332) and B's (`r13-0x6cb4`) at
  (6, 352), both 14 × 14, "Wait In Tower" at (20, 336) and "Quit Game" at
  (20, 356), in `8Hifonts` × 1.2 (`r2-0x5fe0`), white; gold, coin, health
  and heart aren't drawn. In the player update (`FUN_8007692c`, state
  `0xB`) B quits (`FUN_80079418`) and A clears `+0x3338`: the player waits.
- Otherwise, outside modes `0x400D`, `0x4013` and `0x4017` (and unless
  `r13-0x70d0` or `0x80256f80` is set): the panel is rebuilt
  (`FUN_80075cac`, panel state 10: `S3` over `S4` in the joined colour,
  framed, coin and heart, no runestones) and "IN TOWER" is drawn in
  `8Hifonts` × 1.2 centred on the panel at y 340 in the player's colour,
  with gold and health (0: going out sets it, `FUN_80079094`); no name or
  level.
- With one player the prompt never shows (nobody is left playing). The
  hero goes out as its death ends; the player update then reports the
  level over, and play ends (`FUN_8009a140(0)`: mode `0x4012`, then the
  tower) once the voice queues are empty (below, "Death") — at the
  earliest the frame after; until then its panel shows "IN TOWER".

Here (`game_hud.rs`, player 1): the key row, the pickup count's wait, both
icons and the hidden turbo meter at 0 health as above. The key row reads
the hero's realms beaten (`PlayerState::realms_beaten` through
`quest::boss_marks`) rather than a copy saved with the record; a level
start is `LevelPopulation` changing, a new runestone a new bit of
`PlayerState::runestones`; the counts run on game time, which stops under
a message box (the game's box doesn't update the row either). A hero
holding `+0x834` = 1 is the boss intro's first state
(`CritterLevel::intro` = 1, `critters.rs`); the legendary weapon's throw
isn't done, so the icon goes when the item is used up (intro state 2).
The out hero (`Frontend::hero_out`, see "Death") gets the "IN TOWER"
panel: `S3`, `S4` in the joined colour, the frame, coin and heart, "IN
TOWER", gold and health, and the key and potion counts when it has any;
no runestones, key row, icons, turbo meter, pickup count, name or level.

Turbo meter (`FUN_80075828`): the shown value (`+0x82C`) moves toward the
meter (`+0x828`, 0–100) by the fields elapsed (down twice as fast). At
f = value × 0.01: below 0.4 the bar is `trbo_full_new` scaled to f / 0.4
of its width about its centre, tinted (v, v, 0) over the second bar in
black; to 0.99 it is (f − 0.4) / 0.6 wide in red (v, 0, 0) over a yellow
bar; full, red over yellow; v = 127 × fraction + 128. The `trbo_glint`
streak (128 × 16, depth 63997) always lies over the bars. When the shown
value moves into another band (`+0x3340` = 1, timer `+0x3344` = 0) the
`trbo_gleem1` sprite at (80, 310) steps through `TRBO_GLEEM1`…`5` and
back (frame = fields >> 2; 5–9 count down; `0x8011f488` holds the first
frame's texture at run time); reaching full (`+0x3340` = 2) the
`turbo_glow_new` sprite fades from opaque to clear and back over 120
fields (alpha 255 − x, x = fields × 512 / 120 folded at 256), over and
over while full (a full meter with nothing playing starts it again), and
both bars are red. While either plays the bars keep their last size and
colours.

## Death

The player record's state `+0xE8`: 1 playing, 8 dying, `0xB` out of the
level. While dying (`FUN_8007692c` case 8) the death animation plays;
then `FUN_80079094`:

- **In the tower** (realm 13): the hero is back at once (state 1), its
  record restored from the character's saved copy (`FUN_8007a738`).
- **Anywhere else**: the hero is out (state `0xB`, `FUN_800a169c` counts
  the death). When no hero is left playing the level ends; with everyone
  out the next level is the tower (`FUN_8009a140(0)`), and there
  `FUN_80053530` revives each out hero with `FUN_8007a5a8`: the record
  saved when the last level started is copied back.

**The wait for the voices.** The player update (`FUN_8007692c`) reports
the level over from the frame after the last hero goes out (in the frame
it goes out, its dying case keeps the level going), and when the heroes
leave through an exit or a boss level's countdown runs out (`r13-0x72f0`).
The play mode then ends the level only if `FUN_8001538c(1)` returns 0;
otherwise it tries again next frame, the level still running (the out
panel up). `FUN_8001538c` returns 0 at once if the audio has failed
(`r13-0x7860`); otherwise it steps the voice queues (`FUN_80015480`) and
returns 1 while either of them holds a line — with a limit of 50,000,000
tries ("Audio Play Timeout"), in effect none. (Its loop over the started
sounds' table afterwards changes nothing.) The next level's start waits
again: `FUN_80053530` → `FUN_800a097c` loops on `FUN_8001538c(1)` a frame
at a time, then `FUN_80015618` empties both queues and stops the started
sounds (`FUN_800166d8(0x1FFF)`).
Two more waits: a boss level's end sequence (step 9, `FUN_80019044`,
[critters.md](critters.md), "The end sequence") and a level's opening
(`FUN_8001a220` state 2, `FUN_8001538c(10)`; its state 0 queues two lines
named by the level's records, `FUN_8009f5bc` — not traced further, not
done here).

### The voice queues

`r13-0x781c`: two counts; `r13-0x7824`: each queue's first line's end (0
until it starts); lines at `0x8023cb58` + queue × `0x140`, 16 of `0x14`
bytes: sound id, volume, pan, priority, length (f32 fields). The clock is
the field counter `r13-0x6bb0`, milliseconds × 3 / 50 since it was
started (`FUN_800c84c0`): real time at 60 a second, running under the
message box and menus.

- **Appending** — `FUN_80015160(length, most wait, queue, id, volume, pan,
  priority)`; `FUN_80015124` is the same with queue 0. With 16 lines the
  line is dropped. Its start is now for an empty queue, else the first
  line's end (now + its length if it hasn't started) plus the lengths of
  the others; with most wait ≥ 0 it's dropped when that start is more than
  most wait × 60 fields away. A length ≤ 0 is the sound's catalog length
  (sound record `+0x14`, seconds; −1 for a loop) × 60 (`r2-0x7de0`). It
  returns the length, or 0 when dropped. It also writes the start into the
  sound record's `+0x18`; the start callback overwrites that with the
  actual start, and nothing reads it (every load of the catalog pointer
  `r13-0x7850` in the binary checked: only these two write it and the
  loader byte-swaps it).
- **Stepping** — `FUN_80015480`, every frame from the main loop
  (`FUN_80067f50`) and twice a frame in the message box's loop: for each
  queue with lines, a first line not started is played (`FUN_80015cac(−1,
  id, volume, 0, pan, priority)`) and its end set to now + length; a first
  line whose end has come is taken off (the next starts on the next
  step). It doesn't step while banks load (`r13-0x7800`) or a sound is
  being started (`r13-0x7804`). It returns 1 if either queue held a line.
- **Starting a sound** — `FUN_80015cac` sends the sound driver three
  words: the call's handle, volume << 16 | pan, and the priority (with a
  first argument ≥ 0 — the jukebox's — that argument `& 0x1FFF | 0x8000`
  instead). When the
  driver has started it, `FUN_80015f24` runs: the record's `+0x18` = now,
  and each voice the driver used gets a slot in the table `0x8023d4e8`
  (12 × `0x14`: id, handle, end = now + 60 × length, …). `FUN_80016358(id)`
  is "is it playing": a slot with that id whose end hasn't come.
  `FUN_80016558(id)` stops it: the voices in slots with that id, and a
  request still pending for it (a queued line not yet started isn't
  touched).
- **The priority** is the driver's, for voice stealing: a started voice
  keeps `priority << 16 | the call's own priority` (the bank call's last
  word, [audio-format.md](audio-format.md) "Calls"). A new sound takes a
  free voice (round-robin from the cursor `r13-0x68c0`); with all 12 busy
  it takes, walking from the cursor, the voice whose key is the highest
  seen so far if that key is still ≤ the new sound's priority
  (`FUN_800d350c`) — so only voices started at priority 0 can be taken,
  and a sound with nowhere to go isn't played. Callers' priorities: the
  announcer's lines 2, the heroes' eating and poison lines `0x42`, their
  steal lines `0x6E`, hurt cries `0x64`, the tower's chimes 10. Nothing
  in the queues themselves orders by it.
- **Queue 1, the announcer** (volume `0xE0`, the player's pan
  `0x80122a90` = centre), its most waits:
  - hints (`FUN_800a4268` → `FUN_8009c37c`), 0.5 s; with one player
    (`r13-0x7390` ≤ 1) four are sentences naming the hero (below):
    the name then `S_NOWIT` or `S_POJOVOX` (5 s), the name, `S_HAS`
    and `S_GAINEDLEVEL` (1 s) or `S_SHRINKVOX` (4 s);
  - the tower's unlocks (`FUN_8009bdf0`, `FUN_8009be58`: `UNLOCKSECTION`,
    `UNLOCKLEVEL` lines, queued before the message box opens), 10 s;
  - the tower wizard's pieces (`FUN_8009bec0`, `FUN_8009bd28`), 10 s,
    and ranks (`FUN_8009c5b8`: a sentence, 5 s);
  - the bosses' wizard's speeches (`FUN_8009bf48`), 10 s;
  - the random taunts (`FUN_8009bbb0`/`FUN_8009bc24`, from
    `FUN_800a11c4`), 0.5 s; `FUN_8009caec`/`FUN_8009cb38`, 10 s;
    `FUN_8009f2ec`, `FUN_8009f368` (from the pickups), 1 s; `FUN_8009f6d8`…
    `FUN_8009f7dc`, 3 s; `FUN_8009f8d8`/`FUN_8009f95c`, 10 s; the
    runestone count `FUN_8009f40c` and the level's names `FUN_8009f5bc`,
    no limit; the health warnings (`FUN_8009f82c` → `FUN_8009f9e0`:
    `S_BADLY`, `S_LIFEFORCE`, `S_NEEDSFOOD`, `S_ABOUT`).

  All of these but the wizards' (`FUN_8009bec0`, `FUN_8009bd28`,
  `FUN_8009bf48`, `FUN_8009bdf0`, `FUN_8009be58`) are refused while
  `r13-0x7790` ≥ 3: a boss level's end sequence from the wizard's
  appearance (it's reset to 0 by `FUN_80053d1c` and `FUN_80057020`).

  **Sentences** (`FUN_8009f9e0(most wait, before, player, line, after)`):
  up to five lines queued in turn — `before`, the colour's name, the
  class's name, `line`, `after` — the first with the most wait and the
  rest with none; when one is dropped the rest aren't queued. The name is
  `S_<colour><class>2` (`S_BLUWAR2`, "Blue Warrior", from the class's
  bank) without a `before`, else `S_<colour><class>1`; the colour's own
  line is never said (its tables hold −1). Colours `YEL`, `BLU`, `RED`, `GRE`
  (`0x8011f8cc`); classes `WAR`, `VAL`, `WIZ`, `ARC`, `DWF`, `KNI`, `SOR`,
  `JES`, `MIN`, `FAL`, `JAC`, `TIG`, `OGR`, `UNI`, `MED`, `HYE`
  (`0x8011f878`); the tables are filled by `FUN_8009fcc8`. A hero as the
  Pojo (`+0x124` flag `0x400`) is `S_POJO2`/`S_POJO1`. While
  `r13-0x6ed4` is set (`FUN_800a079c`) other tables are used (`S_%s%s1S`,
  `S_%s%s2S`; not traced).
- **Queue 0, the heroes**: their own lines (`FUN_80015124`, 1 s, volume
  `0xC0`, pan from the hero's position): eating (`FUN_8009f098`: one time
  in four the class's `S_<CLS>EAT` — the archer's by fruit — else
  `S_<CLS>EATSFX`, table `0x80122dcc`), poison (`FUN_8009f010`,
  `S_<CLS>POISON`), `FUN_8009ef80` (`S_<CLS>STEAL`), the hurt cries of the
  damage function (`FUN_8009f220`, `S_<CLS>PAIN1`–`4`, volume `0xE0`);
  and, while banks load (`r13-0x7800`) or with `r13-0x7830` < 0, a sound
  asked for without a position (`FUN_80015cac` → queue 0, 2 s).
- Nothing else counts: sounds played directly (`FUN_80015a30`,
  `FUN_80015a94`, `FUN_800157ec`) — hits, pickups, the death cry
  (`FUN_8009ea88`: sound 1 and the class's cry), `S_GAMEOVERVOX` — and
  the music.

Here (`audio.rs`: `QueueVoice`, `VoiceQueues`): both queues, with the
catalog lengths, the most waits, the 16-line limit, sentences and the
boss end's gate, on real time. In them: the hints (0.5 s, gated), the
message box's line (the tower's unlocks, 10 s), the tower wizard's pieces
(10 s) and ranks (the hero's name, then `S_EXP…`, 5 s), the bosses'
wizard (10 s), and the heroes' eating and poison lines. Every level
change by name waits while a queue holds a line (`exits.rs`): an exit,
the boss level's end, the last hero out (`frontend.rs`, `death`: once
the DEATH action is over outside the tower the hero is out — the HUD
shows its "IN TOWER" panel, "In-game HUD" — and from the next frame the
level change to the tower is asked for), the menus. Stand-ins: the old
level keeps running during the wait even where the game's is frozen (a
level start's own wait, `FUN_800a097c`); the queues aren't emptied and
the voices aren't stopped as the new level starts (they're empty by
then); lines aren't panned and there's no 12-voice limit, so the priority
does nothing. Not done: the level's name lines and the opening's wait,
the health warnings, the taunts, the hurt cries, the runestone count.

The save (`FUN_8007a670(p, 1)`, from `FUN_80053530`) happens when a level
outside the tower starts, except in the secret realm (12) and in `levelE2`
and `levelF2` (realm 5 and 6, level 1). There are no lives or continues on
the GameCube.

## GAME OVER (`0x4014`)

Entered when no player is left in the game (`r13-0x72d4` = 0 — the last
player quit). `FUN_80052200` draws string group `0xA9` (`GAME_OVER`) in
`font32` × 2.0 (`r2-0x6c68`), centred on 256 at y 120, typed out: after 60
fields one more letter every 8 (the centring uses the whole string). The
level keeps drawing behind. After 240 fields (`r13-0x7384` = `0xF0`) the
attract loop resumes (movies, then the title).

## Message box (`FUN_8006d7f4`)

`FUN_8006d7f4(players, group, page, voice)` shows pages of a text group of
bank 0 (`TEXT/SCROLL_E.ROM`; `FUN_8001fc00` finds a group by name, any
case) — page −1 means every page in turn — and returns when the last is
put away. Its callers: a scroll's pickup (`FUN_8005de3c` class 14: page
amount − 1 of the level's `SCROLLS<level>`, only the picking player's B
counts), the tower's gates (`NEEDCRYSTALS`/`NEEDGARGITEMS`,
`FUN_800a200c`/`FUN_800a1f88`), its unlocks (`UNLOCKLEVEL`/
`UNLOCKSECTION` with their voice lines, `FUN_800a20c4`), the tower's
`WELCOMEMESSAGE`/`GARMMESSAGE` (below), the demo's notices and `ALLCOINS`.

- It runs its own frame loop, so play stops: the pads are cleared
  (`FUN_80032788`), `r13-0x7598` = 1 (the players' update is skipped),
  and only the frozen scene, the HUD and the box are drawn.
- Per page: text width = the widest line of the group's font at its scale
  (`FUN_8001ef74`), height = lines × (font height × scale + 4)
  (`FUN_8001ed24`, lines split at `\n`); the prompt `r13-0x7e7c` "Press
  Button when done." is measured in `font32` at 0.5. The `Scroll_A` panel
  (`r13-0x7e78`) is max(prompt + 32, text + 96) wide (text + 96 capped at
  512) and text + 96 high, centred at x 256 with its middle at y 160; the
  lines are centred on 256 from 32 below its top, 4 apart, in `0x160C03`
  (dark brown on the parchment); the prompt shimmers (`FUN_8001eb80`,
  scale 0.5) 8 below the last line, the B button's picture (`BUTTON_TRI`,
  `r13-0x6cb4`, 20 × 20) at x 190 in its gap.
- After 15 fields a page is put away by B (`0x8000000`) of any of the
  given players (all, for −1); that stops the voice line
  (`FUN_80016558`). After the last page the pads are ignored for 4 frames
  (`FUN_80032a90(4)`).

Here (`message_box.rs`, `ShowMessage`): the same panel, sizes, colours,
prompt and timing; virtual time is paused while it's up, the hero's
input is ignored until 4 frames after it closes, and the front end's
Start does nothing meanwhile. Stand-in: any player's B puts a page away
(one player). Before, messages were plain text for 5 s over live play.

## Captions (`FUN_80019e64`)

The wizards' words during their scenes aren't in the box: a page of a
group (bank 0 for the tower's, the default `ENGLISH.ROM` bank for the
bosses' speeches) is typed out centred at a line of the screen — 16, in a
cut's top bar, for a boss's speech; 312, in the bottom bar, for the
tower's announcements — white, in the group's font at 0.667 of its scale,
lines 32 × that apart (`FUN_80019f5c`). The caller passes the ticks
(fields ÷ 2) since the page began as a budget: each letter costs 1.75
ticks (5.5 in one mode), a comma or full stop 2 more, a tab 5; a carriage
return costs 30 and, with time left after it, empties the line it ends
(what follows takes that line's place). A line being typed stands where
the whole line is centred. Paging through a group (page −1), a finished
page is held 60 fields before the next; the bosses' speeches step their
pages themselves the same way, the last page held too.

Here: `ShowCaption` (`message_box.rs`), `critters.rs` for the bosses'
speeches (which had typed a letter a tick, twice the game's speed).

## Runtime flow

Title → (Start) title menu → Start → "Loading..." → select (tower loaded
behind) → New → name → class/colour → "Loading..." → play in `levelL1`.
Start in play opens the Tower Menu or Game Menu; gameplay (virtual time)
is paused while any front-end screen or menu is up. `--level` /
`--character` start in play. `GDL_MENU="start@60,accept@90"` presses
front-end buttons on those frames (up, down, left, right, accept, back,
start, l, r) for scripted screenshots.

Keys: arrows/WASD move, Enter/Space/J accept, Escape/Backspace/H back,
Enter/Escape start, Q/P and E/O for L/R; pads: d-pad or stick, A, B,
Start, triggers.

## Stand-ins

- No attract loop (movies, scroll screens, credits, demo play) and no
  30 s title timeout.
- Menu sounds aren't played (the game plays sound indices `0xD`–`0x13`
  through `FUN_800157ec`; their names aren't mapped).
- The menus' spinning 3D arrow (`ICON_ARROW`) is drawn as the flat
  `MENU_MARKER` texture, 24 px, 16 left of the items.
- Options, Game Options, Compass and Controls list their items but change
  nothing; Shop and Inventory aren't implemented.
- Saving: a file stands in for the memory card, so the card's screens
  ("Accessing Memory Card in Slot A.", "Save File?", the save/load
  confirmations) aren't shown, the load list is plain text in the
  player's column, and a record keeps one level and experience rather
  than one per class (see Saving).
- Audio: the game's Mono/Stereo item is replaced by a Master Volume
  slider (not in the game); the three sliders set `GameOptions`
  (`options.rs`) and sit 40 apart instead of 52 so all three fit above the
  hints.
- The border sprites' glow flag, the `*X` inline icons and the menu fade
  out aren't drawn.
- The dying time is the DEATH animation, at most 4 s (the game's own
  counter isn't traced).
- A level change waiting for the voice queues leaves the old level
  running even where the game's is frozen (a level start's own wait);
  the queued lines aren't panned, and there's no 12-voice limit (see "The
  voice queues").
- Only player 1 is interactive on the select screen; class attributes show
  the start values (no per-character points or levels); changing class
  starts that class fresh (the game keeps each class's progress in the
  character record).
- HUD: players 2–4's panels only wait (`S3` over `S4` in the slot's dim
  colour `0x8011f9b0`, framed: no joining yet); the quest
  icon goes when the legendary item is used up, not at the weapon's throw
  (not done); the key row reads the hero's current realms beaten; the
  panel is hidden while a menu is up (its numbers would draw over the
  menu's parchment, text being drawn after images).
  Runestone slot i is lit by stone i (the stones are numbered 0–11 by
  their item type's `+0x40`, RUNEA1 = 0 … RUNED3 = 11; RUNEE1 = 12 is
  rune 13); slots used to be lit one stone late.
