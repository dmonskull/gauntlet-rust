# Implementation notes

One file per system, written only once it's confirmed against the actual
`main.dol` — not assumed from other Gauntlet ports or similarly-shaped games.

| file | about |
| --- | --- |
| [disc-format.md](disc-format.md) | GameCube disc boot header and DOL executable layout |

## Ghidra project

`~/ghidra-projects/projects/GauntletDarkLegacy` — `main.dol` imported and
auto-analyzed (2962 functions, entry point `0x800051fc`). Driven headlessly
via `analyzeHeadless` and the Cerberus RE bridge
(`~/start-cerberus-bridge.sh`).

## Not started yet

- Level/map file formats (`LEVELS/`, `MAPS/` on disc)
- Actor/monster data (`MONSTERS/`, `CRITTER/`)
- Model/texture formats (`objects.ngc`, `textures.ngc`, `ANIM.PS2` — names
  are original disc filenames, not yet reverse engineered)
- Main game loop / entity update
- Co-op and combat systems
