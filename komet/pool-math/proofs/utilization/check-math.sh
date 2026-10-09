#!/bin/sh
set -eu
lean_cmd=${1:-lean}
cd -- "$(dirname -- "$0")/lean"
export LEAN_PATH="$PWD"
"$lean_cmd" --trust=0 -o UtilizationWords.olean UtilizationWords.lean
"$lean_cmd" --trust=0 -o Fixed64Packing.olean Fixed64Packing.lean
"$lean_cmd" --trust=0 -o ByteWrap.olean ByteWrap.lean
"$lean_cmd" --trust=0 -o HalfWord.olean HalfWord.lean
"$lean_cmd" --trust=0 -o UtilizationAnd.olean UtilizationAnd.lean
for proof_source in WordOr.lean UtilizationPacking.lean UtilizationQuotient.lean UtilizationOrdering.lean Left32.lean ProjectedComplement.lean; do
  "$lean_cmd" --trust=0 "$proof_source"
done
