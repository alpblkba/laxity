#!/usr/bin/env bash
set -euo pipefail

DIAGRAM_DIR="$(cd "$(dirname "$0")" && pwd)"
CONFIG="$DIAGRAM_DIR/config/alpblkba.json"

if [ "$#" -eq 0 ]; then
    set -- "$DIAGRAM_DIR"/src/*.mmd
fi

mkdir -p "$DIAGRAM_DIR/generated"
for diagram_source in "$@"; do
    if [[ "$diagram_source" != */* ]]; then
        diagram_source="$DIAGRAM_DIR/src/${diagram_source%.mmd}.mmd"
    fi
    "$DIAGRAM_DIR/node_modules/.bin/mmdc" \
        --input "$diagram_source" \
        --output "$DIAGRAM_DIR/generated/$(basename "$diagram_source" .mmd).svg" \
        --configFile "$CONFIG" --backgroundColor white
done
