#!/usr/bin/env bash
# Run one Komet proof in the pinned prover image and check its graph.
# Usage: prove.sh <wasm> <test-id> <pass|fail|reject> <out-dir> [extra-module.k:MODULE] [cap-seconds]
# <out-dir> must not exist; it receives proof/, stdout.log, stderr.log,
# run.json (command, exit code, wall time) and check.json (graph verdict).
# Exit status: 0 when the graph meets the expectation, 1 otherwise.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=${KOMET_REPO_ROOT:-$(cd -- "$here/../.." && pwd)}
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

# Optional larger initial SMT timeout (KOMET_SMT_TIMEOUT_MS): a wrapper that
# replaces the runner's --smt-timeout and keeps the recorded backend flags.
# A timeout only affects completeness; the evidence checks are unchanged.
mounts=()
if [ -n "${KOMET_SMT_TIMEOUT_MS:-}" ]; then
  cat > "$out/input/kore-rpc-booster" <<WRAP
#!/bin/bash
args=(); skip=0
for a in "\$@"; do
  if ((skip)); then skip=0; continue; fi
  if [[ \$a == --smt-timeout ]]; then skip=1; continue; fi
  args+=("\$a")
done
flags=\$(sed -n 's/^booster_flags=//p' /opt/toolchain.txt)
exec /opt/komet-backend/bin/kore-rpc-booster "\${args[@]}" --smt-timeout ${KOMET_SMT_TIMEOUT_MS} \$flags
WRAP
  chmod 0755 "$out/input/kore-rpc-booster"
  mounts=(-v "$out/input/kore-rpc-booster:/opt/komet-backend/wrapper/kore-rpc-booster:ro")
fi

started=$(date -u +%FT%TZ)
t0=$(date +%s)
set +e
docker run --rm --network=none -u "$(id -u):$(id -g)" "${mounts[@]}" \
  -e HOME=/out/home -e TMPDIR=/out/tmp -v "$out:/out" -w /out "$PROVER_IMAGE" \
  timeout --signal=INT --kill-after=30s "$cap" komet "${args[@]}" \
  > "$out/stdout.log" 2> "$out/stderr.log"
rc=$?
set -e
wall=$(( $(date +%s) - t0 ))
python3 - "$out/run.json" "$PROVER_IMAGE" "$id" "$expect" "$rc" "$wall" "$cap" "$started" "${KOMET_SMT_TIMEOUT_MS:-5000}" "${args[@]}" <<'PY'
import json, sys
path, image, pid, expect, rc, wall, cap, started, smt, *args = sys.argv[1:]
json.dump({'image': image, 'id': pid, 'expect': expect, 'exit_code': int(rc), 'wall_seconds': int(wall),
           'cap_seconds': int(cap), 'started_utc': started, 'smt_timeout_ms': int(smt),
           'command': ['komet', *args]},
          open(path, 'w'), indent=1)
PY
docker run --rm --network=none -u "$(id -u):$(id -g)" -e HOME=/out/home \
  -e KOMET_REJECT_CODES="${KOMET_REJECT_CODES:-}" -v "$out:/out" -v "$here/scripts:/scripts:ro" "$PROVER_IMAGE" \
  python3 /scripts/check-proof.py /out/proof "$id" "$expect" /out/check.json \
  || { echo "check rejected $id (komet exit $rc)" >&2; exit 1; }
# Exit codes must agree with the graph: 0 for a proof, 1 for a refuted control.
want=0; [ "$expect" = pass ] || want=1
[ "$rc" -eq "$want" ] || { echo "komet exit $rc, expected $want for $id" >&2; exit 1; }
rm -rf "$out/tmp" "$out/home"
