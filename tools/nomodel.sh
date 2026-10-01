#!/bin/zsh
# Loads every level briefly and lists the placements the port builds no
# model for (all.txt in $GDL_TEST_OUT/nomodel).
ROOT=${0:A:h:h}
cd $ROOT
GAME=${GDL_GAME:-$HOME/Desktop/GauntletDarkLegacy}
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}/nomodel
mkdir -p $OUT
: > $OUT/all.txt
for dir in $GAME/Gauntlet/LEVELS/level*(/); do
  level=${dir:t}
  [[ $level == levelC2_acorn || $level == levelT4 ]] && continue
  GDL_SHOT_CLOCK=ticks GDL_SHOT_AT=3 GDL_SHOTS=1 GDL_SCREENSHOT=$OUT/$level.png RUST_LOG=warn,gdl_game::population=debug \
    $ROOT/tools/waitrun.sh timeout 120 ./target/debug/gdl-game $GAME --level $level 2>&1 | grep "no model" \
    | sed "s/.*Z[^ ]* *//; s/^/$level: /" >> $OUT/all.txt
done
echo done
