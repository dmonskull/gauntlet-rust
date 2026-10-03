# Melee combat

Implemented in [`combat.rs`](../crates/gdl-game/src/combat.rs) (buttons,
intents, target search, blows, the `Targetable`/`Hit` interface),
[`actions.rs`](../crates/gdl-game/src/actions.rs) (action table, categories,
factors, the action state machine) and
[`player.rs`](../crates/gdl-game/src/player.rs) (the 30 Hz tick that runs it
all). Both logic modules are pure and unit-tested. Addresses are in
`main.dol`; `r2`/`r13` as in [INDEX.md](INDEX.md). Movement and the
locomotion half of the chaining are in [player-movement.md](player-movement.md).

## Controls

### Logical buttons

Each pad is turned into three words per player (a `0x3C`-byte record per
player at `0x802407a4`): held (`+0x00`, from `0x8023ff78`), pressed this
frame (`+0x04`, from `0x8023ffd8`; `FUN_800330b4` computes it as held &
!previous) and pressed-with-autorepeat (`+0x08`), then the left stick as
angle/magnitude (`+0x18`/`+0x1C`) and the C-stick (`+0x20`/`+0x24`, read
only by scheme 2). `FUN_800330b4` sets logical bit `0x100 << i` when the raw
pad word matches the scheme's mask for button `i` (table `0x8011a730`, 10
buttons × 4 schemes × two words). Names from the debug table at
`0x8011a8a4` (`"Control %d has %s pressed"` in `FUN_80032b94`):

| bit | name | default (GameCube) | keyboard here |
| --- | --- | --- | --- |
| `0x100` | `S_MAGIC` | X | U |
| `0x200` | `S_ATK_QUICK` | A | J |
| `0x400` | `S_ATK_SLOW` | Y | L |
| `0x800` | `S_TURBO` | B | H |
| `0x1000` | `S_DEFEND` | B (same button as turbo) | H |
| `0x2000` | `S_CHARGE` | L | P |
| `0x4000` | `S_STRAFE` | R | O |
| `0x8000` | `S_MAGIC_SHIELD` | — | — |
| `0x10000` | `S_THROW_MAGIC` | — | — |
| `0x20000` | `S_COMBO_MOVE` | Z | G |

The gamepad is mapped by position (A south, B west, X east, Y north, L
left trigger/bumper, R right trigger, Z right bumper).

How a GameCube button reaches a raw bit: the port emulates PS2 `libpad`.
`FUN_800aea9c` builds a DualShock 2 buffer from `PADStatus` (button query
`FUN_800cb5f0`; digital bits from the table at `0x80126f60`, pressure bytes
from the index table at `0x80126fa0`); the pad state machine in
`FUN_80031a50` ends in pressure mode (5) because the emulated `scePad*`
stubs return constants (`FUN_800aea8c` → 7, `FUN_800aea7c` → 1, …).
`FUN_80031e84` copies that buffer into 24 `{flags, value}` pairs and
`FUN_80033c3c` packs them into the raw word: D-pad left/right/up/down
`0x10000000`/`0x20000000`/`0x40000000`/`0x80000000`, Y `0x4000000`, X
`0x1000000`, B `0x8000000`, A `0x2000000`, L `0x100000`, R `0x400000`, Z
`0x800000`, Start `0x40000`, the sticks' directions in the low byte.

The four schemes (names at `0x8011e810`, the options menu cycles the first
three in `FUN_80070c24`; default 0 from `FUN_80032974`):

| scheme | magic | attack | power | turbo/defend | charge | strafe | combo |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 Default | X | A | Y | B | L | R | Z |
| 1 Arcade | X | A | B | Y | L | R | Z |
| 2 Robotron | B | A | L | R | Y or X | — (C-stick attacks) | Z |
| 3 One Handed | B | A | L | R | — | — | Z |

Two more per-pad options, both on by default (`FUN_80032974`), toggled in
the same menu: `+0x30` "attack aim" (attacking in place turns toward the
target) and `+0x34` "walk-into attack". Their on-screen names weren't
looked up.

### Intents (`FUN_80088170`)

Called at the top of the player update `FUN_80080d3c` with the camera
heading. In order:

1. THROW_MAGIC → `0x19`, MAGIC_SHIELD → `0x1A`, MAGIC → `0x18` (held).
2. COMBO_MOVE held, turbo ≥ 50 (`r2-0x5bf8`), a partner in reach
   (`FUN_8008872c`) → `0x16`.
3. TURBO held + ATK_QUICK pressed, turbo ≥ 0 → `0x15` (turbo attack).
4. DEFEND pressed → `2`.
5. Scheme without C-stick: if STRAFE is held and the stick is pushed, the
   side (stick heading − facing: within ±45° front, beyond ±135° back,
   positive right) gives strafe walks `9`–`0xC` (F, B, L, R), or strafe
   attacks `0x11`–`0x14` if an attack button is held. Otherwise CHARGE
   pressed with turbo ≥ 5 → `7`; ATK_QUICK held → `0xF`; ATK_SLOW held →
   `0x10`. (With the C-stick in scheme 2, pushing it attacks toward it.)
6. Stick magnitude > 0.75 → `0xD` run, > 0 → `8` walk, else `1` idle.

It also keeps two attack-button words: `+0x8F4` latches the attack bits
while either attack button is held (cleared when both are up) and `+0x8F8`
accumulates presses since the last attack began (`held ^ latch`), cleared
when most actions start (below).

Note that attacking is on the *held* word: holding A keeps attacking.

## Targets

### Search (`FUN_800864b0`)

From the hero's collision centre (`+0x64`, the feet + 2.5; `FUN_80080d3c`
passes it, moved by this tick's step), in a heading, up to 30
units (`r2-0x5b28`; 200 in some special mode). Nothing hit last tick is
tried first (the monster bumped into while moving, within a 60° cone, and
the last generator, within 45°); otherwise the nearest of:

- **Monsters** (`FUN_80044428`, records at `0x802515e8`, `0x394` bytes;
  state `+0xB4` 1 or 6; position `+0x54`, radius `+0x238`): within 10
  units vertically (`r2-0x6f08`), distance = |v| − radius.
- **Objects** (`FUN_80037e9c`/`FUN_80038008`, records at `0x80240bd4`,
  `0xAE0` bytes, state `+0x08` ≥ 2, hit points `+0x4B0` > 0; centre `+0x5C`,
  radius type `+0x7C`; multi-part objects test each part): |v| ≤ 30,
  distance = |v| − radius.
- **Generators and breakables** (`FUN_8005b260`, `0xF0`-byte records at
  `r13-0x71a8`: placement class 3 GENERATOR, 2 CONTAINER barrels
  `0x2B`–`0x2D`, 10 OBSTACLE except `0x29`, 5 TRIGGER `0x1F`): within 2 ×
  height (type `+0x10`) vertically, distance = |v| × scale − min(radius,
  5), scale 1 for generators, 0.9 for exploding/poison barrels and 1.2 for
  the rest.

For all, the cone narrows linearly with distance: accept when
`horizontal(n) × (cone + distance × (1 − cone) / 30) ≤ n·dir` for the unit
vector `n` to the target — `cone` = 0.5 (60° either side next to the hero,
`r2-0x5ca0`), straight ahead at 30 units; 0.707 when `0x80274874` ≥ 1
(not traced). With nothing found the direction returned is the hero's
facing.

### Range flags (`+0x90C`)

Set every tick in `FUN_80080d3c` from the search toward where the hero is
heading (stick heading, or facing):

| bit | meaning |
| --- | --- |
| 1 | close: distance < 1 + radius + e |
| 4 | medium: < 2 + radius + e |
| 8 | far (also: nothing found) |
| 2 | low target: a monster with height `+0x23C` ≤ 2, or a generator/breakable with height ≤ 3.5, within 2 + radius |
| `0x10` | target is a monster or object |
| `0x20` | target is a generator or breakable |

`e` is 1 when the intent is an attack (`r2-0x5c70`), 0 otherwise. The
hero's radius is `PDAT +0x4C` (1.5 for every class; `FUN_80079ed8` copies
it to `+0x850`, and half of `PDAT +0x48`, 2.5, to `+0x854` as height).

`+0x904` is the angle from the facing to the target (or, while strafing or
defending, with attack aim off or with the C-stick, to where the controls
point).

**Walk-into attack**: with the option on, intent walk/run, no pending
press, not already attacking (action outside `0x27`–`0x72`), and a monster,
object (except runtime category 4) or generator within 1 + radius
(`r2-0x5be0`), the intent becomes a quick attack and `e` = 0.

### Requested action (switch at the end of `FUN_80080d3c`, into `+0x20C`)

| intent | requested |
| --- | --- |
| idle / walk / run | READY / WALK1 / RUN1 (or hit reactions, PUSHED) |
| 2 | DEFEND1 |
| 9–0xC | STRAFE_WLKF1 / B1 / L1 / R1 |
| 0x11–0x14 | STRAFE_ATKF1 / B1 / L1 / R1 |
| `0xF` quick | medium, not low, stick pushed → ATTSTEP1 (and `+0x904` = stick heading − facing); close or walked into → ATTLOWK if low else ATTQUICK1; otherwise THROW1S (a thrown weapon) |
| `0x10` power | combo count ≠ 0, stick pushed, close or medium → ATTSTART; medium, not low, stick → ATTSTEP1; close or walked into → ATTLOW1 if low else ATTSTART; otherwise ATTPWRATHROW |
| `0x15` turbo | turbo ≥ 100 → ATTPWRC, ≥ 40 → ATTPWRB (costing it); else unchanged |
| `0x16` / `7` | COMBOACT1 / SHOVE |
| magic | MAGICS / THROWPOTIONS, or with no potions a "no magic" cue and locomotion |

Weapon power-ups (`+0x124`/`+0x11C` flags) replace attacks with
fire/breath/chop/shot actions; not modelled.

## The action state machine (`FUN_800ab898`)

Its class checks read the record's `+0x8`, which the class switch
(`FUN_80079a00`) sets to the class less 8 for the alternate characters:
the minotaur fights by the warrior's rules, the falconess by the
valkyrie's, … the hyena by the jester's (`Player::base_class`). The
tables indexed by `+0x8` ([projectiles.md](projectiles.md): the throw
sounds, the missile records) go the same way; the weapon table and the
hand bones are by the class itself (`+0xC`).

Picks the next action from the playing one (`+0x208`) and the requested one
(`+0x20C`), with a transition mode applied by `FUN_80011134` →
`FUN_8000eb70`: **2** now if the clip differs or has ended, **0** once
ended if different (the default), **1** once ended, **3** always. It logs
`"ACTION %s NEXT %s D %s INT %d RP %d …"` in a debug mode. Before the
switch: a request above `0x72` while an action of category 1–10 plays, or a
request of category > 10, is treated as coming from READY (mode 2); the
combo count `+0x908` resets unless the playing action is in categories 2–6
or 8. Action names are the table at `0x80126430`; categories are
`FUN_800ad42c` ([player-movement.md](player-movement.md)).

| playing | next (mode 0 unless noted) |
| --- | --- |
| ATTQUICK1/2/3 | power pressed and combo ≠ 0 → by combo count 1 ATTPWRACLOSE, 2 ATTPWRAMED, ≥3 ATT360; no press pending and nothing held, or far → recovery (Q2 → ATTQUICK2R, else ATTQUICK3R); medium → ATTSTEP3 from Q2 else ATTSTEP2; else Q2 → Q3, Q1/Q3 → Q2 |
| ATTQUICK2R/3R | finisher as above; a press pending, frame ≤ 2 (`r2-0x4dd8`) and close → back into the combo (Q2R → Q3, Q3R → Q2) **now**; defend request → now |
| ATTSTEP1/2/3 | like the quick attacks, with recoveries ATTSTEP2R/3R and steps continuing into Q3 (from STEP2) / Q2 |
| ATTSTEP2R/3R | finisher, or defend now |
| ATTSTART | ATTSLOW1 (unless a throw/fire/special is requested: now) |
| ATTSLOW1 → ATTSLOW1R | then the request (defend now) |
| ATTPWRACLOSE → ATTPWRACLOSER, ATTPWRAMED → ATTPWRAMEDR, ATTPWRALOW → ATTPWRALOWR, ATTPWRATHROW → ATTPWRATHROWR | not interruptible except by being grabbed |
| ATT360 | ATTPWRAMED on a power press with combo, else ATT360R |
| directional swings (below) | their own recovery (`0x2C`→`0x2E`, `0x2D`→`0x2F`, `0x30`→`0x32`, `0x31`→`0x33`, `0x34`→`0x36`, `0x35`→`0x37`, `0x38`→`0x3A`, `0x39`→`0x3B`) |
| ATTLOW1/2 | alternate while ATTLOW1 is requested, else ATTLOWR |
| ATTLOWK | ATTPWRALOW on power + combo, else ATTLOWKR (mode 1) |
| THROW1S / THROW2S | THROW1 / THROW2 — at the clip's end, or now once past frame 2 |
| THROW1 / THROW2 | THROW1R / THROW2R |
| STRAFE_ATK*1/2, STRAFE_WLK*1/2 | alternate first/second clips like WALK1/WALK2 |
| DEFEND1 → DEFEND2 → DEFENDR | mode 1 |

Then the next action is rewritten:

- A quick attack or lunge toward a target off to the side or behind
  (`+0x904`) turns into a directional swing: beyond 3π/4 (`r2-0x4dc8`) →
  ATTQ2180 (after Q1/Q3/steps 1 and 3) or ATTQ3180 (after Q2/step 2);
  below −3π/4 → ATTQ2180L / ATTQ3180L; beyond π/3 (`r2-0x4db8`) →
  ATTQ3RIGHT / ATTQ2RIGHT; below −π/3 → ATTQ3LEFT / ATTQ2LEFT. (The
  names' 2 and 3 are the game's.)
- ATTSTEP1 from WALK2/RUN2 → ATTWALK2, from ATTQUICK1/3 → ATTQ3TOSTEP1.
- THROW1S from WALK1/RUN1 → THROW2S.
- ATTPWRACLOSE against a low target → ATTPWRALOW; if the class has no
  ATTPWRALOW(R) clip the close finisher's clip plays instead (the archer
  has none). A missing clip plays the first clip.

Returning to READY blends over 0.0667 s (`r2-0x4dcc`) except after
`0x56`–`0x93`, HITREACT and `0x81`/`0x82`, and the archer's ATTQUICK2R.

### On a switch

Keyed on the action that **ended**, bits go into the event word `+0x900`:

| ended | bit | effect |
| --- | --- | --- |
| ATTSLOW1, ATTSTEP1–3, ATTQ3TOSTEP1, ATTWALK2 | 4 | strong blow |
| ATTPWRACLOSE, ATTPWRAMED (not the sorceress), ATTPWRALOW | `0x10` | finisher blow |
| ATTQUICK1–3, directional swings, ATT360, ATTLOW1/2 | 2 | blow |
| ATTLOWK | 8 | kick |
| strafe attacks, THROW1/2 | `0x100` | projectile |
| ATTPWRATHROW | `0x1000` | power projectile |

So a swing lands when its clip ends and hands over to the next action.
Keyed on the action that **starts**: the attacks (ATTSTART, ATTSLOW1, the
quick attacks and swings, ATT360, steps, ATTLOW1/2, ATTLOWK) set bit 1 and
count the combo — `+0x908` += 1 if a press is pending, else 0 — then
clear the pending presses; strafe attacks and throws set bit 1 and clear
them; recoveries, strafe walks and defends keep them; everything else
clears them.

### Movement and turn factors

Set by the same function for the action playing when it runs (`+0xA48`
move, `+0xA4C` turn; the movement one scales the next tick's step, the
turn one this tick's turn):

| actions | move | turn |
| --- | --- | --- |
| ATTSTART, ATTSLOW1, ATTSLOW1R | 0 | 1 |
| ATTPWRACLOSE(R), ATTPWRAMEDR | 0.5 (wizard 0.25; knight, sorceress 0) | 1 (0) |
| ATTPWRAMED | 0.5 (wizard, archer 0.25; sorceress, jester 0) | 1 (0) |
| ATTQUICK1–3 and recoveries | 0.25 | 0 |
| directional swings | 1 | 1 |
| ATT360(R) | 0.5 | 1 |
| steps, ATTQ3TOSTEP1, ATTWALK2 (`0x3E`–`0x46`) | 1 | 0.25 |
| strafe attacks | 0.667 | 1 |
| low attacks and kick | 1 | 1 |
| ATTPWRALOW(R) | 0.25 | 1 |
| throws (`0x5B`–`0x62`) | 0 | 0.5 |
| ATTPWRATHROW(R) | 0.25 | 1 |

`+0xA50` scales the stick: 0 for the actions from MAGICS (`0x73`) on
outside the attack range, except VICTORY, WEBREACT and `0x8F` — so a
defending hero stands still. With the stick released, the actions
`0x3E`–`0x4E` (steps, strafe attacks) move at stick 0.5 along the facing.

### Facing while attacking

After the state machine, if the action's category is 1–10 but not 7, no
defend/strafe button is held, attack aim is on and neither stick is
pushed, the desired facing becomes the direction to the target found this
tick — the hero turns toward it at 5π rad/s × turn factor (not at all
during the quick attacks, whose turn factor is 0).

## Blows (`FUN_80080d3c`, after the state machine)

When `+0x900 & 0xFE` is set:

1. Damage = the hero's strength (`+0x104`: 5 + 0.015 × strength stat,
   clamped 5–20, `FUN_8007c4f0`). Kind = the hero's weapon power-up bits
   (`+0x11C`; ported — [items.md](items.md), "Timed powerups"). Finisher (`0xF0`): kind `|= 0x20`, damage × 3
   (`r2-0x5c88`), and any turbo cost is paid. Else strong (4): kind `|=
   0x10`, damage × 2. Else kick (8) on a monster ≤ 2 tall: kind `|= 0x20`.
2. (With no stick and no generator lock, `FUN_80086e44` nudges the hero
   toward position + facing × 2 × radius — not ported.)
3. Search again, from the hero's position in the **facing** direction.
4. Hit if the distance < 2 + radius (`r2-0x5b88`). Monsters and objects
   also need a clear line from the hero to them (`FUN_8000d308`, walls,
   radius 0.1); then `FUN_8008625c(damage, cooldown 0, …, kind, hit point,
   1)`. Generators and breakables go to `FUN_8008615c` (no push, no
   strength power-up), other players in versus mode to `FUN_80086028`.
5. The hit point is position + facing × (2 + radius).

`FUN_8008625c(damage, cooldown, player, target, kind, point, effects)`:

- A positive cooldown first checks the target's per-attacker slot (monster
  `+0x2B8 + 4 × player`, the time until which that player can't hit it;
  objects keep four `{attacker, until}` pairs at `+0x4E0`/`+0x4E8`,
  `FUN_80037c5c`/`FUN_80037de8`) and afterwards sets it to now + cooldown.
  Melee blows pass 0: no cooldown. The charge (SHOVE) passes 1 s with
  damage 3 and kind `0x20`.
- Damage 0 means strength; the strength power-up (`+0x124 & 0x100`)
  doubles it.
- Push = (facing X, min(0.05 × damage, 2), facing Z), added by the monster
  to its `+0x2A8` accumulator (`FUN_8004e660`); objects get it too
  (`FUN_800382c0`).
- Monsters take it through `FUN_8004e660` (only in state 1 or 6 with hit
  points `+0x200` > 0; resistances `FUN_8002f58c`, difficulty, death,
  score); objects through `FUN_800382c0`.

## In this rewrite

- `Targetable { kind, radius, height }` on any entity; its
  `GlobalTransform` translation is the reference point (feet, like the
  hero's). Kinds: `Monster`, `Generator`, `Breakable` (containers and
  obstacles), `Object` (the second object table). Remove the component
  when the thing can't be hit any more — the game's state/hit-point checks
  are the owner's.
- `Hit { target, attacker, damage, kind, push, at, target_kind }` message:
  damage before the target's resistances; `kind` bits `0x10` strong, `0x20`
  heavy; `push` as above (zero for generators and breakables). Owners
  apply it.
- `Targetable::can_be_hit_by` / `start_cooldown` implement the per-attacker
  cooldown for blows that use one (none of the melee ones do).
- `GDL_DUMMY=<distance>[,<kind>[,<radius>[,<height>]]]` spawns a practice
  target in front of the hero that logs and flashes on hits;
  `GDL_BUTTONS=attack@30-32,power@60-62` scripts buttons by tick.

### Stand-ins and differences

- The turbo meter fills and drains and the turbo attacks swing (below);
  co-op combos aren't done. The charge's blow is (`player.rs`, from
  `FUN_80080d3c`: while SHOVE plays and no reaction was picked this tick,
  a monster or non-boss critter the search finds within 1 of the hero
  takes 3, heavy, once a second — in place of walking into it; grown,
  × 2), with the cooldown kept per target on the hero. Walking into a
  boss critter (type class 4) doesn't attack it except on the levels
  whose boss is type `0x25` or `0x29`. Magic without
  potions just walks, as the game does for a hero with none (without the
  cue).
- Projectiles (throws, strafe attacks, power throw) are released as the
  game does ([projectiles.md](projectiles.md)); the weapon power-up shots
  (`0x800`, `0x6000`) aren't.
- A looping clip counts as ended each time it comes round (the game's end
  flag for loops isn't traced; DEFEND2 needs it to finish).
- Searches start from the hero's position before this tick's move, not
  after it.
- Last tick's bumped monster / locked generator aren't preferred, and the
  0.9 scale for exploding/poison barrels isn't distinguished (breakables
  all use 1.2). Critters with hit spheres follow the game's per-critter
  pick ([critters.md](critters.md), "Found and hit").
- Knockback on the hero, hit reactions, pushing, the idle timers, the
  start-frame offset for interrupted throws, and animation speed (DEFEND2
  plays at 0.2 × armour) aren't done. The attack-sound indices in `PDAT`
  (`+0x0C`–`+0x1E`) aren't played.
- The four schemes' buttons follow the game's table (`controls.rs`); the
  Robotron style's C-stick is the right stick: pushed alone, an attack
  (the power attack with its button held) toward it, aimed where it
  points; with the left stick pushed too, a strafe attack to its side of
  the left stick's heading (the hero's facing then isn't traced: it's
  kept).

## What a blow does to its target

`damage.rs` applies the hero's `Hit` messages.

**Monsters** (`FUN_8004e660`): hit points (`+0x200`) go down by the
damage. The monster's own damage (`+0xBC`) is then recomputed from its
type's damage × the level's scale, × 0.667 below two thirds of its full
hit points and × 0.333 below one third (`r2-0x6cf0`, `r2-0x6cf8`). At 0
or less (`r2-0x6e88`) it dies: state `+0xB4` = 8 and its slot is released
at once (`FUN_8004f240`), so its generator can make another; the body plays
DEATH. Before that the blow goes through the resistance routine
(`FUN_8002f58c`) with the monster's armour `+0xC0` (0, 1 for Death) and
resistances `+0xC8` (0 for every type): `damage.rs::resist`, so an
elemental blow does × 1.5 (× 1.25 on a boss level); a hero's blow still
under 1 (`r2-0x6f10`) then does 1 (`r2-0x6e30`). Not applied yet: the
scale by the player's level against the level's (`+0x9C` of the level
record), the shrink power's × 2 (`r13-0x7320` < 1), the push accumulator
(`+0x2A0`) and score.

**Generators**: hit points are the item type's × strength × the level's
scale; each item-type's worth lost drops a strength level (its monsters
come out a tier lower); at 0 it's removed with its model. The per-level
model swap (`GEN_<code><n>`) isn't shown yet.

## Blows that land on the hero

`FUN_80078560` (hurt a player) runs a blow through `FUN_8002f58c` with the
player's derived armour (`+0x108`, 0–5 = 0.001 × armour stat × 5): unless
the blow's kind has `0x200` or `0x800` (armour-piercing), armour is taken
off the damage, and a blow no stronger than the armour does nothing.
The same routine then applies the armour powers and elemental
resistances by kind (`& 0xF`): `damage.rs::resist`, through
`Player::take_blow` ([powers.md](powers.md), "The resistance routine").
Blows above 1 are
also scaled by the level record's `+0xA4`, which is 1.0 in every retail
level. Defending (`+0x964 & 0x600`) cuts or blocks blows from the front
(`r2-0x5ea8`, angle limits `r2-0x5e18`/`r2-0x5e10`) — not ported yet.

### The hero's reaction

A monster's blow (`FUN_8004dec0`) carries its current damage (× 1.5 on the
strong third attack, `r2-0x6ee8`) and kind flags: `0x10` when a big
monster's (`+0x23C` > 2) strong attack lands, `0x20` for monster type
`0x1D`, `0x40000000` for small monsters; knocking kinds (`& 0x130`) push
along monster → hero. `FUN_80078560` accumulates damage `+0x8D0`, flags
`+0x8D4` and push `+0x8DC`; the next player update (`FUN_80085ca8`) picks a
reaction:

| flags (damage > 1) | class | knockback | action |
| --- | --- | --- | --- |
| `0x10000` | 30 | push × 100 (`r2-0x5b60`) | FLYUP (`0x87`) |
| `0x40` | 20 | push × 100 | FALLFRNT |
| `0x120` | 20 | push × 32 (80 with power `0x400`) | FALLFRNT (`0x85`) |
| `0x10` | 10 | push × 16 (`r2-0x5a68`) | HITREACT (`0x82`) |
| `0x2000` / `0x80` | 3 / 2 | — | HITREACT (`0x81`) / STUN1 (`0x7F`) |
| other | 1 | — | HITREACT (`0x1B`) while standing or moving |

Knockback classes gain 1 when the push comes from more than 90° off the
facing (FALLFRNT → FALLDOWN) and turn the hero to face the blow. A blow of
more than 1 also plays its hit effect (`FUN_8009399c`, unless kind
`0x1000000`) and flashes the hero for two updates
([rendering.md](rendering.md), "Texture overrides").

**The stuns.** A blow that does damage (> 0 after the resistance
routine) with kind `0x800` (poison) sets the hero's stun time `+0x898` to
now + 1 s (`r2-0x5F20`), with `0x1000` (Death's drain) now + 1/15 s
(`r2-0x5DF0`) — the later test, so the drain's wins; the latest blow's
counts. The player update then adjusts the class by the action playing
(`FUN_80080d3c`, after `FUN_80085ca8`; classes below 300): FALLDOWN
(`0x83`) → 20 and FALLFRNT (`0x85`) → 11 (the knock-down slides on while
it falls; the chaining gets up), HITREACT `0x82` with a class below 10 →
10; else, **while `+0x898` is ahead and the intent is to stand (1)**, 100
— even over a new blow's class; else with no new blow, HITREACT `0x81`
playing → 3, STUN1 playing → 2. Class 100 asks for STUN2 (`0x7A`, intent
`0x20`), 2 and 3 for STUN1 (`0x7F`) and HITREACT (`0x81`, intent `0x1F`)
— or, with one of those already playing, stand (intent 1, class 0:
READY). All three zero the move (the stick is ignored: the hero stands).
Which blows: kind `0x80` is the damage tiles' (their type value `| 0x80`,
[mechanics.md](mechanics.md)); poison's `0x800` (the poisoned food's blow
is one); the drain Death's. The chooser (`FUN_800ab898`): STUN2 loops (its own flag
and the chooser's), a READY request ends it with its clip, any other
cuts it at once — unless in the first 10 frames (`r2-0x4DD0`) of its
first pass (the animation status `+0xB6`, the slot's end/restart flags,
still 0), when it waits for the clip's end; STUN1 and the HITREACTs
`0x81`/`0x82` go on to what's asked at their end, READY if asked for
again, and a knock-down (above `0x82`) takes over at once (mode 3);
WEBREACT (`0x80`) loops until something other than READY is asked for.

Ported in `player.rs` (`hit_reaction`, `blow_stun`, `stun_class`,
`stun_action`) and `actions.rs` (the chooser's cases); the monster size is
the radius stand-in from [monsters.md](monsters.md). Stand-in: a blow's
stun starts on the tick after it lands (with its reaction), not as it
lands.

## Turbo

The turbo meter is player `+0x828`, 0–100. Each update (`FUN_80080d3c`),
while the playing action's category is under 11: during the charge
(SHOVE, action 8) it drains at 20/s (`r2-0x5ba8`), otherwise it refills at
2/s (`r2-0x5b88`; 15/s in a debug mode) up to 100 (`r2-0x5b68`), with
announcer cues at 40 and when full. Turbo held with attack (intent `0x15`)
asks for ATTPWRB (`0x56`) at 40 or more (`+0x910` cost = 40, `r2-0x5b58`)
or ATTPWRC (`0x57`) with a full meter (cost 100); with the `0x400` power
it's ATTBREATHE (`0x6E`; the breath itself is ported, [powers.md](
powers.md) "breaths", the Pojo's turbo breath isn't). When the turbo blow lands the cost
comes off the meter and the blow is × 3 (`r2-0x5c88`) with kind `0x20`
(ported as a finisher strike — a stand-in: the game's turbo blows are the
class's `DAMG` attack records, `PDAT +0x16` and `+0x18/+0x1A`, through
`FUN_80088b88`; [chunk-files.md](chunk-files.md) "The heroes' attack
records"). Turbo-table experience also adds 0.025 ×
experience to the meter (`FUN_80076144` with its third argument 1) — not
ported, as the calls that pass it aren't traced.
