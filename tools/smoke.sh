#!/bin/zsh
# Runs every level 120 s with the hero attacking and moving; stops at the
# first panic or missing screenshot.
# Usage: tools/smoke.sh [first-level]   (resume from that level)
# Output in $GDL_TEST_OUT/smoke (default $TMPDIR/gdl-tests); the game
# folder is $GDL_GAME.
ROOT=${0:A:h:h}
cd $ROOT
GAME=${GDL_GAME:-$HOME/Desktop/GauntletDarkLegacy}
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}/smoke
mkdir -p $OUT
start=${1:-}
started=0
[[ -z $start ]] && started=1
for dir in $GAME/Gauntlet/LEVELS/level*(/) $GAME/Gauntlet/LEVELS/DEMO1; do
  level=${dir:t}
  [[ $level == $start ]] && started=1
  (( started )) || continue
  if [[ $level == levelC2_acorn || $level == levelT4 ]]; then
    echo "$level skipped (empty folder)"; continue
  fi
  GDL_BUTTONS=attack GDL_STICK="0.4,1" GDL_SHOT_AT=400 GDL_SCREENSHOT=$OUT/$level.png RUST_LOG=warn \
    $ROOT/tools/waitrun.sh timeout 120 ./target/debug/gdl-game $GAME --level $level > $OUT/$level.log 2>&1
  code=$?
  if grep -q "panicked" $OUT/$level.log; then
    echo "$level PANIC (exit $code)"; grep -m 3 -A 2 "panicked" $OUT/$level.log
    exit 1
  elif [[ ! -f $OUT/$level.png ]]; then
    echo "$level NO SHOT (exit $code)"; tail -5 $OUT/$level.log
    exit 1
  else
    echo "$level ok"
  fi
done
echo "smoke: all done"
