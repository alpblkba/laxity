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

# The tapes that record a running board. Everything else is a capture or an ELF and needs no hardware.
board_tape() {
    case "$1" in overview|memory|cross-region) return 0 ;; *) return 1 ;; esac
}

# The console keys that select the cell a tape records, or nothing to record whatever the schedule is on.
#
# The board's arena cross shuffles, so an unarmed recording catches a cell rather than choosing one, which is
# why cross-region.tape could not keep the promise its name makes. i puts the victim on the stress schedule,
# where the arena is the slot rather than the cross, 7 puts that arena in SRAM1 and c puts the aggressor in
# SRAM3, so the arena and the requester are in different regions for the whole recording.
tape_keys() {
    case "$1" in cross-region) echo "i7c" ;; *) echo "" ;; esac
}

# Whether the board is attached, flashed and streaming cleanly, decided once and by the same reader the tapes record.
#
# The headless reader fails outright when no ST-LINK port is there and when the stream carries no header
# frame, which is what an unflashed or halted board looks like. It does not fail when a second process is
# also reading the port, because macOS opens the device twice and the two readers split the bytes. That
# case exits zero and records a starved screen, so the stream's own counters decide as well.
#
# The test is that more frames were accepted than were thrown away. Opening a port mid frame costs one
# rejection and one false sync whatever else is true, so zero is the wrong bar: a healthy five seconds here
# reads 504 frames against 2 discards, and a port shared with one other reader read 16 against 78.
board_state=unknown
board_ready() {
    if [ "$board_state" = unknown ]; then
        board_state=absent
        laxity boards 2>&1 | sed 's/^/  /' || true
        if laxity tui --serial --headless --duration 5 > build/tapes/board.txt 2>&1 &&
           awk -F= '$1 == "accepted_frames" { good = $2 } $1 ~ /^(crc_rejections|false_sync)$/ { bad += $2 } END { exit (good > bad) ? 0 : 1 }' build/tapes/board.txt; then
            board_state=ready
            # The reader just held the port for five seconds and VHS is about to open it again.
            sleep 1
        fi
        sed 's/^/  /' build/tapes/board.txt
    fi
    [ "$board_state" = ready ]
}

scratch=$(mktemp -d "$PWD/build/tapes/inputs.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
# One scenario is still a replay. A rejected frame is fault injection and the board will not corrupt one on cue.
for scenario in crc-corruption; do
    ./build/tapes/laxity-sim --scenario "$scenario" --seed 1 --output "$scratch/$scenario.bin"
    laxity tui --file "$scratch/$scenario.bin" --profile profiles/stm32u585.toml --headless > "$scratch/$scenario.txt"
    mv "$scratch/$scenario.bin" "$scratch/$scenario.txt" build/tapes/
done

for tape in "$@"; do
    if board_tape "$tape"; then
        echo "checking the board for $tape"
        if ! board_ready; then
            echo "skipping $tape: no board streaming, so assets/ keeps the recording it has" >&2
            continue
        fi
        keys=$(tape_keys "$tape")
        if [ -n "$keys" ]; then
            # The console writer confirms the board reports what the keys ask for and exits non zero when it
            # does not, so a tape never records a cell nobody selected while the caption says one was.
            laxity console "$keys" | sed 's/^/  /'
            sleep 1
        fi
    fi
    if [ "$tape" = audit ]; then
        laxity audit > build/tapes/audit.txt
    fi
    vhs "docs/tapes/$tape.tape"
    output=$(awk '$1 == "Output" && $2 ~ /\.gif$/ { print $2 }' "docs/tapes/$tape.tape")
    # A failed recording leaves the published GIF intact.
    mv "$output" "assets/$(basename "$output")"
done
