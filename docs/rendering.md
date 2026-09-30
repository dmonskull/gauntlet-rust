# Rendering a level

Implemented in [`crates/gdl-game/src/world.rs`](../crates/gdl-game/src/world.rs)
and [`level_material.rs`](../crates/gdl-game/src/level_material.rs) /
[`level.wgsl`](../crates/gdl-game/src/level.wgsl).

## What the game does

`FUN_800c3bbc` draws an instance: for each submesh it binds the diffuse
texture to GX texture map 0 and, when the lightmap binding is non-zero, the
lightmap to map 1, then `FUN_800c48c0` decodes the VIF packet and emits GX
triangle strips. Level geometry is lit by its lightmaps (or by prelit
vertex colours), not by dynamic lights.

## What we do

1. For every `WORLDS.PS2` node with a model, place that model's submeshes
   at the node's world position.
2. Merge everything sharing a (diffuse, lightmap) pair into one mesh — a
   level is a few hundred draw calls instead of tens of thousands.
3. Draw with `LevelMaterial`, which reproduces the game's TEV setup
   (`FUN_800c46f8`, programming `GXSetTevColorIn`/`GXSetTevColorOp` at
   `FUN_800ffbe0`/`FUN_800ffce4`):
   - stage 0, always: `clamp(texture × rasterized colour × 2)`
     (`GX_CC_TEXC × GX_CC_RASC`, `GX_CS_SCALE_2`)
   - stage 1, chosen by `FUN_800c48c0` when the submesh has a lightmap
     (render state flag 2): `stage 0 × lightmap alpha` at 1×
     (`GX_CC_CPREV × GX_CC_TEXA`, texture map 1 on texture coordinate 1).
   The rasterized colour of lightmapped geometry is its prelit vertex colour
   (every lightmapped submesh on the disc has one). In gamma space like the
   hardware, and written out in gamma space: the 3D camera renders into a
   float target, so every blend works on gamma values as the GameCube's
   frame buffer does, and `gamma.rs` turns the finished picture linear for
   the sRGB output (before the UI draws). Textures upload as plain
   `Rgba8Unorm`; tonemapping is off.
4. Emit both windings of every triangle (the game doesn't cull this
   geometry).

## Assumptions not yet traced to the binary

- **Dynamic lighting.** Geometry without prelit colours (about 10,000
  submeshes, and all characters) is lit by the game's lights in
  `FUN_800c48c0` (normal · light direction plus ambient); we use a neutral
  0.5 until those lights are implemented.
- The ambient/base term `FUN_800c48c0` adds to prelit colours before
  clamping (`DAT_802cc088`) is assumed 0 for level geometry.
- Alpha: `Mask(0.5)` for textures with transparency, blending for the
  shared-palette formats. The submesh/binding flags that really choose the
  blend mode aren't decoded yet.

## Handedness

The data is right-handed (Y up, counter-clockwise front faces), but the
game shows it through a left-handed view: its projection (built in
`FUN_800c87c8` into the view record at `+0x80`) puts `+1` in the w row, so
the camera looks along +Z in view space, with a negative Y scale; and the
models agree — a character faces +Z with its right side at +X (the
Warrior's `R_WRIST` at x +0.85, `L_WRIST` −0.96; the weapon hangs from
`R_WRIST`). A plain right-handed render is therefore the game's picture
mirrored left to right: on the tower's start the orange crystals (x
15–28) came out on the hero's right, where the game has them on his left.

We draw with `camera::MirroredPerspective` — Bevy's perspective with clip
space flipped left to right — so the picture is the game's. Everything
else stays in the game's own coordinates. Level materials take clockwise
triangles as front faces (the flip reverses winding on screen), the stick
maps right to +X when facing +Z (the game's heading is the camera's yaw +
the stick's angle), and the free camera's strafe and mouse look are
flipped to follow the screen. Camera-facing nodes need nothing: they turn
+Z to the camera as the game does, which the flip then shows the game's
way round.

## Blending and depth (instance render flags)

Each `WORLDS.PS2` node's `+0x18` holds the flags its instance is created
with: `FUN_800aafb0` reads the field, then overwrites it with the parent
pointer, and ORs the value into the instance's `+0x60` (`FUN_800ba658`).
Level artists' node-name codes line up with the bits (`XP` nodes are
additive, `FF` nodes carry `0x1000000`, `CF` nodes `0x4000000`).

`FUN_800c3bbc` draws an instance with `+0x60 & 0x1090D7C0`;
`FUN_800c5894`/`FUN_800c664c` turn the bits into GX state (they're the PS2
GS settings the data was authored for):

| bit | effect |
| --- | --- |
| `0x40` | depth compare always (`GXSetZMode` func 7 instead of 6) |
| `0x80` | no depth writes |
| `0x4000` | skip the lightmap stage |
| `0x800000` | additive: `GXSetBlendMode(BLEND, SRCALPHA, ONE)` (PS2 ALPHA `0x48`; normal is `0x44` = `SRCALPHA, INVSRCALPHA`). Bevy draws `AlphaMode::Add` as premultiplied alpha, so `level.wgsl` outputs colour × alpha with alpha 0 for these (without that, glows drew as dark discs) |
| `0x100`, `0x200`/`0x400`/`0x100000` | tint colour from the instance / fade alpha from instance `+0x53` (not implemented) |
| `0x8000` (→ `0x20000`), `0x10000000` | extra texture stages (not implemented) |

The draw traversal (`FUN_800c7d6c`) skips an instance's subtree on `0x2`
and its own draw on `0x1`. Bits `0x0F000000` pick a facing mode that
`FUN_800c81ac` applies to the instance matrix each frame, from the camera
position (`FUN_800b8f54`): `0x1000000` turns about Y so +Z faces the camera
(`FF` bushes and trees), `0x3000000` also tilts freely, `0x5000000`/
`0x6000000`/`0x7000000` tilt at most 15°/30°/45° (`r2-0x4780..`), and
`0x4000000` takes the camera's rotation (`CF` sprites and glows). We spawn
those instances as their own entities with a `Billboard` component
(`billboard.rs`); `0x2000000` and `0x8000000` (other routines) are unused
by the levels.

Everything else is alpha-blended with an alpha test of > 2. We keep a
cut-out mask for textures whose alpha is only ever 0 or 255, blend textures
with real partial alpha (fog cards, glass), and specialize the level
material's pipeline on the two depth switches. `FUN_800b4490` is the same
table for effects and 2D.

## Lighting (geometry without prelit colours)

Characters, items and any object whose vertices carry no colour are lit
in software by the model interpreter (`FUN_800c48c0`), per vertex, in byte
units (0x80 = 1.0, clamped to 255):

```
colour = objColour × ambient + max(0, N · L) × lightColour × objColour
       (+ each point light: min(1, N·D × strength × (1 − k·|D|²) / |D|²) × colour)
```

`L` is the scene light's direction negated and normalized (`FUN_800ad810`,
×`r2-0x4d68` = −1); `objColour` is the instance's colour (0x80 grey for
ordinary instances). The scene light comes from the level's `WDATA` `LEVL`
record (`FUN_800678f0` → `FUN_800b6568`/`FUN_800b64cc`): ambient `+0xEC`,
direction `+0xF0`, colour `+0xFC`, intensity `+0x108` — ambient 0.8 and a
white light along (−1, −6, 2) in every level except B6. Up to three lights
are supported (`+0x9C` count, `+0xBC` array of the lighting environment);
the levels use one. Prelit vertices are multiplied by the instance colour
instead. Point lights (the `r13-0x6a9c` list) aren't implemented yet.

We light per pixel in `level.wgsl` with the same formula (`SceneLight`,
set from the level's record when it loads).

## Colour space

The GameCube blends in its 8-bit frame buffer, on gamma-encoded colour.
Blending linear light instead (an sRGB target) changes every translucent
and additive layer: dim additive texels all but vanish — the tower's
brazier flames (`P_TORCH`, texels at most 0.3, eight or so overlapping)
drew as a faint glow instead of the flames the game shows — and dark
translucent layers such as blob shadows come out too light. So the level
shader writes gamma-space colour into a float target, blends happen on
those values, and `gamma.rs` decodes the finished picture to linear light
(clamped to 1 first, as the 8-bit buffer saturates). Stand-in left: the
float target doesn't saturate between one blend and the next as 8 bits
do. Bevy's own materials (debug markers) come out darker for it.

## Texture animation (`LEVELS/<level>/ANIM.PS2`)

Despite the name, a level's `ANIM.PS2` is its texture-modifier list
(`texmod.rs`), loaded as "anim" into `0x8028c4ec` (`FUN_800a8b2c`).
Header `{0, 0, count, offset}`, then 0x58-byte records; set up by
`FUN_800185b0` (message "TEXMOD with 0 texidx") and stepped every tick by
`FUN_80018788` (from `FUN_80010a4c`):

| offset | field |
| --- | --- |
| `+0x04` | texture name (32 bytes) |
| `+0x24` | first-frame texture name (16 bytes), used when `+0x48` is −1 |
| `+0x44` | binding of the modified texture in the level's `objects.ngc` |
| `+0x48` | ≥ 0: first frame's binding; −1: look it up by the `+0x24` name; −2 / −3: scroll U / V |
| `+0x4C` | i16: frames, or steps per whole scroll (negative scrolls backwards) |
| `+0x4E` | i16: scroll phase |
| `+0x50` | ticks per step (≤ 1: every tick; checked against the frame counter `r13-0x7580`) |
| `+0x54` | starting step |

A flipbook swaps the binding's texture for `first + step % frames`
(`FUN_800ba278`, which copies the frame's texture record into the bank's
binding slot: every model drawing that texture changes); a scroll writes
`±(step − phase) / |count|` into the scroll table at `0x802c7b08` (U at
`+4`, V at `+0xC`), which the draw path applies as a texture-matrix offset
for bindings flagged `0x40` (`FUN_800c6a78` → `FUN_800c68e4`). All 67
levels' files parse (623 modifiers) and point at real bindings. A first
frame given by name (`+0x48` −1: torches' `TORCH00`) is looked up through
the loaded banks in load order (`FUN_800b82f4` → `FUN_800b8354`); here the
bank's own names, then `WEAPONS`'. We evaluate scrolls between ticks so
they glide.

**Banks.** Model banks' `ANIM.PS2` files end in the same list. At load
(`FUN_80010b4c` → `FUN_800185b0` on each, the bank index in the binding's
high half) every flipbook — whoever owns it — puts its first frame on its
texture (the last listed wins), and only the scrolls with owner −1 get a
scroll slot. Every tick `FUN_8005b974` steps the free-running ones (owner
`+0x00` = −1, `FUN_80010a4c`) of four banks — the level's items
(`r13-0x7180`), the realm's items (`-0x7184`), `WEAPONS` (`-0x7188`) and
`POWERUPS` (`-0x718c`) — and `FUN_8007ba80` three tables per loaded
monster (its folders'). In the item banks these are the torches, `RED_ARROW`, the purple
transporters and `DESTRANS`, `GOLD_CHUNKS`; in `POWERUPS` the potions'
glows, the gems' sheens (U and V scrolls on one texture), the glow rings,
`SPLASH`, `SAND_ANIM`; in monster banks the generators' lava and rock,
fuses. A force field's generator and field (`FFGEN`, `FFIELD`) have
one-frame free modifiers: frame 47 (lit) and 49 (clear) from the first
tick. `population.rs` gives each bank's modifiers to the materials of the
item and generator models drawn from it (the level's own to items built
from the level's model), merged with the level's (`texanim::bank_anims`);
monsters, golems and bosses run their folders' on their own models
(`CharacterModel::build_animated`): the snakes' slithering bodies, the
imps' flames, the ice bombs and fuses, the Skorne bosses' chest, back,
arm and energy-body textures.

**Actions.** A modifier whose owner is an atree index belongs to that
atree's actions: action `+0x28` is how many it runs and `+0x2C` the index
of the first in the list (made a pointer at load, `FUN_8001267c`; all 346
such links on the disc point at their own atree's). While an object plays
an action (`FUN_80011134`), each of them in order (`FUN_80018990`) gets
the action's frame f = ⌊frame + 0.5⌋ — counted down (frames − f − 1) when
action `+0x2A` bit 0 is set; wrapped modulo `count × period` on a looping
action once past it — and:

- a flipbook shows frame `(f − phase) / period` of its run (frame 0
  before `phase`, holding the last) on **that object only**
  (`FUN_800ba85c`: object `+0x5C` = binding, `+0x58` = texture). An
  object has one such replacement, so the last modifier listed wins; it
  stays after the action until another replaces it (objects start with
  none, `0xFFFF`).
- −4 fades the object in, −5 out, over `count` frames from `phase`
  (`FUN_800ba9b0`): t = (f − phase) / count in 0…1, value =
  ⌊(1 − t) × 255⌋ in, ⌊t × 255⌋ out; transparency `+0x53` = 255 − value
  (its opacity), flag `0x200` while value ≥ 1. The tower's rune displays
  and some `WEAPONS` effects use them.
- action scrolls (−2 U, −3 V) are texture wipes (`FUN_80018cb4` →
  `FUN_800ba4ac`): an object's texture coordinates are scaled and
  shifted (a "UV Scale Add" slot, `0x802c7708`, index in object `+0x5E`,
  flag `0x10000000`; identity 1, 0) so the texture's strip slides along
  the mesh — with x = f − phase, m = min(period, count), w = count / m,
  the visible span [start, end] goes [x(1 + w)/m − 2w, 1] while x < m, then
  to [0, w] by x = count, then [(x − count)/m, w] until x = count + m, and
  [1, w] after. −6 does nothing.

Each setter walks from the object down its subtree (param 1): itself, its
children and theirs, but no further into a child whose render flags have
`0x10` — and none of the children at all if the first child has it (the
effects' roots get `0x10`, `FUN_80093858`, so nothing set above reaches
into them).

Atree nodes of kind 3 carry one modifier each (node `+0x34`: the byte
offset from the action table to its record in the list; all 1,626 on the
disc point at their own atree's), run the same way on that node and its
subtree with the action's frame, not wrapped (`FUN_80011334`, walking the
nodes parent first, so a node's modifier wins over the action's below
it). The heroes' power effects, bosses' and monsters' attack effects,
`WEAPONS` (the potions' blasts: 46 flipbook nodes in `MP_ACID`, fades in
`MP_FIRE` and `MP_LIGHT`, the combos' wipes) and a few item effects have
them (the tower's rune displays, `LEGENDFX`, `LEGENDPRJ`, `SAFEREXP`) —
none of the placed items. Every animated object runs these steps — items,
heroes, monsters, effects, the tower's wizard (`FUN_80011104`'s callers).

So a transporter's `ACTIVE` loop swirls `NEWTRAN_` on each pad, a force
field's `ONA` lights `FFGEN` (33…47), `ON` runs `FFIELD` (49…68) and
`ONB` puts `FFGEN` out backwards — leaving its own generator at the clear
frame 33 through `OFF`, while one that has never cycled shows the bank's
lit 47. `items.rs` runs these on copies of the materials of the parts
drawing the texture (`ItemRig::texmod_parts`).

Models built as characters (effects, monsters, critters, the tower's
shards and runestones) run both — the action's and their kind-3 nodes' —
through `texanim::ModelMods`: each node keeps what the modifiers left on
it (a replaced texture, an opacity) and each drawn part shows its node's
on its own copy of its material (`character.rs`, `show_look`). This took
over from the effects' old stand-in, which stepped every modifier an
effect's atree owned on the tick count: the acid blast's gas and rings
now run node by node from their own phases, the fire and light blasts
fade out. Stand-in: action scrolls (the wipes) aren't run yet.

The frames a flipbook steps through can need more blending than the
texture it starts from — `FFIELD`'s base and first frame are clear, its
later frames soft — so a material a flipbook drives blends as its most
demanding frame needs (`LevelMaterial::widen_alpha`; the game blends
everything); a fade makes its part's copy blend while it's see-through.
Not done: `WEAPONS`' and the heroes' banks' running modifiers (the hand
glows, which aren't drawn yet), and the heroes' own actions' modifiers
(their clips and skeletons come from different files).

## Texture overrides

An object's texture override (`FUN_800ba85c`: object `+0x5C` mode, `+0x58`
texture, set on the object and its subtree like the modifiers above) is
read by the draw's texture choice (`FUN_800c3d60`, `FUN_800c3df8`):

| mode | draw |
| --- | --- |
| −1 | none: each submesh's own texture |
| −2 | the override's texture in place of every submesh's |
| −3 | the override's texture, flag `0x80000` |
| −4 | each submesh's own texture, flag `0x8000000`: the override becomes a texture **over** the object |
| ≥ 0 | a flipbook: the override replaces that one binding |

**The texture over the object.** Flag `0x8000000` binds the override as a
second texture (`FUN_800c5894` → `FUN_800c6040`), and the draw
(`FUN_800c48c0`) picks the TEV setup `FUN_800c46f8(2)` for an object with
no lightmap (a lightmapped one keeps mode 1, the lightmap stage, and shows
no override). Mode 2 runs three stages (GX constants as in the SDK):

- stage 0, as always: colour `RASC × TEXC` × 2 clamped, alpha `TEXA ×
  RASA` (`GXSetTevOp(0, GX_MODULATE)`, then the colour inputs and scale of
  "What we do");
- stage 1 (`GXSetTevOrder(1, texcoord 0, map 1, COLOR0A0)`: the override
  sampled with the object's own coordinates): colour `RASC × TEXC(override)`
  × 2 clamped — the object's own texture colour is dropped — and alpha by
  the comparison `GX_TEV_COMP_A8_GT`: the override's alpha where stage 0's
  is above the konst alpha, else 0. Stage 1's konst alpha is `K0_A`
  (`GXSetTevKAlphaSel(1, 0x1C)` at start-up, `FUN_80067b20`), and `KColor0`
  is `r2-0x6490` = (0, 0, 0, 2): above 2/255;
- stage 2 (set once at start-up): colour passed on, alpha `RASA ×` stage
  1's.

`level.wgsl` does this for death textures (a `LevelMaterial` copy with
the frame in the lightmap slot, `params.x` = 2) and hit flashes.

**Hit flashes** use the texture `AAAWHITE`: the powerups bank's
(`r13-0x6f00`, loaded with `CHROMESILVER` by `FUN_800972dc`, which sets
up the effect list) for bodies, the level's own world bank's
(`DAT_8028c4f4`, from the bank `DAT_8028c4f0` that `FUN_800a8dfc` loads
as `WORLD`) for obstacles. It is
an 8×8 CI4 texture of one colour, RGB5A3 `0xDF97` = (189, 231, 189),
opaque, in every bank that has it (the powerups, every realm's items,
67 of the 70 levels: not `levelT4`, `ORIGlevelL1`, `levelC2_acorn`), so
over a body it draws the lit silhouette in a pale green-white.

- **Monsters** (`FUN_8004db94`, the reaction): after a blow of at least 1
  (`r2-0x6f10`) is turned into a reaction, a monster left with hit points
  gets the timed texture effect (`+0x1E4`, `FUN_80090a00(1, fx, AAAWHITE,
  end 1, repeat 1)`); the monster's update steps it (`FUN_80090a48`) and
  draws it (`FUN_80090aec`: mode −4, `+0x6A` = 999, `r2-0x5758`) — on for
  the update it starts in and the next, then mode −1. A death texture
  later takes the same slot.
- **Heroes** (`FUN_80085ca8`): a blow of more than 1 (`r2-0x5c70`) — the
  same one that picks a reaction — starts the same effect on `+0x7DC`,
  stepped by `FUN_80077ccc`. The slot is shared with the heroes' other
  texture effects (the chrome power-ups: `CHROMESILVER`/`CHROMEGOLD`, mode
  −3: here `flash.rs::Retexture`, copies of the body's materials drawn in
  `level.wgsl`'s mode 3, coordinates from the normals —
  [powers.md](powers.md), "`0x10000` invulnerability").
- **Critters** (`0x800382c0`, the damage routine, a blow the critter lives
  through without kind `0x1000000`): kinds `0x100320` set the critter's
  flash `+0xABC` = 2; other kinds, landing on a `NODE` sphere, set that
  node's `+0x548` = 2 (checked in the machine code at `0x80038bc0`: `and.`
  with `0x100320`, then the node branch). `FUN_8003eaa4` (from the critter
  update `FUN_80038cf4`, the body and then each part) shows a positive
  count (mode −4 with `AAAWHITE`, `+0x6A` = 255) and counts it down; at 0
  it turns the override off. A part's flash covers its subtree; the
  body's the whole model.
- **Obstacles** (class 10, `FUN_8005c1c8`): a blow that does damage and
  leaves the item standing (not subtype 0x29) sets `+0xE0` = 1; the item
  update (`FUN_800606e8`) then draws the root object with mode −2 and the
  level's `AAAWHITE` (its own object only, `param_4` = 0) and render flag
  `0x4000` (no lightmap) over the whole model, for that update, and clears
  it. Counts of 2 and more blink (30 fields on, 30 off, one count each 60).

In this rewrite the hit flash's colour and mode ride on the meshes'
`MeshTag` (`flash.rs`, bits 0–23 the colour, 24 over the object, 25 in
place of the texture, 26 no lightmap), read by `level.wgsl`, so a flash
costs no material copies. Stand-ins: node-sphere flashes aren't drawn
(which model node a `NODE` names, `+0x500`, isn't traced); the `+0x6A`
value the flashes set (a sort key, `FUN_800c67a0`) isn't used; heroes'
other texture effects aren't done. A flash's two updates are game ticks;
a frame slower than two ticks can miss one.

## Particle-system nodes

World nodes with node flag `0x800` (named `…PSYSE_FLAME…`, `…PSYSF_SPARK…`)
are particle emitters: their model is only a placeholder shape (a white
pyramid with `AAAWHITE`), which the game doesn't draw; triggers switch
some on and off (`docs/mechanics.md`). `world.rs` leaves the shape out and
[`particles.rs`](../crates/gdl-game/src/particles.rs) runs the flames,
smoke, pool fires, mist, embers and fireflies.

**The records** (`gdl_formats::psys`): a level's `WORLDS.PS2` header words
28/29 are the count and offset of its `0x138`-byte particle records (the
world struct's `+0x9C`/`+0xA0`, `DAT_8028c508`/`DAT_8028c50c`); they end
the file. Fields, by word, with the bit of word 4 that says they're set
and what `FUN_800ceeb8` makes of them:

| words | bit | meaning |
| --- | --- | --- |
| 0 | — | kind, ≥ `0x100` |
| 1 (`+4` i16, `+6` byte) | 1 | built-in preset first (`0x80127fc4`, ids 0–7); the letter `PSYS<letter>` names |
| 2, 3 | — | flag values and mask: `0x80` → instance `0x800000` (additive), `0x100` → `0x40000000`, `0x200` → `0x800`, `0x400` → `0x40`, `0x800` → `0x80`; `1/2/4/0x20/0x40` → emitter flags |
| 5, 6, 7 | 2, 4, 8 | emitter counts (`+0x2E`: ring size, `+0x30`, `+0x32`) |
| 8, 9 | `0x10` | the two emitting phases' lengths, s (× 30 → frames; 999 on every record) |
| 10, 11 | `0x20` | particle life: shortest, plus a random extra, s |
| 14 | `0x40` | spray cone, degrees (half-angle π·v/360; ≥ 359 all round) |
| 15 | `0x8000` | `+0x5C` |
| 0x10–0x17 | `0x4000` | texture name (in the level's `objects.ngc`, or `WEAPONS`) |
| 0x18–0x1A | `0x80` | direction (default up) |
| 0x1B–0x1D | `0x100` | start box half-size |
| 0x1E–0x21 | `0x200` | emission rates, particles/s: phase A start and slope, phase B start and slope (`+0xD0..DC`) |
| 0x22 | `0x400` | rate jitter, % |
| 0x23 | `0x800` | buoyancy: × −32/900 per frame² (negative rises) |
| 0x24 | `0x1000` | `+0x9C` (× −1; not identified) |
| 0x25 | `0x2000` | start speed, units/s |
| 0x26–0x29 | `0x10000` / `0x20000` | four colour keys `0xAARRGGBB` (RGB / alpha) over the life |
| 0x2A–0x2D | `0x40000` | four size keys |
| 0x2E | `0x80000` | a start delay, s |

The library (`FUN_800cbf44`, per emitter per frame) keeps particles in
packed rings filled through emitter callbacks and states 0 (delay) → 2/3
(phase A: rate `+0xD0` + `+0xD4`·t) → 4/5 (phase B) → 6 (done) → 8
(freed); `particles.rs` simulates each particle directly from the record
instead (stand-in), with the phases taken as always on. Unconfirmed: whether
the library counts frames or video fields (twice as many: brighter, busier
torches). `GDL_PARTICLE_TEST=<letter>` with `GDL_LOOK_AT="0,0.5,-6,10"`
on levelA1 shows one record's system in the open. What's known of the
setup:

- `FUN_800aae??` (the world-instance setup, around `0x800aaeb0`): a node
  whose name contains `PSYS` (`r2-0x5000`) takes the letter after it
  (`PSYSE_FLAME` → `E`) and looks for the particle record whose `+0x06`
  byte is that letter in the level's table (`DAT_8028c508`, count
  `DAT_8028c50c`, `0x138` bytes each — the ANIM.PS2 `+0x10`/`+0x14`
  records, [animation-format.md](animation-format.md)); `"Unable to find
  world psys %c"` otherwise. A node with collision (`+0x36` > 0) passes
  its first collision point (× `r2-0x4ff8`) as an offset. It then creates
  the system (`FUN_800cede8` → `FUN_800d0af4`) on the node's instance and
  sets instance flag `0x800000`.
- `FUN_800ceeb8(system, emitter, record, offset)` applies a record: the
  word at `+0x00` must be ≥ 0x100 (`"setWorldParms: WORLDPSYS type is"`);
  `+0x10` is a bit set saying which fields follow — bit 1 first applies the
  built-in preset whose `+0x04` short matches (table `0x80127fc4`, `0x138`
  stride, ends at −1); bits 2/4/8 set emitter shorts `+0x2E`/`+0x30`/`+0x32`
  from words 5–7; 0x10 two clamped rates from words 8–9 (× a constant) to
  `+0x3A`/`+0x3C`; 0x20 two bytes from words 10–11; 0x40 a size from word
  14; 0x80000 a short from word 0x2E; … (not finished).
- `"TOO MANY PSYS OBJECTS"`, `"PSYS requires too many bits"` and
  `"Setting PSYS attribute after draw"` mark the library's other entry
  points (`0x800cc…`–`0x800d1…`).

## Item models

Items built from an atree draw each node with that node's render flags
(blending, depth, camera facing), like characters; hidden nodes and
`…GLOW` nodes (textured by the effects system at run time) are left out.
