#!/usr/bin/env bash
# capture a telemetry run into the raw experiment tree. every run records its own metadata,
# since a measurement without its build configuration cannot be compared against anything later.
#
#   ./tools/stm32/capture.sh <run-name> <seconds>
#
# results/raw/ is append only and board access is limited, so everything that can refuse is
# checked before the directory is created. the earlier version wrote three metadata files and
# then aborted on a missing timeout(1), leaving a directory that looked like a capture.
set -euo pipefail
cd "$(dirname "$0")/../.."

NAME="${1:?run name required}"
SECS="${2:-60}"
BAUD="${LAXITY_BAUD:-921600}"
BUILD="${LAXITY_BUILD:-build/target}"
ELF="$BUILD/laxity-u585.elf"
DRAIN="${LAXITY_DRAIN:-2}"

# macOS ships no timeout(1). coreutils provides it and installs it under either name depending
# on how it was installed, so resolve it rather than assuming and refuse rather than running
# without a time limit.
TIMEOUT=""
for candidate in timeout gtimeout; do
  if command -v "$candidate" >/dev/null 2>&1; then TIMEOUT="$candidate"; break; fi
done
[ -n "$TIMEOUT" ] || { echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }

# the virtual COM port node number is assigned at enumeration, so ls | head -1 picks arbitrarily
# when a second usbmodem is present and writes an empty or wrong telemetry.bin beside correct
# looking metadata. STM32_Programmer_CLI already prints the serial, the description and the node
# together, so match on those. LAXITY_STLINK_SN picks one board when two are attached.
list_ports() {
  STM32_Programmer_CLI -l 2>/dev/null \
    | sed $'s/\033\\[[0-9;]*m//g' \
    | awk -v want="${LAXITY_STLINK_SN:-}" '
        function trim(s) { sub(/^[^:]*:[[:space:]]*/, "", s); sub(/[[:space:]]+$/, "", s); return s }
        /^ST-LINK SN[[:space:]]*:/  { serial = trim($0) }
        /^Location[[:space:]]*:/    { loc = trim($0) }
        /^Description[[:space:]]*:/ {
          desc = trim($0)
          if (desc ~ /STLINK/ && loc ~ /^\/dev\/cu\./ && (want == "" || want == serial)) {
            print serial "\t" loc
          }
          loc = ""; desc = ""
        }'
}

if [ -n "${LAXITY_PORT:-}" ]; then
  PORT="$LAXITY_PORT"
  SERIAL="override"
else
  # read the whole listing into a variable. a grep -q here would exit at the first match, kill
  # the CLI with SIGPIPE and let pipefail report no board while the board is attached, which is
  # the false negative block 1 paid for in doctor.sh.
  CANDIDATES="$(list_ports)"
  COUNT="$(printf '%s' "$CANDIDATES" | grep -c . || true)"
  if [ "$COUNT" -eq 0 ]; then
    echo "no ST-LINK virtual COM port found, refusing to capture" >&2
    exit 1
  fi
  if [ "$COUNT" -ne 1 ]; then
    echo "$COUNT ST-LINK virtual COM ports found, set LAXITY_STLINK_SN to choose one:" >&2
    printf '%s\n' "$CANDIDATES" >&2
    exit 1
  fi
  SERIAL="$(printf '%s' "$CANDIDATES" | cut -f1)"
  PORT="$(printf '%s' "$CANDIDATES" | cut -f2)"
fi
[ -c "$PORT" ] || { echo "$PORT is not a character device" >&2; exit 1; }

[ -f "$ELF" ] || { echo "no build at $ELF, run ./tools/stm32/build.sh first" >&2; exit 1; }
CC_JSON="$BUILD/compile_commands.json"
[ -f "$CC_JSON" ] || { echo "no $CC_JSON, the optimisation level cannot be recorded" >&2; exit 1; }

# docs/EXPERIMENTS.md requires the optimisation level with every data point, and CMakeCache.txt
# is the wrong place to read it: the cache says CMAKE_C_FLAGS_DEBUG=-g while every object is
# actually compiled with -O0, because cmake/gcc-arm-none-eabi.cmake sets that variable as a
# normal variable which shadows the cache entry. the compile database is what the compiler saw.
OPT="$(grep -o -- '-O[0-9a-zA-Z]*' "$CC_JSON" | sort -u | tr '\n' ' ')"
[ -n "$OPT" ] && OPT="${OPT% }" || OPT="none (gcc defaults to -O0)"
TYPE="$(sed -n 's/^CMAKE_BUILD_TYPE:STRING=//p' "$BUILD/CMakeCache.txt" 2>/dev/null || true)"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
DIR="results/raw/${STAMP}-${NAME}"
[ -e "$DIR" ] && { echo "$DIR already exists and results/raw is append only" >&2; exit 1; }

mkdir -p "$DIR"
./tools/stm32/versions.sh > "$DIR/toolchain.txt"
git rev-parse HEAD > "$DIR/commit.txt"
git status --porcelain > "$DIR/dirty.txt"

{
  printf 'build_type=%s\n' "${TYPE:-unset}"
  printf 'opt=%s\n' "$OPT"
  printf 'elf=%s\n' "$ELF"
  printf 'elf_sha256=%s\n' "$(shasum -a 256 "$ELF" | cut -d' ' -f1)"
  arm-none-eabi-size "$ELF" | tail -1 | awk '{printf "text=%s\ndata=%s\nbss=%s\n", $1, $2, $3}'
  # the activation arena address is chosen by the linker in this baseline and moves whenever bss
  # changes, so a placement comparison has to read it per capture rather than assume block 4's value.
  arm-none-eabi-nm -S "$ELF" | awk '$4 == "laxity_activations" { printf "arena_addr=0x%s\narena_size=0x%s\n", $1, $2 }'
} > "$DIR/build.txt"

# hold the port open while stty runs. termios settings live only as long as the device is open,
# so setting the speed in one process and reading in the next leaves the reader at the default
# 9600 and the ST-LINK forwards a handful of framing errors instead of the stream.
exec 3<"$PORT"
stty -f "$PORT" "$BAUD" raw -echo

# the virtual COM port buffers thousands of bytes while nothing is reading and the backlog
# survives closing and reopening the port, measured in block 4. draining first shortens the
# stale prefix; the parser discarding everything before the first header frame is what makes
# the capture correct either way.
"$TIMEOUT" "$DRAIN" cat "$PORT" >/dev/null 2>&1 || true
"$TIMEOUT" "$SECS" cat "$PORT" > "$DIR/telemetry.bin" || true
exec 3<&-

printf 'run=%s\nport=%s\nstlink_sn=%s\nbaud=%s\nseconds=%s\ndrain_seconds=%s\nutc=%s\nbytes=%s\n' \
  "$NAME" "$PORT" "$SERIAL" "$BAUD" "$SECS" "$DRAIN" "$STAMP" \
  "$(wc -c < "$DIR/telemetry.bin" | tr -d ' ')" > "$DIR/run.txt"
echo "captured to $DIR"
