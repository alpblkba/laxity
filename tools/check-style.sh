#!/usr/bin/env bash
# mechanise the writing rules a grep can catch. the rest still needs reading, but these are the ones that drift first.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
report() { echo "$2"; echo "$1"; fail=1; }

# the markdown this repository publishes. asking git rather than walking the tree keeps private
# notes out of scope on their own, and the style guide itself lists the forbidden words so it
# cannot be scanned for them.
DOCS=$(git ls-files -c -o --exclude-standard '*.md' | grep -v 'STYLE\.md' || true)

# a file git lists but the worktree does not have, a deletion not yet committed for instance, makes grep print the hits it did find and then exit 2. the four rules below used to take that exit status as their condition, so one missing file silenced all four while the script still reported ok. the scope is narrowed to what can be read, what was dropped is named, and the rules test their own output instead of grep's exit status.
SKIPPED=$(for f in $DOCS; do [ -r "$f" ] || echo "  $f"; done)
DOCS=$(for f in $DOCS; do [ -r "$f" ] && echo "$f"; done)
if [ -n "$SKIPPED" ]; then
  echo "git lists these and the worktree does not have them, so nothing below scanned them:"
  echo "$SKIPPED"
fi

# the two characters are written literally rather than as dollar quoted \u escapes. bash expands such an escape only when the locale can represent the character, and it leaves the escape text in place when it cannot, so this rule matched nothing in a POSIX locale. the arrow rule below has always been literal, which is why that one has been firing.
hits=$(grep -n '—\|–' $DOCS 2>/dev/null || true)
if [ -n "$hits" ]; then
  report "$hits" "em dash or en dash:"
fi

hits=$(grep -n '→\|←\|⇒' $DOCS 2>/dev/null || true)
if [ -n "$hits" ]; then
  report "$hits" "arrow in prose:"
fi

VOCAB='delve|leverage|harness|unlock|seamless|robust|powerful|elegant|cutting-edge'
VOCAB="$VOCAB|groundbreaking|revolutioniz|realm|tapestry|deep dive|unveil|illuminate"
VOCAB="$VOCAB|showcase|empower|streamline|holistic|comprehensive|crucial|vital|pivotal"
VOCAB="$VOCAB|dramatically|effortlessly|simply put|in essence|ultimately|moreover"
VOCAB="$VOCAB|furthermore|notably|importantly|that said|worth noting|should be noted"
hits=$(grep -niE "\b($VOCAB)\b" $DOCS 2>/dev/null || true)
if [ -n "$hits" ]; then
  report "$hits" "forbidden vocabulary:"
fi

# headings must be sentence case: flag a second capitalised word in a heading.
# the first grep's exit status would reach the pipeline under pipefail, and a hit the second grep keeps is still a hit, so the output is what decides.
hits=$(grep -nE '^#{1,6} +[A-Z][a-z]+ +[A-Z][a-z]+' $DOCS 2>/dev/null \
       | grep -vE '(STM32|Laxity|Apache|ThreadX|NetX|Edge AI|Cortex|SRAM|DWT|CubeMX|TinyML|RTOS|Wi-Fi)' || true)
if [ -n "$hits" ]; then
  report "$hits" "possible title case heading:"
fi

# every laxity command this repository quotes, run past the binary's own argument parser.
#
# four times a piece of prose here has said something the thing beside it does not say, and the fix each time was to bind the copy to what it copies. this binds a quoted command to the binary: --dry-run parses the arguments with the code the subcommand uses and touches neither the board nor a file, so nothing here holds a second idea of what a command accepts.
#
# self-docs and tracelog are excluded on purpose rather than for convenience. they are dated records of a moment, so a command that no longer runs is correct inside them. only live documentation and live configuration are checked.
COMMAND_FILES=$(git ls-files -c -o --exclude-standard '*.md' '*.toml' '*.sh' '*.py' \
  | grep -vE '^(target|results|self-docs|tracelog)/|^firmware/stm32u585/Middlewares/' || true)
LAXITY_BIN="${LAXITY_BIN:-./target/release/laxity}"

# a gate that passes by doing nothing is worse than no gate, so an absent binary fails rather than
# reporting itself and exiting zero.
if [ -n "$COMMAND_FILES" ] && [ ! -x "$LAXITY_BIN" ]; then
  report "  cargo build --release -p laxity" "no $LAXITY_BIN, so no quoted command can be checked:"
fi

if [ -n "$COMMAND_FILES" ] && [ -x "$LAXITY_BIN" ]; then
  # a command is the first word of a backtick span, of a quoted string, or of a line in a fenced block. prose that mentions a subcommand mid sentence is not one of those and is not checked, which is the boundary this pattern draws.
  # a command is the first word of a backtick span, of a quoted string, or of a line in a fenced block. prose that mentions a subcommand mid sentence is not one of those and is not checked, which is the boundary this pattern draws.
  quoted=$(grep -hoE '`[^`]+`|"[^"]+"|^ *\.?/?[a-z/]*laxity [a-z]+.*' $COMMAND_FILES 2>/dev/null \
    | sed -e 's/^ *//' -e 's/`//g' -e 's/"//g' -e 's|^\./||' -e 's|^[a-z/]*/laxity |laxity |' \
    | grep -E '^laxity (audit|console|characterise|tui|doctor|boards|build|flash|capture|analyse|run|sim|wifi)\b' \
    | sed -e 's/ [0-9]*[|>].*//' -e 's/ *&&.*//' -e 's/ *$//' | sort -u || true)
  # a backtick span holding exactly "laxity <word>" is a document naming a subcommand. the pattern
  # above cannot find one that does not exist, because it is anchored on the names that do, and a
  # README naming a subcommand nobody built is the hole this closes. prose is not inside backticks,
  # so this produces no false positive on a sentence that happens to contain the word.
  named=$(grep -hoE '`laxity [a-z-]+`' $COMMAND_FILES 2>/dev/null | tr -d '`' | sort -u || true)
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    # the binary exits non zero for an unknown name and pipefail would make the whole pipeline
    # fail with it, which would throw away the grep's own answer.
    if { $LAXITY_BIN --dry-run "${line#laxity }" 2>&1 || true; } | grep -q 'unknown subcommand'; then
      report "  $line" "a document names a subcommand this binary does not have:"
    else
      echo "  ok, name only: $line"
    fi
  done <<EOF
$named
EOF

  while IFS= read -r line; do
    [ -z "$line" ] && continue
    skip=""
    # a usage template is not a command. an angle bracket, a square bracket, an ellipsis or a
    # capitalised placeholder marks one.
    case "$line" in
      *'<'*|*'['*|*...*) skip="a usage template" ;;
      # a shell variable is not resolvable from here, so what it would expand to is unknown.
      *'$'*) skip="carries a shell variable" ;;
      # a trailing backslash is a continued line and what follows it is not on this one.
      *'\') skip="a continued line" ;;
      # the help text lays a subcommand beside its description in columns, which is prose in a table.
      *'  '*) skip="help text, not an invocation" ;;
      # a bare subcommand is a name rather than an invocation, and the pass above has already
      # checked that the name exists.
      'laxity '*' '*) ;;
      *) skip="a name, checked above" ;;
    esac
    if [ -z "$skip" ] && echo "$line" | grep -qE ' [A-Z][A-Z_]+( |$)'; then
      skip="a usage template"
    fi
    if [ -n "$skip" ]; then
      echo "  skipped, $skip: $line"
      continue
    fi
    # shellcheck disable=SC2086
    if ! out=$(eval "$LAXITY_BIN --dry-run ${line#laxity }" 2>&1); then
      report "  $line
  $out" "the binary refuses a command this repository quotes:"
    else
      echo "  ok: $line"
    fi
  done <<EOF
$quoted
EOF
fi

[ "$fail" -eq 0 ] && echo "style ok" || exit 1
