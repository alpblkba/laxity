#!/usr/bin/env bash
# put laxity on PATH by symlinking it into ~/.local/bin.
#
# a symlink rather than a copy, so the command and the repository cannot drift apart. laxity
# resolves the link to find the repository, which is also why moving the checkout is fine and
# copying the file is not.
set -euo pipefail
cd "$(dirname "$0")/.."

SRC="$(pwd)/tools/laxity"
BIN="${LAXITY_BIN_DIR:-$HOME/.local/bin}"
DST="$BIN/laxity"

[ -x "$SRC" ] || { echo "$SRC is missing or not executable" >&2; exit 1; }

mkdir -p "$BIN"

if [ -e "$DST" ] || [ -L "$DST" ]; then
  CURRENT="$(readlink "$DST" 2>/dev/null || echo "a regular file")"
  if [ "$CURRENT" = "$SRC" ]; then
    echo "already linked: $DST -> $SRC"
  else
    echo "$DST already exists and points at $CURRENT"
    printf 'replace it? [y/N] '
    read -r answer
    case "$answer" in
      y|Y|yes) ln -sfn "$SRC" "$DST"; echo "replaced: $DST -> $SRC" ;;
      *) echo "left alone. nothing was installed."; exit 0 ;;
    esac
  fi
else
  ln -s "$SRC" "$DST"
  echo "linked: $DST -> $SRC"
fi

# the shell rc file belongs to whoever owns the shell, so this reports and does not edit.
case ":$PATH:" in
  *":$BIN:"*) echo "$BIN is on PATH, run: laxity doctor" ;;
  *)
    echo
    echo "$BIN is not on PATH. add this line to your shell rc file yourself:"
    echo
    echo "    export PATH=\"$BIN:\$PATH\""
    echo
    case "$(basename "${SHELL:-}")" in
      zsh)  echo "on zsh that file is ~/.zshrc" ;;
      bash) echo "on bash that file is ~/.bash_profile on macOS, ~/.bashrc on Linux" ;;
    esac
    ;;
esac
