#!/usr/bin/env bash
# run one stress sweep configuration and capture it.
#
#   ./bench/sweeps/stress_run.sh <sweep 0-3> <region a|b|c> <seconds>
#
# the four knobs are chosen on the board at run time, so this sends the selection and then calls
# the existing capture script. one image produces every configuration: a binary per configuration
# would put a relink inside every measured difference, and a relink alone has moved a median on
# this project by 85 cycles.
set -euo pipefail
cd "$(dirname "$0")/../.."
. tools/stm32/lib.sh

SWEEP="${1:?sweep index 0-3}"
REGION="${2:?region a, b or c}"
SECS="${3:-45}"

case "$SWEEP" in 0) SNAME=bw ;; 1) SNAME=xact ;; 2) SNAME=chan ;; 3) SNAME=stride ;;
  *) echo "sweep must be 0..3" >&2; exit 1 ;; esac
case "$REGION" in a) RNAME=sram1 ;; b) RNAME=sram2 ;; c) RNAME=sram3 ;;
  *) echo "region must be a, b or c" >&2; exit 1 ;; esac

PORT="$(laxity_stlink_ports | cut -f2 | head -1)"
[ -c "$PORT" ] || { echo "no ST-LINK serial port" >&2; exit 1; }

# the port settings live only while the device is open, so the descriptor is held across the
# stty and the writes. closing between them puts the line back to 9600 and the board sees noise.
exec 3<>"$PORT"
stty -f "$PORT" 921600 raw -echo
# victim first, then the axis, then the region. each byte is independent and the board applies it
# on the next pass, so a short gap between them is enough and none of them has to arrive together.
printf 'r' >&3; sleep 0.3
printf '%s' "$SWEEP" >&3; sleep 0.3
printf '%s' "$REGION" >&3; sleep 0.3
exec 3>&-

# let the board finish the pass it was in the middle of before the capture starts, so the first
# records of the capture already belong to the configuration this run is filed under.
sleep 3

NAME="stress-${SNAME}-${RNAME}"
LAXITY_STRESS_SWEEP="$SNAME" LAXITY_STRESS_REGION="$RNAME" \
  ./tools/stm32/capture.sh "$NAME" "$SECS"

# the sweep and the region are not in the record, because the wire format is not being changed for
# this. they go beside the capture instead, where analysis can read them.
DIR="$(ls -dt results/raw/*-"$NAME" | head -1)"
{
  printf 'victim=read_loop\n'
  printf 'sweep=%s\n' "$SNAME"
  printf 'aggressor_region=%s\n' "$RNAME"
  printf 'victim_region=sram1\n'
  printf 'aggressor=gpdma_stress\n'
  printf 'channels_used=GPDMA1_12..15\n'
  printf 'loads_per_window=%s\n' "$((1024 * 8))"
} > "$DIR/stress.txt"
echo "wrote $DIR/stress.txt"
