#!/usr/bin/env bash
# Run one Komet proof in the pinned prover image and check its graph.
# Usage: prove.sh <wasm> <test-id> <pass|fail|reject> <out-dir> [extra-module.k:MODULE] [cap-seconds]
# <out-dir> must not exist; it receives proof/, stdout.log, stderr.log,
# run.json (command, exit code, wall time) and check.json (graph verdict).
# Exit status: 0 when the graph meets the expectation, 1 otherwise.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=$(cd -- "$here/../.." && pwd)
wasm=$1 id=$2 expect=$3 out=$4 module=${5:-} cap=${6:-14400}
eval "$("$here/scripts/build-images.sh" print-tags)"
test ! -e "$out" || { echo "refusing to reuse existing $out" >&2; exit 2; }
mkdir -p "$out/tmp" "$out/home" "$out/input"
out=$(cd -- "$out" && pwd)
cp "$wasm" "$out/input/$(basename "$wasm")"
args=(prove run --always-allocate --wasm "/out/input/$(basename "$wasm")" --id "$id" --proof-dir /out/proof)
if [ -n "$module" ]; then
  file=${module%%:*} name=${module#*:}
  cp "$repo/$file" "$out/input/$(basename "$file")"
  args+=(--extra-module "/out/input/$(basename "$file"):$name")
fi

started=$(date -u +%FT%TZ)
t0=$(date +%s)
set +e
docker run --rm --network=none -u "$(id -u):$(id -g)" \
  -e HOME=/out/home -e TMPDIR=/out/tmp -v "$out:/out" -w /out "$PROVER_IMAGE" \
  timeout --signal=INT --kill-after=30s "$cap" komet "${args[@]}" \
  > "$out/stdout.log" 2> "$out/stderr.log"
rc=$?
set -e
wall=$(( $(date +%s) - t0 ))
python3 - "$out/run.json" "$PROVER_IMAGE" "$id" "$expect" "$rc" "$wall" "$cap" "$started" "${args[@]}" <<'PY'
import json, sys
path, image, pid, expect, rc, wall, cap, started, *args = sys.argv[1:]
json.dump({'image': image, 'id': pid, 'expect': expect, 'exit_code': int(rc), 'wall_seconds': int(wall),
           'cap_seconds': int(cap), 'started_utc': started, 'command': ['komet', *args]},
          open(path, 'w'), indent=1)
PY
docker run --rm --network=none -u "$(id -u):$(id -g)" -e HOME=/out/home \
  -v "$out:/out" -v "$here/scripts:/scripts:ro" "$PROVER_IMAGE" \
  python3 /scripts/check-proof.py /out/proof "$id" "$expect" /out/check.json \
  || { echo "check rejected $id (komet exit $rc)" >&2; exit 1; }
# Exit codes must agree with the graph: 0 for a proof, 1 for a refuted control.
want=0; [ "$expect" = pass ] || want=1
[ "$rc" -eq "$want" ] || { echo "komet exit $rc, expected $want for $id" >&2; exit 1; }
rm -rf "$out/tmp" "$out/home"
