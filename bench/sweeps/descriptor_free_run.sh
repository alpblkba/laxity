#!/usr/bin/env bash
# the twelve data coefficient cells again, with the DMA descriptor page out of the victim's way.
#
#   ./bench/sweeps/descriptor_free_run.sh [name ...]
#
# with no argument it runs every row of the table. with names it runs only those, which is how a single cell is repeated without touching the rest. a row whose capture directory already holds a non-empty telemetry.bin and a stress.txt without a void line is skipped, so an interrupted campaign continues where it stopped rather than starting over.
#
# self-docs/ARENA-OR-STACK-2026-09-16.md measured the data coefficients over twelve cells with the descriptor page in SRAM3 in every capture. six of those cells have a victim object in SRAM3, so they carry a descriptor fetch term on top of the data term and the two were never separated, which is why 0.116 and 0.151 are means across configurations in the characterisation rather than values that can be keyed by a region.
#
# this campaign runs the same twelve cells with the page in SRAM4, where no victim object ever sits. self-docs/CLOSING-2026-09-19.md experiment 1 reads the descriptor term at 0.001 with the page in SRAM4 against 0.020 with it in SRAM3, which is why SRAM4 is where it goes.
#
# this script produces captures and medians. it fits no coefficient, compares nothing against the earlier campaign's numbers and writes nothing under self-docs or profiles.
set -euo pipefail
cd "$(dirname "$0")/../.."
. tools/stm32/lib.sh

# the UART carries framed binary telemetry records interleaved with the human readable status lines, so a line matched out of that stream can hold bytes that are not text. macOS tr answers "Illegal byte sequence" and dies on a byte that is not valid in the current locale, and the filters that take a confirmed status line apart run on exactly such a line. the grep that produced it already carries -a and LC_ALL=C for the same reason, so the locale belongs to the script rather than to individual pipelines.
export LC_ALL=C

SECS="${LAXITY_SECS:-75}"
# long enough for the board to finish the pass it was in the middle of. the arena cross pass is 328 inferences at 50 Hz, which is 6.6 seconds, and it is the longest one the board can be in.
SETTLE="${LAXITY_SETTLE:-8}"
CONFIRM="${LAXITY_CONFIRM:-20}"
# the stack region is fixed when the thread is created, so a change to it resets the board and the rest of the configuration has to wait for the board to come back. bytes that arrive during boot are dropped, which is how the first attempt at this class of run lost its victim byte.
REBOOT="${LAXITY_REBOOT:-12}"
LAST_STACK=""
CAMPAIGN=descriptor-free-2026-09-26
# the image pin lives beside the campaign rather than inside a capture, since it has to exist before the first capture does.
CAMPAIGN_DIR="${LAXITY_CAMPAIGN_DIR:-results/raw/$CAMPAIGN}"

# the page this campaign exists to move, and the only knob it turns. a cell that ran with the page anywhere else is worthless and looks correct in the output, so it is checked before every capture.
DESC_ID=4
DESC_ADDR=0x28003000

TIMEOUT="$(laxity_timeout)" || { echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }
PORT="$(laxity_stlink_ports | cut -f2 | head -1)"
[ -c "$PORT" ] || { echo "no ST-LINK serial port" >&2; exit 1; }

# the console byte alphabet, from laxity_poll_console() in firmware/stm32u585/Src/app_threadx.c.
sweep_name()  { case "$1" in 0) echo bw ;; 1) echo xact ;; 2) echo chan ;; 3) echo stride ;; 4) echo sat ;;
                             5) echo low ;; 6) echo dmat ;; o) echo none ;; esac; }
region_name() { case "$1" in a|X) echo sram1 ;; b|Y) echo sram2 ;; c|Z) echo sram3 ;; d) echo sram4 ;; esac; }
region_id()   { case "$1" in a|X) echo 1 ;; b|Y) echo 2 ;; c|Z) echo 3 ;; d) echo 4 ;; esac; }
victim_name() { case "$1" in r) echo read_loop ;; i) echo inference ;; t) echo dma_only ;; esac; }
# the descriptor page each region keeps, from laxity_place() in the firmware. the status line carries the address, so what is waited for is the page rather than the request.
desc_addr()   { case "$1" in 1) echo 0x2000e000 ;; 2) echo 0x2003e000 ;; 3) echo 0x2004b000 ;;
                             4) echo 0x28003000 ;; esac; }
desc_id()     { case "$1" in *P*) echo 1 ;; *Q*) echo 2 ;; *S*) echo 4 ;; *) echo 3 ;; esac; }
stack_id()    { case "$1" in j) echo 1 ;; k) echo 2 ;; n) echo 3 ;; esac; }
# the point table of each sweep, mirroring laxity_sweeps in the firmware, as index:channels/width/block/stride/triggerHz. a capture that carries this describes every configuration it contains, which the confirmation time snapshot below does not.
sweep_points() {
  case "$1" in
    0) echo "1:1/4/256/0/50000 2:1/4/256/0/100000 3:1/4/256/0/200000 4:1/4/256/0/400000 5:1/4/256/0/800000" ;;
    o) echo "none, no channel is started at any point" ;;
    *) echo "unlisted sweep $1, this campaign runs only the bandwidth sweep" ;;
  esac
}
arena_id()    { case "$1" in 7) echo 1 ;; 8) echo 2 ;; 9) echo 3 ;; esac; }
foot_bytes()  { case "$1" in A) echo 1024 ;; B) echo 2048 ;; C) echo 4096 ;; D) echo 8192 ;;
                             E) echo 16384 ;; F) echo 32768 ;; G) echo 65536 ;; H) echo 131072 ;; esac; }
load_count()  { case "$1" in L) echo 8192 ;; l) echo 4096 ;; esac; }
# the read loop window each region has, from LAXITY_VICTIM_BYTES_S1 to _S3 in the firmware. the footprint is clamped to it on the board, so the host has to clamp the same way or the line it waits for is one the board will never print.
window_bytes() { case "$1" in X) echo 131072 ;; Y) echo 4096 ;; Z) echo 4096 ;; esac; }

send_bytes() {
  # the port settings live only while the device is open, so the descriptor is held across the stty and the writes. closing between them puts the line back to 9600 and the board sees noise.
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  local b
  for b in "$@"; do printf '%s' "$b" >&3; sleep 0.25; done
  exec 3>&-
}

# wait for the board to say it is in the configuration this capture is about to be filed under. a sleep alone cannot tell a pass boundary from a missed byte, and fifty seconds of records filed under the wrong knobs is worse than a failed run.
await_status() {
  local want="$1" line
  exec 3<>"$PORT"
  stty -f "$PORT" 921600 raw -echo
  # the stream carries framed binary records alongside the text status lines, so grep has to treat it as text rather than refusing it as binary.
  line="$("$TIMEOUT" "$CONFIRM" cat "$PORT" 2>/dev/null | LC_ALL=C grep -a -m1 -F "$want" || true)"
  exec 3>&-
  printf '%s' "$line"
}

# one space separated key=value out of a line the board printed, by exact key. awk reads the whole stream rather than stopping at the first match, since a stage that exits early leaves the one above it writing into a closed pipe and pipefail turns that into a failed pipeline.
field() {
  printf '%s' "$2" | tr ' ' '\n' \
    | awk -v k="$1" -F= '!seen && $1 == k { print substr($0, length(k) + 2); seen = 1 }'
}

# the image in the build tree, which is what capture.sh records beside every capture. it cannot read what is actually in flash, so whether the board is running this image is a question the flash step answers and this one does not.
image_hash() {
  local elf=build/target/laxity-u585.elf
  local img hash
  [ -f "$elf" ] || { echo "no build at $elf, run ./tools/stm32/build.sh" >&2; exit 1; }
  img="$(mktemp -t laxity-image)"
  arm-none-eabi-objcopy -O binary "$elf" "$img"
  hash="$(shasum -a 256 "$img" | cut -d' ' -f1)"
  rm -f "$img"
  printf '%s' "$hash"
}

# every cell of this campaign has to come from one binary, so the hash is pinned on the first cell and compared on every one after it. a rebuild in the middle is refused here rather than discovered when two tables disagree.
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

# one capture read three ways: how many times the sequence number went backwards, how many records name an aggressor region other than the one the capture is filed under, and the median victim cycles at the standard point of the bandwidth sweep. it answers with those three fields separated by tabs, and the median reads "-" when the capture restarted.
#
# both counts come from bench/sweeps/campaign_report.py, which paid for them. the sequence number restarts at zero when the board resets, so a capture that spans one holds records from two configurations and the second half looks like a plausible measurement. a record whose aggressor region disagrees with the filing is a capture filed under a configuration the board was not in.
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
# stress.txt can hold bytes that came off a stream carrying framed records beside the text, so it is read as bytes and decoded the way the parser decodes that same stream, telemetry_parse.py line 148. text mode would raise on the first such byte whichever encoding the locale resolves to.
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
    # point 3 of the bandwidth sweep, one channel of width 4 moving 256 byte blocks at 200 kHz, which is the point every comparison in this tree reads and the only one in the capture under load at the configured rate.
    #
    # the high byte is the sweep point on the stress path and the footprint index on the arena cross path, app_threadx.c lines 1358 and 1365, and nothing in the record says which path wrote it. campaign_report.py does not separate them either. that is a property of this tree rather than of this script, and it is left as it is.
    cyc = [r["exec_cyc"] for r in records
           if ((r["aggressor_idx"] >> 8) & 0xFF) == 3]
    if not cyc:
        raise SystemExit("no records at point 3 in %s" % d)
    median = int(statistics.median(cyc))

print("%d\t%d\t%s" % (restarts, wrong, median))
PY
}

# the four lines every cell of this campaign reports, all of them out of capture_stats.
cell_report() {
  local name="$1" dir="$2"
  local stats r w m
  stats="$(capture_stats "$dir")"
  IFS=$'\t' read -r r w m <<< "$stats"
  printf '%s dir %s\n' "$name" "$dir"
  printf '%s restarts %s\n' "$name" "$r"
  printf '%s wrong_region %s\n' "$name" "$w"
  printf '%s median %s\n' "$name" "$m"
}

# the capture directory one row produced, newest first, or nothing.
cell_dir() {
  ls -dt results/raw/*-"$1" 2>/dev/null | head -1 || true
}

run_one() {
  local name="$1" experiment="$2" vic="$3" sweep="$4" aggr="$5" vreg="$6" foot="$7" loads="$8"
  local extra="${9:-NS}"
  local stackb="${10:-j}" arenab="${11:-7}"
  local dir words passes want status did

  # a capture whose stress.txt carries a void line does not count as done, so a row that produced an unusable capture is taken again under a new timestamp. the unusable one stays on disk with its reason beside it, since results/raw is append only. stress.txt has to be there as well, because a capture killed part way through writing leaves a telemetry.bin and no stress.txt and the void check would then read nothing at all.
  dir="$(cell_dir "$name")"
  if [ -n "$dir" ] && [ -s "$dir/telemetry.bin" ] && [ -s "$dir/stress.txt" ] &&
     ! grep -q '^void=' "$dir/stress.txt"; then
    echo "skip $name, already captured in $dir"
    cell_report "$name" "$dir"
    return 0
  fi

  did="$(desc_id "$extra")"
  if [ "$did" != "$DESC_ID" ]; then
    echo "$name asks for descriptors in region $did and this campaign is the SRAM4 one," >&2
    echo "so nothing is captured. the extra column has to carry S" >&2
    exit 1
  fi

  # the same arithmetic the measurement thread does at the top of every pass, so the status line can be matched exactly rather than approximately.
  words=$(( $(foot_bytes "$foot") / 4 ))
  if [ "$words" -gt $(( $(window_bytes "$vreg") / 4 )) ]; then
    words=$(( $(window_bytes "$vreg") / 4 ))
  fi
  passes=$(( $(load_count "$loads") / words ))
  [ "$passes" -eq 0 ] && passes=1

  local vname
  case "$vic" in r) vname=read ;; t) vname=dma ;; *) vname=infer-stress ;; esac
  want="stress victim=$vname"
  want="$want m2m=0"
  # the stack region and the descriptor page are part of what a capture is filed under, since both decide where the address stream goes and neither is visible in a record.
  want="$want stack=$(stack_id "$stackb") arena=$(arena_id "$arenab") desc=$did($(desc_addr "$did"))"
  want="$want vregion=$(region_id "$vreg") vwords=$words vpasses=$passes vloads=$(( words * passes ))"
  want="$want sweep=$(sweep_name "$sweep") region=$(region_id "$aggr") ok=1"

  echo "=== $name: $experiment, victim $(victim_name "$vic"), arena sram$(arena_id "$arenab"), stack sram$(stack_id "$stackb"), aggressor in $(region_name "$aggr"), descriptors sram$did"

  # the stack request goes first and on its own. a source that is already in use resets nothing, so a run that keeps the same stack across rows pays the reboot once rather than per capture.
  if [ "$stackb" != "$LAST_STACK" ]; then
    send_bytes "$stackb"
    sleep "$REBOOT"
    LAST_STACK="$stackb"
  fi
  send_bytes "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" "$arenab" $(echo "$extra" | sed 's/./& /g')
  sleep "$SETTLE"

  status="$(await_status "$want")"
  if [ -z "$status" ]; then
    echo "board never reported \"$want\" within ${CONFIRM}s, refusing to capture $name" >&2
    exit 1
  fi

  # the descriptor page again, off the line the board just printed rather than off the want string that matched it, so the failure names the one knob this campaign turns. a cell that ran with the old page is worthless and would look correct in the output, so this stops rather than warns.
  if [ "$(field desc "$status")" != "$did($DESC_ADDR)" ]; then
    echo "the board reports desc=$(field desc "$status") and this campaign needs" >&2
    echo "desc=$did($DESC_ADDR), so $name is not captured" >&2
    exit 1
  fi

  # the golden vector is the only end to end check this firmware has, and a capture is not taken against a board that is no longer classifying the known window correctly.
  if [ -z "$(await_status "MATCH mismatch=0")" ]; then
    echo "golden check is not reading MATCH with a zero mismatch count, stopping before $name" >&2
    exit 1
  fi

  ./tools/stm32/capture.sh "$name" "$SECS"

  dir="$(cell_dir "$name")"

  # one image for the whole campaign. the hashes of every capture filed under it are compared against each other rather than against a value computed here, so a rebuild between two resumed halves is caught as well as one in the middle of a single run.
  local images
  images="$(for d in $(grep -l "^campaign=$CAMPAIGN\$" results/raw/*/stress.txt 2>/dev/null | xargs -n1 dirname 2>/dev/null); do
              grep -h '^image_sha256=' "$d/build.txt"; done | sort -u | wc -l)"
  if [ "$images" -gt 1 ]; then
    echo "captures in this campaign carry more than one image hash, stopping after $name" >&2
    exit 1
  fi

  # the sweep, the victim and the placement are not in the record. the wire format was not changed for this campaign, so they go beside the capture, and the board's own status line goes with them so the filing can be checked against what the board said rather than against this table.
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
    # the region the board says the page is in, taken off the status line rather than off the table that asked for it, since the table is the request and the line is what happened.
    printf 'descriptor_region=sram%s\n' "$(field desc "$status" | sed 's/(.*//')"
    printf 'descriptor_addr=%s\n' "$(field desc "$status" | sed 's/^[0-9]*(//; s/)$//')"
    printf 'image_sha256=%s\n' "$(sed -n 's/^image_sha256=//p' "$dir/build.txt")"
    printf 'config=%s\n' "$experiment"
    # the address stream again rather than the knob: both of these are read back off the board's own status line, not from the table that asked for them.
    printf 'stack_high_water=%s\n' "$(field shw "$status")"
    printf 'stack_guard_hit=%s\n' "$(field sguard "$status")"
    # the status line is sampled when the configuration is confirmed and the schedule is shuffled, so this is whichever point happened to be active then and not the point any table reads. it is kept under a name that says so, beside the whole point table of the sweep.
    printf 'aggressor_config_at_confirm=%s\n' \
      "$(printf '%s' "$status" | tr ' ' '\n' | grep -E '^(chan|width|block|stride|hz)=' | tr '\n' ' ')"
    printf 'aggressor_sweep_points=%s\n' "$(sweep_points "$sweep")"
    # tab and printable ASCII only. what is dropped is the frame bytes that were interleaved ahead of the match, which are an artifact of one UART carrying the records and the text together rather than anything the board said.
    printf 'status_line=%s\n' "$(printf '%s' "$status" | LC_ALL=C tr -cd '\11\40-\176')"
    printf 'victim_access_mix=not counted for inference; regions touched: arena@sram%s stack@sram%s statics@sram3(88B) weights@flash(12256B)\n' \
      "$(arena_id "$arenab")" "$(stack_id "$stackb")"
  } > "$dir/stress.txt"
  echo "wrote $dir/stress.txt"
  cell_report "$name" "$dir"
}

# one gate cell read at both ends of the campaign, in cycles. the difference is printed rather than read, and it reads "-" when either end restarted, which is the placeholder capture_stats answers with.
gate_pair() {
  local label="$1"
  local dopen dclose so sc r w mo mc
  dopen="$(cell_dir "df-gate-open-$label")"
  dclose="$(cell_dir "df-gate-close-$label")"
  if [ -z "$dopen" ] || [ -z "$dclose" ]; then
    printf 'gate-%s open and close were not both captured\n' "$label"
    return 0
  fi
  so="$(capture_stats "$dopen")"
  sc="$(capture_stats "$dclose")"
  IFS=$'\t' read -r r w mo <<< "$so"
  IFS=$'\t' read -r r w mc <<< "$sc"
  printf 'gate-%s open median %s\n' "$label" "$mo"
  printf 'gate-%s close median %s\n' "$label" "$mc"
  if [ "$mo" = "-" ] || [ "$mc" = "-" ]; then
    printf 'gate-%s difference -\n' "$label"
  else
    printf 'gate-%s difference %s\n' "$label" "$(( mc - mo ))"
  fi
}

# name              experiment  victim sweep aggressor victim-region footprint loads extra stack arena
#
# the twelve cells are the six off diagonal arena and stack placements, each with the aggressor in the arena's region and then in the stack's region. the extra column is NS in every row, which sends N to hold the second memory to memory aggressor off and S to put the descriptor page in SRAM4, so the whole campaign sits on one descriptor placement including its gates.
#
# the second and third gate cells are the same configuration as df-a1-s2-ag1 and df-a1-s2-ag2, which gives those two a free repeat across the run rather than being a mistake. the first gate cell has the arena and the stack both in SRAM1 and is not one of the twelve.
TABLE="
df-gate-open-1    gate-open   i 0 a X C L NS j 7
df-gate-open-2    gate-open   i 0 a X C L NS k 7
df-gate-open-3    gate-open   i 0 b X C L NS k 7
df-a1-s2-ag1      cell        i 0 a X C L NS k 7
df-a1-s2-ag2      cell        i 0 b X C L NS k 7
df-a1-s3-ag1      cell        i 0 a X C L NS n 7
df-a1-s3-ag3      cell        i 0 c X C L NS n 7
df-a2-s1-ag1      cell        i 0 a X C L NS j 8
df-a2-s1-ag2      cell        i 0 b X C L NS j 8
df-a2-s3-ag2      cell        i 0 b X C L NS n 8
df-a2-s3-ag3      cell        i 0 c X C L NS n 8
df-a3-s1-ag1      cell        i 0 a X C L NS j 9
df-a3-s1-ag3      cell        i 0 c X C L NS j 9
df-a3-s2-ag2      cell        i 0 b X C L NS k 9
df-a3-s2-ag3      cell        i 0 c X C L NS k 9
df-gate-close-1   gate-close  i 0 a X C L NS j 7
df-gate-close-2   gate-close  i 0 a X C L NS k 7
df-gate-close-3   gate-close  i 0 b X C L NS k 7
"

pin_image
echo "image $IMAGE, campaign $CAMPAIGN"
echo

# a plain string rather than an array, since an empty array under set -u is an error in the bash that ships with macOS.
WANTED=" $* "
while read -r name experiment vic sweep aggr vreg foot loads extra stackb arenab; do
  [ -z "${name:-}" ] && continue
  if [ -n "$*" ]; then
    case "$WANTED" in *" $name "*) ;; *) continue ;; esac
  fi
  run_one "$name" "$experiment" "$vic" "$sweep" "$aggr" "$vreg" "$foot" "$loads" \
    "${extra:-NS}" "${stackb:-j}" "${arenab:-7}"
done <<< "$TABLE"

echo
gate_pair 1
gate_pair 2
gate_pair 3

echo
echo "campaign done"
