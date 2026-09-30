# Magic: potions and their effects

Implemented in [`effects.rs`](../crates/gdl-game/src/effects.rs), with the
controls in `player.rs` / `actions.rs` and the thrown potion on the
missile code ([projectiles.md](projectiles.md)).

## Using a potion

The magic button (X) with potions held (`PlayerState::potions`, kinds by
colour):

- **tap**: MAGICS → MAGICR, a **blast** around the hero (mode 0);
- **tap twice** (let go and press again during MAGICS): the **shield**
  (mode 1);
- **hold**: THROWPOTIONS winds up (fields held, `+0x958`) and throws the
  potion (modes 2/3), which bursts where it lands;
- **no potion**: the "collect magic first" hint, and the hero moves as the
  stick says.

After a use, magic does nothing until it's let go (the control flags'
`0x80`). The game fires the use from the hero's hit flags `0x20000` (magic)
/ `0x40000` (throw potion) on `+0x900` in `FUN_80080d3c`, calling
`FUN_80076618(scale, player, pos, kind, mode)`.

## The numbers (`FUN_80076618`)

- The potion's kind is its colour (`kind & 0xF`): 1 red (fire), 2 blue
  (lightning), 3 yellow (light), 4 green (acid); 0 cycles 1–4.
- Magic power: the hero's `+0x10C` = 8 + 0.024 × the magic stat (8–32).
- Blast (mode 0, `FUN_8009262c`): damage 40 out to the power; shield
  (mode 1, `FUN_80091fcc`): 25 out to a quarter of it; thrown (modes 2/3,
  `FUN_80092390`): 40 out to three quarters.
- The player's own colour (yellow player 1, blue 2, red 3, green 4):
  damage + 10%, reach × 1.1. Heroes above level 24 add kind `0x800000`.
  Every potion's kind gets `0x200` (it doesn't hurt players).
- Sounds: `S_POTION1–4` for the blast, `S_SHIELD1–4` for the shield
  (`FUN_8009e860`).

## The blast in time (`FUN_80094418`)

A growing blast lives as long as its effect model's clip (frames × rate /
900 s). Its front runs from a third of the radius (full damage × 1.005) out
to all of it (nothing) over the first two thirds of its life:
at `f` = time left / life, reach = radius × (1.33 − f) and damage share
1.5 × (f − 0.33), nothing once `f` ≤ 0.33. Each monster, critter,
generator and breakable is hit once when the front reaches it.

## Drawing

The effect models are the game's, from `WEAPONS`: `MP_FIRE`, `MP_ELEC`,
`MP_LIGHT`, `MP_ACID` (blast), `MS_*` (shield), `POT_*_TW` (thrown), scaled
to the blast's radius (the models are made for 32 units). Without a model
a translucent sphere in the potion's light colour stands in.

Testing: `GDL_POTIONS=<n>[,<kind>]` hands the hero potions at the level
start; `GDL_BUTTONS="magic@60-62"` taps magic; frame it with
`GDL_LOOK_AT` and take a burst with `GDL_SHOTS` / `GDL_SHOT_EVERY` (on
levelA1 `GDL_LOOK_AT="0,-1,-10,24" GDL_SHOT_AT=300 GDL_SHOTS=12
GDL_SHOT_EVERY=15`): the light potion's starburst spreads round the hero
and the grunts near it die.

Stand-ins: the effects' own lights aren't cast; the blast front is
checked against targets' centres; the shield's contact damage follows the
decoded numbers but its timing (every tick it touches) is a guess.

## The effect table, flipbooks and depth bias

Effects are the table at `0x801218e0` (0x50 entries of 0x28 bytes: name,
`+0x20` depth bias, `+0x24` transparency; runtime entries up to `0xD9`,
e.g. `0x50` SUICIDEEXP). Transparency is 96 for entries 1–`0x15` (SPARKS,
the hit and die effects: drawn at 159/255) and 0 for every other.

**Flipbooks**: a bank's `ANIM.PS2` texture modifiers (`0x58` bytes, as a
level's) whose `+0x00` is an atree index belong to that effect (−1: free
running on the frame counter, `FUN_80010a4c`). Each spans its clip: the
fireball's FBALL_EXP is 15 frames × 2 ticks for EXPLOSION's 30, the gas
clouds' POISON_GAS 5 × 4, 15 × 4 and 10 × 2, EXPRING's SUICIDE_RING 10 × 2;
HITCOL and HITDIE own EXPLDSML_ and EXPLBIG_. Here each copy of an effect
runs its own from its start, on copies of the materials drawing the
texture (`EffectFrames`).

**Depth bias** (`FUN_800baa74` → object `+0x68`, used by the draw,
`FUN_800c5894`): the object is depth-tested `bias` × −2048 (render context
`+0x78`, `r2-0x4888`) of the 24-bit z-buffer nearer: −128 for most effects
(1/64 of the range), −512 for SUICIDEEXP and the sparkles round a hero, 0
for the breaths and bags. The game's clip planes are 1 and 65536
(`FUN_800c9a18`), so the step is fixed in 1 / distance; our reverse-Z
buffer gets the same step × our near plane (`LevelMaterial::
set_depth_bias`, the shader's `DEPTH_BIAS` variant writes the depth). It
keeps camera-facing fireballs from being cut by the floor they sit on.

Not done: the bank modifiers that run free (50 in `WEAPONS`) aren't
animated on characters and weapons yet (item and generator models run
their banks' — `docs/rendering.md`, "Texture animation"). The game steps
an effect's modifiers by its action's frame and holds the last frame past
the run (wrapping only on a looping action); here they wrap on the tick
count, the same while the clip lasts.
