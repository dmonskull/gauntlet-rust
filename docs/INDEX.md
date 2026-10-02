# Implementation notes

**Resuming work? Start with [HANDOFF.md](HANDOFF.md).** How close the
rewrite is to the original, and everything left: [STATUS.md](STATUS.md).

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header, DOL layout, FST filesystem, RVZ images |
| [objects-ngc-format.md](objects-ngc-format.md) | Models: header, names, texture bindings, PS2 VIF geometry |
| [textures-ngc-format.md](textures-ngc-format.md) | Texture formats, palettes, lightmaps |
| [worlds-format.md](worlds-format.md) | Level scene graph and model placement |
| [animation-format.md](animation-format.md) | Skeletons, actions and keyframed animation |
| [chunk-files.md](chunk-files.md) | The `.WAD`/`.ROM` tagged-chunk container, and game text |
| [level-population.md](level-population.md) | Items, generators, monsters, exits and player starts in `WORLDS.PS2` |
| [rendering.md](rendering.md) | How a level is drawn: diffuse × colour × lightmap |
| [camera.md](camera.md) | The play camera: level camera points, per-level distance and bounds, smoothing |
| [first-person.md](first-person.md) | Optional personal views, controls, equipped hands, local panes and interaction rules |
| [player-movement.md](player-movement.md) | Stats → speed, stick → walk/run, per-tick movement and turning, footsteps |
| [audio-format.md](audio-format.md) | DSP-ADPCM sound banks, sound catalog, music streams, level music, positional sounds |
| [collision.md](collision.md) | Level collision triangles, grid, floor/wall queries, actor movement |
| [combat.md](combat.md) | Controls → logical buttons, attack intents and chaining, target search, blows, damage |
| [monsters.md](monsters.md) | Monster stats and tiers, the realm's monster slots, generators, placed monsters, the monster AI and mover, hit reactions, deaths (death textures, die effects); `CRITTER` files |
| [critters.md](critters.md) | The scripted monsters (bosses, golem, gargoyles, general): `CRITTER` file records, loading, spawning, move choice and switching, blows, damage, death; the placed golem runs |
| [projectiles.md](projectiles.md) | Thrown weapons and monster missiles: release, aim, lob, flight, collision, blasts, the throwing AIs |
| [items.md](items.md) | The hero's state (health, gold, keys, potions, powerups), item touch, pickups, doors, chests, exits, transporters, hints |
| [powers.md](powers.md) | The armour and special power-ups: invulnerability, the shields, halo, gas mask, levitation, x-ray, invisibility, time stop, breaths, phoenix, growth, shrinking, the Pojo, Skorne's items, the shop's |
| [frontend.md](frontend.md) | Fonts (`FONTS/*.FNT`, font slots), the 2D screen, title, character select, saving, the in-game menus, death, the loading screens and movies (`VQMOVIES`, MidiVid VQ), GAME OVER |
| [mechanics.md](mechanics.md) | Triggers and what they move (lifts, bridges, doors), rotators, carrying the hero, damage tiles, damaging walls, breakables — decoded; moving collision built, the rest not yet run |

## Confirmed and implemented

- Disc image: boot header, `main.dol` layout, FST — every game file read
  straight from the `.iso` (all 2,481 match an extracted copy byte-for-byte).
- Dolphin's RVZ compressed images (Zstandard, junk packing), read on the
  fly — the whole disc matches the ISO byte-for-byte.
- Locating a user's copy from a disc image (`.iso`/`.gcm`/`.rvz`), extracted
  folder or `main.dol` (`gdl-install`).
- Models, textures, lightmaps and world placement for all 67 levels.
- Skeletons and animation for every player class and monster, skeletal and
  flipbook (`--viewer`).
- The `.WAD`/`.ROM` container every data file uses, and all game text.
- Player class stats, and walking/running a hero around a level from its
  start point at the game's 30 Hz tick with its speeds and turn rate, on the
  level's collision, followed by the game's own play camera.
- Audio: all 65 sound banks, the sound catalog and all 111 music streams
  decode; each level plays its own music, sound effects play by name.
- What populates every level — item types, placements (pickups,
  generators, monsters, doors, triggers, exits, transporters) and locators
  (player starts, boss spawn, camera points) — shown in the level view.
- Level collision (triangles, grid) and the game's floor, wall and
  move-with-collision queries, including the player's own wall/floor chain
  and its size from `PDAT`.
- Monsters: the per-type stat tables, tiers, each realm's monster slots
  (the level data's `gru`/`rat` placeholders become the realm's own),
  generators spawning at the game's rate and limits, placed monsters, and
  the chase/wander AIs walking at the player on the level's collision and
  attacking (hits are messages; no health yet).
- The hero's melee combat: the default GameCube control scheme, attack
  intents, the quick combo, recoveries, slow/power attacks and finishers,
  lunges, low attacks, directional swings, strafing, the target search and
  hit test, strength-derived damage — emitted as `Hit` messages against
  `Targetable` entities.
- Projectiles: the hero's thrown weapon per class (release, aim, reach by
  wind-up, lob, stat-derived damage and speed, models) and the throwing
  monster AIs' arrows and bombs, flying and hitting monsters, generators,
  the hero and the level; bombs burst.
- The hero's record (health, gold, keys, potions, timed powerups) and the
  items it touches: pickups, keyed doors and chests, exits, transporters,
  item animation and the game's hints.
- Magic potions ([effects.md](effects.md)): blast, shield and thrown potion with the game's effect models.
- Monster deaths ([monsters.md](monsters.md) "Deaths"): DEATH or the knock-down while the body dissolves through its death texture; die effects for elemental kills. Animation clips play at the game's rate (rate / 900 s a frame, [animation-format.md](animation-format.md)).
- World particle systems ([rendering.md](rendering.md)): torch flames, smoke, pool fires, mist.
- Level mechanics ([mechanics.md](mechanics.md)): trigger pads, switches
  and chains; lifts, elevators, trap walls, fading bridges and rotators
  (moving their collision and meshes, carrying the hero), with their
  sounds, camera cuts ([camera.md](camera.md)) and shakes; damage tiles and
  damaging walls; breakables (barrels and their contents — Deaths too —,
  exploding and poison barrels, secret walls, hit switches).
- Placed golems woken by their triggers ([critters.md](critters.md)).
- Monster hit and death sounds; generators' damage, experience and sounds.
- The front end ([frontend.md](frontend.md)): fonts, title, character
  select, pause menus, death flow, saving and loading characters.

## Reverse engineering setup

Ghidra project: `~/ghidra-projects/projects/GauntletDarkLegacy` (`main.dol`,
GameCube loader + Gekko/Broadway language). Drive it with `analyzeHeadless`
and small Java `GhidraScript`s, or the Cerberus RE bridge
(`~/start-cerberus-bridge.sh`).

A full decompilation of every function lives at
`~/ghidra-projects/exports/GauntletDarkLegacy-main.dol.c` (2,962 functions,
one `//// FUNCTION <addr> <name>` header each). Grepping it is the fastest
way in: for strings, constants, callers (`FUN_xxxxxxxx(`), and GPU FIFO
writes (`DAT_cc008000`).

Globals are addressed off two base registers set in the entry stub at
`0x80005310`: **`r2 = 0x8034D100`** (read-only constants: `unaff_r2 + -0x...`)
and **`r13 = 0x8034B4E0`** (mutable globals: `unaff_r13 + -0x...`). Map an
address to a file offset with the DOL section table
([disc-format.md](disc-format.md)).

The retail binary kept its debug strings (asserts, error messages, file
names), which is how most systems here were found.

## Not reverse engineered yet

Kept current in [STATUS.md](STATUS.md) "What's left" (this list was
outgrown: the critters, the bosses, the HUD, the effects, the quest and
the audio's positional sounds have since been decoded and ported).
- [coop.md](coop.md): local co-op — the party, devices, joining, per-player settings.
