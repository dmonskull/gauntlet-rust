# Gauntlet: Dark Legacy — Rust Engine

A from-scratch Rust/Bevy runtime for the GameCube release of *Gauntlet: Dark
Legacy* (game ID `GUNE5D`), built from reverse engineering the original
`main.dol` with Ghidra and reading public technical references on the
GameCube disc/DOL format.

**The goal: a 1:1 rewrite** — every level, monster, boss, item and
mechanic working exactly as in the original — **plus convenient extras**
the original never had: online co-op for up to four players with invite
codes (each with their own camera, or the classic overhead one), Windows,
macOS and Linux builds, widescreen, rebindable keyboard and mouse or any
pad (switch at will), and reading your disc image directly. How close it
is today is under [Progress](#progress).

This project ships no game assets and no code copied from the original
binary. Point it at a copy of the game you already own and it reads that
copy's own data at runtime.

## Download

Ready-to-run builds for Windows, macOS and Linux are on the
[Releases page](https://github.com/dmonskull/gauntlet-rust/releases):
unzip, run `gdl-game`, and pick your own copy of the game when it asks.

### Which copy of the game you need

The **original USA GameCube release, Rev 0**: game ID `GUNE5D`, disc
revision 0 ([Redump #7032](http://redump.org/disc/7032/)). Everything here
is checked against it. A full disc image of it matches:

| Size | CRC-32 | MD5 | SHA-1 |
| --- | --- | --- | --- |
| 1,459,978,240 bytes | `5bd30dd8` | `308cab2e7a82f72a5aed2209d766f65a` | `8046ffb66a16a612277df12246390ed21b6db4ad` |

The later USA **Rev 1** ([Redump #70030](http://redump.org/disc/70030/),
also `GUNE5D`) and the European release (`DL-DOL-GUNP-EUR`,
[Redump #16413](http://redump.org/disc/16413/)) haven't been tested: they
load with a warning. Online, every player needs the same game ID and
revision. To check yours in Dolphin: View → List Columns → Revision shows
the revision, and right-click → Properties → Verify computes the hashes.

## Progress

How close the rewrite is to the original GameCube game, judged area by area
against the original's own code ([docs/STATUS.md](docs/STATUS.md) has the
details and the full to-do list):

| | |
| --- | --- |
| **Faithful to the original, one player** | `████████░░` **≈ 84 %** |
| **Done overall** (2–4 player co-op, every level checked, no known issues) | `████████░░` **≈ 83 %** |

| area | done |
| --- | --- |
| Disc, files and formats | `██████████` 97 % |
| Level look (geometry, lighting, texture animation, particles) | `█████████░` 88 % |
| Hero movement, camera, cuts | `██████████` 97 % |
| Hero combat, magic, power-ups | `█████████░` 88 % |
| Monsters and generators | `████████░░` 77 % |
| Bosses and critters | `████████░░` 78 % |
| Items, pickups, doors, exits, hazards | `█████████░` 93 % |
| Level mechanics (triggers, lifts, moving objects, secret walls) | `█████████░` 86 % |
| What each level places and hides | `█████████░` 90 % |
| Every level played start to exit | `████░░░░░░` 35 % |
| Quest, tower, saving | `█████████░` 90 % |
| Front end, menus, HUD, hints | `█████████░` 88 % |
| Audio | `█████████░` 85 % |
| Local co-op (2–4 players) | `███████░░░` 72 % |
| Speed and stability | `█████████░` 85 % |

### Beyond the original

- **Online co-op** for up to four players over the internet: invite codes,
  no port forwarding, each player with their own screen, camera and
  settings, watching a teammate while down. Friends can join a game under
  way, heroes can be changed mid-game, and if the machines ever disagree
  the level simply starts again for everyone.
- Optional **first-person view**, independently chosen by each player:
  animated equipped weapons and hands, mouse/right-stick look, throws that
  go where you look, and local split screens when needed.
- Windows, macOS and Linux; any window size, widescreen included.
- Keyboard and mouse with rebindable keys, or any pad — each player can
  switch between them at will.
- Reads your disc image directly (`.iso`, `.gcm`, `.rvz`) or an extracted
  folder.

### Still to do, biggest first

1. Play every level from start to exit and finish the level-by-level audit
   (47 of 67 levels left): levers, lifts, doors and keys all working.
2. Monsters: several AIs still use a stand-in chase; monsters don't yet push
   each other.
3. Bosses: missile lifetimes, the blast areas' hit guards, head stumps and
   look nodes (grabs, the lich's aura, the health meters and the key are in).
4. Screens: options, the memory card screens, the attract mode and the
   ending movies (the shop, inventory, after-level screens, the levels'
   loading maps and opening movies are in).
5. Effects: texture wipes, effect lights, weapon and hand glows, exact
   particle maths.
6. Audio: music switching and ducking.
7. Co-op: co-op combos (decoded, not yet ported), the monsters' crowd
   penalty.

## Play

[PLAYING.md](PLAYING.md) is the players' guide: starting the game, local
co-op, and **online co-op** (Title → Start → Online Game → Host / Join, the
invite code through the clipboard). `tools/package.sh` builds a release zip
for this machine; the `build` GitHub workflow builds Windows, macOS and
Linux ones.

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
| arrows / D-pad | the power menu: Up opens it and turns the power shown on or off, Left/Right pick, Down closes (a power picked up is held until turned on) |
| `J` / A | attack (hold or tap for combos) |
| `L` / Y | power attack (finishes a combo) |
| `H` / B | defend (turbo when held with attack) |
| `U` / X, `P` / L, `O` / R, `G` / Z | magic, charge, strafe, combo move |
| `-` / `=` | master volume down / up (5% steps; starts at 25%) |
| `Esc` or `Enter` / Start | pause menu (and the front end: arrows move, Enter/`J` accept, Esc/`H` back; on a pad A accepts, B or X backs) |

Options → Controls has the game's own control styles (with its
controller diagram), rumble, auto aim and auto attack. Options or
Settings → **PC Settings** rebinds the keyboard, mouse and pad, and has
video (full screen, vsync) and debug switches; everything is saved with
the options. An Xbox pad works out of the box: A attack, Y power attack,
X turbo/defend, B magic, LT/LB charge, RT strafe, RB combo move, Start
pause, D-pad power menu.

Developer keys, only with `GDL_DEV_KEYS=1` or PC Settings → Debug →
Developer Keys (the original has none, and they sit among the controls,
so a stray press would change the game):

| key | |
| --- | --- |
| F1 | developer overlay |
| `C` | free camera (WASD fly, Space/Ctrl up/down, Shift fast, right-drag look, wheel speed) |
| `[` / `]` | previous / next level |
| `I` | cycle item models / debug markers |
| `K` | collision overlay |
| `M` / `N` | mute music / play the level's next sound effect |

A bare run opens on the game's title screen (the game's own fonts and
art), then character select, and the tower; `--level` or `--character`
go straight into play. `Esc`/Start pauses (Settings → Audio has the
volume sliders). In the tower, Manage Character → Save keeps the hero
(level, gold, keys, potions, runestones, crystals and the realms beaten),
and Load on the select screen brings it back; a file stands in for the
memory card ([`docs/frontend.md`](docs/frontend.md), "Saving").

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

Not yet: co-op, the shop and inventory, the legendary weapons, and
multi-part bosses such as the chimera (the other bosses are untested).
[`docs/INDEX.md`](docs/INDEX.md) has what's confirmed.

Environment switches for testing: `GDL_SCREENSHOT=out.png` (with
`GDL_SHOT_AT=<frame>`) saves a screenshot and exits, `GDL_STICK=x,y` holds
the stick, `GDL_FREE_CAMERA=1` starts in the free camera, `GDL_WARP="x,y,z"` (start the hero there), `GDL_LIST_NEAR="x,y,z"` (log level objects near a point), `GDL_LOOK_AT="x,y,z[,distance[,yaw]]"` (pin the camera on a point; yaw in degrees turns it round from −Z), `GDL_PARTICLE_TEST=<letter>` (that particle record 6 units ahead of the start), `GDL_POTIONS=n` (potions at level start), `GDL_TOUR=<seconds>` (move to the next level every that many seconds; `GDL_TOUR_STEP=0` reloads the same one), `GDL_MEMSTATS=1` (logs entity and asset counts every 2 s), `GDL_SAVE_DIR=<dir>` (where `characters.ron` is kept), `GDL_KEYS=n` and `GDL_CRYSTALS="<counter>:<n>,…"` (start with keys or crystals), `GDL_MENU="start@60,down@70,accept@80"` (press front-end buttons on those frames), `GDL_MUTE=1` (silence this run; the saved volumes stay), `GDL_SHOT_CLOCK=ticks` (count `GDL_SHOT_AT`/`GDL_SHOT_EVERY` in 30 Hz game ticks, so shots land on the same moment of play every run), `GDL_THROWER=<distance>,<ai>,<tier>` (a grunt in front of the hero, e.g. `15,0x12,6` a suicide runner), `GDL_FPS=1` logs
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

## How it was made

Made with Anthropic's **Claude Opus 5.5 at max effort ("ultracode")** in
Claude Code, working with **multiple subagents** side by side: reverse
engineering the original `main.dol`, writing the engine, and testing every
level, local co-op and online play.

## Legal

A fan project, not affiliated with or endorsed by Midway, Warner Bros.
Interactive or Nintendo; *Gauntlet* and *Gauntlet: Dark Legacy* belong to
their owners. The code is MIT-licensed ([LICENSE](LICENSE)); no game data
is included or distributed — you need your own copy of the game.
