# Handoff: where the rewrite stands and how to continue

Last updated 2026-09-29. Read this first when resuming.

## State of `master`

Builds clean (`cargo clippy --all-targets` has no warnings) and `cargo test`
passes. The all-levels smoke test passed all 67 real levels; `levelC2_acorn`
and `levelT4` are empty folders and are expected to fail.

What plays (details in [INDEX.md](INDEX.md)):

- **Boot and front end**
  - Boots from a disc image, an RVZ or an extracted folder.
  - Title, character select, pause menus with volume sliders, death flow,
    GAME OVER.
  - A bare run opens on the title; `--level` goes straight into play.
- **Levels**
  - Drawn with lightmaps and blending. Additive glows are correct; they used
    to draw as dark discs (fixed with premultiplied output in `level.wgsl`).
  - Particle emitter nodes are hidden, because their flames and sparks
    aren't ported yet.
- **The hero**
  - Movement, combat, combos, throws and turbo attacks.
  - Experience and levels, pickups, doors, exits and transporters.
- **Monsters**
  - Generators and placed monsters, chase AIs, arrow and bomb throwers.
  - Fireball AIs fire on their attack blows.
  - Hit and death sounds (close and far versions).
- **Generators**: level-scaled damage, armour, experience ×5, realm sounds.
- **Level mechanics** (`mechanics.rs`)
  - Triggers and chains; lifts, elevators and trap walls.
  - Bridges fade in and out; rotators turn.
  - The hero rides moving platforms, and lift pads ride their lifts.
  - Mover, bridge and rotator sounds.
  - Camera cuts with cinematic bars, and camera shakes.
- **Hazards** (`hazards.rs`): damage tiles cycle and hurt; damaging walls.
- **Breakables** (`breakables.rs`)
  - Barrels break and drop their contents with models; Deaths come out of
    barrels.
  - Exploding and poison barrels, secret walls, hit switches.
- **Golems** (`critters.rs`): statues woken by their trigger fight, block,
  take hits and die.

Test aids:

- `GDL_WARP="x,y,z"`: start the hero at a point.
- `GDL_LIST_NEAR="x,y,z"`: log the level objects near a point.
- `GDL_FPS=1`: frame rate, plus a line for every frame over 20 ms.
- `GDL_WAKE_STATUES`, `GDL_CRITTER_HP`: critter testing.
- `GDL_MENU`, `GDL_BUTTONS`, `GDL_STICK`, `GDL_SHOT_AT` + `GDL_SCREENSHOT`:
  scripted input and screenshots.

Handy test spots are listed in [mechanics.md](mechanics.md) and
[camera.md](camera.md) (levelA1 elevator switch, levelA4 lift, barrels,
the levelA2 Death barrel).

## Latest check

The all-levels smoke test on master `1853c94` (bosses, throw aim, golem
fix) passed all 67 real levels; only the two empty folders fail.

## Work in progress

- **In-game HUD**: merged (`46a1365`). Player 1's bottom panel draws with
  the game's art (frame, class portrait, keys, potions, gold, health,
  "LV n", runestones, turbo meter). The old text line shows only with F1 or
  `GDL_DEBUG_HUD=1`. Not drawn yet: the turbo flash and glow, the
  legendary-key row, the quest and rune-13 icons, and the "Wait In Tower"
  prompt. See [frontend.md](frontend.md) "In-game HUD".
- **Effects and magic potions**: handed to the projectiles helper (branch
  `worktree-agent-a8dd539691ef57ca7`) as its current job. When it
  reports, review, merge, test and smoke test.

- **Bosses**: the B6 dragon is merged into master (`5e74b35`). It wakes,
  uses fireballs, breath, claws and stomp, hits the hero, takes hits and
  dies. Tests and clippy pass.
  - Fixed: blows on a critter's spent body-part sphere now land on the body
    in full, as in the game. The golems used to stop taking damage.
  - Stand-in: the dragon's fireball draws as an orange glow sphere (its
    model needs the effects system).
  - Victory: the boss's death marks the realm in `PlayerState::realms_beaten`
    (the record's `+0x1EC8`, `FUN_8001b854`). After 5 s the party goes back
    to the tower (a stand-in: the game's heroes pick up the key (action `0x1C`, PICK), the
    `BOSSKEY`/`BOSSKEY2` effect shows, and their exit state
    (`FUN_8007692c` case 4 → `FUN_80077ccc`) ends the level). Tested on B6
    with `GDL_WARP="-3.2,29.7,-12" GDL_CRITTER_HP=0.02` + attack.
  - Not done: the `BOSSKEY` effect, the key pick-up, the boss intro and
    camera, and multi-part bosses such as the chimera; the other bosses are
    untested.
- **Throw aim** (a user report): throws now aim from the release height at
  the target's centre, tilting up or down, and look 200 units out on boss
  levels. See [projectiles.md](projectiles.md), "The throw's target".
- **Warm-up hitches**
  - The early frames of a level can take 20–40 ms.
  - A pre-warm was added (level meshes skip frustum culling for 3 frames).
  - It isn't verified yet: measure with `GDL_FPS=1` while no other build is
    running.

## Next jobs, in order

Run one helper agent at a time; each job ends in a report, then the agent
waits.

1. **Finish bosses**: A5 chimera or B6 dragon first, then the rest. See
   [critters.md](critters.md).
2. ~~In-game HUD~~: done (the remaining HUD details are listed above).
3. **Effects system and magic potions** (in progress with the projectiles agent)
   (it knows the effect slots at `0x802855ac` and `FUN_80094418`):
   - Potions are used through `FUN_80076618`: mode 0 is the magic blast
     (`FUN_8009262c`, radius 40, damage = magic power `+0x10C`), mode 1 the
     shield, mode ≥ 2 the thrown potion.
   - The blast's damage falls off over time in `FUN_80094418`.
4. **World particle systems** (torch flames, sparks):
   - What's decoded so far is in [rendering.md](rendering.md) "Particle-system
     nodes": ANIM.PS2 `0x138`-byte records, selected by the letter after
     `PSYS` in the node name.
   - Records are applied by `FUN_800ceeb8`; the update is `FUN_800cdfdc`.
5. **Tower progression**, partly decoded in [items.md](items.md) "Quest
   items and the tower's gates":
   - gem colours and their requirements;
   - gargoyle sections;
   - quest triggers (flag 0x40) gating realms;
   - UnlockLevel / UnlockSection.
6. **Co-op** (up to 4 players), **hints 0x14/0x15/0x1B**, **hit flashes**,
   **monster hit-effect sparks**.

## Working rules (from the user)

- At most **one helper agent** besides the main session. It gets one
  bounded job, commits on its branch, reports, and stops.
- When a batch or smoke retest fails partway, **resume from the failing
  level**, not level 1. Don't rebuild the binary while a batch run is going.
- Never commit game assets.
- Keep retail addresses and decompiler names (`FUN_`, `DAT_`, `0x80…`) out
  of `crates/`; they belong in `docs/` only. The `no_retail_addresses` test
  checks this.
- Only confirmed behaviour goes in; label stand-ins as stand-ins.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Volume defaults to 25%: `-` / `=` keys, or Settings → Audio.
- The user can set a standing goal with `/goal` (only they can run it), e.g.
  `/goal get Gauntlet: Dark Legacy working 1:1 with the original, running
  better, with zero issues or flaws`.

## Tools

- Ghidra dump: `~/ghidra-projects/exports/GauntletDarkLegacy-main.dol.c`.
  Find a function with
  `awk '/^\/\/\/\/ FUNCTION/{p=($3=="<addr>")} p'`.
- DOL constant reader: `scratchpad/mydol.py` (`f32`, `f64`, `u32`, `cstr`,
  with `R2 = 0x8034D100`).
- Session scratchpad scripts (`smoke.sh`, `triggers.py`, `lifts.py`,
  `gems.py`, `world.py`) live in the session's temp scratchpad and may be
  gone. `smoke.sh` just runs every level folder with
  `GDL_BUTTONS=attack GDL_STICK="0.4,1" GDL_SHOT_AT=400` under a
  120 s timeout and greps for panics.
- Disk: `target/` grows large (~26 GB). Agent worktrees have their own
  `target/` dirs; delete those after merging.
