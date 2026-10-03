#!/bin/zsh
# Two games on this machine playing online over loopback: one hosts, the
# other joins with the invite (through a file, not the clipboard), each
# picks a new hero, and they play for a while on scripted sticks. Every 30
# ticks both log a hash of their game state (`GDL_SYNC_LOG`); the hashes
# must match tick for tick.
# Usage: [GDL_ONLINE_LEVEL=levelA1] [SHOT_AT=<frame>] [HOST_MENU=<GDL_MENU>]
#        [CLIENT_MENU=<GDL_MENU>] [HOST_PLAYERS=1: the host starts alone and the
#        client joins the game under way] [CLIENT_DESYNC_AT=<tick>: the client's
#        game goes its own way then; a sync point must put the games together
#        again, with no level starting again]
#        [FIGHT=1: the heroes hop to HOPS (default 0,0,40, among levelA1's
#        grunts), stand (or HOST_STICK / CLIENT_STICK) and throw every 9 ticks,
#        immortal unless MORTAL=1] [HOST_HERO=WAR] [CLIENT_HERO=VAL]
#        [CLIENT_PREFIX="taskpolicy -b" (or HOST_PREFIX): that side runs slowed
#        down, as a slower machine would — the games must agree however
#        unevenly the two draw] tools/online_test.sh [seconds]
ROOT=${0:A:h:h}
GAME=${GDL_GAME:-$HOME/Desktop/GauntletDarkLegacy}
BIN=${GDL_BIN:-$ROOT/target/debug/gdl-game}
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}/online
SECS=${1:-90}
mkdir -p $OUT
rm -f $OUT/invite.txt $OUT/host.log $OUT/client.log
export GDL_NET_LOCAL=1 GDL_INVITE_FILE=$OUT/invite.txt GDL_SYNC_LOG=${GDL_SYNC_LOG:-1} GDL_SKIP_BOXES=1 RUST_LOG=${RUST_LOG:-info}
HOST_BUTTONS=attack@200-900 CLIENT_BUTTONS=attack@300-1200
if [[ -n ${FIGHT:-} ]]; then
  # A press every 9 ticks (held, the attack button throws once).
  HOST_BUTTONS=$(python3 -c "print(','.join(f'attack@{t}-{t+2}' for t in range(90,20000,9)))")
  CLIENT_BUTTONS=$HOST_BUTTONS
  export GDL_ONLINE_LEVEL=${GDL_ONLINE_LEVEL:-levelA1} GDL_HOP_TICKS=60
  [[ ${HOPS:-} != none ]] && export GDL_HOPS=${HOPS:-0,0,40}
  [[ -z ${MORTAL:-} ]] && export GDL_IMMORTAL=1
  : ${HOST_STICK:=0,0} ${CLIENT_STICK:=0,0}
fi
$ROOT/tools/waitrun.sh zsh -c "
  GDL_ARTIFACTS='${HOST_ARTIFACTS:-$ROOT/target/debug/gdl-artifacts}' GDL_MENU='${HOST_MENU:-}' GDL_ONLINE=host GDL_ONLINE_PLAYERS=${HOST_PLAYERS:-2} GDL_ONLINE_HERO=${HOST_HERO:-WAR} GDL_STICK=${HOST_STICK:-0.3,1} GDL_BUTTONS=$HOST_BUTTONS \
    GDL_SCREENSHOT=$OUT/host.png GDL_SHOT_AT=${SHOT_AT:-999999} \
    ${HOST_PREFIX:-} timeout $SECS $BIN $GAME > $OUT/host.log 2>&1 &
  for i in {1..60}; do [[ -s $OUT/invite.txt ]] && break; sleep 0.5; done
  GDL_ARTIFACTS='${CLIENT_ARTIFACTS:-$ROOT/target/debug/gdl-artifacts}' GDL_MENU='${CLIENT_MENU:-}' GDL_DESYNC_AT='${CLIENT_DESYNC_AT:-}' GDL_ONLINE=join GDL_ONLINE_HERO=${CLIENT_HERO:-VAL} GDL_STICK=${CLIENT_STICK:--0.3,1} GDL_BUTTONS=$CLIENT_BUTTONS \
    GDL_SCREENSHOT=$OUT/client.png GDL_SHOT_AT=${SHOT_AT:-999999} \
    ${CLIENT_PREFIX:-} timeout $SECS $BIN $GAME > $OUT/client.log 2>&1 &
  wait
"
for side in host client; do
  grep -o 'sync tick [0-9]*: [0-9a-f]*' $OUT/$side.log | sed 's/sync tick //; s/://' | sort > $OUT/$side.sync
done
echo "host ticks: $(wc -l < $OUT/host.sync), client ticks: $(wc -l < $OUT/client.sync)"
if [[ -n ${CLIENT_DESYNC_AT:-} ]]; then
  # The games come apart, a sync point puts them together again (no level
  # starts again), and every check from the point on must agree.
  if ! join $OUT/host.sync $OUT/client.sync | awk '$2 != $3' | grep -q .; then
    echo "NO DESYNC (the client's game never went its own way)"; exit 1
  fi
  grep -h 'games differ\|sync point\|together again\|starts again' $OUT/host.log $OUT/client.log | sed 's/.*Z.\[0m //; s/^.*INFO //; s/^.*WARN //' | sort | uniq -c
  if grep -q 'starts again' $OUT/host.log $OUT/client.log; then echo "THE LEVEL STARTED AGAIN"; exit 1; fi
  from=$(grep -h -o 'together again from tick [0-9]*' $OUT/host.log | tail -1 | grep -o '[0-9]*$')
  if [[ -z $from ]]; then echo "NO SYNC POINT"; exit 1; fi
  for side in host client; do awk -v from=$from '$1 >= from' $OUT/$side.sync > $OUT/$side.sync.tmp && mv $OUT/$side.sync.tmp $OUT/$side.sync; done
fi
mismatch=$(join $OUT/host.sync $OUT/client.sync | awk '$2 != $3' | head -3)
if grep -q 'panicked' $OUT/host.log $OUT/client.log; then
  echo "PANIC"; grep -h -m 3 -A 3 'panicked' $OUT/host.log $OUT/client.log
  exit 1
elif [[ -n $mismatch ]]; then
  echo "OUT OF SYNC (tick host client):"; echo $mismatch
  exit 1
elif [[ ! -s $OUT/host.sync || ! -s $OUT/client.sync ]] || [[ $(join $OUT/host.sync $OUT/client.sync | wc -l) -eq 0 ]]; then
  echo "NO TICKS"; tail -5 $OUT/host.log $OUT/client.log
  exit 1
else
  echo "in sync over $(join $OUT/host.sync $OUT/client.sync | wc -l | tr -d ' ') checks"
fi
