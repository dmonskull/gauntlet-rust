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

## Not yet

- Co-op combos (two heroes' combo attack); monsters' crowd penalty in
  their choice of target.
