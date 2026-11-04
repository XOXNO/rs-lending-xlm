#!/usr/bin/env bash
# Check every Lean lemma file under komet/**/proofs/*/lean with the pinned
# Lean 4.22.0 release and --trust=0. Fails on any compile error and on any
# sorry / admit / axiom / native_decide / implemented_by / extern in a source.
# Usage: check-lean.sh [log-file]
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=$(cd -- "$here/../.." && pwd)
set -a; . "$here/pins.env"; set +a
log=${1:-/dev/stdout}
cache=${KOMET_PIPELINE_CACHE:-$repo/target-komet/pipeline-cache}
lean_root="$cache/lean-$LEAN_VERSION-linux"
lean="$lean_root/bin/lean"

if [ ! -x "$lean" ]; then
  mkdir -p "$cache"
  tarball="$cache/lean-$LEAN_VERSION-linux.tar.zst"
  [ -f "$tarball" ] || curl -fsSL -o "$tarball" \
    "https://github.com/leanprover/lean4/releases/download/v$LEAN_VERSION/lean-$LEAN_VERSION-linux.tar.zst"
  echo "$LEAN_TAR_SHA256  $tarball" | sha256sum -c - >&2
  tar --zstd -xf "$tarball" -C "$cache"
  rm -f "$tarball"
fi
echo "$LEAN_BIN_SHA256  $lean" | sha256sum -c - >&2

{
  "$lean" --version
  total=0
  fail=0
  for dir in $(find "$repo/komet" -type d -name lean -path '*/proofs/*' -not -path '*/target*' | sort); do
    out=$(mktemp -d)
    echo "=== ${dir#"$repo"/}"
    if grep -nE '\bsorry\b|\badmit\b|^\s*(private\s+)?axiom\b|native_decide|implemented_by|@\[extern' "$dir"/*.lean; then
      echo "FORBIDDEN construct found above"
      fail=$((fail + 1))
    fi
    # Compile in dependency order (imports among files of the same directory).
    order=$(python3 - "$dir" <<'PY'
import os, re, sys
d = sys.argv[1]
mods = {f[:-5] for f in os.listdir(d) if f.endswith('.lean')}
deps = {m: [i for i in re.findall(r'^import\s+(\S+)', open(os.path.join(d, m + '.lean')).read(), re.M) if i in mods] for m in mods}
seen = []
def visit(m):
    if m in seen:
        return
    for x in deps[m]:
        visit(x)
    seen.append(m)
for m in sorted(mods):
    visit(m)
print(' '.join(seen))
PY
)
    for m in $order; do
      total=$((total + 1))
      if msg=$(cd "$dir" && LEAN_PATH="$out" "$lean" --trust=0 -o "$out/$m.olean" "$m.lean" 2>&1); then
        [ -z "$msg" ] || printf '%s\n' "$msg"
        # Any `#print axioms` report may name only Lean's three standard axioms.
        if printf '%s\n' "$msg" | grep 'depends on axioms' \
          | sed 's/.*depends on axioms: \[\(.*\)\]/\1/' | tr ',' '\n' | tr -d ' ' \
          | grep -vxE 'propext|Classical\.choice|Quot\.sound' | grep -q .; then
          echo "FAIL $m (non-standard axiom)"
          fail=$((fail + 1))
          continue
        fi
        echo "OK   $m $(sha256sum "$dir/$m.lean" | cut -d' ' -f1)"
      else
        printf '%s\n' "$msg"
        echo "FAIL $m"
        fail=$((fail + 1))
      fi
    done
    rm -rf "$out"
  done
  echo "LEAN total=$total fail=$fail"
  [ "$fail" -eq 0 ]
} 2>&1 | tee "$log"
exit "${PIPESTATUS[0]}"
