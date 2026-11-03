#!/usr/bin/env bash
# Resume every test of an evidence directory whose last run stopped at its cap
# without a verdict or block (run.json outcome "stopped"), from its saved graph.
# Usage: resume-stopped.sh <evidence-dir> [cap-seconds]
set -euo pipefail
ev=$(cd -- "$1" && pwd) cap=${2:-14400}
snap="$ev/pipeline"
export KOMET_REPO_ROOT=${KOMET_REPO_ROOT:-$(cd -- "$(dirname -- "$0")/../../.." && pwd)}
for runf in "$ev"/*/test_*/run.json; do
  # Only runs that stopped at the cap, at most KOMET_MAX_ATTEMPTS (3) in all.
  [ "$(python3 -c "import json,sys;r=json.load(open(sys.argv[1]));print(r.get('outcome') if r.get('attempt',1) < int(sys.argv[2]) else 'done')" "$runf" "${KOMET_MAX_ATTEMPTS:-3}")" = stopped ] || continue
  d=$(dirname "$runf") id=$(basename "$(dirname "$runf")") pdir=$(dirname "$(dirname "$runf")")
  name=$(basename "$pdir")
  read -r module smt < <(python3 - "$snap/proofs.json" "$name" <<'PY'
import json, sys
p = next(p for p in json.load(open(sys.argv[1]))['proofs'] if p['name'] == sys.argv[2])
print(p.get('module', '-'), p.get('smt_timeout_ms', 0) or '-')
PY
)
  [ "$module" = - ] && module=
  [ "$smt" = - ] && smt=
  wasm=$(ls "$pdir"/wasm/*.wasm)
  echo "resuming $name/$id" >> "$pdir/$id.prove.log"
  ( KOMET_RESUME=1 KOMET_SMT_TIMEOUT_MS="$smt" "$snap/scripts/prove.sh" "$wasm" "$id" pass "$d" "$module" "$cap" \
      >> "$pdir/$id.prove.log" 2>&1 || true ) &
done
wait
