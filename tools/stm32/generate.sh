#!/usr/bin/env bash
# regen the firmware project from the .ioc using STM32CubeMX in script  mode, so hardware configuration stays reproducible from one source of truth.

# verified on this machine by the script mode invocation is -q <script file>.
# CubeMX returns 0 even after refusing a command, so the exit status carries no information and success has to be established from the output and the tree.
set -euo pipefail
LAXITY_STM32_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$LAXITY_STM32_DIR/lib.sh"
cd "$LAXITY_STM32_DIR/../.."

CUBEMX="/Applications/STMicroelectronics/STM32CubeMX.app/Contents/MacOs/STM32CubeMX"

# script mode still opens modal dialogs and waits on them, which is how one generate sat for eight minutes and forty six seconds, so refuse to run without a time limit.
TIMEOUT_BIN="$(laxity_timeout)" || { echo "no timeout(1) or gtimeout(1) on PATH, brew install coreutils" >&2; exit 1; }
LIMIT="${CUBEMX_TIMEOUT:-300}"
IOC="$PWD/firmware/stm32u585/laxity-u585.ioc"
LOG="$(mktemp -t cubemx-log)"
SCRIPT_FILE="$(mktemp -t cubemx)"

[ -f "$IOC" ] || { echo "no .ioc at $IOC" >&2; exit 1; }

cat > "$SCRIPT_FILE" <<EOS
config load $IOC
project generateunderroot 1
project generate
exit
EOS

rc=0
"$TIMEOUT_BIN" "$LIMIT" "$CUBEMX" -q "$SCRIPT_FILE" >"$LOG" 2>&1 || rc=$?
rm -f "$SCRIPT_FILE"

# 124 is timeout(1) killing it. a generate that finished normally took thirteen seconds, so reaching the limit means it is waiting on a dialog nobody can see.
if [ "$rc" -eq 124 ]; then
  echo "CubeMX did not finish within ${LIMIT}s, most likely a modal dialog. log kept at $LOG" >&2
  exit 1
fi

# KO catches a command CubeMX refused. the file check catches a run that started, printed nothing wrong, and produced nothing.
if grep -q '^KO' "$LOG"; then
  echo "CubeMX refused a command, log kept at $LOG" >&2
  exit 1
fi

# the CMake toolchain emits Src/ and Inc/ at the project root, not Core/. an mtime test does not work here since CubeMX regenerates into a temporary file and leaves the original untouched when the content is identical, 
# so an unchanged main.c stays older than the .ioc after a run that did everything right. the log naming each file is the evidence that generation actually reached it
for f in firmware/stm32u585/Src/main.c firmware/stm32u585/Inc/main.h; do
  [ -f "$f" ] || { echo "missing $f, log kept at $LOG" >&2; exit 1; }
  grep -Fq "Generated code: $PWD/$f" "$LOG" \
    || { echo "$f not reported as generated, log kept at $LOG" >&2; exit 1; }
done

rm -f "$LOG"
echo "generated from $IOC"
