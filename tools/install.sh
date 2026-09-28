#!/usr/bin/env bash
# build the laxity binary and put it on PATH.
#
# the binary owns audit and tui and hands every board subcommand to tools/laxity, so the script stays
# where it is and is not installed: the binary finds it next to itself, at tools/laxity under the
# current directory, or at $LAXITY_PYTHON. installing a copy of the script would give the binary two
# scripts to choose between, which is the one thing it must not have to guess about.
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT="$(pwd)"
BIN="${LAXITY_BIN_DIR:-$HOME/.local/bin}"

command -v cargo >/dev/null || { echo "cargo is not on PATH, install Rust first" >&2; exit 1; }
[ -x "$ROOT/tools/laxity" ] || { echo "$ROOT/tools/laxity is missing or not executable" >&2; exit 1; }

# --root puts the binary in BIN/bin, so the parent of the bin directory is what cargo is given.
PARENT="$(dirname "$BIN")"
[ "$(basename "$BIN")" = "bin" ] || PARENT="$BIN"

echo "building laxity"
cargo install --path crates/laxity --root "$PARENT" --force
DST="$PARENT/bin/laxity"

echo
echo "installed: $DST"
echo "the Python script stays at $ROOT/tools/laxity for the delegated subcommands"
echo "audit and tui are the binary's own, laxity --help says which is which"

case ":$PATH:" in
  *":$PARENT/bin:"*) echo "$PARENT/bin is on PATH, run: laxity doctor" ;;
  *)
    echo
    echo "$PARENT/bin is not on PATH. add this line to your shell rc file yourself:"
    echo
    echo "    export PATH=\"$PARENT/bin:\$PATH\""
    echo
    case "$(basename "${SHELL:-}")" in
      zsh)  echo "on zsh that file is ~/.zshrc" ;;
      bash) echo "on bash that file is ~/.bash_profile on macOS, ~/.bashrc on Linux" ;;
    esac
    ;;
esac

# the binary looks for the script next to itself first, so a checkout that moves keeps working only
# when the repository is the one the current directory is in, or when LAXITY_PYTHON names it.
echo
echo "if you run laxity from outside this checkout, set:"
echo
echo "    export LAXITY_PYTHON=\"$ROOT/tools/laxity\""
