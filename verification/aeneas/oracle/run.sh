#!/usr/bin/env bash
# Extracts the production LP oracle code to Lean (Charon + Aeneas) and checks
# every proof in lean/OracleAeneas. Fails if the regenerated Lean differs from
# the committed Funs.lean/Types.lean (the production code changed and the
# proofs must be re-checked; set ALLOW_DRIFT=1 to re-check anyway).
#
# Needs: AENEAS_HOME pointing at an unpacked Aeneas release (../README.md,
# "Setup"), rustup, and elan.
set -euo pipefail
cd "$(dirname "$0")"

: "${AENEAS_HOME:?set AENEAS_HOME to the unpacked Aeneas release (see ../README.md)}"
mkdir -p target/llbc

"$AENEAS_HOME/charon" cargo --preset=aeneas --dest-file target/llbc/oracle_aeneas.llbc
"$AENEAS_HOME/aeneas" -backend lean -dest ../lean/OracleAeneas -split-files \
  target/llbc/oracle_aeneas.llbc
# Aeneas writes templates for the hand-written external files; those live in
# FunsExternal.lean / TypesExternal.lean.
rm -f ../lean/OracleAeneas/*_Template.lean

if git ls-files --error-unmatch ../lean/OracleAeneas/Funs.lean >/dev/null 2>&1 &&
  ! git diff --quiet -- ../lean/OracleAeneas/Funs.lean ../lean/OracleAeneas/Types.lean; then
  echo "extracted Lean changed: the production code differs from what the proofs cover" >&2
  git --no-pager diff --stat -- ../lean/OracleAeneas/Funs.lean ../lean/OracleAeneas/Types.lean >&2
  [ "${ALLOW_DRIFT:-0}" = 1 ] || exit 1
fi

cd ../lean
lake exe cache get
lake build OracleAeneas
