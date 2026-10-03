# Name codes: secret characters and cheats

A hero named one of the game's codes is a secret character, or carries
powers that never run out. Decoded from `FUN_8007b4ec` and its callers in
the Ghidra dump, with the tables read from `main.dol` (`r2 = 0x8034D100`,
`r13 = 0x8034B4E0`). Player fields are offsets into the player record
(`0x802754C0` + player × `0x335C`); the name is at `+0xA80`.

## The check (`FUN_8007b4ec(record)`)

1. **The developers' codes.** The name (6 letters, `FUN_800e7664` =
   `strncmp`) against `r2-0x5d90` **MNTHRX** and `r2-0x5d88` **ARIENT**:
   every gameplay code's bit is set (`0xFFFFFFFF`), and three debug flags
   come on if a pad newly presses certain buttons on that very frame
   (`FUN_800314c4`: `DAT_8023ff78` held and not `DAT_8023ff98`): bits 8 →
   `DAT_80256f78 = 1` (read beside mode flag `r13-0x7534 & 0x10`, dump
   line 79733: not traced), `0x12` → `DAT_80256f64 = 3` (its bit 1, at
   dump lines 53080 and 53177: a key check that, with no keys `+0x1EB8`,
   answers −1 instead of 0 — not traced further), `0x13` →
   `DAT_80256f60 = 1` (a damage override; at 3, dump lines 29655 and
   43648, blows take fixed values; at 1, line 70349 — not traced). Against `r2-0x5d80` **ADMBLY**:
   the bits are the game's random number (`FUN_800c8340`).
2. **The secret characters** — only while the record has no model of its
   own yet (`+0xF0` = 0): 27 entries at `0x8011fde0`, `0x24` bytes each:
   colour (u32), class (u32), code (8 bytes), model (4 bytes), three u32
   (0), a flag. A matching code (with the flag 0, or `r13-0x72d0` > 1)
   sets the colour `+4`, the class `+0xC` and `+0xF0` = the model's
   address, and returns 1 — no gameplay codes then.
3. **The gameplay codes**, still only with `+0xF0` = 0: entries at
   `0x801201ac`, `0x14` bytes each: code (8), kind (i32), amount (f32),
   bits (u32). A matching code, or its bit set by a developers' code:
   kind 1 → gold `+0x1EC4` = the amount; 2 → keys `+0x1EB8`; 4 → potions
   `+0x1EBC` (the count only: the kinds at `+0x3300` stay); any other →
   `FUN_8007ee10(amount, r2-0x5e38 = −1.0, record, kind, bits)`, the
   pickups' grant with time −1, and kind 9 also ORs the bits into the
   special bits `+0x124` at once (recomputed by the stats routine every
   tick from the slots that are on).
4. With `+0xF0` already set: when it's `r13-0x7d94` (the real Sumner's
   model, a `"sum"` string at `0x803470e8`), class `+0xC` = 2 and `+8` =
   2; otherwise nothing. Returns 0 after the gameplay codes.

The loop over the gameplay codes runs **27** times (`0x1b`, the secret
characters' count) over an **18**-entry table: entries 18–26 are the code
pointers that follow it. A typed name can't match their bytes, but the
developers' codes set every bit, so with MNTHRX / ARIENT the grant is also
called 9 times with junk kinds and bits (and the last entry with kind 0,
amount −1, bits 0): those slots take room and do nothing.

**`r13-0x72d0`** is set to 1 at start-up (`FUN_80053438`) and never
changed: the check always runs, and NAK069 (the one flagged secret
character) never matches on the retail disc.

## When

- **The select screen** (`FUN_8008be04`): as the name entry ends and the
  name has blinked (`+0x3348` past `0x77` fields), and after a Load. A
  secret character returns 1: `FUN_8008dbf4(player, class)` makes the
  player ready (state `+0xE8` = 3) with that class and colour, shows the
  class's weapon (`S12_WEAP_<class>`) and voices it — no class card.
  Otherwise the class card (step 4). The gameplay codes are granted here
  too.
- **Every level's start**: `FUN_80079ed8` → `FUN_8007ae84` sets each hero
  up for the level and runs the check (`r13-0x72d0` > 0). So the codes are
  granted again each level (a power already carried is topped up: same
  subtype and bits; its time −1 stays −1), and ALLFUL / 10000K set the
  keys, potions and gold again.

## The tables

Secret characters (colour 0 yellow, 1 blue, 2 red, 3 green; class 0 WAR,
1 VAL, 2 WIZ, 3 ARC, 4 DWF, 5 KNI, 6 SOR, 7 JES; the model is a folder in
the class's, `PLAYERS/<class>/<model>`):

| code | colour | class | model | | code | colour | class | model |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ICE600 | red | DWF | GEI | | NAK069 | blue | VAL | NUD (debug only) |
| NUD069 | blue | DWF | SNM | | TWN300 | green | VAL | GET |
| STX222 | blue | JES | STK | | AYA555 | blue | VAL | SCH |
| KJH105 | green | JES | KJH | | CEL721 | green | VAL | CEL |
| PNK666 | yellow | JES | PNK | | CAS400 | blue | WAR | GEC |
| BAT900 | green | KNI | GEB | | MTN200 | red | WAR | GEM |
| TAK118 | red | KNI | NIN | | RAT333 | red | WAR | RAT |
| STG333 | green | KNI | STG | | GARM99 | yellow | WIZ | GA2 |
| KAO292 | red | KNI | WTR | | GARM00 | green | WIZ | GAM |
| CSS222 | yellow | KNI | CSS | | DES700 | yellow | WIZ | GED |
| RIZ721 | blue | KNI | RIZ | | SKY100 | green | WIZ | GEP |
| ARV984 | blue | KNI | ARV | | SUM224 | yellow | WIZ | SUM |
| DIB626 | red | KNI | DIB | | DARTHC | green | KNI | DCY |
| SJB964 | blue | KNI | SJB | | | | | |

SUM224 has Sumner's model on a Wizard, but its `+0xF0` is the table's
`"sum"`, not `r13-0x7d94`'s: it isn't the real Sumner (999 in every stat,
his own record; [frontend.md](frontend.md) "Sumner", [shop.md](shop.md),
[items.md](items.md)).

Gameplay codes (the powers' names are the power menu's, [powers.md](
powers.md)):

| code | kind | amount | bits | what |
| --- | --- | --- | --- | --- |
| INVULN | 6 armour | 0 | `0x10000` | invulnerability |
| SSHOTS | 5 weapon | −1 | `0x100000` | super shots (the crossbow's), never spent |
| EGG911 | 9 special | 0 | `0x400` | the Pojo |
| 1ANGEL | 9 / 6 | 0 | `0x1` / `0x80000` | levitation, and the halo (anti-Death) |
| DELTA1 | 9 | 0 | `0x300` | growth, and the enemies shrunk |
| 000000 | 9 | 0 | `0x4` | invisibility |
| PEEKIN | 9 | 0 | `0x2` | x-ray glasses |
| PURPLE | 9 | 0 | `0x80000` | the turbo meter kept full (spent only on a timed slot) |
| XSPEED | 9 | 4 | `0x10000` | faster actions (the speed bit; the amount is unused) |
| QCKSHT | 5 | 0 | `0x20000000` | rapid fire |
| MENAGE | 5 | 0 | `0x80000` | the three-way shot |
| REFLEX | 5 | 0 | `0x200000` | reflecting shots |
| ALLFUL | 2 / 4 | 9 | | nine keys, nine potions |
| 10000K | 1 | 10000 | | gold set to 10,000 |
| NOVATO | 9 | 0 | `0x8` | time stopped |
| MEBERT | 9 | 0 | `0x80` | the phoenix familiar |

## The grant's slot (`FUN_8007ee10`)

A power already carried (same subtype and bits) is topped up: amount added
when positive; a positive new time adds to a positive old one, a negative
one replaces it. Otherwise a pass over the eleven slots picks one: a slot
whose time is below 0 counts −1 when it's of the same subtype, else −2;
the pick starts at −2; a slot is taken when the pick is −2, or it's free
(0), or 0 ≤ its time < the pick; a free pick ends the pass. So a free slot
wins, then (from a timed first slot) the timed one nearest to running out,
and once a slot held for good of the same subtype is picked no timed slot
replaces it. The new slot is held (`+0x1E0` = 1): the hero turns it on
from the power menu, and a time of −1 never runs down.

## Here

`crates/gdl-game/src/cheats.rs`: the tables (18 gameplay entries — the
junk the developers' codes also feed the grant is left out — and the 27
characters, NAK069 marked debug-only), `secret_character`, `grants`,
`apply`, and the level-start system (before the front end keeps the
heroes' records for the level, `frontend.rs` `level_started`). A secret
character's variant is its colour's folder then its model's (`GREDCY`):
everything that goes by the hero's colour reads the first three letters,
and `cheats::model_folder` gives the model (`character.rs`, the throw
models in `projectiles.rs`). The select screen (`select_tick`) picks a
secret character as the name stops blinking and readies the player
(`Select::special`, `auto_pick`). `player_state::grant_power` takes the
game's slot pass (`grant_slot`). ADMBLY's bits are a hash of what every
machine has alike (the level, the slot, gold and experience) instead of
the game's generator, so online stays in step. `--name NAME` (with
`--level`) and `GDL_ONLINE_HERO=<class>:<name>` (testing) name the hero.

Checked in game: eight secret characters' models (DARTHC, SUM224, ICE600,
PNK666, GARM00, CAS400, TAK118, CEL721); every gameplay code's power on
from the power menu (the Pojo, chrome, growth with shrunk grunts,
invisibility, the phoenix, the weapon shots, the hourglass, x-ray, speed,
turbo); ALLFUL's 9 keys and 9 potions and 10000K's gold on the HUD;
MNTHRX's 18 grants; online, a WAR named INVULN and DARTHC's knight alike
on both machines.

Not ported: the three debug flags the developers' codes set on a button
press (`DAT_80256f78/64/60`), and the junk grants.
