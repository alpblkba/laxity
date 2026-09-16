#!/usr/bin/env bash
# the memory interference campaign, one capture per configuration, in one pass over a table.
#
#   ./bench/sweeps/campaign_run.sh [name ...]
#
# with no argument it runs every configuration in the table. with names it runs only those, which
# is how a single cell is repeated without touching the rest. a configuration whose capture
# directory already holds a non-empty telemetry.bin is skipped, so an interrupted campaign
# continues where it stopped rather than starting over.
#
# every knob is chosen on the board at run time. one image produces all 28 captures, since
# relinking the same source has moved a median here by 85 cycles and a binary per configuration
# would put the linker inside every measured difference.
#
# the predictions this campaign tests were written before it ran, in
# self-docs/INTERFERENCE-CAMPAIGN-2026-09-15.md.
set -euo pipefail
cd "$(dirname "$0")/../.."
. tools/stm32/lib.sh

SECS="${LAXITY_SECS:-50}"
# long enough for the board to finish the pass it was in the middle of. the arena cross pass is
# 328 inferences at 50 Hz, which is 6.6 seconds, and it is the longest one the board can be in.
SETTLE="${LAXITY_SETTLE:-8}"
CONFIRM="${LAXITY_CONFIRM:-20}"
CAMPAIGN=mechanism-2026-09-16

TIMEOUT="$(laxity_timeout)" || { echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }
PORT="$(laxity_stlink_ports | cut -f2 | head -1)"
[ -c "$PORT" ] || { echo "no ST-LINK serial port" >&2; exit 1; }

# the console byte alphabet, from laxity_poll_console() in firmware/stm32u585/Src/app_threadx.c.
sweep_name()  { case "$1" in 0) echo bw ;; 1) echo xact ;; 2) echo chan ;; 3) echo stride ;; 4) echo sat ;;
                             5) echo low ;; 6) echo dmat ;; esac; }
region_name() { case "$1" in a|X) echo sram1 ;; b|Y) echo sram2 ;; c|Z) echo sram3 ;; d) echo sram4 ;; esac; }
region_id()   { case "$1" in a|X) echo 1 ;; b|Y) echo 2 ;; c|Z) echo 3 ;; d) echo 4 ;; esac; }
victim_name() { case "$1" in r) echo read_loop ;; i) echo inference ;; t) echo dma_only ;; esac; }
# the descriptor page each region keeps, from laxity_place() in the firmware. the status line
# carries the address, so what is waited for is the page rather than the request.
desc_addr()   { case "$1" in 1) echo 0x2000e000 ;; 2) echo 0x2003e000 ;; 3) echo 0x2004b000 ;;
                             4) echo 0x28003000 ;; esac; }
desc_id()     { case "$1" in *P*) echo 1 ;; *Q*) echo 2 ;; *S*) echo 4 ;; *) echo 3 ;; esac; }
foot_bytes()  { case "$1" in A) echo 1024 ;; B) echo 2048 ;; C) echo 4096 ;; D) echo 8192 ;;
                             E) echo 16384 ;; F) echo 32768 ;; G) echo 65536 ;; H) echo 131072 ;; esac; }
load_count()  { case "$1" in L) echo 8192 ;; l) echo 4096 ;; esac; }
# the read loop window each region has, from LAXITY_VICTIM_BYTES_S1 to _S3 in the firmware. the
# footprint is clamped to it on the board, so the host has to clamp the same way or the line it
# waits for is one the board will never print.
window_bytes() { case "$1" in X) echo 131072 ;; Y) echo 4096 ;; Z) echo 4096 ;; esac; }

send_bytes() {
  # the port settings live only while the device is open, so the descriptor is held across the
  # stty and the writes. closing between them puts the line back to 9600 and the board sees noise.
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  local b
  for b in "$@"; do printf '%s' "$b" >&3; sleep 0.25; done
  exec 3>&-
}

# wait for the board to say it is in the configuration this capture is about to be filed under.
# a sleep alone cannot tell a pass boundary from a missed byte, and fifty seconds of records filed
# under the wrong knobs is worse than a failed run.
await_status() {
  local want="$1" line
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  # the stream carries framed binary records alongside the text status lines, so grep has to treat
  # it as text rather than refusing it as binary.
  line="$("$TIMEOUT" "$CONFIRM" cat "$PORT" 2>/dev/null | LC_ALL=C grep -a -m1 -F "$want" || true)"
  exec 3>&-
  printf '%s' "$line"
}

run_one() {
  local name="$1" experiment="$2" vic="$3" sweep="$4" aggr="$5" vreg="$6" foot="$7" loads="$8"
  # an optional extra console byte, for a configuration the six main knobs cannot express. "-" is
  # the normal case and sends nothing.
  local extra="${9:--}"
  local dir words passes want status

  # a capture whose stress.txt carries a void line does not count as done, so a configuration that
  # produced an unusable capture is taken again under a new timestamp. the unusable one stays on
  # disk with its reason beside it, since results/raw is append only.
  dir="$(ls -dt results/raw/*-"$name" 2>/dev/null | head -1 || true)"
  if [ -n "$dir" ] && [ -s "$dir/telemetry.bin" ] && ! grep -q '^void=' "$dir/stress.txt"; then
    echo "skip $name, already captured in $dir"
    return 0
  fi

  # the same arithmetic the measurement thread does at the top of every pass, so the status line
  # can be matched exactly rather than approximately.
  words=$(( $(foot_bytes "$foot") / 4 ))
  if [ "$words" -gt $(( $(window_bytes "$vreg") / 4 )) ]; then
    words=$(( $(window_bytes "$vreg") / 4 ))
  fi
  passes=$(( $(load_count "$loads") / words ))
  [ "$passes" -eq 0 ] && passes=1

  local vname did m2m
  case "$vic" in r) vname=read ;; t) vname=dma ;; *) vname=infer-stress ;; esac
  did="$(desc_id "$extra")"
  m2m=0
  case "$extra" in *M*) m2m=1 ;; esac
  want="stress victim=$vname"
  want="$want m2m=$m2m"
  # the stack region and the descriptor page are part of what a capture is filed under, since both
  # decide where the address stream goes and neither is visible in a record.
  want="$want stack=1 desc=$did($(desc_addr "$did"))"
  want="$want vregion=$(region_id "$vreg") vwords=$words vpasses=$passes vloads=$(( words * passes ))"
  want="$want sweep=$(sweep_name "$sweep") region=$(region_id "$aggr") ok=1"

  echo "=== $name: $experiment, victim $(victim_name "$vic") in $(region_name "$vreg"), aggressor in $(region_name "$aggr"), $(sweep_name "$sweep") sweep"
  # the extra column is a string of console bytes for anything the six main knobs cannot express.
  # "-" sends the two bytes that put the board back in its default state.
  if [ "$extra" = "-" ]; then
    send_bytes "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" N R
  else
    send_bytes "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" $(echo "$extra" | sed 's/./& /g')
  fi
  sleep "$SETTLE"

  status="$(await_status "$want")"
  if [ -z "$status" ]; then
    echo "board never reported \"$want\" within ${CONFIRM}s, refusing to capture $name" >&2
    exit 1
  fi

  ./tools/stm32/capture.sh "$name" "$SECS"

  dir="$(ls -dt results/raw/*-"$name" | head -1)"
  # the sweep, the victim and the footprint are not in the record. the wire format was not changed
  # for this campaign, so they go beside the capture, and the board's own status line goes with
  # them so the filing can be checked against what the board said rather than against this table.
  {
    printf 'campaign=%s\n' "$CAMPAIGN"
    printf 'experiment=%s\n' "$experiment"
    printf 'victim=%s\n' "$(victim_name "$vic")"
    printf 'victim_region=%s\n' "$(region_name "$vreg")"
    printf 'victim_words=%s\n' "$words"
    printf 'victim_passes=%s\n' "$passes"
    printf 'loads_per_window=%s\n' "$(( words * passes ))"
    printf 'sweep=%s\n' "$(sweep_name "$sweep")"
    printf 'aggressor_region=%s\n' "$(region_name "$aggr")"
    printf 'aggressor=gpdma_stress\n'
    printf 'channels_used=GPDMA1_12..15\n'
    printf 'console_bytes=%s\n' "$vic$sweep$aggr$vreg$foot$loads$extra"
    printf 'status_line=%s\n' "$(printf '%s' "$status" | tr -d '\r')"
    # what the victim's address stream actually does, counted from the disassembly of the image that
    # ran rather than taken from the knob. only the read loop has one.
    if [ "$vic" = r ]; then
      ./bench/sweeps/victim_access_mix.py --elf build/target/laxity-u585.elf \
        --words "$words" --passes "$passes" \
        --buffer-region "$(region_name "$vreg")" --stack-region sram1
    else
      printf 'victim_access_mix=not applicable, the victim is %s\n' "$(victim_name "$vic")"
    fi
  } > "$dir/stress.txt"
  echo "wrote $dir/stress.txt"
}

# name                        experiment     victim sweep aggressor victim-region footprint loads extra
TABLE="
stress-dma-a1                 dma-throughput t 6 a X C L -
stress-dma-a2                 dma-throughput t 6 b X C L -
stress-dma-a3                 dma-throughput t 6 c X C L -
stress-dma-a4                 dma-throughput t 6 d X C L -
stress-k2-v1-a1               k-matrix-2     r 0 a X C L -
stress-k2-v1-a2               k-matrix-2     r 0 b X C L -
stress-k2-v1-a3               k-matrix-2     r 0 c X C L -
stress-k2-v1-a4               k-matrix-2     r 0 d X C L -
stress-k2-v2-a1               k-matrix-2     r 0 a Y C L -
stress-k2-v2-a2               k-matrix-2     r 0 b Y C L -
stress-k2-v2-a3               k-matrix-2     r 0 c Y C L -
stress-k2-v2-a4               k-matrix-2     r 0 d Y C L -
stress-k2-v3-a1               k-matrix-2     r 0 a Z C L -
stress-k2-v3-a2               k-matrix-2     r 0 b Z C L -
stress-k2-v3-a3               k-matrix-2     r 0 c Z C L -
stress-k2-v3-a4               k-matrix-2     r 0 d Z C L -
stress-low-v1-a1              low-rate       r 5 a X C L -
stress-low-v1-a3              low-rate       r 5 c X C L -
stress-low-v3-a1              low-rate       r 5 a Z C L -
stress-low-v3-a3              low-rate       r 5 c Z C L -
stress-desc-p1                descriptor     r 2 b X C L P
stress-desc-p2                descriptor     r 2 b X C L Q
stress-desc-p3                descriptor     r 2 b X C L R
stress-desc-p4                descriptor     r 2 b X C L S
"

# a plain string rather than an array, since an empty array under set -u is an error in the bash
# that ships with macOS.
WANTED=" $* "
while read -r name experiment vic sweep aggr vreg foot loads extra; do
  [ -z "${name:-}" ] && continue
  if [ -n "$*" ]; then
    case "$WANTED" in *" $name "*) ;; *) continue ;; esac
  fi
  run_one "$name" "$experiment" "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" "${extra:--}"
done <<< "$TABLE"

echo "campaign done"
