#!/usr/bin/env bash
# Capture a telemetry run into the raw experiment tree. Every run records its
# own metadata, since a measurement without its build configuration cannot be
# compared against anything later.
#
#   ./tools/stm32/capture.sh <run-name> <seconds>
set -euo pipefail
cd "$(dirname "$0")/../.."

NAME="${1:?run name required}"
SECS="${2:-60}"
PORT="${LAXITY_PORT:-$(ls /dev/cu.usbmodem* 2>/dev/null | head -1)}"
BAUD="${LAXITY_BAUD:-921600}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
DIR="results/raw/${STAMP}-${NAME}"

mkdir -p "$DIR"
./tools/stm32/versions.sh > "$DIR/toolchain.txt"
git rev-parse HEAD > "$DIR/commit.txt"
git status --porcelain > "$DIR/dirty.txt"

stty -f "$PORT" "$BAUD" raw -echo
timeout "$SECS" cat "$PORT" > "$DIR/telemetry.bin" || true

printf 'run=%s\nport=%s\nbaud=%s\nseconds=%s\nutc=%s\n' \
  "$NAME" "$PORT" "$BAUD" "$SECS" "$STAMP" > "$DIR/run.txt"
echo "captured to $DIR"
