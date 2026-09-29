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

- the disc image — `.iso` / `.gcm`
- an extracted disc folder (plain extraction, or Dolphin's
  *Extract Entire Disc* `sys/` + `files/` layout)
- the extracted `main.dol`

It checks it's really Gauntlet: Dark Legacy, remembers the choice, and
loads everything from there. Next launch goes straight in.

You can also pass the game directly, which is remembered too:

```bash
cargo run -p gdl-game -- "/path/to/Gauntlet - Dark Legacy (USA).iso"
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

Dolphin's compressed formats (`.rvz`, `.gcz`, `.wia`, `.ciso`) aren't
supported yet — convert to ISO in Dolphin (right-click → *Convert File…*).

Your game files are only ever read. The remembered path lives in
`gdl-artifacts/settings.txt` next to the executable (override the folder
with `GDL_ARTIFACTS`).

## Status

Boots straight from your disc, validates all 67 levels, and renders any of
them in 3D with the game's own geometry, textures and baked lightmaps. A
hero (Warrior by default; `--character VAL --variant RED` for others) runs
around the level with the game's own speeds, turn rate and walk/run
animations at its 30 Hz tick: WASD or a gamepad's left stick, Shift to
walk. `C` switches to a free camera (WASD fly, Space/Ctrl up/down, Shift
fast, right-drag to look, mouse wheel for speed); `[` / `]` switch levels.
The level's own music plays (`M` mutes it; `N` steps through the level's
sound effects).

Not a full game yet: collision, monsters, combat and items are still being
reverse engineered. See [`docs/INDEX.md`](docs/INDEX.md) for what's
confirmed and what's next.

## Crates

| crate | about |
| --- | --- |
| `gdl-formats` | Parsers for the game's on-disc formats (disc, FST, models, textures, worlds, audio) |
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
