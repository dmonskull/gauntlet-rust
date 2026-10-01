# Local co-op

Up to four players on one machine, as the game's four player records
(`docs/items.md`), with the PC's devices. Implemented in
[`party.rs`](../crates/gdl-game/src/party.rs) (the party, inputs),
[`frontend.rs`](../crates/gdl-game/src/frontend.rs) (joining, the select
screen), [`player.rs`](../crates/gdl-game/src/player.rs) (heroes and
their controls) and every module that touches a hero (by its slot).
Online play builds on the same slots ([online.md](online.md)).

## The party

`Party` holds a member per slot (0–3): the hero's choice (class, colour),
name, record (`PlayerState`, which outlives levels) and devices. The
hero on a level is a `Player` entity with its slot; damage, healing and
power uses are messages naming the slot. The game's per-player rules
apply per slot: each panel, power menu, hint box (stand-in: one box, the
first player's), death and revival from the level-start record (so gold
picked up in a level is lost when the hero dies there), experience for
the blows each lands. What the game takes from all players: a level is
finished for every player still in it; the tower's gates, exits and
trophies open by any player's progress; the boss's death marks every
record.

## Devices

Each tick every slot's controls are gathered into `Inputs` (stick, right
stick, logical buttons): from the keyboard and mouse if the slot holds
them, and from its pad. Alone, the player holds every device (the
keyboard and every pad nobody else holds): a spare pad becomes theirs
the first time it's played with (any button but Start, or a stick), so
switching between keyboard/mouse and pad is free. With several players
each plays with their own; the keyboard belongs to one of them.

## Joining (as the game has it)

- **The select screen** (a new game, Manage Character in the tower, and
  — still to come — the screen after a level): four columns
  (`docs/frontend.md`); the device that started the game is player 1
  (keeping the keyboard alongside a pad); another device joins the first
  free column with Start (a pad's A too). Each column names, picks or
  loads its own hero; the game goes on once every player in is ready (a
  ready player opens their character menu again with A); backing out of
  New/Load leaves; quitting a character takes the player out (the last
  one ends the game). The bottom strip shows the four player panels.
- **During play**: Start from a device nobody plays with joins — in the
  tower the select screen opens with a column for it (the players
  already in wait ready); anywhere else the player takes the next free
  panel, which says "IN TOWER", and picks their hero as the party comes
  back to the tower (the game: the player's present bit, then the
  tower's select screen, `FUN_8008f70c`). While a pad nobody holds is
  connected, the next free panel says PRESS START (not the game's: our
  auto-detection).

## Per-player settings

The game's Controls menu (style, rumble, auto aim, auto attack) is per
pad; here per player (`GameOptions::players`, saved as `p1.scheme=…`),
and the Controls menu sets those of the player who opened it. PC
Settings → Players lists each player and what they play with, each
opening their own Controls.

## The level for several players

Placements for 2–4 players (`Placement::active_for`) come with the
players in the game as the level loads (`LevelPopulation::players`):
items, monsters, generators, critters, hazards, rotators. Start
positions: the game's 2 × 2 square beside the first hero (`player.rs`
`start_spot`, `FUN_80080154`). The camera frames everyone
([camera.md](camera.md), "Several players"). Bosses take blows × 1, 1,
0.5, 0.3, 0.2 and roar after 50 × 1, 1, 1.5, 2, 2 damage by the count
(`critters.rs`); lifts that wait for players wait (players − 1) × 60
fields; "every player" pads need every player in the game on them. An
exit's portal steps on only while every living hero stands in it.

## Testing

`GDL_FAKE_PAD` scripts a pad through Bevy's raw pad events
(`fake_pad.rs`): join on the select screen, or with
`GDL_FAKE_PAD_PLAYER=<class>` (and `GDL_FAKE_PAD_AT=1`) join at once as
the next player on a level picked from the command line.

## Co-op combos (decoded, not ported)

Two heroes' combo attack (the help screen's "2P COMBO"; the COMBO_MOVE
button, `combat.md`). From the player update (`FUN_80080d3c`) and the
input classifier (`FUN_80088170`); player records are `0x335c` bytes from
`0x802754c0`, `+8` the class (0 WAR, 1 VAL, 2 WIZ, 3 ARC, 4 DWF, 5 KNI,
6 SOR, 7 JES).

- **Asking** (classifier, after the magic buttons): COMBO_MOVE held, turbo
  (`+0x828`) ≥ 50 and a partner found (`FUN_8008872c`) → intent `0x16`,
  the partner's record kept in `+0x6BC`. The asker must be in play
  (`+0xE8` = 1), not in a combo (`+0x6B8`, `+0x6BC` both 0), have a carry
  node (`+0x6DC` ≠ 0), not be the Pojo (`+0x124 & 0x400`), and stand on a
  node (`+0x8C4`) without flag `0x1000`.
- **The partner**: the nearest other hero in play, not in a combo, without
  `+0x964` flags `0x50`, not the Pojo, standing on a node without flag
  `0x1000`, whose playing action (`+0x208`) is below `0x54` or in
  `0x5B`–`0x6A`; within 5 (`r2-0x5b44`) and 3 up or down (`r2-0x5c88`),
  and ahead of the asker: the unit vector to it · the asker's facing
  (`+0x34`, `+0x3C`) ≥ 0.707 (`r2-0x5aa8`). Not while the asker's
  legendary-weapon state (`+0x834`) runs on a boss level.
- **Starting**: intent `0x16` asks for COMBOACT1 (`0x58`) and puts the cost,
  50 (`r2-0x5b5c`), in `+0x910`. When COMBOACT1 or 2 is playing with a
  partner pending, the pair link (`+0x6B8` each way), the partner gets
  `+0x964 |= 0x10` and the asker's turbo loses the cost.
- **The partner's side**: the classifier gives intent `0x27` while its
  `+0x964` has `0x40`, else `0x26` with `0x10`, else `0x17` with `0x80`.
  `0x26` plays the action named after the asker's class — COMBOWAR1,
  COMBOVAL, COMBOWIZ, COMBOARC, COMBODWF1, COMBOKNI, COMBOSOR, COMBOJES
  (`0x88`, `0x8B`–`0x8E`, `0x91`–`0x93`); `0x27` COMBOWAR2 (`0x89`) after a
  warrior, COMBODWF2 (`0x8F`) after a dwarf, else READY; `0x17` COMBOACT2.
- **By the asker's class**, while its COMBOACT plays (`FUN_800747ac`
  attaches a hero to another's carry node, `FUN_80074644` lets go):
  - VAL, ARC: the partner is turned to face away from the asker (heading
    from the asker + π, `r2-0x5ab8`, also into its `+0x894`) and the asker
    rides on the partner's carry node; when COMBOACT ends the asker lets
    go and the pair unlink.
  - WAR: for COMBOACT1's first 30 frames (`+0x98` < `r2-0x5b28`) the
    partner rides on the warrior's carry node (its `+0x8FC` = now); then
    it's thrown: let go, `0x10` → `0x40` (COMBOWAR2), and 240 fields
    (`+0x1FA`) later the pair unlink (the warrior's `+0x964` loses `0x80`).
  - DWF: on COMBOACT1 the partner is turned as above and the dwarf rides
    the partner (`+0x964 |= 0x80`: COMBOACT2 next), 240 fields; after it the
    partner's `0x10` → `0x40` (COMBODWF2: the dwarf thrown); when the
    fields run out the dwarf lets go and the pair unlink.
  - WIZ, KNI, SOR: the partner rides on the asker's carry node for the
    whole COMBOACT, then is let go and the pair unlink.
  - JES: the partner is taken onto the carry node and let go at once.
- **The thrown hero** (COMBOWAR2 / COMBODWF2 movement): what it runs into
  takes 50 (warrior's) or 10 (dwarf's) heavy (`0x20`) damage
  (`FUN_8008615c`), objects 50 or 20 (`FUN_8008625c`).
- **The state machine** (`FUN_800ab898`): COMBOACT1 hands over at its end
  (mode 1) to COMBOACT2 when the class has that clip (the action's clip
  index, record `+0x210 + 8 × action`, ≥ 0), else COMBOACT3 if it has it,
  else as asked (mode 0 unless knocked). COMBOACT2, COMBOWAR2 and
  COMBODWF2 loop (the loop flag) while still asked for (mode 0), and as
  soon as something else is asked for go to their next action (`+1`: …3)
  if the class has its clip (mode 2, at once). COMBOACT3 and the other
  per-class actions end normally (mode 0 unless knocked). None of them
  strikes on a switch (no `+0x900` bits).
- **The hit moment** (end of `FUN_80080d3c`, while `+0x834` < 2): the
  action's time `+0x98` crossing 0 (last tick's `+0x958` < 0 ≤ now) during
  COMBOACT1 starts the effects `FUN_80091c9c(at, own colour, partner's
  colour)`: COMBO_SPH (effect `0x3D`, `0x80122648`) tinted by the
  partner's colour (tables `0x80119bb8`, `0x801218bc` → `0x80121868`) and
  COMBO_<own colour> (`0x80122638`: `0x3E`–`0x41` YEL/BLU/RED/GRE), at the
  hero's top point (`+0x54`), or for the dwarf and jester at `+0xD0` +
  `+0x838`. The blow: the class's `DAMG` record named by `PDAT +0x1C`
  (COMBOACT1; `+0x1E` for COMBOACT3) through `FUN_80088b88(last time, time,
  hero, record, partner, 0)` — the same routine the power and turbo
  attacks use ([chunk-files.md](chunk-files.md) "The heroes' attack
  records"; WAR's combo record 8: kind 4, damage 10, from frame 0, hint
  102).
- Not traced: `FUN_80089114` (what kinds 2–4 hit and how far),
  `FUN_80030094` (kind 10's areas), the carry node's bone (`+0x6DC`), how a
  carried hero is placed each frame (`FUN_800747ac` / `FUN_80074644`).
  `tools/hdamg.py <class>` dumps a class's records; `tools/mydol.py` reads
  the constants.

## Not yet

- Co-op combos (decoded above, not ported); monsters' crowd penalty in
  their choice of target.
