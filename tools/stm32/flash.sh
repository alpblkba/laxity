#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ELF="${1:-build/target/laxity-u585.elf}"
STM32_Programmer_CLI -c port=SWD mode=UR -w "$ELF" -rst
