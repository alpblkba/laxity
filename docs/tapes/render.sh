#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

if [ "$#" -eq 0 ]; then
    set -- overview memory audit cross-region crc
fi
for tape in "$@"; do
    case "$tape" in
        overview|memory|audit|cross-region|crc) ;;
        *) echo "Unknown tape $tape" >&2; exit 1 ;;
    esac
done
vhs validate docs/tapes/*.tape

# A separate build keeps recordings independent of installed commands and another build in the workspace.
cargo build --release --locked -p laxity --target-dir build/tapes/cargo
./tools/laxity-sim/build.sh build/tapes
export PATH="$PWD/build/tapes/cargo/release:$PATH"
export PYTHONDONTWRITEBYTECODE=1

scratch=$(mktemp -d "$PWD/build/tapes/inputs.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
for scenario in same-region-contention cross-region-contention crc-corruption; do
    ./build/tapes/laxity-sim --scenario "$scenario" --seed 1 --output "$scratch/$scenario.bin"
    laxity tui --file "$scratch/$scenario.bin" --profile profiles/stm32u585.toml --headless > "$scratch/$scenario.txt"
    mv "$scratch/$scenario.bin" "$scratch/$scenario.txt" build/tapes/
done

for tape in "$@"; do
    if [ "$tape" = audit ]; then
        laxity audit build/target/laxity-u585.elf profiles/stm32u585.toml profiles/stm32u585.characterisation.toml examples/stm32u585-reference/laxity.toml > build/tapes/audit.txt
    fi
    vhs "docs/tapes/$tape.tape"
    output=$(awk '$1 == "Output" && $2 ~ /\.gif$/ { print $2 }' "docs/tapes/$tape.tape")
    # A failed recording leaves the published GIF intact.
    mv "$output" "assets/$(basename "$output")"
done
