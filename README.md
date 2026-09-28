# Gauntlet: Dark Legacy — Rust Engine

A from-scratch Rust/Bevy runtime for the GameCube release of *Gauntlet: Dark
Legacy* (game ID `GUNE5D`), built from reverse engineering the original
`main.dol` with Ghidra and reading public technical references on the
GameCube disc/DOL format.

This project ships no game assets and no code copied from the original
binary. Point it at a GameCube disc image you already own and it reads that
disc's own data at runtime.

## Status

Day zero. `gdl-formats` parses the disc boot header and `main.dol` layout;
`gdl-game` opens a bare window as a plumbing check. Nothing playable yet —
see [`docs/INDEX.md`](docs/INDEX.md) for what's been reverse engineered so
far and what's next.

## Run

```bash
export GAUNTLET_DISC="/path/to/Gauntlet - Dark Legacy (USA).iso"
cargo run -p gdl-game
```

## Reverse engineering

`main.dol` is analyzed with Ghidra (12.1.4) using
[Cuyler36/Ghidra-GameCube-Loader](https://github.com/Cuyler36/Ghidra-GameCube-Loader)
for the DOL loader and its bundled Gekko/Broadway PowerPC SLEIGH language.
The game was built with Metrowerks CodeWarrior, so CodeWarrior demangling
resolves a meaningful slice of runtime-library symbols automatically.

Findings live in [`docs/`](docs/), one file per system, written as they're
confirmed against the actual binary — not guessed from similarly-shaped
games.

## License

Gauntlet: Dark Legacy and all related assets, trademarks and intellectual
property belong to their respective owners. This project is unaffiliated
with them.
