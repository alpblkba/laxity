#!/usr/bin/env bash
# build and exercise every checked-in scenario without a board or an STM32 SDK.
set -euo pipefail
cd "$(dirname "$0")/../.."

./tools/laxity-sim/build.sh >/dev/null
python3 tools/laxity-sim/self_test.py
