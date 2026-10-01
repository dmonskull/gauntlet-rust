#!/bin/zsh
# One run per level: hops the hero onto each one-player trigger in turn
# (GDL_HOPS, 3 s apart, attacking at each for hit switches; message boxes
# put away, the hero kept alive) and reports which switched on and
# whether the node each one moves arrived (tour.py).
# Usage: tools/tour.sh level…   (EVERY=<ticks> changes the spacing)
# Output in $GDL_TEST_OUT/tour (default $TMPDIR/gdl-tests).
ROOT=${0:A:h:h}
cd $ROOT
GAME=${GDL_GAME:-$HOME/Desktop/GauntletDarkLegacy}
BIN=${GDL_BIN:-./target/debug/gdl-game}
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}/tour
mkdir -p $OUT
EVERY=${EVERY:-90}
cargo build -q -j 2 -p gdl-formats --example items || exit 1
for lv in "$@"; do
  hops=""; buttons=""; i=0
  # Placements for one player: "players 0/1" and "11" (exactly one).
  ./target/debug/examples/items $GAME/Gauntlet/LEVELS/$lv 2>/dev/null | grep -E " TRIGGER | ROTATOR " \
    | grep -v -E "players ( [234]|1[234])" > $OUT/$lv.triggers
  while read -r line; do
    pos=$(echo "$line" | sed -E 's/.*pos \( *([-0-9.]+), *([-0-9.]+), *([-0-9.]+)\).*/\1,\2,\3/')
    x=${pos%%,*}; rest=${pos#*,}; y=${rest%%,*}; z=${rest#*,}
    hops+="$x,$((y+0.3)),$z;"
    t=$(( EVERY*(i+1) ))
    buttons+="attack@$((t+10))-$((t+40)),"
    i=$((i+1))
  done < $OUT/$lv.triggers
  if (( i == 0 )); then echo "$lv: no one-player triggers"; continue; fi
  end=$(( EVERY*(i+1) + 150 ))
  GDL_SKIP_BOXES=1 GDL_IMMORTAL=1 GDL_HOPS="$hops" GDL_HOP_TICKS=$EVERY GDL_STICK="0,0.15" GDL_BUTTONS="${buttons%,}" \
    GDL_SHOT_CLOCK=ticks GDL_SHOT_AT=$end GDL_SHOTS=1 GDL_SCREENSHOT=$OUT/$lv.png \
    RUST_LOG=warn,gdl_game::mechanics=debug,gdl_game::player=info,gdl_game::items=info,gdl_game::message_box=info \
    $ROOT/tools/waitrun.sh timeout $(( 40 + end/30 )) $BIN $GAME --level $lv > $OUT/$lv.log 2>&1
  python3 $ROOT/tools/tour.py $OUT/$lv.triggers $OUT/$lv.log $lv
done
echo done
