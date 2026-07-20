#!/usr/bin/env bash
# Run every rule of a conf as its own local prover invocation.
# Default sequential; -j N runs N rules concurrently (staggered starts so
# report-dir numbering in the shared conf dir cannot collide).
#
# Soundness note: Verified verdicts are final regardless of machine load;
# only Timeout verdicts are load-sensitive — retry those solo (-j 1).
#
# Usage: run-rules-local.sh [-j N] <path/to/conf> [rule ...]
set -euo pipefail
jobs=1
if [ "${1:-}" = "-j" ]; then jobs="$2"; shift 2; fi
conf="$1"; shift
dir=$(dirname "$conf"); name=$(basename "$conf")
if [ $# -gt 0 ]; then rules=("$@"); else
  mapfile -t rules < <(python3 -c "import json,sys; [print(r) for r in json.load(open(sys.argv[1]))['rule']]" "$conf")
fi

# Bound each job's JVM heap so parallel runs cannot exhaust the box
# (unbounded JVMs default to 1/4 of physical RAM *each*).
heap="${CERTORA_JAVA_HEAP:--Xmx24g}"

# Local prover invocation (see README "Local prover"): CLI script + emv.jar.
# Override CERTORA_LOCAL with a full command prefix if the install moves.
install_dir="${CERTORA_INSTALL:-$HOME/certora-install}"
local_cmd="${CERTORA_LOCAL:-python3 $install_dir/certoraSorobanProver.py}"

run_one() {
  local r="$1"
  (cd "$dir" && $local_cmd "$name" --jar "$install_dir/emv.jar" \
    --rule "$r" --java_args "$heap" 2>&1 \
    | grep -E 'Verified|Violated|Timeout|Error' | head -4 \
    | sed "s|^|[$r] |") || true
}

if [ "$jobs" -le 1 ]; then
  for r in "${rules[@]}"; do echo "=== $name --rule $r"; run_one "$r"; done
else
  for r in "${rules[@]}"; do
    while [ "$(jobs -rp | wc -l)" -ge "$jobs" ]; do wait -n; done
    echo "=== $name --rule $r (parallel)"
    run_one "$r" &
    sleep 3 # stagger: unique report dirs in the shared conf dir
  done
  wait
fi
