# Handoff: where the rewrite stands and how to continue

Last updated 2026-10-01 (v0.2.0). Read this first when resuming;
[STATUS.md](STATUS.md) has how close the rewrite is to the original
(≈ 84 % for one player, ≈ 83 % done overall) and everything left to do,
in order.


## 2026-10-03: online sync, v0.2.3

Done and released as v0.2.3 (`docs/online.md`: "What a tick may read",
"The same maths everywhere", "Sync points", "The lag sign"):

- **The out-of-sync restarts' cause**: the ticks read where things were
  *drawn* (a hero's aim found monsters by their transforms, which are
  smoothed between ticks by each machine's own frame timing). Fixed in
  `tick_places.rs`; `FIGHT=1 tools/online_test.sh` reproduces the old
  failure (out of sync at the first throw) and passes now, with
  `CLIENT_PREFIX="taskpolicy -b"` slowing one side too.
- **Sync points** (`resync.rs`): no level restart; each hero from its own
  machine. `CLIENT_DESYNC_AT=400:<what>` tests each kind.
- **The lag sign** (`lag_sign.rs`, `GDL_LAG_SIGN=1` to see it).
- Co-op quest pickups go to every hero in play (`items.rs`, `docs/items.md`).
- The build's `test` job runs the tests on Windows, macOS and Linux:
  `every_system_computes_the_same_bits` (`detmath.rs`) and
  `every_processor_computes_the_same_bits` (`rotations.rs`) hold sums
  taken on an ARM Mac. **Check that job after each push**: a failure on
  Intel there is a real cross-machine difference to fix.

Since v0.2.3 (not released): online Quit Level (the host's, to the tower
for everyone), leaving or a game ending under you plays on alone in the
tower with the level-start record, first person shows every player's
panel. Checked: pausing on both machines through a fight stays in sync;
the v0.2.3 build's `test` job passed on Windows, Linux and macOS (the
"same bits" tests too). Monsters are stopped by doors, chests, barrels
and generators as the game's mover stops them (`docs/monsters.md`,
"Items in the way") — they used to walk through shut doors.

Asked for and **not done yet** (next, in this order):

1. Other classes' throw and attack animations (check each class's action
   table against the original's).
2. Sync points don't carry yet: broken barrels and walls, chest contents
   let out in another order, `mechanics.rs`, a critter's move under way,
   camera cuts and the boss camera.

## Pick up here

**2026-10-02 session: the user's bug list (do these first).** Done and
checked in game (muted, scratch runs): first-person throws follow the gaze
incl. pitch (A1 secret wall 5 broken from 9 units, chest revealed); hands
kept out of the lens/middle (attack frame grid); secret walls break by
throw and by melee; Manage Character keeps the heroes' places (tower
(3.2,−15) kept — **a deliberate change from the original, the user chose
it**, see frontend.md); G3's rising barrel breaks (`breakables::follow`:
targets follow riding items); G4's hit switch is auto-aimed and hit (the
search now starts at the hero's collision centre, feet + 2.5, as
`FUN_800864b0`'s callers pass `+0x64`); generators leave their `GEN_<code>0`
wreck (`FUN_8005ba08`). Online host-classic / client-first-person: 48
matching checks. Changed but **not run-checked**: levitation raises the
eye; after a cut / the opening the view returns straight to first person
(`PlayCamera::showing_shot`); hint boxes in each first-person pane
(`game_hud::PanelPlace`); G2's plank barrels (same `follow` path as G3,
the plank moves by its animation matrix, node 1098; not triggered in a
run). **Not reproduced:** "scroll message boxes sometimes don't appear in
first person" — in A1 the box shows; all 133 placed scrolls have their
text page (only levelL3/T2 lack groups); ask the user for the level/spot.
**Cheat codes: done (2026-10-02).** `cheats.rs`, docs/cheats.md: 26
secret characters (models in the base classes' folders, variant = colour
+ model, e.g. `GREDCY`), 16 gameplay codes (permanent held powers via the
power menu, ALLFUL / 10000K counts), the developers' MNTHRX / ARIENT /
ADMBLY; re-granted every level start; online alike (`GDL_ONLINE_HERO=
<class>:<name>`); `--name` for skip mode. `grant_power` now takes the
game's exact slot pass. Not ported: the developers' button-combo debug
flags.

**User's open reports (do next):** (1) on Windows the audio while
loading into a level is "bugged out and staticy" — suspects: the movie's
8-bit PCM at ~55 kHz resampled by rodio, the level's sounds starting
behind the loading screen, the map bank's narration lines (check with
`audio_dump` roughness); (2) other classes' throwing (and other) attacks
play the wrong animations — check each class's action table against the
original.

**Loading screens and level movies: done (2026-10-02), offline and
online.** `level_intro.rs` (screen, narration, dash sounds, movie),
`gdl_formats::movie` (MVDV decoder, checked against FFmpeg), `LEVL +0x04`
/ `+0x34` / `+0x5C` → `MAPS`, `AUDS +0x10` → `SNDS` in `world_data.rs`;
all in docs/frontend.md, "Loading screens and movies". Checked in game:
the tower's A1 portal → the Castle map → the courtyard picture with
"Loading..." (6.55 s, the game's 390 fields) → movieA1 (5.1 s) → play with
the opening shot; any button skips the movie; E1 → E2 draws the temple
path's 6 dashes; first person shows nothing over it. Online (scratch
`online_intro.zsh`: two games over loopback, `GDL_HOPS` onto the portal —
hops now count lockstep ticks online): both machines go through it on the
same tick and stay in sync (70 checks); the host's press skips the movie
for both. `GDL_SKIP_INTRO=1` skips it (smoke.sh sets it). Intros only
come with `ChangeLevelTo::finishing` (not a game starting on a level).
Left: the sound bank step, the ending movies (`victory`, `garm`).

**Online leaving (2026-10-02).** Leaving or quitting never holds the host
(measured); a silent machine is dropped after 4 s (`SILENT_LEAVES`, was
10); Leave Game before the tower warns that the level's progress for that
character is lost (`LEAVE_LEVEL`, `Menu::with_note`). The disc header's
maker/disc/revision bytes were read from the wrong offsets (fixed: 4–5,
6, 7); the install now has `revision` and warns on anything but Rev 0,
and the online build string carries it. The user's disc is the original
USA release, Rev 0 (Redump #7032; README "Which copy of the game you
need").

Disk: `target/` had grown to
35 GB and filled the disk; keep one build (no helper worktree builds —
the user doesn't want repeated full rebuilds).

**Next jobs, in order** (the user's original queue). The online work and
boss implementation are merged; the boss gameplay checks below remain.
Complete the personal-view follow-up checks in this handoff first:

1. **Co-op combos** — step 1 tracing and the step 2 parser are done.
   **Pick up with the runtime hero record attacks**, then pairing/carrying.
   Decoded: [coop.md](coop.md) "Co-op combos" (partner test,
   COMBOACT1/2/3, the partner's per-class actions, carrying, the throws,
   the state machine's cases, the hit moment and its effects) and
   [chunk-files.md](chunk-files.md) "The heroes' attack records" (the
   class's `DAMG` records the combo, power and turbo attacks hit with;
   `tools/hdamg.py WAR` dumps them). Steps:
   1. **Done:** traced `FUN_80089114` (missile / steady area / growing
      blast slots), `FUN_80030094` (kind 10's repeated ordinary hero
      missiles), and carrying (`%sDUMMY`, model-root attachment and the
      two detach modes). See the updated chunk/coop notes. Decompile: `~/ghidra-projects/exports/
      GauntletDarkLegacy-main.dol.c`; constants: `tools/mydol.py`.
   2. **Parser done:** `HeroAttacks` in `gdl-formats/src/pdata.rs`, the
      twelve `PDAT +0x0C..+0x22` indices and all 16 PDATA files checked.
      Hero SFXX position starts +0x34; +0x4C is packed colour, not scale
      or a node selector. **Next: port `FUN_80088b88`** as the
      heroes' record blows — which also replaces the turbo attacks'
      finisher stand-in (ATTPWRB/C) and gives the power attacks their
      records.
   3. The combo itself: the classifier's partner test (`combat.rs`, its
      comment marks the spot; the partner needs other heroes' state, so
      compute it in `player.rs` and pass it in), the requests (`0x16` →
      COMBOACT1 with the 50 cost; the partner's intents `0x26`/`0x27`/`0x17`
      from its `+0x964` flags), `actions.rs`'s cases for `0x58`–`0x5A`,
      `0x89`, `0x8F` (looping) and `0x88`–`0x93`, the linking and turbo
      payment, carrying per class, the effects (COMBO_SPH and
      COMBO_<colour> by `effects::EffectOn`), the thrown hero's hits.
   4. Test with two local players (`GDL_FAKE_PAD`, [coop.md](coop.md)
      "Testing"; `GDL_BUTTONS=combo@…` presses Z) and online
      (`tools/online_test.sh`, both sides holding combo near each other).
2. **The monsters' crowd penalty** (`FUN_80051660`: a hero's score is its
   distance + its record's `+0xA28` past `r2-0x6eb0` × a monster value;
   find what keeps `+0xA28`). The helper started this and was stopped by
   the usage limit before changing anything.
3. **Settings**: Game Options (Difficulty, Multiplayer Mode are stand-ins),
   the memory card screens, the attract mode ([frontend.md](frontend.md)).
4. **Check the bosses' new work in game** (merged unverified but for the
   meters): the lich's aura (levelG5, `GDL_IMMORTAL=1`, stand near the lich:
   "…'s NULLFX slot: 10 to the first hero within 16…"), the 3D `GMETER`
   (golems on levelC2/levelJ1, the A1 gargoyle with `GDL_WAKE_STATUES=40`),
   the chimera's three fills (levelA5), the drider's whip cones (levelD5).
5. Then the older queue below: the G–T tour's leads, the AIs on the chase
   stand-in, the level-by-level audit (top priority overall).

Decoding tools: `tools/mydol.py` reads `main.dol` constants by address or
`r2`/`r13` offset (`f32:r2-5b44`, `u32:80122628`); `tools/hdamg.py <class>`
dumps a hero class's attack records; the decompile is
`~/ghidra-projects/exports/GauntletDarkLegacy-main.dol.c`. Run every game
through `tools/waitrun.sh` (muted, one at a time).

## State of `master`

**2026-10-01 (personal views and idle/audio fixes), v0.2.0:**
- User added: idle instead of walking in place, a quieter mix, and a
  per-player first-person setting for solo/local/online play.
  `first_person.rs` implements eye-level mouse/right-stick look, actual
  equipped weapon/hand meshes posed by the existing Animator, an
  independent transparent weapon render so walls do not cut through it,
  a centre marker, and individual local panes with fitted status panels.
  Online uses each machine's own settings; first-person/mouse-look bits
  travel in the existing input buttons so movement/aim stays deterministic.
  The state checksum includes the look state. The host's overhead choice
  does not override another user's first-person preference.
- Forward walk/run still uses the existing speeds, collision and action
  factors; side/back movement uses existing strafe actions. Throws aim
  toward the gaze with a small target assist if Auto Aim is on; damage,
  release timing, projectile speed and power costs stay with normal combat.
  Camera cuts/opening shots keep their original views. Mouse capture is
  released for menus and when the window loses focus.
- Walk/run return to READY immediately on release. A 15% radial pad dead
  zone prevents drift from selecting WALK. READY breathes; after 20 s a
  longer retail idle plays. The mix has 6 dB of headroom applied once to
  new/live sounds, preserves the saved sliders, and rejects nonfinite
  saved volume values. Positional audio follows the first-person view.
- Hero DAMG/SFXX parsing and the resumed co-op decode are saved. **No
  runtime co-op combo or hero record blows yet**; the remaining queue
  below remains in the same order.
- Final build, `cargo test -j4 --workspace` (**390 passed, 1 ignored**),
  strict workspace clippy and `git diff --check` pass. The only build notice
  is the existing upstream `block v0.1.6` future-incompatibility warning.
- Actual muted runtime: mixed local WAR first-person / VAL classic panes
  with separate HUDs; Settings → Controls toggled On and Off and saved each
  choice; a neutral tower run logs READY → IDLE2 → IDLE2_LOOP.
- Online host first-person / client classic passed **34 matching checks**
  through tick 990, including scripted right-stick look. **This run predates
  the final first-person contact exemption in `items.rs`; rerun online on
  the checkpoint before calling that change verified online.** Revision 2
  rejects older builds. Host/client preference directories are already
  prepared for the reverse case: host classic with shared cameras, client
  first-person. That reverse case has not been run.
- First-person mechanics run on the final binary: KEYRING gives 2 keys;
  GATEDS consumes one; TREAS_SILVER adds 100 gold; transporter reaches its
  midpoint/arrival hint; trigger 393 activates and mover 1774 arrives at
  −4; the normal A1 exit logs `levelA1 finished`, then the AfterLevel/Shop
  screen opens in the tower. Scripted hops were used, **not a manual full
  level or campaign clear**. The run ends via its 60 s timeout after Shop
  pauses the fixed clock; no panic. Final first-person touches bypass only
  the rendered-view activation gate, retaining all contact, key, quest,
  capacity, collision and exit requirements. Transporter destinations need
  not be in view for a first-person hero. Classic rules stay unchanged.
- Full technical notes: [first-person.md](first-person.md). Tests verify
  Off restores classic facing and retains original combat buttons and
  Robotron sticks. Mouse pixels accumulate until a fixed tick / online
  input commit consumes them; fast turns retain their remainder. Eye
  height tracks growth. New/live audio has 6 dB of headroom; all test runs
  were muted, so **no subjective volume/listening comparison** yet.

### Immediate follow-up on the user's added requests

The user asked to wrap up because usage was low. All code and notes are
saved in this checkpoint. Finish these checks before returning to the
numbered co-op queue above:

1. **Done 2026-10-02 (48 matching checks, both views shot).** Rerun online after the final item contact change, with the host in
   classic/shared view and the client in first-person. The existing
   `tools/online_test.sh` accepts `HOST_ARTIFACTS` / `CLIENT_ARTIFACTS` for
   separate settings. Set `RUST_LOG=info` for checksum output. It already
   calls `waitrun.sh`; invoke the helper directly. The harness now refuses
   zero overlapping hash checks. The prepared directories and logs are
   under `/Users/dmonskull/Documents/Codex/2026-10-01/con/work/`:
   `net-host`, `net-client`, `net-proof/online`, `online-result.log`.
   Example (muted via the helper):
   ```sh
   env HOST_ARTIFACTS=/Users/dmonskull/Documents/Codex/2026-10-01/con/work/net-host \
     CLIENT_ARTIFACTS=/Users/dmonskull/Documents/Codex/2026-10-01/con/work/net-client \
     GDL_TEST_OUT=/Users/dmonskull/Documents/Codex/2026-10-01/con/work/net-reverse \
     GDL_ONLINE_LEVEL=levelA1 RUST_LOG=info GDL_SHOTS=2 GDL_SHOT_EVERY=999999 \
     SHOT_AT=1800 tools/online_test.sh 55
   ```
   Multiple shots keep the first screenshot from exiting the host before
   the client's screenshot. Verify nonzero matching checks and both views.
2. **More equipment visual checks**: WAR hands/axe and two-player mixed
   panes were checked in live first-person. The early WIZ screenshot was
   during a scripted overhead sequence, so it does not prove staff framing.
   Check WIZ, ARC and the other class/power weapons after the opening cut.
   Use `GDL_STICK=0,0` to prevent live keyboard/pad activity moving the hero
   during a neutral check. A tower shot at 240 ticks can still show a
   scripted camera; use later ticks. Look for clipped equipment, attacking,
   defending and throwing. Three/four pane rectangle geometry has tests,
   but no three/four-player visual run yet. World billboards/particles use
   a single world-view orientation; per-pane facing is still a visual lead.
3. **Progression breadth**: real key/door/gold/transporter/switch/exit paths
   pass; test potion use, powers menu/equipment swaps and boss completion
   in first-person. A complete campaign remains unverified, and the
   underlying rewrite's unfinished items in STATUS.md still apply. The
   user explicitly wants first-person to be usable through the game.
4. **Listening/feel**: the user should try mouse/pad sensitivity, narrow
   panes and sound volume. Keep automated tests muted. Do not claim a
   listening comparison or a complete campaign clear from these checks.

Proof logs: `work/mechanics-final.log`, `pickup.log`, `idle-neutral.log`,
`toggle-on.log`, `toggle-off.log`, `local-final.log`, `tests.log`,
`clippy.log`, `build.log` in that same chat directory. The mechanics run
used `GDL_HOPS` at 90 ticks with these A1 points (all real placements):
`-22.38,-2.53,-4.25;39.06,-7.02,25.84;104.59,20.25,-20.79;`
`-70.84,0.12,94.78;47.5,0.14,67.5;-20.62,20.38,113.72`,
`GDL_STICK=0,0.15 GDL_IMMORTAL=1 GDL_SKIP_BOXES=1`.
Screenshots in `/Users/dmonskull/Documents/Codex/2026-10-01/con/outputs/`:
`first-person-single.png`, `first-person-local-coop.png`,
`first-person-online-host.png`, `first-person-pickup.png`. Some other
screenshots there show scripted cameras/message boxes/Game Over and are
not first-person visual proof. No game assets are committed.

**2026-10-01 (latest), v0.2.0:**
- **Online, finished** ([online.md](online.md) "Starting again"): joining a
  game under way (the joiner picks a hero and waits; the host restarts
  everyone at the tower with them), Manage Character online (a changed or
  loaded hero restarts everyone in the tower; Save writes to that
  machine), and out-of-sync recovery (every machine says so; the host
  restarts the level under way from its records). `gdl-net` protocol 2: a
  run number (`epoch`) on every input, bundle and checksum, late joiners'
  slots kept until `restart()`. The online menus' **Invite** copies the code
  again; the tower's online menu has seven items (smaller). A message box
  hides an open online menu. Our own `FONT32` lines map the `: ; - "` the
  font lacks; invites written with a space for the dash still join.
  **The fixed tick and `NetTick` run single-threaded** (`online.rs`): every
  machine runs a tick's systems in the same order.
  Checked: `tools/online_test.sh` plain (51–53 checks), `HOST_PLAYERS=1`
  (late join, 71), `CLIENT_MENU=…` (Manage Character VAL → WIZ, 86) and
  `CLIENT_DESYNC_AT=600` (recovers, 48–51 after the restart), all in sync.
- **Bosses** (helper, merged `fd427c5`): the blows' effect slots (the lich's
  axe and chain spheres = its aura; held attacks' steady areas; area
  cones), the boss key (its pick-up is dead code in retail; the realm bit
  does everything), the 2D health meters and the 3D `GMETER`. Heroes take
  no harm once a boss's end sequence starts (`player_state::take_damage`).
  Checked: the dragon's (B6) and the lich's (G5) meters show.
- Boot checks: title → Local Game → select screen; `--level` runs of
  levelL1, levelA1, levelG5, levelB6 without a panic; all tests and clippy
  clean.
- The older entries' "parked, unmerged" helper branches (secret realm
  `worktree-agent-a7499ea16152eadcd`, shop `worktree-agent-aba24fd14c15b99a9`)
  are merged since. No helper is running; `worktree-agent-a6d001c5d652b400d`
  holds nothing new.

**2026-10-01 (later), playable release work:** the shop, inventory and
after-level screen (tally with kills, level-up, shop, stats, inventory,
final stats after H4; online on the ticks for everyone); points bought,
kills, generators, gold found and play time kept in the hero's record
(saved); monsters and critters ride moving floors (K2's battle boat); the
bursting objects' blasts; the critters' grabs (checked on I5); hint 0xB at
exits in co-op and the hints sent to the right players; the HUD panels
under the shop screen. Merged the helper's audit round 4 (level ids
through the realm records: the tower's a2 portal leads to levelA6;
trigger ids and camera points; the tower's gates open as it loads).
**Parked, unmerged:** the helper's secret realm timer and coins
(`worktree-agent-a7499ea16152eadcd`, WIP 048cd59, unverified: check S1's
coins with `GDL_HOPS` before merging — the S levels can't be finished
without it). **Next:** the G–T tour's leads (G2, G3, G4, H3, J3, K3; the
"NOT ARRIVED" animated lifts are likely tour.py treating animated targets
with heights as movers), the boss health meter (`GMETER`, the 2D
`<name>METER_BG/FG` meters), the monsters' crowd penalty
(`FUN_80051660`: + the hero's `+0xA28` past a distance) and the AIs still on
the chase stand-in, co-op combos.


**2026-10-01, online co-op in the game** (`online.rs`, `docs/online.md` "In
the game"): Title → Start → Local / Online; Host (invite to the clipboard) /
Join (from the clipboard); the select screen is the lobby (New, or Load a
hero saved on that machine); lockstep over `gdl-net` with one tick a frame,
level changes settled first, per-hero cameras and spectating, a state hash
every second. `tools/online_test.sh` (two games over loopback) stays in sync
in the tower and in levelA1. Not online yet: joining after the start, Manage
Character, the shops. The helper's shop decoding is parked unmerged on
`worktree-agent-aba24fd14c15b99a9` (WIP commit 5411df7; `shop.rs` still
names retail addresses). Packaging: `PLAYING.md`, `tools/package.sh`,
`.github/workflows/build.yml` (Windows / macOS / Linux zips).


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
- `GDL_HOPS="x,y,z;x,y,z"` moves the hero onto each point in turn every
  `GDL_HOP_TICKS` ticks (default 120), on the first level;
  `GDL_SKIP_BOXES=1` puts message boxes away as soon as B could;
  `GDL_IMMORTAL=1` keeps the hero at its last hit point. `tools/tour.sh`
  uses all three.
- Run every game through `tools/waitrun.sh` (one instance at a time, muted:
  `GDL_MUTE=1`).

Handy test spots are listed in [mechanics.md](mechanics.md) and
[camera.md](camera.md) (levelA1 elevator switch, levelA4 lift, barrels,
the levelA2 Death barrel).

## Latest check

**Session end 2026-10-01, `master` at `e231026`** (build, clippy, tests
pass; keys checked turning in game). Done this session: the A1 lift
regression (a teleport swept the touch test across secret walls); the
crystal sparkle that never ended (effect emitters stop with their
effect); keys turning (the game's rule: an item's first action loops
until its action changes); developer keys off unless `GDL_DEV_KEYS=1`
(`I` swapped every item for a debug marker — likely the user's "stray
objects": **ask the user to confirm**); hints in the game's own box
(parchment, font, ink, timing, cool-down); the helper's audit rounds 2–3
(items drop onto the movers' start heights and ride them, touches from
the hero's centre, bursting animated objects, levels B1–I5 audited).
The trigger tour on the merged build passed A1–F1 but for leads the
helper explained (hit switches, chained or ordered triggers, D2 307 and
C1 433 unreachable in the original too); G1–T3 weren't toured.

**Next, in order:** 1) tour G1–T3 (`tools/tour.sh`), then the all-levels
smoke test (`tools/smoke.sh`) and a look over its screenshots (32
character bones changed with the track-flag fix); 2) the bursts' blast in
`effects.rs` (damage 50, radius 6 in realms I/K, else 5; kind 0x800 / 0x21;
no owner) — `mechanics.rs` needs one line to send it; 3) the frame rate
with the animated objects (`GDL_FPS=1`, logs need
`RUST_LOG=bevy_diagnostic=info`); 4) F2's arena (only its north half has
floor collision; 21 rock falls hang under the floor) — look in play;
5) the audit's last realms: A5, A6, J, K, L, S, T, DEMO1 (J4 335/366/422/
429 and L1/L3 pads flagged); 6) then [STATUS.md](STATUS.md)'s list.
The helper's worktree branch is fully merged; give it the next round.

Earlier:

2026-10-01, on `ef1f596` (the level audit's first round merged: the
world's animated objects — A2's drawbridges, the plank, the diving board,
B2's snakes, B3's rock groups —, secret walls blocking until broken,
falling obstacles, key rings, every trigger's target held at its off
height): build, clippy and tests pass. In game, A2's first drawbridge
starts raised and swings down when the hero stands on lever 379; A1's
exit leads to A6 again (exits are shut only in the tower, `9d385c9`).

The trigger tour (`tools/tour.sh`) on the merged build, A1–C1: every
one-player trigger switches on and its node arrives except **A1 402/403
(A1ELEV1/ELEV2: a regression from the merge — they fired on `6cffb3a`)**,
A4 267/268, B1 96, B5 275/278/280, C1 432/433 (STATUS.md "Now"). The
tour before the merge also flagged C2 43, C3 415, C4 556/575/669, D2
307/406, D3 46/48/51/324/363 and D4 338/342/355; C2–T3 haven't been
toured on the merged build, and the smoke test hasn't run since the merge.

Earlier:

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

- **Helper** (worktree branch `worktree-agent-aba24fd14c15b99a9`): the
  level audit's second round, B1 onward (static: what each level makes,
  skips and hides, every trigger link, the tour's failures above); its
  first round is merged (`ef1f596`). Before that, the level-up flash (additive, at
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
- **Critter effect sounds**: every effect a critter starts (move sounds,
  a blow's effect on its first frame, a sphere's or cone's hit effect as
  it lands — which also sets the no-hit-look bit) plays its `SFXX` sound
  chain from the critter at 0xE0, faded (panned during DEATH); the boss
  key's sound is panned from the key. Left: `take_hit`'s sounds (played
  centred by `damage.rs`: switch once the helper's damage.rs work is
  merged). **Next (decoded, critters.md "What a DAMG does"):** critter
  missiles as the game has them — hit radius `DAMG +0x08` (the Rust field
  `life` is misnamed), model size `SFXX +0x4C` (`CritterSound::size`),
  life `SFXX +0x3C` else the effect's clip, no missile without an effect
  record; at a stop the hit record's sound (faded, 0xE0) and, with a hit
  record (or at a wall/expiry, the element's default), the burst of
  `+0x0C`; kinds 2 (attached to the node), 3 (attached) and 8 (at the
  target) are still area blasts over the effect's life like 5/6
  (`effects::CritterBlast`, needs a node-following shape), not missiles
  or instant rings; the blow effects' models.
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

**Level fidelity (user request, 2026-09-30; top priority after the job in
hand).** Progress and the full list: [STATUS.md](STATUS.md) "What's
left" sections 1–2. The user found levels with things placed that shouldn't be
there, and levels that can't be finished because objects don't move when
a lever is used, "etc". Every level must match the original and every
item/object mechanic work. Split, side by side:
- the helper (static, no game runs): the decompile's placement
  activation rules (player count, difficulty, quest/realm state, the
  placement flags, what the population skips) and every trigger/lever →
  target link kind (movers, lifts, bridges, doors, rotators, generators,
  hidden items), against `population.rs`/`items.rs`/`mechanics.rs`; a dev
  example that scans all 67 levels and lists each placement or link the
  port mishandles, then fixes;
- me (in game): each level from its start — pull every lever, ride the
  lifts, open the doors, reach the exit — fixing what fails, logging
  each level's state here.

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
   (NEWSHARDS … RUNE13*) and the pickup notices; the secret realm's
   timer, coins and ALLCOINS (items.md "The secret realm"). Saving is done
   (a file for the card).
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
- Test scripts in `tools/` (output in `$GDL_TEST_OUT`, default
  `$TMPDIR/gdl-tests`, never in the repository; the game folder is
  `$GDL_GAME`):
  - `smoke.sh [first-level]`: every level folder with `GDL_BUTTONS=attack
    GDL_STICK="0.4,1" GDL_SHOT_AT=400` under a 120 s timeout; stops at a
    panic or a missing screenshot.
  - `tour.sh <levels…>`: per level, one run that stands the hero on each
    one-player trigger in turn (`GDL_HOPS`, 3 s apart, attacking at each;
    `GDL_SKIP_BOXES` puts message boxes away, `GDL_IMMORTAL` keeps the
    hero alive) and prints which switched on and whether the node each
    one moves arrived (`tour.py`). Animated targets read "no mover":
    check those on screen.
  - `nomodel.sh`: the placements the port builds no model for.
  - `waitrun.sh <command…>`: runs one game at a time (a lock, then waits
    for any `gdl-game`), muted. Run spot checks through it too.
- `level_audit` example (`cargo run -p gdl-formats --example level_audit
  -- <game>/Gauntlet <level> --triggers --anim --falls`): what a level's
  build changes, each trigger's link with a warp point, each animated
  object's first and last pose.
- Disk: `target/` grows large. Split debug info leaves every build's
  object files for our crates in `target/debug/deps` (37 GB after a day
  of edits, which filled the disk on 2026-09-30). Free it with
  `cargo clean -p gdl-game -p gdl-formats -p gdl-install` (only our crates
  rebuild; deleting the files by hand breaks cargo's fingerprints). Agent
  worktrees have their own `target/` dirs; delete those after merging.
