#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ELF="${1:-build/target/laxity-u585.elf}"

# with one probe attached the programmer picks it on its own. with two it picks one of them, and
# which one is not something the caller can see afterwards, so the serial is passed through when
# the caller named one. LAXITY_STLINK_SN is the same variable lib.sh already narrows the port
# enumeration with, so one selection covers flashing and capturing.
CONN=(port=SWD mode=UR)
if [ -n "${LAXITY_STLINK_SN:-}" ]; then
  CONN+=("sn=$LAXITY_STLINK_SN")
fi

STM32_Programmer_CLI -c "${CONN[@]}" -w "$ELF" -rst
