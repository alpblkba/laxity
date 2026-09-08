#!/usr/bin/env bash
# verify the local environment before anything else runs. fails loudly, so a missing STM32Cube component surfaces here instead of as invented code later.
# macOS arm64 is the canonical host, linux is only expected in ci
set -uo pipefail

fail=0
ok()   { printf '  ok    %s\n' "$1"; }
miss() { printf '  MISS  %s\n' "$1"; fail=1; }
warn() { printf '  warn  %s\n' "$1"; }

CUBEMX_APP="/Applications/STMicroelectronics/STM32CubeMX.app/Contents/MacOs/STM32CubeMX"
CUBEIDE_APP="/Applications/STM32CubeIDE.app"
CUBE_REPO="${HOME}/STM32Cube/Repository"

echo "host"
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) ok "macOS arm64" ;;
  Linux/*)      warn "Linux host, expected only for CI" ;;
  *)            warn "untested host $(uname -s)/$(uname -m)" ;;
esac

echo "tools"
[ -x "$CUBEMX_APP" ] && ok "STM32CubeMX" || miss "STM32CubeMX at $CUBEMX_APP"
[ -d "$CUBEIDE_APP" ] && ok "STM32CubeIDE" || miss "STM32CubeIDE at $CUBEIDE_APP"
command -v arm-none-eabi-gcc >/dev/null && ok "arm-none-eabi-gcc $(arm-none-eabi-gcc -dumpversion)" \
                                        || miss "arm-none-eabi-gcc not on PATH"
command -v cmake >/dev/null   && ok "cmake $(cmake --version | head -1 | awk '{print $3}')" || miss "cmake"
command -v ninja >/dev/null   && ok "ninja"      || warn "ninja not found, cmake will fall back to make"
command -v STM32_Programmer_CLI >/dev/null && ok "STM32_Programmer_CLI" \
                                           || miss "STM32_Programmer_CLI not on PATH"
command -v stedgeai >/dev/null && ok "stedgeai" || miss "stedgeai not on PATH"

echo "cube packages"
[ -d "$CUBE_REPO" ] && ok "Cube repository at $CUBE_REPO" || miss "Cube repository at $CUBE_REPO"
for pat in "STM32Cube_FW_U5_*"; do
  found=$(ls -d "$CUBE_REPO"/$pat 2>/dev/null | tail -1 || true)
  [ -n "$found" ] && ok "$(basename "$found")" || miss "STM32CubeU5 firmware package"
done

echo "components expected by the firmware project"
FW="$(cd "$(dirname "$0")/../.." && pwd)/firmware/stm32u585"
for c in \
  "Drivers/BSP/B-U585I-IOT02A" \
  "Drivers/BSP/Components/ism330dhcx" \
  "Middlewares/ST/threadx" \
  "Middlewares/ST/netxduo" \
  "Middlewares/ST/STM32_Network_Library" ; do
  if [ -e "$FW/$c" ]; then ok "$c"
  else warn "$c not generated yet"; fi
done

echo "board"
if command -v STM32_Programmer_CLI >/dev/null; then
  # grep -q exits at the first match, the CLI dies of SIGPIPE with status 141, and pipefail turns that into a false "unplugged". grep -c reads it all.
  if STM32_Programmer_CLI -l 2>/dev/null | grep -c -i 'ST-LINK' >/dev/null; then ok "ST-LINK visible"
  else warn "no ST-LINK detected, board may be unplugged"; fi
fi

echo
[ "$fail" -eq 0 ] && echo "environment ok" || { echo "environment incomplete"; exit 1; }
