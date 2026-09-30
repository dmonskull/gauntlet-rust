# Gauntlet: Dark Legacy — Rust Engine

A from-scratch Rust/Bevy runtime for the GameCube release of *Gauntlet: Dark
Legacy* (game ID `GUNE5D`), built from reverse engineering the original
`main.dol` with Ghidra and reading public technical references on the
GameCube disc/DOL format.

This project ships no game assets and no code copied from the original
binary. Point it at a copy of the game you already own and it reads that
copy's own data at runtime.

## Run

```bash
cargo run -p gdl-game
```

On first launch a file picker asks for your game. Any of these work:

- the disc image — `.iso` / `.gcm`, or Dolphin's compressed `.rvz`
  (read directly, no conversion)
- an extracted disc folder (plain extraction, or Dolphin's
  *Extract Entire Disc* `sys/` + `files/` layout)
- the extracted `main.dol`

It checks it's really Gauntlet: Dark Legacy, remembers the choice, and
loads everything from there. Next launch goes straight in.

You can also pass the game directly, which is remembered too:

```bash
cargo run -p gdl-game -- "/path/to/Gauntlet - Dark Legacy (USA).iso"
cargo run -p gdl-game -- "/path/to/Gauntlet - Dark Legacy (USA).rvz"
cargo run -p gdl-game -- --level levelC1   # pick a level
cargo run -p gdl-game -- --forget          # forget the remembered game
```

`GAUNTLET_GAME` works in place of the path argument.

A character viewer shows any player class or monster with its animations
(`[` / `]` action, `Tab` next character):

```bash
cargo run -p gdl-game -- --viewer --character KNI --action RUN1
cargo run -p gdl-game -- --viewer --monster LICH
```

Dolphin's other compressed formats (`.gcz`, `.wia`, `.ciso`, and RVZ
made with bzip2/LZMA instead of the default Zstandard) aren't supported —
convert to RVZ or ISO in Dolphin (right-click → *Convert File…*).

Your game files are only ever read. The remembered path lives in
`gdl-artifacts/settings.txt` and the options (master, music and effects
volume) in `gdl-artifacts/options.txt`, next to the executable (override the folder
with `GDL_ARTIFACTS`).

## Status

Boots straight from your disc (ISO or RVZ), validates all 67 levels, and
plays them in 3D with the game's own geometry, textures, baked lightmaps,
blending (additive glows, fog cards) and camera-facing foliage. The level's
own music plays.

A hero (Warrior by default; `--character VAL --variant RED` for others)
starts on the level's start point and runs around with the game's own
speeds, turn rate, stride animations and collision (walls, ledges, steps),
at its 30 Hz tick with smooth interpolation. The camera is the game's: it
follows the level's own camera points and distances.

| key | |
| --- | --- |
| WASD / left stick | move (Shift walks) |
| `J` / A | attack (hold or tap for combos) |
| `L` / Y | power attack (finishes a combo) |
| `H` / B | defend (turbo when held with attack) |
| `U` / X, `P` / L, `O` / R, `G` / Z | magic, charge, strafe, combo move |
| `-` / `=` | master volume down / up (5% steps; starts at 25%) |
| `Esc` or `Enter` / Start | pause menu (and the front end: arrows move, Enter/`J` accept, Esc/`H` back) |
| F1 | developer overlay |
| `C` | free camera (WASD fly, Space/Ctrl up/down, Shift fast, right-drag look, wheel speed) |
| `[` / `]` | previous / next level |
| `I` | cycle item models / debug markers |
| `K` | collision overlay |
| `M` / `N` | mute music / play the level's next sound effect |

A bare run opens on the game's title screen (the game's own fonts and
art), then character select, and the tower; `--level` or `--character`
go straight into play. `Esc`/Start pauses (Settings → Audio has the
volume sliders).

Generators pour out the realm's own monsters, which chase, shoot arrows,
lob bombs and hit the hero, crying out as they're hit and die; the hero
fights back with the game's combos, finishers, throws, turbo attacks and
target search, earning experience and levels. Wounded monsters hit softer,
broken generators weaken and vanish. The hero keeps the game's health,
gold, keys, potions and powerups; pickups, keyed doors, exits and
transporters work. Switches and pads raise elevators, drop trap walls,
show bridges and run lifts that carry the hero; spikes, flame vents, force
fields and saw blades cycle and hurt; barrels break open (dropping what
they hold) or explode, and secret walls can be broken.

Not yet: bosses and golems (the critter system is decoded and being
built), using magic potions, the effects system (particles, flames, magic
blasts), co-op. [`docs/INDEX.md`](docs/INDEX.md) has what's confirmed.

Environment switches for testing: `GDL_SCREENSHOT=out.png` (with
`GDL_SHOT_AT=<frame>`) saves a screenshot and exits, `GDL_STICK=x,y` holds
the stick, `GDL_FREE_CAMERA=1` starts in the free camera, `GDL_WARP="x,y,z"` (start the hero there), `GDL_LIST_NEAR="x,y,z"` (log level objects near a point), `GDL_LOOK_AT="x,y,z[,distance[,yaw]]"` (pin the camera on a point; yaw in degrees turns it round from −Z), `GDL_PARTICLE_TEST=<letter>` (that particle record 6 units ahead of the start), `GDL_POTIONS=n` (potions at level start), `GDL_TOUR=<seconds>` (move to the next level every that many seconds; `GDL_TOUR_STEP=0` reloads the same one), `GDL_MEMSTATS=1` (logs entity and asset counts every 2 s), `GDL_FPS=1` logs
the frame rate, `GDL_BUTTONS=attack@20-21,power` holds buttons,
`GDL_DUMMY=4` places a practice target, `GDL_DEBUG_HUD=1` shows the
developer overlay.

## Crates

| crate | about |
| --- | --- |
| `gdl-formats` | Parsers for the game's on-disc formats (disc, RVZ, FST, models, textures, worlds, audio) |
| `gdl-install` | Finds and validates a user's copy of the game, read-only file access |
| `gdl-game` | The runtime: launcher, boot, level rendering, camera |

## Development

`GDL_SCREENSHOT=out.png cargo run -p gdl-game` renders a few frames, saves a
screenshot and exits. `cargo run -p gdl-formats --example audio_dump --
STREAMS/CASTLE1.ads out.wav` (or a `.VBK` and an output folder) decodes
game audio to `.wav`. Tests that need game data use your own copy via
`GAUNTLET_DISC` / `GAUNTLET_ASSET_ROOT` and skip cleanly without it.

## Reverse engineering

`main.dol` is analyzed with Ghidra (12.1.4) using
[Cuyler36/Ghidra-GameCube-Loader](https://github.com/Cuyler36/Ghidra-GameCube-Loader)
for the DOL loader and its bundled Gekko/Broadway PowerPC SLEIGH language.
Findings live in [`docs/`](docs/), one file per system, written as they're
confirmed against the actual binary — not guessed from similarly-shaped
games.

## License

Gauntlet: Dark Legacy and all related assets, trademarks and intellectual
property belong to their respective owners. This project is unaffiliated
with them.
