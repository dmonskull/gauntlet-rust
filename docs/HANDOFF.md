# Handoff: where the rewrite stands and how to continue

Last updated 2026-09-30. Read this first when resuming.

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
  - Saving: Tower Menu → Manage Character → Save writes the hero's record
    to `characters.ron` (`saves.rs`; `GDL_SAVE_DIR` moves it), and Load
    (New/Load or Manage Character) brings it back: level, experience,
    gold, keys, potions, runestones, realms beaten and the quest
    ([frontend.md](frontend.md) "Saving"). Checked with scripted menus:
    a save from the tower with 3 keys and 7 orange crystals, then a fresh
    start → Load → the character came back with its keys and name.
- **Picture and placement** (checked against a screenshot of the original's
  tower start, which the user provided)
  - The 3D view is mirrored like the game's left-handed camera
    (`camera::MirroredPerspective`, clockwise fronts, the stick's right is
    +X facing +Z): before, everything was the mirror image of the original
    — the tower's orange crystals sat on the hero's right instead of his
    left, heroes held weapons in the left hand ([rendering.md](rendering.md)
    "Handedness").
  - Blending happens in gamma space like the GameCube's frame buffer (a
    float target and a decode pass, `gamma.rs`): additive fire and glows
    show (the tower's brazier flames were faint glows), dark translucent
    layers are as dark as the game's ("Colour space").
  - Only the placements a one-player game makes are drawn (40% of a level's
    placements are extra copies for 2–4 players); barrels (flipbook
    atrees) are drawn and play their break; the tower's arrival start
    follows the realm last played; the idle wizard (`GWIZ`) reads at his
    podium on the tower's pedestal; lookouts and the boss's spot use the
    game's locator rotation (the C5 djinn faced away from the heroes)
    ([level-population.md](level-population.md), [items.md](items.md)
    "The tower's wizard").
  - The light from the tower's window (`L1XPLIGHTRAY01`) shone in every
    game; the game shows it only once all eight shards are in
    ([items.md](items.md) "The tower's shards and runes").
  - Boss levels now have the game's boss camera (`boss_camera.rs`): the
    opening from the entry's starting point, the heroes framed from the
    nearest camera point before the boss wakes, then the boss (key,
    wizard) framed with the heroes along the way from them, eased with
    the game's limits ([critters.md](critters.md) "Boss camera").
  - Boss levels' safe rocks were missing (no `SAFEROCK` model): they show
    their stage, `SAFEROCK<placement count>` (A5 1, B6 and K5 3), block
    only while standing, and I5 starts without them because the yeti
    throws its own down ([critters.md](critters.md) "Safe rocks").
- **Levels**
  - Drawn with lightmaps and blending. Additive glows are correct; they used
    to draw as dark discs (fixed with premultiplied output in `level.wgsl`).
  - Item and generator models run their banks' texture animations the way
    the level's do: potions' glows, gems' sheens, torches, arrows, the
    generators' lava; transporters swirl and force fields light up, run
    and fade through their actions' own modifiers (per model, on copies of
    its materials). An animated texture blends as its most demanding frame
    needs: force fields were drawn as hard-edged opaque blotches
    ([rendering.md](rendering.md) "Texture animation").
  - World particles (torch flames, smoke, fires, mist) run from the
    level's `PSYS` nodes (see "World particles" below).
- **The hero**
  - Movement, combat, combos, throws and turbo attacks.
  - Experience and levels, pickups, doors, exits and transporters.
- **Monsters**
  - Generators and placed monsters, chase AIs, arrow and bomb throwers.
  - Fireball AIs fire on their attack blows.
  - Hit and death sounds (close and far versions).
  - Deaths as in the game: DEATH or the knock-down plays while the body
    dissolves through its death texture (blood, or fire/electric/light/acid
    for magic, knights' and trees' own), then it's gone; small monsters go
    at once; kills leave their die effect ([monsters.md](monsters.md)
    "Deaths").
  - Every blow that hurts a monster sprays blood (or its element's hit
    effect); kills spray more.
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
- **Suicide runners** (AI `0x12`, placed on 40 levels): wait, yell, run at
  the hero and blow up on contact or after 4 s (or when killed): a
  fireball (poison cloud in realms G and K) that hurts the hero and nearby
  monsters, chaining into other runners ([monsters.md](monsters.md)
  "Suicide runners"). Before this they walked up and "hit" every couple of
  ticks (no attack clips).
- **Effects**: each effect runs its own texture flipbook (the fireball,
  gas clouds, rings, hit sparks) and has the game's depth bias, so
  camera-facing sprites aren't cut by the floor ([effects.md](effects.md)).
  Only heroes' blows earn experience (a monster's bomb or blast doesn't).
- **Tower progression** (`quest.rs`): gems count toward their colour's
  realm, gargoyle pieces toward the tower's wings; the tower's realm gates
  stay shut ("You need 15 Orange Crystals…") until the crystals are there,
  when the tower announces the unlock with its voice; exits to levels not
  reached yet show `EXIT_OFF` and their glow is hidden; scrolls show their
  text ([items.md](items.md) "Quest items and the tower's gates").

Test aids:

- `GDL_WARP="x,y,z"`: start the hero at a point.
- `GDL_LIST_NEAR="x,y,z"`: log the level objects near a point.
- `GDL_FPS=1`: frame rate, plus a line for every frame over 20 ms.
- `GDL_WAKE_STATUES`, `GDL_CRITTER_HP`: critter testing.
- `GDL_MENU`, `GDL_BUTTONS`, `GDL_STICK`, `GDL_SHOT_AT` + `GDL_SCREENSHOT`:
  scripted input and screenshots. `GDL_SHOT_CLOCK=ticks` counts shots in
  game ticks, so a burst lands on the same moment of play every run (frame
  counts drift with shader warm-up).

Handy test spots are listed in [mechanics.md](mechanics.md) and
[camera.md](camera.md) (levelA1 elevator switch, levelA4 lift, barrels,
the levelA2 Death barrel).

## Latest check

The all-levels smoke test on master `c2424af` (the mirrored view, gamma
blending, one-player placements, barrels, the tower's wizard and arrival
starts, boss facing) passed all 67 real levels.

Before that, the smoke test on master `58d602b` (suicide runners, effect
flipbooks and depth bias, the helper's chimera heads merged) passed all 67
real levels.

Before that, the smoke test on master `b089b35` (boss intro, the level-change
crash fix, shared materials, saving) passed all 67 real levels (`DEMO1`
included; `ORIGlevelL1` is a leftover folder the game doesn't list).

## Fixed from user reports (2026-09-30)

- **Monsters swinging and running too fast**: an action's rate is time per
  frame (rate / 900 s), not frames per second. Most monster attacks and
  some walks are rate 60, so they ran 4× too fast (a grunt landed ~5 blows
  a second; now one every ~0.6 s as in the game). Fixed for everything
  animated (heroes too, whose 45/60/15-rate clips were off the same way),
  with the game's end-of-clip and loop timing; see
  [animation-format.md](animation-format.md) "Playing an action". The
  critters use the same clock (`advance_clip`), and effect lifetimes too.
- **Enemies vanishing in place**: see "Deaths" above.
- Also: "big monster" is the floor step `+0x23C` > 2 (was a radius
  stand-in) — knock-down push and the hero's low-target test (kicks and
  low attacks now find small monsters).
- **Lag / memory** (user report): the game doesn't leak within a level
  (flat ~650 MB over 90 s of fighting) and is mostly idle between frames.
  Across level loads memory crept up ~15 MB a load: each load built one
  material per mesh — ~13,600 for a level's monster flipbook frames — and
  the allocator and Metal's resource lists grew with that churn. Character
  models now share one material per texture and draw state
  (`TextureCache::sharing_materials`): 893 materials instead of 13,634 on
  levelA1, ~470 MB instead of ~680 MB, and 28 reloads of the same level
  plateau at 520–575 MB. Check with `GDL_TOUR=3 GDL_TOUR_STEP=0` +
  `footprint -p <pid>`, and `GDL_MEMSTATS=1`.
- **Crash on level change**: the mechanics tick could run one frame with
  the old level's triggers against the new level's nodes (index out of
  bounds, e.g. levelA4 → A5, or tower → a realm). The old `Mechanics` is
  now removed as the level change starts. `GDL_TOUR=4` through 13 levels
  runs clean.
- The user's Mac (16 GB) sits at ~14.7 GB "wired" (kernel/driver) memory
  after 11 days up, with heavy swapping: keep one game instance and one
  build at a time (`cargo build -j 4`).

## Work in progress

- **Helper branch `worktree-agent-abe3330002fa1838d` (`827a9a5`): merged
  into master as `b30b1c3`** (builds; tests and clippy clean). It brought
  the chimera's heads (each head animates its own part
  of the body's skeleton; wounds shared as the game does; a unit test on
  the real data), and docs/critters.md with the heads, the **boss camera
  decode** (`BCAM` records, 0x54 bytes from `LEVL +0x8C`; before the boss
  wakes: the heroes' centre from the nearest play-camera point; after:
  wizard → shard → boss with near/far distance and pitch easing) and a
  smoke test of every boss level (B6 fights and dies; C5, H4 and the rest
  wake; A5 chimera fights and dies; nothing panicked). Boss levels: A5 B6
  C5 D5 E2 F2 G5 H4 I5 J5 K5 (none in the tower). Left from it: build the
  boss camera; head-stump effects; heads' look nodes; longer boss runs.
  - Finding for `player.rs` (mine): the game's attack search measures 3D
    distance to each living hit sphere in the facing cone, so melee only
    reaches a chimera head while it's down biting; missiles and magic hit
    a sphere only within its radius sideways and at most that much above
    the missile. Our search uses its own rules.

- **In-game HUD**: merged (`46a1365`). Player 1's bottom panel draws with
  the game's art (frame, class portrait, keys, potions, gold, health,
  "LV n", runestones, turbo meter). The old text line shows only with F1 or
  `GDL_DEBUG_HUD=1`. Not drawn yet: the turbo flash and glow, the
  legendary-key row, the quest and rune-13 icons, and the "Wait In Tower"
  prompt. See [frontend.md](frontend.md) "In-game HUD".
- **Magic potions**: merged. Tap for a blast, tap twice for the shield,
  hold to throw; the game's effect models are used (the light potion's
  starburst is verified). See [effects.md](effects.md).
- **Test aids**: `GDL_CRYSTALS="<counter>:<n>,…"` seeds crystal counts
  (1 orange … 8 black); `GDL_LOOK_AT` (now with a yaw: `"x,y,z,dist,180"` looks
  from the other side), `GDL_PARTICLE_TEST`, `GDL_POTIONS`, `GDL_THROWER`
  (a monster in front of the hero), and screenshot bursts with `GDL_SHOTS`
  / `GDL_SHOT_EVERY`. Frame what's being tested instead of hunting for it
  (a user rule). A death check on levelA1: `GDL_THROWER=4,7,2
  GDL_BUTTONS=attack GDL_LOOK_AT="0,-1.5,-9,10,180" GDL_SHOT_AT=36
  GDL_SHOTS=60 GDL_SHOT_EVERY=2`; for a fire kill add `GDL_POTIONS=2,1` and
  use `GDL_BUTTONS="magic@30-32"` with the burst from frame 70.
- **World particles**: `particles.rs` runs every level's `PSYS` nodes
  (torch flames, smoke, pool fires, mist, embers) from the decoded records
  and presets ([rendering.md](rendering.md)). The emitter's own ring and
  callback machinery is a stand-in (each particle is simulated directly),
  and trigger-switched emitters always run.

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

0. The tower's wizard scenes: the `WIZARD` model appearing at a lookout to
   announce new shards and runestones, the shard set in `L1WINDOWFRAME`, the
   stones in `L1RUNEPLACE`, his camera cuts and speeches, and his other
   idle actions (WELCOME, GOAWAY…) — decoded in [items.md](items.md) "The
   tower's wizard". (The tower start's camera now frames the wizard whole
   as in the user's screenshot: the play camera looks at the hero's top
   point, 4.4 above the feet, as the game's does — [camera.md](camera.md);
   the HUD's four panels are done.)
   (The smoke test, `smoke.sh`: every level folder plus DEMO1,
   `GDL_BUTTONS=attack GDL_STICK="0.4,1" GDL_SHOT_AT=400`, 120 s timeout,
   stop at the first panic; resume from the failing level.) Next: the
   hero's attack search against
   critter spheres in 3D (above); the boss camera; free-running bank
   texture modifiers on characters/weapons; AI 5/6 (unaware: follow the
   leader, wander) and the leader runner.

1. **Finish bosses** (the critters helper: intro and victory merged;
   chimera parts, all boss levels and the boss camera next). Player side,
   mine: the intro's stand-still and darkening are done; the legendary
   weapon's throw, the highlights and the owner's glow aren't
   ([critters.md](critters.md), "The heroes").
2. ~~In-game HUD~~: done (the remaining HUD details are listed above).
3. ~~Effects system and magic potions~~: done (effects.md)
   (it knows the effect slots at `0x802855ac` and `FUN_80094418`):
   - Potions are used through `FUN_80076618`: mode 0 is the magic blast
     (`FUN_8009262c`, radius 40, damage = magic power `+0x10C`), mode 1 the
     shield, mode ≥ 2 the thrown potion.
   - The blast's damage falls off over time in `FUN_80094418`.
4. ~~World particle systems~~: done (the stand-ins are listed above). What remains is the game's exact emitter maths:
   - What's decoded so far is in [rendering.md](rendering.md) "Particle-system
     nodes": ANIM.PS2 `0x138`-byte records, selected by the letter after
     `PSYS` in the node name.
   - Records are applied by `FUN_800ceeb8`; the update is `FUN_800cdfdc`.
5. ~~Tower progression~~: done for one player, with the gem count above
   the panel and the legendary items' hints. Left: pickup notices, the
   tower's other messages (shards after a boss, runestones: `NEWSHARDS`,
   `ALL12RUNES*`, `RUNE13*`). Saving is done (a file for the card).
6. **Co-op** (up to 4 players), **hints 0x14/0x15/0x1B**, **hit flashes**.
7. ~~Hit effects~~: done — blood sprays (BLOODFX1 per blow, BLOODFX2 on
   kills) run as particle bursts from the effects' kind-4 nodes; FIREHIT,
   HITCOL and the die effects as models. Left: the effects' depth bias,
   the no-blood switch (`r13-0x72cc`), exact particle maths.

### Notes for the next jobs

- **Texture modifiers still to run**: `WEAPONS`' and the heroes' banks'
  (hand glows, once glows are drawn; `CharacterModel::build_animated` does
  it for monsters and critters), action fades (−4/−5) and kind-3 modifier
  nodes (the tower's rune displays, legendary weapon effects), action
  scrolls.
- **Critter hit spheres in the attack search**: ours already uses the 3D
  surface distance and the game's cone (spheres are `TargetKind::Object`).
  Left: the game's per-critter pick (surface ÷ (sphere weight `+0x1C` ×
  margin inside the cone)), each sphere's own max range (`+0x18`), and
  skipping spent spheres (`FUN_80038008`).

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
