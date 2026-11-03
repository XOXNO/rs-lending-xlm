#!/usr/bin/env bash
# Run one Komet proof in the pinned prover image and check its graph.
# Usage: prove.sh <wasm> <test-id> <pass|fail|reject> <out-dir> [extra-module.k:MODULE] [cap-seconds]
# <out-dir> must not exist unless KOMET_RESUME=1: then the saved proof graph
# in <out-dir>/proof is continued (Komet checks that claim, module, runner and
# compiled definitions are unchanged), earlier logs move to attempt-N/, and
# run.json records the attempt number and the checkpoint it resumed from.
# Fail fast: a claim expected to pass is stopped at the first stuck node, and
# an SMT canceled/unknown answer ends it as `blocked` (blocked.txt holds the
# query); blocked is not a verdict. Outputs: proof/, stdout.log, stderr.log,
# run.json, check.json. Exit 0 when the graph meets the expectation, 1 when it
# does not, 3 when blocked or stopped at the cap without a verdict.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=${KOMET_REPO_ROOT:-$(cd -- "$here/../.." && pwd)}
wasm=$1 id=$2 expect=$3 out=$4 module=${5:-} cap=${6:-14400}
eval "$("$here/scripts/build-images.sh" print-tags)"
attempt=1 resumed_from=null
if [ -e "$out" ]; then
  [ "${KOMET_RESUME:-0}" = 1 ] || { echo "refusing to reuse existing $out (set KOMET_RESUME=1)" >&2; exit 2; }
  out=$(cd -- "$out" && pwd)
  while [ -d "$out/attempt-$attempt" ]; do attempt=$((attempt + 1)); done
  mkdir -p "$out/attempt-$attempt"
  for f in run.json check.json stdout.log stderr.log blocked.txt memory.peak; do
    [ -f "$out/$f" ] && mv "$out/$f" "$out/attempt-$attempt/"
  done
  resumed_from=$(python3 - "$out/proof/$id/kcfg/kcfg.json" "$attempt" <<'PY'
import json, sys
try:
    g = json.load(open(sys.argv[1]))
    print(json.dumps({'attempt': int(sys.argv[2]), 'nodes': len(g['nodes']),
                      'rewrite_steps': sum(e['depth'] for e in g['edges'])}))
except FileNotFoundError:
    print('null')
PY
)
  attempt=$((attempt + 1))
fi
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
# Optional GHC RTS options for the backend (KOMET_BOOSTER_RTS, e.g. "-c" for
# the compacting collector, which needs far less headroom than copying GC).
if [ -n "${KOMET_SMT_TIMEOUT_MS:-}" ] || [ -n "${KOMET_BOOSTER_RTS:-}" ]; then
  smt_arg=${KOMET_SMT_TIMEOUT_MS:+--smt-timeout $KOMET_SMT_TIMEOUT_MS}
  rts_arg=${KOMET_BOOSTER_RTS:++RTS $KOMET_BOOSTER_RTS -RTS}
  cat > "$out/input/kore-rpc-booster" <<WRAP
#!/bin/bash
args=(); skip=0
for a in "\$@"; do
  if ((skip)); then skip=0; continue; fi
  if [[ -n "${KOMET_SMT_TIMEOUT_MS:-}" && \$a == --smt-timeout ]]; then skip=1; continue; fi
  args+=("\$a")
done
flags=\$(sed -n 's/^booster_flags=//p' /opt/toolchain.txt)
exec /opt/komet-backend/bin/kore-rpc-booster "\${args[@]}" ${smt_arg} ${rts_arg} \$flags
WRAP
  chmod 0755 "$out/input/kore-rpc-booster"
  mounts=(-v "$out/input/kore-rpc-booster:/opt/komet-backend/wrapper/kore-rpc-booster:ro")
fi

# Resource limits (mandatory after a host-wide OOM): one proof container per
# slot (KOMET_SLOTS, default 3, shared by every run through flock), a hard
# memory cap (KOMET_MEMORY, default 6g, no swap), and no start while the host
# has less than KOMET_MIN_AVAIL_GB (default 8) GB available.
slots=${KOMET_SLOTS:-3} slot_dir=${KOMET_SLOT_DIR:-/tmp/komet-proof-slots}
mkdir -p "$slot_dir"
exec 9>/dev/null
while :; do
  got=
  for i in $(seq 1 "$slots"); do
    exec 9>"$slot_dir/slot$i"
    if flock -n 9; then got=$i; break; fi
  done
  if [ -n "$got" ]; then
    avail=$(awk '/MemAvailable/ {print int($2/1048576)}' /proc/meminfo)
    [ "$avail" -ge "${KOMET_MIN_AVAIL_GB:-8}" ] && break
    flock -u 9
  fi
  sleep 30
done
mem=${KOMET_MEMORY:-6g}
name="komet-proof-$$-$RANDOM"

started=$(date -u +%FT%TZ)
t0=$(date +%s)
# Fail-fast watcher: a claim expected to pass cannot succeed once a node is
# stuck; stop the container as soon as a saved graph shows one.
(
  [ "$expect" = pass ] || exit 0
  while sleep 60; do
    g="$out/proof/$id/kcfg/kcfg.json"
    [ -f "$g" ] || continue
    if python3 -c "import json,sys; sys.exit(0 if json.load(open(sys.argv[1])).get('stuck') else 1)" "$g" 2>/dev/null; then
      echo "stuck node in a claim expected to pass" > "$out/blocked.txt"
      docker stop -t 30 "$name" >/dev/null 2>&1
      exit 0
    fi
  done
) &
watch=$!
set +e
docker run --name "$name" --network=none --memory="$mem" --memory-swap="$mem" \
  -u "$(id -u):$(id -g)" "${mounts[@]}" \
  -e HOME=/out/home -e TMPDIR=/out/tmp -v "$out:/out" -w /out "$PROVER_IMAGE" \
  bash -c 'timeout --signal=INT --kill-after=30s "$0" komet "$@"; rc=$?
    cat /sys/fs/cgroup/memory.peak > /out/memory.peak 2>/dev/null; exit $rc' "$cap" "${args[@]}" \
  > "$out/stdout.log" 2> "$out/stderr.log"
rc=$?
kill "$watch" 2>/dev/null || true
oom=$(docker inspect -f '{{.State.OOMKilled}}' "$name" 2>/dev/null || echo unknown)
docker rm -f "$name" >/dev/null 2>&1
set -e
flock -u 9
wall=$(( $(date +%s) - t0 ))
peak=$(cat "$out/memory.peak" 2>/dev/null || echo 0)
# Classify runs that ended without a verdict.
outcome=verdict
if grep -qE "SmtSolverError|canceled|retry limit" "$out/stderr.log" 2>/dev/null; then
  outcome=blocked
  python3 - "$out/stderr.log" >> "$out/blocked.txt" <<'PY'
import sys
s = open(sys.argv[1]).read()
i = s.rfind('CTermSMTError:')
print('SMT canceled/unknown on query:')
print(s[i:i + 4000] if i >= 0 else s[-4000:])
PY
elif [ -f "$out/blocked.txt" ]; then
  outcome=blocked
elif [ "$oom" = true ] || grep -q "Empty response received" "$out/stderr.log" 2>/dev/null; then
  outcome=infrastructure
elif [ "$rc" = 124 ] || [ "$rc" = 130 ] || [ "$rc" = 137 ] || [ "$rc" = 143 ]; then
  outcome=stopped
fi
python3 - "$out/run.json" "$PROVER_IMAGE" "$id" "$expect" "$rc" "$wall" "$cap" "$started" "${KOMET_SMT_TIMEOUT_MS:-5000}" "$mem" "$peak" "$oom" "$outcome" "$attempt" "$resumed_from" "${args[@]}" <<'PY'
import json, sys
import os
path, image, pid, expect, rc, wall, cap, started, smt, mem, peak, oom, outcome, attempt, resumed, *args = sys.argv[1:]
json.dump({'image': image, 'id': pid, 'expect': expect, 'exit_code': int(rc), 'wall_seconds': int(wall),
           'cap_seconds': int(cap), 'started_utc': started, 'smt_timeout_ms': int(smt),
           'booster_rts': os.environ.get('KOMET_BOOSTER_RTS', ''),
           'memory_limit': mem, 'memory_peak_bytes': int(peak or 0), 'oom_killed': oom,
           'outcome': outcome, 'attempt': int(attempt), 'resumed_from': json.loads(resumed),
           'command': ['komet', *args]},
          open(path, 'w'), indent=1)
PY
if [ "$outcome" != verdict ]; then
  echo "$id: no verdict ($outcome); see $out/blocked.txt / stderr.log" >&2
  exit 3
fi
docker run --rm --network=none --memory=2g --memory-swap=2g -u "$(id -u):$(id -g)" -e HOME=/out/home \
  -e KOMET_REJECT_CODES="${KOMET_REJECT_CODES:-}" -v "$out:/out" -v "$here/scripts:/scripts:ro" "$PROVER_IMAGE" \
  python3 /scripts/check-proof.py /out/proof "$id" "$expect" /out/check.json \
  || { echo "check rejected $id (komet exit $rc)" >&2; exit 1; }
# Exit codes must agree with the graph: 0 for a proof, 1 for a refuted control.
want=0; [ "$expect" = pass ] || want=1
[ "$rc" -eq "$want" ] || { echo "komet exit $rc, expected $want for $id" >&2; exit 1; }
rm -rf "$out/tmp" "$out/home"
