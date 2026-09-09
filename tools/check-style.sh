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

if hits=$(grep -n $'\u2014\|\u2013' $DOCS 2>/dev/null); then
  report "$hits" "em dash or en dash:"
fi

if hits=$(grep -n '→\|←\|⇒' $DOCS 2>/dev/null); then
  report "$hits" "arrow in prose:"
fi

VOCAB='delve|leverage|harness|unlock|seamless|robust|powerful|elegant|cutting-edge'
VOCAB="$VOCAB|groundbreaking|revolutioniz|realm|tapestry|deep dive|unveil|illuminate"
VOCAB="$VOCAB|showcase|empower|streamline|holistic|comprehensive|crucial|vital|pivotal"
VOCAB="$VOCAB|dramatically|effortlessly|simply put|in essence|ultimately|moreover"
VOCAB="$VOCAB|furthermore|notably|importantly|that said|worth noting|should be noted"
if hits=$(grep -niE "\b($VOCAB)\b" $DOCS 2>/dev/null); then
  report "$hits" "forbidden vocabulary:"
fi

# headings must be sentence case: flag a second capitalised word in a heading.
if hits=$(grep -nE '^#{1,6} +[A-Z][a-z]+ +[A-Z][a-z]+' $DOCS 2>/dev/null \
          | grep -vE '(STM32|Laxity|Apache|ThreadX|NetX|Edge AI|Cortex|SRAM|DWT|CubeMX|TinyML|RTOS|Wi-Fi)'); then
  report "$hits" "possible title case heading:"
fi

[ "$fail" -eq 0 ] && echo "style ok" || exit 1
