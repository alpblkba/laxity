#!/usr/bin/env bash
# enforce the portability contract. intention is not a mechanism.
set -euo pipefail
cd "$(dirname "$0")/.."

FORBIDDEN='stm32|b_u585i|tx_api|tx_port|stai_|ai_platform|main\.h'
GUARDED='include/qos runtime bench/kernels'
fail=0

for dir in $GUARDED; do
  [ -d "$dir" ] || continue
  if hits=$(grep -rnE "#include.*($FORBIDDEN)" "$dir" 2>/dev/null); then
    echo "layering violation in $dir:"; echo "$hits"; fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "Vendor headers belong behind include/qos/port/. See docs/ARCHITECTURE.md."
  exit 1
fi
echo "layering ok"
