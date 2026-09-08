#!/usr/bin/env bash
# record exact tool versions for an experiment. output is plain text and is copied into every raw result directory.
set -uo pipefail
CUBE_REPO="${HOME}/STM32Cube/Repository"

printf 'host=%s %s\n' "$(uname -s)" "$(uname -m)"
# -dumpversion yields 14.3.1 and cannot separate ST's 14.3.rel1 build from any other 14.3.1. the compiler decides the numbers this project reports, so record the full identification line.
printf 'gcc=%s\n' "$(arm-none-eabi-gcc --version 2>/dev/null | head -1 || echo missing)"
printf 'cmake=%s\n' "$(cmake --version 2>/dev/null | head -1 | awk '{print $3}')"
printf 'cubemx=%s\n' "$(plutil -extract CFBundleShortVersionString raw /Applications/STMicroelectronics/STM32CubeMX.app/Contents/Info.plist 2>/dev/null || echo missing)"
printf 'cubeide=%s\n' "$(plutil -extract CFBundleShortVersionString raw /Applications/STM32CubeIDE.app/Contents/Info.plist 2>/dev/null || echo missing)"
# stedgeai is not on PATH and should not have to be. a PATH that only exists in one shell profile is not reproducible, so resolve it from where the installer put it and let an env var override for a different install.
STEDGEAI="${LAXITY_STEDGEAI:-$HOME/STEdgeAI/4.0/4.0/Utilities/macarm/stedgeai}"
printf 'stedgeai=%s\n' "$("$STEDGEAI" --version 2>/dev/null | head -1 || echo missing)"

# the programmer CLI prints an ANSI coloured banner before its version line, so  head -1 recorded the banner rule as the version. say unparsed rather than emit text that looks like data.
prog=$(STM32_Programmer_CLI --version 2>/dev/null | sed $'s/\033\\[[0-9;]*m//g' | sed -n 's/.*version: *\([0-9][0-9.]*\).*/\1/p' | head -1)
command -v STM32_Programmer_CLI >/dev/null && printf 'programmer=%s\n' "${prog:-unparsed}" || printf 'programmer=missing\n'
for d in "$CUBE_REPO"/STM32Cube_FW_U5_*; do
  [ -d "$d" ] && printf 'cubeu5=%s\n' "$(basename "$d")"
done
