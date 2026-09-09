#!/usr/bin/env bash
# regenerate the ST Edge AI network sources from the pinned model, so the command line is code rather than a comment that rots.
# the generated sources are not in Git. they carry the model's trained weights, the model is SLA0044 and cannot be committed, so this script is what stands between a clean clone and a build. the same is already true of the CubeMX tree and tools/stm32/generate.sh.
# generation is reproducible where it matters. the weights array and the graph come out byte identical across runs. the date banner and STAI_NETWORK_MODEL_SIGNATURE in network.h differ per run, so two clones build binaries that differ by eight bytes and a date string.
set -euo pipefail
cd "$(dirname "$0")/../.."

MODEL="${LAXITY_MODEL:-$HOME/laxity-inputs/st_ign_wl_24.keras}"
STEDGEAI="${LAXITY_STEDGEAI:-$HOME/STEdgeAI/4.0/4.0/Utilities/macarm/stedgeai}"
DEST="backend/stedgeai"
WS="$(mktemp -d -t stedgeai-ws)"
GEN="$(mktemp -d -t stedgeai-out)"
LOG="$(mktemp -t stedgeai-log)"

[ -x "$STEDGEAI" ] || { echo "no stedgeai at $STEDGEAI, install ST Edge AI Core and see TOOLCHAIN.md" >&2; exit 1; }
[ -f "$MODEL" ]    || { echo "no model at $MODEL, TOOLCHAIN.md records where it comes from" >&2; exit 1; }

# a pin nobody checks is decoration. the hash lives in toolchain.toml, which is in the repository, because this script has to work in a clean clone.
WANT="$(sed -n 's/^sha256 = "\(.*\)"/\1/p' toolchain.toml)"
HAVE="$(shasum -a 256 "$MODEL" | cut -d' ' -f1)"
[ -n "$WANT" ] || { echo "no model sha256 in toolchain.toml" >&2; exit 1; }
if [ "$WANT" != "$HAVE" ]; then
  echo "model hash mismatch, refusing to generate" >&2
  echo "  expected $WANT" >&2
  echo "  found    $HAVE" >&2
  exit 1
fi

# the model is pinned and checked, so the generator has to be too. the same tool at the same path can be upgraded under us, and a different generator on the same model produces different code without anything in the tree changing.
WANT_TOOL="$(sed -n 's/^stedgeai = "\(.*\)"/\1/p' toolchain.toml)"
HAVE_TOOL="$("$STEDGEAI" --version 2>/dev/null | head -1)"
[ -n "$WANT_TOOL" ] || { echo "no stedgeai version in toolchain.toml" >&2; exit 1; }
if [ "$WANT_TOOL" != "$HAVE_TOOL" ]; then
  echo "generator version mismatch, refusing to generate" >&2
  echo "  expected $WANT_TOOL" >&2
  echo "  found    $HAVE_TOOL" >&2
  exit 1
fi

# generation goes to a temporary directory and only the five sources are copied across, since stedgeai also writes a licence, a json summary and a report that nothing here consumes.
"$STEDGEAI" generate --model "$MODEL" --target stm32u5 --c-api st-ai --name network \
  --workspace "$WS" --output "$GEN" </dev/null >"$LOG" 2>&1 || true
rm -rf "$WS"

# the exit status has lied three times in this project, once from CubeMX after KO, once from swmgr, once from the ST Edge AI installer after refusing its own arguments. success comes from the artifacts and the log.
for f in network.c network.h network_data.c network_data.h network_details.h; do
  [ -f "$GEN/$f" ] || { echo "missing $f, log kept at $LOG" >&2; exit 1; }
  grep -Fq "$GEN/$f" "$LOG" || { echo "$f not reported as generated, log kept at $LOG" >&2; exit 1; }
  cp "$GEN/$f" "$DEST/$f"
done

rm -rf "$GEN"; rm -f "$LOG"
echo "generated into $DEST from $MODEL"
