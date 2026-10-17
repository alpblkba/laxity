#!/usr/bin/env bash
# the paired audio acquisition run, one capture with the inference path running and one without.
#
#   ./bench/sweeps/audio_loss_run.sh [seconds]
#
# it produces two captures of one configuration that differ only in whether inference runs, and it produces no loss figure and no verdict: tools/analyse.py turns the folded half count in each capture into halves lost, and nothing here or there says what a count means.
#
# written and never executed. it was validated by reading stress_run.sh and tools/stm32/lib.sh and by bash -n, and by nothing else, so every timing constant below is inherited from stress_run.sh rather than measured here.
#
# the two arms are console keys that already exist. v makes the inference path the victim, which is the arm where a pass can overrun a half buffer period, and t selects no victim and times the DMA transfer instead, which is the arm where the acquisition path runs with nothing competing for the core. one image produces both, because a binary per arm would put a relink inside the difference and a relink alone has moved a median on this project by 85 cycles.
set -euo pipefail
cd "$(dirname "$0")/../.."
. tools/stm32/lib.sh

# the UART carries framed binary records interleaved with human readable status lines, so a byte that is not valid in the current locale reaches the filters below. macOS tr dies on one, which is what the locale is pinned for.
export LC_ALL=C

SECS="${1:-${LAXITY_SECS:-75}}"
# long enough for the board to finish the pass it was in the middle of, which is the same 3 seconds stress_run.sh waits for the same reason.
SETTLE="${LAXITY_SETTLE:-3}"

PORT="$(laxity_stlink_ports | cut -f2 | head -1)"
[ -c "$PORT" ] || { echo "no ST-LINK serial port" >&2; exit 1; }

arm() {
  local key="$1" name="$2"
  # the port settings live only while the device is open, so the descriptor is held across the stty and the write. closing between them puts the line back to 9600 and the board sees noise.
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  printf '%s' "$key" >&3
  exec 3>&-
  # the board applies the byte on its next pass, so the wait is what keeps the first records of the capture inside the arm this run is filed under.
  sleep "$SETTLE"
  ./tools/stm32/capture.sh "$name" "$SECS"
  local dir
  dir="$(ls -dt results/raw/*-"$name" | head -1)"
  # the arm is not in the record, because the wire format is not being changed for this. it goes beside the capture, where analysis can read it.
  {
    printf 'arm=%s\n' "$name"
    printf 'console_key=%s\n' "$key"
    printf 'seconds=%s\n' "$SECS"
  } > "$dir/audio.txt"
  echo "wrote $dir/audio.txt"
}

arm v audio-infer-on
arm t audio-infer-off
