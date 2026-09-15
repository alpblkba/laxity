#!/usr/bin/env bash
# build the boardless target with the production telemetry implementation.
set -euo pipefail
cd "$(dirname "$0")/../.."

OUT="${1:-build/laxity-sim}"
mkdir -p "$OUT"

cc -std=c11 -Wall -Wextra -Werror -O2 \
   -Iinclude -Itools/laxity-sim \
   runtime/telemetry.c tools/laxity-sim/scenarios.c tools/laxity-sim/main.c \
   -o "$OUT/laxity-sim"

echo "$OUT/laxity-sim"
