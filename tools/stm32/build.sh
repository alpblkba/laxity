#!/usr/bin/env bash
# one canonical non-interactive build. CubeIDE stays available for debugging and is not required here
#
# the CMake project is the one CubeMX generates, so -S points at it rather than at the repository root, and the toolchain file comes from the same tree
set -euo pipefail
cd "$(dirname "$0")/../.."
BUILD="${1:-build/target}"
SRC="firmware/stm32u585"
TYPE="${LAXITY_BUILD_TYPE:-Debug}"

cmake -B "$BUILD" -S "$SRC" -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE="$PWD/$SRC/cmake/gcc-arm-none-eabi.cmake" \
  -DCMAKE_BUILD_TYPE="$TYPE"
cmake --build "$BUILD" -j
arm-none-eabi-size "$BUILD"/laxity-u585.elf
