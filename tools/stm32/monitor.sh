#!/usr/bin/env bash
# follow the board's human readable status lines.
#
#   ./tools/stm32/monitor.sh            follow until interrupted
#   ./tools/stm32/monitor.sh 20         stop after about twenty seconds
#   ./tools/stm32/monitor.sh --reset    reset the board first, then follow
#
# whatever else holds the port is closed first, because only one reader gets the bytes and a
# terminal left open in another window is the usual reason this prints nothing. every process it
# closes is named on the way past. --keep leaves them alone.
#
# the UART carries framed binary telemetry as well as text, and the frames are most of the
# volume, so a terminal attached to the port shows mostly noise with the text buried and
# overwritten by control bytes inside the frames. strings(1) picks the text back out.
#
# it reads in short bursts rather than one long one because strings fills a four kilobyte buffer
# before it writes anything, which on this stream is half a minute of silence and looks exactly
# like a board that has stopped. ending each burst flushes it. a few bytes are lost between
# bursts, which costs nothing here and would matter for a capture, so captures do not do this.
set -euo pipefail
LAXITY_STM32_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$LAXITY_STM32_DIR/lib.sh"
cd "$LAXITY_STM32_DIR/../.."

BAUD="${LAXITY_BAUD:-921600}"
BURST="${LAXITY_BURST:-2}"
CLAIM=1
RESET=0
SECS=""
for arg in "$@"; do
  case "$arg" in
    --reset) RESET=1 ;;
    --keep)  CLAIM=0 ;;
    ''|*[!0-9]*) echo "usage: monitor.sh [seconds] [--reset] [--keep]" >&2; exit 2 ;;
    *) SECS="$arg" ;;
  esac
done

# close anything already reading the port. only processes this device names are touched, and
# never this script or the shell that started it.
release_port() {
  local pid cmd left
  for pid in $(lsof -t "$1" 2>/dev/null | sort -u); do
    [ "$pid" = "$$" ] && continue
    [ "$pid" = "$PPID" ] && continue
    cmd="$(ps -p "$pid" -o comm= 2>/dev/null || echo unknown)"
    # a shell holding the port has a descriptor left open by something that already finished. it
    # reads nothing, so it steals no bytes, and killing it would close the window it belongs to.
    case "${cmd##*/}" in
      -*|zsh|bash|sh|dash|fish|login)
        echo "note: $cmd ($pid) still has $1 open, harmless, not touching it" >&2
        continue ;;
    esac
    echo "releasing $1 from $cmd ($pid)" >&2
    kill "$pid" 2>/dev/null || true
  done
  sleep 1
  left="$(lsof -t "$1" 2>/dev/null | sort -u | grep -v "^$$\$" || true)"
  for pid in $left; do
    [ "$pid" = "$PPID" ] && continue
    cmd="$(ps -p "$pid" -o comm= 2>/dev/null || echo unknown)"
    case "${cmd##*/}" in -*|zsh|bash|sh|dash|fish|login) continue ;; esac
    kill -9 "$pid" 2>/dev/null || true
  done
  [ -n "$left" ] && sleep 1
  # a detached screen leaves a dead session behind once its process is gone
  screen -wipe >/dev/null 2>&1 || true
}

TIMEOUT="$(laxity_timeout)" || { echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }

if [ -n "${LAXITY_PORT:-}" ]; then
  PORT="$LAXITY_PORT"
else
  CANDIDATES="$(laxity_stlink_ports)"
  COUNT="$(printf '%s' "$CANDIDATES" | grep -c . || true)"
  if [ "$COUNT" -ne 1 ]; then
    echo "expected exactly one ST-LINK virtual COM port, saw $COUNT" >&2
    printf '%s\n' "$CANDIDATES" >&2
    exit 1
  fi
  PORT="$(printf '%s' "$CANDIDATES" | cut -f2)"
fi
[ -c "$PORT" ] || { echo "$PORT is not a character device" >&2; exit 1; }

[ "$CLAIM" -eq 1 ] && release_port "$PORT"

# a debug session leaves the core halted, and a halted board is silent in exactly the way a
# crashed one is. this is the fix for a port that stays quiet after everything else is closed.
if [ "$RESET" -eq 1 ]; then
  echo "resetting the board" >&2
  STM32_Programmer_CLI -c port=SWD mode=Hotplug -rst >/dev/null 2>&1 || true
  sleep 2
fi

# holding the port open keeps the line settings alive, since they last only as long as the device
# is open and stty on its own opens and closes it again.
exec 3<"$PORT"
stty -f "$PORT" "$BAUD" raw -echo

# the first burst is discarded. the port buffers seconds of output while nothing is reading and
# that backlog survives reopening, so without this the first lines describe a state the board
# left behind some time ago.
"$TIMEOUT" "$BURST" cat "$PORT" >/dev/null 2>&1 || true

deadline=0
[ -n "$SECS" ] && deadline=$(( $(date +%s) + SECS ))

while :; do
  "$TIMEOUT" "$BURST" cat "$PORT" 2>/dev/null | strings -n 12 | grep -E 'infer |sensors |placement |net |telemetry ' || true
  # an "&&" chain whose first test fails returns non-zero, and set -e would end the loop there,
  # which made this stop after a single burst whether or not a deadline had been asked for.
  if [ "$deadline" -ne 0 ] && [ "$(date +%s)" -ge "$deadline" ]; then break; fi
done
exec 3<&-
