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

SECS="${LAXITY_SECS:-75}"
# long enough for the board to finish the pass it was in the middle of. the arena cross pass is
# 328 inferences at 50 Hz, which is 6.6 seconds, and it is the longest one the board can be in.
SETTLE="${LAXITY_SETTLE:-8}"
CONFIRM="${LAXITY_CONFIRM:-20}"
# the stack region is fixed when the thread is created, so a change to it resets the board and the
# rest of the configuration has to wait for the board to come back. bytes that arrive during boot
# are dropped, which is how the first attempt at this lost its victim byte.
REBOOT="${LAXITY_REBOOT:-12}"
LAST_STACK=""
CAMPAIGN=final-2026-09-19

TIMEOUT="$(laxity_timeout)" || { echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }
PORT="$(laxity_stlink_ports | cut -f2 | head -1)"
[ -c "$PORT" ] || { echo "no ST-LINK serial port" >&2; exit 1; }

# the console byte alphabet, from laxity_poll_console() in firmware/stm32u585/Src/app_threadx.c.
sweep_name()  { case "$1" in 0) echo bw ;; 1) echo xact ;; 2) echo chan ;; 3) echo stride ;; 4) echo sat ;;
                             5) echo low ;; 6) echo dmat ;; o) echo none ;; esac; }
region_name() { case "$1" in a|X) echo sram1 ;; b|Y) echo sram2 ;; c|Z) echo sram3 ;; d) echo sram4 ;; esac; }
region_id()   { case "$1" in a|X) echo 1 ;; b|Y) echo 2 ;; c|Z) echo 3 ;; d) echo 4 ;; esac; }
victim_name() { case "$1" in r) echo read_loop ;; i) echo inference ;; t) echo dma_only ;; esac; }
# the descriptor page each region keeps, from laxity_place() in the firmware. the status line
# carries the address, so what is waited for is the page rather than the request.
desc_addr()   { case "$1" in 1) echo 0x2000e000 ;; 2) echo 0x2003e000 ;; 3) echo 0x2004b000 ;;
                             4) echo 0x28003000 ;; esac; }
desc_id()     { case "$1" in *P*) echo 1 ;; *Q*) echo 2 ;; *S*) echo 4 ;; *) echo 3 ;; esac; }
stack_id()    { case "$1" in j) echo 1 ;; k) echo 2 ;; n) echo 3 ;; esac; }
# the point table of each sweep, mirroring laxity_sweeps in the firmware, as
# index:channels/width/block/stride/triggerHz. a capture that carries this describes every
# configuration it contains, which the confirmation time snapshot below does not.
sweep_points() {
  case "$1" in
    0) echo "1:1/4/256/0/50000 2:1/4/256/0/100000 3:1/4/256/0/200000 4:1/4/256/0/400000 5:1/4/256/0/800000" ;;
    1) echo "1:1/4/256/0/200000 2:1/2/256/0/200000 3:1/1/256/0/200000" ;;
    2) echo "1:1/4/512/0/200000 2:2/4/256/0/200000 3:4/4/128/0/200000" ;;
    3) echo "1:1/4/256/0/200000 2:1/4/256/4/200000 3:1/4/256/12/200000 4:1/4/256/28/200000 5:1/4/256/60/200000 6:1/4/256/124/200000" ;;
    4) echo "1:1/2/256/0/800000 2:1/1/256/0/800000 3:1/4/256/0/0 4:1/1/256/0/0 5:1/4/256/0/armed" ;;
    5) echo "1:1/4/256/0/6250 2:1/4/256/0/12500 3:1/4/256/0/25000 4:1/4/256/0/50000" ;;
    6) echo "1:1/4/4096/0/0 2:1/2/4096/0/0 3:1/1/4096/0/0 4:1/4/2048/0/0" ;;
    o) echo "none, no channel is started at any point" ;;
  esac
}
arena_id()    { case "$1" in 7) echo 1 ;; 8) echo 2 ;; 9) echo 3 ;; esac; }
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
  local stackb="${10:-j}" arenab="${11:-7}"
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
  want="$want stack=$(stack_id "$stackb") arena=$(arena_id "$arenab") desc=$did($(desc_addr "$did"))"
  want="$want vregion=$(region_id "$vreg") vwords=$words vpasses=$passes vloads=$(( words * passes ))"
  want="$want sweep=$(sweep_name "$sweep") region=$(region_id "$aggr") ok=1"

  echo "=== $name: $experiment, victim $(victim_name "$vic"), arena sram$(arena_id "$arenab"), stack sram$(stack_id "$stackb"), aggressor in $(region_name "$aggr"), $(sweep_name "$sweep") sweep"

  # the stack request goes first and on its own. a region that is already in use resets nothing, so
  # a run that keeps the same stack across rows pays the reboot once rather than per capture.
  if [ "$stackb" != "$LAST_STACK" ]; then
    send_bytes "$stackb"
    sleep "$REBOOT"
    LAST_STACK="$stackb"
  fi
  # the extra column is a string of console bytes for anything the six main knobs cannot express.
  # "-" sends the two bytes that put the board back in its default state.
  if [ "$extra" = "-" ]; then
    send_bytes "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" "$arenab" N R
  else
    send_bytes "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" "$arenab" $(echo "$extra" | sed 's/./& /g')
  fi
  sleep "$SETTLE"

  status="$(await_status "$want")"
  if [ -z "$status" ]; then
    echo "board never reported \"$want\" within ${CONFIRM}s, refusing to capture $name" >&2
    exit 1
  fi

  # the golden vector is the only end to end check this firmware has, and a stack page that landed
  # somewhere wrong would break the setup rather than the measurement. a capture is not taken
  # against a board that is no longer classifying the known window correctly.
  if [ "$vic" = i ] || [ "$vic" = v ]; then
    if [ -z "$(await_status "MATCH mismatch=0")" ]; then
      echo "golden check is not reading MATCH with a zero mismatch count, stopping before $name" >&2
      exit 1
    fi
  fi

  ./tools/stm32/capture.sh "$name" "$SECS"

  dir="$(ls -dt results/raw/*-"$name" | head -1)"

  # one image for the whole campaign. the hashes of every capture filed under it are compared
  # against each other rather than against a value computed here, so a rebuild between two resumed
  # halves is caught as well as one in the middle of a single run.
  local images
  images="$(grep -h '^image_sha256=' results/raw/*/build.txt 2>/dev/null | sort -u | wc -l)"
  if [ "$(grep -l "^campaign=$CAMPAIGN\$" results/raw/*/stress.txt 2>/dev/null | wc -l)" -gt 0 ]; then
    images="$(for d in $(grep -l "^campaign=$CAMPAIGN\$" results/raw/*/stress.txt | xargs -n1 dirname); do
                grep -h '^image_sha256=' "$d/build.txt"; done | sort -u | wc -l)"
    if [ "$images" -gt 1 ]; then
      echo "captures in this campaign carry more than one image hash, stopping after $name" >&2
      exit 1
    fi
  fi
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
    printf 'console_bytes=%s\n' "$stackb$vic$sweep$aggr$vreg$foot$loads$arenab$extra"
    printf 'arena_region=sram%s\n' "$(arena_id "$arenab")"
    printf 'stack_region=sram%s\n' "$(stack_id "$stackb")"
    printf 'descriptor_region=sram%s\n' "$did"
    # the image every capture of this campaign has to share, copied from the capture's own
    # build.txt rather than recomputed, and the configuration label the comparison is filed under.
    printf 'image_sha256=%s\n' "$(sed -n 's/^image_sha256=//p' "$dir/build.txt")"
    printf 'config=%s\n' "$(printf '%s' "$experiment" | sed 's/-o[nf]*$//')"
    # the address stream again rather than the knob: both of these are read back off the board's
    # own status line, not from the table that asked for them.
    printf 'stack_high_water=%s\n' "$(printf '%s' "$status" | tr ' ' '\n' | sed -n 's/^shw=//p')"
    printf 'stack_guard_hit=%s\n' "$(printf '%s' "$status" | tr ' ' '\n' | sed -n 's/^sguard=//p')"
    # the status line is sampled when the configuration is confirmed and the schedule is shuffled,
    # so this is whichever point happened to be active then and not the point any table reads. it
    # is kept under a name that says so, beside the whole point table of the sweep.
    printf 'aggressor_config_at_confirm=%s\n' "$(printf '%s' "$status" | tr ' ' '\n' | grep -E '^(chan|width|block|stride|hz)=' | tr '\n' ' ')"
    printf 'aggressor_sweep_points=%s\n' "$(sweep_points "$sweep")"
    printf 'status_line=%s\n' "$(printf '%s' "$status" | tr -d '\r')"
    # what the victim's address stream actually does, counted from the disassembly of the image that
    # ran rather than taken from the knob. only the read loop has one.
    if [ "$vic" = r ]; then
      ./bench/sweeps/victim_access_mix.py --elf build/target/laxity-u585.elf \
        --words "$words" --passes "$passes" \
        --buffer-region "$(region_name "$vreg")" --stack-region sram1
    elif [ "$vic" = i ]; then
      # inference has a data dependent call graph, so the static count the read loop gets is not
      # available. what is known from the code is which regions it touches at all.
      printf 'victim_access_mix=not counted for inference; regions touched: arena@sram%s stack@sram%s statics@sram3(88B) weights@flash(12256B)\n' \
        "$(arena_id "$arenab")" "$(stack_id "$stackb")"
    else
      printf 'victim_access_mix=not applicable, the victim is %s\n' "$(victim_name "$vic")"
    fi
  } > "$dir/stress.txt"
  echo "wrote $dir/stress.txt"
}

# name                        experiment     victim sweep aggressor victim-region footprint loads extra stack arena
TABLE="
stress-fn-gate-open-1         gate-open      i 0 a X C L - j 7
stress-fn-gate-open-2         gate-open      i 0 a X C L - k 7
stress-fn-gate-open-3         gate-open      i 0 b X C L - k 7
stress-fn-a-b1-default-off    default-off    i o c X C L - n 7
stress-fn-a-b1-default-on     default-on     i 0 c X C L - n 7
stress-fn-a-b1-placed-off     placed-off     i o c X C L - j 7
stress-fn-a-b1-placed-on      placed-on      i 0 c X C L - j 7
stress-fn-a-b2-default-off    default-off    i o c X C L - n 7
stress-fn-a-b2-default-on     default-on     i 0 c X C L - n 7
stress-fn-a-b2-placed-off     placed-off     i o c X C L - j 7
stress-fn-a-b2-placed-on      placed-on      i 0 c X C L - j 7
stress-fn-a-b3-default-off    default-off    i o c X C L - n 7
stress-fn-a-b3-default-on     default-on     i 0 c X C L - n 7
stress-fn-a-b3-placed-off     placed-off     i o c X C L - j 7
stress-fn-a-b3-placed-on      placed-on      i 0 c X C L - j 7
stress-fn-a-b4-default-off    default-off    i o c X C L - n 7
stress-fn-a-b4-default-on     default-on     i 0 c X C L - n 7
stress-fn-a-b4-placed-off     placed-off     i o c X C L - j 7
stress-fn-a-b4-placed-on      placed-on      i 0 c X C L - j 7
stress-fn-b-desc1             b-descriptor   i 0 c X C L P j 7
stress-fn-b-desc2             b-descriptor   i 0 c X C L Q j 7
stress-fn-b-desc3             b-descriptor   i 0 c X C L R j 7
stress-fn-b-desc4             b-descriptor   i 0 c X C L S j 7
stress-fn-c1-v1-s1           c1-shape       r o c X C L - j 7
stress-fn-c1-v2-s1           c1-shape       r o c Y C L - j 7
stress-fn-c1-v3-s1           c1-shape       r o c Z C L - j 7
stress-fn-c1-v1-s2           c1-shape       r o c X C L - k 7
stress-fn-c1-v2-s2           c1-shape       r o c Y C L - k 7
stress-fn-c1-v3-s2           c1-shape       r o c Z C L - k 7
stress-fn-c1-v1-s3           c1-shape       r o c X C L - n 7
stress-fn-c1-v2-s3           c1-shape       r o c Y C L - n 7
stress-fn-c1-v3-s3           c1-shape       r o c Z C L - n 7
stress-fn-c2-p04-s3          c2-window      r o c Z C l - j 7
stress-fn-c2-p04-s1          c2-window      r o c X C l - j 7
stress-fn-c2-p08-s3          c2-window      r o c Z B l - j 7
stress-fn-c2-p08-s1          c2-window      r o c X B l - j 7
stress-fn-c2-p16-s3          c2-window      r o c Z A l - j 7
stress-fn-c2-p16-s1          c2-window      r o c X A l - j 7
stress-fn-c2-p32-s3          c2-window      r o c Z A L - j 7
stress-fn-c2-p32-s1          c2-window      r o c X A L - j 7
stress-fn-gate-close-1        gate-close     i 0 a X C L - j 7
stress-fn-gate-close-2        gate-close     i 0 a X C L - k 7
stress-fn-gate-close-3        gate-close     i 0 b X C L - k 7
"

# a plain string rather than an array, since an empty array under set -u is an error in the bash
# that ships with macOS.
WANTED=" $* "
while read -r name experiment vic sweep aggr vreg foot loads extra stackb arenab; do
  [ -z "${name:-}" ] && continue
  if [ -n "$*" ]; then
    case "$WANTED" in *" $name "*) ;; *) continue ;; esac
  fi
  run_one "$name" "$experiment" "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" "${extra:--}" "${stackb:-j}" "${arenab:-7}"
done <<< "$TABLE"

echo "campaign done"
