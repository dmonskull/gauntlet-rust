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
rune rows; `FUN_80074850` hides a panel.

Sprites (x from the panel's left, sprite depth: lower is in front):

| sprite | position, size | texture |
| --- | --- | --- |
| `0x80275460` | (0, 304), 128 wide | `S3`; `BK_RUNE_STONE_02` while playing |
| `0x80275464` | (0, 320), 128 wide | `S4` tinted `0x8011f9a0[colour]` (joined) / `0x8011f9b0` (not); `S4_<class>` while playing |
| `0x80275468` | (0, 320), 128 wide | `S4_FRAME` |
| `0x8027546c` | (0, 344), 148 wide | (not set here) |
| `0x80275470` | (6, 357), 20 × 20, depth 63990 | `coin` |
| `0x80275474` | (61, 357), 20 × 20 | `heart` |
| `0x802753a0` × 12 | (15 + 8i + i/3, 306) | `SM_RUNE_<blu,red,yel,gre>_<01..03>`, shown for bit i of the runestones held (`+0x1ECA`) |
| `0x80275320` × 8 | (12 + 12i, 300) | `SM_KEY_<colour>` (`0x8011fdc0`), per-class key bits; shown only while `r13-0x6fdc` (300 fields from a level's start) runs |
| `0x802752e0` × 4 | (26, 322 + 3i) | — |
| `0x80275270` × 7 | table `0x8011f414` (name, x, y, depth; stride `0x14`): `trbo_full_new` ×2 (0, 304), `trbo_glint`, `turbo_glow_new`, `black_bar` ×2, `trbo_gleem1` (80, 310) | the turbo meter |
| `0x80275260` | centre x − 14, y −323 (off screen), hidden | `BTMBK_LEVL`: the level plate, shown only in the mode `r13-0x7338` |
| `0x80274894` | (8, 340), 16 × 16 | `RUNE13` (in the 13th-rune modes) |
| `0x802748a4` | (104, 338), 16 × 16 | `QUEST_ICON` (a quest item held, not in the tower) |

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
- a player out of the level: "Wait In Tower" / "Quit Game" with the A/B
  buttons (in play) or "IN TOWER".

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
- Only player 1 is interactive on the select screen; class attributes show
  the start values (no per-character points or levels); changing class
  starts that class fresh (the game keeps each class's progress in the
  character record).
- HUD: players 2–4's panels only wait (`S3` over `S4` in the slot's dim
  colour `0x8011f9b0`, framed: no joining yet); the
  legendary-key row, the quest and rune-13 icons and the "Wait In Tower"
  prompt aren't drawn; the panel is hidden while a menu is up (its numbers
  would draw over the menu's parchment, text being drawn after images).
  Runestone slot i is lit by stone i (the stones are numbered 0–11 by
  their item type's `+0x40`, RUNEA1 = 0 … RUNED3 = 11; RUNEE1 = 12 is
  rune 13); slots used to be lit one stone late.
