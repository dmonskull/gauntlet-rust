# Projectiles

Implemented in [`projectiles.rs`](../crates/gdl-game/src/projectiles.rs)
(tables, launch maths, flight, collision, blasts, models), with the
release hooks in [`player.rs`](../crates/gdl-game/src/player.rs)
(`HeroShot`) and [`monsters.rs`](../crates/gdl-game/src/monsters.rs)
(the throwing AIs, `MonsterShot`). Addresses are in `main.dol`; `r2`/`r13`
as in [INDEX.md](INDEX.md). The action chaining that decides *when* a hero
throws is in [combat.md](combat.md).

Verified in game (levelA1): the Archer's arrow and the Warrior's axe fly,
render and hit a `GDL_DUMMY`; a grunt thrower (`GDL_THROWER=25`) throws an
arrow about every two seconds that hurts the hero for 10; a bomber
(`GDL_THROWER=20,0x11,5`) lobs bombs that hit and burst.

## The game's missile slots

Missiles are ordinary slots of the effects table (64 × `0xF0` bytes; the
array's model-instance words start at `0x802855ac`, allocated by
`FUN_80096bcc` through `FUN_8009423c(lifetime, model, owner, flags,
0x80000)`), updated once a frame by `FUN_80094418`. Fields used here
(addresses of slot 0):

| address | field | set by |
| --- | --- | --- |
| `0x80285600` / `0x80285604` | end time / lifetime | `FUN_8009423c` |
| `0x802855fc` | collision flags (below) | `FUN_8009423c` |
| `0x80285618..20` | velocity, units/s | `FUN_80093688` |
| `0x80285624..2c` | spin about X, Y, Z (rad/s) | `FUN_80093688` |
| `0x80285630` | radius | `FUN_80093688` |
| `0x80285638` (+`0x8028563c`, `0x80285640`) | Y (X, Z) acceleration: gravity | `FUN_80093688` |
| `0x80285644` | damage | `FUN_80093768` |
| `0x8028564c` | damage kind bits (low nibble: element) | `FUN_80093768` |
| `0x80285650` | blast radius | `FUN_80093768` |
| `0x80285654` (i16) | owner: player + 1, 0 for monsters | `FUN_80093768` |
| `0x80285656` (i16), `0x80285664`, `0x80285668` | follow-up effect, hit effect, wall effect | `FUN_800937ac` |

## Hero release (end of `FUN_80080d3c`)

When the event word `+0x900` has any of `0xFF00` (set by the action state
machine: `0x100` when a strafe attack or THROW1/2 ends, `0x1000` when
ATTPWRATHROW ends — [combat.md](combat.md)), after the melee blow block:

| bits | wind-up `w` | damage × | offset mode | extra kind |
| --- | --- | --- | --- | --- |
| `0x100` (throw, strafe attack) | now − `+0x8FC` − 0.27 (`r2-0x5ad8`), clamped to 0 … 0.1 (`r2-0x5bd0`) | 1 | 1 (PDAT `+0x5C`) | — |
| `0x1000` (power throw) | 0.06 (`r2-0x5ae0`) | 2 (`r2-0x5c18`) | 2 (PDAT `+0x158`) | `0x2000010` |
| `0x6000` | 0.06 | 1 | 0 (no offset) | — |
| `0x800` | 0 | 1 (1.5/2 with a powerup, `FUN_8007ed38`) | 0 | `0x100000` |

- **`0x6000`, Skorne's gauntlets** (ATTFIREL → 0x69 raises `0x2000`,
  0x68 → 0x6A `0x4000`; [items.md](items.md), "Attack overrides"): w =
  0.06 (`r2-0x5AE0`) — reach 27 — damage × 1, no hand offset (from the
  centre `+0x64`, feet + 2.5), the kind the hero's weapon bits
  (`+0x11C`). The spawn picks the record by the **special** bits, not
  the event: `0x8000` (left) → `0x80119B58`, `0x4000` (right) →
  `0x80119B88` (below). Sound `S_GAUNTLET1` (`0x5C`, `FUN_8009ec48`) for
  the left, `S_GAUNTLET2` (`0x5D`, `FUN_8009ec08`) for the right (volume
  `0xE0`), in place of the throw sound.
- **`0x800`, the super crossbow** (SSHOT1/SSHOT2 handing over; SSHOT2
  repeats while held, a bolt each — items.md): `FUN_8007ed38(1, player,
  5, 0x100000)` spends a shot from the first weapon slot with the
  crossbow bit and returns the amount it had (1 for an unlimited one, 0
  with no such slot). With a shot: damage × 2 (`r2-0x5C18`; × 1.5,
  `r2-0x5ADC`, on a boss level) and kind |= `0x100000`, which picks the
  bolt record `0x80119B28` and model `SUPERARROW` (`WEAPONS`) and makes
  it pierce; without, × 1 and the class's own missile. w = 0 (reach 15,
  unused: a `0x100000` shot isn't lobbed, it flies along the aim), no
  hand offset (the centre `+0x64`). The throw sound (`FUN_8009ee70`)
  plays `S_SUPERSHOT` (`0x42`) because the weapon bits hold `0x100000`
  (any of `0x580000`: the multi-shots too), else the element's.

**The sound** of every release (after the spawn, whatever became of the
throw): special `0x8000` → `S_GAUNTLET1` (`FUN_8009ec48`), `0x4000` →
`S_GAUNTLET2` (`FUN_8009ec08`), else `FUN_8009ee70`: weapon bits
`& 0x580000` → `S_SUPERSHOT` (`0x42`); else by the element (`& 0xF`) 1
`S_AMULETFIRE` (`0x44`), 2 `S_AMULETLIGHTNI` (`0x46`), 3 `S_AMULETLIGHT`
(`0x45`), 4 `S_AMULETACID` (`0x43`); else (0, or above 4) the class's
from the table at `0x80122e6c` (by player `+0x08`, catalog ids `bank <<
16 | call`): `S_WARTHROW`, `S_VALTHROW`, `S_WIZTHROW`, `S_ARCTHROW`,
`S_DWFTHROW`, `S_KNITHROW`, `S_SORTHROW`, `S_JESTHROW` — eight entries,
the turbo table follows (sound ids are catalog indices, from 0).

**Kind `0x100000`** (the passed kind: a crossbow bolt, or any throw while
the crossbow's bit is held) also changes the spawn (`FUN_80030094`): the
wall check at release is skipped (not only the item one's ending), the
aim is the facing (the aim's first case), and it isn't lobbed — the
velocity is the unit aim × the speed.

Here (`projectiles.rs`: `release`, `throw_sound`, `launch_hero`; the
actions in [items.md](items.md), "Attack overrides"): the four releases
in that order, the gauntlets' records and models (`BOSSG_ELEC`,
`BOSSG_ACID`, `SUPERARROW` from `WEAPONS`), the crossbow's use spent
(`SpendPower`; with none left the class's missile), × 2 / × 1.5, the bolt
flying straight along the facing through walls at release and on past
an item it strikes there, the sound. The alternate characters' throw sound is
their base class's: the record's `+0x08` is the class less 8 for them
(`FUN_80079a00`).

After any release the event word loses `0xFF00` and gains `0x10000000`
— the throw event the phoenix, the familiars and the body looks read
on the next tick (below, "The familiars and the phoenix").

`+0x8FC` is the time the latest attack started: the `0xFF` block (which
runs *before* this one) sets it whenever bit 1 (attack started) is set. So
a strafe attack chaining into the next one, which sets both in the same
switch, throws with no wind-up at all, while THROW1S → THROW1 → THROW1R
counts from THROW1S.

Then `FUN_800857d8(distance, w, player, facing dir, aim dir, kind, target,
object)` (the aim) and `FUN_80030094(reach, damage ×, player, aim, kind,
mode)` (the spawn).

### Aim (`FUN_800857d8`)

The aim vector is the search's unit vector to the target from this tick's
search (toward the stick heading or the facing), or that heading itself
when nothing was found, or — with DEFEND/STRAFE held, attack aim off or the
C-stick pushed — the heading even when something was found. `e` =
`+0x8BC`, the sine of the floor's slope along the facing (set by
`FUN_8008764c` from the floor normal), capped at 0.707; if `+0x8C0 & 8` and
0 < e < 0.5 it's 0.5; the dwarf (class 4) adds 0.2 (`r2-0x5aa0`).

1. If `facing · aim` (horizontal) < 0.866 × |aim horizontal|
   (`r2-0x5a98`, the target is more than 30° off), or kind `& 0x100000`:
   aim = (facing X, e, facing Z).
2. Else with no target: aim X, Z × √(1 − e²), aim Y = e.
3. Else: aim Y × 1.2 (`r2-0x5a90`) if positive; if |aim Y| < |e|, aim Y =
   (aim Y + e) / 2.
4. aim Y × 0.5 if negative; normalise; if aim Y > 0.866 the facing is used.

It returns the **reach** 200 (`r2-0x5a88`) × w + 15 (`r2-0x5c00`): 15–35
units, 27 for the power throw.

### Spawn (`FUN_80030094`, `FUN_800307ec`)

- Missile record (`0x30` bytes), the first of: special `0x8000` (Skorne's
  left gauntlet) → `0x80119B58`; special `0x4000` (right) →
  `0x80119B88`; kind `0x100000` without `0x2000000` → `0x80119B28`; else
  per class at `0x801189A8` + class (player `+0x08`) × `0x30`. The three
  power records (the "special" ones: no element trail, `0x8023FD34`):

  | record | kind `+0x00` | radius `+0x0C` | gravity, spin, blast | `+0x04`/`+0x08` | `+0x2C` | model | flags |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | `0x80119B28` crossbow | `0x20` | 5 | 0 | 50 / 40 | 5 | `SUPERARROW` | — |
  | `0x80119B58` left gauntlet | 2 (lightning) | 2 | 0 | 50 / 40 | 5 | `BOSSG_ELEC` | `0x10000` |
  | `0x80119B88` right gauntlet | 4 (acid) | 2 | 0 | 50 / 40 | 5 | `BOSSG_ACID` | `0x10000` |

  The missile's kind is the passed kind | the record's `+0x00`
  (`FUN_80093768`); the models are loaded from `WEAPONS` by
  `FUN_80030c38` (`r13-0x7540`, `-0x7544`, `-0x7548`); flag `0x10000`
  (meaning not traced) goes on the gauntlets' missiles and the Pojo's
  (`PHOENIX_FBALL`, `0x8023FDE4`, with the class record). A crossbow bolt
  gets a white streak (`0xFFFFFF`, alpha `0x40`) instead of the class
  colours.
- Damage = player `+0x114` × the multiplier. Speed v = player `+0x118`
  clamped to 1 … 100 (`r2-0x7408`, `r2-0x7378`). Both are set by
  `FUN_8007c4f0` from the strength stat (`+0xF4`), or the magic stat
  (`+0xFC`) for the wizard and sorceress (class 2, 6):
  `+0x114` = 5 + 0.001 × stat × 15, `+0x118` = 20 + 0.001 × stat × 40
  (ranges at `r13-0x7d64`/`-0x7d60` and `r13-0x7d5c`/`-0x7d58`).
- Hand = centre `+0x64` (feet + 2.5) + PDAT offset (mode 1 `+0x5C`, mode 2
  `+0x158`, else none; `+0x124 & 0x400` uses `0x80119be8`) turned by the
  player matrix (`FUN_800bdef0`, rotation only).
- Check segment from hand − 3 × aim (`r2-0x7380`) to start = hand + 2 × aim
  (`r2-0x7398`): a wall there (`FUN_8000cfa0`, the missile's radius; not
  checked with kind `0x100000`) ends the throw with a wall spark
  (`FUN_800938b8`), unless kind `0x200000` (then it starts from the back
  end); an item there (`FUN_8005ed30`) takes the damage at once
  (`FUN_8005c1c8`, with the passed kind, not the record's) and the throw
  ends (unless kind `0x100000`).
- **Lob**: T = aim × reach; d = |T horizontal|; horizontal direction
  T / d; slope = (0.5 × g × d / v + (T.y − 0.5) × v / d) / v (`r2-0x73b0`,
  `r2-0x7370`); velocity = (dir X, slope, dir Z) × v. It comes down 0.5
  below the hand `reach` units ahead after d / v seconds.
- `FUN_800307ec`, once (five times with kinds `0x80000`/`0x400000`, turned
  by the angle tables at `0x80111580`/`0x80111594`): lifetime 3 s
  (`r2-0x7360`; 2 s for the spreads), flags `0x1000000` | (owner 0:
  `0x1107`; players: `0x20e`, `0x200f` in game mode 1, `0xf` in versus;
  `& ~4` with kind `0x100000`) | `0x20000` when the record has no spin.
  Radius × 1.8 (`r2-0x7358`) with kind `0x2000000`; drawn × the damage
  multiplier when above 1 (1.8, `r2-0x7350`, set first for the power
  throw, is then overwritten by it), × 1.2 (`r2-0x7368`) above player
  level 98 (`+0x3324`).

### Per-class missile record (`0x801189A8`, 8 × `0x30`)

| class | radius `+0x0C` | spin `+0x14` | gravity `+0x20` | blast `+0x10` | effects `+0x24/28/2C` |
| --- | --- | --- | --- | --- | --- |
| WAR | 1.0 | 6π about X | 12 | — | 0, 0, 5 |
| VAL | 1.0 | 6π | 8 | — | 0, 0, 5 |
| WIZ | 1.2 | — | 8 | — | 0, 0, 5 |
| ARC | 0.7 | — | 8 | — | 0, 0, 5 |
| DWF | 1.0 | 6π | 20 | — | 0, 0, 5 |
| KNI | 1.0 | 6π | 8 | — | 0, 0, 5 |
| SOR | 1.2 | — | 8 | — | 0, 0, 5 |
| JES | 0.7 | — | 8 | 2 | `0x17`, 0, 5 |

`+0x00` (kind) is 0 for all; `+0x04`/`+0x08` (10/25, 10/30, 12/40, 8/50…)
aren't read by the hero's path. PDAT offsets (`+0x5C`, `+0x158`): WAR
(−0.5, 0.5, 1.5) / (0, 0.5, 1.5), VAL (0, 1.8, 2) / (0, 1, 0), WIZ
(0.5, 1.5, 1) / (0, 0.5, 0), ARC (0, 1.5, 0) / (0, 1.3, 0), DWF (0, 0, 2) /
(0, 0.5, 0), KNI (0, 0.5, 2) / (0, 0.5, 0), SOR (0, 1, 0.5) / (0, 0.5, 0),
JES (−0.5, 1, 0.5) / (0, 1.1, 0); the eight secret classes repeat these in
the same order.

### Models (`FUN_80030c38`)

The weapon table at `0x80118868` (`0x14` bytes, by player `+0x0C`): name,
then ten digits by player level ÷ 10:

| AXE | SWD | STF | BOW | HAM | MAC | WND | BOM | MIN | FAL |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0000000000 | 0000000000 | 1112223333 | 1112223333 | 0000000000 | 0000000000 | 1112223333 | 1111112233 | 1111111111 | 1111111111 |

Sixteen entries, one per class: after the falconess come **STF** (the
jackal: the wizard's), **BOW** (the tigress: the archer's), **OGR**,
**UNI** (digits `1111111111`, as MIN and FAL), **WND** (the medusa: the
sorceress's) and **BOM** (the hyena: the jester's), each with the digits
of the entry it repeats. (The rewrite had only the first ten: those six
characters threw nothing to see.) STF and WND carry `0x80` in their last
byte.

Digit `0` → atree `"%s_THROW0"` in the class's own `ANIM.PS2`
(`PLAYERS/WAR/BLU`: `AXE_THROW0`); else `"%s_THROW%c"` in its effects
folder (`PLAYERS/ARC/SFXBLU`: `BOW_THROW1…4`); failing both, `"%s_THROW1"`
in its own (`"Player Missile not found"`). The atree's nodes name the
objects (`BOW_THROW1` + `ARC_A`). Monster missiles: `FUN_80030b54` loads
`"%s_%s"` — the type's name and `ARROW` / `BOMB` / `FBALL`
(`0x80118b50`) — from the type's anim file (`GRU_ARROW`, `GRU_BOMB`,
`DEM_FBALL`, `SOR_FBALL`…).

### The familiars and the phoenix

Confirmed from the decompile, constants from the DOL.

**Who has one** (`FUN_8007ddd0`, every tick from the stats routine): a
hero of level 30–79 wears `FAMILIAR1`, from 80 `FAMILIAR2` (below 30
none; the slot `+0x748` is freed). Both come from the hero's effects bank
(`PLAYERS/<class>/SFX<colour>`, `0x80274DDC`; loaded with `FAMILIAR_SPIT`
by `FUN_80030c38` into `0x8023FDE8`/`0x8023FDEC` + player × 8 and
`0x8023FDD4` + player × 4). The secret classes' folders have no `SFX`
banks of their own; which bank the game gives them isn't traced. It's an atree instance (flags
`0x800`, `+0x10` set, transparency 0) under the hero model `+0x74`, at
PDAT `+0x164` (× 1.2, `r2-0x5D20`, at level 99): (0, 0, 0) for most
classes, (−1, 1.7, 0) for the valkyrie (and the falconess), (0, −1, 0)
for the ogre. `FAMILIAR1`'s own joint sits at (−0.91, 5.08, −0.61) — over
the left shoulder. Each tick it's hidden in play (flag 2) while the hero
has the phoenix (special `0x80`), else shown, playing action 1 (ATTACK)
in mode 2 on the throw-event tick, else action 0 (READY).

**When it fires** (the player update `FUN_80080d3c`, before this tick's
release): when the event word holds the throw event `0x10000000` —
raised by the previous tick's release of any hero missile (a throw, a
power throw, a gauntlet shot or a crossbow bolt; "Hero release") — and
the hero has the phoenix or a familiar. The event is cleared right after,
so each hero missile brings one shot, a tick later.

**From where**: the mouth, PDAT `+0x170` (per axis × the hero model's
scale, `+0x74` `+0x40..+0x48`: the ogre's 1.6, growth's 1.3, level 99's
1.2) through the hero's matrix (`+0x14`, `FUN_800bdff4`: rotation and
position — the feet). The offsets: WAR (−1, 5, −1), VAL (−1, 6, −1), WIZ
(−1.2, 5, −1), ARC (−1.3, 5.5, −1.5), DWF (−1.7, 4.2, −1), KNI (−1.5, 6,
−1), SOR (−1.2, 6.5, −1), JES (−1.5, 6, −1); the secret classes repeat
them (the ogre (−1.7, 3.2, −1)).

**The aim**: `FUN_800857d8` (the aim above) with this tick's search
(distance, aim vector, target) and w = 0.03 (`r2-0x5AF4`): reach 200 ×
0.03 + 15 = 21; T = aim × 21. Then `FUN_80030a9c(50, 0.02, g, y0, T)`
(`r2-0x5B5C`, `r2-0x5AF0`) turns T into a direction for speed 50: with d
= |T horizontal|, (T.x / d, (0.5 × g × d / 50 + (T.y + y0) × 50 / d) / 50,
T.z / d) — g = 10, y0 = −0.5 (`r2-0x5B24`, `r2-0x5AEC`) outside boss
levels, g = 0, y0 = 0 on them.

**The shot** (`FUN_80093150(35, damage, g, type, player, mouth, dir)`,
`r2-0x5B00`): an effect missile of the model `0x8023FDD4[type]`,
velocity = dir × **35** (the lob's slope was worked out for 50, so it
lands short of T — the game's own mismatch), acceleration g down (10, 0
on boss levels), life 3 s (`r2-0x56B8`), flags `0x101000E` (the players'
missile set: it hits monsters and items), kind `0x8012264C[type]` (bits
`0xC` cleared for an element above 4), owner the player:

| who | type | model | damage | kind |
| --- | --- | --- | --- | --- |
| a familiar | the player (0–3) | `FAMILIAR_SPIT` | 0.1 × (level − 25) + 2.5 (`r2-0x5BD0`, `r2-0x5AE8`): 3 at 30, 8 at 80, 9.9 at 99 | 0 |
| the phoenix | 4 | `PHOENIX_FBALL` (`WEAPONS`) | 10 (`r2-0x5B24`) | `0x11` (fire, strong) |

The phoenix's body look plays its action 1 (ATTACK) on the same event
([powers.md](powers.md), "The looks").

Here: `familiars.rs` — the familiar by the hero's level from its effects
bank (the secret classes fall back to the bank of the class eight before
them, as their thrown weapons do), under the hero's root at PDAT `+0x164`
(× 1.2 when put on at level 99), hidden under the phoenix, ATTACK on each
`HeroShot` and READY once it's played; on the tick after each `HeroShot`
the spit or the fireball from the mouth point (the hero's position and
facing on that tick, × its model's scale), aimed with `hero_aim` (along
the facing with the crossbow bit) at reach 21 and `lob`bed for 50,
launched at 35, gravity 10 and drop −0.5 (none on boss levels), radius 1,
through `projectiles::spawn_hero_missile` (flies and hits as a hero
missile for 3 s). The bank's running flipbooks on the models are stepped
by `familiars.rs`, their frames found by name in `WEAPONS` when the bank
only names them (the spit's `PIXIE_<colour>`, `WIZ_HEAD_<colour>`), as
the game resolves such 0 × 0 textures. Stand-in: the aim is the release
tick's search, not the firing tick's.

## Flight (`FUN_80094418`)

Per frame, `dt` = `r13-0x7570`: new position = position + velocity × dt,
then velocity −= acceleration × dt. With no spin and flag `0x20000` the
model is pointed along the velocity (`FUN_800bd77c`); otherwise it's
turned by the spin × dt. Below `r13-0x7278` − 25 − 2 × radius it's gone.
Then, each only while nothing was hit yet, against the segment from the
old to the new position:

1. **Players** (flag 1): `FUN_8002f978` — the first player whose cylinder
   (centre `+0x64`, radius r + `+0x850`, half height r + `+0x854`) the
   segment enters (`FUN_8002fa24`), not the owner. Flag `0x200` (heroes'
   missiles in co-op) shrinks the radius to 0.01 and skips the damage. A
   player with `+0x120 & 0x1020000` reflects it (velocity × −1, it then
   hits monsters, damage capped at 15). Otherwise, if the player's guard
   `+0x8E8` has passed, `FUN_80078560(damage, player, 1, kind, dir)` and,
   for damage > 2 (`r2-0x5674`), guard = now + 0.25 (`r2-0x55e8`). The
   missile stops at the hit point (not with kind `0x100000`, pierce).
2. **Monsters** (flag 8): `FUN_8002f820` — cylinder at monster `+0x54`,
   radius r + `+0x238`, half height r + `+0x23C`, the first in slot order;
   the per-attacker cooldown (monster `+0x2B8 + 4 × player`) is only
   checked once flag `0x1000000` is cleared by a first hit (pierce).
   `FUN_8004e660(damage, monster, owner − 1, kind, point, dir, 2)`; that
   cooldown = now + 0.25 (3.25 when piercing). Stops unless piercing.
3. **Objects** (flag 8): `FUN_80037414` / `FUN_800382c0`.
4. **Items** (flag 2): `FUN_8005ed30` (the item touch test `FUN_8005f0e0`
   with the missile's radius), filtered by `FUN_8009682c` — missiles with
   flag `0x1000` (monsters') pass generators, flag `0x100` passes most
   other items — then `FUN_8005c1c8(damage, item, kind, owner − 1)` and,
   for a player's missile that did damage, `FUN_8002f400` (a potion lying
   on the floor goes off: [effects.md](effects.md), "Shooting potions").
5. **Level** (flag 4): `FUN_8000cfa0` (every surface, nodes `0x23E`) with
   0.5 × radius (`r2-0x5700`). Kind `0x200000` bounces (reflected, upward
   speed × 0.4); otherwise it stops.

A stop (`uVar19` 1: wall/expired, 2–3: a target) plays the record's
wall or hit effect; with a follow-up effect (`+0x24`) and a blast radius
the slot turns into that effect and **bursts**: flags become area mode
(`0x20`) that hit monsters (8), plus players for a monster's bomb
(heroes' keep `0x200`: not players). A missile that runs out its lifetime
just goes.

**Blast**: over the effect's life, with f = time left / lifetime going 1 →
0, the blast radius is R × (0.33 + 1 − f) (`r2-0x56a0`) and the damage ×
1.5 × (f − 0.33) (`r2-0x5690`), nothing once f ≤ 0.33. Everything within
the radius + its own radius is hit once (players through their guard,
monsters by a 3-second cooldown). So something whose surface is `d` from
the burst takes 1.5 × (min(1, 1.33 − d / R) − 0.33) of the damage — about
all of it close in, none at R.

## Monster missiles (`FUN_8002fc08`)

Fired from `FUN_8004dec0` when hit flag `0x10` is set (below), at the
target player's centre `+0x64` (or 20 units ahead, `r2-0x6d30`, with no
target), from the monster's `+0x54`; AIs `0x1C`, `0x1D` and `0x1F` fire on
their ordinary attack flags instead. `FUN_8004e3b0` picks the kind: AI
`0x10`/`0x17` arrow (0), `0x11`/`0x1A` bomb (1), anything else fireball (2).

- Record at `0x80118B68` + type × `0x90` + kind × `0x30` (types 0–28).
- v = record `+0x08` × level `+0xC4`. Fireballs fly straight at the
  target (normalised); arrows and bombs are lobbed like the hero's with
  rise = dy + level `+0xC8` × (−2.5 + random 0…5) − 3.5 (arrows,
  `r2-0x73c4`) or − 5.5 (bombs, `r2-0x73c8`). A negative slope is made 0.
- Only if the direction is within 45° of the monster's facing (0.707,
  `r2-0x73a8`; its matrix `+0x24`/`+0x2C`).
- Let go 2.5 (`r2-0x73a0`) above the centre (fireballs: at it), by type:
  gru arrow 1.5; sor, war fireball 1.5; zom arrow 1.0; pla fireball 1.0,
  arrow 1.5; wrm 2.0 and 2 ahead; imp arrow 0, bomb 0 and 2.5 back; grm 0
  (and kind `0x100000`, flag `0x8000000`). Start 3 (`r2-0x7388`) along the
  direction; a wall or item in between cancels it.
- Damage = record `+0x04` (× 0.5 while `r13-0x7320` < 1: 0.667 while a
  player carries the `0x200` power, `FUN_80054140`). Flags: owner 0 →
  `0x1107` (players, items, walls; pass generators); arrows `0x20000`,
  fireballs `0x40000`.

| types | kind | kind bits | damage | speed | radius | blast | spin | gravity |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| tro gru sor liz zom pla ice ske imp war | arrow | 0 | 10 | 25 | 0.5 | — | — | 30 |
| same | bomb | `0x10` | 10 | 20 | 0.2 | 3 (effect `0x17`) | 1 about Y | 35 |
| wrm | arrow / bomb / fireball | 0 | 5 / 10 / 15 | 15 / 20 / 25 | 0.3 | — | — | 1 |
| dem, gho | fireball | 1 | 15 | 25 | 0.3 | — | — | 1 |
| pla | fireball | 0 | 15 | 25 | 0.3 | — | — | 1 |
| sor, war | fireball | 3 | 20 | 20 | 0.2 | — | — | 1 |
| grm | fireball | 2 | 25 | 80 | 2 | — | — | 0 |

(Type 28, unused, has three records with damage 50, speed 40.)

## The throwing AIs

Special variants: tier 4 (`A`) gets AI `0x17`, tier 5 (`B`) `0x11`
([monsters.md](monsters.md)); placed monsters use `0x10`, `0x11`, `0x17`,
`0x1A`. Dispatch in `FUN_800467bc`.

- **`0x11`, `0x17`** (`FUN_8004ba7c`): heading = angle to the target
  player's feet (turned to at the usual rate, never moving). With a
  target, near the screen (`+0x2DC`), within awareness and −10 ≤ monster Y
  − player Y ≤ 10 (`r2-0x6dd0`, `r2-0x6f08`): if the timer `+0x320` < 1,
  ask for THROW1 (`0x18`), else count it down by the fields elapsed.
- **`0x10`, `0x1A`** (`FUN_8004a6f4`): the same, and it backs off: past
  the conditions, `+0x31C` (backing off) is set once the target is within
  0.6 × awareness (`r2-0x6dc8`) and cleared beyond 0.8 × (`r2-0x6de0`);
  backing off, it asks for RUNATTACK1 (`0x16`) and moves at 0.8 × its speed
  (`FUN_8004cc84`) directly away (+ π, plus an angle from `0x8011b9c0`
  after wall bumps, `+0x358`), still facing the player. A leader monster
  (`r13-0x73b8`) can turn it into AI `0x18` instead.
- The timer starts at random(10) fields (`FUN_800502fc`) and isn't reset,
  so they keep throwing.

Requests go through `FUN_800ad5ec`: priority table `0x80126710` (READY
100, locomotion 200, attacks and throws 300, hits 400–460, DEATH 999), and
attack/throw requests (`0xC`–`0x14`, `0x18`–`0x1A`) are refused while the
throw pause `+0x37C` > 0.

**Animation** (`FUN_800ab110`): THROW1/THROW2 → THROWF (mode 0: at the
end); THROWF → THROW2 if THROW1 is asked again, ATTTOREADY if READY,
else the request; RUNATTACK1 → RUNATTACK2 while asked; ATTTOREADY → READY.
Missing THROW2 falls back to THROW1, ATTTOREADY to READY. Switching into
THROWF (from THROW1/2) or into RUNATTACK2 sets hit flag `0x10`: the throw.

**Throw pause**: on switching into THROW1, THROW2 or THROWF, owed =
`+0x378` × level `+0xC0` + carry (`+0x380`); every whole unit above 1 goes
into the pause `+0x37C`, the rest is carried; a pause of 1 or more gets
the new clip's length (`+0x80`, frames / 30) added. `+0x37C` counts down
in seconds (`FUN_8004cfe0`). `+0x378` is 1 (`r2-0x6e30`, `FUN_8004ffbc`),
or for placed monsters their parameter (`+0xEC`, placement `+0x38`): 1 →
0, n > 1 → n / 10 (`FUN_80060100`). With 1 × 1 the first throw is free
and every later throw action pauses a second plus its clip — about a
throw every two seconds.

## In this rewrite

- `HeroShot` (feet, facing, aim, whether a target was found, strike bits,
  seconds since the attack started) from `player.rs`; `MonsterShot`
  (monster, type, AI, centre, target point, facing, a random 0–1) from
  `monsters.rs`; both launched in `FixedUpdate` after the monsters, then
  every `Projectile` flies. `Hit` messages for monsters, generators,
  breakables and the practice dummy; for the hero `Player::take_blow`
  (armour, armour powers, the reaction) with the missile's kind, pushed
  along its flight — a burst's blow as the effects' blasts land
  (`effects::blast_on_hero`: under 5 it loses kinds `0x170` and gains
  `0x1000000`; pushed by 0.25 × the way out from its centre, across the
  floor) — then `DamagePlayer`.
- Level tuning `+0xC0`/`+0xC4`/`+0xC8` are `LevelTuning::throw_timing`,
  `missile_speed`, `missile_spread` (0 read as 1, as the other scales);
  PDAT `+0x5C`/`+0x158` are `PlayerStats::throw_offset`,
  `power_throw_offset`.
- `GDL_THROWER=<distance>[,<ai>[,<tier>]]` puts a grunt that far in front
  of the hero (default AI `0x17`, tier 4) to watch the throwers.

### Stand-ins and differences

- The floor-slope elevation `+0x8BC` is taken as 0 (flat).
- The monster's `+0x54` centre is taken as feet + centre height (as the
  player's `+0x64` is; not traced for monsters); the dummy and other
  targets use their own extent.
- Generators and breakables are hit as an upright cylinder of their
  target radius and height (not the item touch test); monster missiles
  pass every item but the safe rocks (the game's filter lets them past
  the rest), and skip the item check at release. Every missile but
  magic stops at a standing safe rock and hits it ([critters.md](
  critters.md), "Safe rocks").
- Hits go to the nearest target along the segment, not the first in slot
  order; per-attacker cooldowns aren't kept (only piercing needs them).
- No wall/hit/burst effects, sparks or hit sounds (the release's sound
  is played); the burst is applied at once with the analytic falloff
  instead of over the effect's life.
- Weapon powers ([items.md](items.md), "Timed powerups"): the throw
  starts from the hero's weapon bits (`+0x11C`); `0x80000` / `0x400000`
  fan it into three / five missiles turned 0°, ±15° (±30°) by the tables
  at `0x80111580` (cosines) / `0x80111594` (sines), living 2 s
  (`r2-0x73e4`) instead of 3 (`FUN_80030094`'s loop: it stops after the
  first without `0x480000`, after the third without `0x400000`); with
  `0x100000` and without `0x2000000` the throw uses the record at
  `0x80119b28` — kind `0x20`, radius 5, no gravity or spin — and pierces:
  a hit doesn't stop it, and it hits each target once (the game's
  per-attacker cooldown of 3.25 s outlives it); `0x200000` bounces off the
  level (`FUN_800bdaf8` reflects the velocity about the surface normal and
  a rise keeps 0.4 of itself) — a missile already leaving the surface it
  touches flies on (our sweep can touch it again). Stand-ins: the bounce
  leaves the lifetime alone (the game trims what's left, constants not
  traced); the spread's first missile's effect (the record's `+0x2C`
  becomes 6 / 7), the element trails (`0x8023fd34`), the magic classes'
  element models and the streaks (the crossbow's white one included)
  aren't done; the time-slow damage halving isn't; no push on missile
  hits on monsters. A bolt that strikes an item at release and flies on
  can meet it again at once (whether the game's item test would isn't
  traced). The reflect shield is (`fly`: velocity ×
  −1, moved on at once, damage at most 15 (`r2-0x5570`), it then hits
  monsters and objects and passes the hero it glanced off; `S_RICOCHET`
  at most once a second) — but its life isn't capped at 10 s more
  (`r2-0x5598`), and it still passes items as monsters' missiles do.
- The kiting thrower's wall-bump angles and the leader logic aren't done;
  AIs `0x1C`/`0x1D`/`0x1F` (fireball casters) move like the chasers (their own
  movement isn't traced) and fire a fireball on each attack blow.
- A monster without the throw clips asked for doesn't throw (the game
  would play READY's animation in their place).
- The throw pause's added clip length (`+0x80`) is taken to be the new
  clip's frame count.
- The alternate characters use the missile record of the class eight
  before them: the game indexes it by the record's `+0x08`, the class
  less 8 for them ([combat.md](combat.md), "The action state machine").

## The throw's target (`FUN_800864b0`)

The throw's aim vector comes from its own target search: from the release
point to the target's **centre** (a monster's `+0x54`, a critter's centre,
an item's), within 30 units (`r2-0x5b28`) — 200 (`r2-0x5a58`) on a boss
level — then `FUN_800857d8` adjusts it (above). So a throw tilts up or down
to meet a target on higher or lower ground. The runtime aims from the
hero's centre height at the found target's centre (feet + half its
height), using the 200-unit range for throws on boss levels.
