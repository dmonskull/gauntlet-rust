#!/bin/zsh
# Two games on this machine playing online over loopback: one hosts, the
# other joins with the invite (through a file, not the clipboard), each
# picks a new hero, and they play for a while on scripted sticks. Every 30
# ticks both log a hash of their game state (`GDL_SYNC_LOG`); the hashes
# must match tick for tick.
# Usage: [GDL_ONLINE_LEVEL=levelA1] [SHOT_AT=<frame>] tools/online_test.sh [seconds]
ROOT=${0:A:h:h}
GAME=${GDL_GAME:-$HOME/Desktop/GauntletDarkLegacy}
BIN=${GDL_BIN:-$ROOT/target/debug/gdl-game}
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}/online
SECS=${1:-90}
mkdir -p $OUT
rm -f $OUT/invite.txt $OUT/host.log $OUT/client.log
export GDL_NET_LOCAL=1 GDL_INVITE_FILE=$OUT/invite.txt GDL_SYNC_LOG=1 GDL_SKIP_BOXES=1 RUST_LOG=${RUST_LOG:-info}
$ROOT/tools/waitrun.sh zsh -c "
  GDL_ONLINE=host GDL_ONLINE_PLAYERS=2 GDL_ONLINE_HERO=WAR GDL_STICK=0.3,1 GDL_BUTTONS=attack@200-900 \
    GDL_SCREENSHOT=$OUT/host.png GDL_SHOT_AT=${SHOT_AT:-999999} \
    timeout $SECS $BIN $GAME > $OUT/host.log 2>&1 &
  for i in {1..60}; do [[ -s $OUT/invite.txt ]] && break; sleep 0.5; done
  GDL_ONLINE=join GDL_ONLINE_HERO=VAL GDL_STICK=-0.3,1 GDL_BUTTONS=attack@300-1200 \
    GDL_SCREENSHOT=$OUT/client.png GDL_SHOT_AT=${SHOT_AT:-999999} \
    timeout $SECS $BIN $GAME > $OUT/client.log 2>&1 &
  wait
"
for side in host client; do
  grep -o 'sync tick [0-9]*: [0-9a-f]*' $OUT/$side.log | sed 's/sync tick //; s/://' | sort -n > $OUT/$side.sync
done
echo "host ticks: $(wc -l < $OUT/host.sync), client ticks: $(wc -l < $OUT/client.sync)"
mismatch=$(join $OUT/host.sync $OUT/client.sync | awk '$2 != $3' | head -3)
if grep -q 'panicked' $OUT/host.log $OUT/client.log; then
  echo "PANIC"; grep -h -m 3 -A 3 'panicked' $OUT/host.log $OUT/client.log
  exit 1
elif [[ -n $mismatch ]]; then
  echo "OUT OF SYNC (tick host client):"; echo $mismatch
  exit 1
elif [[ ! -s $OUT/host.sync ]]; then
  echo "NO TICKS"; tail -5 $OUT/host.log $OUT/client.log
  exit 1
else
  echo "in sync over $(join $OUT/host.sync $OUT/client.sync | wc -l | tr -d ' ') checks"
fi
