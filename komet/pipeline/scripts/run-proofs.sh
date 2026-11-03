#!/usr/bin/env bash
# Rebuild harness Wasms, re-run the selected Komet proofs and controls,
# re-check Lean, verify hashes and assemble an evidence bundle.
# Usage: run-proofs.sh <evidence-dir> [proof-name ...]   (default: every proof)
# Needs: docker, python3, curl, zstd. Images are built when missing.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=$(cd -- "$here/../.." && pwd)
ev=$1; shift
test ! -e "$ev" || { echo "evidence dir $ev exists" >&2; exit 2; }
mkdir -p "$ev"
ev=$(cd -- "$ev" && pwd)
manifest="$here/proofs.json"
mapfile -t names < <(python3 - "$manifest" "$@" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
want = sys.argv[2:]
known = [p['name'] for p in m['proofs']]
bad = [w for w in want if w not in known]
if bad:
    sys.exit(f'unknown proof(s): {bad}; known: {known}')
print('\n'.join(want or known))
PY
)

# Run from a snapshot of the pipeline scripts: editing the checkout during a
# long run must not change the scripts that bash is still reading.
snap="$ev/pipeline"
mkdir -p "$snap"
cp -pr "$here/scripts" "$here/docker" "$here/pins.env" "$here/proofs.json" "$snap/"
export KOMET_REPO_ROOT="$repo"

eval "$("$here/scripts/build-images.sh" print-tags)"
docker image inspect "$PROVER_IMAGE" >/dev/null 2>&1 || "$here/scripts/build-images.sh" prover
docker image inspect "$WASM_IMAGE" >/dev/null 2>&1 || "$here/scripts/build-images.sh" wasm

{
  echo "repo_commit=$(git -C "$repo" rev-parse HEAD)"
  echo "repo_dirty=$(git -C "$repo" status --porcelain -- komet common contracts interfaces | wc -l)"
  echo "prover_image=$PROVER_IMAGE $(docker image inspect -f '{{.Id}}' "$PROVER_IMAGE")"
  echo "wasm_image=$WASM_IMAGE $(docker image inspect -f '{{.Id}}' "$WASM_IMAGE")"
  echo "host=$(uname -srm) cpus=$(nproc)"
  echo "started_utc=$(date -u +%FT%TZ)"
} > "$ev/context.txt"
cp "$here/pins.env" "$manifest" "$ev/"
docker run --rm "$PROVER_IMAGE" cat /opt/toolchain.txt > "$ev/toolchain.txt"
docker run --rm "$PROVER_IMAGE" cat /opt/semantics.sha256 > "$ev/semantics.sha256"

status=0
"$here/scripts/check-lean.sh" "$ev/lean.log" > /dev/null || { echo "Lean check failed" >&2; status=1; }

# One proof = build its Wasm, run its false controls first, then (only if
# every control failed correctly) its positive claims: a short feasibility
# stage (KOMET_FEASIBILITY_CAP, default 600 s) that is resumed from its saved
# graph with the full cap only when it stopped at the cap without a solver
# block. Proof containers are limited globally by prove.sh (3 slots, 6 GB).
run_test() { # run_test <dir> <wasm> <id> <expect> <codes> <cap> [resume]
  local d=$1 wasm=$2 id=$3 expect=$4 codes=$5 cap=$6 resume=${7:-0}
  KOMET_RESUME=$resume KOMET_REJECT_CODES="$codes" KOMET_SMT_TIMEOUT_MS="$smt_ms" \
    "$snap/scripts/prove.sh" "$wasm" "$id" "$expect" "$d/$id" "$module" "$cap" >> "$d/$id.prove.log" 2>&1
}
run_proof() {
  local name=$1
  eval "$(python3 - "$manifest" "$name" <<'PY'
import json, shlex, sys
p = next(p for p in json.load(open(sys.argv[1]))['proofs'] if p['name'] == sys.argv[2])
print(f"crate={shlex.quote(p['crate'])} feature={shlex.quote(p['feature'])}")
print(f"want_sha={shlex.quote(p['wasm_sha256'])} module={shlex.quote(p.get('module', ''))}")
print(f"cap={int(p.get('cap_seconds', 14400))} smt_ms={int(p.get('smt_timeout_ms', 0)) or ''}")
print('tests=(' + ' '.join(shlex.quote(f"{t['id']}:{t['expect']}:{','.join(map(str, t.get('codes', [])))}") for t in p['tests']) + ')')
PY
)"
  local dir="$ev/$name" sha wasm t id rest expect codes rc cpids=() ok=1
  mkdir -p "$dir"
  sha=$("$snap/scripts/build-wasm.sh" "$crate" "$feature" "$dir/wasm" 2> "$dir/wasm-build.log")
  echo "$sha" > "$dir/wasm.sha256"
  if [ "$sha" != "$want_sha" ]; then
    echo "$name: Wasm SHA-256 $sha != recorded $want_sha" >> "$ev/errors.txt"
    return
  fi
  wasm=$(ls "$dir"/wasm/*.wasm)
  for t in "${tests[@]}"; do
    id=${t%%:*} rest=${t#*:} expect=${rest%%:*} codes=${rest#*:}
    [ "$expect" = pass ] && continue
    run_test "$dir" "$wasm" "$id" "$expect" "$codes" "$cap" &
    cpids+=($!)
  done
  for p in "${cpids[@]}"; do wait "$p" || ok=0; done
  if [ "$ok" = 0 ]; then
    echo "$name: a false control did not fail correctly; positive claims skipped" >> "$ev/errors.txt"
    return
  fi
  for t in "${tests[@]}"; do
    id=${t%%:*} rest=${t#*:} expect=${rest%%:*} codes=${rest#*:}
    [ "$expect" = pass ] || continue
    (
      run_test "$dir" "$wasm" "$id" pass "$codes" "${KOMET_FEASIBILITY_CAP:-600}"; rc=$?
      outcome=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get('outcome'))" "$dir/$id/run.json" 2>/dev/null)
      if [ "$rc" = 3 ] && [ "$outcome" = stopped ]; then
        echo "feasibility stage stopped at the cap without a block: promoted" >> "$dir/$id.prove.log"
        run_test "$dir" "$wasm" "$id" pass "$codes" "$cap" 1; rc=$?
      fi
      [ "$rc" = 0 ] || echo "$name/$id: no proof (exit $rc, see $id/run.json outcome)" >> "$ev/errors.txt"
    ) &
  done
  wait
}
# Fixed-constant framings first; manifest `lane` > 0 (fully symbolic) last.
mapfile -t names < <(python3 - "$manifest" "${names[@]}" <<'PY'
import json, sys
m = {p['name']: p.get('lane', 0) for p in json.load(open(sys.argv[1]))['proofs']}
for n in sorted(sys.argv[2:], key=lambda n: m[n]):
    print(n)
PY
)
pids=()
for name in "${names[@]}"; do
  run_proof "$name" &
  pids+=($!)
done
for p in "${pids[@]}"; do wait "$p" || true; done
[ ! -s "$ev/errors.txt" ] || status=1

python3 - "$ev" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1])
rows = []
for runf in sorted(ev.glob('*/test_*/run.json')):
    run = json.loads(runf.read_text())
    cf = runf.parent / 'check.json'
    c = json.loads(cf.read_text()) if cf.exists() else {}
    rows.append({'proof': runf.parent.parent.name, 'id': run['id'], 'expect': run['expect'],
                 'outcome': run.get('outcome', 'verdict'), 'attempt': run.get('attempt', 1),
                 'status': c.get('status'), 'verdict': c.get('verdict', 'NONE'), 'exit_code': run['exit_code'],
                 'wall_seconds': run['wall_seconds'], 'memory_peak_bytes': run.get('memory_peak_bytes'),
                 'nodes': c.get('nodes'), 'rewrite_steps': c.get('rewrite_steps'), 'errors': c.get('errors', [])})
(ev / 'summary.json').write_text(json.dumps(rows, indent=1) + '\n')
for r in rows:
    print(f"{r['proof']:<20} {r['id']:<34} expect={r['expect']:<6} {r['outcome']:<9} {str(r['status']):<7} {r['verdict']:<8} "
          f"exit={r['exit_code']} wall={r['wall_seconds']}s nodes={r['nodes']} steps={r['rewrite_steps']}")
PY
echo "finished_utc=$(date -u +%FT%TZ) status=$status" >> "$ev/context.txt"
(cd "$ev" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
exit "$status"
