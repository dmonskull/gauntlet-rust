#!/bin/zsh
# One run per level with the level walker (crates/gdl-game/src/walker.rs):
# the hero plays the level on its own, from its start to its exit, and the
# last line says whether it finished and what it never reached.
# Usage: tools/walk.sh level…   (MODE=all: every lever before the exit;
# LIMIT=<seconds> per level, default 600; HERO=<class code>, default SUM)
# Output in $GDL_TEST_OUT/walk (default $TMPDIR/gdl-tests): <level>.log,
# <level>.png (where it stopped).
ROOT=${0:A:h:h}
cd $ROOT
GAME=${GDL_GAME:-$HOME/Desktop/GauntletDarkLegacy}
BIN=${GDL_BIN:-./target/debug/gdl-game}
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}/walk
mkdir -p $OUT/save
for lv in "$@"; do
  GDL_SAVE_DIR=$OUT/save GDL_WALK=${MODE:-1} GDL_WALK_SHOT=$OUT/$lv.png GDL_IMMORTAL=1 GDL_SKIP_BOXES=1 GDL_SKIP_INTRO=1 \
    RUST_LOG=${WALK_LOG:-warn,gdl_game::walker=debug,gdl_game::items=info} \
    $ROOT/tools/waitrun.sh timeout ${LIMIT:-600} $BIN $GAME --level $lv --character ${HERO:-SUM} > $OUT/$lv.log 2>&1
  last=$(sed 's/\x1b\[[0-9;]*m//g' $OUT/$lv.log | grep -E "walker: $lv (FINISHED|STUCK)" | tail -1 | sed -E 's/^.*walker: //')
  if grep -q panicked $OUT/$lv.log; then echo "$lv: PANIC"; fi
  echo "${last:-$lv: ran out of time (${LIMIT:-600} s)}" | cut -c1-600
done
echo done
