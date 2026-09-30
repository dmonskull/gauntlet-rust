# Armour and special power-ups

What the armour bits (player `+0x120`) and special bits (`+0x124`) do,
decoded from the Ghidra dump with the constants read from `main.dol`
(`r2 = 0x8034D100`, `r13 = 0x8034B4E0`). How a power-up becomes a slot and
how the slots add up to the bits each tick is in [items.md](items.md),
"Timed powerups"; the weapon bits (`+0x11C`) on missiles are in
[projectiles.md](projectiles.md). Player fields are offsets into the
player record (`0x802754C0` + player × `0x335C`).

**Confirmed** means read in the decompile (and, where noted, the machine
code), with the constants read from the DOL. **Not traced** marks what
was seen but not followed; **unconfirmed** marks a reading of the code
that could be wrong.

Names come from the game's own data: the pickup hints (the hint table
`0x80124668` → `TEXT/ENGLISH.ROM` group and `VOICE1`/`VOICE2` line), the
models' names, the effect table (`0x801218E0`) and the sound catalog.

## Summary

| bits | power (hint text) | what it does |
| --- | --- | --- |
| armour `0x10000` | LIMITED INVULNERABILITY | no damage, no hit reactions; silver chrome |
| armour `0x110000` | (INVULG, same hint) | blows over 1 **heal** a tenth of themselves; gold chrome |
| armour `0x20000` | REFLECTIVE SHIELD | flying effects bounce back off the hero at monsters |
| armour `0x200000` | FIRE WALL SHIELD | 3 fire damage every tick to what the hero touches |
| armour `0x400000` | LIGHTNING SHIELD | 20 lightning damage (once a second per target) to what the hero touches, with a bolt |
| armour `0x80000` | ANTI DEATH POWER (halo) | Death can't drain the hero and flees it; a Death in front of the hero is drained back |
| armour `0x2008` | GAS MASK | poison does nothing; acid × 0.5 (light × 2) |
| special `0x1` | LIMITED LEVITATION | floats; no damage tiles, no blows from small (or shrunk) monsters |
| special `0x2` | X RAY GLASSES | the nearest closed container shows what's inside |
| special `0x4` | LIMITED INVISIBILITY | see-through; monsters and critters (not bosses) don't pick the hero |
| special `0x8` | STOPPED TIME | monsters, critters, generators and damage tiles stop; an hourglass counts down |
| special `0x10`/`0x20`/`0x40` | FIRE / ACID / LIGHTNING BREATH | attacks become a breath: 40 damage, 20 units, 5 uses |
| special `0x80` | PHOENIX FAMILIAR | a phoenix rides the hero and fires with each throw |
| special `0x100` | LIMITED GROWTH | the hero × 1.3; contact blows × 2 |
| special `0x200` | "HAS SHRUNK ENEMIES" | every monster × 0.667 (not on boss levels), takes × 2 and deals × 0.5 |
| special `0x400` | "IS NOW POJO" | the hero becomes the Pojo |
| special `0x1000`/`0x2000` | HORNS / MASK OF SKORNE | attacks become Skorne's breath (50 damage) |
| special `0x4000`/`0x8000` | SKORNE'S GAUNTLET (right / left) | attacks throw Skorne's acid / lightning bolts |
| special `0x100000` | Mikey Powerup | a decoy state (not traced) |
| special `0x200000` | Hand Of Death | a monster's blow on the hero hurts the monster instead |
| special `0x400000` | Health Vampire | the same, and the hero heals by the blow |

Every armour and special power above down to the Pojo is placed on the
disc ([items.md](items.md), "Timed powerups", with the durations); Mikey,
Hand Of Death and Health Vampire are sold in the shop (`SHPDATA/SHOP.WAD`: `SHP_MIKEY`,
`SHP_HANDOFDEATH`, `SHP_HEALTHVAMPIRE`, beside most of the placed ones).
Where Skorne's horns, mask and gauntlets come from wasn't traced.

## Timing (`FUN_8007c4f0`)

Confirmed, and more exact than items.md: a slot's time (`+0x130` + 0x10 ×
slot) runs down only outside the tower (realm 13), outside camera cuts
(`r13-0x774c`) and while `r13-0x7340` is 0:

- not a boss level (`r13-0x7764` < 0): by the frame's seconds
  (`r13-0x7570`);
- a boss level: × 3 (`r2-0x5d70`) while the boss is awake (`r13-0x7788`)
  and not dead (`r13-0x7784`), and **not at all** before it wakes or
  after it dies.

A counted power (time −1: the breaths, the crossbow, the hammer) is spent
by `FUN_8007ed38(1, player, subtype, bits)`: the first slot of that
subtype with any of the bits loses 1 from its amount (`+0x138`; not in the
tower); at 0 the slot ends; a negative amount never runs out.

The powers' models on the body fade out in their last second: over the
slots with special bits `0x7004F1` or armour `0x200000`, the longest time
(99999 for a counted one) sets the body model's transparency to
255 × (1 − time) once it's under 1 s.

Here: `player_state::power_clock` gives the rate — 0 in the tower and
camera cuts, × 3 on a boss level while the boss is awake (`BossWatch`)
and not dead, 0 before and after — and `PlayerState::tick_powers` runs
the clocks down by it. `r13-0x7340` isn't traced. Counted powers aren't
spent yet.

## The looks: where they hang

Every tick `FUN_8007c4f0` attaches (`FUN_800b89b0(model, 0, node,
flags)`, found by name with `FUN_800b8658`), swaps (`FUN_800b8aa0`, the
new model object on the same instance) or frees one model per slot. The
flags are ORed into the new instance's draw flags (`+0x60`): `0x8000` is
the extra texture stage of [rendering.md](rendering.md) (the
environment-map path — Skorne's pieces shine), `0x1000` isn't
identified, `0x10` and `0x800` are outside the draw mask. The nodes are
found when the hero is built (`FUN_8007ae84`): `<class>` + the class's
left wrist → `+0x6CC`, right wrist → `+0x6D0`, `<class>HEAD` → `+0x6D4`
(`r2-0x5DB8` "%s%s", `r2-0x5DB0` "%sHEAD"; `<class>` is the skeleton's
name, `+0x6C0`). The wrist names are per class (`0x8011F90C` left,
`0x8011F94C` right, by class index): `L_WRIST`/`R_WRIST` for classes
0–3 and 8–11, `LEFTHAND`/`RIGHTHAN` for 4–6 and 12–14, `LEFTHAND`/`RHEND`
for the jester (7) and 15. The models are in
`POWERUPS/objects.ngc` (and `WEAPONS`); the body's atrees come from the
`POWERUPS` bank (`r13-0x718C`) or `WEAPONS` (`r13-0x7188`), loaded by
`FUN_80030c38`.

| slot | first that applies | node, flags |
| --- | --- | --- |
| left wrist `+0x72C` | `BOSSGAUNTL` (special `0x8000`), `RF_SHLD` (armour `0x20000`), `FW_SHLD` (`0x200000`), `L_SHLD` (`0x400000`) | L_WRIST; `0x9010` for the gauntlet, else `0x810` |
| right wrist `+0x730` | `BOSSGAUNTR` (special `0x4000`), `SUPERXBOW` (weapon `0x100000`), `HAMMER_HD` (weapon `0x10000000`) | R_WRIST; `0x9010`, `0x810`, `0x10`; the held weapon (`+0x6E0`) is hidden while one is on |
| head `+0x734` | `BOSSHORNS` (special `0x1000`), `BOSSMASK` (`0x2000`), `HEAD_HALO` (armour `0x80000`), `HEAD_GAS` (armour `0x2000`), `HEAD_XRAY` (special `0x2`) | HEAD; `0x9010` for Skorne's, else `0x810` |
| head 2 `+0x968` | `HEAD_HANDOFDEATH` (special `0x200000`), `HEAD_HEALTHVAMP` (`0x400000`) | HEAD, `0x810`; the pickup sparkle (`FUN_8009176c(hero, 1)`) when it comes on |
| body `+0x790` | `POJO` (special `0x400`), `FW_SHLD_ACTIVE` (armour `0x200000` while the action is SHIELD_RUN `0x16`), `PHOENIX` (`0x80`), `HEAD_BREATHEF` (`0x10`), `HEAD_BREATHEA` (`0x20`), `HEAD_BREATHEE` (`0x40`), `WINGS` (`0x1`) | an atree instance (`FUN_80012f78`, flags `0x800`) on the hero model (`+0x74`; the wings on its `+0x78` → `+0x78` child, the breaths on the head node) |

- While a left-wrist model is on, the valkyrie's and knight's (class 1,
  5) left-wrist child and the jester's (7) wrist get bit 1 of `+0x60`
  (their own shield or prop hidden — unconfirmed).
- Without a right-wrist model the weapon's element glows
  (`WEAP_HOLD_RED/BLU/YEL/GRE`, `0x8023FD84`, from `WEAPONS`) on the
  weapon — the weapon powers' part, for projectiles.md.
- The body slot's animation (`FUN_80011104(slot, action, mode)`): the
  Pojo's by the hero's action (`+0x208`): SHOVE `0x08`, WALK1–RUN2
  `0x11`–`0x14`, SHIELD_RUN `0x16`, PUSH `0x19`, PUSHED `0x1A` → 1 (RUN);
  ATTBREATHE `0x6E` → 3 (ATTPWR); HITREACT `0x1B`, `0x7F`–`0x83` (STUN1,
  WEBREACT, the HITREACTs, FALLDOWN), FALLFRNT `0x85`, FLYUP `0x87`,
  GRABBED `0x94` → 4 (HIT; not GETUP `0x84` or GETUP2 `0x86`); DEATH
  `0x7E` → 5; a peck → 2 (ATTACK); else 0 (READY). The peck is `+0x900`
  bit `0x20000000`, set by the player update when the attack's blow event
  (`+0x900` bit 1) comes with the Pojo, and cleared here. The others play
  their action 1 on a throw event (`+0x900` bit `0x10000000`, set when a
  throw lets go and cleared by the next player update after the phoenix
  fires) while the short `+0x7A0` is above 1 (their action count,
  unconfirmed), else action 0. Mode 2 goes with 2–5 and the throw's 1,
  and whenever the short `+0x7A2` is 0; 0 otherwise (not traced).
- The second head slot latches each model when it comes on (`+0xA1E` the
  Hand of Death, `+0xA20` the Health Vampire; both clear when both powers
  are off): a model is swapped in only when its power comes on afresh.
- The Pojo hides the hero's skeleton (flag 2 on its root node instance,
  `**(+0x7C)`: the draw skips the subtree, the wrist and head models
  with it); a right-wrist model hides the held weapon (`+0x6E0`) the same
  way.
- Instances link parent `+0x74`, first child `+0x78`, next sibling
  `+0x7C` (`FUN_800bb084`); the hero model's first child is the
  skeleton's root node, so the wings hang on that node's first child
  (`ROOT_PELVIS` for the warrior).
- Model scale (`FUN_800ba6f8` on `+0x74`): the ogre (weapon type `+0x0C`
  = 12, OGR) 1.6
  (`r2-0x5E74`); grown 1.3 (`r2-0x5D28`); at level 99 1.2 (`r2-0x5FE0`);
  else 1. Above level 98 the head node is × 1.5 (`r2-0x5FE8`).

Here: `power_looks.rs` — every tick after the powers, the four object
slots and the body slot from `PlayerState.bits` as the table (the head 2
latches too), on the class's nodes (`Animator::node`), the body's atree
on the hero's root, head or root's first child; the held weapon hidden
with a right-wrist model, the skeleton with the Pojo; the Pojo's actions
by the hero's (`Player::actions`), the others' action 1 on a `HeroShot`
when they have a second; the body faded (`fade.rs`) in its power's last
second. Stand-ins: mode 2 is read as "plays through once before the next
action takes over"; the peck is a landed hand blow (`combat::Hit` from
the hero, not ranged), not the blow event itself. Not done: the attach
flags (no environment map; drawn with the objects' own materials), the
class 1/5/7 `+0x60` bits, the weapon glows, the head 2 pickup sparkle, the
model scales. `FW_SHLD_ACTIVE` waits for SHIELD_RUN, which the action
machine doesn't produce yet (the shields' actions, below).

## The resistance routine (`FUN_8002f58c`)

The hero's damage (`FUN_80078560`, [combat.md](combat.md)) passes the
blow through `FUN_8002f58c(armour stat +0x108, &damage, &kind, armour bits
+0x120)`, the same routine monsters use with their type's resistances.
In order (confirmed, also in the machine code for the first test):

1. bits `0x100000`: a blow of at most 1 (`r2-0x7408`) does nothing; a
   bigger one becomes −0.1 × itself (`r2-0x7400`);
2. bits `0x10000`, or `0x1000` with kind `0x200`, or `0x2000` with kind
   `0x800`: nothing;
3. bits `0x40000` clear the knockback kinds (`& ~0x10170`);
4. bits `0x10` with kind `0x200`: × the resist factor;
5. unless kind `0x200` or `0x800`: armour is taken off (a blow no bigger
   than it does nothing);
6. what's left, if above 0, by the kind's element (`& 0xF`):

| element | × resist | nothing | × weak | × otherwise |
| --- | --- | --- | --- | --- |
| 1 fire | bit `1` | `0x100` | bit `2` or `0x200` | neutral |
| 2 lightning | `2` | `0x200` | `1` or `0x100` | neutral |
| 3 light | `4` | `0x400` | `8` or `0x800` | neutral |
| 4 acid | `8` | `0x800` | `4` or `0x400` | neutral |

The factors (resist, neutral, weak) are 0.5, 1.5, 2.0 (`r2-0x73E8`,
`-0x73EC`, `-0x73E4`) outside boss levels and 0.75, 1.25, 1.5 (`r2-0x73F4`,
`-0x73F0`, `-0x73EC`) on them — so any elemental blow does at least ×
1.25. Kind `0x800` is poison (poisoned food, the Pojo's chicken, the
poison clouds); `0x200` is magic.

Here: `damage.rs::resist`, for every blow: on heroes through
`Player::take_blow` (armour stat and armour bits), on monsters (armour 0,
1 for Death; no resistance bits — so elemental blows do × 1.5 / 1.25, and
a hero's blow at least 1), on critters (their type's armour and
resistances). The hero's routine around it (`FUN_80078560`): a
non-negative result is queued for the reaction (blows of 2 or less,
`r2-0x5e7c`, lose the knockback kinds `0x10170`; a blow armour stopped
still carries its stun); a negative one heals and draws none. Not yet:
poisoned food (`items.rs` takes it straight off health, not as kind
`0x800` through the routine), defending, the `+0x3C` bit-1 test for kind
`0x40000000`.

## Armour bits (`+0x120`)

### `0x10000` invulnerability, `0x100000` gold

INVULICON gives `0x10000` (30 s), INVULGICON `0x110000` (25 s); both have
the hint INVULNERABILITY ("LIMITED INVULNERABILITY", `S_INVULVOX`).
Confirmed:

- **Damage** (above): `0x10000` takes every blow to 0. With `0x100000` a
  blow over 1 heals the hero by a tenth of it: `FUN_80078560` subtracts
  the negative damage from health (`+0x1EB4`) directly (no cap there —
  whether health is capped elsewhere wasn't checked) and skips the
  accumulated reaction; a blow of 1 or less does nothing.
- **No reactions**: the reaction picker (`FUN_80085ca8`) does nothing with
  `0x10000` (no knockback, stun or flash, since no damage).
- **Footsteps** (`FUN_8009e804` from `FUN_80080d3c`): `S_STEPMET1/2`
  whatever the floor.
- **No low-health warning** (`FUN_80077f50`: `S_WARN` skipped with
  `0x110000`) and no `S_<CLS>DIE1` for mode-3 damage (`FUN_8009f198`).
- **Chrome** (`FUN_8007c4f0`): with the longest `0x10000` slot's time `t`,
  while `t` < 0, `t` > 3 (`r2-0x5D70`) or (int)(8 × `t`, `r2-0x5D58`) is
  odd, `FUN_80090a00(1, player +0x7DC, texture, 1, 1)` re-arms the hero's
  timed texture effect with `CHROMEGOLD` (`r13-0x6F08`) if `0x100000` is
  set, else `CHROMESILVER` (`r13-0x6F04`) — so it blinks off and on every
  1/8 s in its last 3 seconds. The textures are looked up by name at
  load (`FUN_800972dc`, in the bank `r13-0x7EB8`). The effect's draw
  (`FUN_80090aec`) uses override mode −3 for these two (−4 for every other
  texture): the chrome texture **replaces** every submesh's texture, with
  draw flag `0x80000`, which takes the same path in `FUN_800c5894` as a
  material's `0x20000` (`FUN_800c60a4`, texture coordinates from a matrix
  it builds) — most likely an environment map; the matrix isn't traced.
  The slot `+0x7DC` is the one hit flashes use ([rendering.md](
  rendering.md), "Texture overrides").

Here: the damage and the heal (`resist`), no reactions
(`player.rs::tick`), no `S_WARN`; the chrome (`player.rs::show_chrome`,
`fade.rs::BodyLook`, `level.wgsl` mode 3) re-armed as above with the
blink, shown for the tick it's armed and the next like a flash. The
texture coordinates are the normal along the camera's right and up
(the rows `FUN_800c60a4` builds: the camera's `(1, 0, 0)` and `(0, 1,
0)` through its matrix and the model's, normalised); their scale and
offset in the GX matrix aren't traced — the software path in
`FUN_800c48c0` uses them as they are, so 1 per unit, no offset, and the
32×32 texture repeats. Footsteps (no footstep sounds are played yet)
and the death voice aren't done.

### `0x20000` reflect shield

`RF_SHLD_ICON` (hint REFLECTARMOR, "REFLECTIVE SHIELD",
`S_REFLECTSHVOX`). Confirmed:

- In the effects/missiles update (`FUN_80094418`), a flying effect that
  hits players (flag 1) and finds a player (`FUN_8002f978`) with armour
  `0x1020000` doesn't hurt it: `S_RICOCHET` (sound `0x41`, at most once a
  second, `FUN_8009e7b4`), its flags gain 8 (it hits monsters) and lose
  `0x400`/`0x800`, its velocity is × −1 (`r2-0x5708`) and it moves on by
  that at once ([projectiles.md](projectiles.md): its damage is then
  capped at 15). Monster missiles, fireballs and the like — whatever
  runs through that branch; blasts use another.
- `RF_SHLD` on the left wrist; the shield actions below.
- Bit `0x1000000` reflects too; no power-up on the disc sets it.

### `0x200000` fire wall shield, `0x400000` lightning shield

`FW_SHLD_ICON` / `L_SHLD_ICON` (30 s; hints FIRESHIELDMSG "FIRE WALL
SHIELD", ELECSHIELDMSG "LIGHTNING SHIELD"). Confirmed, in the player
update `FUN_80080d3c`:

The hero's target search (`FUN_800864b0`, the one attacks use, toward
where the hero heads) gives a target and its distance; while the hero
isn't reacting to a hit and the target is within 1 (`r2-0x5BE0`) of its
surface:

- **Fire wall**: if the intent is standing (1, turned into walking 8),
  walking or running (8, `0xD`): `FUN_8008625c(3 (r2-0x5BB8), cooldown 0,
  player, target, kind 1 (fire), point, 0)` — 3 fire damage every tick
  (no per-attacker cooldown), in place of the walk-into attack.
- **Lightning** (only when the fire wall didn't apply — with both, the
  fire wall wins): `FUN_8008625c(20
  (r2-0x5B70), cooldown 1 s (r2-0x5C70), …, kind 0x22 (lightning,
  heavy))`; when it lands (≥ 0) the effect `0x37` `L_SHLD_ACTIVE` spawns
  (`FUN_8009333c(0x37, 0)`) turned toward the target (`FUN_800bd77c`)
  at the left wrist node.
- The damage goes through the contact routine as a blow: monsters'
  resistances, the grow power's × 2 (below).

Looks: `FW_SHLD` / `L_SHLD` on the left wrist; with the fire wall,
`FW_SHLD_ACTIVE` (`WEAPONS`) on the body while the action is SHIELD_RUN.
The pickup plays `S_PICKUPSHIELD` for the fire wall only ([items.md](
items.md)). `S_FIRESHIELD`, `S_LIGHTNGSHIELD` and `S_SHIELD1`–`4` are in
the catalog, but no call to them was found (not traced).

### The shields' actions

Confirmed (`FUN_800ab898`, the action machine): with any of `0x620000`,
READY becomes SHIELD_READY (`0x15`) and WALK1–RUN2 (`0x11`–`0x14`) become
SHIELD_RUN (`0x16`).

Here (`player.rs`, `projectiles.rs`): the reflect in `fly`; the fire wall
and lightning blows on the searched target within 1 (`WALK_INTO`) of the
hero while no reaction was picked this tick, in place of walking into it
(lightning's 1 s cooldown kept per target on the hero), with
`L_SHLD_ACTIVE` played from the left wrist (`L_WRIST`) toward the target
— turned about Y only; the SHIELD_READY / SHIELD_RUN swap where the next
action's clip is played (`shield_action`). The models on the arm are
the looks' (above).

### `0x80000` halo (anti-Death)

`HALOICON` (120 s; hint HALO "ANTI DEATH POWER", `S_ANTIDEATHVOX`).
Death is monster type `0x1E`. Confirmed:

- **Death doesn't drain the hero** (`FUN_800460a8`, Death's grab: its
  drain — `FUN_80078560` with kind `0x1000`, or the experience drain
  `FUN_80076144` — is skipped when the grabbed hero, Death's `+0x284`,
  has the halo).
- **Death doesn't pick it** (`FUN_80051660`): a haloed hero isn't a
  target for Death; the nearest one within its range goes into Death's
  `+0x328` instead, and with no target Death runs straight away from it
  (`FUN_80047350`: heading = angle to the hero + π, `FUN_8004cc84(1.0,
  …)`).
- **Draining Death** (`FUN_80080d3c`): when the target search gives a
  Death less than 90° off the facing (`r2-0x5B78`; no distance test of
  its own beyond the search's, unconfirmed), every tick
  `FUN_8004e660(1, Death, player, 0, 0, 0, 1)`: any blow takes exactly 1
  (`r2-0x6F10`) of Death's hit points, and with the halo the hero gains
  it — 1 health, or experience (`FUN_80076144(player, 1, −2)`) from a
  Death whose `+0x206` is 2 (the one that drains experience, hint
  DEATHDRAINEXP). Without the halo a blow on Death gives hint 0 ("USE
  MAGIC TO KILL DEATH"). The first drain of a halo plays `S_HALO`
  (`FUN_8009e990`, flag `+0x95E`, cleared when the halo ends); each tick
  `S_DEATHSUCK` loops at the hero (`FUN_800a045c`) and `S_DEATHDIE` at
  Death (`FUN_800a035c`); `+0x128` bits 1 (and 2 for an experience Death)
  make `FUN_8007c4f0` put the drain effect on the hero
  (`FUN_800911c4(model, 1 or 2, 0x10)`: runtime effect `0x5F` or `0x60`,
  names not traced) and stop `S_DEATHSUCK` when it ends.
- `HEAD_HALO` on the head.

To build: Death's grab and drain (monsters.md lists Death as not run),
then the halo's three checks.

### `0x2008` gas mask

`GASMASK` (60 s; hint GASMASK, `S_GASMASK`). Confirmed through the
resistance routine: `0x2000` makes kind `0x800` (poison) do nothing —
poison clouds, poisoned food, the Pojo's chicken; `0x8` is the acid
resist bit (acid × 0.5 / 0.75) and, by the same table, light's weakness
(× 2.0 / 1.5 instead of 1.5 / 1.25). `HEAD_GAS` on the head.

## Special bits (`+0x124`)

### `0x1` levitation

`LEVITICON` (hint LEVITATION, `S_LEVVOX`; pickup `S_LEVITATEUP`).
Confirmed:

- no damage tiles (`FUN_8005d71c`, done here: `hazards.rs`);
- no damage from blows of kind `0x40000000` (`FUN_80078560`): small
  monsters' blows, and every monster's blow while the enemies are shrunk
  (below);
- the hero's model is lifted 1.9375 (`r2-0x5C28`) above its feet
  (`FUN_80080d3c`: the animation object `+0x7C`'s height);
- no footsteps (`FUN_8009e804` skipped);
- `WINGS` on the body; when it ends, `S_LEVITATEDOWN` (`FUN_8009cd28`).

No other read of the bit was found (every `+0x124 & 1` in the dump).

Here: the tiles (`hazards.rs`) and the blows (`Player::take_blow`, the
`+0x124 & 1` test on kind `0x40000000` in `FUN_80078560`). The lift and
`S_LEVITATEDOWN` aren't done (footsteps aren't played at all).

### `0x2` x-ray

`XRAYICON` (hint SEETHRU "X RAY GLASSES", `S_XRAYVOX`). Confirmed
(`FUN_8007e678`, each tick while it's on):

- `FUN_8007ebbc` finds the nearest item within 10 (`r2-0x5FA4`) of the
  hero that is a container (class 2), in this game and not opened, whose
  contents type (`+0xDC`) is a powerup (class 1), a placed monster (class
  4) or a random pick (−1, resolved with `FUN_800675e4` — whether it's the
  pick the container later releases isn't traced), and passes
  `FUN_800bb8e4` with 2 × its type's first extent (presumably on screen);
  one another hero x-rays is skipped.
- The container goes see-through (transparency 192) and is re-parented
  under a holder with the contents' model inside it, at 0.65
  (`r2-0x5D18`): `DEATH_ICON` for a monster (`r13-0x71C0`), `KEYRING`
  for more than one key (`r13-0x71C4`), else the contents' own model;
  `S_XRAY` (`FUN_8009e950`) when it changes. When nothing is found or the
  power ends, the container is put back.
- `HEAD_XRAY` on the head.

### `0x4` invisibility

`INVISICON` (hint INVISIBILITY "LIMITED INVISIBILITY", `S_INVISVOX`).
Confirmed:

- **Looks** (`FUN_8007c4f0`): the hero model's transparency is 160 + 16 ×
  sin(2π `t`) (`FUN_800e9cbc` is fdlibm's sine; `r2-0x5D50` = 16), `t` the
  longest invisibility slot's time; in its last 3 s it's fully visible on
  every other 1/8 s. (During the sorceress's combo, COMBOSOR `0x92`, the
  transparency is 95 whatever the powers.)
- **Monsters lose the hero**: `FUN_80051660` skips an invisible hero as a
  target — also as the hero that last hurt them (`r13-0x6FD4`).
- **Critters** (`0x80036b88`) skip it too; **bosses** (`0x80036ed4`,
  types whose `DESC +0x20` is 4) still pick it.

Here: the looks (`player.rs::show_body_looks`, the model's meshes through
`fade.rs::BodyLook`; the shadow stays), monsters' target pick
(`monsters.rs::select_target`: a target they hold is dropped at their
next pick, one tick in eight) and critters' tracking (`critters.rs::track`,
bosses excepted). Not done: the sorceress's combo's 95, the last hero to
hurt a monster (`r13-0x6FD4`). Monsters still swing at a hero they walk
into (that test doesn't look at targets).

### `0x8` time stop

`TIMEICON` (hint TIMESTOP "STOPPED TIME", `S_STOPPEDVOX`). Confirmed:
`FUN_80054140` (every frame) sets the global `r13-0x731C` while any
playing hero has the bit, and `FUN_8007c4f0` sets the hero's `+0x960`.

- **Monsters** (`FUN_8004cfe0`): a monster in state 1 (not the boss)
  skips its targeting, blows, reactions, move and action choice — it
  stands frozen.
- **Critters** (`FUN_800395bc`): no new pattern or move is chosen and the
  move's steps don't run (except moves of kind `0x11`).
- **Generators** don't spawn (`FUN_8004f41c`).
- **Damage tiles** reset to off ([mechanics.md](mechanics.md)) and the
  item update skips their class (8, `FUN_800606e8`).
- Dying monsters (state 8) stay targets for critters' blows and a
  target search while it lasts (`FUN_80034c14`, `FUN_80087258`;
  unconfirmed reading).
- **The hourglass** (`FUN_8009fee8`, from the player update): with a hero
  holding it, the level timer's sprites (`0x80257000`: `TIMER_SAND`,
  `SAND_ANIM`, built by `FUN_800553b4`) show the time-stop slot's time left
  (`FUN_800552a4`), and `S_HOURGLASS` (`0x53`, looping) plays at that hero;
  otherwise it's stopped and the sprites hidden (except in realm 12).

Not traced: `FUN_800a00ec` (music) takes another value while it's on, as
during cuts; `FUN_800a7ff8` (animated objects) holds those without flag
`0x100` in their `+0x0C`.

`r13-0x7788`, mentioned with the damage halving elsewhere, is the boss
being awake (the slots' × 3, above); the halving of monster damage is the
shrink power's (`r13-0x7320`, below).

### `0x10`, `0x20`, `0x40` breaths

`BREATHEF/A/E_ICON` (5 uses; hints FIREBREATHE, ACIDBREATHE, ELECBREATHE:
"FIRE BREATH", "ACID BREATH", "LIGHTNING BREATH"). Confirmed:

- **The attack** (`FUN_80080d3c`): with a breath (and no walk-into
  attack), every attack intent — quick, power, the four strafing attacks —
  asks for ATTBREATHE (`0x6E`). The same table gives the other special
  attacks: Skorne's horns and mask → ATTBREATHE, left gauntlet → ATTFIREL
  (`0x67`), right gauntlet → ATTFIRELR (`0x68`), crossbow (weapon
  `0x100000`) → SSHOT1 (`0x6B`), hammer (weapon `0x10000000`) → ATTCHOP
  (`0x70`).
- **The breath**: on the action's hit event (`+0x900` bit `0x1000000`) an
  effect is spawned (`FUN_8009418c(0, fx, 0, 0x2A, 0x800)`) on the head
  node and given (`FUN_80093768`) damage 40 (`r2-0x5B58`), radius 20
  (`r2-0x5B70`), kind fire `0x21` / acid `0x24` / lightning `0x22` (with
  `0x20`, heavy): `FIREBREATHE` (`0x34`), `ACIDBREATHE` (`0x35`),
  `ELECBREATHE` (`0x36`); its `+0x88` = 0.866 (`r2-0x5AF8`, cos 30° —
  presumably the cone's half-angle, unconfirmed). Sounds `S_BREATHFIRE`,
  `S_BREATHGAS`, `S_BREATHELEC` (`FUN_8009ed08`). One use is spent
  (`FUN_8007ed38(1, player, 9, 0x70)`).
- `HEAD_BREATHEF/A/E` on the body slot.

How the breath effect hurts (its cone and reach in `FUN_80094418`) isn't
traced.

### `0x80` phoenix

`PHOENIX_ICON` (hint PHOENIX "PHOENIX FAMILIAR", `S_PHOENIXVOX`).
Confirmed:

- Heroes of level 30 have a familiar (`FAMILIAR1`, `FAMILIAR2` from 80;
  `FUN_8007ddd0`) that spits on each throw. The phoenix hides the
  familiar in play and rides the hero itself (`PHOENIX` on the body).
- On a throw event (`+0x900` bit `0x10000000`) with the phoenix (or a
  familiar): aimed (`FUN_800857d8`) and lobbed with gravity 10
  (`r2-0x5B24`; none on boss levels, `FUN_80030a9c`), `FUN_80093150`
  fires `PHOENIX_FBALL` (`WEAPONS`, the type-4 entry of `0x8023FDD4`) at
  speed 35 (`r2-0x5B00`), damage 10, kind `0x11` (fire, strong;
  `0x8012265C`), from the familiar's mouth offset (class PDAT `+0x170` ×
  the model's scale). A familiar's spit (the player's entry of
  `0x8023FDD4`, `FAMILIAR_SPIT`) does 0.1 × (level − 25) + 2.5, kind 0.

### `0x100` grow

`GROWPOT` (hint GROWTHMSG "LIMITED GROWTH", `S_GROWTHVOX`; pickup
`S_GROW`). Confirmed: the hero model × 1.3 (not the ogre, which stays
1.6); every blow through the contact routine `FUN_8008625c` — melee, the
charge, the shields — × 2 (`r2-0x5B88`), and its push with it; when it
ends below level 99, `S_UNGROW` (`FUN_8009cd98`).

Here: the blows (`player.rs::strike_blow`, `shield_blow`) and the model
scale on the hero's root (with the ogre's 1.6 and level 99's 1.2; the
blob shadow scales with it, and the head's × 1.5 above level 98 isn't
done); `S_UNGROW`, `S_LEVITATEDOWN` and `S_UNSHRINK` from the bits
(`player_state.rs::powers_and_warning`). The charge's blow isn't ported.

### `0x200` shrink (enemies)

`SHRINKPOT` (hint SHRINKMSG "%s %s HAS SHRUNK ENEMIES", `S_SHRINKVOX`;
pickup `S_SHRINK`). It shrinks the monsters, not the hero. Confirmed:
each frame `FUN_80054140` sets the monster scale `r13-0x7320` to 1 and,
outside boss levels, multiplies it by 0.667 (`r2-0x6C20`) for each
playing hero with the bit. While it's below 1:

- monsters are drawn at it (`FUN_8004cfe0`), critters too
  (`FUN_800395bc`);
- monsters take × 2 (`FUN_8004e660`), non-boss critters × 2
  (`0x800382c0`);
- a monster's blow is × 0.5 (`r2-0x6EF0`) and of kind `0x40000000` — a
  small monster's, which levitation dodges (`FUN_8004dec0`); monster
  missiles × 0.5 (`FUN_8002fc08`); critters' blows × 0.5
  (`0x80036050`, `0x8003633c`, `0x800366e4`; the first leaves bosses
  out);
- the hero's range flags treat targets as low (`FUN_80080d3c`).

When it grows back in play, `S_UNSHRINK` (`FUN_8009cd68`).

Here: `player_state::EnemyScale`; monsters and critters drawn at it,
monsters taking × 2 (`damage.rs`, after the resistance routine) and
dealing × 0.5 with kind `0x40000000` (`hurt_hero`), their missiles × 0.5
(`projectiles.rs`), non-boss critters taking × 2 and dealing × 0.5
(their missiles not). Their collision sizes stay; the low-target range
flags aren't done.

### `0x400` Pojo

`POJOEGG` (hint POJOMSG "%s %s IS NOW POJO" — shown every time;
`S_POJOVOX`; pickup `S_POJO`). Confirmed:

- The hero's own model (`+0x7C`) is hidden and `POJO` rides the body slot
  with its own actions (above); when it ends, `S_UNPOJO` (`FUN_8009cdd8`).
- **Turbo**: with 40 turbo (`r2-0x5C20`) the turbo attack is ATTBREATHE
  at a cost of 40: a fire breath (`FIREBREATHE`, as above) from the
  Pojo's `POJOBODY1_HE_1` node, `S_POJOTURBO`. `FUN_8008872c` (a move
  choice needing turbo; not traced) gives nothing for the Pojo.
- **Throws**: `PHOENIX_FBALL` from (0, −0.5, −1.25) (`0x80119BE8`) instead
  of the class's missile (`FUN_80030094`).
- Its lines are the Pojo's: `S_POJOPAIN`, `S_POJOEATSFX`, `S_POJOPOISON`,
  `S_POJO1/2` in the announcer's sentences ([frontend.md](frontend.md),
  "The voice queues").
- Eating a CHICKEN is 100 poison damage (kind `0x800`, [items.md](
  items.md)); a FALLFRNT knockback is × 80 instead of 32
  ([combat.md](combat.md)); the magic button is turned into button `0x200`
  (`FUN_80088170`; not traced).

### `0x1000`, `0x2000`, `0x4000`, `0x8000` Skorne's items

Hints HORNSMSG "THE HORNS OF SKORNE" (`S_HORNSVOX`), MASKMSG "THE MASK OF
SKORNE" (`S_MASKVOX`), GAUNTLETMSG "SKORNE'S GAUNTLET" (`S_GAUNTLETVOX`)
for both gauntlets. A special pickup with any of `0xF000` is refused while
the hero holds one (`FUN_8005de3c`). Confirmed:

- **Horns, mask**: `BOSSHORNS` / `BOSSMASK` on the head; attacks are
  ATTBREATHE and breathe `BOSS_BREATHE` (`0x38`): 50 (`r2-0x5B5C`),
  radius 20, kind `0x21`; `S_HORNS` / `S_MASK`.
- **Gauntlets** (the special missile records of projectiles.md):
  `BOSSGAUNTR` (`0x4000`) / `BOSSGAUNTL` (`0x8000`) on the wrists; attacks
  are ATTFIRELR / ATTFIREL and the throw uses the record `0x80119B88`
  (kind 4 acid, radius 2) with `BOSSG_ACID` for the right, `0x80119B58`
  (kind 2 lightning, radius 2) with `BOSSG_ELEC` for the left (both
  `+0x04`/`+0x08` = 50/40), the missile flagged `0x10000` (meaning not
  traced). `S_GAUNTLET1/2` are in the catalog; their calls weren't found.

### Other special bits

- `0x10000`: set while a speed power runs (items.md); the action machine
  sets `+0xA8` (1.0 otherwise) to 0.75 (`r2-0x4DA4`) — also for the
  rapid-fire weapon in two action categories; presumably the actions'
  time scale (unconfirmed).
- `0x80000`: the turbo refill (done).
- `0x100000` Mikey (hint MIKEY "Mikey Powerup"): sets `+0xA1C` to 1; from
  3 on, the monsters' targeting measures to `+0xA04` instead of the hero
  (a decoy; the state's steps aren't traced).
- `0x200000` Hand Of Death, `0x400000` Health Vampire: arm `+0xA1E` /
  `+0xA20` (with the sparkle). A monster's blow on the hero
  (`FUN_8004dec0`) then goes to the monster instead (`FUN_8004e660(damage,
  monster, −1, kind, …)`; the hero takes 0); with Health Vampire the kind
  is `0x200` and the hero heals by the blow (`FUN_80078474`). Critters'
  blows and missiles aren't affected.

## Pickup hints

Hint by bit (`FUN_8005de3c`; all priority 50, shown once but the Pojo's):

| armour | hint | special | hint |
| --- | --- | --- | --- |
| `0x100000` | `0x36` | `0x4` | `0x24` |
| `0x10000` | `0x23` | `0x2` | `0x27` |
| `0x80000` | `0x31` | `0x8` | `0x33` |
| `0x20000` | `0x34` | `0x1` | `0x35` |
| `0x200000` | `0x5B` | `0x10`/`0x20`/`0x40` | `0x51`/`0x52`/`0x53` |
| `0x400000` | `0x5C` | `0x80` | `0x54` |
| `0x2000` | `0x84` | `0x100`/`0x200`/`0x400` | `0x58`/`0x59`/`0x5D` |
| | | `0x2000`/`0x1000`/`0x8000`/`0x4000` | `0x62`/`0x63`/`0x64`/`0x64` |
| | | `0x80000` | `0x71` |
| | | `0x100000`/`0x200000`/`0x400000` | `0x94`/`0x95`/`0x96` |

## Open

- The chrome's texture matrix (`FUN_800c60a4`) and whether it's an
  environment map.
- The breath effects' damage shape in `FUN_80094418`.
- The drain effects `0x5F`/`0x60` and the missile flag `0x10000`.
- Mikey's state machine; who sets Skorne's items; where `S_FIRESHIELD`,
  `S_LIGHTNGSHIELD`, `S_SHIELD1`–`4`, `S_GAUNTLET1/2` play.
- Whether health is capped after the gold heal.
