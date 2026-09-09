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
LAXITY_STM32_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$LAXITY_STM32_DIR/lib.sh"
cd "$LAXITY_STM32_DIR/../.."

NAME="${1:?run name required}"
SECS="${2:-60}"
BAUD="${LAXITY_BAUD:-921600}"
BUILD="${LAXITY_BUILD:-build/target}"
ELF="$BUILD/laxity-u585.elf"
DRAIN="${LAXITY_DRAIN:-2}"

# refuse rather than running without a time limit.
TIMEOUT="$(laxity_timeout)" || { echo "no timeout(1) or gtimeout(1) on PATH, install coreutils" >&2; exit 1; }

if [ -n "${LAXITY_PORT:-}" ]; then
  PORT="$LAXITY_PORT"
  SERIAL="override"
else
  # ls | head -1 picks arbitrarily when a second usbmodem is present and writes an empty or wrong
  # telemetry.bin beside correct looking metadata, so the port comes from the enumeration in
  # lib.sh. exactly one candidate is this script's rule and not the library's.
  CANDIDATES="$(laxity_stlink_ports)"
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

# every data point carries its optimisation level, and CMakeCache.txt
# is the wrong place to read it: the cache says CMAKE_C_FLAGS_DEBUG=-g while every object is
# actually compiled with -O0, because cmake/gcc-arm-none-eabi.cmake sets that variable as a
# normal variable which shadows the cache entry. the compile database is what the compiler saw.
OPT="$(grep -o -- '-O[0-9a-zA-Z]*' "$CC_JSON" | sort -u | tr '\n' ' ')"
[ -n "$OPT" ] && OPT="${OPT% }" || OPT="none (gcc defaults to -O0)"
TYPE="$(sed -n 's/^CMAKE_BUILD_TYPE:STRING=//p' "$BUILD/CMakeCache.txt" 2>/dev/null || true)"

MAIN="firmware/stm32u585/Src/main.c"
[ -f "$MAIN" ] || { echo "no $MAIN, the clock configuration cannot be recorded" >&2; exit 1; }
SYSCLK="${LAXITY_SYSCLK_HZ:-160000000}"

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
  # the ELF hash is not reproducible. a clean rebuild of identical sources reorders .debug_str and
  # shifts every section header after it, so the file differs by 48 bytes while every allocated
  # section, every symbol and the flashed image are identical. the image is what ran and what a
  # rebuild reproduces, so it is the hash that ties a capture to something anyone can recreate.
  # objcopy seeks its output, so it writes nothing to a pipe and silently yields the hash of an
  # empty file. a temporary file is the only form that produces the image.
  IMG="$(mktemp -t laxity-image)"
  arm-none-eabi-objcopy -O binary "$ELF" "$IMG"
  printf 'image_sha256=%s\n' "$(shasum -a 256 "$IMG" | cut -d' ' -f1)"
  rm -f "$IMG"
  arm-none-eabi-size "$ELF" | tail -1 | awk '{printf "text=%s\ndata=%s\nbss=%s\n", $1, $2, $3}'
  # the arenas are carved out of one span reservation, so what the ELF can say is where that
  # reservation landed. the per region arena addresses are in the header frame of the capture
  # itself, since they are a run time fact.
  arm-none-eabi-nm -S "$ELF" | awk '$4 == "laxity_arena_span" { printf "span_addr=0x%s\nspan_size=0x%s\n", $1, $2 }'

  # a reported entry needs the clock configuration, the flash wait states and the voltage
  # scaling range. they are read out of the generated SystemClock_Config() rather than out of the
  # .ioc, since the .ioc stores the request and the generated code is what runs.
  printf 'board=%s\n' "$(sed -n 's/^board = "\(.*\)"/\1/p' toolchain.toml)"
  printf 'sysclk_hz=%s\n' "$SYSCLK"
  printf 'flash_latency=%s\n' "$(grep -oE 'FLASH_LATENCY_[0-9]+' "$MAIN" | sort -u | tr '\n' ' ' | sed 's/ $//')"
  printf 'voltage_scale=%s\n' "$(grep -oE 'PWR_REGULATOR_VOLTAGE_SCALE[0-9]+' "$MAIN" | sort -u | tr '\n' ' ' | sed 's/ $//')"
  printf 'icache=%s\n' "$(grep -c 'HAL_ICACHE_Enable' "$MAIN" | awk '{print ($1 > 0) ? "enabled" : "disabled"}')"
} > "$DIR/build.txt"

# hold the port open while stty runs. termios settings live only as long as the device is open,
# so setting the speed in one process and reading in the next leaves the reader at the default
# 9600 and the ST-LINK forwards a handful of framing errors instead of the stream.
exec 3<"$PORT"
stty -f "$PORT" "$BAUD" raw -echo

# the virtual COM port buffers thousands of bytes while nothing is reading and the backlog
# survives closing and reopening the port, measured rather than assumed. draining first shortens the
# stale prefix; the parser discarding everything before the first header frame is what makes
# the capture correct either way.
"$TIMEOUT" "$DRAIN" cat "$PORT" >/dev/null 2>&1 || true
"$TIMEOUT" "$SECS" cat "$PORT" > "$DIR/telemetry.bin" || true
exec 3<&-

printf 'run=%s\nport=%s\nstlink_sn=%s\nbaud=%s\nseconds=%s\ndrain_seconds=%s\nutc=%s\nbytes=%s\n' \
  "$NAME" "$PORT" "$SERIAL" "$BAUD" "$SECS" "$DRAIN" "$STAMP" \
  "$(wc -c < "$DIR/telemetry.bin" | tr -d ' ')" > "$DIR/run.txt"
echo "captured to $DIR"
