#!/usr/bin/env bash
# Re-extracts every crate to Lean (Charon + Aeneas) and checks every proof.
#
# Fails if
# * a hand-copied stand-in differs from the production item it copies
#   (check_standins.py), a copied proof file is out of date (sync_copies.py),
#   or the release profile no longer traps on overflow;
# * a regenerated Funs.lean/Types.lean differs from the committed one (the
#   production code changed, so the proofs no longer describe it; set
#   ALLOW_DRIFT=1 to re-check anyway);
# * any proof breaks, or any declaration depends on `sorry` or a non-standard
#   axiom (`#assert_standard_axioms` at the end of each library root).
#
# Needs AENEAS_HOME (the unpacked Aeneas release pinned in README.md), rustup
# and elan. Usage: ./run.sh [rates|pool|controller|oracle ...]  (default: all)
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(pwd)

: "${AENEAS_HOME:?set AENEAS_HOME to the unpacked Aeneas release (see README.md)}"

# Production code the crates carry as copies (not `#[path]` includes).
./check_standins.py
# Proof files that are renamed copies of RatesAeneas proofs.
./sync_copies.py --check

# Aeneas translates `+`, `-`, `*` on machine integers as aborting on overflow.
# Production behaves that way only while the release profile keeps overflow
# checks on (README.md, Trusted base).
if ! awk '/^\[/ { section = $0 }
          section == "[profile.release]" && /^overflow-checks *= *true/ { ok = 1 }
          END { exit !ok }' ../../Cargo.toml; then
  echo "Cargo.toml: [profile.release] must set overflow-checks = true; the proofs assume integer overflow aborts" >&2
  exit 1
fi

# area -> crate dir, LLBC name, Lean library
declare -A DIR=([rates]=. [pool]=pool [controller]=controller [oracle]=oracle)
declare -A LLBC=([rates]=rates_aeneas [pool]=pool_aeneas [controller]=controller [oracle]=oracle_aeneas)
declare -A LIB=([rates]=RatesAeneas [pool]=PoolAeneas [controller]=ControllerAeneas [oracle]=OracleAeneas)

AREAS=("$@")
[ ${#AREAS[@]} -eq 0 ] && AREAS=(rates pool controller oracle)

extract() {
  local area=$1 dir=${DIR[$1]} llbc=${LLBC[$1]} lib=${LIB[$1]}
  echo "== extract $area"
  if [ "$area" = controller ]; then
    # Translates only the call graph of the proved functions (--start-from list).
    (cd controller && ./extract.sh)
  else
    # pool/ reads its extraction roots from [package.metadata.charon].
    (cd "$dir" && mkdir -p target/llbc &&
      "$AENEAS_HOME/charon" cargo --preset=aeneas --dest-file "target/llbc/$llbc.llbc" &&
      "$AENEAS_HOME/aeneas" -backend lean -dest "$ROOT/lean/$lib" -split-files \
        "target/llbc/$llbc.llbc")
    # Aeneas writes templates for the hand-written external files, which live
    # in FunsExternal.lean / TypesExternal.lean.
    rm -f "lean/$lib"/*_Template.lean
  fi
  # Optional, documented workaround for an Aeneas code-generation bug in this
  # area: <crate>/postextract.sh runs on the generated Lean before the drift
  # check, so the committed files are its output.
  if [ -x "$dir/postextract.sh" ]; then
    (cd "$dir" && ./postextract.sh "$ROOT/lean/$lib")
  fi
}

drift=0
for area in "${AREAS[@]}"; do
  extract "$area"
  lib=${LIB[$area]}
  if git ls-files --error-unmatch "lean/$lib/Funs.lean" >/dev/null 2>&1 &&
    ! git diff --quiet -- "lean/$lib/Funs.lean" "lean/$lib/Types.lean"; then
    echo "extracted Lean for $area changed: the production code differs from what the proofs cover" >&2
    git --no-pager diff --stat -- "lean/$lib/Funs.lean" "lean/$lib/Types.lean" >&2
    drift=1
  fi
done
if [ "$drift" = 1 ] && [ "${ALLOW_DRIFT:-0}" != 1 ]; then exit 1; fi

cd lean
lake exe cache get
# One library at a time: each carries its own copy of the shared external
# names, so they are never imported into the same Lean file.
# After each build the library root is elaborated once more outside Lake's
# cache: its last line, `#assert_standard_axioms`, also fails on a source
# file that no root imports, and adding such a file does not invalidate the
# cached root.
for lib in FixedPoint $(for area in "${AREAS[@]}"; do echo "${LIB[$area]}"; done); do
  echo "== build $lib"
  lake build "$lib" | grep -v 'declarations audited'
  lake env lean "$lib.lean"
done
