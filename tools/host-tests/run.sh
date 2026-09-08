#!/usr/bin/env bash
# build and run the host checks, then feed the streams they produce to the parser.
#
# the parser is a second implementation of docs/TELEMETRY.md written in another language, so
# running it over a stream the C side produced is what tests the format rather than one
# implementation's agreement with itself.
set -euo pipefail
cd "$(dirname "$0")/../.."

OUT="${TMPDIR:-/tmp}/laxity-host-tests"
rm -rf "$OUT"
mkdir -p "$OUT"

cc -std=c11 -Wall -Wextra -Werror -O1 \
   -Iinclude -Iplatform/cortex-m33 \
   runtime/telemetry.c platform/cortex-m33/counters_dwt.c tools/host-tests/telemetry_test.c \
   -o "$OUT/telemetry_test"

"$OUT/telemetry_test" "$OUT"

fail=0
# the parser writes one key=value per line. reading it into a variable rather than piping into
# grep -q keeps the whole stream consumed, which is the false negative block 1 paid for.
expect() {
  local out="$1" key="$2" want="$3" got
  got=$(printf '%s\n' "$out" | sed -n "s/^${key}=//p")
  if [ "$got" != "$want" ]; then
    echo "FAIL $key=$got, expected $want"
    fail=1
  fi
}

good=$(python3 tools/telemetry-parse.py "$OUT/stream.bin" --csv "$OUT/stream.csv")
expect "$good" version 1
expect "$good" clock_hz 160000000
expect "$good" cyccnt_hz 159999450
expect "$good" stall_available 1
# block 3 measured that the counters exist and nothing fills stall_cyc from them yet, so a
# stream that claimed both would be describing a measurement this build does not make.
expect "$good" stall_populated 0
expect "$good" record_size 32
expect "$good" header_frames 2
expect "$good" batch_frames 2
expect "$good" records 36
expect "$good" dropped 4
expect "$good" gaps 1
expect "$good" wrapped 4
expect "$good" records_before_header 0
# three status lines carry the magic "LX" and none of them survives the CRC, which is the
# resync the mixed stream depends on rather than an incidental property of this data.
expect "$good" false_sync 3

# one byte of the first header payload is flipped, so that frame must fail its CRC and the
# batch behind it must be reported as records that arrived without a header rather than parsed.
bad=$(python3 tools/telemetry-parse.py "$OUT/corrupt.bin")
expect "$bad" header_frames 1
expect "$bad" records 4
expect "$bad" records_before_header 32
expect "$bad" false_sync 4

lines=$(wc -l < "$OUT/stream.csv")
[ "$lines" -eq 37 ] || { echo "FAIL csv had $lines lines, expected 37"; fail=1; }

[ "$fail" -eq 0 ] && echo "parser ok" || { echo "parser checks failed"; exit 1; }
