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
  - Messages show in the game's message box (`message_box.rs`): the
    `Scroll_A` parchment with the page in dark brown and "Press [B]
    Button when done.", play frozen until B puts each page away — scrolls
    (which never showed their text before: their page is the placement's
    `+0x30`), the tower's gate notices and unlocks, and a new hero's
    five-page welcome in the tower, after which the wizard points the way
    under a camera cut; `GARMMESSAGE` after `levelF2`. The bosses'
    speeches are typed captions in the cut's top bar at the game's speed
    ([frontend.md](frontend.md) "Message box", "Captions"). The pads
    aren't read during camera cuts, as in the game.
  - The tower wizard's scenes (`tower_scenes.rs`): a hero's new rank
    (every ten levels: "Blue Warrior is now a level 10 Fighter!") and the
    first shard or stone won since he last spoke are announced — the glowing `WIZARD` at
    the lookout nearest the heroes under a cut, his words typed in the
    bottom bar with his voice, then the piece's effect in full at its
    place under a cut, and the follow-ups (more shards, all eight with
    the window's light coming on, all twelve stones, the thirteenth).
    Only announced pieces are set out at load (saved with the character)
    ([items.md](items.md) "The tower wizard's scenes").
  - The tower sets out what the heroes have won as it loads (`tower.rs`):
    each shard's pane in the window over the door (a starfield until
    then), each runestone in its slot on the rune place, the thirteenth
    on its own slab below — the game's `SHARD<n>`/`RUNE<n>` effects wound
    on to their last frames. Open-for-good gates are open on arrival
    (the game fires them as the tower loads; ours stayed shut until
    touched) ([items.md](items.md) "Quest items and the tower's gates").
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
  - Effects, monsters and critters run their actions' modifiers and their
    kind-3 modifier nodes (flipbooks and fades, per node and subtree, by
    the action's frame; `texanim::ModelMods`): the acid blast's gas and
    rings, the fire and light blasts' fade-outs, the acid blob's body per
    action. Flipbook nodes show their runs only from their start frame to
    their end, as the game does: effects' streaks and starbursts used to
    hang on their last frame ([animation-format.md](animation-format.md)
    "Flipbook nodes").
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
  - Hit flashes: a monster that lives through a blow, a hero hit for more
    than a point, and a critter or part struck by the kinds that reach
    past its hit spheres (finishers, potions) glow in `AAAWHITE` for two
    ticks; a struck obstacle shows it for one (`flash.rs`, carried on
    `MeshTag`). The death textures now use the decoded second stage
    (lighting × frame × 2, the body's texture dropped)
    ([rendering.md](rendering.md) "Texture overrides"). Checked on
    screen: a grunt, the B6 dragon (fire potion), a levelA1 hedge wall.
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

- `GDL_WARP="x,y,z"`: start the hero at a point, on the floor below it
  that the player's floor check stands on (with none, a warning and the
  level's own start: a point over a gap would drop the hero out of the
  level and back to the point for good); the first level only. The `floors` example lists every
  floor down a line (`cargo run -p gdl-formats --example floors --
  <level folder> x z`).
- `GDL_LIST_NEAR="x,y,z"`: log the level objects near a point.
- `GDL_FPS=1`: frame rate, plus a line for every frame over 20 ms.
- `GDL_WAKE_STATUES`, `GDL_CRITTER_HP`: critter testing.
- `GDL_MENU`, `GDL_BUTTONS`, `GDL_STICK`, `GDL_SHOT_AT` + `GDL_SCREENSHOT`:
  scripted input and screenshots (`GDL_MENU="b@300,b@340"` turns message
  box pages). `GDL_SHOT_CLOCK=ticks` counts shots in
  game ticks, so a burst lands on the same moment of play every run (frame
  counts drift with shader warm-up).
- `GDL_BEATEN=<realm bits>` and `GDL_RUNES=<stone bits>` (decimal or
  `0x…`): the realms beaten and runestones held, set once at the first
  level (`0xE9E` = the eight main bosses, `0x1FFF` = all thirteen stones);
  `GDL_CRYSTALS="1:-1"` opens a counter for good; `GDL_EXPERIENCE=<n>`
  gives the hero experience with its rank last checked at its old level
  (12000 → level 10: the tower announces the new rank).
- `GDL_LOOK_AT="x,y,z,dist[,yaw]"` pins the camera on what's tested (a
  user rule: frame it rather than hunt for it); `GDL_SHOTS` /
  `GDL_SHOT_EVERY` take a burst of screenshots; `GDL_THROWER=<distance>[,
  <ai>[,<tier>]]` puts a grunt in front of the hero; `GDL_POTIONS=<n>[,
  <kind>]` hands it potions; `GDL_POWERS="<subtype>:<value>[:<amount>[:
  <seconds>]],…"` powerups; `GDL_PARTICLE_TEST`. A death check on levelA1:
  `GDL_THROWER=4,7,2 GDL_BUTTONS=attack GDL_LOOK_AT="0,-1.5,-9,10,180"
  GDL_SHOT_AT=36 GDL_SHOTS=60 GDL_SHOT_EVERY=2` (a fire kill: add
  `GDL_POWERS=5:1`). Screenshots slow the frames: a two-tick flash can
  fall between two of them.
- Gate every game run on `pgrep -x gdl-game` (one instance at a time; the
  helper runs games too) and give it `GDL_MUTE=1`.

Handy test spots are listed in [mechanics.md](mechanics.md) and
[camera.md](camera.md) (levelA1 elevator switch, levelA4 lift, barrels,
the levelA2 Death barrel).

## Latest check

The all-levels smoke test on `70b2f0a` (Death and the halo, the familiars
and the phoenix, the Pojo's throws, Skorne's gauntlets and the super
crossbow with every release's throw sound, and the helper's level-up
flash and level-up routine) passed all 67 real levels; only the known
warnings (the test levels' missing `dream1a_1.ads`, no lookout 0 on L2/L3).

Before that, the all-levels smoke test on `5a1b996` (the armour and special power-ups:
the resistance routine everywhere, chrome, shields, invisibility, shrink,
grow, time stop, breaths, the hammer; the helper's blasts on items,
CHESTEXP, the power-up looks, x-ray, the lift and the hourglass) passed
all 67 real levels.

Before that, the all-levels smoke test on `0b8a828` (hit flashes, hints, critters
found and hit once, floor potions set off, power-ups stages A–B, and the
helper's HUD key row, "IN TOWER" and voice queues) passed all 67 real
levels.

Before that, the all-levels smoke test on the tower wizard's scenes (the commit after
`6ca8c15`) passed all 67 real levels; so did `6ca8c15` (the message box,
captions, the welcome, input blocked in cuts; the tower's check has the
welcome box up).

Before that, the all-levels smoke test on the texture-modifier nodes, flipbook runs and
the tower's shards, runestones and gates at load (`2faf43c`) passed all
67 real levels; so did `c8fb3f9` (the turbo meter,
the boss camera's stick) before it.

Before that, the all-levels smoke test on master `f83e379` (item, monster and boss
texture animations, safe rocks, the shards' light, the camera's top-point
target, unaware monsters and running from a charging runner) passed all
67 real levels; so did `5b4ad91` and `2bb0c8c` before it.

Before that, the smoke test on master `c2424af` (the mirrored view, gamma
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

- **Helper** (worktree branch `worktree-agent-aba24fd14c15b99a9`): its
  work so far is merged — this session the level-up flash (additive, at
  the effect table's depth bias −512) and the game's level-up
  (`levelup.rs`: hint `0x22` and the flash on a rise, the lost level's
  sentence on a fall), before that the familiars and the phoenix's
  shots, the Pojo's throws, the blasts on items, CHESTEXP, the power-up
  looks, x-ray, the lift, the hourglass, `Animator::hold` and the pickup
  hints (checked on `levelT2`, a test level with every power-up laid
  out: `GDL_WARP="26,0,50"` stands on the Pojo egg). Check its branch
  for new commits before merging anything else.
- **Power-ups** (mine; `GDL_POWERS` grants them, e.g. `6:0x10000`,
  `9:0x10:5:-1` for five fire breaths, `5:0x100000:5:-1` the crossbow,
  `9:0x8000` the left gauntlet — [items.md](items.md) "Timed powerups"):
  - Stage C done: every blow goes through the resistance routine
    (`damage::resist`; heroes via `Player::take_blow`, monsters, critters;
    elemental blows × 1.5 on monsters) — invulnerability with its chrome
    (`fade::BodyLook`, `level.wgsl` mode 3), gold's heal, the gas mask;
    power clocks held in cuts and × 3 in a boss fight; the reflect, fire
    wall and lightning shields with SHIELD_READY/RUN; blasts and monster
    missiles hit heroes with their kind and push.
  - Stage D done: invisibility, levitation's dodge (and the helper's
    lift), shrink (`EnemyScale`), grow, time stop (`TimeStop`), the
    breaths (ATTBREATHE, a cone blast riding the head, uses spent via
    `SpendPower`), the hammer's chop (ATTCHOP).
  - Also done: Hand of Death and Health Vampire, the Pojo's turbo breath
    and knockdown, the charge's blow (bosses aren't walked into or
    charged), Death (type 30: the drain, blows taking 1, magic killing
    it, leaving) and the halo's drain, the familiars and the phoenix,
    **Skorne's gauntlets and the super crossbow** (ATTFIREL/ATTFIRELR
    and SSHOT1/2/R with the chooser's loop flag `Next::again`; the
    `BOSSG_ELEC`/`BOSSG_ACID`/`SUPERARROW` shots; every release's throw
    sound — projectiles.md "Hero release"); the stuns (STUN1 on the damage
    tiles, STUN2 while a poison or drain stun holds a standing hero —
    combat.md "The stuns").
- **Placed critters** (mine): the **generals** (made on sight on most
  levels) and **gargoyles** run with the golem's update; statues wake by
  trigger or by being walked into (and block); placed golems, gargoyles and
  generals hold the powerup on their spot (keys, mostly) and drop it where
  they die, a gargoyle holding none its `GARG<kind>` piece, with hints
  `0x86`/`0x8A` (critters.md "Placed critters", "In this rewrite"). Left: a
  missile or blast waking a statue; the drop's toss.
- **The boss's loot** (mine, `loot.rs`): each boss's DEATH throws its
  realm's coins (the first Skorne his four pieces — where the heroes get
  them) that bounce to rest, and an unseen sweep breaks the level's
  items (critters.md "The boss's loot").
- **Safe rocks** take blows (every missile but magic stops on a standing
  rock; blasts too): hit points, armour, restaging with the stage models
  and GENDEST (critters.md "Safe rocks"); the P-boss's spouts at every
  rock (kind 5) and the yeti's boulders (kind 6, remaking a rock) as
  growing blasts over their effects' clips (checked on K5 and I5). Grabs
  (kind 7) are decoded, not ported.
- **Pickup notices** (the helper's `pickup_notices.rs`, merged): each
  pickup's plate rises over the panel (checked: a key's KEY_RING plate on
  A1); a new runestone has the announcer count the stones (unit-tested;
  not seen on screen — a pickup in a level's first second counts as
  already held).
  - Next: rapid fire's rate (items.md "Attack overrides": how the 0.75
    reaches the clip isn't pinned down); the hero missiles' streaks
    (`WEP_STREAK`, the crossbow's white one) and the magic classes'
    element models. Footsteps are in (the helper's `footsteps.rs`: a step
    as each stepping action ends, by the floor — player-movement.md);
    not their pan, nor the shadow on water.
- Also fixed on the way: effects whose clip has no frames lasted 0 s
  (particle effects: the breaths, blood sprays, `L_SHLD_ACTIVE`) — they
  last 30 frames, as the game's spawner has it.
- **Older notes** kept for reference:
  - The boss fights: B6 dragon verified; the chimera's heads animate
    their own subtrees; the boss camera and intro are in. Not done: the
    `BOSSKEY` effect and key pick-up (a 5 s stand-in returns to the
    tower), head-stump effects, the heads' look nodes.
  - Warm-up hitches: early frames of a level can take 20–40 ms; a
    pre-warm skips frustum culling for 3 frames. Not verified yet
    (`GDL_FPS=1` with nothing else building).

## Next jobs, in order

Run one helper agent at a time; each job ends in a report, then the agent
waits.

0. Power-ups, what's left: the head-2 sparkle, rapid fire's rate, the
   missile streaks (above). The heroes' step cut under the boss
   camera; the hand glows (and their banks' running modifiers); the
   node spheres' flashes (which model node a `NODE` names, `+0x500`).
   (The smoke test, `smoke.sh`: every level folder plus DEMO1,
   `GDL_BUTTONS=attack GDL_STICK="0.4,1" GDL_SHOT_AT=400`, 120 s timeout,
   stop at the first panic; resume from the failing level.)

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
   the panel, the legendary items' hints, the wizard's messages
   (NEWSHARDS … RUNE13*) and the pickup notices. Left: the secret realm's
   coins and ALLCOINS (items.md). Saving is done (a file for the card).
6. **Co-op** (up to 4 players), **hints 0x14/0x15/0x1B**. (Hit flashes
   are done but for node spheres', whose model node isn't traced.)
7. ~~Hit effects~~: done — blood sprays (BLOODFX1 per blow, BLOODFX2 on
   kills) run as particle bursts from the effects' kind-4 nodes; FIREHIT,
   HITCOL and the die effects as models. Left: the effects' depth bias,
   the no-blood switch (`r13-0x72cc`), exact particle maths.

### Notes for the next jobs

- **Texture modifiers still to run**: `WEAPONS`' and the heroes' banks'
  free-running ones (hand glows, once glows are drawn;
  `CharacterModel::build_animated` does it for monsters and critters), the
  heroes' own actions' (their skeleton and clips come from different
  files), and action scrolls — the texture wipes, decoded in
  [rendering.md](rendering.md) "Actions" (the combos', bosses' attack
  effects); the draw's texture-shift maths (`FUN_800c68e4`, `r2-0x4800`)
  isn't pinned down.
- **Critter hit spheres**: done — the search's per-critter pick, each
  sphere's reach, spent spheres skipped, the body's fallback at its
  centre, and one hit per critter for missiles, bombs and potion blasts
  (a blast used to hit a boss once per sphere it reached)
  ([critters.md](critters.md) "Found and hit").

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
- Disk: `target/` grows large. Split debug info leaves every build's
  object files for our crates in `target/debug/deps` (37 GB after a day
  of edits, which filled the disk on 2026-09-30). Free it with
  `cargo clean -p gdl-game -p gdl-formats -p gdl-install` (only our crates
  rebuild; deleting the files by hand breaks cargo's fingerprints). Agent
  worktrees have their own `target/` dirs; delete those after merging.
