#!/usr/bin/env bash
# the placement determinism campaign, one board session per arm.
#
#   ./bench/sweeps/placement_run.sh <arm> [arm ...]
#
# written but never executed. it was validated by reading campaign_run.sh and
# tools/stm32/lib.sh and by bash -n, and by nothing else. no cell of it has been
# run against a board, so every timing constant below is inherited from
# campaign_run.sh rather than measured here.
#
# with no argument it prints usage and does nothing, since every arm here is a
# long board session and none of them should start by accident.
#
# what this campaign measures is where the measurement thread's stack lands when
# it comes from the ThreadX byte pool instead of from one of the three fixed
# pages, and what a block taken out of that pool ahead of it does to the answer.
# the console bytes for both are new: p selects the vendor default stack, and f,
# g, h, m and q select a ballast of 0, 64, 256, 1024 and 4096 bytes. each one
# resets the board, because a thread's stack is fixed when the thread is created
# and the ballast has to be taken before that.
#
# the health arm exists for a bench condition rather than for the campaign. the
# board's JP3 IDD jumper was bent and the target read 0.00 V for a day, so the
# first thing wanted after the repair is two identical captures under DMA load,
# which is where an intermittent supply shows up and idle does not. its two
# numbers belong to this image alone and comparing them against a capture taken
# with any earlier image compares two binaries and not two board states.
set -euo pipefail
cd "$(dirname "$0")/../.."
. tools/stm32/lib.sh

# the UART carries framed binary telemetry records interleaved with the human
# readable status lines, so a line matched out of that stream can hold bytes
# that are not text. macOS tr answers "Illegal byte sequence" and dies on a byte
# that is not valid in the current locale, which is what killed the first health
# run: the grep ahead of it had been told the stream was bytes and the tr after
# it had not, so it died, await_line answered empty, and the caller read that as
# the line never having arrived.
#
# every filter in this script reads that same stream, so the locale belongs to
# the script rather than to individual pipelines. the explicit LC_ALL=C prefixes
# further down are redundant once this is set and they stay, because they carry
# the reason at the point where it applies.
export LC_ALL=C

SECS="${LAXITY_SECS:-75}"
# long enough for the board to finish the pass it was in the middle of, which is
# the same 8 seconds campaign_run.sh uses for the same reason.
SETTLE="${LAXITY_SETTLE:-8}"
CONFIRM="${LAXITY_CONFIRM:-20}"
# this script reboots the board 80 times in arms a and b alone, so REBOOT is the
# largest single term in its wall clock rather than a detail: at the default it
# is 16 minutes of the run. the read that follows every reboot waits up to
# CONFIRM seconds for the board's line anyway, so lowering REBOOT trades wall
# clock against how much of the boot the serial read sits through, and it cannot
# be lowered past the point where the reset itself has not taken effect.
REBOOT="${LAXITY_REBOOT:-12}"
# the virtual COM port keeps a backlog while nothing is reading and the backlog
# survives closing and reopening the port, measured in capture.sh. a boot line
# read without draining first can therefore be the previous boot's line, which
# would be recorded as this boot's address and would be wrong in the one way
# this campaign cannot detect afterwards. what the drain throws away is a mix of
# stale and fresh lines rather than stale ones only, since laxity_export_entry
# prints the whole human readable set once per second and not once at boot,
# app_threadx.c around line 1591. discarding a fresh line costs nothing because
# the next second carries the same one, and that repetition is the whole reason
# a blind drain is safe here.
BOOTDRAIN="${LAXITY_BOOTDRAIN:-3}"

# fifteen boots per half of arm a, thirty in total, and the halves bracket arm b
# so that arm a is its own control against anything that moved during arm b.
BOOTS_A=15
# ten boots per ballast value, fifty in total.
BOOTS_B=10

# one directory for the whole campaign rather than one per cell, since the boot
# tables are not captures and nothing in results/raw's timestamped layout reads
# them. it sits under results/raw because it is append only experiment data and
# because that path is already in .gitignore.
CAMPAIGN_DIR="${LAXITY_CAMPAIGN_DIR:-results/raw/placement-determinism}"
CAMPAIGN=placement-determinism

# the ballast sweep, as console byte and byte count, in the order arm b runs
# them. the counts mirror laxity_ballast_sizes in the firmware and the board
# prints what it actually allocated, which is what the table records.
BALLASTS="f:0 g:64 h:256 m:1024 q:4096"

# the console byte for the vendor default stack, which is what arms a and b both
# measure. 15 is LAXITY_STACK_SRC_POOL as the board reports it.
POOL_BYTE=p
POOL_SRC=15

TIMEOUT="$(laxity_timeout)" || {
  echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }
PORT="$(laxity_stlink_ports | cut -f2 | head -1)"
[ -c "$PORT" ] || { echo "no ST-LINK serial port" >&2; exit 1; }

usage() {
  cat <<'USAGE'
usage: ./bench/sweeps/placement_run.sh <arm> [arm ...]

  health     two captures of one fixed stress configuration, back to back
  a-first    fifteen boots on the vendor default stack, before arm b
  b          ten boots at each of five ballast sizes
  a-second   the other fifteen boots, after arm b
  all        a-first, then b, then a-second

  LAXITY_REBOOT   seconds waited after every reset, default 12. arms a and b
                  reboot 80 times, so this sets most of the wall clock.
  LAXITY_SECS     seconds per health capture, default 75
  LAXITY_SETTLE   seconds after the console bytes, default 8
  LAXITY_CONFIRM  seconds waited for a line from the board, default 20
USAGE
}

# the port settings live only while the device is open, so the descriptor is
# held across the stty and the writes. closing between them puts the line back
# to 9600 and the board sees noise.
send_bytes() {
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  local b
  for b in "$@"; do printf '%s' "$b" >&3; sleep 0.25; done
  exec 3>&-
}

# the first line the board prints that contains "$1", or nothing.
await_line() {
  local want="$1" line
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  # the stream carries framed binary records alongside the text status lines, so
  # grep has to treat it as text rather than refusing it as binary.
  line="$("$TIMEOUT" "$CONFIRM" cat "$PORT" 2>/dev/null \
          | LC_ALL=C grep -a -m1 -F "$want" || true)"
  exec 3>&-
  printf '%s' "$line" | tr -d '\r'
}

# one read window, answered as the infer line and then the placement line. both
# come out of the same window because the pass counter is a witness for the
# address recorded beside it, and one read later it would be a witness for a
# different second of the same boot. the exporter prints the set once per second
# in a fixed order with the infer line ahead of the placement line, so awk holds
# the first and prints the pair when the second arrives, which ends the read the
# way grep -m1 does.
await_boot_lines() {
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  "$TIMEOUT" "$CONFIRM" cat "$PORT" 2>/dev/null \
    | LC_ALL=C tr -d '\0' \
    | LC_ALL=C awk '
        /infer mode=/   { infer = $0 }
        /placement ok=/ { if (infer != "") { print infer; print $0; exit } }
      ' | tr -d '\r' || true
  exec 3>&-
}

# one space separated key=value out of a line the board printed, by exact key, so
# that ballast does not also answer for ballast_addr. awk reads the whole stream
# rather than stopping at the first match, since a stage that exits early leaves
# the one above it writing into a closed pipe and pipefail turns that into a
# failed pipeline. it is the fault lib.sh describes with grep -q.
field() {
  printf '%s' "$2" | tr ' ' '\n' \
    | awk -v k="$1" -F= '!seen && $1 == k { print substr($0, length(k) + 2); seen = 1 }'
}

src_name() {
  case "$1" in
    1) echo page-sram1 ;; 2) echo page-sram2 ;; 3) echo page-sram3 ;;
    "$POOL_SRC") echo pool ;; *) echo "unknown-$1" ;;
  esac
}

# the image in the build tree, which is what capture.sh records beside every
# capture. it cannot read what is actually in flash, so whether the board is
# running this image is a question the flash step answers and this one does not.
image_hash() {
  local elf=build/target/laxity-u585.elf img hash
  [ -f "$elf" ] || { echo "no build at $elf, run ./tools/stm32/build.sh" >&2; exit 1; }
  img="$(mktemp -t laxity-image)"
  arm-none-eabi-objcopy -O binary "$elf" "$img"
  hash="$(shasum -a 256 "$img" | cut -d' ' -f1)"
  rm -f "$img"
  printf '%s' "$hash"
}

# every cell of this campaign has to come from one binary, so the hash is pinned
# on the first cell and compared on every one after it. a rebuild in the middle
# is refused here rather than discovered when two tables disagree.
pin_image() {
  local now pinned
  now="$(image_hash)"
  mkdir -p "$CAMPAIGN_DIR"
  if [ -s "$CAMPAIGN_DIR/image.txt" ]; then
    pinned="$(cat "$CAMPAIGN_DIR/image.txt")"
    if [ "$pinned" != "$now" ]; then
      echo "the build tree holds $now and this campaign is pinned to $pinned," >&2
      echo "so nothing is run. the pin is in $CAMPAIGN_DIR/image.txt" >&2
      exit 1
    fi
  else
    printf '%s\n' "$now" > "$CAMPAIGN_DIR/image.txt"
  fi
  IMAGE="$now"
}

# put the board on a stack source and a ballast size, then read back what it
# says it is on. both bytes reset the board when they change something and do
# nothing at all when they do not, so each one is followed by a reboot wait and
# the configuration is confirmed from the board's own line rather than from the
# request.
select_config() {
  local src_byte="$1" ballast_byte="$2" want_ballast="$3" line got_src got_bal
  send_bytes "$src_byte"
  sleep "$REBOOT"
  send_bytes "$ballast_byte"
  sleep "$REBOOT"
  line="$(await_line 'placement ok=')"
  if [ -z "$line" ]; then
    echo "board printed no placement line within ${CONFIRM}s" >&2
    exit 1
  fi
  got_src="$(field stack_src "$line")"
  got_bal="$(field ballast "$line")"
  if [ "$got_src" != "$POOL_SRC" ] || [ "$got_bal" != "$want_ballast" ]; then
    echo "asked for stack_src=$POOL_SRC ballast=$want_ballast and the board says" >&2
    echo "stack_src=$got_src ballast=$got_bal, so nothing is recorded" >&2
    exit 1
  fi
}

# one reset, then one window of the board's status set.
boot_once() {
  # a hardware reset through the probe rather than a console byte, since the
  # console bytes only reset when they change something and this loop asks for
  # the same configuration at every boot.
  #
  # its status is not swallowed. a reset that did not happen leaves the board
  # running, and arm a would then record fifteen identical addresses and report
  # a determinism it never tested, which is a false negative on the question the
  # campaign exists to ask. nothing in the table afterwards separates that from
  # fifteen real boots that agreed.
  STM32_Programmer_CLI -c port=SWD mode=Hotplug -rst >/dev/null 2>&1 || return 1
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  # timeout answers 124 when it reaches its limit, which is what a drain that
  # ran for its whole window looks like, so this one is absorbed.
  "$TIMEOUT" "$BOOTDRAIN" cat "$PORT" >/dev/null 2>&1 || true
  exec 3>&-
  await_boot_lines
}

# n boots into one tab separated table. the table is written to a partial file
# and moved into place at the end, so an interrupted cell leaves nothing that
# the skip rule would read as a finished one, and the partial stays on disk.
boot_loop() {
  local arm="$1" cell="$2" want_ballast="$3" n="$4" out="$5"
  local i lines infer line src addr region bal baddr pass

  if [ -s "$out" ]; then
    echo "skip $arm $cell, already recorded in $out"
    return 0
  fi

  echo "=== $arm $cell: $n boots, stack from the byte pool, ballast $want_ballast"
  mkdir -p "$CAMPAIGN_DIR"
  printf '# campaign=%s image_sha256=%s\n' "$CAMPAIGN" "$IMAGE" > "$out.partial"
  printf '# utc\tarm\tcell\tboot\tsrc\tsrc_name\tstack_addr\tstack_region\tballast\tballast_addr\tpass\n' \
    >> "$out.partial"

  i=1
  while [ "$i" -le "$n" ]; do
    if ! lines="$(boot_once)"; then
      echo "the probe refused the reset at boot $i of $arm $cell, so the cell" >&2
      echo "stops here and $out.partial holds what it had" >&2
      exit 1
    fi
    infer="$(printf '%s\n' "$lines" | sed -n 1p)"
    line="$(printf '%s\n' "$lines" | sed -n 2p)"
    if [ -z "$line" ]; then
      echo "boot $i of $arm $cell printed no infer and placement pair within" >&2
      echo "${CONFIRM}s, so the cell stops and $out.partial holds what it had" >&2
      exit 1
    fi
    src="$(field stack_src "$line")"
    addr="$(field stack_addr "$line")"
    region="$(field stack_region "$line")"
    bal="$(field ballast "$line")"
    baddr="$(field ballast_addr "$line")"
    # the reboot witness. it is the pass counter this boot has reached, read out
    # of the same window as the address beside it. a reset that silently did not
    # happen leaves it climbing from boot to boot instead of restarting near
    # zero, and that is the only trace such a boot leaves in the table. it is
    # recorded and not judged: no threshold here and no verdict.
    pass="$(field pass "$infer")"
    printf '%s\t%s\t%s\t%d\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$(date -u +%Y%m%dT%H%M%SZ)" "$arm" "$cell" "$i" \
      "$src" "$(src_name "$src")" "$addr" "$region" "$bal" "$baddr" "$pass" \
      >> "$out.partial"
    echo "  boot $i/$n  stack=$addr region=$region ballast=$bal pass=$pass"
    # a boot that came back on another source or another ballast is not a boot of
    # this cell, and filing it under this cell would put two configurations in
    # one table. the line is on disk before the stop, since it is the evidence.
    if [ "$src" != "$POOL_SRC" ] || [ "$bal" != "$want_ballast" ]; then
      echo "boot $i came back with stack_src=$src ballast=$bal, not" >&2
      echo "stack_src=$POOL_SRC ballast=$want_ballast, so the cell stops here" >&2
      exit 1
    fi
    i=$((i + 1))
  done

  mv "$out.partial" "$out"
  echo "wrote $out"
}

arm_a() {
  local half="$1"
  out="$CAMPAIGN_DIR/arm-a-$half.tsv"
  if [ -s "$out" ]; then
    echo "skip arm a $half, already recorded in $out"
    return 0
  fi
  select_config "$POOL_BYTE" f 0
  boot_loop a "$half" 0 "$BOOTS_A" "$out"
}

arm_b() {
  local pair byte bytes out
  for pair in $BALLASTS; do
    byte="${pair%%:*}"
    bytes="${pair##*:}"
    out="$CAMPAIGN_DIR/arm-b-$bytes.tsv"
    if [ -s "$out" ]; then
      echo "skip arm b ballast $bytes, already recorded in $out"
      continue
    fi
    select_config "$POOL_BYTE" "$byte" "$bytes"
    boot_loop b "$bytes" "$bytes" "$BOOTS_B" "$out"
  done
}

# one capture read three ways: how many times the sequence number went backwards,
# how many records name an aggressor region other than the one the capture is
# filed under, and the median victim cycles at the standard point of the
# bandwidth sweep. it answers with those three fields separated by tabs, and the
# median reads "-" when the capture restarted.
#
# both counts come from bench/sweeps/campaign_report.py, which paid for them.
# the sequence number restarts at zero when the board resets, so a capture that
# spans one holds records from two configurations and the second half looks like
# a plausible measurement. a record whose aggressor region disagrees with the
# filing is a capture filed under a configuration the board was not in.
#
# this arm cannot average through a restart at all. it exists to find a marginal
# supply after the JP3 repair, and a brownout in the middle of a capture is
# exactly the shape that failure takes, so a median across one would average the
# thing being looked for into the thing it is being compared against.
capture_stats() {
  python3 - "$1" <<'PY'
import pathlib
import statistics
import sys

sys.path.insert(0, "tools")
import telemetry_parse

REGION_ID = {"sram1": 1, "sram2": 2, "sram3": 3, "sram4": 4}

d = pathlib.Path(sys.argv[1])
kv = {}
# stress.txt can hold bytes that came off a stream carrying framed records
# beside the text, so it is read as bytes and decoded the way the parser decodes
# that same stream, telemetry_parse.py line 148. text mode would raise on the
# first such byte whichever encoding the locale resolves to.
for ln in (d / "stress.txt").read_bytes().decode("ascii", "replace").splitlines():
    if "=" in ln:
        k, _, v = ln.partition("=")
        kv[k.strip()] = v.strip()

_, _, _, records = telemetry_parse.parse((d / "telemetry.bin").read_bytes())
if not records:
    raise SystemExit("no records in %s" % d)

restarts = sum(1 for a, b in zip(records, records[1:]) if b["seq"] < a["seq"])

want = REGION_ID[kv["aggressor_region"]]
wrong = sum(1 for r in records
            if ((r["aggressor_idx"] >> 8) & 0xFF) > 0
            and (r["aggressor_idx"] & 0xFF) != want)

median = "-"
if restarts == 0:
    # point 3 of the bandwidth sweep, one channel of width 4 moving 256 byte
    # blocks at 200 kHz, which is the point every comparison in this tree reads
    # and the only one in the capture under load at the configured rate.
    #
    # the high byte is the sweep point on the stress path and the footprint
    # index on the arena cross path, app_threadx.c lines 1358 and 1365, and
    # nothing in the record says which path wrote it. campaign_report.py does
    # not separate them either. that is a property of this tree rather than of
    # this script, and it is left as it is.
    cyc = [r["exec_cyc"] for r in records
           if ((r["aggressor_idx"] >> 8) & 0xFF) == 3]
    if not cyc:
        raise SystemExit("no records at point 3 in %s" % d)
    median = int(statistics.median(cyc))

print("%d\t%d\t%s" % (restarts, wrong, median))
PY
}

# one health capture. the console bytes are the ones stress-fn-gate-open-1 used,
# j i 0 a X C L 7 with N and R putting the two knobs it does not name back in
# their normal state, and both captures send the same sequence.
health_one() {
  local name="$1" dir status line

  # a capture killed part way through writing leaves a non-empty telemetry.bin
  # and no stress.txt, which is how stress-pr-gate-close-2 was lost, so both
  # have to be there before a cell counts as done.
  dir="$(ls -dt results/raw/*-"$name" 2>/dev/null | head -1 || true)"
  if [ -n "$dir" ] && [ -s "$dir/telemetry.bin" ] && [ -s "$dir/stress.txt" ]; then
    echo "skip $name, already captured in $dir" >&2
    printf '%s' "$dir"
    return 0
  fi

  send_bytes j
  sleep "$REBOOT"
  send_bytes i 0 a X C L 7 N R
  sleep "$SETTLE"

  status="$(await_line 'stress victim=infer-stress m2m=0 stack=1 arena=1')"
  if [ -z "$status" ]; then
    echo "board never reported the health configuration within ${CONFIRM}s" >&2
    exit 1
  fi
  # the ballast survives a reset in the same word the stack source does, so a
  # health cell run after arm b would otherwise carry whatever arm b left in it.
  # it is read rather than set, since the byte sequence for this arm is fixed.
  line="$(await_line 'placement ok=')"
  if [ -z "$line" ]; then
    echo "board printed no placement line within ${CONFIRM}s" >&2
    exit 1
  fi
  if [ "$(field ballast "$line")" != "0" ] || [ "$(field stack_src "$line")" != "1" ]; then
    echo "the board is on stack_src=$(field stack_src "$line")" >&2
    echo "ballast=$(field ballast "$line") and this arm is defined on the SRAM1" >&2
    echo "page with no ballast. send f and j, then run health again" >&2
    exit 1
  fi
  # the golden vector is the only end to end check this firmware has, and a
  # capture is not taken against a board that has stopped classifying the known
  # window correctly.
  if [ -z "$(await_line 'MATCH mismatch=0')" ]; then
    echo "golden check is not reading MATCH with a zero mismatch count" >&2
    exit 1
  fi

  ./tools/stm32/capture.sh "$name" "$SECS" >&2
  dir="$(ls -dt results/raw/*-"$name" | head -1)"

  # the sweep, the victim and the placement are not in the record. the wire
  # format was not changed for this campaign, so they go beside the capture, and
  # the board's own two lines go with them so the filing can be checked against
  # what the board said rather than against this script.
  {
    printf 'campaign=%s\n' "$CAMPAIGN"
    printf 'arm=health\n'
    printf 'victim=inference\n'
    printf 'victim_region=sram1\n'
    printf 'sweep=bw\n'
    printf 'aggressor_region=sram1\n'
    printf 'aggressor=gpdma_stress\n'
    printf 'channels_used=GPDMA1_12..15\n'
    printf 'console_bytes=ji0aXCL7NR\n'
    printf 'arena_region=sram1\n'
    printf 'stack_source=%s\n' "$(src_name "$(field stack_src "$line")")"
    printf 'stack_addr=%s\n' "$(field stack_addr "$line")"
    printf 'stack_region=sram%s\n' "$(field stack_region "$line")"
    printf 'ballast=%s\n' "$(field ballast "$line")"
    printf 'ballast_addr=%s\n' "$(field ballast_addr "$line")"
    printf 'image_sha256=%s\n' "$(sed -n 's/^image_sha256=//p' "$dir/build.txt")"
    printf 'stack_high_water=%s\n' "$(field shw "$status")"
    printf 'stack_guard_hit=%s\n' "$(field sguard "$status")"
    # sampled when the configuration was confirmed and the schedule is shuffled,
    # so this is whichever point was active then and not the point the median
    # below reads. it is kept under a name that says so.
    printf 'aggressor_config_at_confirm=%s\n' \
      "$(printf '%s' "$status" | tr ' ' '\n' \
         | grep -E '^(chan|width|block|stride|hz)=' | tr '\n' ' ')"
    printf 'aggressor_sweep_points=1:1/4/256/0/50000 2:1/4/256/0/100000 '
    printf '3:1/4/256/0/200000 4:1/4/256/0/400000 5:1/4/256/0/800000\n'
    # tab and printable ASCII only. what is dropped is the frame bytes that were
    # interleaved ahead of the match, which are an artifact of one UART carrying
    # the records and the text together rather than anything the board said. the
    # records themselves are in telemetry.bin, and these two fields exist to
    # check a capture against the name it was filed under, which the board's own
    # text does on its own.
    printf 'status_line=%s\n' \
      "$(printf '%s' "$status" | LC_ALL=C tr -cd '\11\40-\176')"
    printf 'placement_line=%s\n' \
      "$(printf '%s' "$line" | LC_ALL=C tr -cd '\11\40-\176')"
    printf 'victim_access_mix=not counted for inference; regions touched: '
    printf 'arena@sram1 stack@sram%s statics@sram3(88B) weights@flash(12256B)\n' \
      "$(field stack_region "$line")"
  } > "$dir/stress.txt"

  printf '%s' "$dir"
}

arm_health() {
  local d1 d2 s1 s2 r1 r2 w1 w2 m1 m2
  echo "=== health: two captures of i 0 a X C L - j 7, back to back"
  d1="$(health_one placement-health-1)"
  d2="$(health_one placement-health-2)"
  s1="$(capture_stats "$d1")"
  s2="$(capture_stats "$d2")"
  IFS=$'\t' read -r r1 w1 m1 <<< "$s1"
  IFS=$'\t' read -r r2 w2 m2 <<< "$s2"
  echo
  printf 'placement-health-1 restarts %s\n' "$r1"
  printf 'placement-health-1 wrong_region %s\n' "$w1"
  printf 'placement-health-2 restarts %s\n' "$r2"
  printf 'placement-health-2 wrong_region %s\n' "$w2"
  # a capture that spans a board reset carries two configurations, so neither a
  # median of it nor a difference against it is a number about one of them.
  if [ "$r1" != 0 ] || [ "$r2" != 0 ]; then
    echo "a health capture restarted, so no median is printed for it" >&2
    exit 1
  fi
  printf 'placement-health-1 median %s\n' "$m1"
  printf 'placement-health-2 median %s\n' "$m2"
  printf 'difference %s\n' "$((m2 - m1))"
}

[ "$#" -gt 0 ] || { usage; exit 2; }

pin_image
echo "image $IMAGE, campaign directory $CAMPAIGN_DIR"

for arm in "$@"; do
  case "$arm" in
    health)   arm_health ;;
    a-first)  arm_a first ;;
    a-second) arm_a second ;;
    b)        arm_b ;;
    all)      arm_a first; arm_b; arm_a second ;;
    *)        echo "unknown arm $arm" >&2; usage >&2; exit 2 ;;
  esac
done

echo "placement campaign done"
