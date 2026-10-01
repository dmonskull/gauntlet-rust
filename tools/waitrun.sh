#!/bin/zsh
# Runs a command (a game run) muted, one at a time: it waits for the lock
# and for any running gdl-game to end. Every test script runs the game
# through this.
# Usage: tools/waitrun.sh [env VAR=… ] <command…>
OUT=${GDL_TEST_OUT:-${TMPDIR:-/tmp}/gdl-tests}
mkdir -p $OUT
LOCK=$OUT/game.lock
until mkdir $LOCK 2>/dev/null; do sleep 1; done
trap 'rmdir $LOCK 2>/dev/null' EXIT INT TERM
until ! pgrep -x gdl-game >/dev/null; do sleep 1; done
export GDL_MUTE=1
"$@"
